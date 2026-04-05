use std::borrow::Cow;
use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::{anyhow, Result};
use futures::FutureExt;
use rmcp::model::{Content, ErrorCode, ErrorData, Tool};
use serde::Deserialize;
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

use crate::agents::harness::{bounded_subagent_depth_from_env, classify_child_task_result};
use crate::agents::subagent_handler::run_complete_subagent_task;
use crate::agents::subagent_task_config::TaskConfig;
use crate::agents::tool_execution::ToolCallResult;
use crate::config::AgimeMode;
use crate::providers;
use crate::recipe::build_recipe::build_recipe_from_template;
use crate::recipe::local_recipes::load_local_recipe_file;
use crate::recipe::{Recipe, SubRecipe};
use crate::session::SessionManager;

pub const SUBAGENT_TOOL_NAME: &str = "subagent";

const SUMMARY_INSTRUCTIONS: &str = r#"
Important: Your parent agent will only receive your final message as a summary of your work.
Make sure your last message provides a comprehensive summary of:
- What you were asked to do
- What actions you took
- The results or outcomes
- Any important findings or recommendations

Be concise but complete.
"#;

#[derive(Debug, Deserialize)]
pub struct SubagentParams {
    pub instructions: Option<String>,
    pub subrecipe: Option<String>,
    pub parameters: Option<HashMap<String, Value>>,
    pub extensions: Option<Vec<String>>,
    pub settings: Option<SubagentSettings>,
    #[serde(default = "default_summary")]
    pub summary: bool,
}

fn default_summary() -> bool {
    true
}

#[derive(Debug, Deserialize)]
pub struct SubagentSettings {
    pub provider: Option<String>,
    pub model: Option<String>,
    pub temperature: Option<f32>,
}

fn normalize_runtime_path(value: &str) -> Option<String> {
    let normalized = value.trim().replace('\\', "/");
    if normalized.is_empty() {
        None
    } else {
        Some(normalized)
    }
}

fn path_within_scope(path: &str, scope: &str) -> bool {
    path == scope
        || path
            .strip_prefix(scope)
            .is_some_and(|suffix| suffix.starts_with('/'))
}

fn validate_bounded_paths(task_config: &TaskConfig) -> Result<()> {
    let normalized_scope = task_config
        .write_scope
        .iter()
        .map(|value| {
            normalize_runtime_path(value)
                .ok_or_else(|| anyhow!("write_scope contains an empty or invalid entry"))
        })
        .collect::<Result<Vec<_>>>()?;
    let normalized_targets = task_config
        .target_artifacts
        .iter()
        .map(|value| {
            normalize_runtime_path(value)
                .ok_or_else(|| anyhow!("target_artifacts contains an empty or invalid entry"))
        })
        .collect::<Result<Vec<_>>>()?;
    let normalized_contract = task_config
        .result_contract
        .iter()
        .map(|value| {
            normalize_runtime_path(value)
                .ok_or_else(|| anyhow!("result_contract contains an empty or invalid entry"))
        })
        .collect::<Result<Vec<_>>>()?;

    if !normalized_scope.is_empty() {
        for path in normalized_targets.iter().chain(normalized_contract.iter()) {
            if !normalized_scope
                .iter()
                .any(|scope| path_within_scope(path, scope))
            {
                return Err(anyhow!(
                    "Bounded subagent path '{}' is outside declared write_scope [{}]",
                    path,
                    normalized_scope.join(", ")
                ));
            }
        }
    }

    Ok(())
}

