//! Capability data types shared between the team-server resolver and the
//! runtime-side prompt composer.
//!
//! This module holds **only** the value-typed data shapes — no resolver logic.
//! The resolver itself (`AgentRuntimePolicyResolver`) still lives in
//! `agime-team-server::agent::capability_policy` and is moved into the runtime
//! in a later batch.

use serde::{Deserialize, Serialize};

use agime_team::models::{
    AgentExtensionConfig, AgentSkillConfig, AttachedTeamExtensionRef, BuiltinExtension,
    CustomExtensionConfig, DelegationPolicy, DelegationPolicyOverride, SkillBindingMode,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DocumentScopeMode {
    AttachedOnly,
    ChannelBound,
    PortalBound,
    Full,
}

impl DocumentScopeMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::AttachedOnly => "attached_only",
            Self::ChannelBound => "channel_bound",
            Self::PortalBound => "portal_bound",
            Self::Full => "full",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DocumentWriteMode {
    ReadOnly,
    DraftOnly,
    ControlledWrite,
    FullWrite,
}

impl DocumentWriteMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::ReadOnly => "read_only",
            Self::DraftOnly => "draft_only",
            Self::ControlledWrite => "controlled_write",
            Self::FullWrite => "full_write",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedDocumentPolicy {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub document_access_mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub document_scope_mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub document_write_mode: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityKind {
    BuiltinPlatform,
    BuiltinMcp,
    TeamMcp,
    CustomMcp,
    Skill,
    SystemReserved,
    SessionInjected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityDisplayGroup {
    PlatformExtensions,
    McpServices,
    CustomExtensions,
    Skills,
    SystemInjected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeDelivery {
    InProcess,
    SubprocessMcp,
    SessionInjected,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityRegistryEntry {
    pub config_key: String,
    pub display_name: String,
    pub kind: CapabilityKind,
    pub display_group: CapabilityDisplayGroup,
    pub runtime_delivery: RuntimeDelivery,
    pub runtime_names: Vec<String>,
    pub editable: bool,
    pub default_enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hidden_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfiguredBuiltinCapability {
    pub extension: BuiltinExtension,
    pub enabled: bool,
    pub registry: CapabilityRegistryEntry,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeExtensionResolution {
    pub builtin_capabilities: Vec<ConfiguredBuiltinCapability>,
    pub custom_extensions: Vec<CustomExtensionConfig>,
    pub attached_team_extensions: Vec<AttachedTeamExtensionRef>,
    pub legacy_team_extensions: Vec<CustomExtensionConfig>,
    pub effective_allowed_extension_names: Vec<String>,
    pub session_injected_capabilities: Vec<CapabilityRegistryEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeSkillResolution {
    pub assigned_skills: Vec<AgentSkillConfig>,
    pub skill_binding_mode: SkillBindingMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effective_allowed_skill_ids: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeCapabilitySnapshot {
    pub session_source: String,
    pub portal_restricted: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub document_access_mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub document_scope_mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub document_write_mode: Option<String>,
    pub extensions: RuntimeExtensionResolution,
    pub skills: RuntimeSkillResolution,
    pub delegation_policy: DelegationPolicy,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_delegation_policy_override: Option<DelegationPolicyOverride>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub portal_delegation_policy_override: Option<DelegationPolicyOverride>,
}

impl RuntimeCapabilitySnapshot {
    pub fn runtime_builtin_extensions(&self) -> Vec<AgentExtensionConfig> {
        self.extensions
            .builtin_capabilities
            .iter()
            .filter(|item| item.enabled && item.registry.editable)
            .filter(|item| {
                item.registry.runtime_names.iter().any(|runtime_name| {
                    self.extensions
                        .effective_allowed_extension_names
                        .iter()
                        .any(|allowed| allowed == runtime_name)
                })
            })
            .map(|item| AgentExtensionConfig {
                extension: item.extension,
                enabled: item.enabled,
                allowed_groups: Vec::new(),
            })
            .collect()
    }

    pub fn runtime_custom_extensions(&self) -> Vec<CustomExtensionConfig> {
        self.extensions
            .custom_extensions
            .iter()
            .filter(|extension| {
                self.extensions
                    .effective_allowed_extension_names
                    .iter()
                    .any(|allowed| allowed.eq_ignore_ascii_case(&extension.name))
            })
            .cloned()
            .collect()
    }

    pub fn runtime_team_extension_refs(&self) -> Vec<AttachedTeamExtensionRef> {
        self.extensions
            .attached_team_extensions
            .iter()
            .filter(|reference| {
                reference
                    .runtime_name
                    .as_deref()
                    .map(normalize_runtime_name)
                    .map(|runtime_name| {
                        self.extensions
                            .effective_allowed_extension_names
                            .iter()
                            .any(|allowed| allowed == &runtime_name)
                    })
                    .unwrap_or(false)
            })
            .cloned()
            .collect()
    }

    pub fn legacy_team_custom_extensions(&self) -> Vec<CustomExtensionConfig> {
        self.extensions
            .legacy_team_extensions
            .iter()
            .filter(|extension| {
                self.extensions
                    .effective_allowed_extension_names
                    .iter()
                    .any(|allowed| allowed.eq_ignore_ascii_case(&extension.name))
            })
            .cloned()
            .collect()
    }
}

pub fn normalize_runtime_name(value: &str) -> String {
    let normalized = value.trim().to_ascii_lowercase().replace('-', "_");
    match normalized.as_str() {
        "todo" => "tasks".to_string(),
        other => other.to_string(),
    }
}
