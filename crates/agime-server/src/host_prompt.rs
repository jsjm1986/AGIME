//! Top-level prompt composer (desktop side).
//!
//! Mirror of `agime-runtime::prompt_composer` adapted for the desktop crate.
//! Per the dual-track plan, desktop does **not** depend on `agime-runtime`,
//! so this file owns its own copy. The shape of the API (base prompt + tagged
//! overlays + `source_order` tracking) is identical to the team-server
//! version so future merges stay mechanical.
//!
//! What's included:
//! - `build_base_business_prompt` — renders `system.md` with the canonical
//!   context block, then appends an `<agent_instructions>` overlay if the
//!   caller passes a custom prompt.
//! - `compose_top_level_prompt` — wraps the base prompt with a stable set of
//!   optional overlays (`prompt_profile_overlay`, `extra_instructions`,
//!   `turn_system_instruction`) and reports the source order for
//!   introspection.
//!
//! V2 overlay self-composition (desktop equivalents):
//! - [`PromptCapabilitySnapshot`] — flattened mirror of team-server's
//!   `RuntimeCapabilitySnapshot` (extension display names already resolved
//!   to `Vec<String>`, no `agime_team::models::*` deps).
//! - [`PromptHarnessOverlay`] — mirror of `HarnessDelegationOverlay`.
//! - [`PromptSurfaceContract`] — mirror of `SurfacePromptContract`.
//! - [`PromptApprovalMode`] — mirror of `agime_team::models::ApprovalMode`
//!   (`LeaderOwned` / `HeadlessFallback`). Distinct from the desktop UI's
//!   `host_capability::ApprovalMode` (`Auto`/`Approve`/`Manual`), which
//!   models tool-call approval policy rather than delegation ownership.
//!
//! The corresponding free functions [`build_runtime_capability_snapshot_overlay`],
//! [`build_harness_delegation_overlay_text`], and
//! [`build_surface_contract_overlay_text`] are byte-for-byte ports of the
//! team-server builders, so the desktop reply path can self-compose V2
//! overlays without depending on externally pre-rendered text.
//!
//! SOURCE: crates/agime-runtime/src/prompt_composer.rs at commit 961109f.
//! Keep in sync manually — see CLAUDE.md long-term maintenance strategy.

#![allow(dead_code)]

use agime::agents::extension::ExtensionInfo;
use agime::agents::subagent_tool::should_enable_subagents;
use agime::prompt_template;
use chrono::Local;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

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

impl std::fmt::Display for PromptPackVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptCompositionReport {
    pub prompt_snapshot_version: String,
    pub source_order: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct AgentPromptComposition {
    pub top_level_prompt: String,
    pub report: PromptCompositionReport,
}

/// Inputs for the desktop top-level composer. Optional fields default to
/// `None`; only `extensions` and `model_name` are required to render the
/// base prompt.
#[derive(Debug, Clone, Default)]
pub struct AgentPromptComposerInput<'a> {
    pub extensions: &'a [ExtensionInfo],
    pub custom_prompt: Option<&'a str>,
    pub session_extra_instructions: Option<&'a str>,
    pub prompt_profile_overlay: Option<&'a str>,
    pub turn_system_instruction: Option<&'a str>,
    /// Pre-rendered runtime-snapshot overlay (team-server passes the result
    /// of `build_runtime_capability_snapshot_overlay` here). Desktop leaves
    /// it `None` until the concept is ported.
    pub runtime_overlay_text: Option<&'a str>,
    /// Pre-rendered harness-delegation overlay text. Desktop leaves it
    /// `None` until the concept is ported.
    pub harness_delegation_overlay_text: Option<&'a str>,
    /// Pre-rendered surface-contract overlay text. Desktop leaves it `None`
    /// until the concept is ported.
    pub surface_contract_overlay_text: Option<&'a str>,
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