fn apply_runtime_boundaries(mut recipe: Recipe, task_config: &TaskConfig) -> Recipe {
    let mut lines = Vec::new();
    lines.push(format!(
        "Delegation depth: {} of {}.",
        task_config.current_depth.saturating_add(1),
        task_config.max_depth
    ));
    if !task_config.write_scope.is_empty() {
        lines.push(format!(
            "Write only inside: {}",
            task_config.write_scope.join(", ")
        ));
    }
    if !task_config.target_artifacts.is_empty() {
        lines.push(format!(
            "Target artifacts for this bounded helper: {}",
            task_config.target_artifacts.join(", ")
        ));
    }
    if !task_config.result_contract.is_empty() {
        lines.push(format!(
            "Result contract for this helper: {}",
            task_config.result_contract.join(", ")
        ));
    }
    if task_config.force_summary_only {
        lines.push(
            "Return only the final concise summary to the parent agent; do not expect the parent to inspect your full transcript."
                .to_string(),
        );
    }

    if !lines.is_empty() {
        let boundary_block = format!(
            "Harness delegation constraints:\n{}",
            lines
                .into_iter()
                .map(|line| format!("- {}", line))
                .collect::<Vec<_>>()
                .join("\n")
        );
        let current = recipe.instructions.unwrap_or_default();
        recipe.instructions = Some(if current.is_empty() {
            boundary_block
        } else {
            format!("{}\n\n{}", boundary_block, current)
        });
    }

    recipe
}

pub fn validate_subagent_runtime_preconditions(
    task_config: &TaskConfig,
    params: &SubagentParams,
) -> Result<()> {
    if task_config.parent_session_id.trim().is_empty() {
        return Err(anyhow!("Subagent requires a parent session id"));
    }

    if task_config.current_depth >= task_config.max_depth {
        return Err(anyhow!(
            "Subagent depth limit reached (current={}, max={})",
            task_config.current_depth,
            task_config.max_depth
        ));
    }

    if task_config.force_summary_only && !params.summary {
        return Err(anyhow!(
            "This harness run requires bounded subagents to return summary-only output"
        ));
    }

    validate_bounded_paths(task_config)?;

    Ok(())
}

pub fn create_subagent_tool(sub_recipes: &[SubRecipe]) -> Tool {
    let description = build_tool_description(sub_recipes);

    let schema = json!({
        "type": "object",
        "properties": {
            "instructions": {
                "type": "string",
                "description": "Instructions for the subagent. Required for ad-hoc tasks. For predefined tasks, adds additional context."
            },
            "subrecipe": {
                "type": "string",
                "description": "Name of a predefined subrecipe to run."
            },
            "parameters": {
                "type": "object",
                "additionalProperties": true,
                "description": "Parameters for the subrecipe. Only valid when 'subrecipe' is specified."
            },
            "extensions": {
                "type": "array",
                "items": {"type": "string"},
                "description": "Extensions to enable. Omit to inherit all, empty array for none."
            },
            "settings": {
                "type": "object",
                "properties": {
                    "provider": {"type": "string", "description": "Override LLM provider"},
                    "model": {"type": "string", "description": "Override model"},
                    "temperature": {"type": "number", "description": "Override temperature"}
                },
                "description": "Override model/provider settings."
            }
        }
    });

    Tool::new(
        SUBAGENT_TOOL_NAME,
        description,
        schema.as_object().unwrap().clone(),
    )
}

fn build_tool_description(sub_recipes: &[SubRecipe]) -> String {
    let mut desc = String::from(
        "Delegate a task to a subagent that runs independently with its own context.\n\n\
         Modes:\n\
         1. Ad-hoc: Provide `instructions` for a custom task\n\
         2. Predefined: Provide `subrecipe` name to run a predefined task\n\
         3. Augmented: Provide both `subrecipe` and `instructions` to add context\n\n\
         The subagent has access to the same tools as you by default. \
         Use `extensions` to limit which extensions the subagent can use.\n\n\
         This tool launches one bounded helper subagent. For true multi-target parallel execution, use `swarm` instead.",
    );

    if !sub_recipes.is_empty() {
        desc.push_str("\n\nAvailable subrecipes:");
        for sr in sub_recipes {
            let params_info = get_subrecipe_params_description(sr);
            let sequential_hint = if sr.sequential_when_repeated {
                " [run sequentially, not in parallel]"
            } else {
                ""
            };
            desc.push_str(&format!(
                "\n• {}{} - {}{}",
                sr.name,
                sequential_hint,
                sr.description.as_deref().unwrap_or("No description"),
                if params_info.is_empty() {
                    String::new()
                } else {
                    format!(" (params: {})", params_info)
                }
            ));
        }
    }

    desc
}

