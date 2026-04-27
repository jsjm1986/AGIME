use std::collections::HashSet;

use agime_team::models::mongo::{PortalDocumentAccessMode, PortalEffectivePublicConfig};
use agime_team::models::{
    AgentExtensionConfig, AgentSkillConfig, AttachedTeamExtensionRef, BuiltinExtension,
    CustomExtensionConfig, DelegationPolicy, DelegationPolicyOverride, SkillBindingMode, TeamAgent,
};
use serde::{Deserialize, Serialize};

use super::session_mongo::AgentSessionDoc;

pub fn is_non_delegating_session_source(session_source: &str) -> bool {
    session_source.eq_ignore_ascii_case("portal_manager")
        || session_source.eq_ignore_ascii_case("scheduled_task")
        || session_source.eq_ignore_ascii_case("automation_builder")
}

fn post_filter_allowed_extensions_for_session_source(
    session_source: &str,
    mut effective_allowed_extension_names: Vec<String>,
) -> Vec<String> {
    if session_source.eq_ignore_ascii_case("scheduled_task") {
        effective_allowed_extension_names.retain(|runtime_name| runtime_name != "tasks");
    }
    effective_allowed_extension_names
}

fn explicitly_requestable_platform_runtime_names(session_source: &str) -> Vec<String> {
    let mut values = Vec::new();
    if !session_source.eq_ignore_ascii_case("scheduled_task") {
        values.push("api_tools".to_string());
    }
    values
}

pub fn source_delegation_override_for_session_source(
    session_source: &str,
) -> Option<DelegationPolicyOverride> {
    if !is_non_delegating_session_source(session_source) {
        return None;
    }

    Some(DelegationPolicyOverride {
        allow_plan: Some(false),
        allow_subagent: Some(false),
        allow_swarm: Some(false),
        allow_worker_messaging: Some(false),
        allow_auto_swarm: Some(false),
        allow_validation_worker: Some(false),
        ..DelegationPolicyOverride::default()
    })
}

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

fn parse_document_scope_mode(raw: Option<&str>) -> Option<DocumentScopeMode> {
    match raw
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_ascii_lowercase())
        .as_deref()
    {
        Some("attached_only" | "attached-only" | "attachedonly") => {
            Some(DocumentScopeMode::AttachedOnly)
        }
        Some("channel_bound" | "channel-bound" | "channelbound") => {
            Some(DocumentScopeMode::ChannelBound)
        }
        Some("portal_bound" | "portal-bound" | "portalbound") => {
            Some(DocumentScopeMode::PortalBound)
        }
        Some("full") => Some(DocumentScopeMode::Full),
        _ => None,
    }
}

fn parse_document_write_mode(raw: Option<&str>) -> Option<DocumentWriteMode> {
    match raw
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_ascii_lowercase())
        .as_deref()
    {
        Some("read_only" | "read-only" | "readonly") => Some(DocumentWriteMode::ReadOnly),
        Some("draft_only" | "draft-only" | "draftonly") => Some(DocumentWriteMode::DraftOnly),
        Some("controlled_write" | "controlled-write" | "controlledwrite") => {
            Some(DocumentWriteMode::ControlledWrite)
        }
        Some("full_write" | "full-write" | "fullwrite") => Some(DocumentWriteMode::FullWrite),
        _ => None,
    }
}

fn default_scope_for_session_source(
    session_source: Option<&str>,
    portal_restricted: bool,
) -> DocumentScopeMode {
    if portal_restricted {
        return DocumentScopeMode::PortalBound;
    }
    match session_source
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_ascii_lowercase())
        .as_deref()
    {
        Some("portal" | "portal_coding" | "portal_manager") => DocumentScopeMode::PortalBound,
        _ => DocumentScopeMode::Full,
    }
}

fn legacy_access_mode_from_policy(
    scope_mode: DocumentScopeMode,
    write_mode: DocumentWriteMode,
) -> String {
    match (scope_mode, write_mode) {
        (DocumentScopeMode::AttachedOnly, DocumentWriteMode::DraftOnly) => {
            "attached_only".to_string()
        }
        (_, DocumentWriteMode::ReadOnly) => "read_only".to_string(),
        (_, DocumentWriteMode::DraftOnly) => "co_edit_draft".to_string(),
        (_, DocumentWriteMode::ControlledWrite) => "controlled_write".to_string(),
        (DocumentScopeMode::Full, DocumentWriteMode::FullWrite) => "full".to_string(),
        (_, DocumentWriteMode::FullWrite) => "full".to_string(),
    }
}