    if version == PromptPackVersion::V2 {
        if let Some(text) = input
            .runtime_overlay_text
            .filter(|value| !value.trim().is_empty())
        {
            append_tagged_section(&mut prompt, "runtime_capability_snapshot", text);
            source_order.push("runtime_capability_snapshot".to_string());
        }
        if let Some(text) = input
            .harness_delegation_overlay_text
            .filter(|value| !value.trim().is_empty())
        {
            append_tagged_section(&mut prompt, "harness_delegation_overlay", text);
            source_order.push("harness_delegation_overlay".to_string());
        }
        if let Some(text) = input
            .surface_contract_overlay_text
            .filter(|value| !value.trim().is_empty())
        {
            append_tagged_section(&mut prompt, "surface_contract_overlay", text);
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
        },
    }
}

/// Render only the session-scoped overlay sections (without re-rendering the
/// base `system.md`). The result is suitable for either
/// [`agime::agents::Agent::extend_system_prompt`] (session-static channel) or
/// [`agime::agents::Agent::extend_host_override_extras`] (per-turn channel)
/// — pick the channel that matches the caller's lifecycle.
///
/// Returns `None` when no overlay text is present after trimming, so the
/// caller can skip the call entirely.
pub fn compose_session_overlays_only(input: AgentPromptComposerInput<'_>) -> Option<String> {
    compose_session_overlays_only_with_version(input, PromptPackVersion::from_env())
}

pub fn compose_session_overlays_only_with_version(
    input: AgentPromptComposerInput<'_>,
    version: PromptPackVersion,
) -> Option<String> {
    let mut buf = String::new();

    if version == PromptPackVersion::V2 {
        if let Some(text) = input
            .runtime_overlay_text
            .filter(|value| !value.trim().is_empty())
        {
            append_tagged_section(&mut buf, "runtime_capability_snapshot", text);
        }
        if let Some(text) = input
            .harness_delegation_overlay_text
            .filter(|value| !value.trim().is_empty())
        {
            append_tagged_section(&mut buf, "harness_delegation_overlay", text);
        }
        if let Some(text) = input
            .surface_contract_overlay_text
            .filter(|value| !value.trim().is_empty())
        {
            append_tagged_section(&mut buf, "surface_contract_overlay", text);
        }
    }

    if let Some(profile_overlay) = input
        .prompt_profile_overlay
        .filter(|value| !value.trim().is_empty())
    {
        append_tagged_section(&mut buf, "prompt_profile_overlay", profile_overlay);
    }

    if let Some(extra) = input
        .session_extra_instructions
        .filter(|value| !value.trim().is_empty())
    {
        append_tagged_section(&mut buf, "extra_instructions", extra);
    }

    if let Some(turn_instruction) = input
        .turn_system_instruction
        .filter(|value| !value.trim().is_empty())
    {
        append_tagged_section(&mut buf, "turn_system_instruction", turn_instruction);
    }

    let trimmed = buf.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
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

// ---------------------------------------------------------------------------
// V2 overlay self-composition — desktop value types + byte-for-byte ports of
// the team-server overlay builders. Callers in the desktop reply path build
// these structs from session metadata and feed the rendered text to
// `AgentPromptComposerInput::{runtime_overlay_text, harness_delegation_overlay_text, surface_contract_overlay_text}`.
//
// SOURCE: crates/agime-runtime/src/prompt_composer.rs at commit 961109f.
// Keep in sync manually — see CLAUDE.md long-term maintenance strategy.
// ---------------------------------------------------------------------------

/// Approval mode for delegation ownership. Mirrors
/// `agime_team::models::ApprovalMode`, *not* the desktop UI's three-state
/// permission policy in [`crate::host_capability::ApprovalMode`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum PromptApprovalMode {
    #[default]
    LeaderOwned,
    HeadlessFallback,
}