fn get_subrecipe_params_description(sub_recipe: &SubRecipe) -> String {
    match load_local_recipe_file(&sub_recipe.path) {
        Ok(recipe_file) => match Recipe::from_content(&recipe_file.content) {
            Ok(recipe) => {
                if let Some(params) = recipe.parameters {
                    params
                        .iter()
                        .filter(|p| {
                            sub_recipe
                                .values
                                .as_ref()
                                .map(|v| !v.contains_key(&p.key))
                                .unwrap_or(true)
                        })
                        .map(|p| {
                            let req = match p.requirement {
                                crate::recipe::RecipeParameterRequirement::Required => "[required]",
                                _ => "[optional]",
                            };
                            format!("{} {}", p.key, req)
                        })
                        .collect::<Vec<_>>()
                        .join(", ")
                } else {
                    String::new()
                }
            }
            Err(_) => String::new(),
        },
        Err(_) => String::new(),
    }
}

/// Note: SubRecipe.sequential_when_repeated is surfaced as a hint in the tool description
/// (e.g., "[run sequentially, not in parallel]") but not enforced. The LLM controls
/// sequencing by making sequential vs parallel tool calls.
pub fn handle_subagent_tool(
    params: Value,
    task_config: TaskConfig,
    sub_recipes: HashMap<String, SubRecipe>,
    working_dir: PathBuf,
    cancellation_token: Option<CancellationToken>,
) -> ToolCallResult {
    let parsed_params: SubagentParams = match serde_json::from_value(params) {
        Ok(p) => p,
        Err(e) => {
            return ToolCallResult::from(Err(ErrorData {
                code: ErrorCode::INVALID_PARAMS,
                message: Cow::from(format!("Invalid parameters: {}", e)),
                data: None,
            }));
        }
    };

    if parsed_params.instructions.is_none() && parsed_params.subrecipe.is_none() {
        return ToolCallResult::from(Err(ErrorData {
            code: ErrorCode::INVALID_PARAMS,
            message: Cow::from("Must provide 'instructions' or 'subrecipe' (or both)"),
            data: None,
        }));
    }

    if parsed_params.parameters.is_some() && parsed_params.subrecipe.is_none() {
        return ToolCallResult::from(Err(ErrorData {
            code: ErrorCode::INVALID_PARAMS,
            message: Cow::from("'parameters' can only be used with 'subrecipe'"),
            data: None,
        }));
    }

    if let Err(e) = validate_subagent_runtime_preconditions(&task_config, &parsed_params) {
        return ToolCallResult::from(Err(ErrorData {
            code: ErrorCode::INVALID_REQUEST,
            message: Cow::from(e.to_string()),
            data: None,
        }));
    }

    let recipe = match build_recipe(&parsed_params, &sub_recipes) {
        Ok(r) => r,
        Err(e) => {
            return ToolCallResult::from(Err(ErrorData {
                code: ErrorCode::INVALID_PARAMS,
                message: Cow::from(e.to_string()),
                data: None,
            }));
        }
    };
    let recipe = apply_runtime_boundaries(recipe, &task_config);

    ToolCallResult {
        notification_stream: None,
        result: Box::new(
            execute_subagent(
                recipe,
                task_config,
                parsed_params,
                working_dir,
                cancellation_token,
            )
            .boxed(),
        ),
    }
}

