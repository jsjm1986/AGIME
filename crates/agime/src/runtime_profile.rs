use serde::{Deserialize, Serialize};

use crate::capabilities::{self, ResolvedCapabilities};
use crate::config::Config;
use crate::context_runtime::{
    CONTEXT_RUNTIME_OUTPUT_RESERVE_TOKENS, DEFAULT_AUTO_COMPACT_THRESHOLD,
};
use crate::model::{CacheEditMode, ModelConfig, PromptCachingMode};
use crate::providers::base::Provider;

const FALLBACK_CONTEXT_LIMIT: usize = 128_000;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderContextCapabilities {
    pub context_length: usize,
    pub max_completion_tokens: Option<usize>,
    pub supports_temperature: bool,
    pub supports_thinking: bool,
    pub supports_reasoning: bool,
    pub supports_prompt_caching: bool,
    pub supports_cache_edit: bool,
    pub use_max_completion_tokens: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserIntentProfile {
    pub model_name: String,
    pub api_format: Option<String>,
    pub api_url: Option<String>,
    pub context_limit: Option<usize>,
    pub max_tokens: Option<i32>,
    pub thinking_enabled: Option<bool>,
    pub thinking_budget: Option<u32>,
    pub reasoning_effort: Option<String>,
    pub output_reserve_tokens: Option<usize>,
    pub auto_compact_threshold: Option<f64>,
    pub prompt_caching_mode: PromptCachingMode,
    pub cache_edit_mode: CacheEditMode,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HintedCapabilityProfile {
    pub matched_pattern: Option<String>,
    pub provider: Option<String>,
    pub context_length: Option<usize>,
    pub max_completion_tokens: Option<usize>,
    pub supports_temperature: bool,
    pub supports_thinking: bool,
    pub supports_reasoning: bool,
    pub supports_tools: bool,
    pub use_max_completion_tokens: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EffectiveExecutionSettings {
    pub context_limit: usize,
    pub max_completion_tokens: Option<usize>,
    pub use_max_completion_tokens: bool,
    pub output_reserve_tokens: usize,
    pub auto_compact_threshold: f64,
    pub thinking_enabled: bool,
    pub thinking_budget: Option<u32>,
    pub reasoning_effort: Option<String>,
    pub prompt_caching_mode: PromptCachingMode,
    pub cache_edit_mode: CacheEditMode,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionDowngrade {
    pub field: String,
    pub from: Option<String>,
    pub to: Option<String>,
    pub reason: String,
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionSourceBreakdown {
    pub api_format: Option<String>,
    pub api_url: Option<String>,
    pub provider_mode: String,
    pub matched_pattern: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EffectiveModelRuntimeProfile {
    pub model_name: String,
    pub context_limit: usize,
    pub max_completion_tokens: Option<usize>,
    pub use_max_completion_tokens: bool,
    pub output_reserve_tokens: usize,
    pub auto_compact_threshold: f64,
    pub thinking_enabled: bool,
    pub thinking_budget: Option<u32>,
    pub reasoning_effort: Option<String>,
    pub prompt_caching_mode: PromptCachingMode,
    pub cache_edit_mode: CacheEditMode,
    pub capabilities: ProviderContextCapabilities,
    pub user_intent: UserIntentProfile,
    pub hinted_capabilities: HintedCapabilityProfile,
    pub effective_execution: EffectiveExecutionSettings,
    pub downgrades: Vec<ExecutionDowngrade>,
    pub warnings: Vec<String>,
    pub source_breakdown: ExecutionSourceBreakdown,
    #[serde(skip)]
    pub resolved_capabilities: ResolvedCapabilities,
}

pub fn resolve_from_model_config(model_config: &ModelConfig) -> EffectiveModelRuntimeProfile {
    build_profile(
        model_config,
        None,
        None,
        capabilities::resolve_with_model_config(model_config),
        false,
        false,
        false,
    )
}

pub fn apply_effective_execution(
    model_config: &ModelConfig,
    profile: &EffectiveModelRuntimeProfile,
) -> ModelConfig {
    let mut effective = model_config.clone();
    effective.context_limit = Some(profile.effective_execution.context_limit);
    effective.output_reserve_tokens = Some(profile.effective_execution.output_reserve_tokens);
    effective.auto_compact_threshold = Some(profile.effective_execution.auto_compact_threshold);
    effective.thinking_enabled = Some(profile.effective_execution.thinking_enabled);
    effective.thinking_budget = profile.effective_execution.thinking_budget;
    effective.reasoning_effort = profile.effective_execution.reasoning_effort.clone();
    effective.prompt_caching_mode = profile.effective_execution.prompt_caching_mode;
    effective.cache_edit_mode = profile.effective_execution.cache_edit_mode;
    effective
}

pub fn resolve_preview_profile(
    model_config: &ModelConfig,
    api_format: Option<&str>,
    api_url: Option<&str>,
) -> EffectiveModelRuntimeProfile {
    let provider_mode = infer_provider_mode(api_format, api_url);
    let supports_prompt_caching = !model_config.prompt_caching_mode.is_disabled()
        && matches!(
            provider_mode.as_str(),
            "anthropic"
                | "openrouter"
                | "litellm"
                | "litellm_openai_compat"
                | "litellm_openai_compat_estimated"
        );
    let supports_cache_edit = false;
    build_profile(
        model_config,
        api_format,
        api_url,
        capabilities::resolve_with_model_config(model_config),
        supports_prompt_caching,
        supports_cache_edit,
        true,
    )
}

pub async fn resolve_for_provider(provider: &dyn Provider) -> EffectiveModelRuntimeProfile {
    let model_config = provider.get_model_config();
    resolve_for_provider_with_model(provider, &model_config).await
}

pub async fn resolve_for_provider_with_model(
    provider: &dyn Provider,
    model_config: &ModelConfig,
) -> EffectiveModelRuntimeProfile {
    let supports_prompt_caching =
        !model_config.prompt_caching_mode.is_disabled() && provider.supports_cache_control().await;
    let supports_cache_edit =
        !model_config.cache_edit_mode.is_disabled() && provider.supports_cache_edit().await;
    build_profile(
        &model_config,
        Some(provider.get_name()),
        None,
        capabilities::resolve_with_model_config(&model_config),
        supports_prompt_caching,
        supports_cache_edit,
        false,
    )
}

fn build_profile(
    model_config: &ModelConfig,
    api_format: Option<&str>,
    api_url: Option<&str>,
    resolved: ResolvedCapabilities,
    supports_prompt_caching: bool,
    supports_cache_edit: bool,
    is_preview: bool,
) -> EffectiveModelRuntimeProfile {
    let user_intent = build_user_intent(model_config, api_format, api_url);
    let hinted_capabilities = build_hinted_capabilities(&resolved);
    let source_breakdown = ExecutionSourceBreakdown {
        api_format: user_intent.api_format.clone(),
        api_url: user_intent.api_url.clone(),
        provider_mode: infer_provider_mode(api_format, api_url),
        matched_pattern: resolved.matched_pattern.clone(),
    };

    let context_limit = user_intent
        .context_limit
        .or(hinted_capabilities.context_length)
        .unwrap_or(FALLBACK_CONTEXT_LIMIT)
        .max(1);
    let output_reserve_tokens = user_intent
        .output_reserve_tokens
        .unwrap_or(CONTEXT_RUNTIME_OUTPUT_RESERVE_TOKENS);
    let auto_compact_threshold = user_intent.auto_compact_threshold.unwrap_or_else(|| {
        Config::global()
            .get_param::<f64>("AGIME_AUTO_COMPACT_THRESHOLD")
            .unwrap_or(DEFAULT_AUTO_COMPACT_THRESHOLD)
    });

    let capabilities = ProviderContextCapabilities {
        context_length: context_limit,
        max_completion_tokens: hinted_capabilities.max_completion_tokens,
        supports_temperature: hinted_capabilities.supports_temperature,
        supports_thinking: hinted_capabilities.supports_thinking
            || user_intent.thinking_enabled.unwrap_or(false)
            || user_intent.thinking_budget.is_some(),
        supports_reasoning: hinted_capabilities.supports_reasoning
            || user_intent.reasoning_effort.is_some(),
        supports_prompt_caching,
        supports_cache_edit,
        use_max_completion_tokens: hinted_capabilities.use_max_completion_tokens,
    };

    let mut downgrades = Vec::new();
    let mut warnings = Vec::new();

    let mut effective_thinking_enabled = user_intent
        .thinking_enabled
        .unwrap_or(resolved.thinking_enabled);
    let mut effective_thinking_budget = user_intent.thinking_budget.or(resolved.thinking_budget);
    let mut effective_reasoning_effort = user_intent
        .reasoning_effort
        .clone()
        .or_else(|| resolved.reasoning_effort.clone());

    if effective_thinking_enabled && !capabilities.supports_thinking {
        if capabilities.supports_reasoning && effective_reasoning_effort.is_none() {
            effective_reasoning_effort = Some(map_budget_to_reasoning_effort(
                effective_thinking_budget.unwrap_or(16_000),
            ));
            if !is_preview {
                downgrades.push(ExecutionDowngrade {
                    field: "thinking_budget".to_string(),
                    from: effective_thinking_budget.map(|value| value.to_string()),
                    to: effective_reasoning_effort.clone(),
                    reason:
                        "provider/runtime does not support think budget; mapped to reasoning intensity"
                            .to_string(),
                    source: "runtime_capabilities".to_string(),
                });
            }
        } else if user_intent.thinking_enabled.unwrap_or(false)
            || user_intent.thinking_budget.is_some()
        {
            if !is_preview {
                warnings.push(
                    "当前模型/运行时不支持 Think 模式，本次执行将忽略 Thinking 配置。".to_string(),
                );
            }
        }

        if !is_preview && user_intent.thinking_enabled.unwrap_or(false) {
            downgrades.push(ExecutionDowngrade {
                field: "thinking_enabled".to_string(),
                from: Some("true".to_string()),
                to: Some("false".to_string()),
                reason: "provider/runtime does not support think mode".to_string(),
                source: "runtime_capabilities".to_string(),
            });
        }
        effective_thinking_enabled = false;
        effective_thinking_budget = None;
    }

    if !is_preview && effective_reasoning_effort.is_some() && !capabilities.supports_reasoning {
        downgrades.push(ExecutionDowngrade {
            field: "reasoning_effort".to_string(),
            from: effective_reasoning_effort.clone(),
            to: None,
            reason: "provider/runtime does not support reasoning effort".to_string(),
            source: "runtime_capabilities".to_string(),
        });
        effective_reasoning_effort = None;
    }

    let effective_prompt_caching_mode = if is_preview || capabilities.supports_prompt_caching {
        user_intent.prompt_caching_mode
    } else {
        if !matches!(user_intent.prompt_caching_mode, PromptCachingMode::Off) {
            downgrades.push(ExecutionDowngrade {
                field: "prompt_caching_mode".to_string(),
                from: Some(format!("{:?}", user_intent.prompt_caching_mode).to_ascii_lowercase()),
                to: Some("off".to_string()),
                reason: "provider/runtime does not support prompt caching".to_string(),
                source: "runtime_capabilities".to_string(),
            });
        }
        PromptCachingMode::Off
    };

    let effective_cache_edit_mode = if is_preview || capabilities.supports_cache_edit {
        user_intent.cache_edit_mode
    } else {
        if !matches!(user_intent.cache_edit_mode, CacheEditMode::Off) {
            downgrades.push(ExecutionDowngrade {
                field: "cache_edit_mode".to_string(),
                from: Some(format!("{:?}", user_intent.cache_edit_mode).to_ascii_lowercase()),
                to: Some("off".to_string()),
                reason:
                    "provider/runtime does not support cache edit; runtime will use no-op downgrade"
                        .to_string(),
                source: "runtime_capabilities".to_string(),
            });
        }
        CacheEditMode::Off
    };

    let effective_execution = EffectiveExecutionSettings {
        context_limit,
        max_completion_tokens: hinted_capabilities.max_completion_tokens,
        use_max_completion_tokens: hinted_capabilities.use_max_completion_tokens,
        output_reserve_tokens,
        auto_compact_threshold,
        thinking_enabled: effective_thinking_enabled,
        thinking_budget: effective_thinking_budget,
        reasoning_effort: effective_reasoning_effort.clone(),
        prompt_caching_mode: effective_prompt_caching_mode,
        cache_edit_mode: effective_cache_edit_mode,
    };

    EffectiveModelRuntimeProfile {
        model_name: model_config.model_name.clone(),
        context_limit,
        max_completion_tokens: hinted_capabilities.max_completion_tokens,
        use_max_completion_tokens: hinted_capabilities.use_max_completion_tokens,
        output_reserve_tokens,
        auto_compact_threshold,
        thinking_enabled: effective_thinking_enabled,
        thinking_budget: effective_thinking_budget,
        reasoning_effort: effective_reasoning_effort,
        prompt_caching_mode: effective_prompt_caching_mode,
        cache_edit_mode: effective_cache_edit_mode,
        capabilities,
        user_intent,
        hinted_capabilities,
        effective_execution,
        downgrades,
        warnings,
        source_breakdown,
        resolved_capabilities: resolved,
    }
}

fn build_user_intent(
    model_config: &ModelConfig,
    api_format: Option<&str>,
    api_url: Option<&str>,
) -> UserIntentProfile {
    UserIntentProfile {
        model_name: model_config.model_name.clone(),
        api_format: api_format.map(str::to_string),
        api_url: api_url.map(str::to_string),
        context_limit: model_config.context_limit,
        max_tokens: model_config.max_tokens,
        thinking_enabled: model_config.thinking_enabled,
        thinking_budget: model_config.thinking_budget,
        reasoning_effort: model_config.reasoning_effort.clone(),
        output_reserve_tokens: model_config.output_reserve_tokens,
        auto_compact_threshold: model_config.auto_compact_threshold,
        prompt_caching_mode: model_config.prompt_caching_mode,
        cache_edit_mode: model_config.cache_edit_mode,
    }
}

fn build_hinted_capabilities(resolved: &ResolvedCapabilities) -> HintedCapabilityProfile {
    HintedCapabilityProfile {
        matched_pattern: resolved.matched_pattern.clone(),
        provider: resolved.provider.clone(),
        context_length: resolved.context_length,
        max_completion_tokens: resolved.max_completion_tokens,
        supports_temperature: resolved.effective_temperature_supported(),
        supports_thinking: resolved.thinking_supported,
        supports_reasoning: resolved.reasoning_supported,
        supports_tools: resolved.tools_supported,
        use_max_completion_tokens: resolved.use_max_completion_tokens,
    }
}

fn map_budget_to_reasoning_effort(budget: u32) -> String {
    match budget {
        0..=8_192 => "low".to_string(),
        8_193..=24_000 => "medium".to_string(),
        _ => "high".to_string(),
    }
}

fn infer_provider_mode(api_format: Option<&str>, api_url: Option<&str>) -> String {
    let normalized_format = api_format
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("openai")
        .to_ascii_lowercase();
    let normalized_url = api_url
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or_default()
        .to_ascii_lowercase();

    match normalized_format.as_str() {
        "litellm" => "litellm".to_string(),
        "openai"
            if normalized_url.contains("litellm")
                || normalized_url.contains("api.litellm.ai")
                || normalized_url.contains("localhost:4000")
                || normalized_url.contains("127.0.0.1:4000") =>
        {
            "litellm_openai_compat".to_string()
        }
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conversation::message::Message;
    use crate::providers::base::{Provider, ProviderMetadata, ProviderUsage, Usage};
    use crate::providers::errors::ProviderError;
    use async_trait::async_trait;
    use rmcp::model::Tool;

    struct MockProvider {
        model: ModelConfig,
        supports_cache_control: bool,
        supports_cache_edit: bool,
    }

    #[async_trait]
    impl Provider for MockProvider {
        fn metadata() -> ProviderMetadata
        where
            Self: Sized,
        {
            ProviderMetadata::empty()
        }

        fn get_name(&self) -> &str {
            "mock"
        }

        async fn complete_with_model(
            &self,
            _model_config: &ModelConfig,
            _system: &str,
            _messages: &[Message],
            _tools: &[Tool],
        ) -> Result<(Message, ProviderUsage), ProviderError> {
            Ok((
                Message::assistant().with_text("ok"),
                ProviderUsage::new("mock".to_string(), Usage::default()),
            ))
        }

        fn get_model_config(&self) -> ModelConfig {
            self.model.clone()
        }

        async fn supports_cache_control(&self) -> bool {
            self.supports_cache_control
        }

        async fn supports_cache_edit(&self) -> bool {
            self.supports_cache_edit
        }
    }

    #[test]
    fn preview_profile_applies_capability_gated_cache_modes() {
        let mut model_config = ModelConfig::new_or_fail("claude-3-7-sonnet-latest");
        model_config.prompt_caching_mode = PromptCachingMode::Prefer;
        model_config.cache_edit_mode = CacheEditMode::Prefer;

        let anthropic = resolve_preview_profile(&model_config, Some("anthropic"), None);
        assert!(anthropic.capabilities.supports_prompt_caching);
        assert!(!anthropic.capabilities.supports_cache_edit);
        assert_eq!(
            anthropic.effective_execution.prompt_caching_mode,
            PromptCachingMode::Prefer
        );
        assert_eq!(
            anthropic.effective_execution.cache_edit_mode,
            CacheEditMode::Prefer
        );

        let openai = resolve_preview_profile(&model_config, Some("openai"), None);
        assert!(!openai.capabilities.supports_prompt_caching);
        assert!(!openai.capabilities.supports_cache_edit);
        assert_eq!(
            openai.effective_execution.prompt_caching_mode,
            PromptCachingMode::Prefer
        );
        assert_eq!(
            openai.effective_execution.cache_edit_mode,
            CacheEditMode::Prefer
        );
    }

    #[test]
    fn model_config_overrides_context_budget_in_profile() {
        let mut model_config = ModelConfig::new_or_fail("gpt-4.1");
        model_config.context_limit = Some(250_000);
        model_config.output_reserve_tokens = Some(12_000);
        model_config.auto_compact_threshold = Some(0.42);

        let profile = resolve_from_model_config(&model_config);
        assert_eq!(profile.context_limit, 250_000);
        assert_eq!(profile.capabilities.context_length, 250_000);
        assert_eq!(profile.output_reserve_tokens, 12_000);
        assert_eq!(profile.auto_compact_threshold, 0.42);
    }

    #[tokio::test]
    async fn resolve_for_provider_uses_provider_capability_hooks() {
        let mut model = ModelConfig::new_or_fail("claude-3-7-sonnet-latest");
        model.prompt_caching_mode = PromptCachingMode::Prefer;
        model.cache_edit_mode = CacheEditMode::Prefer;

        let unsupported = MockProvider {
            model: model.clone(),
            supports_cache_control: false,
            supports_cache_edit: false,
        };
        let unsupported_profile = resolve_for_provider(&unsupported).await;
        assert!(!unsupported_profile.capabilities.supports_prompt_caching);
        assert!(!unsupported_profile.capabilities.supports_cache_edit);
        assert_eq!(
            unsupported_profile.effective_execution.prompt_caching_mode,
            PromptCachingMode::Off
        );

        let supported = MockProvider {
            model,
            supports_cache_control: true,
            supports_cache_edit: true,
        };
        let supported_profile = resolve_for_provider(&supported).await;
        assert!(supported_profile.capabilities.supports_prompt_caching);
        assert!(supported_profile.capabilities.supports_cache_edit);
    }

    #[tokio::test]
    async fn resolve_for_provider_with_model_uses_call_specific_intent() {
        let provider_model = ModelConfig::new_or_fail("gpt-4o");
        let mut call_model = provider_model.clone();
        call_model.thinking_enabled = Some(true);
        call_model.thinking_budget = Some(16_000);
        call_model.prompt_caching_mode = PromptCachingMode::Prefer;

        let provider = MockProvider {
            model: provider_model,
            supports_cache_control: false,
            supports_cache_edit: false,
        };

        let profile = resolve_for_provider_with_model(&provider, &call_model).await;
        assert!(profile.user_intent.thinking_enabled.unwrap_or(false));
        assert_eq!(profile.user_intent.thinking_budget, Some(16_000));
        assert_eq!(
            profile.effective_execution.prompt_caching_mode,
            PromptCachingMode::Off
        );
    }

    #[test]
    fn preview_profile_detects_litellm_openai_compat() {
        let model = ModelConfig::new_or_fail("gpt-4o-mini");
        let profile = resolve_preview_profile(
            &model,
            Some("openai"),
            Some("http://localhost:4000/v1/chat/completions"),
        );
        assert_eq!(
            profile.source_breakdown.provider_mode,
            "litellm_openai_compat"
        );
        assert!(profile.capabilities.supports_prompt_caching);
    }

    #[test]
    fn preview_profile_preserves_user_intent_but_downgrades_thinking() {
        let mut model = ModelConfig::new_or_fail("glm-5.1");
        model.thinking_enabled = Some(true);
        model.thinking_budget = Some(16_000);
        let profile = resolve_preview_profile(&model, Some("openai"), None);
        assert!(profile.user_intent.thinking_enabled.unwrap_or(false));
        assert!(profile.effective_execution.thinking_enabled);
        assert!(profile.downgrades.is_empty());
    }

    #[test]
    fn apply_effective_execution_materializes_downgraded_model_config() {
        let mut model = ModelConfig::new_or_fail("glm-5.1");
        model.thinking_enabled = Some(true);
        model.thinking_budget = Some(16_000);
        model.prompt_caching_mode = PromptCachingMode::Prefer;

        let profile = resolve_preview_profile(&model, Some("openai"), None);
        let effective = apply_effective_execution(&model, &profile);

        assert_eq!(effective.thinking_enabled, Some(true));
        assert_eq!(effective.thinking_budget, Some(16_000));
        assert_eq!(effective.prompt_caching_mode, PromptCachingMode::Prefer);
        assert_eq!(effective.model_name, model.model_name);
    }
}