/// Flattened, render-ready mirror of team-server's
/// `RuntimeCapabilitySnapshot`. Extension display names are pre-resolved so
/// the builder has no dependency on `agime_team::models`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PromptCapabilitySnapshot {
    pub session_source: String,
    #[serde(default)]
    pub portal_restricted: bool,
    #[serde(default)]
    pub document_access_mode: Option<String>,
    #[serde(default)]
    pub document_scope_mode: Option<String>,
    #[serde(default)]
    pub document_write_mode: Option<String>,
    #[serde(default)]
    pub builtin_capabilities: Vec<String>,
    #[serde(default)]
    pub session_injected_capabilities: Vec<String>,
    #[serde(default)]
    pub attached_team_extensions: Vec<String>,
    #[serde(default)]
    pub custom_extensions: Vec<String>,
    #[serde(default)]
    pub skill_binding_mode: Option<String>,
    #[serde(default)]
    pub effective_allowed_skill_ids: Option<Vec<String>>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PromptHarnessOverlay {
    pub prompt_snapshot_version: String,
    pub session_source: String,
    pub tasks_enabled: bool,
    pub plan_enabled: bool,
    pub subagent_enabled: bool,
    pub swarm_enabled: bool,
    pub worker_peer_messaging_enabled: bool,
    pub auto_swarm_enabled: bool,
    pub validation_worker_enabled: bool,
    pub approval_mode: PromptApprovalMode,
    pub require_final_report: bool,
    #[serde(default)]
    pub document_access_mode: Option<String>,
    #[serde(default)]
    pub document_scope_mode: Option<String>,
    #[serde(default)]
    pub document_write_mode: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PromptSurfaceContract {
    pub session_source: String,
    pub portal_restricted: bool,
    pub require_final_report: bool,
    #[serde(default)]
    pub document_access_mode: Option<String>,
    #[serde(default)]
    pub document_scope_mode: Option<String>,
    #[serde(default)]
    pub document_write_mode: Option<String>,
}

pub fn build_runtime_capability_snapshot_overlay(snapshot: &PromptCapabilitySnapshot) -> String {
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
            join_or_none(&snapshot.builtin_capabilities)
        ),
        &format!(
            "- Session-injected capabilities: {}",
            join_or_none(&snapshot.session_injected_capabilities)
        ),
        &format!(
            "- Attached team MCPs: {}",
            join_or_none(&snapshot.attached_team_extensions)
        ),
        &format!(
            "- Attached custom MCPs: {}",
            join_or_none(&snapshot.custom_extensions)
        ),
        &format!(
            "- Skill binding mode: {}",
            snapshot.skill_binding_mode.as_deref().unwrap_or("hybrid")
        ),
        &format!(
            "- Skill allowlist: {}",
            snapshot
                .effective_allowed_skill_ids
                .as_ref()
                .map(|items| items.join(", "))
                .unwrap_or_else(|| "unrestricted".to_string())
        ),
        "Treat this snapshot as authoritative when explaining what is currently available.",
    ]
    .join("\n")
}

pub fn build_harness_delegation_overlay_text(overlay: &PromptHarnessOverlay) -> String {
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
                PromptApprovalMode::LeaderOwned => "leader_owned",
                PromptApprovalMode::HeadlessFallback => "headless_fallback",
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
        "- Task-board updates are not execution. After creating or updating tasks for a user request, continue with the real work using the appropriate non-task tools.".to_string(),
        "- Before a final reply, close completed task-board work as completed; if work cannot finish, leave the board truthful and explain the blocker instead of leaving stale in_progress work.".to_string(),
        "- If subagent delegation is enabled, you may delegate bounded helper work.".to_string(),
        "- If worker peer messaging is disabled, do not claim that swarm workers can directly message each other.".to_string(),
        "- If explicit swarm is disabled but auto swarm is enabled, do not claim that a direct swarm tool is available; the runtime may still upgrade suitable work automatically.".to_string(),
        "- If validation worker is disabled, do not promise an extra validation worker pass.".to_string(),
        "- If approval mode is leader_owned, describe worker permission requests as going through the leader/coordinator path. Only describe direct policy fallback when approval mode says headless_fallback.".to_string(),
    ]);
    lines.join("\n")
}