async fn execute_subagent(
    recipe: Recipe,
    task_config: TaskConfig,
    params: SubagentParams,
    working_dir: PathBuf,
    cancellation_token: Option<CancellationToken>,
) -> Result<rmcp::model::CallToolResult, ErrorData> {
    let session = SessionManager::create_session(
        working_dir,
        "Subagent task".to_string(),
        crate::session::session_manager::SessionType::SubAgent,
    )
    .await
    .map_err(|e| ErrorData {
        code: ErrorCode::INTERNAL_ERROR,
        message: Cow::from(format!("Failed to create session: {}", e)),
        data: None,
    })?;

    let mut task_config = task_config;
    task_config.current_depth = task_config.current_depth.saturating_add(1);
    task_config.max_depth = task_config.max_depth.max(bounded_subagent_depth_from_env());

    let worker_name = format!("subagent_{}", &session.id);
    let task_config = apply_settings_overrides(task_config, &params)
        .await
        .map_err(|e| ErrorData {
            code: ErrorCode::INVALID_PARAMS,
            message: Cow::from(e.to_string()),
            data: None,
        })?
        .with_worker_runtime(
            None,
            Some(worker_name),
            Some("leader".to_string()),
            crate::agents::harness::CoordinatorRole::Worker,
            None,
            None,
            None,
            false,
            false,
            Vec::new(),
            Vec::new(),
            false,
        );

    let task_targets = task_config.target_artifacts.clone();
    let task_write_scope = task_config.write_scope.clone();
    let session_id = session.id.clone();
    let result = run_complete_subagent_task(
        recipe,
        task_config,
        true,
        session_id.clone(),
        cancellation_token,
    )
    .await;

    match result {
        Ok(text) => {
            let classification = classify_child_task_result(
                crate::agents::harness::TaskKind::Subagent,
                &task_targets,
                &task_write_scope,
                &text,
            );
            Ok(rmcp::model::CallToolResult {
                content: vec![Content::text(text.clone())],
                structured_content: Some(json!({
                    "status": "completed",
                    "session_id": session_id,
                    "summary": text,
                    "accepted_targets": classification.accepted_targets,
                    "produced_delta": classification.produced_delta,
                    "summary_only": true,
                })),
                is_error: Some(false),
                meta: None,
            })
        }
        Err(e) => Err(ErrorData {
            code: ErrorCode::INTERNAL_ERROR,
            message: Cow::from(e.to_string()),
            data: None,
        }),
    }
}

fn build_recipe(
    params: &SubagentParams,
    sub_recipes: &HashMap<String, SubRecipe>,
) -> Result<Recipe> {
    let mut recipe = if let Some(subrecipe_name) = &params.subrecipe {
        build_subrecipe(subrecipe_name, params, sub_recipes)?
    } else {
        build_adhoc_recipe(params)?
    };

    let current = recipe.instructions.unwrap_or_default();
    recipe.instructions = Some(format!("{}\n{}", current, SUMMARY_INSTRUCTIONS));

    Ok(recipe)
}

fn build_subrecipe(
    subrecipe_name: &str,
    params: &SubagentParams,
    sub_recipes: &HashMap<String, SubRecipe>,
) -> Result<Recipe> {
    let sub_recipe = sub_recipes.get(subrecipe_name).ok_or_else(|| {
        let available: Vec<_> = sub_recipes.keys().cloned().collect();
        anyhow!(
            "Unknown subrecipe '{}'. Available: {}",
            subrecipe_name,
            available.join(", ")
        )
    })?;

    let recipe_file = load_local_recipe_file(&sub_recipe.path)
        .map_err(|e| anyhow!("Failed to load subrecipe '{}': {}", subrecipe_name, e))?;

    let mut param_values: Vec<(String, String)> = Vec::new();

    if let Some(values) = &sub_recipe.values {
        for (k, v) in values {
            param_values.push((k.clone(), v.clone()));
        }
    }

    if let Some(provided_params) = &params.parameters {
        for (k, v) in provided_params {
            let value_str = match v {
                Value::String(s) => s.clone(),
                other => other.to_string(),
            };
            param_values.push((k.clone(), value_str));
        }
    }

    let mut recipe = build_recipe_from_template(
        recipe_file.content,
        &recipe_file.parent_dir,
        param_values,
        None::<fn(&str, &str) -> Result<String, anyhow::Error>>,
    )
    .map_err(|e| anyhow!("Failed to build subrecipe: {}", e))?;

    // Merge prompt into instructions so the subagent gets the actual task.
    // The subagent handler uses `instructions` as the user message.
    let mut combined = String::new();

    if let Some(instructions) = &recipe.instructions {
        combined.push_str(instructions);
    }

    if let Some(prompt) = &recipe.prompt {
        if !combined.is_empty() {
            combined.push_str("\n\n");
        }
        combined.push_str(prompt);
    }

    if let Some(extra_instructions) = &params.instructions {
        if !combined.is_empty() {
            combined.push_str("\n\n");
        }
        combined.push_str("Additional context from parent agent:\n");
        combined.push_str(extra_instructions);
    }

    if !combined.is_empty() {
        recipe.instructions = Some(combined);
    }

    Ok(recipe)
}

