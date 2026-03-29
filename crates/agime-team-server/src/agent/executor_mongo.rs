//! Task executor for running agent tasks (MongoDB version)
//!
//! This module provides the TaskExecutor which executes approved tasks
//! using the agime Provider abstraction layer for unified LLM access.

use agime::agents::extension::ExtensionInfo;
use agime::agents::final_output_tool::{
    FinalOutputTool, FINAL_OUTPUT_CONTINUATION_MESSAGE, FINAL_OUTPUT_TOOL_NAME,
};
use agime::agents::subagent_tool::{create_subagent_tool, should_enable_subagents};
use agime::agents::types::{
    RetryConfig, SuccessCheck, DEFAULT_ON_FAILURE_TIMEOUT_SECONDS, DEFAULT_RETRY_TIMEOUT_SECONDS,
};
use agime::context_mgmt::{
    compact_messages_with_strategy, ContextCompactionStrategy, DEFAULT_COMPACTION_THRESHOLD,
};
use agime::conversation::message::{Message, MessageContent};
use agime::conversation::{fix_conversation, Conversation};
use agime::prompt_template;
use agime::providers::base::{Provider, ProviderUsage};
use agime::providers::errors::ProviderError;
use agime::recipe::Response;
use agime::security::scanner::PromptInjectionScanner;
use agime::subprocess::configure_command_no_window;
use agime::token_counter::create_token_counter;
use agime_team::models::mongo::{ShellSecurityMode, Team};
use agime_team::models::{
    AgentTask, ApiFormat, BuiltinExtension, CustomExtensionConfig, TaskResultType, TaskStatus,
    TeamAgent,
};
use agime_team::MongoDb;
use anyhow::{anyhow, Result};
use chrono::{Local, Utc};
use futures::future::join_all;
use futures::StreamExt;
use mongodb::bson::{doc, Document};
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::process::{Output, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::process::Command;
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use super::context_injector::DocumentContextInjector;
use super::extension_installer::{AutoInstallPolicy, ExtensionInstaller};
use super::extension_manager_client::{DynamicExtensionState, TeamExtensionManagerClient};
use super::mcp_connector::{
    ApiCaller, ElicitationBridgeCallback, ElicitationBridgeEvent, McpConnector,
    ToolTaskProgressCallback,
};
use super::mission_manager::MissionManager;
use super::harness_core::{
    ActionPacket, HarnessDelegationMode, HarnessTurnMode, RunCheckpoint, RunCheckpointKind,
    RunJournal, RunMemory, RunStatus, TaskNode, TurnOutcome,
};
use super::hook_runtime;
use super::mission_mongo::{
    LaunchPolicy, MissionActionPacket, MissionProgressMemory, WorkerCompactState,
};
use super::platform_runner::PlatformExtensionRunner;
use super::provider_factory;
use super::resource_access::is_runtime_resource_allowed;
use super::runtime;
use super::service_mongo::{AgentService, AgentTaskDoc, TeamAgentDoc};
use super::subagent_scheduler;
use super::swarm_scheduler::{self, SwarmBootstrapExecution, SwarmBootstrapRequest};
use super::session_mongo::CreateSessionRequest;
use super::task_manager::{StreamEvent, TaskManager};

/// Build an HTTP client that respects system proxy settings.
/// On Windows, reads proxy from HTTPS_PROXY/HTTP_PROXY env vars,
/// and falls back to reading the Windows registry proxy settings.
pub(crate) fn build_http_client() -> Result<reqwest::Client> {
    let mut builder = apply_reqwest_tls_backend(reqwest::Client::builder())
        .timeout(std::time::Duration::from_secs(120));

    // Check env vars first (reqwest reads these automatically),
    // but also check Windows registry as fallback
    if std::env::var("HTTPS_PROXY").is_err() && std::env::var("HTTP_PROXY").is_err() {
        if let Some(proxy_url) = detect_system_proxy() {
            tracing::info!("Using system proxy: {}", proxy_url);
            let proxy =
                reqwest::Proxy::all(&proxy_url).map_err(|e| anyhow!("Invalid proxy URL: {}", e))?;
            builder = builder.proxy(proxy);
        }
    }

    builder
        .build()
        .map_err(|e| anyhow!("Failed to build HTTP client: {}", e))
}

fn apply_reqwest_tls_backend(builder: reqwest::ClientBuilder) -> reqwest::ClientBuilder {
    #[cfg(feature = "tls-native")]
    {
        builder.use_native_tls()
    }
    #[cfg(not(feature = "tls-native"))]
    {
        builder.use_rustls_tls()
    }
}

/// Detect system proxy on Windows from registry
#[cfg(target_os = "windows")]
pub(crate) fn detect_system_proxy() -> Option<String> {
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;

    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let settings = hkcu
        .open_subkey("Software\\Microsoft\\Windows\\CurrentVersion\\Internet Settings")
        .ok()?;
    let enabled: u32 = settings.get_value("ProxyEnable").ok()?;
    if enabled == 0 {
        return None;
    }
    let server: String = settings.get_value("ProxyServer").ok()?;
    if server.is_empty() {
        return None;
    }
    // Ensure it has a scheme
    if server.starts_with("http://")
        || server.starts_with("https://")
        || server.starts_with("socks5://")
    {
        Some(server)
    } else {
        Some(format!("http://{}", server))
    }
}

#[cfg(not(target_os = "windows"))]
pub(crate) fn detect_system_proxy() -> Option<String> {
    None
}

/// Max allowed tool timeout from env (2 hours).
const MAX_TOOL_TIMEOUT_SECS: u64 = 7200;
// This is a transport watchdog for a hung non-streaming provider call, not a business timeout.
// Mission control flow must treat expiry as external waiting/retry pressure instead of direct failure.
const DEFAULT_FALLBACK_COMPLETE_TIMEOUT_SECS: u64 = 900;
const MAX_FALLBACK_COMPLETE_TIMEOUT_SECS: u64 = 7200;

/// Max allowed turns from env to prevent runaway.
const MAX_UNIFIED_MAX_TURNS: usize = 5000;

/// Maximum characters for a single tool result before truncation
const MAX_TOOL_RESULT_CHARS: usize = 32_000;
/// Team Server always uses legacy segmented compaction; CFPM is local-only.
const SERVER_COMPACTION_MODE: &str = "legacy_segmented";
/// If context usage reaches this ratio again soon after compaction, allow immediate re-compaction.
const COMPACTION_REENTRY_RATIO: f64 = 0.90;
/// Minimum turns to wait before any follow-up compaction.
const MIN_TURNS_BETWEEN_COMPACTIONS: usize = 2;
/// With normal pressure (> threshold but < reentry ratio), wait longer before compaction again.
const MIN_TURNS_FOR_NORMAL_REENTRY: usize = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TeamResourceMode {
    Explicit,
    Auto,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TeamSkillMode {
    Assigned,
    OnDemand,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TeamAutoExtensionPolicy {
    ReviewedOnly,
    All,
}

#[derive(Debug, Clone)]
struct TeamRuntimeSettings {
    resource_mode: TeamResourceMode,
    skill_mode: TeamSkillMode,
    auto_extension_policy: TeamAutoExtensionPolicy,
    auto_install_extensions: AutoInstallPolicy,
    extension_cache_root: String,
    portal_base_url: String,
    workspace_root: String,
}

impl TeamRuntimeSettings {
    fn from_env() -> Self {
        let resource_mode = match std::env::var("TEAM_AGENT_RESOURCE_MODE")
            .unwrap_or_else(|_| "explicit".to_string())
            .to_lowercase()
            .as_str()
        {
            "auto" => TeamResourceMode::Auto,
            _ => TeamResourceMode::Explicit,
        };
        let skill_mode = match std::env::var("TEAM_AGENT_SKILL_MODE")
            .unwrap_or_else(|_| "on_demand".to_string())
            .to_lowercase()
            .as_str()
        {
            "on_demand" | "ondemand" => TeamSkillMode::OnDemand,
            _ => TeamSkillMode::Assigned,
        };
        let auto_extension_policy = match std::env::var("TEAM_AGENT_AUTO_EXTENSION_POLICY")
            .unwrap_or_else(|_| "reviewed_only".to_string())
            .to_lowercase()
            .as_str()
        {
            "all" => TeamAutoExtensionPolicy::All,
            _ => TeamAutoExtensionPolicy::ReviewedOnly,
        };
        let auto_install_extensions = if std::env::var("TEAM_AGENT_AUTO_INSTALL_EXTENSIONS")
            .map(|s| s.eq_ignore_ascii_case("true"))
            .unwrap_or(true)
        {
            AutoInstallPolicy::Enabled
        } else {
            AutoInstallPolicy::Disabled
        };
        let extension_cache_root = std::env::var("TEAM_AGENT_EXTENSION_CACHE_ROOT")
            .unwrap_or_else(|_| "./data/runtime/extensions".to_string());
        let portal_base_url = std::env::var("PORTAL_BASE_URL")
            .unwrap_or_else(|_| "http://127.0.0.1:8080".to_string());
        let workspace_root =
            std::env::var("WORKSPACE_ROOT").unwrap_or_else(|_| "./data/workspaces".to_string());

        Self {
            resource_mode,
            skill_mode,
            auto_extension_policy,
            auto_install_extensions,
            extension_cache_root,
            portal_base_url,
            workspace_root,
        }
    }
}

/// Mission-specific context injected into the system prompt when executing mission steps.
#[derive(Clone, serde::Deserialize)]
pub struct MissionPromptContext {
    pub goal: String,
    pub context: Option<String>,
    pub approval_policy: String,
    #[serde(default)]
    pub launch_policy: Option<LaunchPolicy>,
    pub total_steps: usize,
    pub current_step: usize,
    #[serde(default)]
    pub progress_memory: Option<MissionProgressMemory>,
    #[serde(default)]
    pub latest_worker_state: Option<WorkerCompactState>,
    #[serde(default)]
    pub task_node_id: Option<String>,
}

/// Context for rendering the system.md prompt template.
/// Mirrors the local agent's SystemPromptContext but simplified for team server use.
#[derive(Serialize)]
struct TeamSystemPromptContext {
    extensions: Vec<ExtensionInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_selection_strategy: Option<String>,
    current_date_time: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    extension_tool_limits: Option<(usize, usize)>,
    agime_mode: String,
    is_autonomous: bool,
    enable_subagents: bool,
    max_extensions: usize,
    max_tools: usize,
}

fn has_concrete_action_targets(packet: &super::mission_mongo::MissionActionPacket) -> bool {
    packet.target_files.iter().any(|item| {
        let trimmed = item.trim();
        !trimmed.is_empty()
            && trimmed.len() <= 160
            && !trimmed.chars().any(|ch| ch.is_whitespace())
            && (trimmed.contains('/')
                || trimmed.rsplit('/').next().is_some_and(|name| name.contains('.')))
    })
}

fn mission_contract_target_file(mission_ctx: Option<&MissionPromptContext>) -> Option<String> {
    let mission_ctx = mission_ctx?;
    let done_keys = mission_ctx
        .progress_memory
        .as_ref()
        .map(RunMemory::from)
        .map(|memory| {
            memory
                .done
                .into_iter()
                .filter_map(|path| runtime::normalize_relative_workspace_path(&path))
                .map(|path| path.to_ascii_lowercase())
                .collect::<HashSet<_>>()
        })
        .unwrap_or_default();
    let first_missing = mission_ctx
        .progress_memory
        .as_ref()
        .map(RunMemory::from)
        .and_then(|memory| memory.first_missing().map(|value| value.to_string()));
    let preferred_if_not_done = |candidate: Option<String>| -> Option<String> {
        let candidate = candidate?;
        let Some(normalized) = runtime::normalize_relative_workspace_path(&candidate) else {
            return None;
        };
        if done_keys.contains(&normalized.to_ascii_lowercase()) {
            return first_missing
                .as_ref()
                .and_then(|value| runtime::normalize_relative_workspace_path(value))
                .filter(|value| !done_keys.contains(&value.to_ascii_lowercase()));
        }
        Some(normalized)
    };
    preferred_if_not_done(first_missing.clone())
}

fn graph_contract_target_file(task_node: Option<&TaskNode>) -> Option<String> {
    let task_node = task_node?;
    task_node
        .target_artifacts
        .iter()
        .chain(task_node.result_contract.iter())
        .chain(task_node.write_scope.iter())
        .find_map(|candidate| runtime::normalize_relative_workspace_path(candidate))
}

fn effective_contract_target_file(
    mission_ctx: Option<&MissionPromptContext>,
    graph_task_node: Option<&TaskNode>,
) -> Option<String> {
    if graph_task_node.is_some() {
        graph_contract_target_file(graph_task_node)
    } else {
        mission_contract_target_file(mission_ctx)
    }
}

fn mission_task_node(mission_ctx: Option<&MissionPromptContext>) -> Option<TaskNode> {
    let mission_ctx = mission_ctx?;
    let task_node_id = mission_ctx.task_node_id.clone()?;
    let action_packet: ActionPacket = synthesized_bootstrap_action_packet(mission_ctx)
        .map(|packet| (&packet).into())
        .unwrap_or_default();
    Some(TaskNode {
        task_node_id,
        title: Some(mission_ctx.goal.clone()),
        mode: super::harness_core::HarnessTurnMode::Execute,
        target_artifacts: action_packet.target_artifacts,
        input_artifacts: action_packet.input_artifacts,
        delegation_mode: None,
        parallelism_budget: None,
        swarm_mode: None,
        swarm_budget: None,
        write_scope: mission_contract_target_file(Some(mission_ctx))
            .into_iter()
            .collect(),
        result_contract: mission_ctx
            .progress_memory
            .as_ref()
            .map(|memory| memory.missing.clone())
            .unwrap_or_default(),
    })
}

fn synthesized_bootstrap_action_packet(
    mission_ctx: &MissionPromptContext,
) -> Option<MissionActionPacket> {
    let target_file = mission_contract_target_file(Some(mission_ctx))?;
    let input_files = mission_ctx
        .progress_memory
        .as_ref()
        .map(RunMemory::from)
        .map(|memory| memory.done.into_iter().take(4).collect::<Vec<_>>())
        .unwrap_or_default();
    Some(MissionActionPacket {
        target_files: vec![target_file.clone()],
        input_files,
        required_tool_use: vec![
            "inspect the most relevant existing inputs or workspace files".to_string(),
            format!("create or materially update {}", target_file),
            format!("run one minimal validation for {}", target_file),
        ],
        expected_artifact_delta: vec![target_file.clone()],
        success_proof: vec![format!("{} exists or changed in this round", target_file)],
        failure_escalation: vec![format!(
            "if {} cannot be produced, save the strongest directly reusable blocker/handoff artifact instead of ending with analysis only",
            target_file
        )],
    })
}

fn mission_bootstrap_missing_candidates(mission_ctx: Option<&MissionPromptContext>) -> Vec<String> {
    let Some(mission_ctx) = mission_ctx else {
        return Vec::new();
    };
    let Some(progress) = mission_ctx.progress_memory.as_ref() else {
        return Vec::new();
    };
    let memory = RunMemory::from(progress);
    if !memory.done.is_empty() {
        return Vec::new();
    }
    let mut seen = HashSet::new();
    let mut values = Vec::new();
    for path in memory.missing {
        let Some(normalized) = runtime::normalize_relative_workspace_path(&path) else {
            continue;
        };
        let key = normalized.to_ascii_lowercase();
        if seen.insert(key) {
            values.push(normalized);
        }
    }
    values
}

pub(crate) fn workspace_target_file_changed(
    before: Option<&runtime::WorkspaceSnapshot>,
    after: &runtime::WorkspaceSnapshot,
    target: &str,
) -> bool {
    let Some(normalized) = runtime::normalize_relative_workspace_path(target) else {
        return false;
    };
    match (before.and_then(|snapshot| snapshot.get(&normalized)), after.get(&normalized)) {
        (_, Some(current)) => before
            .and_then(|snapshot| snapshot.get(&normalized))
            .is_none_or(|previous| previous != current),
        _ => false,
    }
}

fn workspace_any_candidate_file_changed(
    before: Option<&runtime::WorkspaceSnapshot>,
    after: &runtime::WorkspaceSnapshot,
    candidates: &[String],
) -> Option<String> {
    candidates
        .iter()
        .find(|candidate| workspace_target_file_changed(before, after, candidate))
        .cloned()
}

fn apply_bootstrap_file_delta_to_mission_context(
    mission_ctx: &mut Option<MissionPromptContext>,
    changed_file: &str,
) {
    let Some(mission_ctx) = mission_ctx.as_mut() else {
        return;
    };
    let Some(progress) = mission_ctx.progress_memory.as_mut() else {
        return;
    };
    let Some(normalized_changed) = runtime::normalize_relative_workspace_path(changed_file) else {
        return;
    };
    let changed_key = normalized_changed.to_ascii_lowercase();
    if !progress
        .done
        .iter()
        .filter_map(|path| runtime::normalize_relative_workspace_path(path))
        .any(|path| path.to_ascii_lowercase() == changed_key)
    {
        progress.done.push(normalized_changed.clone());
    }
    progress.missing.retain(|path| {
        runtime::normalize_relative_workspace_path(path)
            .map(|normalized| normalized.to_ascii_lowercase() != changed_key)
            .unwrap_or(true)
    });
    let next_missing = progress.missing.first().cloned();
    progress.next_best_action = next_missing.as_ref().map(|path| {
        format!(
            "Create or materially update {} first in this round using the strongest completed outputs as inputs.",
            path
        )
    });
}

/// Build the system prompt: core template + optional agent custom instructions.
///
/// The core system.md template is ALWAYS rendered as the base, ensuring identity,
/// behavioral rules, safety guardrails, and tool usage guidelines are never lost.
/// If the agent has a custom `system_prompt`, it is appended as `<agent_instructions>`
/// rather than replacing the core template.
fn build_system_prompt(
    extensions: &[ExtensionInfo],
    custom_prompt: Option<&str>,
    mission_context: Option<&MissionPromptContext>,
    enable_subagents: bool,
) -> String {
    let (mode, autonomous) = match mission_context {
        Some(mc) => ("mission".to_string(), mc.approval_policy == "auto"),
        None => ("chat".to_string(), false),
    };

    let context = TeamSystemPromptContext {
        extensions: extensions.to_vec(),
        tool_selection_strategy: None,
        current_date_time: Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
        extension_tool_limits: None,
        agime_mode: mode,
        is_autonomous: autonomous,
        enable_subagents: autonomous && enable_subagents,
        max_extensions: 5,
        max_tools: 50,
    };

    let mut prompt = match prompt_template::render_global_file("system.md", &context) {
        Ok(rendered) => rendered,
        Err(e) => {
            tracing::warn!("Failed to render system.md template: {}, using fallback", e);
            "You are a helpful AI assistant. Answer the user's questions accurately and concisely."
                .to_string()
        }
    };

    // Append agent-specific custom instructions (never replace core template)
    if let Some(custom) = custom_prompt {
        prompt.push_str("\n\n<agent_instructions>\n");
        prompt.push_str("The following are custom instructions configured for this agent. ");
        prompt.push_str("Follow these instructions while maintaining all core behavioral rules and safety guardrails above.\n\n");
        prompt.push_str(custom);
        prompt.push_str("\n</agent_instructions>");
    }

    // Append mission context when executing mission steps
    if let Some(mc) = mission_context {
        let bootstrap_packet = synthesized_bootstrap_action_packet(mc);
        prompt.push_str("\n\n<mission_context>\n");
        prompt.push_str("You are executing a multi-step mission autonomously.\n\n");
        prompt.push_str(&format!("## Mission Goal\n{}\n", mc.goal));
        if let Some(ref ctx) = mc.context {
            prompt.push_str(&format!("\n## Additional Context\n{}\n", ctx));
        }
        prompt.push_str("\n## Execution Rules\n");
        prompt.push_str("- You are in AUTONOMOUS execution mode. Complete each step without asking questions.\n");
        prompt.push_str(
            "- Focus on the current step. Do not skip ahead or revisit completed steps.\n",
        );
        prompt.push_str("- If a step cannot be completed, explain what went wrong clearly.\n");
        prompt.push_str("- Verify your work before reporting completion.\n");
        prompt.push_str(
            "- Be concise in your output — your response will be saved as step summary.\n",
        );
        prompt.push_str("- This turn must produce a concrete change: write a target file, run a tool-backed verification, or record a blocking file if completion is impossible.\n");
        prompt.push_str("- Do not stop at diagnosis. If `missing` or `next_best_action` is present below, treat it as your execution target for this turn.\n");
        prompt.push_str("- If you finish by only thinking or summarizing without touching files/tools, that is considered incomplete execution.\n");
        if context.enable_subagents {
            prompt.push_str("- The `subagent` tool is available in this mission. Use bounded delegation only when it materially improves the final result.\n");
        }
        prompt.push_str(&format!(
            "\n## Progress\nStep {}/{} — Approval policy: {}\n",
            mc.current_step, mc.total_steps, mc.approval_policy
        ));
        if let Some(task_node) = mission_task_node(Some(mc)) {
            prompt.push_str("\n## Shared Task Node\n");
            prompt.push_str(&format!("- task_node_id: {}\n", task_node.task_node_id));
            if !task_node.target_artifacts.is_empty() {
                prompt.push_str(&format!(
                    "- target_artifacts: {}\n",
                    task_node.target_artifacts.join(", ")
                ));
            }
            if !task_node.result_contract.is_empty() {
                prompt.push_str(&format!(
                    "- result_contract: {}\n",
                    task_node.result_contract.join(", ")
                ));
            }
        }
        if let Some(packet) = bootstrap_packet.as_ref() {
            prompt.push_str("\n## Bootstrap Action Packet\n");
            if !packet.target_files.is_empty() {
                prompt.push_str(&format!(
                    "- action_packet.target_files: {}\n",
                    packet.target_files.join(", ")
                ));
            }
            if !packet.input_files.is_empty() {
                prompt.push_str(&format!(
                    "- action_packet.input_files: {}\n",
                    packet.input_files.join(", ")
                ));
            }
            if !packet.required_tool_use.is_empty() {
                prompt.push_str(&format!(
                    "- action_packet.required_tool_use: {}\n",
                    packet.required_tool_use.join(", ")
                ));
            }
            if !packet.expected_artifact_delta.is_empty() {
                prompt.push_str(&format!(
                    "- action_packet.expected_artifact_delta: {}\n",
                    packet.expected_artifact_delta.join(", ")
                ));
            }
            if !packet.success_proof.is_empty() {
                prompt.push_str(&format!(
                    "- action_packet.success_proof: {}\n",
                    packet.success_proof.join(" | ")
                ));
            }
        }
        let contract_target_file = bootstrap_packet
            .as_ref()
            .and_then(|packet| packet.target_files.first().cloned())
            .or_else(|| {
                mc.progress_memory
                    .as_ref()
                    .and_then(|memory| memory.missing.first().cloned())
            });
        if let Some(target_file) = contract_target_file.as_deref() {
            prompt.push_str("\n## Contract Target\n");
            prompt.push_str(&format!("- target_file: {}\n", target_file));
            prompt.push_str("- rule: before ending this round, either create or materially update this file, or record a concrete environment/tooling blocker in the strongest directly reusable blocked/handoff file allowed by the task.\n");
        }
        if let Some(progress) = mc.progress_memory.as_ref() {
            if !progress.done.is_empty() && !progress.missing.is_empty() {
                prompt.push_str("\n## Reusable Inputs\n");
                prompt.push_str(&format!(
                    "- strongest_available_inputs: {}\n",
                    progress.done.iter().take(4).cloned().collect::<Vec<_>>().join(", ")
                ));
                prompt.push_str("- transaction_rule: use the strongest existing outputs as inputs to produce the first missing deliverable before expanding scope.\n");
            }
            if progress.done.is_empty() && !progress.missing.is_empty() {
                prompt.push_str("\n## First-Round Progress Contract\n");
                prompt.push_str("- You may inspect the workspace, query the environment, or verify assumptions first.\n");
                prompt.push_str("- Before ending this round, produce a concrete delta: create or update one missing deliverable file, save one reusable evidence file, or save one directly reusable blocked/handoff file.\n");
                prompt.push_str("- Do not end the round with only analysis text.\n");
            }
            prompt.push_str("\n## Progress Memory\n");
            if !progress.done.is_empty() {
                prompt.push_str(&format!("- done: {}\n", progress.done.join(", ")));
            }
            if !progress.missing.is_empty() {
                prompt.push_str(&format!("- missing: {}\n", progress.missing.join(", ")));
            }
            if let Some(blocked_by) = progress.blocked_by.as_deref() {
                prompt.push_str(&format!("- blocked_by: {}\n", blocked_by));
            }
            if let Some(last_failed_attempt) = progress.last_failed_attempt.as_deref() {
                prompt.push_str(&format!("- last_failed_attempt: {}\n", last_failed_attempt));
            }
            if let Some(next_best_action) = progress.next_best_action.as_deref() {
                prompt.push_str(&format!("- next_best_action: {}\n", next_best_action));
            }
        }
        if let Some(worker_state) = mc.latest_worker_state.as_ref() {
            prompt.push_str("\n## Latest Worker State\n");
            if let Some(current_goal) = worker_state.current_goal.as_deref() {
                prompt.push_str(&format!("- current_goal: {}\n", current_goal));
            }
            if !worker_state.core_assets_now.is_empty() {
                prompt.push_str(&format!(
                    "- core_assets_now: {}\n",
                    worker_state.core_assets_now.join(", ")
                ));
            }
            if !worker_state.assets_delta.is_empty() {
                prompt.push_str(&format!(
                    "- assets_delta: {}\n",
                    worker_state.assets_delta.join(", ")
                ));
            }
            if let Some(blocker) = worker_state.current_blocker.as_deref() {
                prompt.push_str(&format!("- current_blocker: {}\n", blocker));
            }
            if let Some(method) = worker_state.method_summary.as_deref() {
                prompt.push_str(&format!("- method_summary: {}\n", method));
            }
            if let Some(next_step) = worker_state.next_step_candidate.as_deref() {
                prompt.push_str(&format!("- next_step_candidate: {}\n", next_step));
            }
            if !worker_state.capability_signals.is_empty() {
                prompt.push_str(&format!(
                    "- capability_signals: {}\n",
                    worker_state.capability_signals.join(", ")
                ));
            }
        }
        if bootstrap_packet.is_some() {
            prompt.push_str(
                "\n## Execution Constraint\n- This round must produce a concrete workspace delta.\n- Start with the current contract target when one is provided.\n- Touch at least one target deliverable file or run one tool-backed validation that directly advances a target file.\n- Do not spend the full round on abstract analysis, restating the plan, or explaining blockers without trying a file-level action.\n- If the required file cannot be produced because of a concrete environment or tooling blocker, capture that blocker in the strongest directly reusable output allowed by the current task instead of looping.\n",
            );
            prompt.push_str(
                "- For creating or replacing a text/code/document file, prefer `developer__text_editor` with explicit `command`, `path`, and `file_text`.\n",
            );
            prompt.push_str(
                "- Use `developer__shell` for execution, validation, directory setup, or short one-line file operations only. Avoid long heredoc-based file generation when `developer__text_editor` can write the file directly.\n",
            );
        }
        prompt.push_str("</mission_context>");
    }

    prompt
}

/// Check if the agent has ExtensionManager enabled in its configuration.
fn agent_has_extension_manager_enabled(agent: &TeamAgent) -> bool {
    agent
        .enabled_extensions
        .iter()
        .any(|ext| ext.enabled && matches!(ext.extension, BuiltinExtension::ExtensionManager))
}

/// Convert enabled built-in extensions to CustomExtensionConfig entries.
/// Subprocess extensions (developer, memory, etc.) are started via `agime mcp <name>`.
/// Platform extensions (skills, todo, etc.) run in-process via PlatformExtensionRunner.
fn builtin_extensions_to_custom(agent: &TeamAgent) -> Vec<CustomExtensionConfig> {
    let agime_bin = find_agime_binary();
    let mut configs = Vec::new();

    for ext_config in &agent.enabled_extensions {
        if !ext_config.enabled {
            continue;
        }

        // Developer runs in-process via PlatformExtensionRunner
        if ext_config.extension == BuiltinExtension::Developer {
            continue;
        }

        // Only subprocess extensions can be started as MCP servers
        let mcp_name = match ext_config.extension.mcp_name() {
            Some(name) => name,
            None => {
                tracing::debug!(
                    "Skipping platform extension {:?} (not supported as subprocess)",
                    ext_config.extension
                );
                continue;
            }
        };

        if let Some(ref bin) = agime_bin {
            configs.push(CustomExtensionConfig {
                name: mcp_name.to_string(),
                ext_type: "stdio".to_string(),
                uri_or_cmd: bin.clone(),
                args: vec!["mcp".to_string(), mcp_name.to_string()],
                envs: HashMap::new(),
                enabled: true,
                source: None,
                source_extension_id: None,
            });
            tracing::info!(
                "Registered builtin extension '{}' as stdio MCP server",
                mcp_name
            );
        } else {
            tracing::warn!(
                "Cannot start builtin extension '{}': agime binary not found",
                mcp_name
            );
        }
    }

    configs
}

/// Find an extension config by name from the agent's full configuration
/// (including disabled extensions). Used to re-enable session extension overrides.
fn find_extension_config_by_name(agent: &TeamAgent, name: &str) -> Option<CustomExtensionConfig> {
    // Check custom extensions first (including disabled ones)
    if let Some(custom) = agent.custom_extensions.iter().find(|e| e.name == name) {
        let mut cfg = custom.clone();
        cfg.enabled = true;
        return Some(cfg);
    }

    // Check builtin extensions (use builtin_to_custom_config from extension_manager_client)
    // Match against both name() (snake_case API name) and mcp_name() (subprocess runtime name)
    for ext_config in &agent.enabled_extensions {
        let matches =
            ext_config.extension.name() == name || ext_config.extension.mcp_name() == Some(name);
        if matches {
            if let Some(cfg) =
                super::extension_manager_client::builtin_to_custom_config(&ext_config.extension)
            {
                return Some(cfg);
            }
        }
    }

    None
}

/// Find the agime binary path for starting subprocess MCP servers.
pub(super) fn find_agime_binary() -> Option<String> {
    // 1. Prefer current executable (agime-team-server now supports `mcp` subcommand)
    if let Ok(exe) = std::env::current_exe() {
        if exe.exists() {
            let is_supported_self = exe
                .file_stem()
                .and_then(|s| s.to_str())
                .map(|s| matches!(s, "agime-team-server" | "agime" | "agime-cli"))
                .unwrap_or(false);
            if is_supported_self {
                return Some(exe.to_string_lossy().to_string());
            }
        }

        // Fallback: look for sibling agime/agime-cli binaries
        if let Some(dir) = exe.parent() {
            let agime_path = dir.join(if cfg!(windows) { "agime.exe" } else { "agime" });
            if agime_path.exists() {
                return Some(agime_path.to_string_lossy().to_string());
            }
            let cli_path = dir.join(if cfg!(windows) {
                "agime-cli.exe"
            } else {
                "agime-cli"
            });
            if cli_path.exists() {
                return Some(cli_path.to_string_lossy().to_string());
            }
        }
    }

    // 2. Try PATH
    if let Ok(output) = std::process::Command::new(if cfg!(windows) { "where" } else { "which" })
        .arg("agime")
        .output()
    {
        if output.status.success() {
            let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !path.is_empty() {
                return Some(path.lines().next().unwrap_or(&path).to_string());
            }
        }
    }

    None
}

/// Load team shared extensions according to policy.
/// Returns empty Vec on failure (does not block task execution).
async fn load_team_shared_extensions(
    db: &MongoDb,
    team_id: &str,
    policy: TeamAutoExtensionPolicy,
    installer: &ExtensionInstaller,
) -> Vec<CustomExtensionConfig> {
    use agime_team::services::mongo::extension_service_mongo::ExtensionService;

    let ext_service = ExtensionService::new(db.clone());
    let extensions = match policy {
        TeamAutoExtensionPolicy::ReviewedOnly => ext_service.list_reviewed_for_team(team_id).await,
        TeamAutoExtensionPolicy::All => ext_service.list_active_for_team(team_id).await,
    };
    let extensions = match extensions {
        Ok(exts) => exts,
        Err(e) => {
            tracing::warn!(
                "Failed to load team shared extensions for team {}: {}",
                team_id,
                e
            );
            return Vec::new();
        }
    };

    let mut configs = Vec::new();
    for ext in extensions {
        if !is_runtime_resource_allowed(&ext.visibility, &ext.protection_level) {
            tracing::debug!(
                "Skipping team extension '{}' due to runtime visibility/protection policy (visibility={}, protection_level={})",
                ext.name,
                ext.visibility,
                ext.protection_level
            );
            continue;
        }

        match installer.resolve_team_extension(team_id, &ext).await {
            Ok(cfg) => {
                tracing::info!(
                    "Loaded team shared extension '{}' (type={}, policy={:?})",
                    ext.name,
                    ext.extension_type,
                    policy
                );
                configs.push(cfg);
            }
            Err(e) => {
                tracing::warn!(
                    "Skipping team extension '{}' due to runtime resolve error: {}",
                    ext.name,
                    e
                );
            }
        }
    }

    configs
}

/// Resolve agent custom extensions that came from team shared resources.
/// This makes explicitly added team extensions benefit from runtime installer/normalizer
/// even when TEAM_AGENT_RESOURCE_MODE is explicit.
async fn resolve_agent_custom_extensions(
    db: &MongoDb,
    team_id: &str,
    custom_extensions: &[CustomExtensionConfig],
    installer: &ExtensionInstaller,
) -> Vec<CustomExtensionConfig> {
    use agime_team::services::mongo::extension_service_mongo::ExtensionService;

    let ext_service = ExtensionService::new(db.clone());
    let mut resolved = Vec::new();

    for ext in custom_extensions.iter().filter(|e| e.enabled) {
        let is_team_source =
            ext.source.as_deref() == Some("team") || ext.source_extension_id.is_some();
        if !is_team_source {
            resolved.push(ext.clone());
            continue;
        }

        let source_id = match ext.source_extension_id.as_deref() {
            Some(id) if !id.trim().is_empty() => id,
            _ => {
                tracing::warn!(
                    "Team-sourced extension '{}' has no source_extension_id, using stored config as-is",
                    ext.name
                );
                resolved.push(ext.clone());
                continue;
            }
        };

        let shared = match ext_service.get(source_id).await {
            Ok(Some(doc)) => doc,
            Ok(None) => {
                tracing::warn!(
                    "Source extension '{}' not found for '{}', using stored config as-is",
                    source_id,
                    ext.name
                );
                resolved.push(ext.clone());
                continue;
            }
            Err(e) => {
                tracing::warn!(
                    "Failed to load source extension '{}' for '{}': {}; using stored config as-is",
                    source_id,
                    ext.name,
                    e
                );
                resolved.push(ext.clone());
                continue;
            }
        };

        match installer.resolve_team_extension(team_id, &shared).await {
            Ok(cfg) => {
                resolved.push(cfg);
            }
            Err(e) => {
                tracing::warn!(
                    "Failed to resolve team extension '{}' ({}): {}; using stored config as-is",
                    ext.name,
                    source_id,
                    e
                );
                resolved.push(ext.clone());
            }
        }
    }

    resolved
}

/// Detects repeated identical tool calls to prevent infinite loops
struct RepetitionDetector {
    last_call: Option<(String, String)>, // (name, args_json)
    repeat_count: u32,
}

impl RepetitionDetector {
    fn new() -> Self {
        Self {
            last_call: None,
            repeat_count: 0,
        }
    }

    fn repetition_threshold_for_tool(name: &str) -> Option<u32> {
        let lower = name.trim().to_ascii_lowercase();
        if lower.starts_with("mission_preflight__") {
            return None;
        }
        let shell_like = ["shell", "bash", "cmd", "exec", "terminal", "run_command"]
            .iter()
            .any(|kw| lower.contains(kw));
        shell_like.then_some(5)
    }

    /// Check if a tool call is allowed. Returns false once an identical call
    /// reaches the tool-specific repetition threshold consecutively.
    fn check(&mut self, name: &str, args: &serde_json::Value) -> bool {
        let args_json = serde_json::to_string(args).unwrap_or_default();
        let current = (name.to_string(), args_json);
        let Some(threshold) = Self::repetition_threshold_for_tool(name) else {
            self.last_call = Some(current);
            self.repeat_count = 1;
            return true;
        };
        if self.last_call.as_ref() == Some(&current) {
            self.repeat_count += 1;
            self.repeat_count < threshold
        } else {
            self.last_call = Some(current);
            self.repeat_count = 1;
            true
        }
    }
}

/// ApiCaller adapter that holds agent config for MCP Sampling
struct AgentApiCaller {
    api_url: Option<String>,
    api_key: Option<String>,
    model: Option<String>,
    api_format: ApiFormat,
}

impl ApiCaller for AgentApiCaller {
    fn call_llm<'a>(
        &'a self,
        system: &'a str,
        messages: Vec<serde_json::Value>,
        max_tokens: u32,
        tools: Option<Vec<rmcp::model::Tool>>,
        tool_choice: Option<rmcp::model::ToolChoice>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<serde_json::Value>> + Send + 'a>>
    {
        Box::pin(async move {
            let client = build_http_client()?;
            match self.api_format {
                ApiFormat::Anthropic => {
                    self.call_anthropic(
                        client,
                        system,
                        &messages,
                        max_tokens,
                        tools.as_deref(),
                        tool_choice.as_ref(),
                    )
                    .await
                }
                ApiFormat::OpenAI => {
                    self.call_openai(
                        client,
                        system,
                        &messages,
                        max_tokens,
                        tools.as_deref(),
                        tool_choice.as_ref(),
                    )
                    .await
                }
                ApiFormat::Local => Err(anyhow!("Local API does not support MCP Sampling")),
            }
        })
    }
}

impl AgentApiCaller {
    fn anthropic_tools_payload(
        tools: Option<&[rmcp::model::Tool]>,
    ) -> Option<Vec<serde_json::Value>> {
        let tools = tools?;
        if tools.is_empty() {
            return None;
        }
        Some(
            tools
                .iter()
                .map(|tool| {
                    serde_json::json!({
                        "name": tool.name,
                        "description": tool.description.clone().unwrap_or_default(),
                        "input_schema": tool.input_schema,
                    })
                })
                .collect(),
        )
    }

    fn openai_tools_payload(tools: Option<&[rmcp::model::Tool]>) -> Option<Vec<serde_json::Value>> {
        let tools = tools?;
        if tools.is_empty() {
            return None;
        }
        Some(
            tools
                .iter()
                .map(|tool| {
                    serde_json::json!({
                        "type": "function",
                        "function": {
                            "name": tool.name,
                            "description": tool.description.clone().unwrap_or_default(),
                            "parameters": tool.input_schema,
                        }
                    })
                })
                .collect(),
        )
    }

    fn anthropic_tool_choice_payload(
        tool_choice: Option<&rmcp::model::ToolChoice>,
    ) -> Option<serde_json::Value> {
        match tool_choice.and_then(|c| c.mode.clone()) {
            Some(rmcp::model::ToolChoiceMode::Required) => Some(serde_json::json!({"type": "any"})),
            Some(rmcp::model::ToolChoiceMode::Auto) => Some(serde_json::json!({"type": "auto"})),
            Some(rmcp::model::ToolChoiceMode::None) => None,
            None => None,
        }
    }

    fn openai_tool_choice_payload(
        tool_choice: Option<&rmcp::model::ToolChoice>,
    ) -> Option<serde_json::Value> {
        match tool_choice.and_then(|c| c.mode.clone()) {
            Some(rmcp::model::ToolChoiceMode::Required) => Some(serde_json::json!("required")),
            Some(rmcp::model::ToolChoiceMode::Auto) => Some(serde_json::json!("auto")),
            Some(rmcp::model::ToolChoiceMode::None) => Some(serde_json::json!("none")),
            None => None,
        }
    }

    fn normalize_anthropic_content_blocks(content: &serde_json::Value) -> Vec<serde_json::Value> {
        fn text_block(text: String) -> serde_json::Value {
            serde_json::json!({
                "type": "text",
                "text": text,
            })
        }

        let mut blocks = Vec::new();
        if let Some(items) = content.as_array() {
            for item in items {
                let block_type = item.get("type").and_then(|v| v.as_str()).unwrap_or("text");
                match block_type {
                    "text" => {
                        let text = item
                            .get("text")
                            .and_then(|v| v.as_str())
                            .unwrap_or_default()
                            .to_string();
                        blocks.push(text_block(text));
                    }
                    "tool_use" => {
                        let id = item
                            .get("id")
                            .and_then(|v| v.as_str())
                            .unwrap_or_default()
                            .to_string();
                        let name = item
                            .get("name")
                            .and_then(|v| v.as_str())
                            .unwrap_or_default()
                            .to_string();
                        let input = item
                            .get("input")
                            .and_then(|v| v.as_object().cloned())
                            .unwrap_or_default();
                        if !id.is_empty() && !name.is_empty() {
                            blocks.push(serde_json::json!({
                                "type": "tool_use",
                                "id": id,
                                "name": name,
                                "input": input,
                            }));
                        }
                    }
                    "tool_result" => {
                        let tool_use_id = item
                            .get("tool_use_id")
                            .and_then(|v| v.as_str())
                            .or_else(|| item.get("toolUseId").and_then(|v| v.as_str()))
                            .unwrap_or_default()
                            .to_string();
                        if tool_use_id.is_empty() {
                            continue;
                        }
                        let content_value = item
                            .get("content")
                            .cloned()
                            .unwrap_or_else(|| serde_json::json!([]));
                        let is_error = item
                            .get("is_error")
                            .and_then(|v| v.as_bool())
                            .or_else(|| item.get("isError").and_then(|v| v.as_bool()))
                            .unwrap_or(false);
                        blocks.push(serde_json::json!({
                            "type": "tool_result",
                            "tool_use_id": tool_use_id,
                            "content": content_value,
                            "is_error": is_error,
                        }));
                    }
                    _ => {
                        if let Some(text) = item.as_str() {
                            blocks.push(text_block(text.to_string()));
                        } else {
                            blocks.push(text_block(item.to_string()));
                        }
                    }
                }
            }
        } else if let Some(text) = content.as_str() {
            blocks.push(text_block(text.to_string()));
        } else if !content.is_null() {
            blocks.push(text_block(content.to_string()));
        }

        if blocks.is_empty() {
            blocks.push(text_block(String::new()));
        }
        blocks
    }

    fn normalize_anthropic_messages(messages: &[serde_json::Value]) -> Vec<serde_json::Value> {
        messages
            .iter()
            .map(|msg| {
                let role = msg
                    .get("role")
                    .and_then(|v| v.as_str())
                    .unwrap_or("user")
                    .to_string();
                let content = msg.get("content").unwrap_or(&serde_json::Value::Null);
                serde_json::json!({
                    "role": role,
                    "content": Self::normalize_anthropic_content_blocks(content),
                })
            })
            .collect()
    }

    fn openai_tool_result_text(content: &serde_json::Value) -> String {
        if let Some(s) = content.as_str() {
            return s.to_string();
        }
        if let Some(items) = content.as_array() {
            let mut parts = Vec::new();
            for item in items {
                if let Some(text) = item.get("text").and_then(|v| v.as_str()) {
                    parts.push(text.to_string());
                } else if let Some(text) = item.as_str() {
                    parts.push(text.to_string());
                }
            }
            return parts.join("\n");
        }
        if content.is_null() {
            return String::new();
        }
        content.to_string()
    }

    fn flush_openai_text_message(
        out: &mut Vec<serde_json::Value>,
        role: &str,
        text_buf: &mut String,
    ) -> bool {
        if text_buf.is_empty() {
            return false;
        }
        out.push(serde_json::json!({
            "role": role,
            "content": text_buf.clone(),
        }));
        text_buf.clear();
        true
    }

    fn normalize_openai_messages(messages: &[serde_json::Value]) -> Vec<serde_json::Value> {
        let mut out = Vec::new();
        for msg in messages {
            let role = msg
                .get("role")
                .and_then(|v| v.as_str())
                .unwrap_or("user")
                .to_string();
            let Some(content) = msg.get("content") else {
                continue;
            };

            if let Some(text) = content.as_str() {
                out.push(serde_json::json!({
                    "role": role,
                    "content": text,
                }));
                continue;
            }

            let Some(items) = content.as_array() else {
                out.push(serde_json::json!({
                    "role": role,
                    "content": content.to_string(),
                }));
                continue;
            };

            let mut text_buf = String::new();
            let mut emitted = false;

            for item in items {
                let item_type = item.get("type").and_then(|v| v.as_str()).unwrap_or("text");
                match item_type {
                    "text" => {
                        if let Some(text) = item.get("text").and_then(|v| v.as_str()) {
                            if !text_buf.is_empty() {
                                text_buf.push('\n');
                            }
                            text_buf.push_str(text);
                        }
                    }
                    "tool_use" => {
                        if Self::flush_openai_text_message(&mut out, &role, &mut text_buf) {
                            emitted = true;
                        }
                        let id = item
                            .get("id")
                            .and_then(|v| v.as_str())
                            .unwrap_or_default()
                            .to_string();
                        let name = item
                            .get("name")
                            .and_then(|v| v.as_str())
                            .unwrap_or_default()
                            .to_string();
                        if id.is_empty() || name.is_empty() {
                            continue;
                        }
                        let args_value = item
                            .get("input")
                            .cloned()
                            .unwrap_or_else(|| serde_json::json!({}));
                        let arguments = if let Some(s) = args_value.as_str() {
                            s.to_string()
                        } else {
                            serde_json::to_string(&args_value).unwrap_or_else(|_| "{}".to_string())
                        };
                        out.push(serde_json::json!({
                            "role": "assistant",
                            "content": serde_json::Value::Null,
                            "tool_calls": [{
                                "id": id,
                                "type": "function",
                                "function": {
                                    "name": name,
                                    "arguments": arguments,
                                }
                            }],
                        }));
                        emitted = true;
                    }
                    "tool_result" => {
                        if Self::flush_openai_text_message(&mut out, &role, &mut text_buf) {
                            emitted = true;
                        }
                        let tool_call_id = item
                            .get("toolUseId")
                            .and_then(|v| v.as_str())
                            .or_else(|| item.get("tool_use_id").and_then(|v| v.as_str()))
                            .unwrap_or_default()
                            .to_string();
                        if tool_call_id.is_empty() {
                            continue;
                        }
                        let result_text = Self::openai_tool_result_text(
                            item.get("content").unwrap_or(&serde_json::Value::Null),
                        );
                        out.push(serde_json::json!({
                            "role": "tool",
                            "tool_call_id": tool_call_id,
                            "content": result_text,
                        }));
                        emitted = true;
                    }
                    _ => {
                        if let Some(text) = item.as_str() {
                            if !text_buf.is_empty() {
                                text_buf.push('\n');
                            }
                            text_buf.push_str(text);
                        }
                    }
                }
            }
            if Self::flush_openai_text_message(&mut out, &role, &mut text_buf) {
                emitted = true;
            }
            if !emitted {
                out.push(serde_json::json!({
                    "role": role,
                    "content": "",
                }));
            }
        }
        out
    }

    async fn call_anthropic(
        &self,
        client: reqwest::Client,
        system: &str,
        messages: &[serde_json::Value],
        max_tokens: u32,
        tools: Option<&[rmcp::model::Tool]>,
        tool_choice: Option<&rmcp::model::ToolChoice>,
    ) -> Result<serde_json::Value> {
        let base_url = self
            .api_url
            .as_deref()
            .unwrap_or("https://api.anthropic.com");
        let model = self.model.as_deref().unwrap_or("claude-3-opus-20240229");
        let api_key = self
            .api_key
            .as_deref()
            .ok_or_else(|| anyhow!("API key not configured"))?;

        let is_volcengine = base_url.contains("ark.cn-beijing.volces.com");

        let api_url = if base_url.ends_with("/messages") || base_url.ends_with("/v1/messages") {
            base_url.to_string()
        } else if base_url.ends_with("/v1") {
            format!("{}/messages", base_url)
        } else {
            format!("{}/v1/messages", base_url.trim_end_matches('/'))
        };

        let mut request = client
            .post(&api_url)
            .header("Content-Type", "application/json");

        if is_volcengine {
            request = request.header("Authorization", format!("Bearer {}", api_key));
        } else {
            request = request
                .header("x-api-key", api_key)
                .header("anthropic-version", "2023-06-01");
        }

        let normalized_messages = Self::normalize_anthropic_messages(messages);
        let mut body = serde_json::json!({
            "model": model,
            "max_tokens": max_tokens,
            "messages": normalized_messages,
        });
        if !system.is_empty() {
            body["system"] = serde_json::json!(system);
        }
        if let Some(tool_defs) = Self::anthropic_tools_payload(tools) {
            body["tools"] = serde_json::json!(tool_defs);
        }
        if let Some(choice) = Self::anthropic_tool_choice_payload(tool_choice) {
            body["tool_choice"] = choice;
        }

        let response = request.json(&body).send().await?;
        if !response.status().is_success() {
            let error = response.text().await?;
            return Err(anyhow!("Anthropic API error: {}", error));
        }
        response
            .json()
            .await
            .map_err(|e| anyhow!("Parse error: {}", e))
    }

    async fn call_openai(
        &self,
        client: reqwest::Client,
        system: &str,
        messages: &[serde_json::Value],
        max_tokens: u32,
        tools: Option<&[rmcp::model::Tool]>,
        tool_choice: Option<&rmcp::model::ToolChoice>,
    ) -> Result<serde_json::Value> {
        let api_url = self
            .api_url
            .as_deref()
            .unwrap_or("https://api.openai.com/v1/chat/completions");
        let model = self.model.as_deref().unwrap_or("gpt-4");
        let api_key = self
            .api_key
            .as_deref()
            .ok_or_else(|| anyhow!("API key not configured"))?;

        // Prepend system message if provided
        let mut all_messages = Vec::new();
        if !system.is_empty() {
            all_messages.push(serde_json::json!({"role": "system", "content": system}));
        }
        all_messages.extend(Self::normalize_openai_messages(messages));

        let mut body = serde_json::json!({
            "model": model,
            "messages": all_messages,
            "max_tokens": max_tokens,
        });
        if let Some(tool_defs) = Self::openai_tools_payload(tools) {
            body["tools"] = serde_json::json!(tool_defs);
        }
        if let Some(choice) = Self::openai_tool_choice_payload(tool_choice) {
            body["tool_choice"] = choice;
        }

        let response = client
            .post(api_url)
            .header("Content-Type", "application/json")
            .header("Authorization", format!("Bearer {}", api_key))
            .json(&body)
            .send()
            .await?;

        if !response.status().is_success() {
            let error = response.text().await?;
            return Err(anyhow!("OpenAI API error: {}", error));
        }
        response
            .json()
            .await
            .map_err(|e| anyhow!("Parse error: {}", e))
    }
}

/// Task executor for running agent tasks (MongoDB version)
pub struct TaskExecutor {
    db: Arc<MongoDb>,
    task_manager: Arc<TaskManager>,
    mission_manager: Option<Arc<MissionManager>>,
    agent_service: Arc<AgentService>,
    security_scanner: PromptInjectionScanner,
    shell_warn_audit_cache: Arc<tokio::sync::Mutex<HashMap<String, Instant>>>,
    runtime_settings: TeamRuntimeSettings,
}

impl TaskExecutor {
    /// Normalize streaming chunks to true incremental deltas.
    ///
    /// Some providers emit strict deltas; others may emit cumulative text in later chunks.
    /// This function keeps only the incremental suffix relative to `accumulated`.
    fn extract_stream_delta(accumulated: &str, incoming: &str) -> String {
        if incoming.is_empty() {
            return String::new();
        }
        if accumulated.is_empty() {
            return incoming.to_string();
        }
        if accumulated.ends_with(incoming) {
            // Exact or trailing duplicate fragment.
            return String::new();
        }
        if let Some(suffix) = incoming.strip_prefix(accumulated) {
            // Cumulative chunk from provider; only append new suffix.
            return suffix.to_string();
        }

        // Handle partial overlap: suffix(accumulated) == prefix(incoming)
        let max_overlap = accumulated.len().min(incoming.len());
        for overlap in (1..=max_overlap).rev() {
            let acc_start = accumulated.len() - overlap;
            if !accumulated.is_char_boundary(acc_start) || !incoming.is_char_boundary(overlap) {
                continue;
            }
            if accumulated.get(acc_start..) == incoming.get(..overlap) {
                if let Some(suffix) = incoming.get(overlap..) {
                    return suffix.to_string();
                }
            }
        }

        incoming.to_string()
    }

    fn should_fallback_to_non_streaming(err: &ProviderError) -> bool {
        match err {
            ProviderError::NotImplemented(_) => true,
            ProviderError::RequestFailed(msg)
            | ProviderError::ServerError(msg)
            | ProviderError::ExecutionError(msg) => {
                let t = msg.to_lowercase();
                t.contains("stream decode error")
                    || t.contains("error decoding response body")
                    || t.contains("stream ended without producing a message")
                    || t.contains("unexpected eof")
                    || t.contains("connection reset")
                    || t.contains("connection closed")
                    || t.contains("connection aborted")
                    || t.contains("timed out")
            }
            _ => false,
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn fallback_to_complete_from_stream(
        &self,
        task_id: &str,
        provider: &Arc<dyn Provider>,
        system_prompt: &str,
        messages: &[Message],
        tools: &[rmcp::model::Tool],
        cancel_token: &CancellationToken,
        streamed_prefix: Option<&str>,
        reason: &str,
    ) -> Result<(Message, Option<ProviderUsage>)> {
        tracing::warn!(
            "Falling back to non-streaming complete() for task {}: {}",
            task_id,
            reason
        );
        self.task_manager
            .broadcast(
                task_id,
                StreamEvent::Status {
                    status: "llm_stream_fallback_complete".to_string(),
                },
            )
            .await;

        let fallback_timeout_secs = std::env::var("TEAM_PROVIDER_FALLBACK_COMPLETE_TIMEOUT_SECS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .filter(|v| *v > 0)
            .map(|v| v.min(MAX_FALLBACK_COMPLETE_TIMEOUT_SECS))
            .unwrap_or(DEFAULT_FALLBACK_COMPLETE_TIMEOUT_SECS);
        let fallback_timeout = Duration::from_secs(fallback_timeout_secs);

        let (msg, usage) = tokio::select! {
            res = tokio::time::timeout(
                fallback_timeout,
                provider.complete(system_prompt, messages, tools)
            ) => {
                match res {
                    Ok(provider_res) => provider_res.map_err(anyhow::Error::from)?,
                    Err(_) => {
                        return Err(anyhow!(
                            "transient provider execution blocked: fallback complete watchdog timed out after {}s",
                            fallback_timeout.as_secs()
                        ));
                    }
                }
            }
            _ = cancel_token.cancelled() => {
                return Err(anyhow!(
                    "transient provider execution blocked: fallback complete call cancelled"
                ));
            }
        };

        let full_text = msg.as_concat_text();
        if !full_text.is_empty() {
            let delta = match streamed_prefix {
                Some(prefix) if !prefix.is_empty() && full_text.starts_with(prefix) => {
                    full_text[prefix.len()..].to_string()
                }
                _ => full_text,
            };
            if !delta.is_empty() {
                self.task_manager
                    .broadcast(task_id, StreamEvent::Text { content: delta })
                    .await;
            }
        }
        // Keep reasoning visible in logs even when we had to fall back to complete().
        for part in &msg.content {
            if let MessageContent::Thinking(tc) = part {
                if !tc.thinking.is_empty() {
                    self.task_manager
                        .broadcast(
                            task_id,
                            StreamEvent::Thinking {
                                content: tc.thinking.clone(),
                            },
                        )
                        .await;
                }
            }
        }

        Ok((msg, Some(usage)))
    }

    /// Create a new task executor
    pub fn new(db: Arc<MongoDb>, task_manager: Arc<TaskManager>) -> Self {
        Self::new_with_mission_manager(db, task_manager, None)
    }

    pub fn new_with_mission_manager(
        db: Arc<MongoDb>,
        task_manager: Arc<TaskManager>,
        mission_manager: Option<Arc<MissionManager>>,
    ) -> Self {
        let agent_service = Arc::new(AgentService::new(db.clone()));
        let runtime_settings = TeamRuntimeSettings::from_env();
        tracing::info!(
            "TaskExecutor runtime settings: resource_mode={:?}, skill_mode={:?}, auto_extension_policy={:?}, auto_install_extensions={:?}, cache_root={}, workspace_root={}",
            runtime_settings.resource_mode,
            runtime_settings.skill_mode,
            runtime_settings.auto_extension_policy,
            runtime_settings.auto_install_extensions,
            runtime_settings.extension_cache_root,
            runtime_settings.workspace_root,
        );
        Self {
            db,
            task_manager,
            mission_manager,
            agent_service,
            security_scanner: PromptInjectionScanner::new(),
            shell_warn_audit_cache: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            runtime_settings,
        }
    }

    async fn should_emit_shell_warn_audit(
        &self,
        task_id: &str,
        tool_name: &str,
        explanation: &str,
    ) -> bool {
        const SHELL_WARN_AUDIT_WINDOW: Duration = Duration::from_secs(300);
        const SHELL_WARN_AUDIT_CACHE_LIMIT: usize = 512;

        let key = format!(
            "{}|{}|{}",
            task_id,
            tool_name.trim().to_ascii_lowercase(),
            explanation.trim().to_ascii_lowercase()
        );
        let now = Instant::now();
        let mut cache = self.shell_warn_audit_cache.lock().await;
        cache.retain(|_, seen_at| now.duration_since(*seen_at) < SHELL_WARN_AUDIT_WINDOW);
        if cache.contains_key(&key) {
            return false;
        }
        if cache.len() >= SHELL_WARN_AUDIT_CACHE_LIMIT {
            cache.retain(|_, seen_at| now.duration_since(*seen_at) < SHELL_WARN_AUDIT_WINDOW);
        }
        cache.insert(key, now);
        true
    }

    fn tasks(&self) -> mongodb::Collection<AgentTaskDoc> {
        self.db.collection("agent_tasks")
    }

    fn agents(&self) -> mongodb::Collection<TeamAgentDoc> {
        self.db.collection("team_agents")
    }

    fn teams(&self) -> mongodb::Collection<Team> {
        self.db.collection("teams")
    }

    fn results(&self) -> mongodb::Collection<Document> {
        self.db.collection("agent_task_results")
    }

    async fn get_team_shell_security_mode(&self, team_id: &str) -> ShellSecurityMode {
        let obj_id = match mongodb::bson::oid::ObjectId::parse_str(team_id) {
            Ok(id) => id,
            Err(_) => return ShellSecurityMode::default(),
        };
        match self.teams().find_one(doc! { "_id": obj_id }, None).await {
            Ok(Some(team)) => team.settings.shell_security.mode,
            Ok(None) => ShellSecurityMode::default(),
            Err(err) => {
                tracing::warn!(
                    "Failed to load team {} shell security mode, using default: {}",
                    team_id,
                    err
                );
                ShellSecurityMode::default()
            }
        }
    }

    fn extract_shell_scan_text(args: &serde_json::Value) -> String {
        let command = args
            .get("command")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        if command.is_empty() {
            return args.to_string();
        }
        Self::strip_heredoc_bodies(command)
    }

    fn strip_heredoc_bodies(command: &str) -> String {
        let mut result = Vec::new();
        let mut lines = command.lines();
        while let Some(line) = lines.next() {
            result.push(line);
            let trimmed = line.trim_start();
            let Some(marker_pos) = trimmed.find("<<") else {
                continue;
            };
            let marker_part = trimmed[(marker_pos + 2)..].trim_start();
            let marker = marker_part
                .trim_start_matches('-')
                .split_whitespace()
                .next()
                .unwrap_or("")
                .trim_matches('\'')
                .trim_matches('"');
            if marker.is_empty() {
                continue;
            }
            result.push("[HEREDOC_BODY_ELIDED]");
            for body_line in lines.by_ref() {
                if body_line.trim() == marker {
                    result.push(body_line);
                    break;
                }
            }
        }
        result.join("\n")
    }

    fn should_soften_shell_security_hit(command_text: &str, explanation: &str) -> bool {
        let explanation = explanation.to_ascii_lowercase();
        let command_text = command_text.to_ascii_lowercase();
        let explanation_is_common_false_positive = explanation
            .contains("unicode character obfuscation")
            || explanation.contains("nested command substitution");
        let looks_like_documentary_or_generated_content = command_text
            .contains("[heredoc_body_elided]")
            || command_text.contains("readme")
            || command_text.contains(".md")
            || command_text.contains(".html")
            || command_text.contains(".csv")
            || command_text.contains("markdown")
            || command_text.contains("deliverable")
            || command_text.contains("reports/final/quality")
            || command_text.contains("/quality/")
            || command_text.contains("- `")
            || command_text.contains("* `")
            || command_text.contains("1. `")
            || command_text.contains("目录")
            || command_text.contains("路径")
            || command_text.contains("说明")
            || command_text.contains("报告")
            || command_text.contains("质量")
            || command_text.contains("产出")
            || command_text.contains('`')
            || command_text.len() > 160;
        explanation_is_common_false_positive && looks_like_documentary_or_generated_content
    }

    /// Execute an approved task
    pub async fn execute_task(&self, task_id: &str, cancel_token: CancellationToken) -> Result<()> {
        // 1. Get task and agent info
        let task = self
            .get_task(task_id)
            .await?
            .ok_or_else(|| anyhow!("Task not found"))?;

        if task.status != TaskStatus::Approved {
            return Err(anyhow!("Task is not approved"));
        }

        let mut agent = self
            .get_agent(&task.agent_id)
            .await?
            .ok_or_else(|| anyhow!("Agent not found"))?;

        // Apply LLM overrides from task content (e.g. document analysis settings)
        if let Some(ov) = task.content.get("llm_overrides") {
            if let Some(u) = ov
                .get("api_url")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
            {
                agent.api_url = Some(u.to_string());
            }
            if let Some(k) = ov
                .get("api_key")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
            {
                agent.api_key = Some(k.to_string());
            }
            if let Some(m) = ov
                .get("model")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
            {
                agent.model = Some(m.to_string());
            }
            if let Some(f) = ov
                .get("api_format")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
            {
                if let Ok(fmt) = f.parse() {
                    agent.api_format = fmt;
                }
            }
        }

        // Debug: Log API key status
        tracing::info!(
            "Executing task {} with agent {}, api_format={:?}, api_key_present={}",
            task_id,
            agent.id,
            agent.api_format,
            agent.api_key.is_some()
        );

        // 2. Update task status to running
        self.update_task_status(task_id, TaskStatus::Running)
            .await?;

        // Broadcast status change
        self.task_manager
            .broadcast(
                task_id,
                StreamEvent::Status {
                    status: "running".to_string(),
                },
            )
            .await;

        // 3. Execute the task (cancel_token passed into the loop for fine-grained checking)
        let result = self.run_task(task_id, &task, &agent, &cancel_token).await;

        // 4. Update task status based on result
        match result {
            Ok(_) => {
                self.update_task_status(task_id, TaskStatus::Completed)
                    .await?;
                self.task_manager
                    .broadcast(
                        task_id,
                        StreamEvent::Done {
                            status: "completed".to_string(),
                            error: None,
                        },
                    )
                    .await;
            }
            Err(e) => {
                let error_msg = e.to_string();
                // Don't propagate DB errors here — must always reach broadcast + complete below
                if let Err(db_err) = self.update_task_error(task_id, &error_msg).await {
                    tracing::error!("Failed to persist task error for {}: {}", task_id, db_err);
                }
                self.task_manager
                    .broadcast(
                        task_id,
                        StreamEvent::Done {
                            status: "failed".to_string(),
                            error: Some(error_msg),
                        },
                    )
                    .await;
            }
        }

        // 5. Complete task in manager
        self.task_manager.complete(task_id).await;

        Ok(())
    }

    /// Run the actual task execution with multi-turn agent loop
    async fn run_task(
        &self,
        task_id: &str,
        task: &AgentTask,
        agent: &TeamAgent,
        cancel_token: &CancellationToken,
    ) -> Result<()> {
        let team_id_for_task = task.team_id.clone();
        let shell_security_mode = self.get_team_shell_security_mode(&team_id_for_task).await;
        let user_messages = task
            .content
            .get("messages")
            .and_then(|m| m.as_array())
            .ok_or_else(|| anyhow!("Invalid task content: missing messages"))?;
        let turn_system_instruction = task
            .content
            .get("turn_system_instruction")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());

        // Session management: load or create session
        let session_id = task
            .content
            .get("session_id")
            .and_then(|s| s.as_str())
            .map(|s| s.to_string());

        let (session, history_messages) = if let Some(ref sid) = session_id {
            // Load existing session
            match self.agent_service.get_session(sid).await {
                Ok(Some(sess)) => {
                    let msgs: Vec<Message> =
                        serde_json::from_str(&sess.messages_json).unwrap_or_default();
                    tracing::info!("Loaded session {} with {} messages", sid, msgs.len());
                    (Some(sess), msgs)
                }
                _ => {
                    tracing::warn!("Session {} not found, starting fresh", sid);
                    (None, Vec::new())
                }
            }
        } else {
            (None, Vec::new())
        };

        // Create session if none exists
        let session_id = if let Some(ref sess) = session {
            sess.session_id.clone()
        } else {
            let new_sess = self
                .agent_service
                .create_session(CreateSessionRequest {
                    team_id: task.team_id.clone(),
                    agent_id: task.agent_id.clone(),
                    user_id: task.submitter_id.clone(),
                    name: None,
                    attached_document_ids: Vec::new(),
                    extra_instructions: None,
                    allowed_extensions: None,
                    allowed_skill_ids: None,
                    retry_config: None,
                    max_turns: None,
                    tool_timeout_seconds: None,
                    max_portal_retry_rounds: None,
                    require_final_report: false,
                    portal_restricted: false,
                    document_access_mode: None,
                    session_source: Some("system".to_string()),
                    source_mission_id: None,
                    hidden_from_chat_list: Some(true),
                })
                .await
                .map_err(|e| anyhow!("Failed to create session: {}", e))?;
            let sid = new_sess.session_id.clone();
            tracing::info!("Created new session: {}", sid);
            sid
        };

        // Broadcast session_id to client
        self.task_manager
            .broadcast(
                task_id,
                StreamEvent::SessionId {
                    session_id: session_id.clone(),
                },
            )
            .await;

        // Build ApiCaller for MCP Sampling support
        let api_caller: Option<Arc<dyn ApiCaller>> = if agent.api_format != ApiFormat::Local {
            Some(Arc::new(AgentApiCaller {
                api_url: agent.api_url.clone(),
                api_key: agent.api_key.clone(),
                model: agent.model.clone(),
                api_format: agent.api_format,
            }))
        } else {
            None
        };

        let allowed_extension_names: Option<HashSet<String>> = session
            .as_ref()
            .and_then(|s| s.allowed_extensions.as_ref())
            .map(|items| {
                items
                    .iter()
                    .map(|s| s.trim().to_lowercase())
                    .filter(|s| !s.is_empty())
                    .collect::<HashSet<_>>()
            });
        let allowed_skill_ids: Option<HashSet<String>> = session
            .as_ref()
            .and_then(|s| s.allowed_skill_ids.as_ref())
            .map(|items| {
                items
                    .iter()
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect::<HashSet<_>>()
            });

        let mut platform_enabled_extensions = agent.enabled_extensions.clone();
        if let Some(allowed) = &allowed_extension_names {
            platform_enabled_extensions.retain(|cfg| {
                let runtime_name = cfg
                    .extension
                    .mcp_name()
                    .unwrap_or_else(|| cfg.extension.name());
                if cfg.extension == BuiltinExtension::Skills {
                    allowed.contains("skills") || allowed.contains("team_skills")
                } else {
                    allowed.contains(&runtime_name.to_lowercase())
                }
            });
        }

        let installer = ExtensionInstaller::new(
            self.db.clone(),
            self.runtime_settings.extension_cache_root.clone(),
            self.runtime_settings.auto_install_extensions,
        );

        // Connect to MCP extensions (builtin + custom + team shared).
        // Custom extensions from team source are normalized via installer first.
        let mut all_extensions = builtin_extensions_to_custom(agent);
        let resolved_custom = resolve_agent_custom_extensions(
            &self.db,
            &task.team_id,
            &agent.custom_extensions,
            &installer,
        )
        .await;
        all_extensions.extend(resolved_custom);

        // Merge team shared extensions (auto-discovery) when enabled.
        // Agent's own extensions take priority over team shared ones (skip duplicates by name).
        if self.runtime_settings.resource_mode == TeamResourceMode::Auto {
            let team_extensions = load_team_shared_extensions(
                &self.db,
                &task.team_id,
                self.runtime_settings.auto_extension_policy,
                &installer,
            )
            .await;
            if !team_extensions.is_empty() {
                let mut existing_names: HashSet<String> =
                    all_extensions.iter().map(|e| e.name.clone()).collect();
                for team_ext in team_extensions {
                    if !existing_names.contains(&team_ext.name) {
                        tracing::info!(
                            "Auto-discovered team extension '{}' for agent",
                            team_ext.name
                        );
                        existing_names.insert(team_ext.name.clone());
                        all_extensions.push(team_ext);
                    } else {
                        tracing::debug!(
                            "Skipping team extension '{}': already exists in agent config",
                            team_ext.name
                        );
                    }
                }
            }
        }

        // Apply session extension overrides (disabled/enabled)
        if let Some(ref sess) = session {
            if !sess.disabled_extensions.is_empty() {
                let disabled_set: HashSet<&str> = sess
                    .disabled_extensions
                    .iter()
                    .map(|s| s.as_str())
                    .collect();
                let before = all_extensions.len();
                all_extensions.retain(|e| !disabled_set.contains(e.name.as_str()));
                tracing::info!(
                    "Session extension overrides: disabled {} extensions ({} -> {})",
                    before - all_extensions.len(),
                    before,
                    all_extensions.len()
                );
            }
            // enabled_extensions: re-enable extensions from agent config that user
            // explicitly enabled during a previous message in this session
            if !sess.enabled_extensions.is_empty() {
                let existing_names: HashSet<String> =
                    all_extensions.iter().map(|e| e.name.clone()).collect();
                let mut added = 0usize;
                for enabled_name in &sess.enabled_extensions {
                    if existing_names.contains(enabled_name) {
                        continue; // already active
                    }
                    // Try to find in agent's disabled builtin extensions
                    if let Some(cfg) = find_extension_config_by_name(agent, enabled_name) {
                        tracing::info!(
                            "Re-enabling session extension override: '{}'",
                            enabled_name
                        );
                        all_extensions.push(cfg);
                        added += 1;
                    } else {
                        tracing::debug!(
                            "Session enabled extension '{}' not found in agent config, skipping",
                            enabled_name
                        );
                    }
                }
                if added > 0 {
                    tracing::info!(
                        "Session extension overrides: re-enabled {} extensions",
                        added
                    );
                }
            }
        }

        if let Some(allowed) = &allowed_extension_names {
            let before = all_extensions.len();
            all_extensions.retain(|ext| allowed.contains(&ext.name.to_lowercase()));
            tracing::info!(
                "Portal/session extension allowlist applied: {} -> {}",
                before,
                all_extensions.len()
            );
        }

        let workspace_path = task
            .content
            .get("workspace_path")
            .and_then(|s| s.as_str())
            .map(|s| s.to_string());
        let mission_id = task
            .content
            .get("mission_id")
            .and_then(|s| s.as_str())
            .map(|s| s.to_string());
        let session_attached_document_ids: Vec<String> = session
            .as_ref()
            .map(|s| s.attached_document_ids.clone())
            .unwrap_or_default();
        let session_doc_scope = if session_attached_document_ids.is_empty() {
            None
        } else {
            Some(session_attached_document_ids.as_slice())
        };
        let session_portal_restricted = session
            .as_ref()
            .map(|s| s.portal_restricted)
            .unwrap_or(false);
        let session_document_access_mode = session
            .as_ref()
            .and_then(|s| s.document_access_mode.as_deref());
        let force_portal_tools = session
            .as_ref()
            .map(|s| {
                s.session_source.eq_ignore_ascii_case("portal_coding")
                    || s.session_source.eq_ignore_ascii_case("portal_manager")
            })
            .unwrap_or(false);
        let actor_user_id = session
            .as_ref()
            .map(|s| s.user_id.as_str())
            .unwrap_or(task.submitter_id.as_str());

        let elicitation_bridge: ElicitationBridgeCallback = {
            let task_manager = Arc::clone(&self.task_manager);
            let task_id_for_bridge = task_id.to_string();
            Arc::new(move |event: ElicitationBridgeEvent| {
                let task_manager = Arc::clone(&task_manager);
                let task_id_for_bridge = task_id_for_bridge.clone();
                tokio::spawn(async move {
                    let mut detail = format!(
                        "MCP elicitation requested (type={}): {}",
                        event.request_type, event.message
                    );
                    if let Some(url) = event.url {
                        detail.push_str(&format!(" | url={}", url));
                    }
                    if let Some(elicitation_id) = event.elicitation_id {
                        detail.push_str(&format!(" | elicitation_id={}", elicitation_id));
                    }
                    task_manager
                        .broadcast(
                            &task_id_for_bridge,
                            StreamEvent::Status {
                                status: "mcp_elicitation_requested".to_string(),
                            },
                        )
                        .await;
                    task_manager
                        .broadcast(&task_id_for_bridge, StreamEvent::Text { content: detail })
                        .await;
                });
            })
        };

        let mcp = if !all_extensions.is_empty() {
            match McpConnector::connect(
                &all_extensions,
                api_caller.clone(),
                Some(elicitation_bridge),
                workspace_path.as_deref(),
            )
            .await
            {
                Ok(c) => {
                    tracing::info!("MCP connector ready, has_tools={}", c.has_tools());
                    Some(c)
                }
                Err(e) => {
                    tracing::warn!("Failed to connect MCP extensions: {}", e);
                    None
                }
            }
        } else {
            None
        };

        // Initialize platform extensions (Skills, Team, Todo, DocumentTools, PortalTools) in-process
        let platform = PlatformExtensionRunner::create(
            &platform_enabled_extensions,
            Some(self.db.clone()),
            Some(&task.team_id),
            Some(actor_user_id),
            session.as_ref().map(|s| s.session_source.as_str()),
            Some(session_id.as_str()),
            mission_id.as_deref(),
            Some(&agent.id),
            self.runtime_settings.skill_mode == TeamSkillMode::OnDemand,
            workspace_path.as_deref(),
            Some(&self.runtime_settings.workspace_root),
            Some(&self.runtime_settings.portal_base_url),
            allowed_extension_names.as_ref(),
            allowed_skill_ids.as_ref(),
            session_doc_scope,
            session_portal_restricted,
            session_document_access_mode,
            force_portal_tools,
            self.mission_manager.clone(),
        )
        .await;
        if platform.has_tools() {
            tracing::info!(
                "Platform extensions ready: {:?}",
                platform.extension_names()
            );
        }

        let has_tools = mcp.as_ref().is_some_and(|m| m.has_tools()) || platform.has_tools();

        // Export attached documents once (used by both Local API and Provider paths)
        let doc_section = match (&session, &workspace_path) {
            (Some(sess), Some(wp)) if !sess.attached_document_ids.is_empty() => {
                let injector = DocumentContextInjector::new((*self.db).clone());
                let exported = injector
                    .export_to_workspace(&task.team_id, &sess.attached_document_ids, wp)
                    .await;
                let section = DocumentContextInjector::format_as_prompt_section(&exported);
                if section.is_empty() {
                    None
                } else {
                    Some(section)
                }
            }
            (Some(sess), None) if !sess.attached_document_ids.is_empty() => {
                tracing::warn!(
                    "Session has attached documents but no workspace_path, skipping export"
                );
                None
            }
            _ => None,
        };

        // Local API: always single-turn (no Provider abstraction)
        if agent.api_format == ApiFormat::Local {
            let ext_infos = self.collect_extension_infos(mcp.as_ref(), &platform);
            let mut local_msgs =
                self.build_messages_with_system_prompt(agent, user_messages, &ext_infos);

            // Inject document context into system message
            if let Some(ref ds) = doc_section {
                if let Some(first) = local_msgs.first_mut() {
                    if let Some(content) = first.get("content").and_then(|c| c.as_str()) {
                        let updated = format!("{}{}", content, ds);
                        first["content"] = serde_json::Value::String(updated);
                    }
                }
            }

            // Inject session extra_instructions into system message
            if let Some(ref sess) = session {
                if let Some(ref extra) = sess.extra_instructions {
                    if !extra.trim().is_empty() {
                        if let Some(first) = local_msgs.first_mut() {
                            if let Some(content) = first.get("content").and_then(|c| c.as_str()) {
                                let updated = format!(
                                    "{}\n\n<extra_instructions>\n{}\n</extra_instructions>",
                                    content, extra
                                );
                                first["content"] = serde_json::Value::String(updated);
                            }
                        }
                    }
                }
            }
            if let Some(ref turn_instruction) = turn_system_instruction {
                if let Some(first) = local_msgs.first_mut() {
                    if let Some(content) = first.get("content").and_then(|c| c.as_str()) {
                        let updated = format!(
                            "{}\n\n<turn_system_instruction>\n{}\n</turn_system_instruction>",
                            content, turn_instruction
                        );
                        first["content"] = serde_json::Value::String(updated);
                    }
                }
            }

            let response = match self.call_local_api(agent, &local_msgs).await {
                Ok(r) => r,
                Err(e) => {
                    if let Some(m) = mcp {
                        m.shutdown().await;
                    }
                    return Err(e);
                }
            };
            let text = response["message"]["content"]
                .as_str()
                .unwrap_or("")
                .to_string();
            self.task_manager
                .broadcast(
                    task_id,
                    StreamEvent::Text {
                        content: text.clone(),
                    },
                )
                .await;
            self.save_task_result(task_id, TaskResultType::Message, &text)
                .await?;
            if let Some(m) = mcp {
                m.shutdown().await;
            }
            return Ok(());
        }

        // Create Provider via factory
        let provider = provider_factory::create_provider_for_agent(agent)?;

        // Extract mission context from task content (injected by execute_via_bridge)
        let mission_ctx: Option<MissionPromptContext> = task
            .content
            .get("mission_context")
            .and_then(|v| serde_json::from_value(v.clone()).ok());
        let mission_run_binding = mission_run_binding_from_task(task, mission_ctx.as_ref());
        let graph_task_node = if let Some(run_id) = mission_run_binding
            .as_ref()
            .map(|binding| binding.run_id.clone())
            .or_else(|| task_string_field(task, "run_id"))
        {
            match self.agent_service.get_run_state(&run_id).await {
                Ok(Some(run_state)) => {
                    if let Some(task_graph_id) = run_state.task_graph_id.as_deref() {
                        match self.agent_service.get_task_graph(task_graph_id).await {
                            Ok(Some(graph)) => {
                                let wanted_node_id = task
                                    .content
                                    .get("task_node_id")
                                    .and_then(serde_json::Value::as_str)
                                    .map(|value| value.to_string())
                                    .or_else(|| mission_run_binding.as_ref().map(|binding| binding.task_node_id.clone()))
                                    .or(run_state.current_node_id.clone());
                                wanted_node_id.and_then(|node_id| {
                                    graph.nodes
                                        .into_iter()
                                        .find(|node| node.task_node_id == node_id)
                                })
                            }
                            _ => None,
                        }
                    } else {
                        None
                    }
                }
                _ => None,
            }
        } else {
            None
        };
        let subagent_runtime =
            subagent_runtime_from_task(
                task,
                mission_ctx.as_ref(),
                mission_run_binding.as_ref(),
                graph_task_node.as_ref(),
            );
        let subagent_tool_enabled = should_enable_subagents(agent.model.as_deref().unwrap_or_default())
            && subagent_runtime
                .as_ref()
                .is_some_and(|ctx| ctx.depth < ctx.max_depth);

        // Build system prompt: core template + optional agent custom instructions
        let mut system_prompt = {
            let ext_infos = self.collect_extension_infos(mcp.as_ref(), &platform);
            let custom = agent
                .system_prompt
                .as_deref()
                .filter(|s| !s.trim().is_empty());
            build_system_prompt(
                &ext_infos,
                custom,
                mission_ctx.as_ref(),
                subagent_tool_enabled,
            )
        };

        // Inject attached document context into system prompt
        if let Some(ref ds) = doc_section {
            system_prompt.push_str(ds);
        }

        // Inject session extra_instructions (e.g. portal project path)
        if let Some(ref sess) = session {
            if let Some(ref extra) = sess.extra_instructions {
                if !extra.trim().is_empty() {
                    system_prompt.push_str("\n\n<extra_instructions>\n");
                    system_prompt.push_str(extra);
                    system_prompt.push_str("\n</extra_instructions>");
                }
            }
        }
        if let Some(ref turn_instruction) = turn_system_instruction {
            system_prompt.push_str("\n\n<turn_system_instruction>\n");
            system_prompt.push_str(turn_instruction);
            system_prompt.push_str("\n</turn_system_instruction>");
        }
        if let Some(ref task_node) = graph_task_node {
            system_prompt.push_str("\n\n<task_graph_node_contract>\n");
            system_prompt.push_str(&format!("task_node_id: {}\n", task_node.task_node_id));
            if let Some(title) = task_node.title.as_deref() {
                system_prompt.push_str(&format!("title: {}\n", title));
            }
            if !task_node.target_artifacts.is_empty() {
                system_prompt.push_str(&format!(
                    "target_artifacts: {}\n",
                    task_node.target_artifacts.join(", ")
                ));
            }
            if let Some(delegation_mode) = task_node.delegation_mode.clone() {
                let budget = task_node.parallelism_budget.unwrap_or(1).clamp(1, 3);
                match delegation_mode {
                    HarnessDelegationMode::Subagent => {
                        system_prompt.push_str(&format!(
                            "delegation_mode: subagent; delegation_budget: {}\n",
                            budget.min(1)
                        ));
                        system_prompt.push_str(
                            "Execution rule: this node authorizes one bounded helper subagent. Use the `subagent` tool only for a single isolated helper thread when it materially advances the current node. Do not fan out multiple workers.\n",
                        );
                    }
                    HarnessDelegationMode::Swarm => {
                        system_prompt.push_str(&format!(
                            "delegation_mode: swarm; swarm_mode: {:?}; parallelism_budget: {}\n",
                            task_node.swarm_mode, budget
                        ));
                        system_prompt.push_str(
                            "Execution rule: this node authorizes bounded swarm orchestration. Partition the work into coordinated worker threads only when parallel fan-out materially improves progress.\n",
                        );
                    }
                    HarnessDelegationMode::Disabled => {}
                }
            }
            if !task_node.write_scope.is_empty() {
                system_prompt.push_str(&format!(
                    "write_scope: {}\n",
                    task_node.write_scope.join(", ")
                ));
            }
            system_prompt.push_str("</task_graph_node_contract>");
        }
        if let Some(ref subagent_ctx) = subagent_runtime {
            if subagent_ctx.delegation_mode == HarnessDelegationMode::Swarm
                && subagent_ctx.depth < subagent_ctx.max_depth
            {
                system_prompt.push_str("\n\n<recursive_swarm_contract>\n");
                system_prompt.push_str(&format!(
                    "This task may use recursive subagents. Current depth: {} of {}.\n",
                    subagent_ctx.depth, subagent_ctx.max_depth
                ));
                if !subagent_ctx.write_scope.is_empty() {
                    system_prompt.push_str(&format!(
                        "Write scope for delegated work: {}\n",
                        subagent_ctx.write_scope.join(", ")
                    ));
                }
                system_prompt.push_str(
                    "If you delegate, keep each subagent bounded, reuse the declared write scope, and consume only the subagent's final summary.\n",
                );
                system_prompt.push_str("</recursive_swarm_contract>");
            }
        }

        let mut messages = self.build_provider_messages(user_messages);
        if !history_messages.is_empty() {
            let mut with_history = history_messages;
            with_history.extend(messages);
            messages = with_history;
        }

        // If no tools and no extension manager, single-turn via streaming
        let ext_manager_enabled = {
            let mut enabled = agent_has_extension_manager_enabled(agent);
            if let Some(ref sess) = session {
                if sess.portal_restricted {
                    enabled = false;
                }
            }
            if enabled {
                if let Some(allowed) = &allowed_extension_names {
                    enabled = allowed.contains("extension_manager");
                }
            }
            enabled
        };
        if !has_tools && !ext_manager_enabled {
            let (response_msg, _usage) = self
                .call_provider_streaming(
                    task_id,
                    &provider,
                    &system_prompt,
                    &messages,
                    &[],
                    cancel_token,
                )
                .await?;
            let text = response_msg.as_concat_text();
            self.save_task_result(task_id, TaskResultType::Message, &text)
                .await?;

            // Save session
            messages.push(response_msg);
            self.save_session_state(&session_id, &messages, 0, 0).await;

            if let Some(m) = mcp {
                m.shutdown().await;
            }
            return Ok(());
        }

        // Wrap MCP + platform in shared state for dynamic extension management
        let dynamic_state = Arc::new(RwLock::new(DynamicExtensionState {
            mcp,
            platform,
            agent: agent.clone(),
            api_caller: api_caller.clone(),
        }));

        // Create extension manager client if enabled (with session persistence)
        let ext_manager = if ext_manager_enabled {
            tracing::info!("ExtensionManager enabled for this agent");
            Some(TeamExtensionManagerClient::with_session(
                dynamic_state.clone(),
                session_id.clone(),
                self.agent_service.clone(),
                (*self.db).clone(),
            ))
        } else {
            None
        };

        // Multi-turn agent loop with Provider
        let portal_restricted = session
            .as_ref()
            .map(|s| s.portal_restricted)
            .unwrap_or(false);
        let session_retry_config = session.as_ref().and_then(|s| s.retry_config.clone());
        let session_require_final_report = session
            .as_ref()
            .map(|s| s.require_final_report)
            .unwrap_or(false);
        let session_max_turns = session
            .as_ref()
            .and_then(|s| s.max_turns)
            .and_then(|v| (v > 0).then_some((v as usize).min(MAX_UNIFIED_MAX_TURNS)));
        let session_tool_timeout_secs = session
            .as_ref()
            .and_then(|s| s.tool_timeout_seconds)
            .filter(|v| *v > 0)
            .map(|v| v.min(MAX_TOOL_TIMEOUT_SECS));
        let session_max_portal_retry_rounds = session
            .as_ref()
            .and_then(|s| s.max_portal_retry_rounds)
            .and_then(|v| (v > 0).then_some(v as usize));
        let result = self
            .run_unified_loop(
                task_id,
                &provider,
                &system_prompt,
                messages,
                mission_ctx.clone(),
                mission_run_binding.clone(),
                graph_task_node.clone(),
                subagent_runtime.clone(),
                dynamic_state.clone(),
                ext_manager.as_ref(),
                cancel_token,
                &session_id,
                portal_restricted,
                workspace_path.clone(),
                session_retry_config,
                session_require_final_report,
                session_max_turns,
                session_tool_timeout_secs,
                session_max_portal_retry_rounds,
                shell_security_mode,
            )
            .await;

        // Save extension state changes to session before shutdown
        if ext_manager_enabled {
            let state_read = dynamic_state.read().await;
            let mut active_names: Vec<String> = Vec::new();
            if let Some(ref mcp_conn) = state_read.mcp {
                active_names.extend(mcp_conn.extension_names());
            }
            active_names.extend(state_read.platform.extension_names());
            drop(state_read);

            let active_set: HashSet<String> = active_names.into_iter().collect();

            let overrides = super::runtime::compute_extension_overrides(agent, &active_set);

            if !overrides.disabled.is_empty() || !overrides.enabled.is_empty() {
                tracing::info!(
                    "Saving extension overrides to session {}: disabled={:?}, enabled={:?}",
                    session_id,
                    overrides.disabled,
                    overrides.enabled
                );
                if let Err(e) = self
                    .agent_service
                    .update_session_extensions(&session_id, &overrides.disabled, &overrides.enabled)
                    .await
                {
                    tracing::warn!("Failed to save extension overrides: {}", e);
                }
            }
        }

        // Shutdown MCP connections (take ownership via write lock to avoid leak
        // when Arc::try_unwrap fails due to lingering references)
        {
            let mcp = {
                let mut state = dynamic_state.write().await;
                state.mcp.take()
            };
            if let Some(m) = mcp {
                m.shutdown().await;
            }
        }

        result
    }

    /// Collect ExtensionInfo from both MCP and platform extensions.
    /// MCP extensions only have names (no instructions); platform extensions have real instructions.
    fn collect_extension_infos(
        &self,
        mcp: Option<&McpConnector>,
        platform: &PlatformExtensionRunner,
    ) -> Vec<ExtensionInfo> {
        let mut infos: Vec<ExtensionInfo> = mcp
            .map(|m| {
                m.extension_names()
                    .into_iter()
                    .map(|name| ExtensionInfo::new(&name, "", false))
                    .collect()
            })
            .unwrap_or_default();
        infos.extend(platform.extension_infos());
        // Sort alphabetically for prompt caching stability (same as local agent's prompt_manager)
        infos.sort_by(|a, b| a.name.cmp(&b.name));
        infos
    }

    /// Build messages with system prompt prepended
    fn build_messages_with_system_prompt(
        &self,
        agent: &TeamAgent,
        user_messages: &[serde_json::Value],
        extensions: &[ExtensionInfo],
    ) -> Vec<serde_json::Value> {
        let mut messages = Vec::new();

        // Build system prompt: core template + optional agent custom instructions
        let custom = agent
            .system_prompt
            .as_deref()
            .filter(|s| !s.trim().is_empty());
        let prompt = build_system_prompt(extensions, custom, None, false);
        messages.push(serde_json::json!({
            "role": "system",
            "content": prompt
        }));

        // Add user messages
        messages.extend(user_messages.iter().cloned());

        messages
    }

    /// Build Provider-compatible Message list from raw JSON user messages
    fn build_provider_messages(&self, user_messages: &[serde_json::Value]) -> Vec<Message> {
        user_messages
            .iter()
            .map(|msg| {
                let role = msg.get("role").and_then(|r| r.as_str()).unwrap_or("user");
                let content = msg.get("content").and_then(|c| c.as_str()).unwrap_or("");
                match role {
                    "assistant" => Message::assistant().with_text(content),
                    _ => Message::user().with_text(content),
                }
            })
            .collect()
    }

    /// Call local Ollama API (no tool calling support)
    async fn call_local_api(
        &self,
        agent: &TeamAgent,
        messages: &[serde_json::Value],
    ) -> Result<serde_json::Value> {
        let api_url = agent
            .api_url
            .as_deref()
            .unwrap_or("http://localhost:11434/api/chat");
        let model = agent.model.as_deref().unwrap_or("llama2");

        let client = build_http_client()?;
        let response = client
            .post(api_url)
            .header("Content-Type", "application/json")
            .json(&serde_json::json!({
                "model": model,
                "messages": messages,
                "stream": false,
            }))
            .send()
            .await?;

        if !response.status().is_success() {
            let error = response.text().await?;
            return Err(anyhow!("API error: {}", error));
        }

        let result: serde_json::Value = response.json().await?;
        Ok(result)
    }

    /// Convert tool content blocks to plain text (for SSE and truncation)
    fn tool_blocks_to_text(blocks: &[super::mcp_connector::ToolContentBlock]) -> String {
        blocks
            .iter()
            .map(|b| match b {
                super::mcp_connector::ToolContentBlock::Text(text) => text.clone(),
                super::mcp_connector::ToolContentBlock::Image { mime_type, .. } => {
                    format!("[Image: {}]", mime_type)
                }
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Truncate a string at a UTF-8 safe byte boundary.
    /// `max_bytes` is the maximum byte length; the result is trimmed back
    /// to the nearest char boundary if it falls mid-character.
    fn safe_truncate(s: &str, max_bytes: usize) -> &str {
        if s.len() <= max_bytes {
            return s;
        }
        let mut end = max_bytes;
        while end > 0 && !s.is_char_boundary(end) {
            end -= 1;
        }
        &s[..end]
    }

    /// Truncate tool result text if it exceeds the maximum allowed length
    fn truncate_tool_result(text: String) -> String {
        if text.len() > MAX_TOOL_RESULT_CHARS {
            let total = text.len();
            let truncated = Self::safe_truncate(&text, MAX_TOOL_RESULT_CHARS);
            format!(
                "{}\n[truncated: showing first {} of {} bytes]",
                truncated,
                truncated.len(),
                total
            )
        } else {
            text
        }
    }

    /// Heuristic: whether this tool call likely changed files in workspace.
    fn tool_may_change_workspace(tool_name: &str) -> bool {
        let name = tool_name.to_lowercase();
        if name == "shell" || name == "text_editor" {
            return true;
        }
        [
            "write",
            "edit",
            "replace",
            "insert",
            "undo",
            "apply_patch",
            "mkdir",
            "touch",
            "rm",
            "cp",
            "mv",
        ]
        .iter()
        .any(|k| name.contains(k))
    }

    /// Governance/portal write tools must run serially to avoid self-inflicted
    /// compare-and-swap conflicts on portal governance state.
    fn tool_requires_serial_write(tool_name: &str) -> bool {
        let name = tool_name.to_lowercase();
        matches!(
            name.as_str(),
            "avatar_governance__review_request"
                | "avatar_governance__submit_capability_request"
                | "avatar_governance__submit_gap_proposal"
                | "avatar_governance__submit_human_review_request"
                | "avatar_governance__submit_optimization_ticket"
                | "portal_tools__configure_portal_service_agent"
        )
    }

    async fn execute_standard_tool_call(
        dynamic_state: Arc<RwLock<DynamicExtensionState>>,
        task_manager: Arc<TaskManager>,
        task_id: String,
        cancel_token: CancellationToken,
        tool_timeout_secs: Option<u64>,
        name: String,
        args: serde_json::Value,
    ) -> (
        u64,
        Result<Vec<super::mcp_connector::ToolContentBlock>, String>,
    ) {
        let started_at = Instant::now();
        let result: Result<Vec<super::mcp_connector::ToolContentBlock>, String> =
            if let Some(timeout_secs) = tool_timeout_secs {
                match tokio::time::timeout(Duration::from_secs(timeout_secs), async {
                    let state = dynamic_state.read().await;
                    if state.platform.can_handle(&name) {
                        state.platform.call_tool_rich(&name, args).await
                    } else if let Some(ref m) = state.mcp {
                        let progress_task_id = task_id.clone();
                        let progress_mgr = task_manager.clone();
                        let progress_cb: ToolTaskProgressCallback = Arc::new(move |p| {
                            let payload = serde_json::json!({
                                "type": "tool_task_progress",
                                "tool_name": p.tool_name,
                                "server_name": p.server_name,
                                "task_id": p.task_id,
                                "status": p.status,
                                "status_message": p.status_message,
                                "poll_count": p.poll_count,
                            })
                            .to_string();
                            let tm = progress_mgr.clone();
                            let tid = progress_task_id.clone();
                            tokio::spawn(async move {
                                tm.broadcast(&tid, StreamEvent::Status { status: payload })
                                    .await;
                            });
                        });
                        m.call_tool_rich_with_progress(
                            &name,
                            args,
                            Some(progress_cb),
                            cancel_token.clone(),
                        )
                        .await
                    } else {
                        Err(anyhow!("No handler for tool: {}", name))
                    }
                })
                .await
                {
                    Ok(Ok(blocks)) => Ok(blocks),
                    Ok(Err(e)) => Err(format!("Error: {}", e)),
                    Err(_) => Err(format!(
                        "Error: tool '{}' timed out after {}s",
                        name, timeout_secs
                    )),
                }
            } else {
                let state = dynamic_state.read().await;
                if state.platform.can_handle(&name) {
                    state
                        .platform
                        .call_tool_rich(&name, args)
                        .await
                        .map_err(|e| format!("Error: {}", e))
                } else if let Some(ref m) = state.mcp {
                    let progress_task_id = task_id.clone();
                    let progress_mgr = task_manager.clone();
                    let progress_cb: ToolTaskProgressCallback = Arc::new(move |p| {
                        let payload = serde_json::json!({
                            "type": "tool_task_progress",
                            "tool_name": p.tool_name,
                            "server_name": p.server_name,
                            "task_id": p.task_id,
                            "status": p.status,
                            "status_message": p.status_message,
                            "poll_count": p.poll_count,
                        })
                        .to_string();
                        let tm = progress_mgr.clone();
                        let tid = progress_task_id.clone();
                        tokio::spawn(async move {
                            tm.broadcast(&tid, StreamEvent::Status { status: payload })
                                .await;
                        });
                    });
                    m.call_tool_rich_with_progress(
                        &name,
                        args,
                        Some(progress_cb),
                        cancel_token.clone(),
                    )
                    .await
                    .map_err(|e| format!("Error: {}", e))
                } else {
                    Err(format!("Error: No handler for tool: {}", name))
                }
            };
        let duration_ms = started_at.elapsed().as_millis() as u64;
        match result {
            Ok(blocks) => (duration_ms, Ok(blocks)),
            Err(err) => {
                tracing::warn!("{}", err);
                (duration_ms, Err(err))
            }
        }
    }

    async fn execute_team_subagent_call(
        db: Arc<MongoDb>,
        agent_service: Arc<AgentService>,
        task_manager: Arc<TaskManager>,
        mission_manager: Option<Arc<MissionManager>>,
        task_id: String,
        session_id: String,
        workspace_path: Option<String>,
        cancel_token: CancellationToken,
        mission_ctx: Option<MissionPromptContext>,
        mission_run_binding: Option<MissionRunBinding>,
        subagent_runtime: Option<SubagentRuntimeContext>,
        args: serde_json::Value,
    ) -> (
        u64,
        Result<Vec<super::mcp_connector::ToolContentBlock>, String>,
    ) {
        let started_at = Instant::now();
        let Some(subagent_runtime) = subagent_runtime else {
            return (
                started_at.elapsed().as_millis() as u64,
                Err("Error: subagent tool is not enabled for this task".to_string()),
            );
        };

        let next_depth = subagent_runtime.depth.saturating_add(1);
        if next_depth > subagent_runtime.max_depth {
            return (
                started_at.elapsed().as_millis() as u64,
                Err(format!(
                    "Error: recursive subagent depth {} exceeds max depth {}",
                    next_depth, subagent_runtime.max_depth
                )),
            );
        }

        let instructions = args
            .get("instructions")
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        let subrecipe = args
            .get("subrecipe")
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        let summary_only = args
            .get("summary")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(true);
        let requested_extensions = args
            .get("extensions")
            .and_then(serde_json::Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(serde_json::Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(|value| value.to_string())
                    .collect::<Vec<_>>()
            })
            .filter(|items| !items.is_empty());
        let explicit_write_scope = args
            .get("write_scope")
            .and_then(serde_json::Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(serde_json::Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(|value| value.to_string())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        let instructions = match (instructions, subrecipe.clone()) {
            (Some(text), Some(recipe)) => {
                format!("Subagent role `{}`.\n\n{}", recipe, text)
            }
            (Some(text), None) => text,
            (None, Some(recipe)) => {
                format!("Execute the bounded subagent role `{}` and return the result.", recipe)
            }
            (None, None) => {
                return (
                    started_at.elapsed().as_millis() as u64,
                    Err("Error: subagent call requires `instructions` or `subrecipe`".to_string()),
                );
            }
        };

        let task = match agent_service.get_task(&task_id).await {
            Ok(Some(task)) => task,
            Ok(None) => {
                return (
                    started_at.elapsed().as_millis() as u64,
                    Err(format!("Error: parent task {} not found", task_id)),
                );
            }
            Err(e) => {
                return (
                    started_at.elapsed().as_millis() as u64,
                    Err(format!("Error: failed to load parent task: {}", e)),
                );
            }
        };
        let session = match agent_service.get_session(&session_id).await {
            Ok(Some(session)) => session,
            Ok(None) => {
                return (
                    started_at.elapsed().as_millis() as u64,
                    Err(format!("Error: session {} not found", session_id)),
                );
            }
            Err(e) => {
                return (
                    started_at.elapsed().as_millis() as u64,
                    Err(format!("Error: failed to load session: {}", e)),
                );
            }
        };

        let effective_write_scope = runtime::constrain_subagent_write_scope(
            &subagent_runtime.write_scope,
            &explicit_write_scope,
        );
        let spec_name = subrecipe.unwrap_or_else(|| subagent_runtime.spec_name.clone());
        let subagent_run_id = Uuid::new_v4().to_string();
        let parent_run_id = subagent_runtime
            .parent_run_id
            .clone()
            .or_else(|| mission_run_binding.as_ref().map(|binding| binding.run_id.clone()))
            .unwrap_or_else(|| "missionless".to_string());
        let parent_task_node_id = subagent_runtime
            .parent_task_node_id
            .clone()
            .or_else(|| mission_run_binding.as_ref().map(|binding| binding.task_node_id.clone()));
        if !effective_write_scope.is_empty() {
            let existing = db
                .collection::<super::harness_core::SubagentRun>("agent_subagent_runs")
                .find_one(
                    doc! {
                        "parent_run_id": &parent_run_id,
                        "status": "executing",
                        "write_scope": bson::to_bson(&effective_write_scope)
                            .unwrap_or(bson::Bson::Null),
                    },
                    None,
                )
                .await;
            match existing {
                Ok(Some(_)) => {
                    return (
                        started_at.elapsed().as_millis() as u64,
                        Err(format!(
                            "Error: subagent write scope already active for parent run {} ({})",
                            parent_run_id,
                            effective_write_scope.join(", ")
                        )),
                    );
                }
                Ok(None) => {}
                Err(err) => {
                    return (
                        started_at.elapsed().as_millis() as u64,
                        Err(format!("Error: failed to inspect active subagent scopes: {}", err)),
                    );
                }
            }
        }

        let _ = agent_service
            .upsert_subagent_run(&super::harness_core::SubagentRun {
                id: None,
                subagent_run_id: subagent_run_id.clone(),
                parent_run_id: parent_run_id.clone(),
                mission_id: subagent_runtime
                    .source_mission_id
                    .clone()
                    .or_else(|| mission_run_binding.as_ref().map(|binding| binding.mission_id.clone())),
                parent_task_node_id: parent_task_node_id.clone(),
                spec_name: spec_name.clone(),
                status: RunStatus::Executing,
                write_scope: effective_write_scope.clone(),
                created_at: Some(mongodb::bson::DateTime::now()),
                updated_at: Some(mongodb::bson::DateTime::now()),
            })
            .await;
        let _ = agent_service
            .mark_run_subagent_started(&parent_run_id, &subagent_run_id)
            .await;

        if let Some(binding) = mission_run_binding.as_ref() {
            let _ = agent_service
                .save_run_checkpoint(&RunCheckpoint {
                    id: None,
                    run_id: binding.run_id.clone(),
                    mission_id: Some(binding.mission_id.clone()),
                    task_graph_id: Some(format!("mission:{}:{}", binding.mission_id, binding.run_id)),
                    current_node_id: Some(binding.task_node_id.clone()),
                    checkpoint_kind: RunCheckpointKind::SubagentFanOut,
                    status: RunStatus::Executing,
                    lease: None,
                    memory: mission_ctx
                        .as_ref()
                        .and_then(|ctx| ctx.progress_memory.as_ref())
                        .map(RunMemory::from),
                    last_turn_outcome: None,
                    created_at: Some(mongodb::bson::DateTime::now()),
                })
                .await;
        }

        let req = runtime::SubagentBridgeRequest {
            team_id: session.team_id.clone(),
            agent_id: task.agent_id.clone(),
            user_id: session.user_id.clone(),
            instructions: if summary_only {
                format!(
                    "{}\n\nReturn only a concise final summary with actions taken and resulting outputs.",
                    instructions
                )
            } else {
                instructions
            },
            cancel_token,
            workspace_path,
            source_mission_id: subagent_runtime
                .source_mission_id
                .clone()
                .or_else(|| mission_run_binding.as_ref().map(|binding| binding.mission_id.clone())),
            parent_run_id: Some(parent_run_id),
            parent_task_node_id,
            write_scope: effective_write_scope.clone(),
            spec_name: spec_name.clone(),
            subagent_depth: next_depth,
            subagent_max_depth: subagent_runtime.max_depth,
            allowed_extensions: requested_extensions.or_else(|| session.allowed_extensions.clone()),
        };

        let result = runtime::execute_subagent_via_bridge(
            &db,
            agent_service.as_ref(),
            &task_manager,
            mission_manager.clone(),
            req,
        )
        .await;

        let mission_id_for_hooks = subagent_runtime
            .source_mission_id
            .clone()
            .or_else(|| mission_run_binding.as_ref().map(|binding| binding.mission_id.clone()));
        let _ = hook_runtime::run_subagent_stop_hooks(
            agent_service.as_ref(),
            mission_id_for_hooks.as_deref(),
        )
        .await;

        let status = if result.is_ok() {
            RunStatus::Completed
        } else {
            RunStatus::Failed
        };
        let _ = agent_service
            .upsert_subagent_run(&super::harness_core::SubagentRun {
                id: None,
                subagent_run_id: subagent_run_id.clone(),
                parent_run_id: mission_run_binding
                    .as_ref()
                    .map(|binding| binding.run_id.clone())
                    .unwrap_or_else(|| "missionless".to_string()),
                mission_id: subagent_runtime
                    .source_mission_id
                    .clone()
                    .or_else(|| mission_run_binding.as_ref().map(|binding| binding.mission_id.clone())),
                parent_task_node_id: mission_run_binding
                    .as_ref()
                    .map(|binding| binding.task_node_id.clone()),
                spec_name: spec_name.clone(),
                status: status.clone(),
                write_scope: effective_write_scope.clone(),
                created_at: None,
                updated_at: Some(mongodb::bson::DateTime::now()),
            })
            .await;
        let _ = agent_service
            .mark_run_subagent_finished(
                mission_run_binding
                    .as_ref()
                    .map(|binding| binding.run_id.as_str())
                    .unwrap_or("missionless"),
                &subagent_run_id,
            )
            .await;

        if let Some(binding) = mission_run_binding.as_ref() {
            let _ = agent_service
                .save_run_checkpoint(&RunCheckpoint {
                    id: None,
                    run_id: binding.run_id.clone(),
                    mission_id: Some(binding.mission_id.clone()),
                    task_graph_id: Some(format!("mission:{}:{}", binding.mission_id, binding.run_id)),
                    current_node_id: Some(binding.task_node_id.clone()),
                    checkpoint_kind: RunCheckpointKind::SubagentFanIn,
                    status: status,
                    lease: None,
                    memory: mission_ctx
                        .as_ref()
                        .and_then(|ctx| ctx.progress_memory.as_ref())
                        .map(RunMemory::from),
                    last_turn_outcome: None,
                    created_at: Some(mongodb::bson::DateTime::now()),
                })
                .await;
        }

        let duration_ms = started_at.elapsed().as_millis() as u64;
        match result {
            Ok(output) => {
                let mut text = format!(
                    "Subagent `{}` completed.\nSession: {}\nTask: {}",
                    spec_name, output.session_id, output.task_id
                );
                if !output.summary.trim().is_empty() {
                    text.push_str("\n\n");
                    text.push_str(output.summary.trim());
                }
                (
                    duration_ms,
                    Ok(vec![super::mcp_connector::ToolContentBlock::Text(text)]),
                )
            }
            Err(e) => (duration_ms, Err(format!("Error: {}", e))),
        }
    }

    /// Return the latest user-authored text from the conversation buffer.
    fn latest_user_text(messages: &[Message]) -> String {
        messages
            .iter()
            .rev()
            .find(|m| m.role == rmcp::model::Role::User)
            .map(|m| m.as_concat_text())
            .unwrap_or_default()
    }

    /// Heuristic: whether user intent is coding/implementation for portal workspace.
    fn has_portal_coding_intent(user_text: &str) -> bool {
        let user_lower = user_text.to_lowercase();
        let coding_keywords = [
            "build",
            "create",
            "make",
            "implement",
            "update",
            "modify",
            "refactor",
            "fix",
            "html",
            "css",
            "javascript",
            "website",
            "代码",
            "页面",
            "网站",
            "修改",
            "创建",
            "实现",
            "修复",
            "重构",
        ];
        coding_keywords.iter().any(|k| user_lower.contains(k))
    }

    /// Heuristic: assistant reply looks like planning-only (no execution/output).
    fn assistant_looks_planning_only(assistant_text: &str) -> bool {
        let t = assistant_text.trim().to_lowercase();
        if t.is_empty() {
            return false;
        }
        // Only match when the response STARTS with planning phrases
        let planning_prefixes = [
            "let me",
            "i will",
            "i'll",
            "first,",
            "first ",
            "i need to",
            "先",
            "让我",
            "我先",
            "我来",
            "我需要",
            "我将",
        ];
        planning_prefixes.iter().any(|p| t.starts_with(p))
    }

    /// Heuristic: assistant explicitly claims completion/delivery.
    fn assistant_claims_completion(assistant_text: &str) -> bool {
        let assistant_lower = assistant_text.trim().to_lowercase();
        if assistant_lower.is_empty() {
            return false;
        }
        if Self::assistant_looks_planning_only(assistant_text) {
            return false;
        }
        let completion_keywords = [
            "done",
            "completed",
            "finished",
            "implemented",
            "created",
            "updated",
            "fixed",
            "ready",
            "已完成",
            "完成了",
            "已经完成",
            "已实现",
            "已创建",
            "已更新",
            "已修复",
            "可以访问",
            "可以测试",
        ];
        if completion_keywords
            .iter()
            .any(|k| assistant_lower.contains(k))
        {
            return true;
        }
        // Fallback: a substantive non-planning response likely indicates completion.
        assistant_lower.len() >= 120
    }

    /// Response schema used by `final_output` when portal sessions require
    /// a structured completion report.
    fn required_final_report_response() -> Response {
        Response {
            json_schema: Some(serde_json::json!({
                "type": "object",
                "properties": {
                    "summary": {
                        "type": "string",
                        "description": "Short summary of what was implemented."
                    },
                    "changed_files": {
                        "type": "array",
                        "items": { "type": "string" },
                        "minItems": 1,
                        "description": "Workspace-relative changed files."
                    },
                    "preview_url": {
                        "type": "string",
                        "minLength": 1,
                        "description": "Preview URL that can be opened for verification."
                    },
                    "verification": {
                        "type": "array",
                        "items": { "type": "string" },
                        "minItems": 1,
                        "description": "Executed verification steps and outcomes."
                    }
                },
                "required": ["changed_files", "preview_url", "verification"],
                "additionalProperties": true
            })),
        }
    }

    fn retry_timeout_seconds(cfg: &RetryConfig) -> u64 {
        cfg.timeout_seconds.unwrap_or(DEFAULT_RETRY_TIMEOUT_SECONDS)
    }

    fn on_failure_timeout_seconds(cfg: &RetryConfig) -> u64 {
        cfg.on_failure_timeout_seconds
            .unwrap_or(DEFAULT_ON_FAILURE_TIMEOUT_SECONDS)
    }

    fn tool_timeout_seconds() -> Option<u64> {
        std::env::var("TEAM_AGENT_TOOL_TIMEOUT_SECS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .filter(|v| *v > 0)
            .map(|v| v.min(MAX_TOOL_TIMEOUT_SECS))
    }

    fn unified_max_turns() -> Option<usize> {
        std::env::var("TEAM_AGENT_MAX_TURNS")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .filter(|v| *v > 0)
            .map(|v| v.min(MAX_UNIFIED_MAX_TURNS))
    }

    async fn execute_shell_command_in_workspace(
        command: &str,
        timeout: Duration,
        workspace_path: Option<&str>,
    ) -> Result<Output> {
        let mut cmd = if cfg!(target_os = "windows") {
            let mut cmd = Command::new("cmd");
            cmd.args(["/C", command]);
            cmd
        } else {
            let mut cmd = Command::new("sh");
            cmd.args(["-c", command]);
            cmd
        };
        if let Some(path) = workspace_path {
            cmd.current_dir(path);
        }
        configure_command_no_window(&mut cmd);
        let fut = async move {
            cmd.stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .stdin(Stdio::null())
                .kill_on_drop(true)
                .output()
                .await
                .map_err(anyhow::Error::from)
        };
        tokio::time::timeout(timeout, fut)
            .await
            .map_err(|_| anyhow!("Command timed out after {:?}", timeout))?
    }

    async fn run_success_checks(
        retry_config: &RetryConfig,
        workspace_path: Option<&str>,
    ) -> Result<()> {
        for check in &retry_config.checks {
            match check {
                SuccessCheck::Shell { command } => {
                    let timeout = Duration::from_secs(Self::retry_timeout_seconds(retry_config));
                    let output =
                        Self::execute_shell_command_in_workspace(command, timeout, workspace_path)
                            .await?;
                    if !output.status.success() {
                        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
                        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
                        return Err(anyhow!(
                            "success check failed: `{}` (status={:?}, stdout={}, stderr={})",
                            command,
                            output.status.code(),
                            stdout,
                            stderr
                        ));
                    }
                }
            }
        }
        Ok(())
    }

    async fn run_on_failure_command(
        retry_config: &RetryConfig,
        workspace_path: Option<&str>,
    ) -> Result<()> {
        let Some(on_failure) = retry_config.on_failure.as_ref() else {
            return Ok(());
        };
        if on_failure.trim().is_empty() {
            return Ok(());
        }
        let timeout = Duration::from_secs(Self::on_failure_timeout_seconds(retry_config));
        let output =
            Self::execute_shell_command_in_workspace(on_failure, timeout, workspace_path).await?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            return Err(anyhow!(
                "on_failure command failed: `{}` (status={:?}, stderr={})",
                on_failure,
                output.status.code(),
                stderr
            ));
        }
        Ok(())
    }

    /// In portal coding mode, decide whether to force a retry reminder when
    /// the model returned no tool calls.
    fn should_force_portal_tool_retry(latest_user_text: &str, assistant_text: &str) -> bool {
        let user_text = latest_user_text.trim();
        if user_text.is_empty() {
            return false;
        }

        let assistant_lower = assistant_text.trim().to_lowercase();
        let has_coding_intent = Self::has_portal_coding_intent(user_text);
        let looks_like_planning_only = Self::assistant_looks_planning_only(assistant_text);

        has_coding_intent && (looks_like_planning_only || assistant_lower.is_empty())
    }

    /// Heuristic: identify transient provider/runtime failures worth retrying.
    fn is_retryable_provider_error(err: &anyhow::Error) -> bool {
        if runtime::is_waiting_external_message(&err.to_string()) {
            return false;
        }
        if let Some(pe) = err.downcast_ref::<ProviderError>() {
            return match pe {
                ProviderError::RateLimitExceeded { .. } => false,
                ProviderError::ServerError(_) => true,
                ProviderError::RequestFailed(msg) => {
                    !Self::is_non_retryable_provider_request_text(msg)
                }
                _ => false,
            };
        }
        Self::is_transient_error_text(&err.to_string())
    }

    fn is_non_retryable_provider_request_text(message: &str) -> bool {
        let lower = message.to_ascii_lowercase();
        let direct_blockers = [
            "authentication token has been invalidated",
            "authentication failed",
            "401 unauthorized",
            "auth_unavailable",
            "no auth available",
            "provider credentials unavailable",
            "credentials unavailable",
            "no valid coding plan subscription",
            "valid coding plan subscription",
            "subscription has expired",
            "subscription expired",
            "subscription is not active",
            "billing account not active",
        ];
        direct_blockers
            .iter()
            .any(|pattern| lower.contains(pattern))
    }

    /// Heuristic for transient text errors coming from wrapped anyhow contexts.
    fn is_transient_error_text(err_text: &str) -> bool {
        let t = err_text.to_lowercase();
        let keywords = [
            "timeout",
            "timed out",
            "temporar",
            "connection reset",
            "connection aborted",
            "connection refused",
            "connection closed",
            "network",
            "eof",
            "stream ended without producing a message",
            "service unavailable",
            "too many requests",
            "429",
            "502",
            "503",
            "504",
        ];
        keywords.iter().any(|k| t.contains(k))
    }

    /// Exponential backoff for provider retry, honoring provider-suggested rate-limit delay.
    fn provider_retry_delay(err: &anyhow::Error, attempt: usize) -> Duration {
        if let Some(ProviderError::RateLimitExceeded {
            retry_delay: Some(delay),
            ..
        }) = err.downcast_ref::<ProviderError>()
        {
            return *delay;
        }
        let shift = attempt.saturating_sub(1).min(4) as u32;
        let millis = (1000u64.saturating_mul(1u64 << shift)).min(15_000);
        Duration::from_millis(millis)
    }

    /// Unified multi-turn agent loop using Provider abstraction.
    /// Uses streaming output, session persistence, and context compaction.
    /// Tools are rebuilt each turn to reflect dynamic extension changes.
    #[allow(clippy::too_many_arguments)]
    async fn run_unified_loop(
        &self,
        task_id: &str,
        provider: &Arc<dyn Provider>,
        system_prompt: &str,
        initial_messages: Vec<Message>,
        mission_ctx: Option<MissionPromptContext>,
        mission_run_binding: Option<MissionRunBinding>,
        graph_task_node: Option<TaskNode>,
        subagent_runtime: Option<SubagentRuntimeContext>,
        dynamic_state: Arc<RwLock<DynamicExtensionState>>,
        ext_manager: Option<&TeamExtensionManagerClient>,
        cancel_token: &CancellationToken,
        session_id: &str,
        portal_restricted: bool,
        workspace_path: Option<String>,
        retry_config: Option<RetryConfig>,
        require_final_report: bool,
        max_turns_override: Option<usize>,
        tool_timeout_secs_override: Option<u64>,
        max_portal_retry_rounds: Option<usize>,
        shell_security_mode: ShellSecurityMode,
    ) -> Result<()> {
        let compaction_mode = ContextCompactionStrategy::LegacySegmented;
        let max_turns = max_turns_override.or_else(Self::unified_max_turns);
        let tool_timeout_secs = tool_timeout_secs_override.or_else(Self::tool_timeout_seconds);
        let mut mission_ctx = mission_ctx;
        let mut subagent_runtime = subagent_runtime;
        let mut messages = initial_messages;
        let mut all_text = String::new();
        let mut repetition_detector = RepetitionDetector::new();
        let mut completed_due_to_max_turns = false;
        let mut portal_tool_retry_count: usize = 0;
        let mut provider_retry_count: usize = 0;
        const DEFAULT_MAX_PROVIDER_RETRIES: usize = 3;
        let max_provider_retries = retry_config
            .as_ref()
            .map(|cfg| cfg.max_retries.max(1) as usize)
            .unwrap_or(DEFAULT_MAX_PROVIDER_RETRIES)
            .min(8);
        const MISSION_NO_TOOL_RETRY_LIMIT: usize = 2;
        const MISSION_NO_DELTA_RETRY_LIMIT: usize = 2;
        let mut portal_successful_tool_calls: usize = 0;
        let mut mission_no_tool_retry_count: usize = 0;
        let mut mission_no_delta_retry_count: usize = 0;
        let mut previous_turn_had_tool_failure = false;
        let mut consecutive_tool_failure_turns: usize = 0;
        let mut subagent_bootstrap_attempted = false;
        let mut swarm_bootstrap_attempted = false;
        let mut swarm_downgraded = false;
        /// After this many consecutive turns where every tool call failed,
        /// inject a reflection prompt forcing the agent to change strategy.
        const CONSECUTIVE_FAILURE_REFLECTION_THRESHOLD: usize = 3;
        let mut accumulated_input: i32 = 0;
        let mut accumulated_output: i32 = 0;
        let mut last_compaction_turn: Option<usize> = None;
        /// Max recovery compaction attempts before giving up (same as local agent)
        const MAX_RECOVERY_COMPACTION_ATTEMPTS: i32 = 3;
        let mut recovery_compaction_attempts: i32 = 0;
        let mut base_system_prompt = system_prompt.to_string();
        let mut final_output_tool = if require_final_report {
            let tool = FinalOutputTool::new(Self::required_final_report_response());
            base_system_prompt.push_str("\n\n");
            base_system_prompt.push_str(&tool.system_prompt());
            Some(tool)
        } else {
            None
        };

        let mut turn: usize = 0;
        loop {
            // Reset effective_system_prompt each turn to avoid V2 memory accumulation
            let effective_system_prompt = base_system_prompt.clone();
            if let Some(limit) = max_turns {
                if turn >= limit {
                    completed_due_to_max_turns = true;
                    break;
                }
            }
            let turn = {
                let current = turn;
                turn = turn.saturating_add(1);
                current
            };
            // Check cancellation
            if cancel_token.is_cancelled() {
                tracing::info!("Unified loop cancelled at turn {}", turn + 1);
                break;
            }

            let contract_target_for_turn =
                effective_contract_target_file(mission_ctx.as_ref(), graph_task_node.as_ref());
            let workspace_before = if contract_target_for_turn.is_some() {
                workspace_path
                    .as_deref()
                    .and_then(|wp| runtime::snapshot_workspace_files(wp).ok())
            } else {
                None
            };

            let run_has_active_subagents = if let Some(binding) = mission_run_binding.as_ref() {
                self.agent_service
                    .get_run_state(&binding.run_id)
                    .await
                    .ok()
                    .flatten()
                    .is_some_and(|state| !state.active_subagents.is_empty())
            } else {
                false
            };

            if !subagent_bootstrap_attempted
                && turn == 0
                && !run_has_active_subagents
                && subagent_runtime
                    .as_ref()
                    .is_some_and(|ctx| {
                        ctx.delegation_mode == HarnessDelegationMode::Subagent
                            && ctx.depth == 0
                            && ctx.max_depth >= 1
                    })
                && mission_run_binding.is_some()
            {
                subagent_bootstrap_attempted = true;
                if let Some(subagent_ctx) = subagent_runtime.as_ref() {
                    let bootstrap_request = subagent_scheduler::SubagentBootstrapRequest {
                        goal: mission_ctx.as_ref().map(|ctx| ctx.goal.clone()),
                        context: mission_ctx.as_ref().and_then(|ctx| ctx.context.clone()),
                        locked_target: contract_target_for_turn.clone(),
                        missing_artifacts: mission_ctx
                            .as_ref()
                            .and_then(|ctx| ctx.progress_memory.as_ref())
                            .map(|memory| memory.missing.clone())
                            .unwrap_or_default(),
                        node_target_artifacts: graph_task_node
                            .as_ref()
                            .map(|node| node.target_artifacts.clone())
                            .unwrap_or_default(),
                        node_result_contract: graph_task_node
                            .as_ref()
                            .map(|node| node.result_contract.clone())
                            .unwrap_or_default(),
                        parent_write_scope: subagent_ctx.write_scope.clone(),
                    };
                    let call = subagent_scheduler::build_subagent_bootstrap_call(&bootstrap_request);
                    let (duration_ms, result) = Self::execute_team_subagent_call(
                        self.db.clone(),
                        self.agent_service.clone(),
                        self.task_manager.clone(),
                        self.mission_manager.clone(),
                        task_id.to_string(),
                        session_id.to_string(),
                        workspace_path.clone(),
                        cancel_token.clone(),
                        mission_ctx.clone(),
                        mission_run_binding.clone(),
                        subagent_runtime.clone(),
                        serde_json::json!({
                            "instructions": call.instructions,
                            "summary": true,
                            "subrecipe": call.spec_name,
                            "write_scope": call.write_scope,
                        }),
                    )
                    .await;

                    match result {
                        Ok(blocks) => {
                            let summary = Self::tool_blocks_to_text(&blocks);
                            messages.push(
                                Message::user()
                                    .with_text(format!(
                                        "[System] Automatic helper subagent completed.\n{}",
                                        summary
                                    ))
                                    .agent_only(),
                            );
                            if let (Some(before_snapshot), Some(workspace_root)) =
                                (workspace_before.as_ref(), workspace_path.as_deref())
                            {
                                if let Ok(after_snapshot) =
                                    runtime::snapshot_workspace_files(workspace_root)
                                {
                                    let mut hinted_paths =
                                        mission_bootstrap_missing_candidates(mission_ctx.as_ref());
                                    if let Some(target) = call.target_artifact.as_ref() {
                                        hinted_paths.push(target.clone());
                                    }
                                    hinted_paths.sort();
                                    hinted_paths.dedup();
                                    if let Some(changed_file) = workspace_any_candidate_file_changed(
                                        Some(before_snapshot),
                                        &after_snapshot,
                                        &hinted_paths,
                                    ) {
                                        apply_bootstrap_file_delta_to_mission_context(
                                            &mut mission_ctx,
                                            &changed_file,
                                        );
                                        if let Some(binding) = mission_run_binding.as_ref() {
                                            let artifact_step_index = mission_artifact_step_index(
                                                binding,
                                                mission_ctx.as_ref(),
                                            );
                                            let _ = runtime::reconcile_workspace_artifacts_with_hints(
                                                &self.agent_service,
                                                &binding.mission_id,
                                                artifact_step_index,
                                                workspace_root,
                                                Some(before_snapshot),
                                                &hinted_paths,
                                            )
                                            .await;
                                            let _ = self
                                                .agent_service
                                                .refresh_delivery_manifest_from_artifacts(
                                                    &binding.mission_id,
                                                )
                                                .await;
                                            let _ = self
                                                .agent_service
                                                .refresh_progress_memory(&binding.mission_id)
                                                .await;
                                        }
                                    }
                                }
                            }
                            self.task_manager
                                .broadcast(
                                    task_id,
                                    StreamEvent::ToolResult {
                                        id: "auto_subagent_bootstrap".to_string(),
                                        success: true,
                                        content: summary,
                                        name: Some("subagent".to_string()),
                                        duration_ms: Some(duration_ms),
                                    },
                                )
                                .await;
                            continue;
                        }
                        Err(err_text) => {
                            messages.push(
                                Message::user()
                                    .with_text(format!(
                                        "[System] {}",
                                        subagent_scheduler::build_subagent_downgrade_message(
                                            contract_target_for_turn.as_deref(),
                                            &mission_ctx
                                                .as_ref()
                                                .and_then(|ctx| ctx.progress_memory.as_ref())
                                                .map(|memory| memory.missing.clone())
                                                .unwrap_or_default(),
                                            &err_text,
                                        )
                                    ))
                                    .agent_only(),
                            );
                            self.task_manager
                                .broadcast(
                                    task_id,
                                    StreamEvent::ToolResult {
                                        id: "auto_subagent_bootstrap".to_string(),
                                        success: false,
                                        content: err_text,
                                        name: Some("subagent".to_string()),
                                        duration_ms: Some(duration_ms),
                                    },
                                )
                                .await;
                        }
                    }
                }
            }

            if !swarm_bootstrap_attempted
                && turn == 0
                && !run_has_active_subagents
                && subagent_runtime
                    .as_ref()
                    .is_some_and(|ctx| {
                        ctx.delegation_mode == HarnessDelegationMode::Swarm
                            && ctx.depth == 0
                            && ctx.max_depth > 0
                    })
                && mission_run_binding.is_some()
            {
                swarm_bootstrap_attempted = true;
                if let Some(subagent_ctx) = subagent_runtime.as_ref() {
                    let bootstrap_request = SwarmBootstrapRequest {
                        goal: mission_ctx.as_ref().map(|ctx| ctx.goal.clone()),
                        context: mission_ctx.as_ref().and_then(|ctx| ctx.context.clone()),
                        locked_target: contract_target_for_turn.clone(),
                        missing_artifacts: mission_ctx
                            .as_ref()
                            .and_then(|ctx| ctx.progress_memory.as_ref())
                            .map(|memory| memory.missing.clone())
                            .unwrap_or_default(),
                        node_target_artifacts: graph_task_node
                            .as_ref()
                            .map(|node| node.target_artifacts.clone())
                            .unwrap_or_default(),
                        node_result_contract: graph_task_node
                            .as_ref()
                            .map(|node| node.result_contract.clone())
                            .unwrap_or_default(),
                        parallelism_budget: graph_task_node
                            .as_ref()
                            .and_then(|node| node.parallelism_budget),
                        swarm_budget: graph_task_node.as_ref().and_then(|node| node.swarm_budget),
                        parent_write_scope: subagent_ctx.write_scope.clone(),
                    };
                    let bootstrap_calls =
                        swarm_scheduler::build_swarm_bootstrap_calls(&bootstrap_request);
                    let subagent_futures: Vec<_> = bootstrap_calls
                        .iter()
                        .cloned()
                        .map(|call| {
                            let db = self.db.clone();
                            let agent_service = self.agent_service.clone();
                            let task_manager = self.task_manager.clone();
                            let mission_manager = self.mission_manager.clone();
                            let task_id = task_id.to_string();
                            let session_id = session_id.to_string();
                            let workspace_path = workspace_path.clone();
                            let cancel_token = cancel_token.clone();
                            let mission_ctx = mission_ctx.clone();
                            let mission_run_binding = mission_run_binding.clone();
                            let subagent_runtime = subagent_runtime.clone();
                            async move {
                                Self::execute_team_subagent_call(
                                    db,
                                    agent_service,
                                    task_manager,
                                    mission_manager,
                                    task_id,
                                    session_id,
                                    workspace_path,
                                    cancel_token,
                                    mission_ctx,
                                    mission_run_binding,
                                    subagent_runtime,
                                    serde_json::json!({
                                        "instructions": call.instructions,
                                        "summary": true,
                                        "subrecipe": call.spec_name,
                                        "write_scope": call.write_scope,
                                    }),
                                )
                                .await
                            }
                        })
                        .collect();
                    let results = join_all(subagent_futures).await;
                    let total_duration_ms =
                        results.iter().map(|(duration_ms, _)| *duration_ms).sum::<u64>();
                    let executions = bootstrap_calls
                        .iter()
                        .zip(results.iter())
                        .map(|(call, (_, result))| SwarmBootstrapExecution {
                            target_artifact: call.target_artifact.clone(),
                            success: result.is_ok(),
                            summary: result
                                .as_ref()
                                .ok()
                                .map(|blocks| Self::tool_blocks_to_text(blocks))
                                .filter(|text| !text.trim().is_empty()),
                            error: result.as_ref().err().cloned(),
                        })
                        .collect::<Vec<_>>();
                    let summary = executions
                        .iter()
                        .filter_map(|execution| execution.summary.as_deref())
                        .map(str::trim)
                        .filter(|text| !text.is_empty())
                        .collect::<Vec<_>>()
                        .join("\n\n---\n\n");
                    let changed_bootstrap_targets = if let (
                        Some(before_snapshot),
                        Some(workspace_root),
                    ) = (
                        workspace_before.as_ref(),
                        workspace_path.as_deref(),
                    ) {
                        runtime::snapshot_workspace_files(workspace_root)
                            .ok()
                            .map(|after_snapshot| {
                                bootstrap_calls
                                    .iter()
                                    .filter_map(|call| call.target_artifact.as_deref())
                                    .filter(|target| {
                                        workspace_target_file_changed(
                                            Some(before_snapshot),
                                            &after_snapshot,
                                            target,
                                        )
                                    })
                                    .map(str::to_string)
                                    .collect::<Vec<_>>()
                            })
                            .unwrap_or_default()
                    } else {
                        Vec::new()
                    };
                    let decision = swarm_scheduler::decide_swarm_bootstrap_outcome(
                        &bootstrap_request,
                        &executions,
                        &changed_bootstrap_targets,
                    );
                    if executions.iter().any(|execution| execution.success) {
                        messages.push(
                            Message::user()
                                .with_text(format!(
                                    "[System] Automatic swarm bootstrap completed.\n{}",
                                    summary
                                ))
                                .agent_only(),
                        );
                        for changed_file in &decision.accepted_targets {
                            apply_bootstrap_file_delta_to_mission_context(&mut mission_ctx, changed_file);
                        }
                        let produced_file_delta = decision.produced_file_delta
                            || if let (
                                Some(before_snapshot),
                                Some(workspace_root),
                                Some(target_file),
                            ) = (
                                workspace_before.as_ref(),
                                workspace_path.as_deref(),
                                contract_target_for_turn.as_deref(),
                            ) {
                                runtime::snapshot_workspace_files(workspace_root)
                                    .ok()
                                    .is_some_and(|after_snapshot| {
                                        workspace_target_file_changed(
                                            Some(before_snapshot),
                                            &after_snapshot,
                                            target_file,
                                        )
                                    })
                            } else {
                                false
                            };
                        if let Some(binding) = mission_run_binding.as_ref() {
                            let outcome = TurnOutcome {
                                mode: mission_turn_mode(mission_ctx.as_ref()),
                                produced_file_delta,
                                produced_evidence_delta: false,
                                produced_blocker_delta: false,
                                tool_calls: executions.iter().filter(|execution| execution.success).count(),
                                success: true,
                                reason: Some("automatic_swarm_bootstrap".to_string()),
                            };
                            let memory = mission_ctx
                                .as_ref()
                                .and_then(|ctx| ctx.progress_memory.as_ref())
                                .map(RunMemory::from);
                            let _ = self
                                .agent_service
                                .append_run_journal(&[RunJournal {
                                    id: None,
                                    run_id: binding.run_id.clone(),
                                    mission_id: Some(binding.mission_id.clone()),
                                    task_node_id: binding.task_node_id.clone(),
                                    mode: outcome.mode.clone(),
                                    tool_calls: outcome.tool_calls,
                                    produced_file_delta: outcome.produced_file_delta,
                                    produced_evidence_delta: false,
                                    produced_blocker_delta: false,
                                    reason: outcome.reason.clone(),
                                    next_node_id: Some(binding.task_node_id.clone()),
                                    created_at: Some(bson::DateTime::now()),
                                }])
                                .await;
                            let _ = self
                                .agent_service
                                .patch_run_state_after_turn(
                                    &binding.run_id,
                                    &binding.task_node_id,
                                    mission_run_status_for_outcome(&outcome),
                                    memory.as_ref(),
                                    &outcome,
                                )
                                .await;
                            if outcome.produced_file_delta {
                                let _ = self
                                    .agent_service
                                    .save_run_checkpoint(&RunCheckpoint {
                                        id: None,
                                        run_id: binding.run_id.clone(),
                                        mission_id: Some(binding.mission_id.clone()),
                                        task_graph_id: Some(format!(
                                            "mission:{}:{}",
                                            binding.mission_id, binding.run_id
                                        )),
                                        current_node_id: Some(binding.task_node_id.clone()),
                                        checkpoint_kind: RunCheckpointKind::NodeSuccess,
                                        status: mission_run_status_for_outcome(&outcome),
                                        lease: None,
                                        memory,
                                        last_turn_outcome: Some(outcome),
                                        created_at: Some(bson::DateTime::now()),
                                    })
                                    .await;
                            }
                        }
                        if produced_file_delta {
                            mission_no_tool_retry_count = 0;
                            mission_no_delta_retry_count = 0;
                            self.task_manager
                                .broadcast(
                                    task_id,
                                    StreamEvent::ToolResult {
                                        id: "auto_swarm_bootstrap".to_string(),
                                        success: true,
                                        content: if summary.len() > 2000 {
                                            format!("{}...", Self::safe_truncate(&summary, 2000))
                                        } else {
                                            summary
                                        },
                                        name: Some("subagent".to_string()),
                                        duration_ms: Some(total_duration_ms),
                                    },
                                )
                                .await;
                            continue;
                        }
                    }
                    if let Some(reminder) = decision.downgrade_message {
                        subagent_runtime = None;
                        swarm_downgraded = true;
                        messages.push(
                            Message::user()
                                .with_text(format!("[System] {}", reminder))
                                .agent_only(),
                        );
                    }
                }
            }

            // Refresh MCP tool cache before rebuilding tools each turn.
            // This keeps dynamic MCP tool lists aligned with list_changed notifications / TTL.
            {
                let mut state = dynamic_state.write().await;
                if let Some(mcp) = state.mcp.as_mut() {
                    mcp.refresh_tools_if_stale().await;
                }
            }

            // Rebuild tools each turn to reflect dynamic extension changes
            let tools = {
                let state = dynamic_state.read().await;
                let mut t = state
                    .mcp
                    .as_ref()
                    .map(|m| m.tools_as_rmcp())
                    .unwrap_or_default();
                t.extend(state.platform.tools_as_rmcp());
                if ext_manager.is_some() {
                    t.extend(TeamExtensionManagerClient::tools_as_rmcp());
                }
                if !swarm_downgraded
                    && subagent_runtime
                    .as_ref()
                    .is_some_and(|ctx| ctx.depth < ctx.max_depth)
                    && !t.iter().any(|tool| tool.name.as_ref() == "subagent")
                {
                    t.push(create_subagent_tool(&[]));
                }
                if let Some(ref final_tool) = final_output_tool {
                    t.push(final_tool.tool());
                }
                t
            }; // read lock released here

            // Context compaction check (skip first turn)
            if turn > 0 {
                if let Ok((threshold_hit, before_tokens, ratio)) = self
                    .check_compaction_needed(provider, &effective_system_prompt, &messages, &tools)
                    .await
                {
                    if threshold_hit && Self::should_compact_now(turn, ratio, last_compaction_turn)
                    {
                        let conversation = Conversation::new_unvalidated(messages.clone());
                        match compact_messages_with_strategy(
                            provider.as_ref(),
                            &conversation,
                            false,
                            compaction_mode,
                        )
                        .await
                        {
                            Ok((compacted, _usage)) => {
                                messages = compacted.messages().to_vec();
                                // Recount actual tokens after compaction (usage.total_tokens is often None)
                                let after_tokens = match create_token_counter().await {
                                    Ok(counter) => counter.count_chat_tokens(
                                        &effective_system_prompt,
                                        &messages,
                                        &tools,
                                    ),
                                    Err(_) => 0,
                                };
                                self.task_manager
                                    .broadcast(
                                        task_id,
                                        StreamEvent::Compaction {
                                            strategy: SERVER_COMPACTION_MODE.to_string(),
                                            before_tokens,
                                            after_tokens,
                                        },
                                    )
                                    .await;
                                let _ = self
                                    .agent_service
                                    .increment_compaction_count(session_id)
                                    .await;
                                last_compaction_turn = Some(turn);
                                tracing::info!(
                                    "Compaction done: {} -> {} tokens",
                                    before_tokens,
                                    after_tokens
                                );
                            }
                            Err(e) => {
                                tracing::warn!("Compaction failed: {}, continuing", e);
                            }
                        }
                    } else if threshold_hit {
                        tracing::debug!(
                            "Compaction deferred by hysteresis: turn={}, ratio={:.3}, last_compaction_turn={:?}",
                            turn + 1,
                            ratio,
                            last_compaction_turn
                        );
                    }
                }
            }

            if let Some(limit) = max_turns {
                tracing::info!("Unified agent loop turn {}/{}", turn + 1, limit);
            } else {
                tracing::info!("Unified agent loop turn {}/unlimited", turn + 1);
            }
            self.task_manager
                .broadcast(
                    task_id,
                    StreamEvent::Turn {
                        current: turn + 1,
                        max: max_turns.unwrap_or(0),
                    },
                )
                .await;

            // Inject MOIM (Message of Immediate Memory) from platform extensions.
            // Inserted before the last assistant message (same as local agent in moim.rs).
            // Uses a temporary copy so the original messages list is not modified.
            let moim = {
                let state = dynamic_state.read().await;
                state.platform.collect_moim().await
            };
            let messages_for_llm = if let Some(moim) = moim {
                let mut tmp = messages.clone();
                let idx = tmp
                    .iter()
                    .rposition(|m| m.role == rmcp::model::Role::Assistant)
                    .unwrap_or(0);
                tmp.insert(idx, Message::user().with_text(moim));
                tmp
            } else {
                messages.clone()
            };

            // Fix conversation format before sending to LLM (same as local agent).
            // Ensures first/last messages are from user, merges consecutive same-role messages, etc.
            let conversation_for_llm = Conversation::new_unvalidated(messages_for_llm);
            let (fixed_conversation, fix_issues) = fix_conversation(conversation_for_llm);
            if !fix_issues.is_empty() {
                tracing::debug!("Conversation fixes applied: {:?}", fix_issues);
            }
            let messages_for_llm = fixed_conversation.messages().to_vec();

            // Call LLM via streaming Provider, with ContextLengthExceeded recovery
            let call_result = self
                .call_provider_streaming(
                    task_id,
                    provider,
                    &effective_system_prompt,
                    &messages_for_llm,
                    &tools,
                    cancel_token,
                )
                .await;

            let (response_msg, usage) = match call_result {
                Ok(result) => {
                    provider_retry_count = 0;
                    result
                }
                Err(e) => {
                    // Check if this is a ContextLengthExceeded error (recovery compaction)
                    if e.downcast_ref::<ProviderError>()
                        .is_some_and(|pe| matches!(pe, ProviderError::ContextLengthExceeded(_)))
                    {
                        if recovery_compaction_attempts >= MAX_RECOVERY_COMPACTION_ATTEMPTS {
                            tracing::error!(
                                "Exceeded max recovery compaction attempts ({})",
                                MAX_RECOVERY_COMPACTION_ATTEMPTS
                            );
                            self.task_manager
                                .broadcast(
                                    task_id,
                                    StreamEvent::Text {
                                        content: format!(
                                            "Context limit exceeded after {} compaction attempts. Please start a new session.",
                                            MAX_RECOVERY_COMPACTION_ATTEMPTS
                                        ),
                                    },
                                )
                                .await;
                            // Save session state before breaking so context is not lost
                            self.save_session_state(
                                session_id,
                                &messages,
                                accumulated_input,
                                accumulated_output,
                            )
                            .await;
                            break;
                        }

                        recovery_compaction_attempts += 1;
                        tracing::info!(
                            "Recovery compaction attempt {}/{} due to ContextLengthExceeded",
                            recovery_compaction_attempts,
                            MAX_RECOVERY_COMPACTION_ATTEMPTS
                        );

                        // Perform recovery compaction
                        let before_tokens = match create_token_counter().await {
                            Ok(counter) => counter.count_chat_tokens(
                                &effective_system_prompt,
                                &messages,
                                &tools,
                            ),
                            Err(_) => 0,
                        };
                        let conversation = Conversation::new_unvalidated(messages.clone());
                        match compact_messages_with_strategy(
                            provider.as_ref(),
                            &conversation,
                            false,
                            compaction_mode,
                        )
                        .await
                        {
                            Ok((compacted, _usage)) => {
                                messages = compacted.messages().to_vec();
                                // Recount actual tokens after recovery compaction
                                let after_tokens = match create_token_counter().await {
                                    Ok(counter) => counter.count_chat_tokens(
                                        &effective_system_prompt,
                                        &messages,
                                        &tools,
                                    ) as i32,
                                    Err(_) => 0,
                                };
                                self.task_manager
                                    .broadcast(
                                        task_id,
                                        StreamEvent::Compaction {
                                            strategy: SERVER_COMPACTION_MODE.to_string(),
                                            before_tokens,
                                            after_tokens: after_tokens as usize,
                                        },
                                    )
                                    .await;
                                last_compaction_turn = Some(turn);
                                tracing::info!("Recovery compaction done, retrying LLM call");
                                continue; // Retry the turn
                            }
                            Err(compact_err) => {
                                tracing::error!("Recovery compaction failed: {}", compact_err);
                                return Err(anyhow!(
                                    "Context length exceeded and compaction failed: {}",
                                    compact_err
                                ));
                            }
                        }
                    }
                    if Self::is_retryable_provider_error(&e)
                        && provider_retry_count < max_provider_retries
                    {
                        provider_retry_count += 1;
                        let delay = Self::provider_retry_delay(&e, provider_retry_count);
                        tracing::warn!(
                            "Transient provider failure on turn {}, retrying ({}/{}), backoff {:?}: {}",
                            turn + 1,
                            provider_retry_count,
                            max_provider_retries,
                            delay,
                            e
                        );
                        self.task_manager
                            .broadcast(
                                task_id,
                                StreamEvent::Status {
                                    status: "llm_retry".to_string(),
                                },
                            )
                            .await;
                        tokio::select! {
                            _ = tokio::time::sleep(delay) => {},
                            _ = cancel_token.cancelled() => {
                                return Err(anyhow!("Task cancelled during llm retry backoff"));
                            }
                        }
                        continue;
                    }
                    // Not a ContextLengthExceeded error, propagate
                    return Err(e);
                }
            };

            // Accumulate token stats
            if let Some(ref u) = usage {
                accumulated_input += u.usage.input_tokens.unwrap_or(0);
                accumulated_output += u.usage.output_tokens.unwrap_or(0);
            }

            // Extract text and tool requests from response.
            // Text/thinking are already streamed via call_provider_streaming (or fallback helper).
            let mut tool_requests: Vec<(String, String, serde_json::Value)> = Vec::new();
            let latest_user_text = if portal_restricted {
                Self::latest_user_text(&messages)
            } else {
                String::new()
            };
            let assistant_text = response_msg.as_concat_text();
            for content in &response_msg.content {
                match content {
                    MessageContent::Text(tc) => {
                        if !tc.text.is_empty() {
                            all_text.push_str(&tc.text);
                        }
                    }
                    MessageContent::ToolRequest(req) => {
                        if let Ok(ref call) = req.tool_call {
                            let name = call.name.to_string();
                            let args = serde_json::to_value(&call.arguments)
                                .unwrap_or(serde_json::json!({}));
                            self.task_manager
                                .broadcast(
                                    task_id,
                                    StreamEvent::ToolCall {
                                        name: name.clone(),
                                        id: req.id.clone(),
                                    },
                                )
                                .await;
                            tool_requests.push((req.id.clone(), name, args));
                        }
                    }
                    MessageContent::Thinking(_tc) => {}
                    _ => {}
                }
            }

            // Append assistant message to conversation
            messages.push(response_msg);

            // If no tool calls, we're done
            if tool_requests.is_empty() {
                if portal_restricted {
                    let max_portal_tool_retries = max_portal_retry_rounds;
                    let has_coding_intent =
                        require_final_report || Self::has_portal_coding_intent(&latest_user_text);
                    let completion_claimed = Self::assistant_claims_completion(&assistant_text);
                    let final_output_collected = final_output_tool
                        .as_ref()
                        .and_then(|tool| tool.final_output.as_ref())
                        .is_some();
                    let has_success_checks = retry_config
                        .as_ref()
                        .map(|cfg| !cfg.checks.is_empty())
                        .unwrap_or(false);
                    let missing_execution = has_coding_intent && portal_successful_tool_calls == 0;
                    let missing_completion_signal = has_coding_intent
                        && !completion_claimed
                        && !(require_final_report && final_output_collected);
                    let missing_final_report =
                        has_coding_intent && require_final_report && !final_output_collected;
                    let missing_success_checks_config = has_coding_intent && !has_success_checks;
                    let mut success_check_failure: Option<String> = None;
                    if has_coding_intent
                        && completion_claimed
                        && !missing_execution
                        && !missing_final_report
                        && !missing_success_checks_config
                    {
                        if let Some(cfg) = retry_config.as_ref() {
                            self.task_manager
                                .broadcast(
                                    task_id,
                                    StreamEvent::Status {
                                        status: "completion_check".to_string(),
                                    },
                                )
                                .await;
                            if workspace_path.is_none() {
                                success_check_failure = Some(
                                    "workspace path missing for retry success checks".to_string(),
                                );
                            } else if let Err(e) =
                                Self::run_success_checks(cfg, workspace_path.as_deref()).await
                            {
                                success_check_failure = Some(e.to_string());
                            }
                        }
                    }
                    let should_force_retry =
                        Self::should_force_portal_tool_retry(&latest_user_text, &assistant_text)
                            || previous_turn_had_tool_failure
                            || missing_execution
                            || missing_completion_signal
                            || missing_final_report
                            || missing_success_checks_config
                            || success_check_failure.is_some();

                    if should_force_retry {
                        let can_retry = max_portal_tool_retries
                            .map(|max| portal_tool_retry_count < max)
                            .unwrap_or(true);
                        if can_retry {
                            portal_tool_retry_count = portal_tool_retry_count.saturating_add(1);
                            let attempt = portal_tool_retry_count;
                            let retry_limit_label = max_portal_tool_retries
                                .map(|v| v.to_string())
                                .unwrap_or_else(|| "unlimited".to_string());
                            let (failure_code, failure_reason) = if let Some(ref err) =
                                success_check_failure
                            {
                                (
                                    "success_checks_failed",
                                    format!("success checks failed: {}", err),
                                )
                            } else if missing_success_checks_config {
                                (
                                    "missing_success_checks_config",
                                    "retry.success_checks is required for portal coding tasks"
                                        .to_string(),
                                )
                            } else if missing_final_report {
                                (
                                    "missing_final_report",
                                    format!(
                                        "missing required final report (prefer final_output tool). {}",
                                        FINAL_OUTPUT_CONTINUATION_MESSAGE
                                    ),
                                )
                            } else if previous_turn_had_tool_failure {
                                (
                                    "previous_tool_failure",
                                    "previous tool execution failed".to_string(),
                                )
                            } else if missing_execution {
                                (
                                    "missing_execution",
                                    "no successful developer tool execution observed".to_string(),
                                )
                            } else if missing_completion_signal {
                                (
                                    "missing_completion_signal",
                                    "assistant did not explicitly report completion".to_string(),
                                )
                            } else {
                                (
                                    "no_actionable_output",
                                    "model produced no actionable output".to_string(),
                                )
                            };
                            tracing::warn!(
                                "Portal coding guard triggered: no tool call on turn {}, code={}, reason={}, injecting retry reminder ({}/{})",
                                turn + 1,
                                failure_code,
                                failure_reason,
                                attempt,
                                retry_limit_label
                            );
                            self.task_manager
                                .broadcast(
                                    task_id,
                                    StreamEvent::Status {
                                        status: "portal_tool_retry".to_string(),
                                    },
                                )
                                .await;

                            if let Some(cfg) = retry_config.as_ref() {
                                self.task_manager
                                    .broadcast(
                                        task_id,
                                        StreamEvent::Status {
                                            status: "on_failure".to_string(),
                                        },
                                    )
                                    .await;
                                if let Err(e) =
                                    Self::run_on_failure_command(cfg, workspace_path.as_deref())
                                        .await
                                {
                                    tracing::warn!("Portal on_failure command failed: {}", e);
                                }
                            }

                            let reminder = format!(
                                "Portal coding mode retry ({}/{}): {}. Continue implementation with concrete developer tool calls. \
If coding work is complete, provide a structured final report with: 1) changed files, 2) preview URL, 3) verification steps/results. {}",
                                attempt,
                                retry_limit_label,
                                failure_reason,
                                if missing_final_report {
                                    FINAL_OUTPUT_CONTINUATION_MESSAGE
                                } else {
                                    ""
                                }
                            );

                            // Agent-only reminder: user will not see this synthetic message.
                            messages.push(Message::user().with_text(reminder).agent_only());
                            continue;
                        }

                        let retry_limit = max_portal_tool_retries.unwrap_or(0);
                        let (reason_code, reason) = if let Some(err) = success_check_failure {
                            (
                                "success_checks_failed",
                                format!("success checks failed: {}", err),
                            )
                        } else if missing_success_checks_config {
                            (
                                "missing_success_checks_config",
                                "retry.success_checks is required for portal coding tasks"
                                    .to_string(),
                            )
                        } else if missing_final_report {
                            (
                                "missing_final_report",
                                "missing required final report".to_string(),
                            )
                        } else if previous_turn_had_tool_failure {
                            (
                                "previous_tool_failure",
                                "previous tool execution failed and the agent stopped without recovery"
                                    .to_string(),
                            )
                        } else if missing_execution {
                            (
                                "missing_execution",
                                "no successful developer tool execution was observed".to_string(),
                            )
                        } else if missing_completion_signal {
                            (
                                "missing_completion_signal",
                                "assistant did not provide an explicit completion result"
                                    .to_string(),
                            )
                        } else {
                            (
                                "no_actionable_output",
                                "model produced no actionable output".to_string(),
                            )
                        };
                        let structured_reason = serde_json::json!({
                            "type": "portal_task_incomplete",
                            "reason_code": reason_code,
                            "reason": reason,
                            "turn": turn + 1,
                            "max_retries": retry_limit,
                            "require_final_report": require_final_report,
                            "success_checks_required": true,
                            "success_checks_present": has_success_checks,
                            "successful_tool_calls": portal_successful_tool_calls
                        });
                        tracing::error!(
                            "Portal task marked incomplete after {} retries: {}",
                            retry_limit,
                            structured_reason
                        );
                        self.task_manager
                            .broadcast(
                                task_id,
                                StreamEvent::Status {
                                    status: "portal_incomplete".to_string(),
                                },
                            )
                            .await;
                        let warning = format!("[Portal task incomplete] {}", structured_reason);
                        all_text.push_str(&warning);
                        self.task_manager
                            .broadcast(task_id, StreamEvent::Text { content: warning })
                            .await;
                        return Err(anyhow!("Portal task incomplete: {}", structured_reason));
                    }
                }

                if require_final_report {
                    if let Some(final_output_text) = final_output_tool
                        .as_ref()
                        .and_then(|tool| tool.final_output.clone())
                    {
                        if assistant_text.trim().is_empty() {
                            self.task_manager
                                .broadcast(
                                    task_id,
                                    StreamEvent::Text {
                                        content: final_output_text.clone(),
                                    },
                                )
                                .await;
                            all_text.push_str(&final_output_text);
                            messages.push(Message::assistant().with_text(final_output_text));
                        }
                    }
                }

                if contract_target_for_turn.is_some() {
                    if let Some(target_file) = contract_target_for_turn.clone() {
                        if mission_no_tool_retry_count < MISSION_NO_TOOL_RETRY_LIMIT {
                            mission_no_tool_retry_count =
                                mission_no_tool_retry_count.saturating_add(1);
                            let reminder = format!(
                                "Mission execution retry ({}/{}): this round ended without any tool call. You must create or materially update `{}` in the next round. Reuse the strongest completed outputs as inputs when possible. If `{}` truly cannot be produced because of environment or tooling limits, save the strongest directly reusable blocked/handoff file allowed by the mission instead of ending with analysis only.",
                                mission_no_tool_retry_count,
                                MISSION_NO_TOOL_RETRY_LIMIT,
                                target_file,
                                target_file
                            );
                            messages.push(Message::user().with_text(reminder).agent_only());
                            if let Some(binding) = mission_run_binding.as_ref() {
                                let outcome = TurnOutcome {
                                    mode: mission_turn_mode(mission_ctx.as_ref()),
                                    produced_file_delta: false,
                                    produced_evidence_delta: false,
                                    produced_blocker_delta: false,
                                    tool_calls: 0,
                                    success: false,
                                    reason: Some(format!("no_tool_calls_retry: {}", target_file)),
                                };
                                let memory = mission_ctx
                                    .as_ref()
                                    .and_then(|ctx| ctx.progress_memory.as_ref())
                                    .map(RunMemory::from);
                                let _ = self
                                    .agent_service
                                    .append_run_journal(&[RunJournal {
                                        id: None,
                                        run_id: binding.run_id.clone(),
                                        mission_id: Some(binding.mission_id.clone()),
                                        task_node_id: binding.task_node_id.clone(),
                                        mode: outcome.mode.clone(),
                                        tool_calls: outcome.tool_calls,
                                        produced_file_delta: false,
                                        produced_evidence_delta: false,
                                        produced_blocker_delta: false,
                                        reason: outcome.reason.clone(),
                                        next_node_id: Some(binding.task_node_id.clone()),
                                        created_at: Some(bson::DateTime::now()),
                                    }])
                                    .await;
                                let _ = self
                                    .agent_service
                                    .patch_run_state_after_turn(
                                        &binding.run_id,
                                        &binding.task_node_id,
                                        mission_run_status_for_outcome(&outcome),
                                        memory.as_ref(),
                                        &outcome,
                                    )
                                    .await;
                            }
                            continue;
                        }
                        return Err(anyhow!(
                            "Mission execution produced no tool calls after {} retries while locked target file {} still required",
                            MISSION_NO_TOOL_RETRY_LIMIT,
                            target_file
                        ));
                    }
                }

                tracing::debug!("Unified loop ended: no tool calls at turn {}", turn + 1);
                if let Some(binding) = mission_run_binding.as_ref() {
                    let outcome = TurnOutcome {
                        mode: mission_turn_mode(mission_ctx.as_ref()),
                        produced_file_delta: false,
                        produced_evidence_delta: false,
                        produced_blocker_delta: false,
                        tool_calls: 0,
                        success: true,
                        reason: Some("turn_ended_without_tool_calls".to_string()),
                    };
                    let memory = mission_ctx
                        .as_ref()
                        .and_then(|ctx| ctx.progress_memory.as_ref())
                        .map(RunMemory::from);
                    let _ = self
                        .agent_service
                        .append_run_journal(&[RunJournal {
                            id: None,
                            run_id: binding.run_id.clone(),
                            mission_id: Some(binding.mission_id.clone()),
                            task_node_id: binding.task_node_id.clone(),
                            mode: outcome.mode.clone(),
                            tool_calls: 0,
                            produced_file_delta: false,
                            produced_evidence_delta: false,
                            produced_blocker_delta: false,
                            reason: outcome.reason.clone(),
                            next_node_id: Some(binding.task_node_id.clone()),
                            created_at: Some(bson::DateTime::now()),
                        }])
                        .await;
                    let _ = self
                        .agent_service
                        .patch_run_state_after_turn(
                            &binding.run_id,
                            &binding.task_node_id,
                            mission_run_status_for_outcome(&outcome),
                            memory.as_ref(),
                            &outcome,
                        )
                        .await;
                }
                break;
            }

            mission_no_tool_retry_count = 0;
            if portal_restricted {
                // Reset no-tool retry counter once model resumes actual tool execution.
                portal_tool_retry_count = 0;
            }

            // Track tool id -> name for richer runtime events (timeline/file updates)
            let tool_name_by_id: HashMap<String, String> = tool_requests
                .iter()
                .map(|(id, name, _)| (id.clone(), name.clone()))
                .collect();

            self.task_manager
                .broadcast(
                    task_id,
                    StreamEvent::Status {
                        status: "tool_execution".to_string(),
                    },
                )
                .await;

            // Check for repeated tool calls
            if cancel_token.is_cancelled() {
                return Err(anyhow!("Task cancelled during tool execution"));
            }

            let mut allowed = Vec::new();
            let mut denied: Vec<(String, String, String)> = Vec::new(); // (id, name, reason)
            for (id, name, args) in &tool_requests {
                if repetition_detector.check(name, args) {
                    allowed.push((id.clone(), name.clone(), args.clone()));
                } else {
                    let threshold =
                        RepetitionDetector::repetition_threshold_for_tool(name).unwrap_or_default();
                    tracing::warn!("Repeated tool call denied: {}", name);
                    denied.push((
                        id.clone(),
                        name.clone(),
                        format!(
                            "Tool call denied: repeated identical call reached the safety threshold ({threshold}). Try a different approach."
                        ),
                    ));
                }
            }

            // Security scan: only check command-execution tools for dangerous patterns.
            // File-write tools (editors, file creators) contain code/markup that triggers
            // false positives on shell-oriented pattern rules.
            let mut security_allowed = Vec::new();
            let shell_keywords = ["shell", "bash", "cmd", "exec", "terminal", "run_command"];
            for (id, name, args) in allowed {
                let name_lower = name.to_lowercase();
                let is_shell_tool = shell_keywords.iter().any(|kw| name_lower.contains(kw));
                if !is_shell_tool {
                    security_allowed.push((id, name, args));
                    continue;
                }
                if shell_security_mode == ShellSecurityMode::Off {
                    security_allowed.push((id, name, args));
                    continue;
                }
                let tool_text = format!("Tool: {}\n{}", name, Self::extract_shell_scan_text(&args));
                match self
                    .security_scanner
                    .scan_for_dangerous_patterns(&tool_text)
                    .await
                {
                    Ok(scan) if scan.is_malicious && scan.confidence >= 0.7 => {
                        if Self::should_soften_shell_security_hit(&tool_text, &scan.explanation) {
                            tracing::debug!(
                                "Security audit: softened shell-like tool '{}' hit (confidence={:.2}) because it looks like documentary/generated content: {}",
                                name,
                                scan.confidence,
                                scan.explanation
                            );
                            security_allowed.push((id, name, args));
                            continue;
                        }
                        match shell_security_mode {
                            ShellSecurityMode::Warn => {
                                if self
                                    .should_emit_shell_warn_audit(task_id, &name, &scan.explanation)
                                    .await
                                {
                                    tracing::warn!(
                                        "Security audit: allowed shell-like tool '{}' under warn mode (confidence={:.2}): {}",
                                        name,
                                        scan.confidence,
                                        scan.explanation
                                    );
                                } else {
                                    tracing::debug!(
                                        "Security audit deduped for tool '{}' under warn mode: {}",
                                        name,
                                        scan.explanation
                                    );
                                }
                                security_allowed.push((id, name, args));
                            }
                            ShellSecurityMode::Block => {
                                tracing::warn!(
                                    "Security: blocked tool '{}' (confidence={:.2}): {}",
                                    name,
                                    scan.confidence,
                                    scan.explanation
                                );
                                let mut reason = format!(
                                    "Tool call blocked by security scanner: {}",
                                    scan.explanation
                                );
                                if scan.explanation.contains("Password file access")
                                    && tool_text.contains(".env")
                                {
                                    reason.push_str(
                                    " Do not print `.env` via shell. If the values are already known, write or update the file directly. Prefer inspecting non-secret templates such as `.env.example` instead.",
                                );
                                }
                                denied.push((id, name, reason));
                            }
                            ShellSecurityMode::Off => {
                                security_allowed.push((id, name, args));
                            }
                        }
                    }
                    _ => {
                        security_allowed.push((id, name, args));
                    }
                }
            }
            let mut hook_allowed = Vec::new();
            let effective_tool_write_scope = subagent_runtime
                .as_ref()
                .map(|ctx| ctx.write_scope.as_slice())
                .filter(|scope| !scope.is_empty())
                .or_else(|| {
                    graph_task_node
                        .as_ref()
                        .map(|node| node.write_scope.as_slice())
                        .filter(|scope| !scope.is_empty())
                });
            for (id, name, args) in security_allowed {
                match hook_runtime::apply_pre_tool_use_hooks(
                    &name,
                    &args,
                    effective_tool_write_scope,
                ) {
                    Ok(adjusted) => hook_allowed.push((id, name, adjusted)),
                    Err(reason) => denied.push((id, name, reason)),
                }
            }
            let allowed = hook_allowed;

            // Split tool calls by execution mode.
            // - final_output is handled in-process (stateful, serial)
            // - ExtensionManager tools are serial (write lock needed)
            // - governance / portal write tools are serial (avoid CAS conflicts)
            // - remaining tools run concurrently
            let mut final_output_calls: Vec<(String, String, serde_json::Value)> = Vec::new();
            let mut subagent_calls: Vec<(String, String, serde_json::Value)> = Vec::new();
            let mut ext_mgr_calls: Vec<(String, String, serde_json::Value)> = Vec::new();
            let mut serial_write_calls: Vec<(String, String, serde_json::Value)> = Vec::new();
            let mut regular_calls: Vec<(String, String, serde_json::Value)> = Vec::new();
            for (id, name, args) in &allowed {
                if final_output_tool.is_some() && name == FINAL_OUTPUT_TOOL_NAME {
                    final_output_calls.push((id.clone(), name.clone(), args.clone()));
                } else if name == "subagent" {
                    subagent_calls.push((id.clone(), name.clone(), args.clone()));
                } else if TeamExtensionManagerClient::can_handle(name) {
                    ext_mgr_calls.push((id.clone(), name.clone(), args.clone()));
                } else if Self::tool_requires_serial_write(name) {
                    serial_write_calls.push((id.clone(), name.clone(), args.clone()));
                } else {
                    regular_calls.push((id.clone(), name.clone(), args.clone()));
                }
            }

            // Execute final_output calls serially first.
            let mut final_output_results: Vec<(
                String,
                u64,
                Result<Vec<super::mcp_connector::ToolContentBlock>, String>,
            )> = Vec::new();
            for (id, name, args) in &final_output_calls {
                let started_at = Instant::now();
                let result: Result<Vec<super::mcp_connector::ToolContentBlock>, String> =
                    if let Some(timeout_secs) = tool_timeout_secs {
                        match tokio::time::timeout(Duration::from_secs(timeout_secs), async {
                            let Some(tool) = final_output_tool.as_mut() else {
                                return Err("Final output tool is not enabled".to_string());
                            };
                            let tool_call = rmcp::model::CallToolRequestParams {
                                name: name.clone().into(),
                                arguments: args.as_object().cloned(),
                                meta: None,
                                task: None,
                            };
                            let tool_result = tool.execute_tool_call(tool_call).await.result.await;
                            match tool_result {
                                Ok(call_result) => Ok(
                                    super::mcp_connector::McpConnector::extract_tool_result_blocks(
                                        &call_result,
                                    ),
                                ),
                                Err(e) => Err(format!("Error: {}", e)),
                            }
                        })
                        .await
                        {
                            Ok(outcome) => outcome,
                            Err(_) => Err(format!(
                                "Error: tool '{}' timed out after {}s",
                                name, timeout_secs
                            )),
                        }
                    } else {
                        match final_output_tool.as_mut() {
                            Some(tool) => {
                                let tool_call = rmcp::model::CallToolRequestParams {
                                    name: name.clone().into(),
                                    arguments: args.as_object().cloned(),
                                    meta: None,
                                    task: None,
                                };
                                let tool_result =
                                    tool.execute_tool_call(tool_call).await.result.await;
                                match tool_result {
                                    Ok(call_result) => Ok(
                                        super::mcp_connector::McpConnector::extract_tool_result_blocks(
                                            &call_result,
                                        ),
                                    ),
                                    Err(e) => Err(format!("Error: {}", e)),
                                }
                            }
                            None => Err("Final output tool is not enabled".to_string()),
                        }
                    };
                let duration_ms = started_at.elapsed().as_millis() as u64;
                match result {
                    Ok(outcome) => {
                        final_output_results.push((id.clone(), duration_ms, Ok(outcome)))
                    }
                    Err(err) => {
                        tracing::warn!("{}", err);
                        final_output_results.push((id.clone(), duration_ms, Err(err)));
                    }
                }
            }

            // Execute ExtensionManager calls serially first (no lock held by caller)
            let mut ext_mgr_results: Vec<(
                String,
                u64,
                Result<Vec<super::mcp_connector::ToolContentBlock>, String>,
            )> = Vec::new();
            for (id, name, args) in &ext_mgr_calls {
                let started_at = Instant::now();
                if let Some(em) = ext_manager {
                    let result: Result<Vec<super::mcp_connector::ToolContentBlock>, String> =
                        if let Some(timeout_secs) = tool_timeout_secs {
                            match tokio::time::timeout(
                                Duration::from_secs(timeout_secs),
                                em.call_tool_rich(name, args.clone()),
                            )
                            .await
                            {
                                Ok(Ok(blocks)) => Ok(blocks),
                                Ok(Err(e)) => Err(format!("Error: {}", e)),
                                Err(_) => Err(format!(
                                    "Error: tool '{}' timed out after {}s",
                                    name, timeout_secs
                                )),
                            }
                        } else {
                            match em.call_tool_rich(name, args.clone()).await {
                                Ok(blocks) => Ok(blocks),
                                Err(e) => Err(format!("Error: {}", e)),
                            }
                        };
                    let duration_ms = started_at.elapsed().as_millis() as u64;
                    match result {
                        Ok(blocks) => ext_mgr_results.push((id.clone(), duration_ms, Ok(blocks))),
                        Err(err) => {
                            tracing::warn!("{}", err);
                            ext_mgr_results.push((id.clone(), duration_ms, Err(err)));
                        }
                    }
                } else {
                    let duration_ms = started_at.elapsed().as_millis() as u64;
                    ext_mgr_results.push((
                        id.clone(),
                        duration_ms,
                        Err("ExtensionManager not enabled".to_string()),
                    ));
                }
            }

            // Execute governance/portal write tools serially.
            let mut serial_write_results: Vec<(
                String,
                u64,
                Result<Vec<super::mcp_connector::ToolContentBlock>, String>,
            )> = Vec::new();
            for (id, name, args) in &serial_write_calls {
                let (duration_ms, result) = tokio::select! {
                    res = Self::execute_standard_tool_call(
                        dynamic_state.clone(),
                        self.task_manager.clone(),
                        task_id.to_string(),
                        cancel_token.clone(),
                        tool_timeout_secs,
                        name.clone(),
                        args.clone(),
                    ) => res,
                    _ = cancel_token.cancelled() => {
                        return Err(anyhow!("Task cancelled during tool execution"));
                    }
                };
                serial_write_results.push((id.clone(), duration_ms, result));
            }

            // Execute subagent calls concurrently.
            let subagent_futures: Vec<_> = subagent_calls
                .iter()
                .map(|(id, _name, args)| {
                    let id = id.clone();
                    let db = self.db.clone();
                    let agent_service = self.agent_service.clone();
                    let task_manager = self.task_manager.clone();
                    let mission_manager = self.mission_manager.clone();
                    let task_id = task_id.to_string();
                    let session_id = session_id.to_string();
                    let workspace_path = workspace_path.clone();
                    let cancel_token = cancel_token.clone();
                    let mission_ctx = mission_ctx.clone();
                    let mission_run_binding = mission_run_binding.clone();
                    let subagent_runtime = subagent_runtime.clone();
                    let args = args.clone();
                    async move {
                        let result = Self::execute_team_subagent_call(
                            db,
                            agent_service,
                            task_manager,
                            mission_manager,
                            task_id,
                            session_id,
                            workspace_path,
                            cancel_token,
                            mission_ctx,
                            mission_run_binding,
                            subagent_runtime,
                            args,
                        )
                        .await;
                        (id, result.0, result.1)
                    }
                })
                .collect();

            let subagent_results = tokio::select! {
                res = join_all(subagent_futures) => res,
                _ = cancel_token.cancelled() => {
                    return Err(anyhow!("Task cancelled during subagent execution"));
                }
            };

            // Execute regular tools concurrently (with read lock)
            let ds = dynamic_state.clone();
            let futures: Vec<_> = regular_calls
                .iter()
                .map(|(id, name, args)| {
                    let id = id.clone();
                    let name = name.clone();
                    let args = args.clone();
                    let ds = ds.clone();
                    let ct = cancel_token.clone();
                    let task_id = task_id.to_string();
                    let task_manager = self.task_manager.clone();
                    async move {
                        let (duration_ms, result) = Self::execute_standard_tool_call(
                            ds,
                            task_manager,
                            task_id,
                            ct,
                            tool_timeout_secs,
                            name,
                            args,
                        )
                        .await;
                        (id, duration_ms, result)
                    }
                })
                .collect();

            let regular_results = tokio::select! {
                res = join_all(futures) => res,
                _ = cancel_token.cancelled() => {
                    return Err(anyhow!("Task cancelled during tool execution"));
                }
            };

            // Merge final_output, ExtensionManager, serial write, and regular results
            let mut results = ext_mgr_results;
            results.extend(final_output_results);
            results.extend(subagent_results);
            results.extend(serial_write_results);
            results.extend(regular_results);

            // Build tool response message
            let mut tool_response_msg = Message::user();
            let mut this_turn_had_tool_failure = !denied.is_empty();
            let mut this_turn_had_tool_success = false;
            let mut delta_retry_reminder: Option<String> = None;
            let mut swarm_failure_reminder: Option<String> = None;

            // Add denied tool responses (repetition + security)
            for (id, name, reason) in &denied {
                self.task_manager
                    .broadcast(
                        task_id,
                        StreamEvent::ToolResult {
                            id: id.clone(),
                            success: false,
                            content: reason.clone(),
                            name: Some(name.clone()),
                            duration_ms: None,
                        },
                    )
                    .await;
                tool_response_msg = tool_response_msg.with_tool_response(
                    id.clone(),
                    Err(rmcp::ErrorData::new(
                        rmcp::model::ErrorCode::INTERNAL_ERROR,
                        reason.clone(),
                        None,
                    )),
                );
            }

            // Add actual tool results
            for (id, duration_ms, result) in results {
                match result {
                    Ok(blocks) => {
                        let tool_name = tool_name_by_id.get(&id).cloned();
                        if portal_restricted {
                            let is_final_output = tool_name
                                .as_ref()
                                .map(|name| name == FINAL_OUTPUT_TOOL_NAME)
                                .unwrap_or(false);
                            if !is_final_output {
                                portal_successful_tool_calls += 1;
                            }
                        }
                        let text_repr = Self::tool_blocks_to_text(&blocks);
                        let looks_empty_tool_success = tool_name
                            .as_deref()
                            .is_some_and(|name| {
                                matches!(name, "developer__shell" | "developer__text_editor")
                            })
                            && text_repr.trim().is_empty();
                        if looks_empty_tool_success {
                            this_turn_had_tool_failure = true;
                            let err_text = "Tool reported success but produced no output and no verifiable file effect".to_string();
                            self.task_manager
                                .broadcast(
                                    task_id,
                                    StreamEvent::ToolResult {
                                        id: id.clone(),
                                        success: false,
                                        content: err_text.clone(),
                                        name: tool_name.clone(),
                                        duration_ms: Some(duration_ms),
                                    },
                                )
                                .await;
                            tool_response_msg = tool_response_msg.with_tool_response(
                                id,
                                Err(rmcp::ErrorData::new(
                                    rmcp::model::ErrorCode::INTERNAL_ERROR,
                                    err_text,
                                    None,
                                )),
                            );
                            continue;
                        }
                        this_turn_had_tool_success = true;
                        let sse_content = if text_repr.len() > 2000 {
                            format!("{}...", Self::safe_truncate(&text_repr, 2000))
                        } else {
                            text_repr.clone()
                        };
                        self.task_manager
                            .broadcast(
                                task_id,
                                StreamEvent::ToolResult {
                                    id: id.clone(),
                                    success: true,
                                    content: sse_content,
                                    name: tool_name_by_id.get(&id).cloned(),
                                    duration_ms: Some(duration_ms),
                                },
                            )
                            .await;

                        if let Some(tool_name) = tool_name.as_ref() {
                            if Self::tool_may_change_workspace(tool_name) {
                                self.task_manager
                                    .broadcast(
                                        task_id,
                                        StreamEvent::WorkspaceChanged {
                                            tool_name: tool_name.clone(),
                                        },
                                    )
                                    .await;
                            }
                        }

                        // Convert ToolContentBlocks to rmcp Content items
                        let content_items: Vec<rmcp::model::Content> = blocks
                            .iter()
                            .map(|b| match b {
                                super::mcp_connector::ToolContentBlock::Text(text) => {
                                    let truncated = Self::truncate_tool_result(text.clone());
                                    rmcp::model::Content::text(truncated)
                                }
                                super::mcp_connector::ToolContentBlock::Image {
                                    mime_type,
                                    data,
                                } => rmcp::model::Content::image(data.clone(), mime_type.clone()),
                            })
                            .collect();

                        let call_result = rmcp::model::CallToolResult {
                            content: content_items,
                            structured_content: None,
                            is_error: Some(false),
                            meta: None,
                        };
                        tool_response_msg =
                            tool_response_msg.with_tool_response(id, Ok(call_result));
                    }
                    Err(err_text) => {
                        this_turn_had_tool_failure = true;
                        if tool_name_by_id
                            .get(&id)
                            .is_some_and(|tool_name| tool_name == "subagent")
                        {
                            subagent_runtime = None;
                            swarm_downgraded = true;
                            let missing_artifacts = mission_ctx
                                .as_ref()
                                .and_then(|ctx| ctx.progress_memory.as_ref())
                                .map(|memory| memory.missing.clone())
                                .unwrap_or_default();
                            swarm_failure_reminder = Some(
                                swarm_scheduler::build_recursive_swarm_downgrade_message(
                                    contract_target_for_turn.as_deref(),
                                    &missing_artifacts,
                                    &err_text,
                                ),
                            );
                        }
                        let sse_content = if err_text.len() > 2000 {
                            format!("{}...", Self::safe_truncate(&err_text, 2000))
                        } else {
                            err_text.clone()
                        };
                        self.task_manager
                            .broadcast(
                                task_id,
                                StreamEvent::ToolResult {
                                    id: id.clone(),
                                    success: false,
                                    content: sse_content,
                                    name: tool_name_by_id.get(&id).cloned(),
                                    duration_ms: Some(duration_ms),
                                },
                            )
                            .await;

                        tool_response_msg = tool_response_msg.with_tool_response(
                            id,
                            Err(rmcp::ErrorData::new(
                                rmcp::model::ErrorCode::INTERNAL_ERROR,
                                Self::truncate_tool_result(err_text),
                                None,
                            )),
                        );
                    }
                }
            }
            let mut turn_file_delta_observed = false;
            if this_turn_had_tool_success {
                if let (Some(before_snapshot), Some(workspace_root)) =
                    (workspace_before.as_ref(), workspace_path.as_deref())
                {
                    if let Ok(after_snapshot) = runtime::snapshot_workspace_files(workspace_root) {
                        let mut hinted_paths = mission_bootstrap_missing_candidates(mission_ctx.as_ref());
                        if let Some(target_file) = contract_target_for_turn.as_ref() {
                            hinted_paths.push(target_file.clone());
                        }
                        hinted_paths.sort();
                        hinted_paths.dedup();

                        let changed_file = contract_target_for_turn
                            .as_deref()
                            .filter(|target_file| {
                                workspace_target_file_changed(
                                    Some(before_snapshot),
                                    &after_snapshot,
                                    target_file,
                                )
                            })
                            .map(str::to_string)
                            .or_else(|| {
                                workspace_any_candidate_file_changed(
                                    Some(before_snapshot),
                                    &after_snapshot,
                                    &hinted_paths,
                                )
                            });

                        if let Some(changed_file) = changed_file.as_deref() {
                            mission_no_delta_retry_count = 0;
                            turn_file_delta_observed = true;
                            apply_bootstrap_file_delta_to_mission_context(
                                &mut mission_ctx,
                                changed_file,
                            );
                        } else if let Some(target_file) = contract_target_for_turn.as_ref() {
                            if mission_no_delta_retry_count < MISSION_NO_DELTA_RETRY_LIMIT {
                                mission_no_delta_retry_count =
                                    mission_no_delta_retry_count.saturating_add(1);
                                delta_retry_reminder = Some(format!(
                                    "Mission execution retry ({}/{}): tools ran but `{}` did not change in this round. In the next round you must directly create or materially update `{}`. Reuse existing completed outputs as inputs and run one minimal validation on `{}` after writing it.",
                                    mission_no_delta_retry_count,
                                    MISSION_NO_DELTA_RETRY_LIMIT,
                                    target_file,
                                    target_file,
                                    target_file
                                ));
                            } else {
                                return Err(anyhow!(
                                    "Mission execution produced no target file delta for {} after {} retries",
                                    target_file,
                                    MISSION_NO_DELTA_RETRY_LIMIT
                                ));
                            }
                        }

                        if let Some(binding) = mission_run_binding.as_ref() {
                            if !hinted_paths.is_empty() {
                                let artifact_step_index =
                                    mission_artifact_step_index(binding, mission_ctx.as_ref());
                                if let Err(error) = runtime::reconcile_workspace_artifacts_with_hints(
                                    &self.agent_service,
                                    &binding.mission_id,
                                    artifact_step_index,
                                    workspace_root,
                                    Some(before_snapshot),
                                    &hinted_paths,
                                )
                                .await
                                {
                                    tracing::warn!(
                                        mission_id = %binding.mission_id,
                                        task_node_id = %binding.task_node_id,
                                        %error,
                                        "Failed to reconcile mission artifacts after workspace changes"
                                    );
                                } else {
                                    let _ = self
                                        .agent_service
                                        .refresh_delivery_manifest_from_artifacts(&binding.mission_id)
                                        .await;
                                    let _ = self
                                        .agent_service
                                        .refresh_progress_memory(&binding.mission_id)
                                        .await;
                                }
                            }
                        }
                    }
                }
            }
            previous_turn_had_tool_failure = this_turn_had_tool_failure;

            // Track consecutive turns where ALL tool calls failed (none succeeded).
            // When the threshold is reached, inject a reflection prompt to force
            // the agent to change its strategy instead of repeating failing patterns.
            if this_turn_had_tool_failure && !this_turn_had_tool_success {
                consecutive_tool_failure_turns += 1;
            } else if this_turn_had_tool_success {
                consecutive_tool_failure_turns = 0;
            }

            messages.push(tool_response_msg);
            if let Some(reminder) = swarm_failure_reminder.take() {
                messages.push(Message::user().with_text(reminder).agent_only());
            }
            if let Some(reminder) = delta_retry_reminder.take() {
                if let Some(binding) = mission_run_binding.as_ref() {
                    let outcome = TurnOutcome {
                        mode: mission_turn_mode(mission_ctx.as_ref()),
                        produced_file_delta: false,
                        produced_evidence_delta: false,
                        produced_blocker_delta: false,
                        tool_calls: tool_requests.len(),
                        success: false,
                        reason: Some(reminder.clone()),
                    };
                    let memory = mission_ctx
                        .as_ref()
                        .and_then(|ctx| ctx.progress_memory.as_ref())
                        .map(RunMemory::from);
                    let _ = self
                        .agent_service
                        .append_run_journal(&[RunJournal {
                            id: None,
                            run_id: binding.run_id.clone(),
                            mission_id: Some(binding.mission_id.clone()),
                            task_node_id: binding.task_node_id.clone(),
                            mode: outcome.mode.clone(),
                            tool_calls: outcome.tool_calls,
                            produced_file_delta: false,
                            produced_evidence_delta: false,
                            produced_blocker_delta: false,
                            reason: outcome.reason.clone(),
                            next_node_id: Some(binding.task_node_id.clone()),
                            created_at: Some(bson::DateTime::now()),
                        }])
                        .await;
                    let _ = self
                        .agent_service
                        .patch_run_state_after_turn(
                            &binding.run_id,
                            &binding.task_node_id,
                            mission_run_status_for_outcome(&outcome),
                            memory.as_ref(),
                            &outcome,
                        )
                        .await;
                }
                messages.push(Message::user().with_text(reminder).agent_only());
                continue;
            }

            if let Some(binding) = mission_run_binding.as_ref() {
                let produced_file_delta = turn_file_delta_observed;
                let outcome = TurnOutcome {
                    mode: mission_turn_mode(mission_ctx.as_ref()),
                    produced_file_delta,
                    produced_evidence_delta: false,
                    produced_blocker_delta: false,
                    tool_calls: tool_requests.len(),
                    success: this_turn_had_tool_success || !this_turn_had_tool_failure,
                    reason: None,
                };
                let memory = mission_ctx
                    .as_ref()
                    .and_then(|ctx| ctx.progress_memory.as_ref())
                    .map(RunMemory::from);
                let _ = self
                    .agent_service
                    .append_run_journal(&[RunJournal {
                        id: None,
                        run_id: binding.run_id.clone(),
                        mission_id: Some(binding.mission_id.clone()),
                        task_node_id: binding.task_node_id.clone(),
                        mode: outcome.mode.clone(),
                        tool_calls: outcome.tool_calls,
                        produced_file_delta: outcome.produced_file_delta,
                        produced_evidence_delta: false,
                        produced_blocker_delta: false,
                        reason: outcome.reason.clone(),
                        next_node_id: Some(binding.task_node_id.clone()),
                        created_at: Some(bson::DateTime::now()),
                    }])
                    .await;
                let _ = self
                    .agent_service
                    .patch_run_state_after_turn(
                        &binding.run_id,
                        &binding.task_node_id,
                        mission_run_status_for_outcome(&outcome),
                        memory.as_ref(),
                        &outcome,
                    )
                    .await;
                if outcome.produced_file_delta {
                    let _ = self
                        .agent_service
                        .save_run_checkpoint(&RunCheckpoint {
                            id: None,
                            run_id: binding.run_id.clone(),
                            mission_id: Some(binding.mission_id.clone()),
                            task_graph_id: Some(format!(
                                "mission:{}:{}",
                                binding.mission_id, binding.run_id
                            )),
                            current_node_id: Some(binding.task_node_id.clone()),
                            checkpoint_kind: RunCheckpointKind::NodeSuccess,
                            status: mission_run_status_for_outcome(&outcome),
                            lease: None,
                            memory,
                            last_turn_outcome: Some(outcome),
                            created_at: Some(bson::DateTime::now()),
                        })
                        .await;
                }
            }

            if consecutive_tool_failure_turns >= CONSECUTIVE_FAILURE_REFLECTION_THRESHOLD {
                let reflection_msg = format!(
                    "[System] Your last {} consecutive turns ALL resulted in tool call failures. \
                     STOP and reflect before your next action:\n\
                     1. What pattern is causing these repeated failures?\n\
                     2. Are you using the wrong tool, wrong syntax, or wrong approach entirely?\n\
                     3. Consider a fundamentally different strategy rather than variations of the same failing approach.\n\
                     4. If shell commands keep failing, check: are you using the correct shell syntax for this OS? \
                        Are paths correct? Is the tool available?\n\
                     Do NOT repeat the same type of action. Change your approach.",
                    consecutive_tool_failure_turns
                );
                messages.push(Message::user().with_text(reflection_msg));
                consecutive_tool_failure_turns = 0;
            }

            self.task_manager
                .broadcast(
                    task_id,
                    StreamEvent::Status {
                        status: "running".to_string(),
                    },
                )
                .await;

            // Persist intermediate session state after each turn (crash recovery)
            // This ensures progress is not lost if the task is interrupted mid-execution.
            if turn % 3 == 2 {
                // Save every 3 turns to balance durability vs write overhead
                self.save_session_state(
                    session_id,
                    &messages,
                    accumulated_input,
                    accumulated_output,
                )
                .await;
            }
        }

        // Warn if configured max turns reached without natural completion.
        if completed_due_to_max_turns {
            if let Some(max_turns_limit) = max_turns {
                tracing::warn!(
                    "Unified loop reached max turns ({}) for task {}",
                    max_turns_limit,
                    task_id
                );
                let warning = format!(
                    "\n[Warning: Agent reached maximum turn limit ({}). Task may be incomplete.]",
                    max_turns_limit
                );
                all_text.push_str(&warning);
                self.task_manager
                    .broadcast(task_id, StreamEvent::Text { content: warning })
                    .await;
            }
        }

        // Save final accumulated text
        if !all_text.is_empty() {
            self.save_task_result(task_id, TaskResultType::Message, &all_text)
                .await?;
        }

        // Save session state
        self.save_session_state(session_id, &messages, accumulated_input, accumulated_output)
            .await;

        Ok(())
    }

    // ========================================
    // Streaming, compaction, and session helpers
    // ========================================

    /// Call Provider's streaming API, broadcast text deltas, return complete Message + Usage
    async fn call_provider_streaming(
        &self,
        task_id: &str,
        provider: &Arc<dyn Provider>,
        system_prompt: &str,
        messages: &[Message],
        tools: &[rmcp::model::Tool],
        cancel_token: &CancellationToken,
    ) -> Result<(Message, Option<ProviderUsage>)> {
        self.task_manager
            .broadcast(
                task_id,
                StreamEvent::Status {
                    status: "llm_call".to_string(),
                },
            )
            .await;

        // Try streaming first, fall back to complete() for known stream failures.
        let stream_result = provider.stream(system_prompt, messages, tools).await;

        let mut msg_stream = match stream_result {
            Ok(s) => s,
            Err(e) if Self::should_fallback_to_non_streaming(&e) => {
                return self
                    .fallback_to_complete_from_stream(
                        task_id,
                        provider,
                        system_prompt,
                        messages,
                        tools,
                        cancel_token,
                        None,
                        &format!("stream initialization failed: {}", e),
                    )
                    .await;
            }
            Err(e) => return Err(anyhow::Error::from(e)),
        };

        // Consume the stream and normalize chunks into true deltas.
        // Some providers emit incremental deltas, others emit cumulative chunks.
        let mut accumulated_text = String::new();
        let mut accumulated_thinking = String::new();
        let mut thinking_signature = String::new();
        let mut tool_requests: Vec<MessageContent> = Vec::new();
        let mut final_usage: Option<ProviderUsage> = None;
        let mut got_any_message = false;

        let chunk_timeout_secs = std::env::var("TEAM_PROVIDER_CHUNK_TIMEOUT_SECS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(120);
        let chunk_timeout = Duration::from_secs(chunk_timeout_secs);
        loop {
            tokio::select! {
                item = tokio::time::timeout(chunk_timeout, msg_stream.next()) => {
                    match item {
                        Err(_) => {
                            return self
                                .fallback_to_complete_from_stream(
                                    task_id,
                                    provider,
                                    system_prompt,
                                    messages,
                                    tools,
                                    cancel_token,
                                    Some(&accumulated_text),
                                    &format!(
                                        "stream idle timeout ({}s without chunks)",
                                        chunk_timeout_secs
                                    ),
                                )
                                .await;
                        }
                        Ok(item) => match item {
                        Some(Ok((msg_opt, usage_opt))) => {
                            if let Some(ref msg) = msg_opt {
                                got_any_message = true;
                                for part in &msg.content {
                                    match part {
                                        MessageContent::Text(tc) => {
                                            if !tc.text.is_empty() {
                                                let delta = Self::extract_stream_delta(
                                                    &accumulated_text,
                                                    &tc.text,
                                                );
                                                if !delta.is_empty() {
                                                    accumulated_text.push_str(&delta);
                                                    self.task_manager
                                                        .broadcast(
                                                            task_id,
                                                            StreamEvent::Text { content: delta },
                                                        )
                                                        .await;
                                                }
                                            }
                                        }
                                        MessageContent::Thinking(tc) => {
                                            if !tc.thinking.is_empty() {
                                                let delta = Self::extract_stream_delta(
                                                    &accumulated_thinking,
                                                    &tc.thinking,
                                                );
                                                if !delta.is_empty() {
                                                    accumulated_thinking.push_str(&delta);
                                                    thinking_signature = tc.signature.clone();
                                                    self.task_manager
                                                        .broadcast(
                                                            task_id,
                                                            StreamEvent::Thinking { content: delta },
                                                        )
                                                        .await;
                                                }
                                            }
                                        }
                                        MessageContent::ToolRequest(_) => {
                                            // Tool requests are already accumulated
                                            // by the stream (yielded once complete)
                                            tool_requests.push(part.clone());
                                        }
                                        other => {
                                            // RedactedThinking, etc. — keep as-is
                                            tool_requests.push(other.clone());
                                        }
                                    }
                                }
                            }
                            if usage_opt.is_some() {
                                final_usage = usage_opt;
                            }
                        }
                        Some(Err(e)) => {
                            if Self::should_fallback_to_non_streaming(&e) {
                                return self
                                    .fallback_to_complete_from_stream(
                                        task_id,
                                        provider,
                                        system_prompt,
                                        messages,
                                        tools,
                                        cancel_token,
                                        Some(&accumulated_text),
                                        &format!("stream decode/runtime error: {}", e),
                                    )
                                    .await;
                            }
                            return Err(anyhow::Error::from(e));
                        }
                        None => break, // Stream ended
                    } // inner match (stream item)
                    } // outer match (timeout result)
                }
                _ = cancel_token.cancelled() => {
                    return Err(anyhow!("Task cancelled during streaming"));
                }
            }
        }

        if !got_any_message {
            return self
                .fallback_to_complete_from_stream(
                    task_id,
                    provider,
                    system_prompt,
                    messages,
                    tools,
                    cancel_token,
                    Some(&accumulated_text),
                    "stream ended without producing a message",
                )
                .await;
        }

        // Build the accumulated message with all collected content
        let mut content: Vec<MessageContent> = Vec::new();
        if !accumulated_thinking.is_empty() {
            content.push(MessageContent::thinking(
                accumulated_thinking,
                thinking_signature,
            ));
        }
        if !accumulated_text.is_empty() {
            content.push(MessageContent::text(accumulated_text));
        }
        content.extend(tool_requests);

        let message = Message::new(
            rmcp::model::Role::Assistant,
            chrono::Utc::now().timestamp(),
            content,
        );
        Ok((message, final_usage))
    }

    fn should_compact_now(turn: usize, ratio: f64, last_compaction_turn: Option<usize>) -> bool {
        match last_compaction_turn {
            None => true,
            Some(last) => {
                let turns_since = turn.saturating_sub(last);
                if turns_since < MIN_TURNS_BETWEEN_COMPACTIONS {
                    return false;
                }
                if ratio >= COMPACTION_REENTRY_RATIO {
                    return true;
                }
                turns_since >= MIN_TURNS_FOR_NORMAL_REENTRY
            }
        }
    }

    /// Check if context compaction is needed based on token count vs context limit.
    /// Returns: (threshold_hit, current_tokens, current_ratio)
    async fn check_compaction_needed(
        &self,
        provider: &Arc<dyn Provider>,
        system_prompt: &str,
        messages: &[Message],
        tools: &[rmcp::model::Tool],
    ) -> Result<(bool, usize, f64)> {
        let context_limit = provider.get_model_config().context_limit();
        let threshold = DEFAULT_COMPACTION_THRESHOLD;

        let counter = create_token_counter()
            .await
            .map_err(|e| anyhow!("Token counter: {}", e))?;
        let current_tokens = counter.count_chat_tokens(system_prompt, messages, tools);

        let ratio = current_tokens as f64 / context_limit as f64;
        tracing::debug!(
            "Compaction check: {}/{} = {:.2} (threshold: {:.2})",
            current_tokens,
            context_limit,
            ratio,
            threshold
        );
        Ok((ratio > threshold, current_tokens, ratio))
    }

    /// Save session state to MongoDB
    async fn save_session_state(
        &self,
        session_id: &str,
        messages: &[Message],
        input_tokens: i32,
        output_tokens: i32,
    ) {
        let messages_json = match serde_json::to_string(messages) {
            Ok(j) => j,
            Err(e) => {
                tracing::warn!("Failed to serialize messages: {}", e);
                return;
            }
        };
        let msg_count = messages.len() as i32;
        let total = if input_tokens > 0 || output_tokens > 0 {
            Some(input_tokens + output_tokens)
        } else {
            None
        };

        if let Err(e) = self
            .agent_service
            .update_session_messages(
                session_id,
                &messages_json,
                msg_count,
                total,
                if input_tokens > 0 {
                    Some(input_tokens)
                } else {
                    None
                },
                if output_tokens > 0 {
                    Some(output_tokens)
                } else {
                    None
                },
            )
            .await
        {
            tracing::warn!("Failed to save session {}: {}", session_id, e);
        }
    }

    // ========================================
    // Database helper methods (MongoDB)
    // ========================================

    /// Get task by ID
    async fn get_task(&self, task_id: &str) -> Result<Option<AgentTask>> {
        let doc = self
            .tasks()
            .find_one(doc! { "task_id": task_id }, None)
            .await?;
        Ok(doc.map(|d| d.into()))
    }

    /// Get agent by ID (includes API key for execution)
    async fn get_agent(&self, agent_id: &str) -> Result<Option<TeamAgent>> {
        let doc = self
            .agents()
            .find_one(doc! { "agent_id": agent_id }, None)
            .await?;

        // Convert to TeamAgent but keep api_key for execution
        Ok(doc.map(|d| {
            let api_key = d.api_key.clone();
            let mut agent: TeamAgent = d.into();
            agent.api_key = api_key; // Restore API key for execution
            agent
        }))
    }

    /// Update task status with precondition check to prevent race conditions.
    /// Only transitions from valid prior states are allowed:
    ///   Approved → Running, Running → Completed/Failed
    /// Cancelled tasks are never overwritten.
    /// Returns `Err` if the transition was rejected (e.g. task already cancelled).
    async fn update_task_status(&self, task_id: &str, status: TaskStatus) -> Result<()> {
        let now = Utc::now();
        let mut set_doc = doc! { "status": status.to_string() };

        // Determine valid prior statuses for this transition
        let allowed_from = match status {
            TaskStatus::Running => vec!["approved"],
            TaskStatus::Completed | TaskStatus::Failed => vec!["running"],
            _ => vec![],
        };

        let filter = if allowed_from.is_empty() {
            doc! { "task_id": task_id }
        } else {
            doc! { "task_id": task_id, "status": { "$in": &allowed_from } }
        };

        if status == TaskStatus::Running {
            set_doc.insert("started_at", bson::DateTime::from_chrono(now));
        }

        if status == TaskStatus::Completed || status == TaskStatus::Failed {
            set_doc.insert("completed_at", bson::DateTime::from_chrono(now));
        }

        let result = self
            .tasks()
            .update_one(filter, doc! { "$set": set_doc }, None)
            .await?;

        if result.modified_count == 0 {
            tracing::warn!(
                "update_task_status: no update for task {} to {:?} (current status not in {:?})",
                task_id,
                status,
                allowed_from
            );
            return Err(anyhow!(
                "Task {} status transition to {:?} rejected (already cancelled or terminal)",
                task_id,
                status
            ));
        }

        Ok(())
    }

    /// Update task with error. Only updates if status is running or approved.
    async fn update_task_error(&self, task_id: &str, error: &str) -> Result<()> {
        let now = Utc::now();

        let result = self
            .tasks()
            .update_one(
                doc! { "task_id": task_id, "status": { "$in": ["running", "approved"] } },
                doc! { "$set": {
                    "status": "failed",
                    "error_message": error,
                    "completed_at": bson::DateTime::from_chrono(now)
                }},
                None,
            )
            .await?;

        if result.modified_count == 0 {
            tracing::warn!(
                "update_task_error: no update for task {} (already terminal?)",
                task_id
            );
        }

        Ok(())
    }

    /// Save task result
    async fn save_task_result(
        &self,
        task_id: &str,
        result_type: TaskResultType,
        content: &str,
    ) -> Result<()> {
        let id = Uuid::new_v4().to_string();
        let now = Utc::now();

        let doc = doc! {
            "result_id": &id,
            "task_id": task_id,
            "result_type": result_type.to_string(),
            "content": doc! { "text": content },
            "created_at": bson::DateTime::from_chrono(now)
        };

        self.results().insert_one(doc, None).await?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{
        apply_bootstrap_file_delta_to_mission_context, mission_bootstrap_missing_candidates,
        mission_contract_target_file, workspace_any_candidate_file_changed, MissionPromptContext,
        RepetitionDetector, TaskExecutor,
    };
    use agime::security::scanner::PromptInjectionScanner;
    use std::collections::HashMap;

    use crate::agent::mission_mongo::MissionProgressMemory;
    use crate::agent::runtime::{WorkspaceFileFingerprint, WorkspaceSnapshot};

    #[test]
    fn strip_heredoc_bodies_preserves_shell_prefix_and_marker() {
        let command = "cat > report.html <<'EOF'\n<h1>中文标题</h1>\n<p>`template` body</p>\nEOF\nls -la report.html";
        let stripped = TaskExecutor::strip_heredoc_bodies(command);
        assert!(stripped.contains("cat > report.html <<'EOF'"));
        assert!(stripped.contains("[HEREDOC_BODY_ELIDED]"));
        assert!(stripped.contains("\nEOF\n"));
        assert!(stripped.contains("ls -la report.html"));
        assert!(!stripped.contains("中文标题"));
        assert!(!stripped.contains("template"));
    }

    #[test]
    fn extract_shell_scan_text_prefers_command_field() {
        let args = serde_json::json!({
            "command": "printf 'ok'\ncat > foo.html <<EOF\n<html>内容</html>\nEOF",
            "cwd": "/tmp/workspace",
            "timeout_ms": 5000
        });
        let extracted = TaskExecutor::extract_shell_scan_text(&args);
        assert!(extracted.contains("printf 'ok'"));
        assert!(extracted.contains("[HEREDOC_BODY_ELIDED]"));
        assert!(!extracted.contains("内容"));
        assert!(!extracted.contains("/tmp/workspace"));
    }

    #[tokio::test]
    async fn safe_heredoc_payload_does_not_trigger_scanner_after_normalization() {
        let args = serde_json::json!({
            "command": "cat > report.html <<'EOF'\n<h1>中文标题</h1>\n<p>`template` body</p>\nEOF\nls -la report.html"
        });
        let text = format!(
            "Tool: developer__shell\n{}",
            TaskExecutor::extract_shell_scan_text(&args)
        );
        let scanner = PromptInjectionScanner::new();
        let result = scanner.scan_for_dangerous_patterns(&text).await.unwrap();
        assert!(
            !result.is_malicious,
            "unexpected scanner hit after heredoc normalization: {}",
            result.explanation
        );
    }

    #[test]
    fn repeated_mutating_tool_calls_are_denied_on_third_identical_call() {
        let mut detector = RepetitionDetector::new();
        let args = serde_json::json!({ "command": "mkdir reports" });

        assert!(detector.check("developer__shell", &args));
        assert!(detector.check("developer__shell", &args));
        assert!(!detector.check("developer__shell", &args));
    }

    #[test]
    fn mission_preflight_calls_get_more_recovery_headroom() {
        let mut detector = RepetitionDetector::new();
        let args = serde_json::json!({
            "step_goal": "Collect context",
            "workspace_path": "/tmp/workspace"
        });

        for _ in 0..20 {
            assert!(detector.check("mission_preflight__preflight", &args));
        }
    }

    #[test]
    fn repeated_non_shell_tool_calls_are_not_blocked_by_generic_guard() {
        let mut detector = RepetitionDetector::new();
        let args = serde_json::json!({ "url": "https://example.com" });

        for _ in 0..8 {
            assert!(detector.check("computercontroller__web_scrape", &args));
        }
    }

    #[test]
    fn shell_security_hit_is_softened_for_documentary_markdown_like_commands() {
        let command = "cat > README.md <<'EOF'\n# 中文说明\n包含 `echo ok` 示例\nEOF\nls README.md";
        let text = format!(
            "Tool: developer__shell\n{}",
            TaskExecutor::strip_heredoc_bodies(command)
        );
        assert!(TaskExecutor::should_soften_shell_security_hit(
            &text,
            "Unicode character obfuscation"
        ));
    }

    #[test]
    fn governance_and_portal_write_tools_are_marked_serial() {
        assert!(TaskExecutor::tool_requires_serial_write(
            "avatar_governance__review_request"
        ));
        assert!(TaskExecutor::tool_requires_serial_write(
            "avatar_governance__submit_capability_request"
        ));
        assert!(TaskExecutor::tool_requires_serial_write(
            "avatar_governance__submit_gap_proposal"
        ));
        assert!(TaskExecutor::tool_requires_serial_write(
            "avatar_governance__submit_human_review_request"
        ));
        assert!(TaskExecutor::tool_requires_serial_write(
            "avatar_governance__submit_optimization_ticket"
        ));
        assert!(TaskExecutor::tool_requires_serial_write(
            "portal_tools__configure_portal_service_agent"
        ));

        assert!(!TaskExecutor::tool_requires_serial_write(
            "avatar_governance__get_runtime_boundary"
        ));
        assert!(!TaskExecutor::tool_requires_serial_write(
            "avatar_governance__list_request_status"
        ));
        assert!(!TaskExecutor::tool_requires_serial_write("skills__search"));
    }

    #[test]
    fn non_retryable_provider_request_text_detects_auth_and_subscription_failures() {
        assert!(TaskExecutor::is_non_retryable_provider_request_text(
            "Authentication failed. Status: 401 Unauthorized. Response: Your authentication token has been invalidated."
        ));
        assert!(TaskExecutor::is_non_retryable_provider_request_text(
            "Request failed: Bad request (400): Your account does not have a valid coding plan subscription, or your subscription has expired"
        ));
        assert!(!TaskExecutor::is_non_retryable_provider_request_text(
            "Rate limit exceeded: All credentials for model gpt-5.2 are cooling down"
        ));
    }

    #[test]
    fn waiting_external_provider_errors_skip_executor_retries() {
        let err = anyhow::anyhow!(
            "Rate limit exceeded: All credentials for model gpt-5.2 are cooling down"
        );
        assert!(!TaskExecutor::is_retryable_provider_error(&err));
    }

    #[test]
    fn single_worker_launch_policy_disables_goal_level_swarm_fallback() {
        let mission_ctx = MissionPromptContext {
            goal: "compare".to_string(),
            context: None,
            approval_policy: "auto".to_string(),
            launch_policy: Some(LaunchPolicy::SingleWorker),
            total_steps: 1,
            current_step: 1,
            progress_memory: Some(MissionProgressMemory {
                done: Vec::new(),
                missing: vec!["compare/comparison.csv".to_string()],
                blocked_by: None,
                last_failed_attempt: None,
                next_best_action: None,
                confidence: None,
                updated_at: None,
            }),
            latest_worker_state: None,
            task_node_id: Some("goal:g-1".to_string()),
        };
        let task = agime_team::models::AgentTask::new(
            "team-1".to_string(),
            "agent-1".to_string(),
            "user-1".to_string(),
            agime_team::models::TaskType::Auto,
            serde_json::json!({
                "mission_id": "m-1",
                "run_id": "r-1",
                "task_role": "mission_worker",
                "task_node_id": "goal:g-1",
            }),
        );
        let binding = MissionRunBinding {
            mission_id: "m-1".to_string(),
            run_id: "r-1".to_string(),
            task_node_id: "goal:g-1".to_string(),
        };
        assert!(
            subagent_runtime_from_task(&task, Some(&mission_ctx), Some(&binding), None).is_none()
        );
    }
}
#[derive(Clone, Debug)]
struct MissionRunBinding {
    mission_id: String,
    run_id: String,
    task_node_id: String,
}

#[derive(Clone, Debug)]
struct SubagentRuntimeContext {
    delegation_mode: HarnessDelegationMode,
    depth: u32,
    max_depth: u32,
    write_scope: Vec<String>,
    parent_run_id: Option<String>,
    parent_task_node_id: Option<String>,
    source_mission_id: Option<String>,
    spec_name: String,
}

fn mission_run_binding_from_task(
    task: &AgentTask,
    mission_ctx: Option<&MissionPromptContext>,
) -> Option<MissionRunBinding> {
    let mission_id = task
        .content
        .get("mission_id")
        .and_then(serde_json::Value::as_str)
        .map(|value| value.to_string())?;
    let run_id = task
        .content
        .get("run_id")
        .and_then(serde_json::Value::as_str)
        .map(|value| value.to_string())?;
    let task_node_id = task
        .content
        .get("task_node_id")
        .and_then(serde_json::Value::as_str)
        .map(|value| value.to_string())
        .or_else(|| mission_ctx.and_then(|ctx| ctx.task_node_id.clone()))?;
    Some(MissionRunBinding {
        mission_id,
        run_id,
        task_node_id,
    })
}

fn mission_turn_mode(mission_ctx: Option<&MissionPromptContext>) -> HarnessTurnMode {
    let _ = mission_ctx;
    HarnessTurnMode::Execute
}

fn mission_artifact_step_index(
    binding: &MissionRunBinding,
    mission_ctx: Option<&MissionPromptContext>,
) -> u32 {
    if let Some(index) = binding
        .task_node_id
        .strip_prefix("step:")
        .and_then(|value| value.parse::<u32>().ok())
    {
        return index;
    }
    mission_ctx
        .map(|ctx| ctx.current_step.saturating_sub(1) as u32)
        .unwrap_or(0)
}

    fn mission_run_status_for_outcome(outcome: &TurnOutcome) -> RunStatus {
        match outcome.mode {
            HarnessTurnMode::Blocked => RunStatus::Blocked,
            HarnessTurnMode::Complete => RunStatus::Completed,
            HarnessTurnMode::Repair => RunStatus::Repairing,
        HarnessTurnMode::Plan => RunStatus::Planning,
        HarnessTurnMode::Execute => RunStatus::Executing,
            HarnessTurnMode::Conversation => RunStatus::Executing,
        }
    }

fn task_string_field(task: &AgentTask, key: &str) -> Option<String> {
    task.content
        .get(key)
        .and_then(serde_json::Value::as_str)
        .map(|value| value.to_string())
}

fn task_u32_field(task: &AgentTask, key: &str) -> Option<u32> {
    task.content
        .get(key)
        .and_then(serde_json::Value::as_u64)
        .map(|value| value.min(u32::MAX as u64) as u32)
}

fn task_string_vec_field(task: &AgentTask, key: &str) -> Vec<String> {
    task.content
        .get(key)
        .and_then(serde_json::Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(serde_json::Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(|value| value.to_string())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

fn inferred_subagent_spec_name(mission_ctx: Option<&MissionPromptContext>) -> String {
    let _ = mission_ctx;
    "general-worker".to_string()
}

fn spec_name_for_task_node(task_node: &TaskNode) -> String {
    match task_node
        .delegation_mode
        .clone()
        .unwrap_or(HarnessDelegationMode::Disabled)
    {
        HarnessDelegationMode::Subagent => "general-worker".to_string(),
        HarnessDelegationMode::Swarm => match task_node.swarm_mode {
            Some(super::harness_core::HarnessSwarmMode::Gather) => "research-gather".to_string(),
            Some(super::harness_core::HarnessSwarmMode::Fill) => "fill".to_string(),
            Some(super::harness_core::HarnessSwarmMode::Draft) => "artifact-draft".to_string(),
            Some(super::harness_core::HarnessSwarmMode::Validate) => "validator".to_string(),
            Some(super::harness_core::HarnessSwarmMode::RecursiveOrchestrate) => {
                "general-worker".to_string()
            }
            _ => "general-worker".to_string(),
        },
        HarnessDelegationMode::Disabled => "general-worker".to_string(),
    }
}

fn subagent_runtime_from_task(
    task: &AgentTask,
    mission_ctx: Option<&MissionPromptContext>,
    mission_run_binding: Option<&MissionRunBinding>,
    graph_task_node: Option<&TaskNode>,
) -> Option<SubagentRuntimeContext> {
    let task_role = task_string_field(task, "task_role")
        .or_else(|| task_string_field(task, "content.task_role"))
        .unwrap_or_default();
    let explicit_depth = task_u32_field(task, "subagent_depth");
    let explicit_max_depth = task_u32_field(task, "subagent_max_depth");
    let explicit_write_scope = task_string_vec_field(task, "subagent_write_scope");
    let explicit_parent_run = task_string_field(task, "subagent_parent_run_id");
    let explicit_parent_task_node = task_string_field(task, "subagent_parent_task_node_id");
    let explicit_spec_name = task_string_field(task, "subagent_spec_name");
    let source_mission_id = task_string_field(task, "source_mission_id")
        .or_else(|| task_string_field(task, "mission_id"));

    if task_role == "subagent_worker"
        || explicit_depth.is_some()
        || explicit_parent_run.is_some()
        || explicit_parent_task_node.is_some()
    {
        return Some(SubagentRuntimeContext {
            delegation_mode: HarnessDelegationMode::Subagent,
            depth: explicit_depth.unwrap_or(1),
            max_depth: explicit_max_depth.unwrap_or(2).max(1),
            write_scope: explicit_write_scope,
            parent_run_id: explicit_parent_run,
            parent_task_node_id: explicit_parent_task_node
                .or_else(|| task_string_field(task, "task_node_id")),
            source_mission_id,
            spec_name: explicit_spec_name
                .filter(|value| !value.trim().is_empty())
                .unwrap_or_else(|| inferred_subagent_spec_name(mission_ctx)),
        });
    }

    if mission_ctx.is_none() {
        return None;
    }

    if mission_ctx
        .and_then(|ctx| ctx.launch_policy.clone())
        .is_some_and(|policy| {
            matches!(
                policy,
                LaunchPolicy::SingleWorker | LaunchPolicy::GuidedCheckpoint
            )
        })
    {
        return None;
    }

    if let Some(task_node) = graph_task_node {
        if let Some(delegation_mode) = task_node.delegation_mode.clone() {
            let write_scope = if task_node.write_scope.is_empty() {
                task_node.target_artifacts.clone()
            } else {
                task_node.write_scope.clone()
            };
            return Some(SubagentRuntimeContext {
                delegation_mode: delegation_mode.clone(),
                depth: 0,
                max_depth: match delegation_mode {
                    HarnessDelegationMode::Swarm => 2,
                    HarnessDelegationMode::Subagent => 1,
                    HarnessDelegationMode::Disabled => 0,
                },
                write_scope,
                parent_run_id: mission_run_binding.map(|binding| binding.run_id.clone()),
                parent_task_node_id: Some(task_node.task_node_id.clone()),
                source_mission_id,
                spec_name: spec_name_for_task_node(task_node),
            });
        }
        return None;
    }

    None
}