pub fn build_surface_contract_overlay_text(contract: &PromptSurfaceContract) -> String {
    let mut lines = vec![match contract.session_source.as_str() {
        "automation_builder" => "This is an Agentify builder surface. Stay in direct builder mode, validate real API paths, and do not use delegation or swarm.".to_string(),
        "automation_runtime" => "This is an Agentify published app runtime surface. Treat session creation as initialization only, and only act when there is an actual user request in the current turn.".to_string(),
        "channel_runtime" => "This is an explicit channel execution turn. Treat it as a focused execution step inside a collaboration thread.".to_string(),
        "channel_conversation" => "This is a channel collaboration conversation surface. Continue the thread naturally, as an ongoing work dialogue rather than a one-shot execution task.".to_string(),
        "system" => "This is a system surface. Completion is contract-driven and may be blocked when required content access or validation is missing.".to_string(),
        "document_analysis" => "This is a document analysis surface. Use the provided workspace file path as the document input and complete the structured analysis from real file content.".to_string(),
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

fn yes_no(value: bool) -> &'static str {
    if value {
        "yes"
    } else {
        "no"
    }
}

fn enabled_disabled(value: bool) -> &'static str {
    if value {
        "enabled"
    } else {
        "disabled"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_round_trips_through_str() {
        assert_eq!(PromptPackVersion::V1.as_str(), "v1");
        assert_eq!(PromptPackVersion::V2.as_str(), "v2");
        assert_eq!(format!("{}", PromptPackVersion::V2), "v2");
    }

    #[test]
    fn v2_attaches_overlays_in_canonical_order() {
        let extensions: [ExtensionInfo; 0] = [];
        let composition = compose_top_level_prompt_with_version(
            AgentPromptComposerInput {
                extensions: &extensions,
                custom_prompt: Some("Be concise."),
                session_extra_instructions: Some("session note"),
                prompt_profile_overlay: Some("profile note"),
                turn_system_instruction: Some("turn note"),
                runtime_overlay_text: Some("runtime overlay"),
                harness_delegation_overlay_text: Some("harness overlay"),
                surface_contract_overlay_text: Some("surface overlay"),
                model_name: "gpt-4o",
            },
            PromptPackVersion::V2,
        );

        let prompt = &composition.top_level_prompt;
        assert!(prompt.contains("<agent_instructions>"));
        assert!(prompt.contains("<runtime_capability_snapshot>"));
        assert!(prompt.contains("<harness_delegation_overlay>"));
        assert!(prompt.contains("<surface_contract_overlay>"));
        assert!(prompt.contains("<prompt_profile_overlay>"));
        assert!(prompt.contains("<extra_instructions>"));
        assert!(prompt.contains("<turn_system_instruction>"));

        // source order should follow append order
        let sources = &composition.report.source_order;
        let position = |needle: &str| sources.iter().position(|s| s == needle);
        assert!(position("embedded:system.md") < position("agent.system_prompt"));
        assert!(position("agent.system_prompt") < position("runtime_capability_snapshot"));
        assert!(position("runtime_capability_snapshot") < position("harness_delegation_overlay"));
        assert!(position("harness_delegation_overlay") < position("surface_contract_overlay"));
        assert!(position("surface_contract_overlay") < position("prompt_profile_overlay"));
        assert!(position("prompt_profile_overlay") < position("session.extra_instructions"));
        assert!(position("session.extra_instructions") < position("turn_system_instruction"));
    }

    #[test]
    fn v1_skips_runtime_overlays_even_if_provided() {
        let extensions: [ExtensionInfo; 0] = [];
        let composition = compose_top_level_prompt_with_version(
            AgentPromptComposerInput {
                extensions: &extensions,
                custom_prompt: None,
                runtime_overlay_text: Some("runtime overlay"),
                harness_delegation_overlay_text: Some("harness overlay"),
                surface_contract_overlay_text: Some("surface overlay"),
                model_name: "gpt-4o",
                ..Default::default()
            },
            PromptPackVersion::V1,
        );

        assert!(!composition
            .top_level_prompt
            .contains("<runtime_capability_snapshot>"));
        assert!(!composition
            .top_level_prompt
            .contains("<harness_delegation_overlay>"));
        assert!(!composition
            .top_level_prompt
            .contains("<surface_contract_overlay>"));
        assert_eq!(composition.report.prompt_snapshot_version, "v1");
    }

    #[test]
    fn overlays_only_emits_tagged_sections_without_base_prompt() {
        let extensions: [ExtensionInfo; 0] = [];
        let text = compose_session_overlays_only_with_version(
            AgentPromptComposerInput {
                extensions: &extensions,
                custom_prompt: Some("ignored — base prompt is not rendered here"),
                session_extra_instructions: Some("session note"),
                prompt_profile_overlay: Some("profile note"),
                turn_system_instruction: Some("turn note"),
                runtime_overlay_text: Some("runtime overlay"),
                harness_delegation_overlay_text: Some("harness overlay"),
                surface_contract_overlay_text: Some("surface overlay"),
                model_name: "gpt-4o",
            },
            PromptPackVersion::V2,
        )
        .expect("overlays present");

        assert!(text.contains("<runtime_capability_snapshot>"));
        assert!(text.contains("<harness_delegation_overlay>"));
        assert!(text.contains("<surface_contract_overlay>"));
        assert!(text.contains("<prompt_profile_overlay>"));
        assert!(text.contains("<extra_instructions>"));
        assert!(text.contains("<turn_system_instruction>"));
        // base prompt must NOT be re-rendered
        assert!(!text.contains("<agent_instructions>"));
    }

    #[test]
    fn overlays_only_returns_none_when_all_overlays_empty() {
        let extensions: [ExtensionInfo; 0] = [];
        let result = compose_session_overlays_only_with_version(
            AgentPromptComposerInput {
                extensions: &extensions,
                session_extra_instructions: Some("   "),
                prompt_profile_overlay: Some(""),
                turn_system_instruction: None,
                model_name: "gpt-4o",
                ..Default::default()
            },
            PromptPackVersion::V2,
        );
        assert!(result.is_none());
    }

    #[test]
    fn overlays_only_v1_skips_runtime_pack() {
        let extensions: [ExtensionInfo; 0] = [];
        let result = compose_session_overlays_only_with_version(
            AgentPromptComposerInput {
                extensions: &extensions,
                runtime_overlay_text: Some("runtime overlay"),
                harness_delegation_overlay_text: Some("harness overlay"),
                surface_contract_overlay_text: Some("surface overlay"),
                model_name: "gpt-4o",
                ..Default::default()
            },
            PromptPackVersion::V1,
        );
        assert!(result.is_none());
    }

    #[test]
    fn capability_snapshot_overlay_lists_extensions_and_skill_state() {
        let snapshot = PromptCapabilitySnapshot {
            session_source: "channel_conversation".to_string(),
            portal_restricted: true,
            document_access_mode: Some("attached_only".to_string()),
            builtin_capabilities: vec!["Tasks".to_string(), "Plan".to_string()],
            attached_team_extensions: vec!["weather".to_string()],
            skill_binding_mode: Some("strict".to_string()),
            effective_allowed_skill_ids: Some(vec!["s1".to_string(), "s2".to_string()]),
            ..Default::default()
        };
        let text = build_runtime_capability_snapshot_overlay(&snapshot);
        assert!(text.contains("Session source: channel_conversation"));
        assert!(text.contains("Portal restricted: yes"));
        assert!(text.contains("Document access mode: attached_only"));
        assert!(text.contains("Built-in capabilities currently enabled: Tasks, Plan"));
        assert!(text.contains("Attached team MCPs: weather"));
        assert!(text.contains("Attached custom MCPs: none"));
        assert!(text.contains("Skill binding mode: strict"));
        assert!(text.contains("Skill allowlist: s1, s2"));
    }

    #[test]
    fn capability_snapshot_overlay_falls_back_to_defaults() {
        let snapshot = PromptCapabilitySnapshot {
            session_source: "chat".to_string(),
            ..Default::default()
        };
        let text = build_runtime_capability_snapshot_overlay(&snapshot);
        assert!(text.contains("Portal restricted: no"));
        assert!(text.contains("Document access mode: default"));
        assert!(text.contains("Skill binding mode: hybrid"));
        assert!(text.contains("Skill allowlist: unrestricted"));
    }

    #[test]
    fn harness_overlay_renders_required_lines() {
        let overlay = PromptHarnessOverlay {
            prompt_snapshot_version: "v2".to_string(),
            session_source: "chat".to_string(),
            tasks_enabled: true,
            plan_enabled: false,
            subagent_enabled: true,
            swarm_enabled: false,
            worker_peer_messaging_enabled: false,
            auto_swarm_enabled: true,
            validation_worker_enabled: true,
            approval_mode: PromptApprovalMode::HeadlessFallback,
            require_final_report: true,
            document_access_mode: Some("read_only".to_string()),
            ..Default::default()
        };
        let text = build_harness_delegation_overlay_text(&overlay);
        assert!(text.contains("AGIME Harness"));
        assert!(text.contains("Prompt pack version: v2"));
        assert!(text.contains("Tasks capability: enabled"));
        assert!(text.contains("Plan mode: disabled"));
        assert!(text.contains("Approval mode: headless_fallback"));
        assert!(text.contains("Final report required: yes"));
        assert!(text.contains("Document access contract: read_only"));
        assert!(text.contains("Rules:"));
    }

    #[test]
    fn surface_contract_renders_session_source_branches() {
        let portal_coding = build_surface_contract_overlay_text(&PromptSurfaceContract {
            session_source: "portal_coding".to_string(),
            portal_restricted: true,
            require_final_report: true,
            ..Default::default()
        });
        assert!(portal_coding.contains("portal coding surface"));
        assert!(portal_coding.contains("Portal/session restriction is active"));
        assert!(portal_coding.contains("structured final report is required"));

        let unknown = build_surface_contract_overlay_text(&PromptSurfaceContract {
            session_source: "weird_surface".to_string(),
            ..Default::default()
        });
        assert!(unknown.contains("standard chat surface"));
    }

    #[test]
    fn helpers_format_optional_lists_and_flags() {
        assert_eq!(join_or_none(&[]), "none");
        assert_eq!(join_or_none(&["a".to_string(), "b".to_string()]), "a, b");
        assert_eq!(yes_no(true), "yes");
        assert_eq!(yes_no(false), "no");
        assert_eq!(enabled_disabled(true), "enabled");
        assert_eq!(enabled_disabled(false), "disabled");
    }

    #[test]
    fn empty_overlays_dont_inject_blank_tags() {
        let extensions: [ExtensionInfo; 0] = [];
        let composition = compose_top_level_prompt_with_version(
            AgentPromptComposerInput {
                extensions: &extensions,
                custom_prompt: Some("   "),
                session_extra_instructions: Some(""),
                prompt_profile_overlay: Some("\n  \n"),
                turn_system_instruction: None,
                model_name: "gpt-4o",
                ..Default::default()
            },
            PromptPackVersion::V2,
        );

        assert!(!composition
            .top_level_prompt
            .contains("<agent_instructions>"));
        assert!(!composition
            .top_level_prompt
            .contains("<extra_instructions>"));
        assert!(!composition
            .top_level_prompt
            .contains("<prompt_profile_overlay>"));
        assert_eq!(
            composition.report.source_order,
            vec!["embedded:system.md".to_string()]
        );
    }
}