pub fn resolve_document_policy(
    legacy_document_access_mode: Option<&str>,
    explicit_scope_mode: Option<&str>,
    explicit_write_mode: Option<&str>,
    session_source: Option<&str>,
    portal_restricted: bool,
) -> ResolvedDocumentPolicy {
    let parsed_scope = parse_document_scope_mode(explicit_scope_mode);
    let parsed_write = parse_document_write_mode(explicit_write_mode);

    let (scope_mode, write_mode, legacy_access_mode) =
        if let (Some(scope_mode), Some(write_mode)) = (parsed_scope, parsed_write) {
            (
                scope_mode,
                write_mode,
                Some(legacy_access_mode_from_policy(scope_mode, write_mode)),
            )
        } else {
            let default_scope = default_scope_for_session_source(session_source, portal_restricted);
            match legacy_document_access_mode
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(|value| value.to_ascii_lowercase())
                .as_deref()
            {
                Some("attached_only" | "attached-only" | "attachedonly") => (
                    DocumentScopeMode::AttachedOnly,
                    DocumentWriteMode::DraftOnly,
                    Some("attached_only".to_string()),
                ),
                Some("read_only" | "read-only" | "readonly") => (
                    default_scope,
                    DocumentWriteMode::ReadOnly,
                    Some("read_only".to_string()),
                ),
                Some("co_edit_draft" | "co-edit-draft" | "coeditdraft") => (
                    default_scope,
                    DocumentWriteMode::DraftOnly,
                    Some("co_edit_draft".to_string()),
                ),
                Some("controlled_write" | "controlled-write" | "controlledwrite") => (
                    default_scope,
                    DocumentWriteMode::ControlledWrite,
                    Some("controlled_write".to_string()),
                ),
                Some("full") => (
                    DocumentScopeMode::Full,
                    DocumentWriteMode::FullWrite,
                    Some("full".to_string()),
                ),
                _ => (
                    default_scope,
                    if matches!(default_scope, DocumentScopeMode::Full) {
                        DocumentWriteMode::FullWrite
                    } else {
                        DocumentWriteMode::ReadOnly
                    },
                    None,
                ),
            }
        };

    ResolvedDocumentPolicy {
        document_access_mode: legacy_access_mode,
        document_scope_mode: Some(scope_mode.as_str().to_string()),
        document_write_mode: Some(write_mode.as_str().to_string()),
    }
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

pub struct AgentRuntimePolicyResolver;

fn resolved_builtin_capabilities(agent: &TeamAgent) -> Vec<ConfiguredBuiltinCapability> {
    let mut capabilities = Vec::new();

    for config in &agent.enabled_extensions {
        if !capabilities
            .iter()
            .any(|item: &ConfiguredBuiltinCapability| item.extension == config.extension)
        {
            capabilities.push(ConfiguredBuiltinCapability {
                extension: config.extension,
                enabled: config.enabled,
                registry: builtin_registry_entry(config.extension),
            });
        }
    }

    for extension in BuiltinExtension::all() {
        let registry = builtin_registry_entry(extension);
        if registry.default_enabled
            && !capabilities
                .iter()
                .any(|item: &ConfiguredBuiltinCapability| item.extension == extension)
        {
            capabilities.push(ConfiguredBuiltinCapability {
                extension,
                enabled: true,
                registry,
            });
        }
    }

    capabilities
}

impl AgentRuntimePolicyResolver {
    pub fn resolve(
        agent: &TeamAgent,
        session: Option<&AgentSessionDoc>,
        portal_effective: Option<&PortalEffectivePublicConfig>,
    ) -> RuntimeCapabilitySnapshot {
        let session_source = session
            .map(|item| item.session_source.clone())
            .unwrap_or_else(|| "chat".to_string());
        let portal_restricted = session.map(|item| item.portal_restricted).unwrap_or(false);
        let document_policy = resolve_document_policy(
            session
                .and_then(|item| item.document_access_mode.as_deref())
                .or_else(|| {
                    portal_effective
                        .map(|item| document_access_mode_key(item.effective_document_access_mode))
                }),
            session.and_then(|item| item.document_scope_mode.as_deref()),
            session.and_then(|item| item.document_write_mode.as_deref()),
            Some(session_source.as_str()),
            portal_restricted,
        );

        let overlay_allowed_extensions = session
            .and_then(|item| item.allowed_extensions.clone())
            .or_else(|| {
                portal_effective.map(|item| {
                    if item.extensions_inherited {
                        Vec::new()
                    } else {
                        item.effective_allowed_extensions.clone()
                    }
                })
            });
        let overlay_allowed_skill_ids = session
            .and_then(|item| item.allowed_skill_ids.clone())
            .or_else(|| {
                portal_effective.map(|item| {
                    if item.skills_inherited {
                        Vec::new()
                    } else {
                        item.effective_allowed_skill_ids.clone()
                    }
                })
            });

        let builtin_capabilities = resolved_builtin_capabilities(agent);

        let custom_extensions = agent
            .custom_extensions
            .iter()
            .filter(|extension| {
                extension.enabled
                    && extension.source.as_deref() != Some("team")
                    && extension.source_extension_id.is_none()
            })
            .cloned()
            .collect::<Vec<_>>();
        let legacy_team_extensions = agent
            .custom_extensions
            .iter()
            .filter(|extension| {
                extension.enabled
                    && (extension.source.as_deref() == Some("team")
                        || extension.source_extension_id.is_some())
            })
            .cloned()
            .collect::<Vec<_>>();
        let attached_team_extensions = merge_attached_team_extensions(agent);

        let mut base_extension_names = HashSet::new();
        for item in builtin_capabilities
            .iter()
            .filter(|item| item.enabled && item.registry.editable)
        {
            for runtime_name in &item.registry.runtime_names {
                base_extension_names.insert(runtime_name.clone());
            }
        }
        for extension in &custom_extensions {
            base_extension_names.insert(normalize_runtime_name(&extension.name));
        }
        for extension in &legacy_team_extensions {
            base_extension_names.insert(normalize_runtime_name(&extension.name));
        }
        for reference in &attached_team_extensions {
            if let Some(runtime_name) = reference.runtime_name.as_deref() {
                base_extension_names.insert(normalize_runtime_name(runtime_name));
            }
        }

        let session_injected_capabilities =
            session_injected_capabilities(session_source.as_str(), portal_restricted);

        let mut effective_allowed_extension_names = match overlay_allowed_extensions {
            Some(values) if !values.is_empty() => {
                let requested = normalize_unique_runtime_names(values);
                let overlay_requestable =
                    explicitly_requestable_platform_runtime_names(session_source.as_str());
                requested
                    .into_iter()
                    .filter(|runtime_name| {
                        base_extension_names.contains(runtime_name)
                            || overlay_requestable.contains(runtime_name)
                    })
                    .collect::<Vec<_>>()
            }
            _ => {
                let mut values = base_extension_names.into_iter().collect::<Vec<_>>();
                values.sort();
                values
            }
        };
        for capability in &session_injected_capabilities {
            for runtime_name in &capability.runtime_names {
                let normalized = normalize_runtime_name(runtime_name);
                if !effective_allowed_extension_names.contains(&normalized) {
                    effective_allowed_extension_names.push(normalized);
                }
            }
        }
        effective_allowed_extension_names = post_filter_allowed_extensions_for_session_source(
            session_source.as_str(),
            effective_allowed_extension_names,
        );
        effective_allowed_extension_names.sort();

        let assigned_skills = agent
            .assigned_skills
            .iter()
            .filter(|skill| skill.enabled)
            .cloned()
            .collect::<Vec<_>>();
        let assigned_skill_ids = normalize_skill_ids(
            assigned_skills
                .iter()
                .map(|skill| skill.skill_id.clone())
                .collect::<Vec<_>>(),
        );
        let restricted_scope = portal_restricted
            || matches!(
                session_source.as_str(),
                "portal" | "portal_coding" | "portal_manager" | "system" | "document_analysis"
            );
        let base_skill_scope = match agent.skill_binding_mode {
            SkillBindingMode::AssignedOnly => Some(assigned_skill_ids.clone()),
            SkillBindingMode::Hybrid => {
                if restricted_scope {
                    Some(assigned_skill_ids.clone())
                } else {
                    None
                }
            }
            SkillBindingMode::OnDemandOnly => {
                if restricted_scope {
                    Some(Vec::new())
                } else {
                    None
                }
            }
        };
        let effective_allowed_skill_ids = match (base_skill_scope, overlay_allowed_skill_ids) {
            (Some(base), Some(values)) if !values.is_empty() => {
                let requested = normalize_skill_ids(values);
                Some(
                    requested
                        .into_iter()
                        .filter(|skill_id| base.contains(skill_id))
                        .collect::<Vec<_>>(),
                )
            }
            (Some(base), _) => Some(base),
            (None, Some(values)) if !values.is_empty() => Some(normalize_skill_ids(values)),
            (None, _) => None,
        };

        let portal_delegation_policy_override =
            portal_effective.and_then(|item| item.delegation_policy_override.clone());
        let session_delegation_policy_override =
            session.and_then(|item| item.delegation_policy_override.clone());
        let source_delegation_policy_override =
            source_delegation_override_for_session_source(session_source.as_str());
        let delegation_policy = agent
            .delegation_policy
            .apply_override(portal_delegation_policy_override.as_ref())
            .apply_override(session_delegation_policy_override.as_ref())
            .apply_override(source_delegation_policy_override.as_ref());

        RuntimeCapabilitySnapshot {
            session_source,
            portal_restricted,
            document_access_mode: document_policy.document_access_mode,
            document_scope_mode: document_policy.document_scope_mode,
            document_write_mode: document_policy.document_write_mode,
            extensions: RuntimeExtensionResolution {
                builtin_capabilities,
                custom_extensions,
                attached_team_extensions,
                legacy_team_extensions,
                effective_allowed_extension_names,
                session_injected_capabilities,
            },
            skills: RuntimeSkillResolution {
                assigned_skills,
                skill_binding_mode: agent.skill_binding_mode,
                effective_allowed_skill_ids,
            },
            delegation_policy,
            session_delegation_policy_override,
            portal_delegation_policy_override,
        }
    }
}

pub fn normalize_runtime_name(value: &str) -> String {
    let normalized = value.trim().to_ascii_lowercase().replace('-', "_");
    match normalized.as_str() {
        "todo" => "tasks".to_string(),
        other => other.to_string(),
    }
}

fn normalize_unique_runtime_names(values: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    values
        .into_iter()
        .map(|value| normalize_runtime_name(&value))
        .filter(|value| !value.is_empty())
        .filter(|value| seen.insert(value.clone()))
        .collect()
}

fn normalize_skill_ids(values: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    values
        .into_iter()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .filter(|value| seen.insert(value.clone()))
        .collect()
}

pub fn builtin_registry_entry(extension: BuiltinExtension) -> CapabilityRegistryEntry {
    let (
        display_name,
        kind,
        display_group,
        runtime_delivery,
        runtime_names,
        editable,
        default_enabled,
        hidden_reason,
    ) = match extension {
        BuiltinExtension::Skills => (
            "Skills",
            CapabilityKind::BuiltinPlatform,
            CapabilityDisplayGroup::Skills,
            RuntimeDelivery::InProcess,
            vec!["skills".to_string(), "team_skills".to_string()],
            true,
            true,
            None,
        ),
        BuiltinExtension::SkillRegistry => (
            "Skill Registry",
            CapabilityKind::BuiltinPlatform,
            CapabilityDisplayGroup::PlatformExtensions,
            RuntimeDelivery::InProcess,
            vec!["skill_registry".to_string()],
            true,
            false,
            None,
        ),
        BuiltinExtension::Tasks => (
            "Tasks",
            CapabilityKind::BuiltinPlatform,
            CapabilityDisplayGroup::PlatformExtensions,
            RuntimeDelivery::InProcess,
            vec!["tasks".to_string()],
            true,
            true,
            None,
        ),
        BuiltinExtension::DocumentTools => (
            "Document Tools",
            CapabilityKind::BuiltinPlatform,
            CapabilityDisplayGroup::PlatformExtensions,
            RuntimeDelivery::InProcess,
            vec!["document_tools".to_string()],
            true,
            true,
            None,
        ),
        BuiltinExtension::Developer => (
            "Developer",
            CapabilityKind::BuiltinMcp,
            CapabilityDisplayGroup::McpServices,
            RuntimeDelivery::InProcess,
            vec!["developer".to_string()],
            true,
            true,
            None,
        ),
        BuiltinExtension::Memory => (
            "Memory",
            CapabilityKind::BuiltinMcp,
            CapabilityDisplayGroup::McpServices,
            RuntimeDelivery::SubprocessMcp,
            vec!["memory".to_string()],
            true,
            false,
            None,
        ),
        BuiltinExtension::ComputerController => (
            "Computer Controller",
            CapabilityKind::BuiltinMcp,
            CapabilityDisplayGroup::McpServices,
            RuntimeDelivery::SubprocessMcp,
            vec![
                "computer_controller".to_string(),
                "computercontroller".to_string(),
            ],
            true,
            false,
            None,
        ),
        BuiltinExtension::AutoVisualiser => (
            "Auto Visualiser",
            CapabilityKind::BuiltinMcp,
            CapabilityDisplayGroup::McpServices,
            RuntimeDelivery::SubprocessMcp,
            vec!["auto_visualiser".to_string(), "autovisualiser".to_string()],
            true,
            false,
            None,
        ),
        BuiltinExtension::Tutorial => (
            "Tutorial",
            CapabilityKind::BuiltinMcp,
            CapabilityDisplayGroup::McpServices,
            RuntimeDelivery::SubprocessMcp,
            vec!["tutorial".to_string()],
            true,
            false,
            None,
        ),
        BuiltinExtension::ExtensionManager => (
            "Extension Manager",
            CapabilityKind::SystemReserved,
            CapabilityDisplayGroup::SystemInjected,
            RuntimeDelivery::SessionInjected,
            vec!["extension_manager".to_string()],
            false,
            true,
            Some("system_assist".to_string()),
        ),
        BuiltinExtension::Team => (
            "Team",
            CapabilityKind::SystemReserved,
            CapabilityDisplayGroup::SystemInjected,
            RuntimeDelivery::SessionInjected,
            vec!["team".to_string()],
            false,
            true,
            Some("legacy_hidden".to_string()),
        ),
        BuiltinExtension::ChatRecall => (
            "Chat Recall",
            CapabilityKind::SystemReserved,
            CapabilityDisplayGroup::SystemInjected,
            RuntimeDelivery::SessionInjected,
            vec!["chat_recall".to_string()],
            false,
            false,
            Some("system_assist".to_string()),
        ),
    };

    CapabilityRegistryEntry {
        config_key: extension.name().to_string(),
        display_name: display_name.to_string(),
        kind,
        display_group,
        runtime_delivery,
        runtime_names,
        editable,
        default_enabled,
        hidden_reason,
    }
}

pub fn session_injected_capabilities(
    session_source: &str,
    portal_restricted: bool,
) -> Vec<CapabilityRegistryEntry> {
    let mut entries = Vec::new();
    if !portal_restricted && !session_source.eq_ignore_ascii_case("scheduled_task") {
        entries.push(CapabilityRegistryEntry {
            config_key: "team_mcp".to_string(),
            display_name: "Team MCP".to_string(),
            kind: CapabilityKind::SessionInjected,
            display_group: CapabilityDisplayGroup::SystemInjected,
            runtime_delivery: RuntimeDelivery::SessionInjected,
            runtime_names: vec!["team_mcp".to_string()],
            editable: false,
            default_enabled: true,
            hidden_reason: Some("management".to_string()),
        });
    }
    match session_source {
        "chat" | "automation_runtime" => {
            entries.push(CapabilityRegistryEntry {
                config_key: "chat_memory".to_string(),
                display_name: "Chat Memory".to_string(),
                kind: CapabilityKind::SessionInjected,
                display_group: CapabilityDisplayGroup::SystemInjected,
                runtime_delivery: RuntimeDelivery::SessionInjected,
                runtime_names: vec!["chat_memory".to_string()],
                editable: false,
                default_enabled: true,
                hidden_reason: Some(if session_source == "automation_runtime" {
                    "automation_runtime_session".to_string()
                } else {
                    "chat_session".to_string()
                }),
            });
            if session_source == "chat" {
                entries.push(CapabilityRegistryEntry {
                    config_key: "chat_delivery".to_string(),
                    display_name: "Chat Delivery".to_string(),
                    kind: CapabilityKind::SessionInjected,
                    display_group: CapabilityDisplayGroup::SystemInjected,
                    runtime_delivery: RuntimeDelivery::SessionInjected,
                    runtime_names: vec!["chat_delivery".to_string()],
                    editable: false,
                    default_enabled: true,
                    hidden_reason: Some("direct_chat_delivery".to_string()),
                });
            }
        }
        "channel_conversation" => {}
        "portal" | "portal_manager" | "portal_coding" => {
            entries.push(CapabilityRegistryEntry {
                config_key: "avatar_governance".to_string(),
                display_name: "Avatar Governance".to_string(),
                kind: CapabilityKind::SessionInjected,
                display_group: CapabilityDisplayGroup::SystemInjected,
                runtime_delivery: RuntimeDelivery::SessionInjected,
                runtime_names: vec!["avatar_governance".to_string()],
                editable: false,
                default_enabled: true,
                hidden_reason: Some("portal_session".to_string()),
            });
            if matches!(session_source, "portal_manager" | "portal_coding") {
                entries.push(CapabilityRegistryEntry {
                    config_key: "portal_tools".to_string(),
                    display_name: "Portal Tools".to_string(),
                    kind: CapabilityKind::SessionInjected,
                    display_group: CapabilityDisplayGroup::SystemInjected,
                    runtime_delivery: RuntimeDelivery::SessionInjected,
                    runtime_names: vec!["portal_tools".to_string()],
                    editable: false,
                    default_enabled: true,
                    hidden_reason: Some("portal_management".to_string()),
                });
            }
        }
        _ => {}
    }
    entries
}

fn document_access_mode_key(mode: PortalDocumentAccessMode) -> &'static str {
    match mode {
        PortalDocumentAccessMode::ReadOnly => "read_only",
        PortalDocumentAccessMode::CoEditDraft => "co_edit_draft",
        PortalDocumentAccessMode::ControlledWrite => "controlled_write",
    }
}

