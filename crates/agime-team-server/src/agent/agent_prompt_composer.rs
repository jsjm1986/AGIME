use std::fmt;

use agime::agents::extension::ExtensionInfo;
use agime::agents::harness::{native_swarm_tool_enabled, planner_auto_swarm_enabled};
use agime::agents::subagent_tool::should_enable_subagents;
use agime::prompt_template;
use agime_team::models::{ApprovalMode, DelegationPolicy};
use chrono::Local;
use serde::{Deserialize, Serialize};

use super::capability_policy::RuntimeCapabilitySnapshot;

const DEFAULT_PROMPT_PACK_VERSION: &str = "v2";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PromptPackVersion {
    V1,
    V2,
}

impl PromptPackVersion {
    pub fn from_env() -> Self {
        match std::env::var("TEAM_AGENT_PROMPT_PACK_VERSION")
            .ok()
            .map(|value| value.trim().to_ascii_lowercase())
            .as_deref()
        {
            Some("v1") => Self::V1,
            Some("v2") => Self::V2,
            Some(_) | None => match DEFAULT_PROMPT_PACK_VERSION {
                "v1" => Self::V1,
                _ => Self::V2,
            },
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::V1 => "v1",
            Self::V2 => "v2",
        }
    }
}

impl fmt::Display for PromptPackVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptBackupSource {
    pub key: String,
    pub source_kind: String,
    pub path: String,
    pub usage: String,
    pub priority: u32,
    pub render_entrypoint: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptBackupManifest {
    pub manifest_version: String,
    pub sources: Vec<PromptBackupSource>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RenderedPromptSnapshot {
    pub scenario_id: String,
    pub prompt_kind: String,
    pub prompt_snapshot_version: String,
    pub source_order: Vec<String>,
    pub rendered_prompt: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HarnessDelegationOverlay {
    pub prompt_snapshot_version: String,
    pub session_source: String,
    pub tasks_enabled: bool,
    pub plan_enabled: bool,
    pub subagent_enabled: bool,
    pub swarm_enabled: bool,
    pub worker_peer_messaging_enabled: bool,
    pub auto_swarm_enabled: bool,
    pub validation_worker_enabled: bool,
    pub approval_mode: ApprovalMode,
    pub require_final_report: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub document_access_mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub document_scope_mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub document_write_mode: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SurfacePromptContract {
    pub session_source: String,
    pub portal_restricted: bool,
    pub require_final_report: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub document_access_mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub document_scope_mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub document_write_mode: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptCompositionReport {
    pub prompt_snapshot_version: String,
    pub source_order: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capability_snapshot: Option<RuntimeCapabilitySnapshot>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delegation_snapshot: Option<DelegationPolicy>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub harness_capabilities: Option<HarnessDelegationOverlay>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approval_snapshot: Option<ApprovalMode>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptIntrospectionSnapshot {
    pub prompt_snapshot_version: String,
    pub capability_snapshot: RuntimeCapabilitySnapshot,
    pub delegation_snapshot: DelegationPolicy,
    pub harness_capabilities: HarnessDelegationOverlay,
    pub tasks_enabled: bool,
    pub subagent_enabled: bool,
    pub swarm_enabled: bool,
    pub worker_peer_messaging_enabled: bool,
    pub auto_swarm_enabled: bool,
    pub validation_worker_enabled: bool,
    pub approval_mode: ApprovalMode,
}

#[derive(Debug, Clone)]
pub struct AgentPromptComposition {
    pub top_level_prompt: String,
    pub report: PromptCompositionReport,
}

#[derive(Debug, Clone)]
pub struct AgentPromptComposerInput<'a> {
    pub extensions: &'a [ExtensionInfo],
    pub custom_prompt: Option<&'a str>,
    pub runtime_snapshot: Option<&'a RuntimeCapabilitySnapshot>,
    pub session_extra_instructions: Option<&'a str>,
    pub prompt_profile_overlay: Option<&'a str>,
    pub turn_system_instruction: Option<&'a str>,
    pub session_source: &'a str,
    pub portal_restricted: bool,
    pub require_final_report: bool,
    pub model_name: &'a str,
}

#[derive(Serialize)]
struct BaseBusinessPromptContext {
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

pub fn prompt_backup_manifest() -> PromptBackupManifest {
    PromptBackupManifest {
        manifest_version: "prompt-backup-v1".to_string(),
        sources: vec![
            PromptBackupSource {
                key: "embedded_system_prompt".to_string(),
                source_kind: "embedded_prompt".to_string(),
                path: "crates/agime/src/prompts/system.md".to_string(),
                usage: "top-level base business prompt".to_string(),
                priority: 1,
                render_entrypoint: "AgentPromptComposer::build_base_business_prompt".to_string(),
            },
            PromptBackupSource {
                key: "embedded_engineering_prompt".to_string(),
                source_kind: "embedded_prompt".to_string(),
                path: "crates/agime/src/prompts/engineering.md".to_string(),
                usage: "core engineering instructions referenced by business prompt pack"
                    .to_string(),
                priority: 2,
                render_entrypoint: "embedded_prompt_reference".to_string(),
            },
            PromptBackupSource {
                key: "embedded_leader_prompt".to_string(),
                source_kind: "embedded_prompt".to_string(),
                path: "crates/agime/src/prompts/leader_system.md".to_string(),
                usage: "internal harness coordinator prompt".to_string(),
                priority: 3,
                render_entrypoint: "Harness bootstrap / coordinator render".to_string(),
            },
            PromptBackupSource {
                key: "embedded_worker_prompt".to_string(),
                source_kind: "embedded_prompt".to_string(),
                path: "crates/agime/src/prompts/worker_system.md".to_string(),
                usage: "internal bounded worker prompt".to_string(),
                priority: 4,
                render_entrypoint: "subagent/swarm worker render".to_string(),
            },
            PromptBackupSource {
                key: "embedded_validation_worker_prompt".to_string(),
                source_kind: "embedded_prompt".to_string(),
                path: "crates/agime/src/prompts/validation_worker_system.md".to_string(),
                usage: "internal validation worker prompt".to_string(),
                priority: 5,
                render_entrypoint: "validation worker render".to_string(),
            },
            PromptBackupSource {
                key: "agent_system_prompt".to_string(),
                source_kind: "mongo_prompt_field".to_string(),
                path: "TeamAgent.system_prompt".to_string(),
                usage: "agent-specific appended custom instructions".to_string(),
                priority: 6,
                render_entrypoint: "AgentPromptComposer::build_base_business_prompt".to_string(),
            },
            PromptBackupSource {
                key: "session_extra_instructions".to_string(),
                source_kind: "mongo_prompt_field".to_string(),
                path: "AgentSessionDoc.extra_instructions".to_string(),
                usage: "session/surface additive overlay".to_string(),
                priority: 7,
                render_entrypoint: "AgentPromptComposer::compose_top_level_prompt".to_string(),
            },
            PromptBackupSource {
                key: "prompt_profiles".to_string(),
                source_kind: "code_overlay".to_string(),
                path: "crates/agime-team-server/src/agent/prompt_profiles.rs".to_string(),
                usage: "portal/manager/coding profile overlay".to_string(),
                priority: 8,
                render_entrypoint: "AgentPromptComposer::compose_top_level_prompt".to_string(),
            },
            PromptBackupSource {
                key: "active_system_prompt_override".to_string(),
                source_kind: "config_override".to_string(),
                path: "AGIME_ACTIVE_SYSTEM_PROMPT".to_string(),
                usage: "runtime system prompt override in core prompt manager".to_string(),
                priority: 9,
                render_entrypoint: "PromptManager".to_string(),
            },
        ],
    }
}

pub fn build_base_business_prompt(
    extensions: &[ExtensionInfo],
    custom_prompt: Option<&str>,
    enable_subagents: bool,
) -> String {
    let context = BaseBusinessPromptContext {
        extensions: extensions.to_vec(),
        tool_selection_strategy: None,
        current_date_time: Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
        extension_tool_limits: None,
        agime_mode: "chat".to_string(),
        is_autonomous: false,
        enable_subagents,
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

    if let Some(custom) = custom_prompt.filter(|value| !value.trim().is_empty()) {
        prompt.push_str("\n\n<agent_instructions>\n");
        prompt.push_str("The following are custom instructions configured for this agent. ");
        prompt.push_str("Follow these instructions while maintaining all core behavioral rules and safety guardrails above.\n\n");
        prompt.push_str(custom);
        prompt.push_str("\n</agent_instructions>");
    }

    prompt
}

pub fn resolve_harness_capabilities(
    snapshot: &RuntimeCapabilitySnapshot,
    model_name: &str,
    require_final_report: bool,
) -> HarnessDelegationOverlay {
    let tasks_enabled = snapshot
        .extensions
        .effective_allowed_extension_names
        .iter()
        .any(|name| name == "tasks");
    let subagent_runtime_allowed =
        snapshot.delegation_policy.allow_subagent && should_enable_subagents(model_name);
    let swarm_runtime_allowed =
        snapshot.delegation_policy.allow_swarm && native_swarm_tool_enabled();
    let auto_swarm_runtime_allowed = snapshot.delegation_policy.allow_swarm
        && snapshot.delegation_policy.allow_auto_swarm
        && planner_auto_swarm_enabled();
    let worker_peer_messaging_enabled =
        swarm_runtime_allowed && snapshot.delegation_policy.allow_worker_messaging;

    HarnessDelegationOverlay {
        prompt_snapshot_version: PromptPackVersion::from_env().to_string(),
        session_source: snapshot.session_source.clone(),
        tasks_enabled,
        plan_enabled: snapshot.delegation_policy.allow_plan,
        subagent_enabled: subagent_runtime_allowed,
        swarm_enabled: swarm_runtime_allowed,
        worker_peer_messaging_enabled,
        auto_swarm_enabled: auto_swarm_runtime_allowed,
        validation_worker_enabled: snapshot.delegation_policy.allow_validation_worker,
        approval_mode: snapshot.delegation_policy.approval_mode,
        require_final_report,
        document_access_mode: snapshot.document_access_mode.clone(),
        document_scope_mode: snapshot.document_scope_mode.clone(),
        document_write_mode: snapshot.document_write_mode.clone(),
    }
}

pub fn render_prompt_snapshot(
    scenario_id: &str,
    prompt_kind: &str,
    prompt_snapshot_version: PromptPackVersion,
    source_order: Vec<String>,
    rendered_prompt: String,
) -> RenderedPromptSnapshot {
    RenderedPromptSnapshot {
        scenario_id: scenario_id.to_string(),
        prompt_kind: prompt_kind.to_string(),
        prompt_snapshot_version: prompt_snapshot_version.to_string(),
        source_order,
        rendered_prompt,
    }
}

pub fn build_prompt_introspection_snapshot(
    snapshot: &RuntimeCapabilitySnapshot,
    model_name: &str,
    require_final_report: bool,
) -> PromptIntrospectionSnapshot {
    let harness_capabilities =
        resolve_harness_capabilities(snapshot, model_name, require_final_report);
    PromptIntrospectionSnapshot {
        prompt_snapshot_version: PromptPackVersion::from_env().to_string(),
        capability_snapshot: snapshot.clone(),
        delegation_snapshot: snapshot.delegation_policy.clone(),
        tasks_enabled: harness_capabilities.tasks_enabled,
        subagent_enabled: harness_capabilities.subagent_enabled,
        swarm_enabled: harness_capabilities.swarm_enabled,
        worker_peer_messaging_enabled: harness_capabilities.worker_peer_messaging_enabled,
        auto_swarm_enabled: harness_capabilities.auto_swarm_enabled,
        validation_worker_enabled: harness_capabilities.validation_worker_enabled,
        approval_mode: harness_capabilities.approval_mode,
        harness_capabilities,
    }
}

pub fn compose_top_level_prompt(input: AgentPromptComposerInput<'_>) -> AgentPromptComposition {
    compose_top_level_prompt_with_version(input, PromptPackVersion::from_env())
}

pub fn compose_top_level_prompt_with_version(
    input: AgentPromptComposerInput<'_>,
    version: PromptPackVersion,
) -> AgentPromptComposition {
    let base_prompt = build_base_business_prompt(
        input.extensions,
        input.custom_prompt,
        should_enable_subagents(input.model_name),
    );
    let mut prompt = base_prompt;
    let mut source_order = vec!["embedded:system.md".to_string()];
    if input
        .custom_prompt
        .is_some_and(|value| !value.trim().is_empty())
    {
        source_order.push("agent.system_prompt".to_string());
    }

    let mut harness_capabilities = None;

    if version == PromptPackVersion::V2 {
        if let Some(snapshot) = input.runtime_snapshot {
            append_tagged_section(
                &mut prompt,
                "runtime_capability_snapshot",
                &build_runtime_capability_snapshot_overlay(snapshot),
            );
            source_order.push("runtime_capability_snapshot".to_string());

            let mut overlay = resolve_harness_capabilities(
                snapshot,
                input.model_name,
                input.require_final_report || snapshot.delegation_policy.require_final_report,
            );
            overlay.prompt_snapshot_version = version.to_string();
            append_tagged_section(
                &mut prompt,
                "harness_delegation_overlay",
                &build_harness_delegation_overlay_text(&overlay),
            );
            source_order.push("harness_delegation_overlay".to_string());
            harness_capabilities = Some(overlay);

            let surface_contract = SurfacePromptContract {
                session_source: input.session_source.to_string(),
                portal_restricted: input.portal_restricted,
                require_final_report: input.require_final_report
                    || snapshot.delegation_policy.require_final_report,
                document_access_mode: snapshot.document_access_mode.clone(),
                document_scope_mode: snapshot.document_scope_mode.clone(),
                document_write_mode: snapshot.document_write_mode.clone(),
            };
            append_tagged_section(
                &mut prompt,
                "surface_contract_overlay",
                &build_surface_contract_overlay_text(&surface_contract),
            );
            source_order.push("surface_contract_overlay".to_string());
        }
    }

    if let Some(profile_overlay) = input
        .prompt_profile_overlay
        .filter(|value| !value.trim().is_empty())
    {
        append_tagged_section(&mut prompt, "prompt_profile_overlay", profile_overlay);
        source_order.push("prompt_profile_overlay".to_string());
    }

    if let Some(extra) = input
        .session_extra_instructions
        .filter(|value| !value.trim().is_empty())
    {
        append_tagged_section(&mut prompt, "extra_instructions", extra);
        source_order.push("session.extra_instructions".to_string());
    }

    if let Some(turn_instruction) = input
        .turn_system_instruction
        .filter(|value| !value.trim().is_empty())
    {
        append_tagged_section(&mut prompt, "turn_system_instruction", turn_instruction);
        source_order.push("turn_system_instruction".to_string());
    }

    AgentPromptComposition {
        top_level_prompt: prompt,
        report: PromptCompositionReport {
            prompt_snapshot_version: version.to_string(),
            source_order,
            capability_snapshot: input.runtime_snapshot.cloned(),
            delegation_snapshot: input
                .runtime_snapshot
                .map(|snapshot| snapshot.delegation_policy.clone()),
            approval_snapshot: harness_capabilities
                .as_ref()
                .map(|value| value.approval_mode),
            harness_capabilities,
        },
    }
}

fn append_tagged_section(prompt: &mut String, tag: &str, content: &str) {
    if content.trim().is_empty() {
        return;
    }
    prompt.push_str("\n\n<");
    prompt.push_str(tag);
    prompt.push_str(">\n");
    prompt.push_str(content.trim());
    prompt.push_str("\n</");
    prompt.push_str(tag);
    prompt.push('>');
}

fn build_runtime_capability_snapshot_overlay(snapshot: &RuntimeCapabilitySnapshot) -> String {
    let builtin = snapshot
        .extensions
        .builtin_capabilities
        .iter()
        .filter(|item| item.enabled)
        .filter(|item| {
            item.registry.runtime_names.iter().any(|runtime_name| {
                snapshot
                    .extensions
                    .effective_allowed_extension_names
                    .iter()
                    .any(|allowed| allowed == runtime_name)
            })
        })
        .map(|item| item.registry.display_name.clone())
        .collect::<Vec<_>>();
    let session_injected = snapshot
        .extensions
        .session_injected_capabilities
        .iter()
        .map(|entry| entry.display_name.clone())
        .collect::<Vec<_>>();
    let attached_team = snapshot
        .extensions
        .attached_team_extensions
        .iter()
        .filter(|item| item.enabled)
        .map(|item| {
            item.display_name
                .clone()
                .or_else(|| item.runtime_name.clone())
                .unwrap_or_else(|| item.extension_id.clone())
        })
        .collect::<Vec<_>>();
    let custom = snapshot
        .extensions
        .custom_extensions
        .iter()
        .filter(|item| item.enabled)
        .map(|item| item.name.clone())
        .collect::<Vec<_>>();

    [
        "Current runtime capability truth for this session:",
        &format!("- Session source: {}", snapshot.session_source),
        &format!(
            "- Portal restricted: {}",
            yes_no(snapshot.portal_restricted)
        ),
        &format!(
            "- Document access mode: {}",
            snapshot
                .document_access_mode
                .as_deref()
                .unwrap_or("default")
        ),
        &format!(
            "- Document scope: {}",
            snapshot.document_scope_mode.as_deref().unwrap_or("default")
        ),
        &format!(
            "- Document write mode: {}",
            snapshot.document_write_mode.as_deref().unwrap_or("default")
        ),
        &format!(
            "- Built-in capabilities currently enabled: {}",
            join_or_none(&builtin)
        ),
        &format!(
            "- Session-injected capabilities: {}",
            join_or_none(&session_injected)
        ),
        &format!("- Attached team MCPs: {}", join_or_none(&attached_team)),
        &format!("- Attached custom MCPs: {}", join_or_none(&custom)),
        &format!(
            "- Skill binding mode: {}",
            serde_json::to_string(&snapshot.skills.skill_binding_mode)
                .unwrap_or_else(|_| "\"hybrid\"".to_string())
                .trim_matches('"')
        ),
        &format!(
            "- Skill allowlist: {}",
            snapshot
                .skills
                .effective_allowed_skill_ids
                .as_ref()
                .map(|items| items.join(", "))
                .unwrap_or_else(|| "unrestricted".to_string())
        ),
        "Treat this snapshot as authoritative when explaining what is currently available.",
    ]
    .join("\n")
}

fn build_harness_delegation_overlay_text(overlay: &HarnessDelegationOverlay) -> String {
    let mut lines = vec![
        "You are running under AGIME Harness. Treat this section as the authoritative execution contract for this session.".to_string(),
        format!("- Prompt pack version: {}", overlay.prompt_snapshot_version),
        format!("- Tasks capability: {}", enabled_disabled(overlay.tasks_enabled)),
        format!("- Plan mode: {}", enabled_disabled(overlay.plan_enabled)),
        format!("- Subagent delegation: {}", enabled_disabled(overlay.subagent_enabled)),
        format!("- Explicit swarm delegation: {}", enabled_disabled(overlay.swarm_enabled)),
        format!(
            "- Worker peer messaging: {}",
            enabled_disabled(overlay.worker_peer_messaging_enabled)
        ),
        format!(
            "- Auto swarm planner upgrade: {}",
            enabled_disabled(overlay.auto_swarm_enabled)
        ),
        format!(
            "- Validation worker: {}",
            enabled_disabled(overlay.validation_worker_enabled)
        ),
        format!(
            "- Approval mode: {}",
            match overlay.approval_mode {
                ApprovalMode::LeaderOwned => "leader_owned",
                ApprovalMode::HeadlessFallback => "headless_fallback",
            }
        ),
        format!(
            "- Final report required: {}",
            yes_no(overlay.require_final_report)
        ),
    ];
    if let Some(document_access_mode) = overlay.document_access_mode.as_deref() {
        lines.push(format!(
            "- Document access contract: {}",
            document_access_mode
        ));
    }
    if let Some(document_scope_mode) = overlay.document_scope_mode.as_deref() {
        lines.push(format!("- Document scope: {}", document_scope_mode));
    }
    if let Some(document_write_mode) = overlay.document_write_mode.as_deref() {
        lines.push(format!("- Document write mode: {}", document_write_mode));
    }
    lines.extend([
        "Rules:".to_string(),
        "- Never claim a capability that is disabled in this section.".to_string(),
        "- If Tasks is enabled and the work is meaningfully multi-step, use the task board and keep exactly one task in_progress at a time.".to_string(),
        "- If subagent delegation is enabled, you may delegate bounded helper work.".to_string(),
        "- If worker peer messaging is disabled, do not claim that swarm workers can directly message each other.".to_string(),
        "- If explicit swarm is disabled but auto swarm is enabled, do not claim that a direct swarm tool is available; the runtime may still upgrade suitable work automatically.".to_string(),
        "- If validation worker is disabled, do not promise an extra validation worker pass.".to_string(),
        "- If approval mode is leader_owned, describe worker permission requests as going through the leader/coordinator path. Only describe direct policy fallback when approval mode says headless_fallback.".to_string(),
    ]);
    lines.join("\n")
}

fn build_surface_contract_overlay_text(contract: &SurfacePromptContract) -> String {
    let mut lines = vec![match contract.session_source.as_str() {
        "channel_runtime" => "This is an explicit channel execution turn. Treat it as a focused execution step inside a collaboration thread.".to_string(),
        "channel_conversation" => "This is a channel collaboration conversation surface. Continue the thread naturally, as an ongoing work dialogue rather than a one-shot execution task.".to_string(),
        "system" => "This is a system surface. Completion is contract-driven and may be blocked when required content access or validation is missing.".to_string(),
        "portal_manager" => "This is a portal manager surface. Stay within governance and capability-management boundaries.".to_string(),
        "portal_coding" => "This is a portal coding surface. Stay within configured project and capability boundaries while producing real execution.".to_string(),
        "portal" => "This is a portal public/service surface. Respect external-facing capability and document boundaries.".to_string(),
        _ => "This is a standard chat surface. Respond naturally while staying inside the current runtime contract.".to_string(),
    }];

    if contract.portal_restricted {
        lines.push("Portal/session restriction is active. Any overlay can only narrow capabilities, never expand them.".to_string());
    }
    if let Some(document_access_mode) = contract.document_access_mode.as_deref() {
        lines.push(format!(
            "Document access mode for this surface: {}.",
            document_access_mode
        ));
    }
    if let Some(document_scope_mode) = contract.document_scope_mode.as_deref() {
        lines.push(format!(
            "Document scope for this surface: {}.",
            document_scope_mode
        ));
    }
    if let Some(document_write_mode) = contract.document_write_mode.as_deref() {
        lines.push(format!(
            "Document write mode for this surface: {}.",
            document_write_mode
        ));
    }
    if contract.require_final_report {
        lines.push(
            "A structured final report is required before this run can be treated as complete."
                .to_string(),
        );
    }
    lines.push(
        "If the runtime snapshot and older business instructions ever disagree, follow the runtime snapshot and report the limitation plainly."
            .to_string(),
    );
    lines.join("\n")
}

fn join_or_none(values: &[String]) -> String {
    if values.is_empty() {
        "none".to_string()
    } else {
        values.join(", ")
    }
}

fn enabled_disabled(value: bool) -> &'static str {
    if value {
        "enabled"
    } else {
        "disabled"
    }
}

fn yes_no(value: bool) -> &'static str {
    if value {
        "yes"
    } else {
        "no"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::capability_policy::{
        CapabilityDisplayGroup, CapabilityKind, CapabilityRegistryEntry, RuntimeDelivery,
        RuntimeExtensionResolution, RuntimeSkillResolution,
    };
    use agime_team::models::{DelegationPolicy, SkillBindingMode};

    fn sample_snapshot() -> RuntimeCapabilitySnapshot {
        RuntimeCapabilitySnapshot {
            session_source: "chat".to_string(),
            portal_restricted: false,
            document_access_mode: Some("attached_only".to_string()),
            document_scope_mode: Some("attached_only".to_string()),
            document_write_mode: Some("draft_only".to_string()),
            extensions: RuntimeExtensionResolution {
                builtin_capabilities: vec![
                    super::super::capability_policy::ConfiguredBuiltinCapability {
                        extension: agime_team::models::BuiltinExtension::Tasks,
                        enabled: true,
                        registry: CapabilityRegistryEntry {
                            config_key: "tasks".to_string(),
                            display_name: "Tasks".to_string(),
                            kind: CapabilityKind::BuiltinPlatform,
                            display_group: CapabilityDisplayGroup::PlatformExtensions,
                            runtime_delivery: RuntimeDelivery::InProcess,
                            runtime_names: vec!["tasks".to_string()],
                            editable: true,
                            default_enabled: true,
                            hidden_reason: None,
                        },
                    },
                ],
                custom_extensions: Vec::new(),
                attached_team_extensions: Vec::new(),
                legacy_team_extensions: Vec::new(),
                effective_allowed_extension_names: vec![
                    "tasks".to_string(),
                    "developer".to_string(),
                ],
                session_injected_capabilities: Vec::new(),
            },
            skills: RuntimeSkillResolution {
                assigned_skills: Vec::new(),
                skill_binding_mode: SkillBindingMode::Hybrid,
                effective_allowed_skill_ids: None,
            },
            delegation_policy: DelegationPolicy::default(),
            session_delegation_policy_override: None,
            portal_delegation_policy_override: None,
        }
    }

    #[test]
    fn backup_manifest_covers_core_prompt_sources() {
        let manifest = prompt_backup_manifest();
        let paths = manifest
            .sources
            .iter()
            .map(|item| item.path.as_str())
            .collect::<Vec<_>>();
        assert!(paths.contains(&"crates/agime/src/prompts/system.md"));
        assert!(paths.contains(&"crates/agime/src/prompts/leader_system.md"));
        assert!(paths.contains(&"crates/agime/src/prompts/worker_system.md"));
        assert!(paths.contains(&"crates/agime/src/prompts/validation_worker_system.md"));
        assert!(paths.contains(&"TeamAgent.system_prompt"));
        assert!(paths.contains(&"AgentSessionDoc.extra_instructions"));
    }

    #[test]
    fn v2_prompt_adds_harness_and_surface_overlays() {
        let extension = ExtensionInfo::new("tasks", "Structured task tracking", false);
        let snapshot = sample_snapshot();
        let composition = compose_top_level_prompt_with_version(
            AgentPromptComposerInput {
                extensions: &[extension],
                custom_prompt: Some("Be concise."),
                runtime_snapshot: Some(&snapshot),
                session_extra_instructions: Some("Session-specific note."),
                prompt_profile_overlay: Some("Profile note."),
                turn_system_instruction: Some("Turn note."),
                session_source: "chat",
                portal_restricted: false,
                require_final_report: false,
                model_name: "glm-5.1",
            },
            PromptPackVersion::V2,
        );

        assert!(composition
            .top_level_prompt
            .contains("<runtime_capability_snapshot>"));
        assert!(composition
            .top_level_prompt
            .contains("<harness_delegation_overlay>"));
        assert!(composition
            .top_level_prompt
            .contains("Worker peer messaging"));
        assert!(composition
            .top_level_prompt
            .contains("Approval mode: leader_owned"));
        assert!(composition
            .top_level_prompt
            .contains("<surface_contract_overlay>"));
        assert!(composition
            .top_level_prompt
            .contains("<prompt_profile_overlay>"));
        assert!(composition
            .top_level_prompt
            .contains("<extra_instructions>"));
        assert!(composition
            .top_level_prompt
            .contains("<turn_system_instruction>"));
    }

    #[test]
    fn v2_prompt_reports_swarm_disabled_when_native_tool_is_off() {
        std::env::remove_var("AGIME_ENABLE_NATIVE_SWARM_TOOL");
        std::env::remove_var("AGIME_ENABLE_SWARM_PLANNER_AUTO");
        let overlay = resolve_harness_capabilities(&sample_snapshot(), "glm-5.1", false);
        assert!(!overlay.swarm_enabled);
        assert!(!overlay.worker_peer_messaging_enabled);
        assert!(!overlay.auto_swarm_enabled);
    }

    #[test]
    fn v1_prompt_preserves_legacy_shape_without_new_overlays() {
        let extension = ExtensionInfo::new("tasks", "Structured task tracking", false);
        let snapshot = sample_snapshot();
        let composition = compose_top_level_prompt_with_version(
            AgentPromptComposerInput {
                extensions: &[extension],
                custom_prompt: None,
                runtime_snapshot: Some(&snapshot),
                session_extra_instructions: None,
                prompt_profile_overlay: None,
                turn_system_instruction: None,
                session_source: "chat",
                portal_restricted: false,
                require_final_report: false,
                model_name: "glm-5.1",
            },
            PromptPackVersion::V1,
        );

        assert!(!composition
            .top_level_prompt
            .contains("<runtime_capability_snapshot>"));
        assert!(!composition
            .top_level_prompt
            .contains("<harness_delegation_overlay>"));
        assert_eq!(composition.report.prompt_snapshot_version, "v1");
    }
}