fn build_adhoc_recipe(params: &SubagentParams) -> Result<Recipe> {
    let instructions = params
        .instructions
        .as_ref()
        .ok_or_else(|| anyhow!("Instructions required for ad-hoc task"))?;

    let recipe = Recipe::builder()
        .version("1.0.0")
        .title("Subagent Task")
        .description("Ad-hoc subagent task")
        .instructions(instructions)
        .build()
        .map_err(|e| anyhow!("Failed to build recipe: {}", e))?;

    if recipe.check_for_security_warnings() {
        return Err(anyhow!("Recipe contains potentially harmful content"));
    }

    Ok(recipe)
}

async fn apply_settings_overrides(
    mut task_config: TaskConfig,
    params: &SubagentParams,
) -> Result<TaskConfig> {
    if let Some(settings) = &params.settings {
        if settings.provider.is_some() || settings.model.is_some() || settings.temperature.is_some()
        {
            let provider_name = settings
                .provider
                .clone()
                .unwrap_or_else(|| task_config.provider.get_name().to_string());

            let mut model_config = task_config.provider.get_model_config();

            if let Some(model) = &settings.model {
                model_config.model_name = model.clone();
            }

            if let Some(temp) = settings.temperature {
                model_config = model_config.with_temperature(Some(temp));
            }

            task_config.provider = providers::create(&provider_name, model_config)
                .await
                .map_err(|e| anyhow!("Failed to create provider '{}': {}", provider_name, e))?;
        }
    }

    if let Some(extension_names) = &params.extensions {
        if extension_names.is_empty() {
            task_config.extensions = Vec::new();
        } else {
            task_config
                .extensions
                .retain(|ext| extension_names.contains(&ext.name()));
        }
    }

    Ok(task_config)
}

