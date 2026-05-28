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
//! What's *deliberately* trimmed compared to team-server:
//! - `RuntimeCapabilitySnapshot` / `HarnessDelegationOverlay` /
//!   `SurfacePromptContract` overlays — these depend on
//!   `agime_team::models::{ApprovalMode, DelegationPolicy, SkillBindingMode}`
//!   types that the desktop doesn't have. Callers can supply pre-rendered
//!   text for those slots via `runtime_overlay_text` and
//!   `surface_contract_overlay_text` if a future desktop story grows the
//!   concept.
//!
//! SOURCE: crates/agime-runtime/src/prompt_composer.rs at commit 961109f.
//! Keep in sync manually — see CLAUDE.md long-term maintenance strategy.

#![allow(dead_code)]

use agime::agents::extension::ExtensionInfo;
use agime::agents::subagent_tool::should_enable_subagents;
use agime::prompt_template;
use chrono::Local;
use serde::{Deserialize, Serialize};

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