fn merge_attached_team_extensions(agent: &TeamAgent) -> Vec<AttachedTeamExtensionRef> {
    let mut seen = HashSet::new();
    let mut refs = Vec::new();
    for reference in &agent.attached_team_extensions {
        if reference.extension_id.trim().is_empty() {
            continue;
        }
        if seen.insert(reference.extension_id.clone()) {
            refs.push(reference.clone());
        }
    }
    for extension in agent.custom_extensions.iter().filter(|extension| {
        extension.enabled
            && (extension.source.as_deref() == Some("team")
                || extension.source_extension_id.is_some())
    }) {
        let Some(extension_id) = extension.source_extension_id.clone() else {
            continue;
        };
        if seen.insert(extension_id.clone()) {
            refs.push(AttachedTeamExtensionRef {
                extension_id,
                enabled: extension.enabled,
                runtime_name: Some(extension.name.clone()),
                display_name: Some(extension.name.clone()),
                transport: Some(extension.ext_type.clone()),
            });
        }
    }
    refs
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::session_mongo::AgentSessionDoc;
    use agime_team::models::mongo::PortalPublicExposure;
    use agime_team::models::{
        AgentExtensionConfig, AgentSkillConfig, BuiltinExtension, CustomExtensionConfig,
    };

    fn sample_agent() -> TeamAgent {
        let mut agent = TeamAgent::new("team-1".to_string(), "agent-1".to_string());
        agent.enabled_extensions = vec![
            AgentExtensionConfig {
                extension: BuiltinExtension::Developer,
                enabled: true,
            },
            AgentExtensionConfig {
                extension: BuiltinExtension::Skills,
                enabled: true,
            },
        ];
        agent.assigned_skills = vec![AgentSkillConfig {
            skill_id: "skill-alpha".to_string(),
            name: "Skill Alpha".to_string(),
            description: None,
            enabled: true,
            version: "1.0.0".to_string(),
        }];
        agent
    }

    fn sample_session(source: &str) -> AgentSessionDoc {
        let now = bson::DateTime::now();
        AgentSessionDoc {
            id: None,
            session_id: format!("session-{source}"),
            team_id: "team-1".to_string(),
            agent_id: "agent-1".to_string(),
            user_id: "user-1".to_string(),
            name: None,
            status: "active".to_string(),
            messages_json: "[]".to_string(),
            message_count: 0,
            total_tokens: None,
            input_tokens: None,
            output_tokens: None,
            context_runtime_state: None,
            disabled_extensions: Vec::new(),
            enabled_extensions: Vec::new(),
            created_at: now,
            updated_at: now,
            title: None,
            pinned: false,
            last_message_preview: None,
            last_message_at: None,
            is_processing: false,
            last_execution_status: None,
            last_execution_error: None,
            last_execution_finished_at: None,
            last_runtime_session_id: None,
            last_delegation_runtime: None,
            attached_document_ids: Vec::new(),
            workspace_path: None,
            workspace_id: None,
            workspace_kind: None,
            workspace_manifest_path: None,
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
            document_scope_mode: None,
            document_write_mode: None,
            delegation_policy_override: None,
            portal_id: None,
            portal_slug: None,
            visitor_id: None,
            session_source: source.to_string(),
            source_channel_id: None,
            source_channel_name: None,
            source_thread_root_id: None,
            thread_branch: None,
            thread_repo_ref: None,
            hidden_from_chat_list: false,
            pending_message_workspace_files: Vec::new(),
        }
    }

    fn sample_portal_effective() -> PortalEffectivePublicConfig {
        PortalEffectivePublicConfig {
            exposure: PortalPublicExposure::PreviewOnly,
            public_access_enabled: false,
            chat_enabled: false,
            show_chat_widget: false,
            show_bound_documents: false,
            effective_document_access_mode: PortalDocumentAccessMode::ReadOnly,
            effective_allowed_extensions: vec!["developer".to_string()],
            effective_allowed_skill_ids: vec!["skill-alpha".to_string()],
            effective_allowed_skill_names: vec!["Skill Alpha".to_string()],
            extensions_inherited: false,
            skills_inherited: false,
            delegation_policy_override: None,
        }
    }

    #[test]
    fn hybrid_skill_mode_is_unrestricted_for_normal_chat_sessions() {
        let agent = sample_agent();
        let snapshot = AgentRuntimePolicyResolver::resolve(&agent, None, None);
        assert_eq!(snapshot.skills.skill_binding_mode, SkillBindingMode::Hybrid);
        assert!(snapshot.skills.effective_allowed_skill_ids.is_none());
    }

    #[test]
    fn resolver_backfills_missing_default_enabled_builtin_extensions_for_legacy_agents() {
        let agent = sample_agent();
        let snapshot = AgentRuntimePolicyResolver::resolve(&agent, None, None);

        assert!(snapshot
            .extensions
            .effective_allowed_extension_names
            .contains(&"document_tools".to_string()));
        assert!(snapshot
            .extensions
            .effective_allowed_extension_names
            .contains(&"tasks".to_string()));
    }

    #[test]
    fn hybrid_skill_mode_is_narrowed_to_assigned_skills_for_system_sessions() {
        let agent = sample_agent();
        let session = sample_session("system");
        let snapshot = AgentRuntimePolicyResolver::resolve(&agent, Some(&session), None);
        assert_eq!(
            snapshot.skills.effective_allowed_skill_ids,
            Some(vec!["skill-alpha".to_string()])
        );
    }

    #[test]
    fn session_injected_chat_memory_survives_extension_allowlist() {
        let agent = sample_agent();
        let mut session = sample_session("chat");
        session.allowed_extensions = Some(vec!["document_tools".to_string()]);

        let snapshot = AgentRuntimePolicyResolver::resolve(&agent, Some(&session), None);
        assert!(snapshot
            .extensions
            .effective_allowed_extension_names
            .contains(&"document_tools".to_string()));
        assert!(snapshot
            .extensions
            .effective_allowed_extension_names
            .contains(&"chat_memory".to_string()));
        assert!(snapshot
            .extensions
            .session_injected_capabilities
            .iter()
            .any(|item| item.config_key == "chat_memory"));
    }

    #[test]
    fn automation_runtime_keeps_chat_memory_without_clamping_delegation() {
        let mut agent = sample_agent();
        agent.delegation_policy = DelegationPolicy::default();
        let session = sample_session("automation_runtime");

        let snapshot = AgentRuntimePolicyResolver::resolve(&agent, Some(&session), None);

        assert!(snapshot.delegation_policy.allow_subagent);
        assert!(snapshot.delegation_policy.allow_swarm);
        assert!(snapshot
            .extensions
            .session_injected_capabilities
            .iter()
            .any(|item| item.config_key == "chat_memory"));
    }

    #[test]
    fn explicit_api_tools_allowlist_survives_capability_filtering() {
        let agent = sample_agent();
        let mut session = sample_session("chat");
        session.allowed_extensions = Some(vec!["api_tools".to_string()]);

        let snapshot = AgentRuntimePolicyResolver::resolve(&agent, Some(&session), None);
        assert!(snapshot
            .extensions
            .effective_allowed_extension_names
            .contains(&"api_tools".to_string()));
    }

    #[test]
    fn internal_chat_defaults_to_full_document_policy() {
        let policy = resolve_document_policy(None, None, None, Some("chat"), false);
        assert_eq!(policy.document_access_mode.as_deref(), None);
        assert_eq!(policy.document_scope_mode.as_deref(), Some("full"));
        assert_eq!(policy.document_write_mode.as_deref(), Some("full_write"));
    }

    #[test]
    fn internal_channel_defaults_to_full_document_policy() {
        let policy = resolve_document_policy(None, None, None, Some("channel_conversation"), false);
        assert_eq!(policy.document_access_mode.as_deref(), None);
        assert_eq!(policy.document_scope_mode.as_deref(), Some("full"));
        assert_eq!(policy.document_write_mode.as_deref(), Some("full_write"));
    }

    #[test]
    fn external_portal_sessions_stay_portal_bound_and_read_only() {
        let policy = resolve_document_policy(None, None, None, Some("portal"), true);
        assert_eq!(policy.document_access_mode.as_deref(), None);
        assert_eq!(policy.document_scope_mode.as_deref(), Some("portal_bound"));
        assert_eq!(policy.document_write_mode.as_deref(), Some("read_only"));
    }

    #[test]
    fn delegation_overrides_only_narrow_the_base_policy() {
        let mut agent = sample_agent();
        agent.delegation_policy = DelegationPolicy {
            allow_plan: true,
            allow_subagent: true,
            allow_swarm: true,
            allow_worker_messaging: true,
            allow_auto_swarm: true,
            allow_validation_worker: true,
            approval_mode: agime_team::models::ApprovalMode::LeaderOwned,
            max_subagent_depth: 5,
            parallelism_budget: Some(8),
            swarm_budget: Some(6),
            require_final_report: true,
        };

        let mut session = sample_session("portal");
        session.delegation_policy_override = Some(DelegationPolicyOverride {
            allow_plan: None,
            allow_subagent: Some(false),
            allow_swarm: None,
            allow_worker_messaging: None,
            allow_auto_swarm: None,
            allow_validation_worker: None,
            approval_mode: None,
            max_subagent_depth: None,
            parallelism_budget: Some(3),
            swarm_budget: None,
            require_final_report: Some(true),
        });

        let mut portal = sample_portal_effective();
        portal.delegation_policy_override = Some(DelegationPolicyOverride {
            allow_plan: Some(true),
            allow_subagent: None,
            allow_swarm: Some(false),
            allow_worker_messaging: Some(false),
            allow_auto_swarm: Some(false),
            allow_validation_worker: None,
            approval_mode: None,
            max_subagent_depth: Some(2),
            parallelism_budget: None,
            swarm_budget: Some(2),
            require_final_report: None,
        });

        let snapshot = AgentRuntimePolicyResolver::resolve(&agent, Some(&session), Some(&portal));
        assert!(snapshot.delegation_policy.allow_plan);
        assert!(!snapshot.delegation_policy.allow_subagent);
        assert!(!snapshot.delegation_policy.allow_swarm);
        assert!(!snapshot.delegation_policy.allow_auto_swarm);
        assert_eq!(snapshot.delegation_policy.max_subagent_depth, 2);
        assert_eq!(snapshot.delegation_policy.parallelism_budget, Some(3));
        assert_eq!(snapshot.delegation_policy.swarm_budget, Some(2));
        assert!(snapshot.delegation_policy.require_final_report);
        assert_eq!(
            snapshot.delegation_policy.approval_mode,
            agime_team::models::ApprovalMode::LeaderOwned
        );
    }

    #[test]
    fn portal_manager_source_clamps_delegation_even_when_agent_allows_it() {
        let mut agent = sample_agent();
        agent.delegation_policy = DelegationPolicy::default();
        let session = sample_session("portal_manager");

        let snapshot = AgentRuntimePolicyResolver::resolve(&agent, Some(&session), None);

        assert!(!snapshot.delegation_policy.allow_plan);
        assert!(!snapshot.delegation_policy.allow_subagent);
        assert!(!snapshot.delegation_policy.allow_swarm);
        assert!(!snapshot.delegation_policy.allow_auto_swarm);
        assert!(!snapshot.delegation_policy.allow_worker_messaging);
        assert!(!snapshot.delegation_policy.allow_validation_worker);
    }

    #[test]
    fn portal_coding_source_keeps_existing_delegation_policy() {
        let mut agent = sample_agent();
        agent.delegation_policy = DelegationPolicy::default();
        let session = sample_session("portal_coding");

        let snapshot = AgentRuntimePolicyResolver::resolve(&agent, Some(&session), None);

        assert!(snapshot.delegation_policy.allow_plan);
        assert!(snapshot.delegation_policy.allow_subagent);
        assert!(snapshot.delegation_policy.allow_swarm);
        assert!(snapshot.delegation_policy.allow_auto_swarm);
    }

    #[test]
    fn source_override_helper_targets_non_delegating_surfaces() {
        let override_policy = source_delegation_override_for_session_source("portal_manager")
            .expect("portal manager override");
        assert_eq!(override_policy.allow_plan, Some(false));
        assert_eq!(override_policy.allow_subagent, Some(false));
        assert_eq!(override_policy.allow_swarm, Some(false));
        assert_eq!(override_policy.allow_auto_swarm, Some(false));
        assert_eq!(override_policy.allow_worker_messaging, Some(false));
        assert_eq!(override_policy.allow_validation_worker, Some(false));
        let scheduled_override = source_delegation_override_for_session_source("scheduled_task")
            .expect("scheduled task override");
        assert_eq!(scheduled_override.allow_plan, Some(false));
        assert_eq!(scheduled_override.allow_subagent, Some(false));
        assert_eq!(scheduled_override.allow_swarm, Some(false));
        assert_eq!(scheduled_override.allow_auto_swarm, Some(false));
        assert_eq!(scheduled_override.allow_worker_messaging, Some(false));
        assert_eq!(scheduled_override.allow_validation_worker, Some(false));
        let builder_override = source_delegation_override_for_session_source("automation_builder")
            .expect("automation builder override");
        assert_eq!(builder_override.allow_plan, Some(false));
        assert_eq!(builder_override.allow_subagent, Some(false));
        assert_eq!(builder_override.allow_swarm, Some(false));
        assert_eq!(builder_override.allow_auto_swarm, Some(false));
        assert_eq!(builder_override.allow_worker_messaging, Some(false));
        assert_eq!(builder_override.allow_validation_worker, Some(false));
        assert!(source_delegation_override_for_session_source("portal_coding").is_none());
        assert!(source_delegation_override_for_session_source("chat").is_none());
    }

    #[test]
    fn scheduled_task_source_clamps_delegation_even_when_agent_allows_it() {
        let mut agent = sample_agent();
        agent.delegation_policy = DelegationPolicy::default();
        let session = sample_session("scheduled_task");

        let snapshot = AgentRuntimePolicyResolver::resolve(&agent, Some(&session), None);

        assert!(!snapshot.delegation_policy.allow_plan);
        assert!(!snapshot.delegation_policy.allow_subagent);
        assert!(!snapshot.delegation_policy.allow_swarm);
        assert!(!snapshot.delegation_policy.allow_auto_swarm);
        assert!(!snapshot.delegation_policy.allow_worker_messaging);
        assert!(!snapshot.delegation_policy.allow_validation_worker);
    }

    #[test]
    fn automation_builder_source_clamps_delegation_even_when_agent_allows_it() {
        let mut agent = sample_agent();
        agent.delegation_policy = DelegationPolicy::default();
        let session = sample_session("automation_builder");

        let snapshot = AgentRuntimePolicyResolver::resolve(&agent, Some(&session), None);

        assert!(!snapshot.delegation_policy.allow_plan);
        assert!(!snapshot.delegation_policy.allow_subagent);
        assert!(!snapshot.delegation_policy.allow_swarm);
        assert!(!snapshot.delegation_policy.allow_auto_swarm);
        assert!(!snapshot.delegation_policy.allow_worker_messaging);
        assert!(!snapshot.delegation_policy.allow_validation_worker);
    }

    #[test]
    fn attached_team_extensions_merge_with_legacy_team_configs_and_obey_overlay_allowlist() {
        let mut agent = sample_agent();
        agent.custom_extensions = vec![
            CustomExtensionConfig {
                name: "playwright".to_string(),
                ext_type: "stdio".to_string(),
                uri_or_cmd: "playwright".to_string(),
                args: Vec::new(),
                envs: std::collections::HashMap::new(),
                enabled: true,
                source: Some("team".to_string()),
                source_extension_id: Some("ext-1".to_string()),
            },
            CustomExtensionConfig {
                name: "custom_data".to_string(),
                ext_type: "stdio".to_string(),
                uri_or_cmd: "custom-data".to_string(),
                args: Vec::new(),
                envs: std::collections::HashMap::new(),
                enabled: true,
                source: None,
                source_extension_id: None,
            },
        ];
        agent.attached_team_extensions = vec![AttachedTeamExtensionRef {
            extension_id: "ext-2".to_string(),
            enabled: true,
            runtime_name: Some("browser_use".to_string()),
            display_name: Some("Browser Use".to_string()),
            transport: Some("stdio".to_string()),
        }];

        let mut session = sample_session("chat");
        session.allowed_extensions = Some(vec![
            "custom_data".to_string(),
            "browser_use".to_string(),
            "missing".to_string(),
        ]);

        let snapshot = AgentRuntimePolicyResolver::resolve(&agent, Some(&session), None);
        assert_eq!(
            snapshot.extensions.effective_allowed_extension_names,
            vec!["browser_use".to_string(), "custom_data".to_string(),]
        );
        assert_eq!(snapshot.runtime_custom_extensions().len(), 1);
        assert_eq!(snapshot.runtime_custom_extensions()[0].name, "custom_data");
        assert_eq!(snapshot.runtime_team_extension_refs().len(), 1);
        assert_eq!(
            snapshot.runtime_team_extension_refs()[0].extension_id,
            "ext-2".to_string()
        );
    }
}