pub fn should_enable_subagents(model_name: &str) -> bool {
    let config = crate::config::Config::global();
    let is_autonomous = config.get_agime_mode().unwrap_or(AgimeMode::Auto) == AgimeMode::Auto;
    if !is_autonomous {
        return false;
    }
    if model_name.starts_with("gemini") {
        return false;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conversation::message::Message;
    use crate::model::ModelConfig;
    use crate::providers::base::{Provider, ProviderMetadata, ProviderUsage};
    use crate::providers::errors::ProviderError;
    use std::path::Path;
    use std::sync::Arc;

    #[derive(Debug)]
    struct DummyProvider {
        model_config: ModelConfig,
    }

    #[async_trait::async_trait]
    impl Provider for DummyProvider {
        fn metadata() -> ProviderMetadata
        where
            Self: Sized,
        {
            ProviderMetadata::empty()
        }

        fn get_name(&self) -> &str {
            "dummy"
        }

        async fn complete_with_model(
            &self,
            _model_config: &ModelConfig,
            _system: &str,
            _messages: &[Message],
            _tools: &[Tool],
        ) -> Result<(Message, ProviderUsage), ProviderError> {
            panic!("DummyProvider should not be invoked in subagent unit tests")
        }

        fn get_model_config(&self) -> ModelConfig {
            self.model_config.clone()
        }
    }

    #[test]
    fn test_tool_name() {
        assert_eq!(SUBAGENT_TOOL_NAME, "subagent");
    }

    #[test]
    fn test_create_tool_without_subrecipes() {
        let tool = create_subagent_tool(&[]);
        assert_eq!(tool.name, "subagent");
        assert!(tool.description.as_ref().unwrap().contains("Ad-hoc"));
        assert!(!tool
            .description
            .as_ref()
            .unwrap()
            .contains("Available subrecipes"));
    }

    #[test]
    fn test_create_tool_with_subrecipes() {
        let sub_recipes = vec![SubRecipe {
            name: "test_recipe".to_string(),
            path: "test.yaml".to_string(),
            values: None,
            sequential_when_repeated: false,
            description: Some("A test recipe".to_string()),
        }];

        let tool = create_subagent_tool(&sub_recipes);
        assert!(tool
            .description
            .as_ref()
            .unwrap()
            .contains("Available subrecipes"));
        assert!(tool.description.as_ref().unwrap().contains("test_recipe"));
    }

    #[test]
    fn test_sequential_hint_in_description() {
        let sub_recipes = vec![
            SubRecipe {
                name: "parallel_ok".to_string(),
                path: "test.yaml".to_string(),
                values: None,
                sequential_when_repeated: false,
                description: Some("Can run in parallel".to_string()),
            },
            SubRecipe {
                name: "sequential_only".to_string(),
                path: "test.yaml".to_string(),
                values: None,
                sequential_when_repeated: true,
                description: Some("Must run sequentially".to_string()),
            },
        ];

        let tool = create_subagent_tool(&sub_recipes);
        let desc = tool.description.as_ref().unwrap();

        assert!(desc.contains("parallel_ok"));
        assert!(!desc.contains("parallel_ok [run sequentially"));

        assert!(desc.contains("sequential_only [run sequentially, not in parallel]"));
    }

    #[test]
    fn test_params_deserialization_full() {
        let params: SubagentParams = serde_json::from_value(json!({
            "instructions": "Extra context",
            "subrecipe": "my_recipe",
            "parameters": {"key": "value"},
            "extensions": ["developer"],
            "settings": {"model": "gpt-4"},
            "summary": false
        }))
        .unwrap();

        assert_eq!(params.instructions, Some("Extra context".to_string()));
        assert_eq!(params.subrecipe, Some("my_recipe".to_string()));
        assert!(params.parameters.is_some());
        assert_eq!(params.extensions, Some(vec!["developer".to_string()]));
        assert!(!params.summary);
    }

    #[test]
    fn runtime_boundaries_are_prefixed_into_recipe_instructions() {
        let recipe = Recipe::builder()
            .title("Subagent Task")
            .description("Ad-hoc subagent task")
            .instructions("Complete the task")
            .build()
            .expect("recipe should build");

        let task_config = TaskConfig::new(
            Arc::new(DummyProvider {
                model_config: ModelConfig::new_or_fail("gpt-4o"),
            }),
            "parent-session",
            Path::new("."),
            Vec::new(),
        )
        .with_delegation_depth(0, 2)
        .with_write_scope(vec!["src/generated".to_string()])
        .with_runtime_contract(
            vec!["src/generated/output.md".to_string()],
            vec!["src/generated/output.md".to_string()],
        );

        let bounded = apply_runtime_boundaries(recipe, &task_config);
        let instructions = bounded.instructions.expect("instructions should exist");
        assert!(instructions.contains("Harness delegation constraints:"));
        assert!(instructions.contains("Write only inside: src/generated"));
        assert!(instructions.contains("Target artifacts for this bounded helper"));
    }
}
