//! Portal service for MongoDB — CRUD, publish/unpublish, interactions

use super::TeamService;
use crate::db::{collections, MongoDb};
use crate::models::mongo::{
    CreatePortalRequest, DocumentBindingPortalRef, DocumentBindingUsageSummary, PaginatedResponse,
    Portal, PortalDocumentAccessMode, PortalDomain, PortalEffectivePublicConfig, PortalInteraction,
    PortalInteractionResponse, PortalPublicExposure, PortalStatus, PortalSummary,
    UpdatePortalRequest,
};
use crate::models::{
    AgentExtensionConfig, AgentSkillConfig, BuiltinExtension, CustomExtensionConfig,
};
use anyhow::{anyhow, Result};
use chrono::Utc;
use futures::TryStreamExt;
use mongodb::bson::{doc, oid::ObjectId, Bson};
use mongodb::options::{FindOptions, IndexOptions};
use mongodb::IndexModel;
use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Deserialize)]
struct TeamAgentPolicyDoc {
    pub agent_id: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub agent_domain: Option<String>,
    #[serde(default)]
    pub agent_role: Option<String>,
    #[serde(default)]
    pub owner_manager_agent_id: Option<String>,
    #[serde(default)]
    pub enabled_extensions: Vec<AgentExtensionConfig>,
    #[serde(default)]
    pub custom_extensions: Vec<CustomExtensionConfig>,
    #[serde(default)]
    pub assigned_skills: Vec<AgentSkillConfig>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EcosystemServiceAgentBinding {
    GeneralTemplate,
    AvatarService,
    AvatarManager,
    AvatarOther,
    EcosystemService,
    EcosystemOther,
    Unsupported,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AvatarBindingShadowIssue {
    ManagerRoleMismatch,
    ServiceRoleMismatch,
    OwnerManagerMismatch,
}

/// Simple kebab-case slug generator (no external crate needed)
fn slugify(input: &str) -> String {
    input
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

/// Validate slug format: lowercase alphanumeric and hyphens, 2-80 chars
fn validate_slug(slug: &str) -> Result<()> {
    if slug.len() < 2 || slug.len() > 80 {
        return Err(anyhow!("Slug must be between 2 and 80 characters"));
    }
    if !slug
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    {
        return Err(anyhow!(
            "Slug may only contain lowercase letters, digits, and hyphens"
        ));
    }
    if slug.starts_with('-') || slug.ends_with('-') {
        return Err(anyhow!("Slug must not start or end with a hyphen"));
    }
    Ok(())
}

/// Max interaction data size: 64KB
const MAX_INTERACTION_DATA_SIZE: usize = 64 * 1024;

pub struct PortalService {
    db: MongoDb,
}

impl PortalService {
    fn active_portal_partial_index_filter() -> bson::Document {
        // Mongo partial indexes do not accept `$ne`, so match active rows via
        // `false` plus `null` (which also covers legacy documents with no field).
        doc! {
            "$or": [
                { "is_deleted": false },
                { "is_deleted": Bson::Null }
            ]
        }
    }

    pub fn new(db: MongoDb) -> Self {
        Self { db }
    }

    /// Create unique index on slug to prevent race conditions (C-2).
    pub async fn ensure_indexes(&self) -> Result<()> {
        let coll = self.db.collection::<Portal>(collections::PORTALS);
        let idx = IndexModel::builder()
            .keys(doc! { "slug": 1 })
            .options(
                IndexOptions::builder()
                    .unique(true)
                    .partial_filter_expression(Self::active_portal_partial_index_filter())
                    .build(),
            )
            .build();
        coll.create_index(idx, None).await?;
        Ok(())
    }

    /// Backfill explicit `domain` field for legacy portals.
    /// Legacy records are detected by missing/null domain and inferred from tags/settings.
    pub async fn backfill_domain_field(&self) -> Result<u64> {
        let coll = self.db.collection::<Portal>(collections::PORTALS);
        let filter = doc! {
            "is_deleted": { "$ne": true },
            "$or": [
                { "domain": { "$exists": false } },
                { "domain": Bson::Null }
            ]
        };
        let mut cursor = coll.find(filter, None).await?;
        let mut updated = 0_u64;

        while let Some(portal) = cursor.try_next().await? {
            let Some(portal_oid) = portal.id else {
                continue;
            };
            let domain = Self::resolve_portal_domain(&portal);
            let result = coll
                .update_one(
                    doc! { "_id": portal_oid, "is_deleted": { "$ne": true } },
                    doc! {
                        "$set": {
                            "domain": bson::to_bson(&domain)?,
                            "updated_at": bson::DateTime::from_chrono(Utc::now())
                        }
                    },
                    None,
                )
                .await?;
            updated += result.modified_count;
        }
        Ok(updated)
    }

    fn normalize_agent_id(value: Option<&str>) -> Option<String> {
        value
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(ToString::to_string)
    }

    fn resolve_coding_agent_id(portal: &Portal) -> Option<String> {
        let explicit = Self::normalize_agent_id(portal.coding_agent_id.as_deref())
            .or_else(|| Self::normalize_agent_id(portal.agent_id.as_deref()));
        if explicit.is_some() {
            return explicit;
        }
        if Self::resolve_portal_domain(portal) == PortalDomain::Avatar {
            return None;
        }
        Self::normalize_agent_id(portal.service_agent_id.as_deref())
    }

    pub fn resolve_service_agent_id(portal: &Portal) -> Option<String> {
        let explicit = Self::normalize_agent_id(portal.service_agent_id.as_deref())
            .or_else(|| Self::normalize_agent_id(portal.agent_id.as_deref()));
        if Self::resolve_portal_domain(portal) == PortalDomain::Avatar {
            if let Some(candidate) = explicit {
                let coding = Self::normalize_agent_id(portal.coding_agent_id.as_deref());
                if coding.as_deref() == Some(candidate.as_str()) {
                    return None;
                }
                return Some(candidate);
            }
            return None;
        }
        explicit.or_else(|| Self::normalize_agent_id(portal.coding_agent_id.as_deref()))
    }

    fn domain_label(domain: PortalDomain) -> &'static str {
        match domain {
            PortalDomain::Ecosystem => "ecosystem",
            PortalDomain::Avatar => "avatar",
        }
    }

    fn parse_domain_str(raw: &str) -> Option<PortalDomain> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "ecosystem" => Some(PortalDomain::Ecosystem),
            "avatar" => Some(PortalDomain::Avatar),
            _ => None,
        }
    }

    fn parse_domain_filter(raw: Option<&str>) -> Result<Option<PortalDomain>> {
        let Some(raw) = raw.map(str::trim).filter(|s| !s.is_empty()) else {
            return Ok(None);
        };
        Self::parse_domain_str(raw)
            .map(Some)
            .ok_or_else(|| anyhow!("Invalid domain '{}'. Use ecosystem or avatar.", raw))
    }

    fn detect_domain_from_tags(tags: &[String]) -> PortalDomain {
        if tags.iter().any(|tag| {
            let v = tag.trim().to_ascii_lowercase();
            v == "digital-avatar" || v.starts_with("avatar:") || v == "domain:avatar"
        }) {
            PortalDomain::Avatar
        } else {
            PortalDomain::Ecosystem
        }
    }

    fn detect_domain_from_settings(settings: &serde_json::Value) -> Option<PortalDomain> {
        let raw = settings
            .get("domain")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())?;
        Self::parse_domain_str(raw)
    }

    fn resolve_portal_domain(portal: &Portal) -> PortalDomain {
        portal.domain.unwrap_or_else(|| {
            let from_tags = Self::detect_domain_from_tags(&portal.tags);
            if from_tags == PortalDomain::Avatar {
                return from_tags;
            }
            Self::detect_domain_from_settings(&portal.settings).unwrap_or(from_tags)
        })
    }

    fn normalize_domain_tags(tags: &mut Vec<String>, domain: PortalDomain) {
        tags.retain(|tag| {
            let v = tag.trim().to_ascii_lowercase();
            match domain {
                PortalDomain::Avatar => v != "domain:ecosystem",
                PortalDomain::Ecosystem => {
                    v != "digital-avatar" && !v.starts_with("avatar:") && v != "domain:avatar"
                }
            }
        });
        match domain {
            PortalDomain::Avatar => {
                if !tags
                    .iter()
                    .any(|tag| tag.trim().eq_ignore_ascii_case("digital-avatar"))
                {
                    tags.push("digital-avatar".to_string());
                }
                if !tags
                    .iter()
                    .any(|tag| tag.trim().eq_ignore_ascii_case("domain:avatar"))
                {
                    tags.push("domain:avatar".to_string());
                }
            }
            PortalDomain::Ecosystem => {
                if !tags
                    .iter()
                    .any(|tag| tag.trim().eq_ignore_ascii_case("domain:ecosystem"))
                {
                    tags.push("domain:ecosystem".to_string());
                }
            }
        }
    }

    fn settings_with_domain(
        settings: serde_json::Value,
        domain: PortalDomain,
    ) -> serde_json::Value {
        let mut obj = match settings {
            serde_json::Value::Object(obj) => obj,
            _ => serde_json::Map::new(),
        };
        obj.insert(
            "domain".to_string(),
            serde_json::Value::String(Self::domain_label(domain).to_string()),
        );
        serde_json::Value::Object(obj)
    }

    fn has_conflicting_domain_tag(tags: &[String], current: PortalDomain) -> bool {
        match current {
            PortalDomain::Avatar => tags
                .iter()
                .any(|tag| tag.trim().eq_ignore_ascii_case("domain:ecosystem")),
            PortalDomain::Ecosystem => tags.iter().any(|tag| {
                let v = tag.trim().to_ascii_lowercase();
                v == "digital-avatar" || v.starts_with("avatar:") || v == "domain:avatar"
            }),
        }
    }

    fn normalize_unique_string_list(values: impl IntoIterator<Item = String>) -> Vec<String> {
        let mut seen = HashSet::new();
        let mut out = Vec::new();
        for value in values {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                continue;
            }
            let normalized = trimmed.to_string();
            if seen.insert(normalized.clone()) {
                out.push(normalized);
            }
        }
        out
    }

    fn normalize_optional_unique_string_list(values: Option<&Vec<String>>) -> Option<Vec<String>> {
        let normalized = Self::normalize_unique_string_list(
            values
                .into_iter()
                .flat_map(|items| items.iter().cloned())
                .collect::<Vec<_>>(),
        );
        if normalized.is_empty() {
            None
        } else {
            Some(normalized)
        }
    }

    fn runtime_extension_id(extension: BuiltinExtension) -> &'static str {
        extension.mcp_name().unwrap_or_else(|| extension.name())
    }

    fn collect_agent_runtime_extensions(agent: &TeamAgentPolicyDoc) -> Vec<String> {
        let mut values = Vec::new();
        for cfg in &agent.enabled_extensions {
            if !cfg.enabled {
                continue;
            }
            values.push(Self::runtime_extension_id(cfg.extension).to_string());
        }
        for custom in &agent.custom_extensions {
            if !custom.enabled {
                continue;
            }
            values.push(custom.name.clone());
        }
        let mut normalized = Self::normalize_unique_string_list(values);
        normalized.sort();
        normalized
    }

    fn collect_agent_runtime_skills(agent: &TeamAgentPolicyDoc) -> Vec<String> {
        let mut normalized = Self::normalize_unique_string_list(
            agent
                .assigned_skills
                .iter()
                .filter(|skill| skill.enabled)
                .map(|skill| skill.skill_id.clone()),
        );
        normalized.sort();
        normalized
    }

    fn resolve_effective_document_access_mode(portal: &Portal) -> PortalDocumentAccessMode {
        let from_settings = portal
            .settings
            .get("documentAccessMode")
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .map(|value| value.to_ascii_lowercase());
        match from_settings.as_deref() {
            Some("read_only") | Some("readonly") | Some("read-only") => {
                PortalDocumentAccessMode::ReadOnly
            }
            Some("co_edit_draft") | Some("co-edit-draft") | Some("coeditdraft") => {
                PortalDocumentAccessMode::CoEditDraft
            }
            Some("controlled_write") | Some("controlled-write") | Some("controlledwrite") => {
                PortalDocumentAccessMode::ControlledWrite
            }
            _ => portal.document_access_mode,
        }
    }

    pub fn document_access_mode_key(mode: PortalDocumentAccessMode) -> &'static str {
        match mode {
            PortalDocumentAccessMode::ReadOnly => "read_only",
            PortalDocumentAccessMode::CoEditDraft => "co_edit_draft",
            PortalDocumentAccessMode::ControlledWrite => "controlled_write",
        }
    }

    pub fn resolve_show_chat_widget(portal: &Portal) -> bool {
        portal
            .settings
            .get("showChatWidget")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(true)
    }

    fn portal_governance_config_bool(portal: &Portal, key: &str) -> Option<bool> {
        portal
            .settings
            .get("digitalAvatarGovernanceConfig")
            .and_then(serde_json::Value::as_object)
            .and_then(|config| config.get(key))
            .and_then(serde_json::Value::as_bool)
            .or_else(|| {
                portal
                    .settings
                    .get("digitalAvatarGovernance")
                    .and_then(serde_json::Value::as_object)
                    .and_then(|governance| governance.get("config"))
                    .and_then(serde_json::Value::as_object)
                    .and_then(|config| config.get(key))
                    .and_then(serde_json::Value::as_bool)
            })
    }

    pub async fn resolve_require_human_for_publish(
        &self,
        team_id: &str,
        portal: &Portal,
    ) -> Result<bool> {
        if let Some(required) =
            Self::portal_governance_config_bool(portal, "requireHumanForPublish")
        {
            return Ok(required);
        }

        let team_settings = TeamService::new(self.db.clone())
            .get_settings(team_id)
            .await?;
        Ok(team_settings.avatar_governance.require_human_for_publish)
    }

    async fn warn_avatar_publish_shadow_gate(&self, team_id: &str, portal: &Portal) -> Result<()> {
        if !Self::is_digital_avatar_portal(portal) {
            return Ok(());
        }
        if !self
            .resolve_require_human_for_publish(team_id, portal)
            .await?
        {
            return Ok(());
        }
        tracing::warn!(
            team_id = %team_id,
            portal_id = %portal.id.map(|id| id.to_hex()).unwrap_or_default(),
            slug = %portal.slug,
            "avatar publish shadow gate: requireHumanForPublish is enabled; shared publish path still allows this publish for compatibility"
        );
        Ok(())
    }

    fn is_internal_worker_avatar(portal: &Portal) -> bool {
        portal
            .tags
            .iter()
            .any(|tag| tag.trim().eq_ignore_ascii_case("avatar:internal"))
            || Self::portal_setting_str(portal, "avatarType")
                .map(|value| value.eq_ignore_ascii_case("internal_worker"))
                .unwrap_or(false)
    }

    pub fn resolve_public_exposure(portal: &Portal) -> PortalPublicExposure {
        if portal.status != PortalStatus::Published {
            return PortalPublicExposure::PreviewOnly;
        }
        if portal.output_form != crate::models::mongo::PortalOutputForm::AgentOnly {
            return PortalPublicExposure::PublicPage;
        }
        if Self::is_digital_avatar_portal(portal) && !Self::is_internal_worker_avatar(portal) {
            return PortalPublicExposure::PublicPage;
        }
        PortalPublicExposure::PreviewOnly
    }

    pub async fn resolve_effective_public_config(
        &self,
        portal: &Portal,
    ) -> Result<PortalEffectivePublicConfig> {
        let service_agent_id = Self::resolve_service_agent_id(portal);
        let team_id = portal.team_id.to_hex();
        let service_agent = match service_agent_id.as_deref() {
            Some(agent_id) => match self.load_team_agent_policy(&team_id, agent_id).await {
                Ok(agent) => Some(agent),
                Err(err) => {
                    tracing::warn!(
                        "Failed to resolve service agent '{}' for portal '{}': {}",
                        agent_id,
                        portal.slug,
                        err
                    );
                    None
                }
            },
            None => None,
        };

        let requested_extensions =
            Self::normalize_optional_unique_string_list(portal.allowed_extensions.as_ref());
        let requested_skills =
            Self::normalize_optional_unique_string_list(portal.allowed_skill_ids.as_ref());

        let (effective_allowed_extensions, extensions_inherited) = match requested_extensions {
            Some(values) => (values, false),
            None => service_agent
                .as_ref()
                .map(Self::collect_agent_runtime_extensions)
                .map(|values| (values, true))
                .unwrap_or_else(|| (Vec::new(), true)),
        };
        let (effective_allowed_skill_ids, skills_inherited) = match requested_skills {
            Some(values) => (values, false),
            None => service_agent
                .as_ref()
                .map(Self::collect_agent_runtime_skills)
                .map(|values| (values, true))
                .unwrap_or_else(|| (Vec::new(), true)),
        };

        let exposure = Self::resolve_public_exposure(portal);
        let chat_enabled = portal.agent_enabled && service_agent_id.is_some();

        Ok(PortalEffectivePublicConfig {
            exposure,
            public_access_enabled: exposure == PortalPublicExposure::PublicPage,
            chat_enabled,
            show_chat_widget: Self::resolve_show_chat_widget(portal),
            effective_document_access_mode: Self::resolve_effective_document_access_mode(portal),
            effective_allowed_extensions,
            effective_allowed_skill_ids,
            extensions_inherited,
            skills_inherited,
        })
    }

    async fn load_team_agent_policy(
        &self,
        team_id: &str,
        agent_id: &str,
    ) -> Result<TeamAgentPolicyDoc> {
        let coll = self
            .db
            .collection::<TeamAgentPolicyDoc>(collections::TEAM_AGENTS);
        coll.find_one(doc! { "team_id": team_id, "agent_id": agent_id }, None)
            .await?
            .ok_or_else(|| anyhow!("Agent '{}' not found in team", agent_id))
    }

    fn collect_agent_extension_capabilities(agent: &TeamAgentPolicyDoc) -> HashSet<String> {
        let mut set = HashSet::new();

        for cfg in &agent.enabled_extensions {
            if !cfg.enabled {
                continue;
            }
            let builtin_name = cfg.extension.name().to_lowercase();
            set.insert(builtin_name);
            if let Some(mcp_name) = cfg.extension.mcp_name() {
                set.insert(mcp_name.to_lowercase());
            }
            if cfg.extension == BuiltinExtension::Skills {
                set.insert("team_skills".to_string());
            }
        }

        for custom in &agent.custom_extensions {
            if !custom.enabled {
                continue;
            }
            let name = custom.name.trim().to_lowercase();
            if !name.is_empty() {
                set.insert(name);
            }
        }

        // These are loaded by team-server runtime as platform fallback extensions.
        set.insert("document_tools".to_string());
        set.insert("portal_tools".to_string());

        set
    }

    fn collect_agent_skill_capabilities(agent: &TeamAgentPolicyDoc) -> HashSet<String> {
        agent
            .assigned_skills
            .iter()
            .filter(|s| s.enabled)
            .map(|s| s.skill_id.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect()
    }

    async fn validate_policy_scope(
        &self,
        team_id: &str,
        agent_id: Option<&str>,
        allowed_extensions: Option<&Vec<String>>,
        allowed_skill_ids: Option<&Vec<String>>,
    ) -> Result<()> {
        let normalized_agent_id = agent_id.map(str::trim).filter(|s| !s.is_empty());
        let has_extension_constraints = allowed_extensions
            .map(|items| items.iter().any(|s| !s.trim().is_empty()))
            .unwrap_or(false);
        let has_skill_constraints = allowed_skill_ids
            .map(|items| items.iter().any(|s| !s.trim().is_empty()))
            .unwrap_or(false);

        // Load agent once if specified (validates existence + provides policy for constraint checks).
        let agent = match normalized_agent_id {
            Some(id) => Some(self.load_team_agent_policy(team_id, id).await?),
            None => None,
        };

        if !has_extension_constraints && !has_skill_constraints {
            return Ok(());
        }

        let agent = agent.ok_or_else(|| anyhow!("agent_id is required when setting allowlists"))?;

        if let Some(exts) = allowed_extensions {
            let capabilities = Self::collect_agent_extension_capabilities(&agent);
            for ext in exts {
                let normalized = ext.trim().to_lowercase();
                if normalized.is_empty() {
                    continue;
                }
                if !capabilities.contains(&normalized) {
                    return Err(anyhow!(
                        "allowed extension '{}' is not available on agent '{}'",
                        ext,
                        agent.agent_id
                    ));
                }
            }
        }

        if let Some(skill_ids) = allowed_skill_ids {
            let capabilities = Self::collect_agent_skill_capabilities(&agent);
            for skill_id in skill_ids {
                let normalized = skill_id.trim();
                if normalized.is_empty() {
                    continue;
                }
                if !capabilities.contains(normalized) {
                    return Err(anyhow!(
                        "allowed skill '{}' is not enabled on agent '{}'",
                        skill_id,
                        agent.agent_id
                    ));
                }
            }
        }

        Ok(())
    }

    async fn validate_service_agent_binding(
        &self,
        team_id: &str,
        portal_domain: PortalDomain,
        service_agent_id: Option<&str>,
    ) -> Result<()> {
        let Some(service_agent_id) = service_agent_id.map(str::trim).filter(|s| !s.is_empty())
        else {
            return Ok(());
        };

        if portal_domain != PortalDomain::Ecosystem {
            return Ok(());
        }

        let agent = self
            .load_team_agent_policy(team_id, service_agent_id)
            .await?;
        let agent_domain = agent
            .agent_domain
            .as_deref()
            .map(str::trim)
            .unwrap_or("")
            .to_ascii_lowercase();
        let agent_role = agent
            .agent_role
            .as_deref()
            .map(str::trim)
            .unwrap_or("")
            .to_ascii_lowercase();

        match Self::classify_ecosystem_service_agent_binding(
            Some(agent_domain.as_str()),
            Some(agent_role.as_str()),
        ) {
            EcosystemServiceAgentBinding::GeneralTemplate => Err(anyhow!(
                "ecosystem portals cannot bind a general agent directly as service_agent_id; clone it into an ecosystem service agent first"
            )),
            EcosystemServiceAgentBinding::AvatarManager => Err(anyhow!(
                "digital avatar manager agents cannot be used as ecosystem portal service agents"
            )),
            EcosystemServiceAgentBinding::AvatarService
            | EcosystemServiceAgentBinding::EcosystemService => Ok(()),
            EcosystemServiceAgentBinding::AvatarOther => Err(anyhow!(
                "only digital avatar service agents can be shared into ecosystem portals"
            )),
            EcosystemServiceAgentBinding::EcosystemOther => Err(anyhow!(
                "ecosystem portal service agents must use the service role"
            )),
            EcosystemServiceAgentBinding::Unsupported => Err(anyhow!(
                "unsupported service agent domain '{}'; use an ecosystem service agent or a digital avatar service agent",
                agent_domain
            )),
        }
    }

    fn classify_ecosystem_service_agent_binding(
        agent_domain: Option<&str>,
        agent_role: Option<&str>,
    ) -> EcosystemServiceAgentBinding {
        let domain = agent_domain
            .map(str::trim)
            .unwrap_or("")
            .to_ascii_lowercase();
        let role = agent_role.map(str::trim).unwrap_or("").to_ascii_lowercase();

        if domain.is_empty() || domain == "general" {
            return EcosystemServiceAgentBinding::GeneralTemplate;
        }

        if domain == "digital_avatar" {
            return match role.as_str() {
                "service" => EcosystemServiceAgentBinding::AvatarService,
                "manager" => EcosystemServiceAgentBinding::AvatarManager,
                _ => EcosystemServiceAgentBinding::AvatarOther,
            };
        }

        if domain == "ecosystem_portal" {
            return if role.is_empty() || role == "service" {
                EcosystemServiceAgentBinding::EcosystemService
            } else {
                EcosystemServiceAgentBinding::EcosystemOther
            };
        }

        EcosystemServiceAgentBinding::Unsupported
    }

    fn classify_avatar_binding_shadow_issues(
        coding_agent_id: &str,
        manager_agent: &TeamAgentPolicyDoc,
        service_agent: &TeamAgentPolicyDoc,
    ) -> Vec<AvatarBindingShadowIssue> {
        let mut issues = Vec::new();

        if manager_agent.agent_domain.as_deref() != Some("digital_avatar")
            || manager_agent.agent_role.as_deref() != Some("manager")
        {
            issues.push(AvatarBindingShadowIssue::ManagerRoleMismatch);
        }
        if service_agent.agent_domain.as_deref() != Some("digital_avatar")
            || service_agent.agent_role.as_deref() != Some("service")
        {
            issues.push(AvatarBindingShadowIssue::ServiceRoleMismatch);
        }
        if service_agent.owner_manager_agent_id.as_deref() != Some(coding_agent_id) {
            issues.push(AvatarBindingShadowIssue::OwnerManagerMismatch);
        }

        issues
    }

    fn avatar_binding_shadow_issue_messages(
        coding_agent_id: &str,
        manager_agent: &TeamAgentPolicyDoc,
        service_agent: &TeamAgentPolicyDoc,
        issues: &[AvatarBindingShadowIssue],
    ) -> Vec<String> {
        issues
            .iter()
            .map(|issue| match issue {
                AvatarBindingShadowIssue::ManagerRoleMismatch => format!(
                    "coding agent '{}' is {}:{} instead of digital_avatar:manager",
                    manager_agent.agent_id,
                    manager_agent.agent_domain.as_deref().unwrap_or("general"),
                    manager_agent.agent_role.as_deref().unwrap_or("default")
                ),
                AvatarBindingShadowIssue::ServiceRoleMismatch => format!(
                    "service agent '{}' is {}:{} instead of digital_avatar:service",
                    service_agent.agent_id,
                    service_agent.agent_domain.as_deref().unwrap_or("general"),
                    service_agent.agent_role.as_deref().unwrap_or("default")
                ),
                AvatarBindingShadowIssue::OwnerManagerMismatch => format!(
                    "service agent '{}' owner_manager_agent_id is '{}' but coding agent is '{}'",
                    service_agent.agent_id,
                    service_agent
                        .owner_manager_agent_id
                        .as_deref()
                        .unwrap_or(""),
                    coding_agent_id
                ),
            })
            .collect()
    }

    fn validate_avatar_binding_policies(
        coding_agent_id: &str,
        manager_agent: &TeamAgentPolicyDoc,
        service_agent: &TeamAgentPolicyDoc,
    ) -> Result<()> {
        let issues = Self::classify_avatar_binding_shadow_issues(
            coding_agent_id,
            manager_agent,
            service_agent,
        );
        if issues.is_empty() {
            return Ok(());
        }

        let messages = Self::avatar_binding_shadow_issue_messages(
            coding_agent_id,
            manager_agent,
            service_agent,
            &issues,
        );
        Err(anyhow!(
            "avatar portals require coding_agent_id=digital_avatar:manager and service_agent_id=digital_avatar:service with matching owner_manager_agent_id; {}",
            messages.join("; ")
        ))
    }

    async fn validate_avatar_agent_binding(
        &self,
        team_id: &str,
        coding_agent_id: Option<&str>,
        service_agent_id: Option<&str>,
    ) -> Result<()> {
        let Some(coding_agent_id) = coding_agent_id.map(str::trim).filter(|id| !id.is_empty())
        else {
            return Ok(());
        };
        let Some(service_agent_id) = service_agent_id.map(str::trim).filter(|id| !id.is_empty())
        else {
            return Ok(());
        };

        let manager_agent = self
            .load_team_agent_policy(team_id, coding_agent_id)
            .await?;
        let service_agent = self
            .load_team_agent_policy(team_id, service_agent_id)
            .await?;
        Self::validate_avatar_binding_policies(coding_agent_id, &manager_agent, &service_agent)
    }

    async fn warn_avatar_binding_shadow(
        &self,
        team_id: &str,
        portal_ref: &str,
        coding_agent_id: Option<&str>,
        service_agent_id: Option<&str>,
    ) {
        let Some(coding_agent_id) = coding_agent_id.map(str::trim).filter(|id| !id.is_empty())
        else {
            return;
        };
        let Some(service_agent_id) = service_agent_id.map(str::trim).filter(|id| !id.is_empty())
        else {
            return;
        };

        let manager_agent = match self.load_team_agent_policy(team_id, coding_agent_id).await {
            Ok(agent) => agent,
            Err(err) => {
                tracing::warn!(
                    team_id = %team_id,
                    portal_ref = %portal_ref,
                    coding_agent_id = %coding_agent_id,
                    service_agent_id = %service_agent_id,
                    error = %err,
                    "avatar binding shadow audit could not load coding agent policy"
                );
                return;
            }
        };
        let service_agent = match self.load_team_agent_policy(team_id, service_agent_id).await {
            Ok(agent) => agent,
            Err(err) => {
                tracing::warn!(
                    team_id = %team_id,
                    portal_ref = %portal_ref,
                    coding_agent_id = %coding_agent_id,
                    service_agent_id = %service_agent_id,
                    error = %err,
                    "avatar binding shadow audit could not load service agent policy"
                );
                return;
            }
        };

        let issues = Self::classify_avatar_binding_shadow_issues(
            coding_agent_id,
            &manager_agent,
            &service_agent,
        );

        if !issues.is_empty() {
            let issue_messages = Self::avatar_binding_shadow_issue_messages(
                coding_agent_id,
                &manager_agent,
                &service_agent,
                &issues,
            );
            tracing::warn!(
                team_id = %team_id,
                portal_ref = %portal_ref,
                coding_agent_id = %coding_agent_id,
                service_agent_id = %service_agent_id,
                issues = %issue_messages.join("; "),
                "avatar portal binding passes shared service validation but violates avatar-specific invariants"
            );
        }
    }

    // -----------------------------------------------------------------------
    // Portal CRUD
    // -----------------------------------------------------------------------

    #[allow(clippy::too_many_lines)]
    pub async fn create(
        &self,
        team_id: &str,
        user_id: &str,
        req: CreatePortalRequest,
    ) -> Result<Portal> {
        let CreatePortalRequest {
            name: raw_name,
            slug: raw_slug,
            description,
            output_form,
            agent_enabled,
            coding_agent_id: raw_coding_agent_id,
            service_agent_id: raw_service_agent_id,
            agent_id: raw_agent_id,
            agent_system_prompt,
            agent_welcome_message,
            bound_document_ids,
            allowed_extensions,
            allowed_skill_ids,
            document_access_mode,
            tags,
            settings,
        } = req;

        let name = raw_name.trim().to_string();
        if name.is_empty() || name.len() > 200 {
            return Err(anyhow!("Portal name must be between 1 and 200 characters"));
        }

        let team_oid = ObjectId::parse_str(team_id)?;
        let now = Utc::now();
        let slug = match raw_slug {
            Some(s) if !s.is_empty() => {
                validate_slug(&s)?;
                self.ensure_slug_available(&s).await?;
                s
            }
            _ => self.generate_slug(&name).await?,
        };

        let mut tags = tags.unwrap_or_default();
        let mut settings = settings.unwrap_or(serde_json::json!({}));
        let detected_domain = Self::detect_domain_from_tags(&tags);
        let domain = if detected_domain == PortalDomain::Avatar {
            PortalDomain::Avatar
        } else {
            Self::detect_domain_from_settings(&settings).unwrap_or(PortalDomain::Ecosystem)
        };
        Self::normalize_domain_tags(&mut tags, domain);
        settings = Self::settings_with_domain(settings, domain);

        let legacy_agent_id = Self::normalize_agent_id(raw_agent_id.as_deref());
        let effective_agent_enabled = agent_enabled.unwrap_or(false);
        let (coding_agent_id, service_agent_id) = if domain == PortalDomain::Avatar {
            let coding = Self::normalize_agent_id(raw_coding_agent_id.as_deref());
            let service = Self::normalize_agent_id(raw_service_agent_id.as_deref());
            if effective_agent_enabled && coding.is_none() {
                return Err(anyhow!(
                    "coding_agent_id is required for avatar portals when agent_enabled is true"
                ));
            }
            if effective_agent_enabled && service.is_none() {
                return Err(anyhow!(
                    "service_agent_id is required for avatar portals when agent_enabled is true"
                ));
            }
            if coding.is_some() && coding == service {
                return Err(anyhow!(
                    "avatar portals require a dedicated service_agent_id distinct from coding_agent_id"
                ));
            }
            (coding, service)
        } else {
            let mut coding = Self::normalize_agent_id(raw_coding_agent_id.as_deref())
                .or_else(|| legacy_agent_id.clone());
            let mut service = Self::normalize_agent_id(raw_service_agent_id.as_deref())
                .or_else(|| legacy_agent_id.clone());
            if coding.is_none() {
                coding = service.clone();
            }
            if service.is_none() {
                service = coding.clone();
            }
            (coding, service)
        };

        if effective_agent_enabled && service_agent_id.is_none() {
            return Err(anyhow!(
                "service_agent_id is required when agent_enabled is true"
            ));
        }

        self.validate_policy_scope(team_id, coding_agent_id.as_deref(), None, None)
            .await?;
        self.validate_policy_scope(
            team_id,
            service_agent_id.as_deref(),
            allowed_extensions.as_ref(),
            allowed_skill_ids.as_ref(),
        )
        .await?;
        self.validate_service_agent_binding(team_id, domain, service_agent_id.as_deref())
            .await?;
        if domain == PortalDomain::Avatar {
            self.validate_avatar_agent_binding(
                team_id,
                coding_agent_id.as_deref(),
                service_agent_id.as_deref(),
            )
            .await?;
            self.warn_avatar_binding_shadow(
                team_id,
                &slug,
                coding_agent_id.as_deref(),
                service_agent_id.as_deref(),
            )
            .await;
        }

        let portal = Portal {
            id: None,
            team_id: team_oid,
            slug,
            name,
            description,
            status: PortalStatus::Draft,
            output_form: output_form.unwrap_or_default(),
            agent_enabled: effective_agent_enabled,
            coding_agent_id: coding_agent_id.clone(),
            service_agent_id: service_agent_id.clone(),
            // Keep legacy field aligned for compatibility.
            agent_id: service_agent_id.clone().or_else(|| coding_agent_id.clone()),
            agent_system_prompt,
            agent_welcome_message,
            bound_document_ids: bound_document_ids.unwrap_or_default(),
            allowed_extensions,
            allowed_skill_ids,
            document_access_mode: document_access_mode.unwrap_or_default(),
            domain: Some(domain),
            tags,
            settings,
            project_path: None,
            created_by: user_id.to_string(),
            is_deleted: false,
            published_at: None,
            created_at: now,
            updated_at: now,
        };

        let coll = self.db.collection::<Portal>(collections::PORTALS);
        let result = coll.insert_one(&portal, None).await?;
        let mut portal = portal;
        portal.id = result.inserted_id.as_object_id();
        Ok(portal)
    }

    pub async fn get(&self, team_id: &str, portal_id: &str) -> Result<Portal> {
        let team_oid = ObjectId::parse_str(team_id)?;
        let portal_oid = ObjectId::parse_str(portal_id)?;
        let coll = self.db.collection::<Portal>(collections::PORTALS);
        coll.find_one(
            doc! { "_id": portal_oid, "team_id": team_oid, "is_deleted": { "$ne": true } },
            None,
        )
        .await?
        .ok_or_else(|| anyhow!("Portal not found"))
    }

    pub async fn get_by_slug(&self, slug: &str) -> Result<Portal> {
        let coll = self.db.collection::<Portal>(collections::PORTALS);
        coll.find_one(doc! { "slug": slug, "is_deleted": { "$ne": true } }, None)
            .await?
            .ok_or_else(|| anyhow!("Portal not found"))
    }

    pub async fn list(
        &self,
        team_id: &str,
        page: u64,
        limit: u64,
        domain: Option<&str>,
    ) -> Result<PaginatedResponse<PortalSummary>> {
        let team_oid = ObjectId::parse_str(team_id)?;
        let coll = self.db.collection::<Portal>(collections::PORTALS);
        let mut filter = doc! { "team_id": team_oid, "is_deleted": { "$ne": true } };
        match Self::parse_domain_filter(domain)? {
            Some(PortalDomain::Avatar) => {
                // Backward compatible filter:
                // - Prefer explicit domain field
                // - Fallback to legacy tags when domain is missing/null
                filter.insert(
                    "$or",
                    Bson::Array(vec![
                        Bson::Document(doc! { "domain": "avatar" }),
                        Bson::Document(doc! {
                            "domain": { "$exists": false },
                            "tags": "digital-avatar"
                        }),
                        Bson::Document(doc! {
                            "domain": Bson::Null,
                            "tags": "digital-avatar"
                        }),
                    ]),
                );
            }
            Some(PortalDomain::Ecosystem) => {
                filter.insert(
                    "$or",
                    Bson::Array(vec![
                        Bson::Document(doc! { "domain": "ecosystem" }),
                        Bson::Document(doc! {
                            "domain": { "$exists": false },
                            "tags": { "$ne": "digital-avatar" }
                        }),
                        Bson::Document(doc! {
                            "domain": Bson::Null,
                            "tags": { "$ne": "digital-avatar" }
                        }),
                    ]),
                );
            }
            None => {}
        }

        let total = coll.count_documents(filter.clone(), None).await?;
        let skip = (page.saturating_sub(1)) * limit;
        let opts = FindOptions::builder()
            .sort(doc! { "created_at": -1 })
            .skip(skip)
            .limit(limit as i64)
            .build();

        let cursor = coll.find(filter, opts).await?;
        let portals: Vec<Portal> = cursor.try_collect().await?;
        let items: Vec<PortalSummary> = portals.into_iter().map(PortalSummary::from).collect();

        Ok(PaginatedResponse::new(items, total, page, limit))
    }

    #[allow(clippy::too_many_lines)]
    pub async fn update(
        &self,
        team_id: &str,
        portal_id: &str,
        req: UpdatePortalRequest,
    ) -> Result<Portal> {
        let UpdatePortalRequest {
            name,
            slug,
            description,
            output_form,
            agent_enabled,
            coding_agent_id: req_coding_agent_id,
            service_agent_id: req_service_agent_id,
            agent_id: req_legacy_agent_id,
            agent_system_prompt,
            agent_welcome_message,
            bound_document_ids,
            allowed_extensions,
            allowed_skill_ids,
            document_access_mode,
            tags,
            settings,
        } = req;

        let team_oid = ObjectId::parse_str(team_id)?;
        let portal_oid = ObjectId::parse_str(portal_id)?;
        let current = self.get(team_id, portal_id).await?;
        let current_domain = Self::resolve_portal_domain(&current);

        let legacy_update = req_legacy_agent_id.clone();
        let coding_update = req_coding_agent_id
            .clone()
            .or_else(|| legacy_update.clone());
        let service_update = req_service_agent_id
            .clone()
            .or_else(|| legacy_update.clone());

        let current_coding_agent_id = if current_domain == PortalDomain::Avatar {
            Self::normalize_agent_id(current.coding_agent_id.as_deref())
        } else {
            Self::resolve_coding_agent_id(&current)
        };
        let current_service_agent_id = if current_domain == PortalDomain::Avatar {
            Self::normalize_agent_id(current.service_agent_id.as_deref())
        } else {
            Self::resolve_service_agent_id(&current)
        };

        let effective_coding_agent_id = match coding_update.as_ref() {
            Some(v) => Self::normalize_agent_id(v.as_deref()),
            None => current_coding_agent_id.clone(),
        };
        let effective_service_agent_id = match service_update.as_ref() {
            Some(v) => Self::normalize_agent_id(v.as_deref()),
            None => current_service_agent_id.clone(),
        };

        let clearing_service_agent = matches!(service_update.as_ref(), Some(None));
        let effective_allowed_extensions = if clearing_service_agent && allowed_extensions.is_none()
        {
            None
        } else {
            allowed_extensions
                .as_ref()
                .or(current.allowed_extensions.as_ref())
        };
        let effective_allowed_skill_ids = if clearing_service_agent && allowed_skill_ids.is_none() {
            None
        } else {
            allowed_skill_ids
                .as_ref()
                .or(current.allowed_skill_ids.as_ref())
        };
        let effective_document_access_mode =
            document_access_mode.unwrap_or(current.document_access_mode);
        let effective_agent_enabled = agent_enabled.unwrap_or(current.agent_enabled);
        let touches_avatar_agent_binding = current_domain == PortalDomain::Avatar
            && (req_coding_agent_id.is_some()
                || req_service_agent_id.is_some()
                || req_legacy_agent_id.is_some()
                || agent_enabled.is_some());
        if touches_avatar_agent_binding
            && effective_agent_enabled
            && effective_coding_agent_id.is_none()
        {
            return Err(anyhow!(
                "coding_agent_id is required for avatar portals when agent_enabled is true"
            ));
        }
        if effective_agent_enabled && effective_service_agent_id.is_none() {
            return Err(anyhow!(
                "service_agent_id is required when agent_enabled is true"
            ));
        }
        if touches_avatar_agent_binding
            && effective_coding_agent_id.is_some()
            && effective_coding_agent_id == effective_service_agent_id
        {
            return Err(anyhow!(
                "avatar portals require a dedicated service_agent_id distinct from coding_agent_id"
            ));
        }

        self.validate_policy_scope(team_id, effective_coding_agent_id.as_deref(), None, None)
            .await?;
        self.validate_policy_scope(
            team_id,
            effective_service_agent_id.as_deref(),
            effective_allowed_extensions,
            effective_allowed_skill_ids,
        )
        .await?;
        if req_service_agent_id.is_some() || req_legacy_agent_id.is_some() {
            self.validate_service_agent_binding(
                team_id,
                current_domain,
                effective_service_agent_id.as_deref(),
            )
            .await?;
        }
        if current_domain == PortalDomain::Avatar {
            if touches_avatar_agent_binding {
                self.validate_avatar_agent_binding(
                    team_id,
                    effective_coding_agent_id.as_deref(),
                    effective_service_agent_id.as_deref(),
                )
                .await?;
            }
            self.warn_avatar_binding_shadow(
                team_id,
                portal_id,
                effective_coding_agent_id.as_deref(),
                effective_service_agent_id.as_deref(),
            )
            .await;
        }

        let mut tags = tags;
        let mut settings = settings;
        if let Some(ref tag_list) = tags {
            if Self::has_conflicting_domain_tag(tag_list, current_domain) {
                return Err(anyhow!(
                    "Cannot change portal domain via tags. Current domain is '{}'.",
                    Self::domain_label(current_domain)
                ));
            }
        }
        if let Some(ref s) = settings {
            if let Some(explicit_domain) = Self::detect_domain_from_settings(s) {
                if explicit_domain != current_domain {
                    return Err(anyhow!(
                        "Cannot change portal domain via settings. Current domain is '{}'.",
                        Self::domain_label(current_domain)
                    ));
                }
            }
        }
        if let Some(ref mut tag_list) = tags {
            Self::normalize_domain_tags(tag_list, current_domain);
        }
        if let Some(s) = settings.take() {
            settings = Some(Self::settings_with_domain(s, current_domain));
        }

        let coll = self.db.collection::<Portal>(collections::PORTALS);

        let mut update_doc = doc! { "updated_at": bson::DateTime::from_chrono(Utc::now()) };

        if let Some(name) = &name {
            let name = name.trim();
            if name.is_empty() || name.len() > 200 {
                return Err(anyhow!("Portal name must be between 1 and 200 characters"));
            }
            update_doc.insert("name", name);
        }
        if let Some(slug) = &slug {
            validate_slug(slug)?;
            // Check slug uniqueness (excluding self)
            let existing = coll
                .find_one(
                    doc! { "slug": slug, "_id": { "$ne": portal_oid }, "is_deleted": { "$ne": true } },
                    None,
                )
                .await?;
            if existing.is_some() {
                return Err(anyhow!("Slug '{}' is already taken", slug));
            }
            update_doc.insert("slug", slug);
        }
        if let Some(desc) = &description {
            update_doc.insert("description", desc);
        }
        if let Some(form) = &output_form {
            update_doc.insert("output_form", bson::to_bson(form)?);
        }
        if let Some(enabled) = agent_enabled {
            update_doc.insert("agent_enabled", enabled);
        }
        let coding_update_present = coding_update.is_some();
        let service_update_present = service_update.is_some();

        if let Some(update) = coding_update.as_ref() {
            match Self::normalize_agent_id(update.as_deref()) {
                Some(v) => update_doc.insert("coding_agent_id", v),
                None => update_doc.insert("coding_agent_id", bson::Bson::Null),
            };
        }
        if let Some(update) = service_update.as_ref() {
            match Self::normalize_agent_id(update.as_deref()) {
                Some(v) => update_doc.insert("service_agent_id", v),
                None => update_doc.insert("service_agent_id", bson::Bson::Null),
            };
        }
        if let Some(update) = legacy_update.as_ref() {
            match Self::normalize_agent_id(update.as_deref()) {
                Some(v) => update_doc.insert("agent_id", v),
                None => update_doc.insert("agent_id", bson::Bson::Null),
            };
        } else if coding_update_present || service_update_present {
            match effective_service_agent_id
                .clone()
                .or_else(|| effective_coding_agent_id.clone())
            {
                Some(v) => update_doc.insert("agent_id", v),
                None => update_doc.insert("agent_id", bson::Bson::Null),
            };
        }

        if let Some(prompt) = agent_system_prompt {
            match prompt {
                Some(v) => update_doc.insert("agent_system_prompt", v),
                None => update_doc.insert("agent_system_prompt", bson::Bson::Null),
            };
        }
        if let Some(msg) = agent_welcome_message {
            match msg {
                Some(v) => update_doc.insert("agent_welcome_message", v),
                None => update_doc.insert("agent_welcome_message", bson::Bson::Null),
            };
        }
        if let Some(ref ids) = bound_document_ids {
            update_doc.insert("bound_document_ids", ids);
        }
        if let Some(ref allowed_exts) = allowed_extensions {
            update_doc.insert("allowed_extensions", allowed_exts);
        } else if clearing_service_agent {
            update_doc.insert("allowed_extensions", bson::Bson::Null);
        }
        if let Some(ref allowed_skills) = allowed_skill_ids {
            update_doc.insert("allowed_skill_ids", allowed_skills);
        } else if clearing_service_agent {
            update_doc.insert("allowed_skill_ids", bson::Bson::Null);
        }
        update_doc.insert(
            "document_access_mode",
            bson::to_bson(&effective_document_access_mode)?,
        );
        update_doc.insert("domain", bson::to_bson(&current_domain)?);
        if let Some(ref tags) = tags {
            update_doc.insert("tags", tags);
        }
        if let Some(ref settings) = settings {
            update_doc.insert("settings", bson::to_bson(settings)?);
        }

        let filter = doc! { "_id": portal_oid, "team_id": team_oid, "is_deleted": { "$ne": true } };
        let result = coll
            .update_one(filter, doc! { "$set": update_doc }, None)
            .await?;
        if result.matched_count == 0 {
            return Err(anyhow!("Portal not found"));
        }

        // Auto-mark/unmark public documents when bound_document_ids changes
        if let Some(ref new_ids) = bound_document_ids {
            let old_ids: HashSet<&str> = current
                .bound_document_ids
                .iter()
                .map(|s| s.as_str())
                .collect();
            let new_set: HashSet<&str> = new_ids.iter().map(|s| s.as_str()).collect();

            let doc_svc = crate::services::mongo::DocumentService::new(self.db.clone());
            let folder_svc = crate::services::mongo::FolderService::new(self.db.clone());

            // Newly added docs → mark public + move to /公共文档
            let added: Vec<&str> = new_set.difference(&old_ids).copied().collect();
            if !added.is_empty() {
                folder_svc
                    .ensure_system_folder(team_id, "公共文档", "/")
                    .await
                    .ok();
                for doc_id in &added {
                    doc_svc.set_public(team_id, doc_id, true).await.ok();
                    doc_svc
                        .move_to_folder(team_id, doc_id, "/公共文档")
                        .await
                        .ok();
                }
            }

            // Removed docs → unmark if no other portal references them
            // H-6: exclude current portal to avoid TOCTOU race
            let removed: Vec<&str> = old_ids.difference(&new_set).copied().collect();
            for doc_id in &removed {
                let refs = self
                    .find_portals_by_document_id(team_id, doc_id, Some(portal_id))
                    .await
                    .unwrap_or_default();
                if refs.is_empty() {
                    doc_svc.set_public(team_id, doc_id, false).await.ok();
                }
            }
        }

        self.get(team_id, portal_id).await
    }

    pub async fn delete(&self, team_id: &str, portal_id: &str) -> Result<()> {
        let team_oid = ObjectId::parse_str(team_id)?;
        let portal_oid = ObjectId::parse_str(portal_id)?;
        // M-9: Fetch portal before soft-delete to get bound_document_ids
        let current = self.get(team_id, portal_id).await?;
        let coll = self.db.collection::<Portal>(collections::PORTALS);
        let result = coll
            .update_one(
                doc! { "_id": portal_oid, "team_id": team_oid, "is_deleted": { "$ne": true } },
                doc! { "$set": { "is_deleted": true, "updated_at": bson::DateTime::from_chrono(Utc::now()) } },
                None,
            )
            .await?;
        if result.matched_count == 0 {
            return Err(anyhow!("Portal not found"));
        }
        // M-9: Unmark public docs no longer referenced by any portal
        if !current.bound_document_ids.is_empty() {
            let doc_svc = crate::services::mongo::DocumentService::new(self.db.clone());
            for doc_id in &current.bound_document_ids {
                let refs = self
                    .find_portals_by_document_id(team_id, doc_id, None)
                    .await
                    .unwrap_or_default();
                if refs.is_empty() {
                    doc_svc.set_public(team_id, doc_id, false).await.ok();
                }
            }
        }
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Publish / Unpublish
    // -----------------------------------------------------------------------

    pub async fn publish(&self, team_id: &str, portal_id: &str) -> Result<Portal> {
        let team_oid = ObjectId::parse_str(team_id)?;
        let portal_oid = ObjectId::parse_str(portal_id)?;
        let coll = self.db.collection::<Portal>(collections::PORTALS);
        let now = Utc::now();
        if let Some(current_portal) = coll
            .find_one(
                doc! {
                    "_id": portal_oid,
                    "team_id": team_oid,
                    "is_deleted": { "$ne": true }
                },
                None,
            )
            .await?
        {
            self.warn_avatar_publish_shadow_gate(team_id, &current_portal)
                .await?;
        }
        // H-5: Only allow publishing from draft/archived states
        let result = coll
            .update_one(
                doc! {
                    "_id": portal_oid, "team_id": team_oid,
                    "is_deleted": { "$ne": true },
                    "status": { "$in": ["draft", "archived"] },
                },
                doc! { "$set": {
                    "status": "published",
                    "published_at": bson::DateTime::from_chrono(now),
                    "updated_at": bson::DateTime::from_chrono(now),
                }},
                None,
            )
            .await?;
        if result.matched_count == 0 {
            return Err(anyhow!("Portal not found or already published"));
        }
        self.get(team_id, portal_id).await
    }

    pub async fn unpublish(&self, team_id: &str, portal_id: &str) -> Result<Portal> {
        let team_oid = ObjectId::parse_str(team_id)?;
        let portal_oid = ObjectId::parse_str(portal_id)?;
        let coll = self.db.collection::<Portal>(collections::PORTALS);
        let now = Utc::now();
        // H-5: Only allow unpublishing from published state
        let result = coll
            .update_one(
                doc! {
                    "_id": portal_oid, "team_id": team_oid,
                    "is_deleted": { "$ne": true },
                    "status": "published",
                },
                doc! { "$set": {
                    "status": "draft",
                    "updated_at": bson::DateTime::from_chrono(now),
                }},
                None,
            )
            .await?;
        if result.matched_count == 0 {
            return Err(anyhow!("Portal not found or not published"));
        }
        self.get(team_id, portal_id).await
    }

    /// Set the project_path for a portal (called after creating the project folder)
    pub async fn set_project_path(
        &self,
        team_id: &str,
        portal_id: &str,
        project_path: &str,
    ) -> Result<()> {
        let team_oid = ObjectId::parse_str(team_id)?;
        let portal_oid = ObjectId::parse_str(portal_id)?;
        let coll = self.db.collection::<Portal>(collections::PORTALS);
        let now = Utc::now();
        let normalized_project_path = Self::normalize_project_path(PathBuf::from(project_path));
        coll.update_one(
            doc! { "_id": portal_oid, "team_id": team_oid, "is_deleted": { "$ne": true } },
            doc! { "$set": {
                "project_path": normalized_project_path,
                "updated_at": bson::DateTime::from_chrono(now),
            }},
            None,
        )
        .await?;
        Ok(())
    }

    fn normalize_project_path(path: PathBuf) -> String {
        let canonical = path.canonicalize().unwrap_or(path);
        let display = canonical.to_string_lossy().to_string();
        #[cfg(windows)]
        {
            if let Some(stripped) = display.strip_prefix(r"\\?\") {
                stripped.to_string()
            } else {
                display
            }
        }
        #[cfg(not(windows))]
        {
            display
        }
    }

    /// Compute project path: {workspace_root}/portals/{team_id}/{slug}/
    pub fn compute_project_path(workspace_root: &str, team_id: &str, slug: &str) -> String {
        let workspace = Path::new(workspace_root);
        let workspace_abs = if workspace.is_absolute() {
            workspace.to_path_buf()
        } else {
            std::env::current_dir()
                .unwrap_or_else(|_| PathBuf::from("."))
                .join(workspace)
        };
        let path = workspace_abs.join("portals").join(team_id).join(slug);
        Self::normalize_project_path(path)
    }

    fn escape_html(s: &str) -> String {
        s.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;")
    }

    fn default_portal_index_html(name: &str) -> String {
        let safe = Self::escape_html(name);
        format!(
            r##"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>{}</title>
<style>
body {{ margin: 0; font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif; display: flex; align-items: center; justify-content: center; min-height: 100vh; background: #f8fafc; color: #1a1a1a; }}
.container {{ text-align: center; padding: 2rem; }}
h1 {{ font-size: 2rem; margin-bottom: 0.5rem; }}
p {{ color: #64748b; }}
</style>
</head>
<body>
<div class="container">
<h1>{}</h1>
<p>This portal is ready for development.</p>
</div>
</body>
</html>"##,
            safe, safe
        )
    }

    pub fn is_digital_avatar_portal(portal: &Portal) -> bool {
        portal
            .tags
            .iter()
            .any(|tag| tag.trim().eq_ignore_ascii_case("digital-avatar"))
    }

    fn portal_setting_str(portal: &Portal, key: &str) -> Option<String> {
        portal
            .settings
            .get(key)
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToString::to_string)
    }

    fn portal_governance_config_str(portal: &Portal, key: &str) -> Option<String> {
        portal
            .settings
            .get("digitalAvatarGovernanceConfig")
            .and_then(serde_json::Value::as_object)
            .and_then(|config| config.get(key))
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToString::to_string)
    }

    pub fn resolve_avatar_type_label(portal: &Portal) -> &'static str {
        if portal
            .tags
            .iter()
            .any(|tag| tag.trim().eq_ignore_ascii_case("avatar:internal"))
            || Self::portal_setting_str(portal, "avatarType")
                .map(|value| value.eq_ignore_ascii_case("internal_worker"))
                .unwrap_or(false)
        {
            "内部执行分身"
        } else {
            "对外服务分身"
        }
    }

    pub fn resolve_run_mode_label(portal: &Portal) -> &'static str {
        match Self::portal_setting_str(portal, "runMode")
            .unwrap_or_else(|| "on_demand".to_string())
            .as_str()
        {
            "scheduled" => "定时运行",
            "event_driven" => "事件触发",
            _ => "按需响应",
        }
    }

    pub fn resolve_doc_mode_label(portal: &Portal) -> &'static str {
        Self::resolve_doc_mode_label_for_mode(Self::resolve_effective_document_access_mode(portal))
    }

    pub fn resolve_doc_mode_label_for_mode(mode: PortalDocumentAccessMode) -> &'static str {
        match mode {
            crate::models::mongo::PortalDocumentAccessMode::ReadOnly => "只读",
            crate::models::mongo::PortalDocumentAccessMode::CoEditDraft => "协作草稿",
            crate::models::mongo::PortalDocumentAccessMode::ControlledWrite => "受控写入",
        }
    }

    fn render_capability_items(portal: &Portal) -> String {
        let mut items = Vec::new();
        items.push("在绑定文档范围内进行检索、总结、问答与结构化输出".to_string());
        if portal.agent_enabled {
            items.push("支持持续对话，并保留会话上下文".to_string());
        }
        if !portal.bound_document_ids.is_empty() {
            items.push(format!(
                "已绑定文档: {} 份（可在页面下方查看）",
                portal.bound_document_ids.len()
            ));
        } else {
            items.push("当前未绑定文档，能力将以通用对话为主".to_string());
        }
        if let Some(extensions) = &portal.allowed_extensions {
            if !extensions.is_empty() {
                items.push(format!(
                    "允许扩展: {}",
                    Self::escape_html(&extensions.join(", "))
                ));
            }
        }
        if let Some(skills) = &portal.allowed_skill_ids {
            if !skills.is_empty() {
                items.push(format!(
                    "允许技能: {}",
                    Self::escape_html(&skills.join(", "))
                ));
            }
        }
        items
            .into_iter()
            .map(|item| format!("<li>{}</li>", item))
            .collect::<Vec<_>>()
            .join("")
    }

    fn render_badge_list(items: &[String], empty_text: &str) -> String {
        if items.is_empty() {
            return format!(
                r#"<span class="empty-badge">{}</span>"#,
                Self::escape_html(empty_text)
            );
        }
        items
            .iter()
            .map(|item| {
                format!(
                    r#"<span class="tag-chip">{}</span>"#,
                    Self::escape_html(item)
                )
            })
            .collect::<Vec<_>>()
            .join("")
    }

    fn render_avatar_boundary_items(portal: &Portal) -> String {
        let mut items = vec![
            "不会访问未绑定文档，也不会越过当前允许的扩展/技能范围。".to_string(),
            "无法满足需求时，会先把能力缺口上交给管理 Agent，而不是自行越权。".to_string(),
        ];
        match Self::resolve_effective_document_access_mode(portal) {
            crate::models::mongo::PortalDocumentAccessMode::ReadOnly => {
                items.push("当前仅支持读取与检索，不会改写文档内容。".to_string());
            }
            crate::models::mongo::PortalDocumentAccessMode::CoEditDraft => {
                items.push("当前仅允许在协作草稿范围内写入，不会直接影响正式文档。".to_string());
            }
            crate::models::mongo::PortalDocumentAccessMode::ControlledWrite => {
                items
                    .push("当前允许直接写入目标文档，也可以继续沿用相关 AI 文档版本。".to_string());
            }
        }
        items
            .into_iter()
            .map(|item| format!("<li>{}</li>", item))
            .collect::<Vec<_>>()
            .join("")
    }

    fn avatar_example_prompt_values(portal: &Portal) -> Vec<&'static str> {
        if portal
            .tags
            .iter()
            .any(|tag| tag.trim().eq_ignore_ascii_case("avatar:internal"))
        {
            vec![
                "帮我梳理这个流程的执行步骤，并标出需要人工确认的节点。",
                "根据绑定文档生成一版内部执行清单，便于后续复用。",
                "复盘最近一次失败，给我三个最小改动的优化建议。",
            ]
        } else {
            vec![
                "先告诉我你能处理哪些问题，以及哪些事情需要转给管理 Agent。",
                "根据绑定文档回答这个问题，并把结论讲清楚给我。",
                "如果当前能力不够，请说明缺什么能力，并告诉我下一步会怎么处理。",
            ]
        }
    }

    fn render_avatar_example_prompts(portal: &Portal) -> String {
        Self::avatar_example_prompt_values(portal)
            .into_iter()
            .map(|item| format!("<li>{}</li>", Self::escape_html(item)))
            .collect::<Vec<_>>()
            .join("")
    }

    fn render_avatar_example_prompt_buttons(portal: &Portal, disabled: bool) -> String {
        let disabled_attr = if disabled { " disabled" } else { "" };
        Self::avatar_example_prompt_values(portal)
            .into_iter()
            .map(|item| {
                format!(
                    r#"<button type="button" class="prompt-chip" data-avatar-prompt="{}"{}><span class="prompt-chip-label">推荐开场</span><span class="prompt-chip-text">{}</span></button>"#,
                    Self::escape_html(item),
                    disabled_attr,
                    Self::escape_html(item)
                )
            })
            .collect::<Vec<_>>()
            .join("")
    }

    fn render_avatar_boundary_cards(portal: &Portal) -> String {
        let doc_scope = if portal.bound_document_ids.is_empty() {
            "当前未绑定文档，分身只能做通用对话，无法引用团队专属材料。".to_string()
        } else {
            format!(
                "当前仅可访问已绑定的 {} 份文档，不会读取团队中的其他文档。",
                portal.bound_document_ids.len()
            )
        };
        let write_policy = match portal.document_access_mode {
            crate::models::mongo::PortalDocumentAccessMode::ReadOnly => {
                "当前是只读模式，只会读取、检索和总结，不会改写文档。".to_string()
            }
            crate::models::mongo::PortalDocumentAccessMode::CoEditDraft => {
                "当前仅允许写入协作草稿，不会直接改动正式文档。".to_string()
            }
            crate::models::mongo::PortalDocumentAccessMode::ControlledWrite => {
                "当前支持直接写入目标文档，也会保留相关 AI 版本，便于继续迭代。".to_string()
            }
        };
        let escalation =
            "若当前能力或权限不足，会先说明缺口，再交由管理 Agent 决策，不会自行越权。";
        [
            ("文档范围", doc_scope),
            ("写入策略", write_policy),
            ("升级路径", escalation.to_string()),
        ]
        .into_iter()
        .map(|(title, desc)| {
            format!(
                r#"<div class="boundary-card"><p class="boundary-card-title">{}</p><p class="boundary-card-desc">{}</p></div>"#,
                Self::escape_html(title),
                Self::escape_html(&desc)
            )
        })
        .collect::<Vec<_>>()
        .join("")
    }

    fn render_avatar_faq_items(portal: &Portal) -> String {
        let write_answer = match portal.document_access_mode {
            crate::models::mongo::PortalDocumentAccessMode::ReadOnly => {
                "不会。当前是只读模式，只能读取、检索和总结绑定文档。".to_string()
            }
            crate::models::mongo::PortalDocumentAccessMode::CoEditDraft => {
                "不会直接改正式文档。当前只允许在协作草稿范围内写入。".to_string()
            }
            crate::models::mongo::PortalDocumentAccessMode::ControlledWrite => {
                "会。当前允许直接写入目标文档；如果已经有相关 AI 版本，也可以继续在该版本上迭代。"
                    .to_string()
            }
        };
        let doc_answer = if portal.bound_document_ids.is_empty() {
            "不会。当前也没有绑定专属文档，所以我只能基于公开信息和对话上下文回答。".to_string()
        } else {
            format!(
                "不会。我只会读取当前绑定的 {} 份文档，不会访问其他未授权文档。",
                portal.bound_document_ids.len()
            )
        };
        let escalate_answer = "我会先明确告诉你缺少什么能力，再把需求上交给管理 Agent，由管理流程决定是否提权、加技能或转人工。";
        let preview_answer = "访客页是给外部用户正式使用的入口；管理预览只给内部管理员验收页面内容、权限边界和提示文案，不建议直接对外发送。";
        let faq_items = [
            ("你会读取所有团队文档吗？", doc_answer),
            ("你会直接修改正式文档吗？", write_answer),
            ("访客页和管理预览有什么区别？", preview_answer.to_string()),
            (
                "如果当前能力不够，你会怎么处理？",
                escalate_answer.to_string(),
            ),
        ];
        faq_items
            .into_iter()
            .map(|(question, answer)| {
                format!(
                    r#"<div class="faq-item"><p class="faq-q">{}</p><p class="faq-a">{}</p></div>"#,
                    Self::escape_html(question),
                    Self::escape_html(&answer)
                )
            })
            .collect::<Vec<_>>()
            .join("")
    }

    pub fn is_generated_digital_avatar_index_html(html: &str) -> bool {
        if html.contains("AGIME_DIGITAL_AVATAR_DEFAULT_TEMPLATE") {
            return true;
        }
        let markers = [
            "<div id=\"doc-list\">正在加载文档清单...</div>",
            "页面右下角为对话入口。你可以直接提出目标，分身会在能力边界内执行，并在必要时触发管理协作。",
            "管理 Agent 协作机制",
        ];
        markers
            .iter()
            .filter(|marker| html.contains(**marker))
            .count()
            >= 2
    }

    fn render_avatar_view_mode_cards(preview_mode: bool) -> String {
        let items = [
            (
                "访客视角",
                "给客户、合作伙伴或外部用户正式使用，重点看能力说明、边界提示和对话入口。",
                !preview_mode,
            ),
            (
                "管理预览",
                "给内部管理员验收页面内容、权限边界和绑定文档是否正确，不建议直接对外分发。",
                preview_mode,
            ),
        ];
        items
            .into_iter()
            .map(|(title, desc, current)| {
                let badge = if current {
                    r#"<span class="mode-current">当前视角</span>"#
                } else {
                    ""
                };
                format!(
                    r#"<div class="boundary-card"><div class="boundary-card-head"><p class="boundary-card-title">{}</p>{}</div><p class="boundary-card-desc">{}</p></div>"#,
                    Self::escape_html(title),
                    badge,
                    Self::escape_html(desc)
                )
            })
            .collect::<Vec<_>>()
            .join("")
    }

    fn render_digital_avatar_runtime_assets(
        slug: &str,
        welcome_message: &str,
        preview_mode: bool,
        chat_interactive: bool,
    ) -> String {
        let slug_js = serde_json::to_string(slug).unwrap_or_else(|_| "\"\"".to_string());
        let welcome_js =
            serde_json::to_string(welcome_message).unwrap_or_else(|_| "\"你好\"".to_string());
        let sdk_script = if chat_interactive {
            r#"<script src="./portal-sdk.js"></script>"#
        } else {
            ""
        };

        format!(
            r#"{sdk_script}
  <script>
  (function () {{
    var previewMode = {preview_mode};
    var chatInteractive = {chat_interactive};
    var slug = {slug_js};
    var welcomeMessage = {welcome_js};
    var detailsEl = document.getElementById('avatar-details');
    var detailsButtons = Array.prototype.slice.call(document.querySelectorAll('[data-open-details]'));
    detailsButtons.forEach(function (button) {{
      button.addEventListener('click', function () {{
        if (detailsEl) {{
          detailsEl.open = true;
          detailsEl.scrollIntoView({{ behavior: 'smooth', block: 'start' }});
        }}
      }});
    }});

    var docHolders = Array.prototype.slice.call(document.querySelectorAll('[data-doc-list]'));
    function escapeHtml(value) {{
      return String(value == null ? '' : value)
        .replace(/&/g, '&amp;')
        .replace(/</g, '&lt;')
        .replace(/>/g, '&gt;')
        .replace(/"/g, '&quot;')
        .replace(/'/g, '&#39;');
    }}
    function setDocText(text) {{
      docHolders.forEach(function (holder) {{
        holder.textContent = text;
      }});
    }}
    function setDocHtml(html) {{
      docHolders.forEach(function (holder) {{
        holder.innerHTML = html;
      }});
    }}

    if (previewMode) {{
      setDocText('管理预览不拉取实时文档清单，请重点检查结构、边界说明和文案口径。');
    }} else if (docHolders.length > 0) {{
      fetch('./api/docs')
        .then(function (res) {{
          if (!res.ok) throw new Error(String(res.status));
          return res.json();
        }})
        .then(function (data) {{
          var docs = Array.isArray(data.documents) ? data.documents : [];
          if (docs.length === 0) {{
            setDocText('当前未绑定文档。');
            return;
          }}
          var html = docs.map(function (doc) {{
            var name = escapeHtml(doc.name || doc.id || '未命名文档');
            var size = doc.file_size ? ('<span class="doc-item-meta">' + doc.file_size + ' B</span>') : '';
            return '<div class="doc-item"><span class="doc-item-name">' + name + '</span>' + size + '</div>';
          }}).join('');
          setDocHtml(html);
        }})
        .catch(function () {{
          setDocText('文档清单加载失败，请稍后重试。');
        }});
    }}

    if (!chatInteractive) {{
      return;
    }}

    if (!window.PortalSDK) {{
      var runtimeNotice = document.getElementById('avatar-chat-runtime-note');
      if (runtimeNotice) {{
        runtimeNotice.textContent = '聊天运行时加载失败，请刷新页面后重试。';
      }}
      return;
    }}

    var sdk = new window.PortalSDK({{ slug: slug }});
    var feedEl = document.getElementById('avatar-chat-feed');
    var inputEl = document.getElementById('avatar-chat-input');
    var sendBtn = document.getElementById('avatar-chat-send');
    var stopBtn = document.getElementById('avatar-chat-stop');
    var clearBtn = document.getElementById('avatar-chat-clear');
    var runtimeNoteEl = document.getElementById('avatar-chat-runtime-note');
    var statusBox = document.getElementById('avatar-chat-status');
    var statusTitleEl = document.getElementById('avatar-chat-status-title');
    var statusDetailEl = document.getElementById('avatar-chat-status-detail');
    var focusChatButtons = Array.prototype.slice.call(document.querySelectorAll('[data-focus-chat]'));
    var promptButtons = Array.prototype.slice.call(document.querySelectorAll('[data-avatar-prompt]'));
    var busy = false;
    var activeControl = null;
    var pendingUserText = '';
    var liveAssistantText = '';
    var liveErrorText = '';

    function historyItems() {{
      return sdk.chat.getLocalHistory().filter(function (item) {{
        return item && typeof item.content === 'string' && item.content.trim() !== '';
      }});
    }}

    function scrollFeed() {{
      if (feedEl) feedEl.scrollTop = feedEl.scrollHeight;
    }}

    function appendBubble(role, text, meta, live) {{
      if (!feedEl) return;
      var item = document.createElement('div');
      item.className = 'chat-item ' + role + (live ? ' live' : '');
      var metaEl = document.createElement('div');
      metaEl.className = 'chat-meta';
      metaEl.textContent = meta;
      var bubbleEl = document.createElement('div');
      bubbleEl.className = 'chat-bubble';
      bubbleEl.textContent = text;
      item.appendChild(metaEl);
      item.appendChild(bubbleEl);
      feedEl.appendChild(item);
    }}

    function renderConversation() {{
      if (!feedEl) return;
      feedEl.innerHTML = '';
      var items = historyItems();
      if (items.length === 0 && !pendingUserText && !liveAssistantText && !liveErrorText) {{
        appendBubble('assistant', welcomeMessage, '数字分身', false);
        var intro = document.createElement('div');
        intro.className = 'chat-empty-note';
        intro.textContent = '直接告诉我目标、问题、限制条件或你希望得到的结果，我会在当前授权范围内处理。';
        feedEl.appendChild(intro);
      }}

      items.forEach(function (item) {{
        var role = item.role === 'user' ? 'user' : 'assistant';
        appendBubble(role, String(item.content || ''), role === 'user' ? '你' : '数字分身', false);
      }});

      if (pendingUserText) {{
        appendBubble('user', pendingUserText, '你 · 正在发送', true);
      }}
      if (liveAssistantText) {{
        appendBubble('assistant', liveAssistantText, '数字分身 · 正在回复', true);
      }}
      if (liveErrorText) {{
        appendBubble('system', liveErrorText, '系统提示', false);
      }}
      scrollFeed();
    }}

    function setBusy(nextBusy) {{
      busy = nextBusy;
      if (inputEl) inputEl.disabled = nextBusy;
      if (sendBtn) sendBtn.disabled = nextBusy;
      if (stopBtn) stopBtn.disabled = !nextBusy;
      promptButtons.forEach(function (button) {{
        button.disabled = nextBusy;
      }});
    }}

    function setStatus(title, detail) {{
      if (!statusBox || !statusTitleEl || !statusDetailEl) return;
      if (!title) {{
        statusBox.hidden = true;
        statusTitleEl.textContent = '';
        statusDetailEl.textContent = '';
        return;
      }}
      statusBox.hidden = false;
      statusTitleEl.textContent = title;
      statusDetailEl.textContent = detail || '';
    }}

    function normalizeStatus(value) {{
      var raw = String(value || '').toLowerCase();
      if (!raw || raw === 'processing' || raw === 'running') return '正在处理你的请求';
      if (raw.indexOf('calling_model') >= 0 || raw.indexOf('llm') >= 0) return '正在生成回复';
      if (raw.indexOf('running_tool') >= 0 || raw.indexOf('tool') >= 0) return '正在调用工具';
      if (raw.indexOf('compacting_context') >= 0 || raw.indexOf('compaction') >= 0) return '正在整理上下文';
      return '正在继续处理';
    }}

    function autoResize() {{
      if (!inputEl) return;
      inputEl.style.height = '0px';
      inputEl.style.height = Math.min(Math.max(inputEl.scrollHeight, 56), 160) + 'px';
    }}

    function resetLiveState() {{
      pendingUserText = '';
      liveAssistantText = '';
      liveErrorText = '';
      activeControl = null;
    }}

    function focusComposer() {{
      if (!inputEl || inputEl.disabled) return;
      inputEl.focus();
      autoResize();
    }}

    async function handleSend(text) {{
      if (busy) return;
      var content = String(text == null ? (inputEl ? inputEl.value : '') : text).trim();
      if (!content) return;
      pendingUserText = content;
      liveAssistantText = '';
      liveErrorText = '';
      if (inputEl) {{
        inputEl.value = '';
        autoResize();
      }}
      setBusy(true);
      setStatus('已发送，正在连接分身', '请稍候，分身会在当前授权范围内开始处理。');
      renderConversation();

      try {{
        activeControl = await sdk.chat.sendAndStream(content, {{
          onEvent: function (kind, data) {{
            if (kind === 'status') {{
              setStatus(normalizeStatus(data && (data.mapped_status || data.status)), '');
              return;
            }}
            if (kind === 'toolcall') {{
              setStatus('正在调用工具', String((data && data.name) || 'tool'));
              return;
            }}
            if (kind === 'toolresult') {{
              setStatus(
                data && data.success === false ? '工具执行失败' : '工具执行完成',
                String((data && data.name) || 'tool')
              );
              return;
            }}
            if (kind === 'thinking') {{
              setStatus('正在思考', '我会先判断当前能力和资料是否足够。');
            }}
          }},
          onTextDelta: function (delta) {{
            if (pendingUserText) pendingUserText = '';
            liveAssistantText += String(delta || '');
            setStatus('正在生成回复', '');
            renderConversation();
          }},
          onDone: function () {{
            resetLiveState();
            renderConversation();
            setBusy(false);
            setStatus('本轮已完成', '你可以继续追问，或切换到详细说明查看完整边界。');
          }},
          onError: function (err) {{
            var message = err && err.message ? err.message : String(err || '请求失败');
            liveErrorText = '对话中断，请稍后重试。' + (message ? '（' + message + '）' : '');
            pendingUserText = '';
            liveAssistantText = '';
            activeControl = null;
            renderConversation();
            setBusy(false);
            setStatus('对话中断', '你可以直接重试，或先查看详细说明确认当前开放范围。');
          }}
        }});
      }} catch (err) {{
        var message = err && err.message ? err.message : String(err || '请求失败');
        liveErrorText = '发送失败，请稍后重试。' + (message ? '（' + message + '）' : '');
        pendingUserText = '';
        liveAssistantText = '';
        activeControl = null;
        renderConversation();
        setBusy(false);
        setStatus('发送失败', '当前未成功建立对话，请稍后再试。');
      }}
    }}

    if (inputEl) {{
      inputEl.addEventListener('input', autoResize);
      inputEl.addEventListener('keydown', function (event) {{
        if (event.key === 'Enter' && !event.shiftKey) {{
          event.preventDefault();
          handleSend();
        }}
      }});
      autoResize();
    }}
    if (sendBtn) {{
      sendBtn.addEventListener('click', function () {{
        handleSend();
      }});
    }}
    focusChatButtons.forEach(function (button) {{
      button.addEventListener('click', function () {{
        window.setTimeout(function () {{
          focusComposer();
        }}, 220);
      }});
    }});
    if (stopBtn) {{
      stopBtn.addEventListener('click', function () {{
        if (!busy) return;
        setStatus('正在停止当前对话', '请稍候，系统会尝试结束本轮生成。');
        Promise.resolve()
          .then(function () {{
            if (activeControl && typeof activeControl.close === 'function') {{
              activeControl.close();
            }}
            return sdk.chat.cancel(sdk.chat.getLocalSessionId());
          }})
          .catch(function () {{}})
          .finally(function () {{
            if (liveAssistantText) {{
              sdk.chat.appendLocalHistory({{
                role: 'bot',
                content: liveAssistantText,
                ts: Date.now(),
                session_id: sdk.chat.getLocalSessionId()
              }});
            }}
            resetLiveState();
            renderConversation();
            setBusy(false);
            setStatus('当前对话已停止', '你可以修改问题后重新发起。');
          }});
      }});
    }}
    if (clearBtn) {{
      clearBtn.addEventListener('click', function () {{
        sdk.chat.clearLocalHistory();
        sdk.chat.clearLocalSession();
        resetLiveState();
        renderConversation();
        setStatus('', '');
      }});
    }}
    promptButtons.forEach(function (button) {{
      button.addEventListener('click', function () {{
        if (button.disabled) return;
        handleSend(button.getAttribute('data-avatar-prompt') || '');
      }});
    }});

    if (runtimeNoteEl) {{
      runtimeNoteEl.textContent = '当前对话历史保存在本机浏览器中，关闭页面后可继续恢复最近会话。';
    }}

    renderConversation();
    setBusy(false);
  }})();
  </script>"#,
            sdk_script = sdk_script,
            preview_mode = if preview_mode { "true" } else { "false" },
            chat_interactive = if chat_interactive { "true" } else { "false" },
            slug_js = slug_js,
            welcome_js = welcome_js
        )
    }

    #[allow(clippy::too_many_lines)]
    fn default_digital_avatar_index_html(
        portal: &Portal,
        preview_mode: bool,
        effective: Option<&PortalEffectivePublicConfig>,
    ) -> String {
        let title = Self::escape_html(&portal.name);
        let subtitle = Self::escape_html(
            portal
                .description
                .as_deref()
                .unwrap_or("这是一个面向真实业务协作场景的数字分身。"),
        );
        let avatar_type = Self::resolve_avatar_type_label(portal);
        let run_mode = Self::resolve_run_mode_label(portal);
        let manager_mode = Self::portal_setting_str(portal, "managerApprovalMode")
            .or_else(|| Self::portal_governance_config_str(portal, "managerApprovalMode"))
            .unwrap_or_else(|| "manager_decides".to_string());
        let optimize_mode = Self::portal_setting_str(portal, "optimizationMode")
            .or_else(|| Self::portal_governance_config_str(portal, "optimizationMode"))
            .unwrap_or_else(|| "dual_loop".to_string());
        let manager_note = if manager_mode.eq_ignore_ascii_case("manager_decides") {
            "能力不足时，先由管理 Agent 评估是否执行、是否提权、是否转人工审批。"
        } else {
            "能力不足时，会进入治理队列，由管理者审批后执行。"
        };
        let optimize_note = if optimize_mode.eq_ignore_ascii_case("dual_loop") {
            "支持“分身自检 + 管理 Agent 监督”的双环优化。"
        } else {
            "优化策略由管理 Agent 按当前配置执行。"
        };
        let effective_doc_mode = effective
            .map(|config| config.effective_document_access_mode)
            .unwrap_or_else(|| Self::resolve_effective_document_access_mode(portal));
        let effective_extensions = effective
            .map(|config| config.effective_allowed_extensions.as_slice())
            .unwrap_or_else(|| portal.allowed_extensions.as_deref().unwrap_or(&[]));
        let effective_skills = effective
            .map(|config| config.effective_allowed_skill_ids.as_slice())
            .unwrap_or_else(|| portal.allowed_skill_ids.as_deref().unwrap_or(&[]));
        let capability_scope_hint = match effective {
            Some(config) if config.extensions_inherited && config.skills_inherited => {
                "当前未额外收敛，按服务分身已启用的扩展与技能对外开放。"
            }
            Some(_) => "当前已按门户白名单收敛，只开放这里列出的扩展与技能。",
            None => "当前开放能力以门户当前配置为准。",
        };
        let view_mode_items = Self::render_avatar_view_mode_cards(preview_mode);
        let capability_items = Self::render_capability_items(portal);
        let boundary_items = Self::render_avatar_boundary_items(portal);
        let boundary_cards = Self::render_avatar_boundary_cards(portal);
        let example_items = Self::render_avatar_example_prompts(portal);
        let faq_items = Self::render_avatar_faq_items(portal);
        let allowed_extensions =
            Self::render_badge_list(effective_extensions, "当前未开放额外扩展");
        let allowed_skills = Self::render_badge_list(effective_skills, "当前未开放专用技能");
        let docs_count = portal.bound_document_ids.len();
        let chat_enabled = effective
            .map(|config| config.chat_enabled)
            .unwrap_or_else(|| {
                portal.agent_enabled && Self::resolve_service_agent_id(portal).is_some()
            });
        let runtime_chat_interactive = chat_enabled && !preview_mode;
        let prompt_buttons =
            Self::render_avatar_example_prompt_buttons(portal, !runtime_chat_interactive);
        let welcome_message = portal
            .agent_welcome_message
            .as_deref()
            .unwrap_or("你好，请直接告诉我你想解决什么问题。");
        let runtime_note = if preview_mode {
            "当前是管理预览模式，用于验收页面结构、文案和边界说明，不会建立真实访客会话。"
        } else if runtime_chat_interactive {
            "直接描述目标、问题、限制条件或你期望得到的结果即可。按 Enter 发送，Shift+Enter 换行。"
        } else {
            "当前分身尚未启用对外对话能力，请先完成服务分身绑定与运行配置。"
        };
        let chat_panel_title = if preview_mode {
            "对话主面预览"
        } else if runtime_chat_interactive {
            "开始对话"
        } else {
            "当前未开放在线对话"
        };
        let chat_panel_description = if preview_mode {
            "这里展示的是访客页的主对话布局。管理预览下不建立真实访客会话，重点检查主次层级、说明口径和对话入口位置。"
        } else if runtime_chat_interactive {
            "这是一块面向访客的主工作区。对话是主入口，说明信息只作为辅助参考。"
        } else {
            "当前页面先展示分身定位和边界说明，真实对话能力尚未对外启用。"
        };
        let chat_placeholder = if runtime_chat_interactive {
            "例如：先帮我梳理目标、背景、限制条件，以及我应该怎么开始"
        } else if preview_mode {
            "管理预览下不建立真实会话，请重点查看布局和说明"
        } else {
            "当前未开放对外对话入口"
        };
        let send_disabled_attr = if runtime_chat_interactive {
            ""
        } else {
            " disabled"
        };
        let clear_disabled_attr = if runtime_chat_interactive {
            ""
        } else {
            " disabled"
        };
        let chat_disabled_notice = if runtime_chat_interactive {
            String::new()
        } else {
            format!(
                r#"<div class="chat-disabled-note" id="avatar-chat-unavailable">{}</div>"#,
                Self::escape_html(runtime_note)
            )
        };
        let chat_entry_note = if preview_mode {
            "当前页面主区域为对话主面预览，用于内部验收结构、文案和边界说明。"
        } else if runtime_chat_interactive {
            "页面上方就是主对话区。访客进入后应直接从这里发起问题或任务。"
        } else {
            "当前分身尚未启用对外对话能力，所以页面只展示说明信息。"
        };
        let runtime_assets = Self::render_digital_avatar_runtime_assets(
            &portal.slug,
            welcome_message,
            preview_mode,
            runtime_chat_interactive,
        );

        format!(
            r##"<!DOCTYPE html>
<html lang="zh-CN">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>{title}</title>
  <style>
    :root {{
      --bg: #eef3f7;
      --bg-strong: #dfe8f1;
      --card: rgba(255, 255, 255, 0.9);
      --text: #142033;
      --muted: #5c6b80;
      --muted-strong: #415065;
      --line: rgba(148, 163, 184, 0.24);
      --brand: #0f766e;
      --brand-soft: rgba(20, 184, 166, 0.1);
      --warn-soft: #fff2e6;
      --ink: #091221;
      --shadow-soft: 0 24px 60px rgba(15, 23, 42, 0.08);
      --shadow-strong: 0 28px 72px rgba(15, 23, 42, 0.18);
    }}
    * {{ box-sizing: border-box; }}
    html {{ scroll-behavior: smooth; }}
    body {{
      margin: 0;
      min-height: 100vh;
      color: var(--text);
      font-family: "Avenir Next", "PingFang SC", "Hiragino Sans GB", "Microsoft YaHei", "Segoe UI", sans-serif;
      line-height: 1.6;
      background:
        radial-gradient(circle at top left, rgba(20, 184, 166, 0.2) 0%, transparent 28%),
        radial-gradient(circle at 88% 10%, rgba(14, 165, 233, 0.14) 0%, transparent 24%),
        linear-gradient(180deg, #f8fbfd 0%, var(--bg) 46%, var(--bg-strong) 100%);
      position: relative;
      overflow-x: hidden;
    }}
    body::before {{
      content: "";
      position: fixed;
      inset: 0;
      pointer-events: none;
      background-image:
        linear-gradient(rgba(255,255,255,0.18) 1px, transparent 1px),
        linear-gradient(90deg, rgba(255,255,255,0.18) 1px, transparent 1px);
      background-size: 26px 26px;
      mask-image: linear-gradient(180deg, rgba(0,0,0,0.26), transparent 88%);
      opacity: 0.38;
    }}
    @keyframes rise-in {{
      from {{ opacity: 0; transform: translateY(16px); }}
      to {{ opacity: 1; transform: translateY(0); }}
    }}
    @keyframes glow-pulse {{
      0% {{ box-shadow: 0 0 0 0 rgba(20, 184, 166, 0.2); }}
      70% {{ box-shadow: 0 0 0 10px rgba(20, 184, 166, 0); }}
      100% {{ box-shadow: 0 0 0 0 rgba(20, 184, 166, 0); }}
    }}
    .wrap {{
      max-width: 1240px;
      margin: 0 auto;
      padding: 34px 20px 52px;
      position: relative;
      z-index: 1;
    }}
    .hero,
    .chat-stage,
    .summary-rail .card,
    .detail-shell {{
      animation: rise-in 0.55s ease-out both;
    }}
    .hero-story-card,
    .prompt-chip,
    .summary-rail .card {{
      animation: rise-in 0.55s ease-out both;
    }}
    .hero {{
      position: relative;
      overflow: hidden;
      border-radius: 28px;
      padding: 30px;
      color: #f8fafc;
      background:
        linear-gradient(135deg, rgba(9, 18, 33, 0.96) 0%, rgba(15, 23, 42, 0.94) 36%, rgba(17, 94, 89, 0.94) 100%);
      border: 1px solid rgba(148, 163, 184, 0.22);
      box-shadow: var(--shadow-strong);
    }}
    .hero::before,
    .hero::after {{
      content: "";
      position: absolute;
      border-radius: 999px;
      pointer-events: none;
    }}
    .hero::before {{
      width: 320px;
      height: 320px;
      right: -80px;
      top: -120px;
      background: radial-gradient(circle, rgba(45, 212, 191, 0.3) 0%, transparent 68%);
    }}
    .hero::after {{
      width: 280px;
      height: 280px;
      left: -90px;
      bottom: -160px;
      background: radial-gradient(circle, rgba(14, 165, 233, 0.18) 0%, transparent 70%);
    }}
    .hero-grid {{
      display: grid;
      grid-template-columns: minmax(0, 1.65fr) minmax(280px, 0.92fr);
      gap: 22px;
      align-items: start;
      position: relative;
      z-index: 1;
    }}
    .hero-main {{ min-width: 0; }}
    .hero-top {{
      display: flex;
      gap: 14px;
      align-items: flex-start;
      justify-content: space-between;
      flex-wrap: wrap;
    }}
    .hero-copy {{ max-width: 760px; }}
    .hero-kicker {{
      margin: 0 0 12px;
      font-size: 12px;
      font-weight: 800;
      letter-spacing: 0.16em;
      text-transform: uppercase;
      color: #99f6e4;
    }}
    .hero h1 {{
      margin: 0;
      font-size: clamp(32px, 4vw, 46px);
      line-height: 1.05;
      letter-spacing: -0.03em;
    }}
    .hero-lead {{
      margin: 14px 0 0;
      max-width: 720px;
      font-size: 16px;
      color: rgba(226, 232, 240, 0.9);
    }}
    .hero-badges {{
      display: flex;
      flex-wrap: wrap;
      gap: 8px;
      justify-content: flex-end;
    }}
    .hero-badge {{
      display: inline-flex;
      align-items: center;
      gap: 6px;
      min-height: 34px;
      border-radius: 999px;
      padding: 0 12px;
      font-size: 12px;
      font-weight: 700;
      color: #d9fbf6;
      background: rgba(255, 255, 255, 0.08);
      border: 1px solid rgba(255, 255, 255, 0.14);
      backdrop-filter: blur(10px);
    }}
    .hero-story {{
      margin-top: 20px;
      display: grid;
      grid-template-columns: repeat(3, minmax(0, 1fr));
      gap: 12px;
    }}
    .hero-story-card {{
      display: grid;
      grid-template-columns: 42px minmax(0, 1fr);
      gap: 12px;
      align-items: start;
      padding: 14px 15px;
      border-radius: 20px;
      background: rgba(255, 255, 255, 0.08);
      border: 1px solid rgba(255, 255, 255, 0.12);
      backdrop-filter: blur(12px);
      transition: transform 0.22s ease, border-color 0.22s ease, background 0.22s ease;
    }}
    .hero-story-card:hover {{
      transform: translateY(-2px);
      border-color: rgba(153, 246, 228, 0.22);
      background: rgba(255, 255, 255, 0.1);
    }}
    .hero-story-card:nth-child(1),
    .summary-rail .card:nth-child(1),
    .prompt-chip:nth-child(1) {{ animation-delay: 0.04s; }}
    .hero-story-card:nth-child(2),
    .summary-rail .card:nth-child(2),
    .prompt-chip:nth-child(2) {{ animation-delay: 0.1s; }}
    .hero-story-card:nth-child(3),
    .summary-rail .card:nth-child(3),
    .prompt-chip:nth-child(3) {{ animation-delay: 0.16s; }}
    .hero-story-no {{
      display: inline-flex;
      align-items: center;
      justify-content: center;
      width: 42px;
      height: 42px;
      border-radius: 14px;
      background: rgba(153, 246, 228, 0.12);
      border: 1px solid rgba(153, 246, 228, 0.22);
      color: #99f6e4;
      font-size: 13px;
      font-weight: 800;
      letter-spacing: 0.06em;
    }}
    .hero-story-copy strong {{
      display: block;
      font-size: 14px;
      font-weight: 800;
      color: #f8fafc;
    }}
    .hero-story-copy p {{
      margin: 4px 0 0;
      font-size: 13px;
      color: rgba(226, 232, 240, 0.78);
    }}
    .hero-panel {{
      position: relative;
      padding: 18px 18px 20px;
      border-radius: 24px;
      background: linear-gradient(180deg, rgba(255,255,255,0.14) 0%, rgba(255,255,255,0.06) 100%);
      border: 1px solid rgba(255,255,255,0.12);
      backdrop-filter: blur(14px);
      box-shadow: inset 0 1px 0 rgba(255,255,255,0.12);
    }}
    .hero-panel-eyebrow {{
      margin: 0;
      font-size: 12px;
      font-weight: 800;
      letter-spacing: 0.12em;
      text-transform: uppercase;
      color: #99f6e4;
    }}
    .hero-panel h2 {{
      margin: 8px 0 6px;
      font-size: 24px;
      line-height: 1.15;
    }}
    .hero-panel-copy {{
      margin: 0;
      font-size: 14px;
      color: rgba(226, 232, 240, 0.86);
    }}
    .hero-panel-steps {{
      margin-top: 14px;
      display: grid;
      gap: 10px;
    }}
    .hero-panel-step {{
      display: grid;
      grid-template-columns: 30px minmax(0, 1fr);
      gap: 10px;
      align-items: start;
    }}
    .hero-panel-step-no {{
      display: inline-flex;
      align-items: center;
      justify-content: center;
      width: 30px;
      height: 30px;
      border-radius: 999px;
      background: rgba(255,255,255,0.08);
      border: 1px solid rgba(255,255,255,0.14);
      color: #f8fafc;
      font-size: 12px;
      font-weight: 800;
    }}
    .hero-panel-step strong {{
      display: block;
      font-size: 13px;
      font-weight: 800;
      color: #f8fafc;
    }}
    .hero-panel-step p {{
      margin: 2px 0 0;
      font-size: 12px;
      color: rgba(226, 232, 240, 0.74);
    }}
    .hero-panel-note {{
      margin: 14px 0 0;
      padding-top: 14px;
      border-top: 1px solid rgba(255,255,255,0.12);
      font-size: 12px;
      color: rgba(226, 232, 240, 0.72);
    }}
    .hero-actions {{
      margin-top: 18px;
      display: flex;
      flex-wrap: wrap;
      gap: 12px;
    }}
    .hero-action {{
      display: inline-flex;
      align-items: center;
      justify-content: center;
      min-height: 46px;
      padding: 0 18px;
      border-radius: 999px;
      border: 1px solid rgba(255, 255, 255, 0.14);
      text-decoration: none;
      cursor: pointer;
      font-size: 14px;
      font-weight: 800;
      touch-action: manipulation;
      transition: transform 0.18s ease, box-shadow 0.18s ease, background 0.18s ease;
    }}
    .hero-action:hover {{
      transform: translateY(-1px);
      box-shadow: 0 12px 24px rgba(9, 18, 33, 0.22);
    }}
    .hero-action-primary {{
      color: #0b1323;
      background: linear-gradient(135deg, #ffffff 0%, #dffcf8 100%);
    }}
    .hero-action-secondary {{
      color: #eff6ff;
      background: rgba(255, 255, 255, 0.08);
    }}
    .meta {{
      margin-top: 18px;
      display: grid;
      grid-template-columns: repeat(auto-fit, minmax(150px, 1fr));
      gap: 12px;
      position: relative;
      z-index: 1;
    }}
    .meta-item {{
      min-height: 88px;
      padding: 14px 16px;
      border-radius: 20px;
      background: rgba(255,255,255,0.08);
      border: 1px solid rgba(255,255,255,0.12);
      backdrop-filter: blur(10px);
    }}
    .meta-label {{
      font-size: 12px;
      font-weight: 700;
      color: rgba(226, 232, 240, 0.68);
    }}
    .meta-value {{
      margin-top: 6px;
      font-size: 15px;
      font-weight: 800;
      color: #f8fafc;
    }}
    .layout {{
      display: grid;
      grid-template-columns: minmax(0, 1.55fr) minmax(280px, 0.92fr);
      gap: 16px;
      margin-top: 18px;
    }}
    .stack {{
      display: grid;
      gap: 14px;
      min-width: 0;
    }}
    .grid {{
      display: grid;
      grid-template-columns: repeat(auto-fit, minmax(280px, 1fr));
      gap: 14px;
    }}
    .card {{
      border-radius: 24px;
      padding: 18px;
      border: 1px solid var(--line);
      background: var(--card);
      box-shadow: var(--shadow-soft);
      transition: transform 0.2s ease, box-shadow 0.2s ease, border-color 0.2s ease;
    }}
    .summary-rail .card:hover {{
      transform: translateY(-2px);
      box-shadow: 0 28px 48px rgba(15, 23, 42, 0.08);
    }}
    .card h2 {{
      margin: 0 0 8px;
      font-size: 18px;
      letter-spacing: -0.02em;
    }}
    .card p {{
      margin: 6px 0;
      color: var(--muted);
    }}
    ul {{
      margin: 8px 0 0 18px;
      padding: 0;
    }}
    li {{
      margin: 7px 0;
      color: var(--muted);
    }}
    .good {{
      background: linear-gradient(180deg, rgba(236, 254, 255, 0.96) 0%, rgba(243, 255, 253, 0.92) 100%);
      border-color: rgba(20, 184, 166, 0.22);
    }}
    .warn {{
      background: linear-gradient(180deg, rgba(255, 247, 237, 0.96) 0%, rgba(255, 250, 244, 0.94) 100%);
      border-color: rgba(251, 146, 60, 0.22);
    }}
    .section-title {{
      margin: 20px 0 10px;
      font-size: 17px;
      font-weight: 800;
    }}
    .sub-title {{
      margin: 0 0 8px;
      font-size: 14px;
      font-weight: 800;
      color: var(--ink);
    }}
    .tag-row {{
      display: flex;
      flex-wrap: wrap;
      gap: 8px;
      margin-top: 8px;
    }}
    .boundary-grid {{
      display: grid;
      grid-template-columns: repeat(auto-fit, minmax(180px, 1fr));
      gap: 10px;
      margin-top: 12px;
    }}
    .boundary-card {{
      border-radius: 16px;
      border: 1px solid rgba(148, 163, 184, 0.2);
      background: rgba(248, 250, 252, 0.92);
      padding: 13px 14px;
    }}
    .boundary-card-head {{
      display: flex;
      align-items: center;
      justify-content: space-between;
      gap: 8px;
    }}
    .boundary-card-title {{
      margin: 0 0 6px;
      font-size: 13px;
      font-weight: 800;
      color: var(--ink);
    }}
    .boundary-card-desc {{
      margin: 0;
      font-size: 13px;
      color: var(--muted);
    }}
    .mode-current {{
      display: inline-flex;
      align-items: center;
      min-height: 26px;
      padding: 0 10px;
      border-radius: 999px;
      background: rgba(15, 118, 110, 0.1);
      color: var(--brand);
      border: 1px solid rgba(15, 118, 110, 0.16);
      font-size: 11px;
      font-weight: 800;
      white-space: nowrap;
    }}
    .tag-chip {{
      display: inline-flex;
      align-items: center;
      padding: 7px 12px;
      border-radius: 999px;
      background: rgba(20, 184, 166, 0.1);
      border: 1px solid rgba(20, 184, 166, 0.16);
      color: #115e59;
      font-size: 12px;
      font-weight: 700;
    }}
    .empty-badge {{
      display: inline-flex;
      align-items: center;
      padding: 7px 12px;
      border-radius: 999px;
      background: #f8fafc;
      border: 1px dashed rgba(148, 163, 184, 0.4);
      color: var(--muted);
      font-size: 12px;
    }}
    .steps {{
      display: grid;
      gap: 12px;
      margin-top: 8px;
    }}
    .step {{
      display: grid;
      grid-template-columns: 30px minmax(0, 1fr);
      gap: 12px;
      align-items: start;
    }}
    .step-no {{
      width: 30px;
      height: 30px;
      border-radius: 999px;
      background: rgba(20, 184, 166, 0.08);
      color: var(--brand);
      font-size: 12px;
      font-weight: 800;
      display: inline-flex;
      align-items: center;
      justify-content: center;
      border: 1px solid rgba(20, 184, 166, 0.16);
    }}
    .step p {{ margin: 0; }}
    .docs {{
      margin-top: 10px;
      border-top: 1px solid rgba(148, 163, 184, 0.18);
      padding-top: 10px;
      font-size: 14px;
      color: var(--muted);
    }}
    .hint {{
      margin-top: 14px;
      font-size: 13px;
      color: #64748b;
    }}
    .faq-list {{
      display: grid;
      gap: 10px;
      margin-top: 10px;
    }}
    .faq-item {{
      border-radius: 16px;
      border: 1px solid rgba(148, 163, 184, 0.18);
      background: rgba(248, 250, 252, 0.88);
      padding: 14px;
    }}
    .faq-q {{
      margin: 0 0 6px;
      font-size: 14px;
      font-weight: 800;
      color: var(--ink);
    }}
    .faq-a {{
      margin: 0;
      font-size: 13px;
      color: var(--muted);
    }}
    .cta {{
      margin-top: 14px;
      padding: 14px 15px;
      border-radius: 18px;
      background: linear-gradient(135deg, #0b1323 0%, #11334a 100%);
      color: #f8fafc;
      box-shadow: 0 20px 38px rgba(15, 23, 42, 0.16);
    }}
    .cta strong {{
      display: block;
      margin-bottom: 4px;
    }}
    .preview-banner {{
      margin-top: 16px;
      padding: 14px 16px;
      border-radius: 18px;
      border: 1px solid rgba(153, 246, 228, 0.26);
      background: linear-gradient(180deg, rgba(236, 254, 255, 0.92) 0%, rgba(216, 255, 246, 0.84) 100%);
      color: #155e75;
      position: relative;
      z-index: 1;
    }}
    .preview-banner strong {{
      display: block;
      margin-bottom: 4px;
      color: #0f172a;
    }}
    .workspace {{
      display: grid;
      grid-template-columns: minmax(0, 1.72fr) minmax(290px, 0.92fr);
      gap: 18px;
      margin-top: 22px;
      align-items: start;
      scroll-margin-top: 18px;
    }}
    .summary-rail {{
      display: grid;
      gap: 14px;
      min-width: 0;
      position: sticky;
      top: 18px;
    }}
    .summary-rail .card {{
      backdrop-filter: blur(16px);
      background: rgba(255, 255, 255, 0.82);
    }}
    .summary-rail .card:first-child {{
      background: linear-gradient(180deg, rgba(236, 254, 255, 0.92) 0%, rgba(255,255,255,0.88) 100%);
      border-color: rgba(20, 184, 166, 0.18);
    }}
    .summary-rail .card:last-child {{
      background: linear-gradient(180deg, rgba(248, 250, 252, 0.92) 0%, rgba(255,255,255,0.96) 100%);
    }}
    .section-eyebrow {{
      margin: 0 0 8px;
      font-size: 11px;
      font-weight: 800;
      letter-spacing: 0.14em;
      text-transform: uppercase;
      color: var(--brand);
    }}
    .chat-stage {{
      overflow: hidden;
      padding: 0;
      border-radius: 28px;
      background: linear-gradient(180deg, rgba(255,255,255,0.92) 0%, rgba(255,255,255,0.84) 100%);
      border: 1px solid rgba(255,255,255,0.48);
      box-shadow: 0 28px 56px rgba(15, 23, 42, 0.1);
    }}
    .chat-stage-head {{
      position: relative;
      padding: 24px 24px 16px;
      border-bottom: 1px solid rgba(148, 163, 184, 0.18);
      background:
        radial-gradient(circle at top right, rgba(20, 184, 166, 0.14) 0%, transparent 34%),
        linear-gradient(180deg, rgba(20, 184, 166, 0.06) 0%, rgba(255,255,255,0.24) 100%);
    }}
    .chat-stage-head::after {{
      content: "";
      position: absolute;
      inset: 0;
      pointer-events: none;
      background: linear-gradient(90deg, transparent 0%, rgba(255,255,255,0.34) 50%, transparent 100%);
      opacity: 0.5;
      mask-image: linear-gradient(180deg, rgba(0,0,0,0.16), transparent 90%);
    }}
    .chat-stage-head h2 {{
      margin: 0;
      font-size: 28px;
      letter-spacing: -0.03em;
      color: var(--ink);
      position: relative;
      z-index: 1;
    }}
    .chat-stage-head p {{
      position: relative;
      z-index: 1;
    }}
    .chat-stage-brief {{
      position: relative;
      z-index: 1;
      margin-top: 14px;
      display: grid;
      grid-template-columns: repeat(2, minmax(0, 1fr));
      gap: 10px;
    }}
    .chat-stage-brief-card {{
      padding: 12px 14px;
      border-radius: 16px;
      background: rgba(255,255,255,0.72);
      border: 1px solid rgba(148, 163, 184, 0.18);
      box-shadow: inset 0 1px 0 rgba(255,255,255,0.4);
    }}
    .chat-stage-brief-label {{
      display: block;
      margin-bottom: 4px;
      font-size: 11px;
      font-weight: 800;
      letter-spacing: 0.08em;
      text-transform: uppercase;
      color: var(--brand);
    }}
    .chat-stage-brief-card strong {{
      display: block;
      font-size: 14px;
      font-weight: 800;
      color: var(--ink);
      line-height: 1.45;
    }}
    .prompt-row {{
      display: grid;
      grid-template-columns: repeat(2, minmax(0, 1fr));
      gap: 12px;
      padding: 18px 24px 0;
    }}
    .prompt-chip {{
      display: grid;
      gap: 6px;
      align-content: start;
      min-height: 88px;
      width: 100%;
      padding: 14px 16px;
      border-radius: 20px;
      border: 1px solid rgba(148, 163, 184, 0.22);
      background: linear-gradient(180deg, rgba(255,255,255,0.92) 0%, rgba(245, 248, 251, 0.92) 100%);
      color: var(--ink);
      cursor: pointer;
      text-align: left;
      touch-action: manipulation;
      transition: transform 0.18s ease, border-color 0.18s ease, box-shadow 0.18s ease;
    }}
    .prompt-chip:hover {{
      transform: translateY(-2px);
      border-color: rgba(20, 184, 166, 0.26);
      box-shadow: 0 18px 34px rgba(15, 23, 42, 0.08);
    }}
    .prompt-chip-label {{
      font-size: 11px;
      font-weight: 800;
      letter-spacing: 0.1em;
      text-transform: uppercase;
      color: var(--brand);
    }}
    .prompt-chip-text {{
      font-size: 14px;
      line-height: 1.5;
      color: var(--muted-strong);
    }}
    .prompt-chip[disabled] {{
      cursor: not-allowed;
      color: #94a3b8;
      background: #f8fafc;
      box-shadow: none;
      transform: none;
    }}
    .prompt-chip[disabled] .prompt-chip-label,
    .prompt-chip[disabled] .prompt-chip-text {{
      color: #94a3b8;
    }}
    .chat-status {{
      position: relative;
      margin: 18px 24px 0;
      padding: 12px 14px 12px 34px;
      border-radius: 16px;
      background: linear-gradient(180deg, rgba(20, 184, 166, 0.08) 0%, rgba(255,255,255,0.94) 100%);
      border: 1px solid rgba(20, 184, 166, 0.18);
      box-shadow: inset 0 1px 0 rgba(255,255,255,0.4);
    }}
    .chat-status::before {{
      content: "";
      position: absolute;
      top: 15px;
      left: 14px;
      display: block;
      width: 10px;
      height: 10px;
      border-radius: 999px;
      background: #14b8a6;
      animation: glow-pulse 1.8s infinite;
    }}
    .chat-status-title {{
      margin: 0;
      font-size: 14px;
      font-weight: 800;
      color: var(--ink);
    }}
    .chat-status-detail {{
      margin: 4px 0 0;
      font-size: 12px;
      color: var(--muted);
    }}
    .chat-feed {{
      min-height: 460px;
      max-height: 640px;
      overflow-y: auto;
      padding: 20px 24px;
      background: linear-gradient(180deg, rgba(255,255,255,0.98) 0%, rgba(247,250,252,0.98) 100%);
      position: relative;
    }}
    .chat-feed::-webkit-scrollbar {{
      width: 10px;
    }}
    .chat-feed::-webkit-scrollbar-thumb {{
      background: rgba(148, 163, 184, 0.38);
      border-radius: 999px;
      border: 2px solid rgba(255,255,255,0.65);
    }}
    .chat-feed::-webkit-scrollbar-track {{
      background: transparent;
    }}
    .chat-feed::before {{
      content: "";
      position: absolute;
      inset: 0;
      pointer-events: none;
      background-image: radial-gradient(circle at 1px 1px, rgba(148,163,184,0.16) 1px, transparent 0);
      background-size: 18px 18px;
      opacity: 0.34;
      mask-image: linear-gradient(180deg, rgba(0,0,0,0.08), transparent 96%);
    }}
    .chat-item {{
      position: relative;
      z-index: 1;
      display: flex;
      flex-direction: column;
      gap: 6px;
      margin-bottom: 16px;
    }}
    .chat-item.user {{
      align-items: flex-end;
    }}
    .chat-item.live .chat-bubble {{
      box-shadow: 0 0 0 1px rgba(20, 184, 166, 0.08), 0 18px 30px rgba(20, 184, 166, 0.12);
    }}
    .chat-item.system .chat-bubble {{
      background: #fff7ed;
      border-color: #fdba74;
      color: #9a3412;
    }}
    .chat-meta {{
      font-size: 12px;
      font-weight: 700;
      color: #64748b;
    }}
    .chat-bubble {{
      max-width: min(92%, 760px);
      border-radius: 22px;
      padding: 15px 17px;
      border: 1px solid rgba(148, 163, 184, 0.2);
      background: rgba(255, 255, 255, 0.96);
      color: var(--ink);
      white-space: pre-wrap;
      word-break: break-word;
      box-shadow: 0 12px 24px rgba(15, 23, 42, 0.04);
    }}
    .chat-item.user .chat-bubble {{
      background: linear-gradient(135deg, #0f766e 0%, #115e59 100%);
      border-color: rgba(15, 118, 110, 0.4);
      color: #f8fafc;
    }}
    .chat-empty-note {{
      position: relative;
      z-index: 1;
      margin-top: 8px;
      max-width: 520px;
      font-size: 13px;
      color: var(--muted);
    }}
    .chat-composer {{
      border-top: 1px solid rgba(148, 163, 184, 0.18);
      padding: 18px 24px 22px;
      background: linear-gradient(180deg, rgba(255,255,255,0.9) 0%, rgba(249,251,252,0.98) 100%);
    }}
    .chat-composer textarea {{
      width: 100%;
      min-height: 60px;
      max-height: 180px;
      resize: none;
      border-radius: 20px;
      border: 1px solid rgba(148, 163, 184, 0.26);
      padding: 15px 17px;
      font: inherit;
      color: var(--ink);
      outline: none;
      background: rgba(255,255,255,0.96);
      box-shadow: inset 0 1px 0 rgba(255,255,255,0.5);
      transition: border-color 0.18s ease, box-shadow 0.18s ease;
    }}
    .chat-composer textarea:focus {{
      border-color: rgba(20, 184, 166, 0.34);
      box-shadow: 0 0 0 4px rgba(20, 184, 166, 0.08);
    }}
    .chat-composer textarea:disabled {{
      background: #f8fafc;
      color: #94a3b8;
    }}
    .chat-composer-foot {{
      margin-top: 14px;
      display: flex;
      flex-wrap: wrap;
      gap: 12px;
      align-items: center;
      justify-content: space-between;
    }}
    .composer-note {{
      margin: 0;
      font-size: 12px;
      color: var(--muted);
      max-width: 540px;
    }}
    .composer-actions {{
      display: flex;
      flex-wrap: wrap;
      gap: 8px;
    }}
    .action-btn {{
      display: inline-flex;
      align-items: center;
      justify-content: center;
      min-height: 40px;
      padding: 0 15px;
      border-radius: 999px;
      border: 1px solid rgba(148, 163, 184, 0.26);
      background: rgba(255,255,255,0.94);
      color: var(--ink);
      font-size: 13px;
      font-weight: 800;
      cursor: pointer;
      touch-action: manipulation;
      transition: transform 0.18s ease, border-color 0.18s ease, box-shadow 0.18s ease;
    }}
    .action-btn:hover {{
      transform: translateY(-1px);
      border-color: rgba(20, 184, 166, 0.2);
      box-shadow: 0 12px 24px rgba(15, 23, 42, 0.06);
    }}
    .action-btn[disabled] {{
      cursor: not-allowed;
      color: #94a3b8;
      background: #f8fafc;
      box-shadow: none;
      transform: none;
    }}
    .action-btn-primary {{
      border-color: transparent;
      background: linear-gradient(135deg, #0f766e 0%, #134e4a 100%);
      color: #f8fafc;
    }}
    .chat-disabled-note {{
      margin: 0 24px 24px;
      padding: 14px 16px;
      border-radius: 16px;
      border: 1px dashed rgba(148, 163, 184, 0.4);
      background: #f8fafc;
      color: var(--muted);
      font-size: 13px;
    }}
    .metric-grid {{
      display: grid;
      grid-template-columns: repeat(2, minmax(0, 1fr));
      gap: 10px;
      margin-top: 12px;
    }}
    .metric-card {{
      border-radius: 16px;
      border: 1px solid rgba(148, 163, 184, 0.18);
      background: linear-gradient(180deg, rgba(248, 250, 252, 0.9) 0%, rgba(255,255,255,0.92) 100%);
      padding: 12px 13px;
    }}
    .metric-card p {{
      margin: 4px 0 0;
      font-size: 14px;
      font-weight: 700;
      color: var(--ink);
    }}
    .detail-link {{
      margin-top: 10px;
      display: inline-flex;
      align-items: center;
      justify-content: center;
      min-height: 42px;
      padding: 0 16px;
      border-radius: 999px;
      border: 1px solid rgba(15, 118, 110, 0.24);
      background: linear-gradient(135deg, rgba(236, 254, 255, 0.96) 0%, rgba(255,255,255,0.96) 100%);
      color: var(--brand);
      font-size: 13px;
      font-weight: 800;
      cursor: pointer;
      touch-action: manipulation;
      transition: transform 0.18s ease, box-shadow 0.18s ease;
    }}
    .detail-link:hover {{
      transform: translateY(-1px);
      box-shadow: 0 12px 24px rgba(15, 23, 42, 0.06);
    }}
    .detail-shell {{
      margin-top: 20px;
      border-radius: 26px;
      border: 1px solid rgba(148, 163, 184, 0.2);
      background: rgba(255, 255, 255, 0.84);
      box-shadow: var(--shadow-soft);
      overflow: hidden;
      backdrop-filter: blur(16px);
      scroll-margin-top: 18px;
    }}
    .detail-shell > summary {{
      cursor: pointer;
      list-style: none;
      padding: 20px 22px;
      font-size: 17px;
      font-weight: 800;
      color: var(--ink);
      background: rgba(255,255,255,0.92);
      display: flex;
      align-items: center;
      justify-content: space-between;
      gap: 12px;
    }}
    .detail-shell > summary::after {{
      content: "+";
      display: inline-flex;
      align-items: center;
      justify-content: center;
      width: 30px;
      height: 30px;
      border-radius: 999px;
      border: 1px solid rgba(148, 163, 184, 0.2);
      color: var(--brand);
      font-size: 20px;
      font-weight: 500;
      flex-shrink: 0;
    }}
    .detail-shell[open] > summary::after {{
      content: "–";
    }}
    .hero-action:focus-visible,
    .detail-link:focus-visible,
    .prompt-chip:focus-visible,
    .action-btn:focus-visible,
    .detail-shell > summary:focus-visible,
    .chat-composer textarea:focus-visible {{
      outline: 3px solid rgba(20, 184, 166, 0.22);
      outline-offset: 2px;
    }}
    .detail-shell > summary::-webkit-details-marker {{
      display: none;
    }}
    .detail-shell-body {{
      border-top: 1px solid rgba(148, 163, 184, 0.16);
      padding: 0 22px 22px;
    }}
    .doc-item {{
      display: flex;
      justify-content: space-between;
      gap: 12px;
      padding: 10px 0;
      border-bottom: 1px dashed rgba(148, 163, 184, 0.28);
    }}
    .doc-item:last-child {{
      border-bottom: none;
    }}
    .doc-item-name {{
      color: var(--ink);
    }}
    .doc-item-meta {{
      white-space: nowrap;
      color: var(--muted);
      font-size: 12px;
    }}
    @media (max-width: 1080px) {{
      .hero-grid,
      .workspace,
      .layout {{
        grid-template-columns: 1fr;
      }}
      .summary-rail {{
        position: static;
        grid-template-columns: repeat(2, minmax(0, 1fr));
      }}
      .hero-story {{
        grid-template-columns: 1fr;
      }}
      .prompt-row {{
        grid-template-columns: 1fr;
      }}
    }}
    @media (max-width: 760px) {{
      .wrap {{
        padding: 18px 14px 30px;
      }}
      .hero {{
        border-radius: 22px;
        padding: 22px 18px;
      }}
      .hero h1 {{
        font-size: 30px;
      }}
      .meta {{
        grid-template-columns: repeat(2, minmax(0, 1fr));
      }}
      .chat-stage-head {{
        padding: 20px 18px 14px;
      }}
      .chat-stage-brief,
      .metric-grid {{
        grid-template-columns: 1fr;
      }}
      .prompt-row,
      .chat-feed,
      .chat-composer {{
        padding-left: 18px;
        padding-right: 18px;
      }}
      .chat-composer {{
        position: sticky;
        bottom: 0;
        z-index: 4;
        backdrop-filter: blur(14px);
        box-shadow: 0 -12px 24px rgba(15, 23, 42, 0.08);
      }}
      .chat-status,
      .chat-disabled-note {{
        margin-left: 18px;
        margin-right: 18px;
      }}
    }}
    @media (max-width: 640px) {{
      .hero-actions,
      .composer-actions,
      .chat-composer-foot {{
        width: 100%;
      }}
      .hero-action,
      .action-btn,
      .detail-link {{
        width: 100%;
      }}
      .meta {{
        grid-template-columns: 1fr;
      }}
      .summary-rail {{
        grid-template-columns: 1fr;
      }}
    }}
    @media (prefers-reduced-motion: reduce) {{
      *,
      *::before,
      *::after {{
        animation: none !important;
        transition: none !important;
        scroll-behavior: auto !important;
      }}
    }}
  </style>
</head>
<body>
  <!-- AGIME_DIGITAL_AVATAR_DEFAULT_TEMPLATE_V4 -->
  <div class="wrap">
    <section class="hero">
      <div class="hero-grid">
        <div class="hero-main">
          <div class="hero-top">
            <div class="hero-copy">
              <p class="hero-kicker">对话优先的数字分身入口</p>
              <h1>{title}</h1>
              <p class="hero-lead">{subtitle}</p>
            </div>
            <div class="hero-badges">
              <span class="hero-badge">数字分身</span>
              <span class="hero-badge">{avatar_type}</span>
              <span class="hero-badge">由管理 Agent 治理</span>
              {preview_badge}
            </div>
          </div>
          <div class="hero-story">
            <div class="hero-story-card">
              <span class="hero-story-no">01</span>
              <div class="hero-story-copy">
                <strong>先描述目标</strong>
                <p>直接说任务、问题、背景和你希望得到的结果。</p>
              </div>
            </div>
            <div class="hero-story-card">
              <span class="hero-story-no">02</span>
              <div class="hero-story-copy">
                <strong>我先在边界内推进</strong>
                <p>优先利用当前允许的文档、技能和扩展完成处理。</p>
              </div>
            </div>
            <div class="hero-story-card">
              <span class="hero-story-no">03</span>
              <div class="hero-story-copy">
                <strong>说明保留在第二层</strong>
                <p>详细 FAQ、治理机制和边界说明依然完整保留，随时可看。</p>
              </div>
            </div>
          </div>
          <div class="hero-actions">
            <a href="#avatar-chat-stage" class="hero-action hero-action-primary" data-focus-chat>开始对话</a>
            <button type="button" class="hero-action hero-action-secondary" data-open-details>查看详细说明</button>
          </div>
        </div>
        <aside class="hero-panel">
          <p class="hero-panel-eyebrow">使用方式</p>
          <h2>{chat_panel_title}</h2>
          <p class="hero-panel-copy">{chat_panel_description}</p>
          <div class="hero-panel-steps">
            <div class="hero-panel-step">
              <span class="hero-panel-step-no">1</span>
              <div>
                <strong>一句话说清任务</strong>
                <p>目标、限制和上下文说得越清楚，我越容易直接开始。</p>
              </div>
            </div>
            <div class="hero-panel-step">
              <span class="hero-panel-step-no">2</span>
              <div>
                <strong>主对话区优先</strong>
                <p>上方主工作区就是正式入口，不再依赖右下角浮动聊天框。</p>
              </div>
            </div>
            <div class="hero-panel-step">
              <span class="hero-panel-step-no">3</span>
              <div>
                <strong>需要时再看完整说明</strong>
                <p>边界、FAQ、资料范围和治理机制都放在详细说明里，避免首页过重。</p>
              </div>
            </div>
          </div>
          <p class="hero-panel-note">{runtime_note}</p>
        </aside>
      </div>
      <div class="meta">
        <div class="meta-item"><div class="meta-label">分身类型</div><div class="meta-value">{avatar_type}</div></div>
        <div class="meta-item"><div class="meta-label">运行方式</div><div class="meta-value">{run_mode}</div></div>
        <div class="meta-item"><div class="meta-label">文档权限</div><div class="meta-value">{doc_mode}</div></div>
        <div class="meta-item"><div class="meta-label">绑定文档</div><div class="meta-value">{docs_count} 份</div></div>
        <div class="meta-item"><div class="meta-label">允许扩展</div><div class="meta-value">{extensions_count} 项</div></div>
        <div class="meta-item"><div class="meta-label">允许技能</div><div class="meta-value">{skills_count} 项</div></div>
      </div>
      {preview_banner}
    </section>

    <section class="workspace" id="avatar-chat-stage">
      <section class="card chat-stage">
        <div class="chat-stage-head">
          <p class="section-eyebrow">主对话区</p>
          <h2>{chat_panel_title}</h2>
          <p>{chat_panel_description}</p>
          <div class="chat-stage-brief">
            <div class="chat-stage-brief-card">
              <span class="chat-stage-brief-label">建议输入方式</span>
              <strong>目标 + 背景 + 限制条件 + 期望结果</strong>
            </div>
            <div class="chat-stage-brief-card">
              <span class="chat-stage-brief-label">默认处理方式</span>
              <strong>先在当前授权范围内推进，越界部分会明确说明并走治理流程</strong>
            </div>
          </div>
        </div>
        <div class="prompt-row">{prompt_buttons}</div>
        <div class="chat-status" id="avatar-chat-status" hidden>
          <p class="chat-status-title" id="avatar-chat-status-title"></p>
          <p class="chat-status-detail" id="avatar-chat-status-detail"></p>
        </div>
        <div class="chat-feed" id="avatar-chat-feed"></div>
        <div class="chat-composer">
          <textarea id="avatar-chat-input" placeholder="{chat_placeholder}"{send_disabled_attr}></textarea>
          <div class="chat-composer-foot">
            <p class="composer-note" id="avatar-chat-runtime-note">{runtime_note}</p>
            <div class="composer-actions">
              <button type="button" class="action-btn" id="avatar-chat-clear"{clear_disabled_attr}>清空记录</button>
              <button type="button" class="action-btn" id="avatar-chat-stop" disabled>停止生成</button>
              <button type="button" class="action-btn action-btn-primary" id="avatar-chat-send"{send_disabled_attr}>发送</button>
            </div>
          </div>
        </div>
        {chat_disabled_notice}
      </section>

      <aside class="summary-rail">
        <section class="card">
          <p class="section-eyebrow">快速了解</p>
          <h2>先从这些任务开始</h2>
          <ul>{capability_items}</ul>
        </section>

        <section class="card">
          <p class="section-eyebrow">可信范围</p>
          <h2>我会先把边界讲清楚</h2>
          <ul>{boundary_items}</ul>
        </section>

        <section class="card">
          <p class="section-eyebrow">资料与权限</p>
          <h2>当前开放范围</h2>
          <div class="metric-grid">
            <div class="metric-card"><div class="sub-title">文档权限</div><p>{doc_mode}</p></div>
            <div class="metric-card"><div class="sub-title">绑定文档</div><p>{docs_count} 份</p></div>
            <div class="metric-card"><div class="sub-title">允许扩展</div><p>{extensions_count} 项</p></div>
            <div class="metric-card"><div class="sub-title">允许技能</div><p>{skills_count} 项</p></div>
          </div>
          <p class="hint">{capability_scope_hint}</p>
        </section>

        <section class="card">
          <p class="section-eyebrow">完整说明</p>
          <h2>完整资料、FAQ 与治理说明仍然保留</h2>
          <p>首页现在只保留高频信息；更完整的 FAQ、治理机制、开放扩展和文档范围都收在详细说明里。</p>
          <button type="button" class="detail-link" data-open-details>查看详细说明</button>
        </section>
      </aside>
    </section>

    <details id="avatar-details" class="detail-shell">
      <summary>展开完整说明与 FAQ</summary>
      <div class="detail-shell-body">
        <div class="layout">
          <div class="stack">
            <div class="grid">
              <section class="card good">
                <h2>我能做什么</h2>
                <ul>{capability_items}</ul>
              </section>
              <section class="card warn">
                <h2>能力边界（不会越权）</h2>
                <ul>{boundary_items}</ul>
              </section>
            </div>

            <section class="card">
              <h2>你可以这样和我协作</h2>
              <p>如果你是第一次使用，建议直接说目标、问题、期望结果，我会在当前边界内完成处理。</p>
              <ul>{example_items}</ul>
              <div class="boundary-grid">{boundary_cards}</div>
              <div class="cta">
                <strong>对话建议</strong>
                遇到需要新增能力、放开权限或引入新文档时，我会先说明缺口，再进入管理 Agent 治理流程。
              </div>
            </section>

            <section class="card">
              <h2>管理 Agent 协作机制</h2>
              <div class="steps">
                <div class="step">
                  <span class="step-no">1</span>
                  <div>
                    <p class="sub-title">分身先在当前边界内执行</p>
                    <p>优先使用当前允许的文档、扩展与技能完成任务。</p>
                  </div>
                </div>
                <div class="step">
                  <span class="step-no">2</span>
                  <div>
                    <p class="sub-title">能力不足时交由管理 Agent 判定</p>
                    <p>{manager_note}</p>
                  </div>
                </div>
                <div class="step">
                  <span class="step-no">3</span>
                  <div>
                    <p class="sub-title">运行中持续优化</p>
                    <p>{optimize_note}</p>
                  </div>
                </div>
              </div>
            </section>
          </div>

          <div class="stack">
            <section class="card">
              <h2>当前开放能力</h2>
              <p class="hint">{capability_scope_hint}</p>
              <p class="sub-title">允许扩展</p>
              <div class="tag-row">{allowed_extensions}</div>
              <p class="sub-title" style="margin-top: 14px;">允许技能</p>
              <div class="tag-row">{allowed_skills}</div>
            </section>

            <section class="card">
              <h2>已绑定文档</h2>
              <p>以下文档会作为当前分身的可访问知识范围展示给你。</p>
              <div class="docs">
                <div data-doc-list>正在加载文档清单...</div>
              </div>
            </section>

            <section class="card">
              <h2>使用提示</h2>
              <p>{chat_entry_note}</p>
              <p class="hint">如果当前分身无法直接完成，我会明确告诉你缺少什么，并通过管理 Agent 进入治理流程，而不是静默失败。</p>
            </section>

            <section class="card">
              <h2>入口说明</h2>
              <p>同一个数字分身通常会有两个入口：正式给访客使用的访客页，以及内部管理人员验收用的管理预览。</p>
              <div class="boundary-grid">{view_mode_items}</div>
            </section>

            <section class="card">
              <h2>常见问题</h2>
              <div class="faq-list">{faq_items}</div>
            </section>
          </div>
        </div>
      </div>
    </details>
  </div>

  {runtime_assets}
</body>
</html>"##,
            title = title,
            subtitle = subtitle,
            avatar_type = avatar_type,
            run_mode = run_mode,
            doc_mode = Self::resolve_doc_mode_label_for_mode(effective_doc_mode),
            docs_count = docs_count,
            extensions_count = effective_extensions.len(),
            skills_count = effective_skills.len(),
            manager_note = manager_note,
            optimize_note = optimize_note,
            view_mode_items = view_mode_items,
            chat_panel_title = Self::escape_html(chat_panel_title),
            chat_panel_description = Self::escape_html(chat_panel_description),
            prompt_buttons = prompt_buttons,
            chat_placeholder = Self::escape_html(chat_placeholder),
            send_disabled_attr = send_disabled_attr,
            clear_disabled_attr = clear_disabled_attr,
            runtime_note = Self::escape_html(runtime_note),
            chat_disabled_notice = chat_disabled_notice,
            capability_items = capability_items,
            boundary_items = boundary_items,
            boundary_cards = boundary_cards,
            example_items = example_items,
            faq_items = faq_items,
            allowed_extensions = allowed_extensions,
            allowed_skills = allowed_skills,
            chat_entry_note = Self::escape_html(chat_entry_note),
            capability_scope_hint = capability_scope_hint,
            runtime_assets = runtime_assets,
            preview_badge = if preview_mode {
                r#"<span class="hero-badge">管理预览</span>"#
            } else {
                ""
            },
            preview_banner = if preview_mode {
                r#"<div class="preview-banner"><strong>管理预览模式</strong>当前页面用于内部验收：重点核对对话主区、边界说明和详细信息是否按预期展示。请勿直接将该入口发送给外部访客。</div>"#
            } else {
                ""
            },
        )
    }

    pub fn render_digital_avatar_index_html(portal: &Portal) -> String {
        Self::default_digital_avatar_index_html(portal, false, None)
    }

    pub fn render_digital_avatar_preview_html(portal: &Portal) -> String {
        Self::default_digital_avatar_index_html(portal, true, None)
    }

    pub fn render_digital_avatar_index_html_with_effective(
        portal: &Portal,
        effective: &PortalEffectivePublicConfig,
    ) -> String {
        Self::default_digital_avatar_index_html(portal, false, Some(effective))
    }

    pub fn render_digital_avatar_preview_html_with_effective(
        portal: &Portal,
        effective: &PortalEffectivePublicConfig,
    ) -> String {
        Self::default_digital_avatar_index_html(portal, true, Some(effective))
    }

    #[allow(clippy::too_many_lines)]
    fn write_portal_agent_scaffold(project_path: &str, slug: &str) -> Result<()> {
        let client_js = r#"// Portal SDK (generated by server scaffold)
// Unified client for chat, documents, data storage, config, and tracking.

class PortalSDK {
  constructor({ slug }) {
    this.slug = slug;
    this.visitorId = localStorage.getItem('portal_vid');
    if (!this.visitorId) {
      this.visitorId = 'v_' + Array.from(crypto.getRandomValues(new Uint8Array(9)), b => b.toString(36)).join('').substring(0, 12);
      localStorage.setItem('portal_vid', this.visitorId);
    }
  }

  async _fetch(path, opts = {}) {
    const res = await fetch(`/p/${this.slug}${path}`, opts);
    if (!res.ok) throw new Error(`${opts.method || 'GET'} ${path} failed: ${res.status}`);
    return res;
  }

  async _json(path, opts) { return (await this._fetch(path, opts)).json(); }

  // ── Chat ──
  chat = {
    createSession: () => this._json('/api/chat/session', {
      method: 'POST', headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ visitor_id: this.visitorId }),
    }),
    sendMessage: (sessionId, content) => this._json('/api/chat/message', {
      method: 'POST', headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ session_id: sessionId, visitor_id: this.visitorId, content }),
    }),
    subscribe: (sessionId, lastEventId) => {
      const q = new URLSearchParams({ visitor_id: this.visitorId });
      if (lastEventId) q.set('last_event_id', String(lastEventId));
      return new EventSource(`/p/${this.slug}/api/chat/stream/${sessionId}?${q}`);
    },
    cancel: (sessionId) => this._fetch('/api/chat/cancel', {
      method: 'POST', headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ session_id: sessionId, visitor_id: this.visitorId }),
    }),
    listSessions: () => this._json(`/api/chat/sessions?visitor_id=${encodeURIComponent(this.visitorId)}`),
  };

  // ── Documents (read-only, bound documents only) ──
  docs = {
    list: () => this._json('/api/docs'),
    get: (docId) => this._json(`/api/docs/${docId}`),
    getMeta: (docId) => this._json(`/api/docs/${docId}/meta`),
    poll: (docId, intervalMs, callback) => {
      let lastUpdated = null;
      return setInterval(async () => {
        try {
          const meta = await this.docs.getMeta(docId);
          if (lastUpdated && meta.updated_at !== lastUpdated) callback(meta);
          lastUpdated = meta.updated_at;
        } catch (_) {}
      }, intervalMs || 5000);
    },
  };

  // ── Data Storage (key-value in _private/) ──
  data = {
    list: () => this._json('/api/data'),
    get: (key) => this._json(`/api/data/${key}`),
    set: (key, value) => this._fetch(`/api/data/${key}`, {
      method: 'PUT', headers: { 'Content-Type': 'application/json', 'x-visitor-id': this.visitorId },
      body: JSON.stringify(value),
    }),
  };

  // ── Config ──
  config = { get: () => this._json('/api/config') };

  // ── Tracking ──
  track = (type, payload) => this._fetch('/api/interact', {
    method: 'POST', headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ visitorId: this.visitorId, interactionType: type, data: payload || {} }),
  }).catch(() => {});
}

// Usage: const sdk = new PortalSDK({ slug: 'my-portal' });
if (typeof window !== 'undefined') window.PortalSDK = PortalSDK;
if (typeof module !== 'undefined') module.exports = { PortalSDK };
"#;

        let doc_md = format!(
            r#"# Portal SDK

Slug: `{slug}`

## 初始化
```html
<script src="portal-sdk.js"></script>
<script>
const sdk = new PortalSDK({{{{ slug: '{slug}' }}}});
</script>
```

## 架构说明
- 运行时 `portal-sdk.js` 由服务端动态输出（`/p/{slug}/portal-sdk.js`），会随服务端版本更新。
- 聊天后端 Agent 已自动获得绑定文档（bound_documents）上下文，前端无需手动拼接文档内容。
- `_private/` 目录存放服务端 key-value 数据，前端通过 `sdk.data` API 访问，静态文件服务不会暴露此目录。
- 聊天会话与历史默认写入 `localStorage`（按 `slug + visitor_id` 隔离，历史默认保留最近 200 条）。
- 默认悬浮聊天窗口是否注入由 `settings.showChatWidget` 控制（默认 `true`）。

## Chat API

### `sdk.chat.sendAndStream(text, handlers)` → `Promise<{{session_id, close()}}>`
推荐的一体化方法：自动创建/恢复 session、发送消息并建立流。

支持回调：
- `onEvent(kind, data, evt)`
- `onTextDelta(delta, data, evt)`
- `onDone(data, context)`
- `onError(err, context)`

```js
await sdk.chat.sendAndStream('你好', {{
  onEvent(kind, data) {{
    if (kind === 'status') {{
      statusEl.textContent = data.mapped_status || data.status || 'running';
    }}
  }},
  onTextDelta(delta) {{
    appendText(delta);
  }},
  onDone() {{
    markDone();
  }},
  onError(err) {{
    showError(String(err));
  }},
}});
```

### 低阶 Chat 方法
- `sdk.chat.createSession()`
- `sdk.chat.createOrResumeSession()`
- `sdk.chat.sendMessage(sessionId, text)`
- `sdk.chat.subscribe(sessionId, lastEventId?)`
- `sdk.chat.cancel(sessionId)`
- `sdk.chat.listSessions()`

### 本地会话辅助方法
- `sdk.chat.getLocalSessionId()`
- `sdk.chat.clearLocalSession()`
- `sdk.chat.getLocalHistory()`
- `sdk.chat.clearLocalHistory()`
- `sdk.chat.appendLocalHistory(item)`

### SSE 事件（`subscribe` 或 `sendAndStream` 的 `onEvent`）
- `status`
- `toolcall`
- `toolresult`
- `turn`
- `compaction`
- `workspace_changed`
- `text`
- `thinking`
- `done`

## Documents API（只读，仅绑定文档）

### `sdk.docs.list()` → `Promise<{{documents: [{{id, name, mime_type, file_size}}]}}>`
列出所有绑定文档的元数据。

### `sdk.docs.get(docId)` → `Promise<{{text, mime_type, total_size}}>`
获取文档完整文本内容。

### `sdk.docs.getMeta(docId)` → `Promise<{{id, name, mime_type, file_size, updated_at}}>`
获取文档元数据（不含内容）。

### `sdk.docs.poll(docId, intervalMs, callback)` → `intervalId`
轮询文档变更，变更时调用callback。

## Data API（_private/ key-value 存储）

### `sdk.data.list()` → `Promise<{{keys: [string]}}>`
列出所有数据键。

### `sdk.data.get(key)` → `Promise<any>`
读取指定键的值。

### `sdk.data.set(key, value)` → `Promise`
写入键值对。value 为任意 JSON 可序列化对象。

```js
await sdk.data.set('user_prefs', {{theme: 'dark'}});
const prefs = await sdk.data.get('user_prefs');
```

## Config & Tracking

### `sdk.config.get()` → `Promise<{{apiVersion, name, agentEnabled, showChatWidget, documentAccessMode, agentWelcomeMessage, chatApi}}>`
获取 Portal 配置信息。`chatApi` 包含 `sessionPath`、`messagePath`、`streamPathTemplate`。

### `sdk.track(type, payload)` → `void`
记录用户交互事件（页面浏览、按钮点击等）。
"#,
            slug = slug
        );

        let base = std::path::Path::new(project_path);
        std::fs::write(base.join("portal-sdk.js"), client_js)?;
        std::fs::write(base.join("PORTAL_SDK.md"), doc_md)?;
        Ok(())
    }

    /// Initialize portal project folder with starter files and persist project_path.
    pub async fn initialize_project_folder(
        &self,
        team_id: &str,
        portal: &Portal,
        workspace_root: &str,
    ) -> Result<String> {
        let portal_id = portal
            .id
            .as_ref()
            .map(|id| id.to_hex())
            .ok_or_else(|| anyhow!("portal id missing"))?;
        let raw_project_path = Self::compute_project_path(workspace_root, team_id, &portal.slug);
        let base = Path::new(&raw_project_path);
        std::fs::create_dir_all(base)?;
        let project_path = Self::normalize_project_path(base.to_path_buf());
        let base = Path::new(&project_path);
        let index_html = if Self::is_digital_avatar_portal(portal) {
            Self::default_digital_avatar_index_html(portal, false, None)
        } else {
            Self::default_portal_index_html(&portal.name)
        };
        std::fs::write(base.join("index.html"), index_html)?;
        Self::write_portal_agent_scaffold(&project_path, &portal.slug)?;
        self.set_project_path(team_id, &portal_id, &project_path)
            .await?;
        Ok(project_path)
    }

    // -----------------------------------------------------------------------
    // Slug helpers
    // -----------------------------------------------------------------------

    pub async fn generate_slug(&self, name: &str) -> Result<String> {
        let base = slugify(name);
        let base = if base.is_empty() {
            "portal".into()
        } else {
            base
        };

        let coll = self.db.collection::<Portal>(collections::PORTALS);
        // Try base slug first
        if coll
            .find_one(doc! { "slug": &base, "is_deleted": { "$ne": true } }, None)
            .await?
            .is_none()
        {
            return Ok(base);
        }

        // Append random suffix
        for _ in 0..10 {
            let suffix: u32 = rand::random::<u32>() % 10000;
            let candidate = format!("{}-{}", base, suffix);
            if coll
                .find_one(
                    doc! { "slug": &candidate, "is_deleted": { "$ne": true } },
                    None,
                )
                .await?
                .is_none()
            {
                return Ok(candidate);
            }
        }
        Err(anyhow!("Failed to generate unique slug"))
    }

    async fn ensure_slug_available(&self, slug: &str) -> Result<()> {
        let coll = self.db.collection::<Portal>(collections::PORTALS);
        if coll
            .find_one(doc! { "slug": slug, "is_deleted": { "$ne": true } }, None)
            .await?
            .is_some()
        {
            return Err(anyhow!("Slug '{}' is already taken", slug));
        }
        Ok(())
    }

    pub async fn check_slug(&self, slug: &str) -> Result<bool> {
        let coll = self.db.collection::<Portal>(collections::PORTALS);
        Ok(coll
            .find_one(doc! { "slug": slug, "is_deleted": { "$ne": true } }, None)
            .await?
            .is_none())
    }

    // -----------------------------------------------------------------------
    // Interactions
    // -----------------------------------------------------------------------

    pub async fn log_interaction(&self, interaction: PortalInteraction) -> Result<()> {
        // Validate data size
        let data_str = serde_json::to_string(&interaction.data).unwrap_or_default();
        if data_str.len() > MAX_INTERACTION_DATA_SIZE {
            return Err(anyhow!("Interaction data exceeds maximum size of 64KB"));
        }
        if interaction.visitor_id.len() > 64 {
            return Err(anyhow!(
                "Visitor ID exceeds maximum length of 64 characters"
            ));
        }

        let coll = self
            .db
            .collection::<PortalInteraction>(collections::PORTAL_INTERACTIONS);
        coll.insert_one(&interaction, None).await?;
        Ok(())
    }

    pub async fn list_interactions(
        &self,
        team_id: &str,
        portal_id: &str,
        page: u64,
        limit: u64,
    ) -> Result<PaginatedResponse<PortalInteractionResponse>> {
        let team_oid = ObjectId::parse_str(team_id)?;
        let portal_oid = ObjectId::parse_str(portal_id)?;
        let coll = self
            .db
            .collection::<PortalInteraction>(collections::PORTAL_INTERACTIONS);
        let filter = doc! { "portal_id": portal_oid, "team_id": team_oid };

        let total = coll.count_documents(filter.clone(), None).await?;
        let skip = (page.saturating_sub(1)) * limit;
        let opts = FindOptions::builder()
            .sort(doc! { "created_at": -1 })
            .skip(skip)
            .limit(limit as i64)
            .build();

        let cursor = coll.find(filter, opts).await?;
        let raw: Vec<PortalInteraction> = cursor.try_collect().await?;
        let items: Vec<PortalInteractionResponse> = raw.into_iter().map(Into::into).collect();

        Ok(PaginatedResponse::new(items, total, page, limit))
    }

    pub async fn get_stats(&self, team_id: &str, portal_id: &str) -> Result<serde_json::Value> {
        let team_oid = ObjectId::parse_str(team_id)?;
        let portal_oid = ObjectId::parse_str(portal_id)?;
        let coll = self
            .db
            .collection::<PortalInteraction>(collections::PORTAL_INTERACTIONS);

        // Single aggregation with $facet for all stats
        let pipeline = vec![
            doc! { "$match": { "portal_id": portal_oid, "team_id": team_oid } },
            doc! { "$facet": {
                "total": [{ "$count": "count" }],
                "pageViews": [
                    { "$match": { "interaction_type": "page_view" } },
                    { "$count": "count" },
                ],
                "chatMessages": [
                    { "$match": { "interaction_type": "chat_message" } },
                    { "$count": "count" },
                ],
                "formSubmits": [
                    { "$match": { "interaction_type": "form_submit" } },
                    { "$count": "count" },
                ],
                "uniqueVisitors": [
                    { "$group": { "_id": "$visitor_id" } },
                    { "$count": "count" },
                ],
            }},
        ];

        let mut cursor = coll.aggregate(pipeline, None).await?;
        let result = cursor.try_next().await?;

        let (total, page_views, chat_messages, form_submits, unique_visitors) = match result {
            Some(doc) => {
                let get_count = |field: &str| -> u64 {
                    doc.get_array(field)
                        .ok()
                        .and_then(|arr| arr.first())
                        .and_then(|d| d.as_document())
                        .and_then(|d| {
                            d.get_i64("count")
                                .ok()
                                .or_else(|| d.get_i32("count").ok().map(|v| v as i64))
                        })
                        .unwrap_or(0) as u64
                };
                (
                    get_count("total"),
                    get_count("pageViews"),
                    get_count("chatMessages"),
                    get_count("formSubmits"),
                    get_count("uniqueVisitors"),
                )
            }
            None => (0, 0, 0, 0, 0),
        };

        Ok(serde_json::json!({
            "totalInteractions": total,
            "pageViews": page_views,
            "chatMessages": chat_messages,
            "formSubmits": form_submits,
            "uniqueVisitors": unique_visitors,
        }))
    }

    /// Find all non-deleted portals that bind a given document ID.
    /// H-6: `exclude_portal_id` prevents TOCTOU when the caller is the portal being updated.
    pub async fn find_portals_by_document_id(
        &self,
        team_id: &str,
        doc_id: &str,
        exclude_portal_id: Option<&str>,
    ) -> Result<Vec<PortalSummary>> {
        let team_oid = ObjectId::parse_str(team_id)?;
        let coll = self.db.collection::<Portal>(collections::PORTALS);
        let mut filter = doc! {
            "team_id": team_oid,
            "is_deleted": { "$ne": true },
            "bound_document_ids": doc_id,
        };
        if let Some(exc_id) = exclude_portal_id {
            if let Ok(oid) = ObjectId::parse_str(exc_id) {
                filter.insert("_id", doc! { "$ne": oid });
            }
        }
        let cursor = coll.find(filter, None).await?;
        let portals: Vec<Portal> = cursor.try_collect().await?;
        Ok(portals.into_iter().map(PortalSummary::from).collect())
    }

    pub async fn get_document_binding_usage(
        &self,
        team_id: &str,
        doc_ids: &[String],
    ) -> Result<Vec<DocumentBindingUsageSummary>> {
        let mut requested_ids = Vec::new();
        let mut seen = HashSet::new();
        for doc_id in doc_ids {
            let trimmed = doc_id.trim();
            if trimmed.is_empty() || !seen.insert(trimmed.to_string()) {
                continue;
            }
            requested_ids.push(trimmed.to_string());
        }

        if requested_ids.is_empty() {
            return Ok(Vec::new());
        }

        let team_oid = ObjectId::parse_str(team_id)?;
        let requested_set: HashSet<String> = requested_ids.iter().cloned().collect();
        let coll = self.db.collection::<Portal>(collections::PORTALS);
        let filter = doc! {
            "team_id": team_oid,
            "is_deleted": { "$ne": true },
            "bound_document_ids": { "$in": requested_ids.clone() },
        };
        let portals: Vec<Portal> = coll.find(filter, None).await?.try_collect().await?;

        let service_agent_ids: Vec<String> = portals
            .iter()
            .filter_map(Self::resolve_service_agent_id)
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        let agent_names = self
            .load_team_agent_names(team_id, &service_agent_ids)
            .await
            .unwrap_or_default();

        let mut usage_map: HashMap<String, DocumentBindingUsageSummary> = requested_ids
            .iter()
            .cloned()
            .map(|doc_id| {
                (
                    doc_id.clone(),
                    DocumentBindingUsageSummary {
                        doc_id,
                        ..DocumentBindingUsageSummary::default()
                    },
                )
            })
            .collect();

        for portal in portals {
            let portal_id = portal.id.map(|id| id.to_hex()).unwrap_or_default();
            let service_agent_id = Self::resolve_service_agent_id(&portal);
            let service_agent_name = service_agent_id
                .as_ref()
                .and_then(|agent_id| agent_names.get(agent_id))
                .cloned();
            let binding = DocumentBindingPortalRef {
                portal_id,
                portal_name: portal.name.clone(),
                portal_slug: portal.slug.clone(),
                portal_domain: portal.domain,
                manager_agent_id: Self::normalize_agent_id(portal.coding_agent_id.as_deref()),
                service_agent_id,
                service_agent_name,
                document_access_mode: Self::resolve_effective_document_access_mode(&portal),
                portal_status: portal.status,
                public_access_enabled: Self::resolve_public_exposure(&portal)
                    == PortalPublicExposure::PublicPage,
            };

            for doc_id in &portal.bound_document_ids {
                if !requested_set.contains(doc_id) {
                    continue;
                }
                let Some(entry) = usage_map.get_mut(doc_id) else {
                    continue;
                };
                match binding.document_access_mode {
                    PortalDocumentAccessMode::ReadOnly => {
                        entry.read_bindings.push(binding.clone());
                    }
                    PortalDocumentAccessMode::CoEditDraft => {
                        entry.draft_bindings.push(binding.clone());
                    }
                    PortalDocumentAccessMode::ControlledWrite => {
                        entry.write_bindings.push(binding.clone());
                    }
                }
            }
        }

        for entry in usage_map.values_mut() {
            entry
                .read_bindings
                .sort_by(|a, b| a.portal_name.cmp(&b.portal_name));
            entry
                .draft_bindings
                .sort_by(|a, b| a.portal_name.cmp(&b.portal_name));
            entry
                .write_bindings
                .sort_by(|a, b| a.portal_name.cmp(&b.portal_name));
        }

        Ok(requested_ids
            .into_iter()
            .filter_map(|doc_id| usage_map.remove(&doc_id))
            .collect())
    }

    async fn load_team_agent_names(
        &self,
        team_id: &str,
        agent_ids: &[String],
    ) -> Result<HashMap<String, String>> {
        if agent_ids.is_empty() {
            return Ok(HashMap::new());
        }

        let coll = self
            .db
            .collection::<TeamAgentPolicyDoc>(collections::TEAM_AGENTS);
        let cursor = coll
            .find(
                doc! {
                    "team_id": team_id,
                    "agent_id": { "$in": agent_ids }
                },
                None,
            )
            .await?;
        let docs: Vec<TeamAgentPolicyDoc> = cursor.try_collect().await?;
        Ok(docs
            .into_iter()
            .filter_map(|agent| {
                let name = agent.name.unwrap_or_default();
                if name.trim().is_empty() {
                    None
                } else {
                    Some((agent.agent_id, name))
                }
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::mongo::{PortalDocumentAccessMode, PortalOutputForm, PortalStatus};
    use serde_json::json;

    fn make_policy_doc(
        agent_id: &str,
        agent_domain: Option<&str>,
        agent_role: Option<&str>,
        owner_manager_agent_id: Option<&str>,
    ) -> TeamAgentPolicyDoc {
        TeamAgentPolicyDoc {
            agent_id: agent_id.to_string(),
            agent_domain: agent_domain.map(str::to_string),
            agent_role: agent_role.map(str::to_string),
            owner_manager_agent_id: owner_manager_agent_id.map(str::to_string),
            enabled_extensions: Vec::new(),
            custom_extensions: Vec::new(),
            assigned_skills: Vec::new(),
        }
    }

    fn make_portal(
        domain: Option<PortalDomain>,
        tags: &[&str],
        settings: serde_json::Value,
        coding_agent_id: Option<&str>,
        service_agent_id: Option<&str>,
        agent_id: Option<&str>,
    ) -> Portal {
        Portal {
            id: None,
            team_id: ObjectId::new(),
            slug: "demo-portal".to_string(),
            name: "Demo Portal".to_string(),
            description: None,
            status: PortalStatus::Draft,
            output_form: PortalOutputForm::AgentOnly,
            agent_enabled: true,
            coding_agent_id: coding_agent_id.map(str::to_string),
            service_agent_id: service_agent_id.map(str::to_string),
            agent_id: agent_id.map(str::to_string),
            agent_system_prompt: None,
            agent_welcome_message: None,
            bound_document_ids: Vec::new(),
            allowed_extensions: None,
            allowed_skill_ids: None,
            document_access_mode: PortalDocumentAccessMode::ReadOnly,
            domain,
            tags: tags.iter().map(|tag| (*tag).to_string()).collect(),
            settings,
            project_path: None,
            created_by: "tester".to_string(),
            is_deleted: false,
            published_at: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn detect_domain_from_tags_recognizes_avatar_markers() {
        assert_eq!(
            PortalService::detect_domain_from_tags(&["digital-avatar".to_string()]),
            PortalDomain::Avatar
        );
        assert_eq!(
            PortalService::detect_domain_from_tags(&["avatar:internal".to_string()]),
            PortalDomain::Avatar
        );
        assert_eq!(
            PortalService::detect_domain_from_tags(&["domain:avatar".to_string()]),
            PortalDomain::Avatar
        );
        assert_eq!(
            PortalService::detect_domain_from_tags(&["domain:ecosystem".to_string()]),
            PortalDomain::Ecosystem
        );
    }

    #[test]
    fn resolve_portal_domain_keeps_avatar_tags_ahead_of_conflicting_settings() {
        let portal = make_portal(
            None,
            &["digital-avatar", "domain:ecosystem"],
            json!({ "domain": "ecosystem" }),
            None,
            None,
            None,
        );

        assert_eq!(
            PortalService::resolve_portal_domain(&portal),
            PortalDomain::Avatar
        );
    }

    #[test]
    fn normalize_domain_tags_preserves_expected_markers_for_each_domain() {
        let mut avatar_tags = vec![
            "domain:ecosystem".to_string(),
            "existing".to_string(),
            "avatar:external".to_string(),
        ];
        PortalService::normalize_domain_tags(&mut avatar_tags, PortalDomain::Avatar);
        assert!(avatar_tags.iter().any(|tag| tag == "digital-avatar"));
        assert!(avatar_tags.iter().any(|tag| tag == "domain:avatar"));
        assert!(!avatar_tags.iter().any(|tag| tag == "domain:ecosystem"));
        assert!(avatar_tags.iter().any(|tag| tag == "avatar:external"));

        let mut ecosystem_tags = vec![
            "digital-avatar".to_string(),
            "avatar:internal".to_string(),
            "keep-me".to_string(),
        ];
        PortalService::normalize_domain_tags(&mut ecosystem_tags, PortalDomain::Ecosystem);
        assert!(ecosystem_tags.iter().any(|tag| tag == "domain:ecosystem"));
        assert!(!ecosystem_tags.iter().any(|tag| tag == "digital-avatar"));
        assert!(!ecosystem_tags.iter().any(|tag| tag == "avatar:internal"));
        assert!(ecosystem_tags.iter().any(|tag| tag == "keep-me"));
    }

    #[test]
    fn resolve_agent_ids_follow_current_avatar_rules() {
        let avatar = make_portal(
            Some(PortalDomain::Avatar),
            &["digital-avatar"],
            json!({ "domain": "avatar" }),
            Some("manager-1"),
            Some("service-1"),
            Some("service-1"),
        );
        assert_eq!(
            PortalService::resolve_coding_agent_id(&avatar).as_deref(),
            Some("manager-1")
        );
        assert_eq!(
            PortalService::resolve_service_agent_id(&avatar).as_deref(),
            Some("service-1")
        );

        let avatar_same_agent = make_portal(
            Some(PortalDomain::Avatar),
            &["digital-avatar"],
            json!({ "domain": "avatar" }),
            Some("manager-1"),
            Some("manager-1"),
            Some("manager-1"),
        );
        assert_eq!(
            PortalService::resolve_coding_agent_id(&avatar_same_agent).as_deref(),
            Some("manager-1")
        );
        assert_eq!(
            PortalService::resolve_service_agent_id(&avatar_same_agent),
            None
        );
    }

    #[test]
    fn resolve_agent_ids_follow_current_ecosystem_fallback_rules() {
        let ecosystem_service_only = make_portal(
            Some(PortalDomain::Ecosystem),
            &["domain:ecosystem"],
            json!({ "domain": "ecosystem" }),
            None,
            Some("service-1"),
            None,
        );
        assert_eq!(
            PortalService::resolve_coding_agent_id(&ecosystem_service_only).as_deref(),
            Some("service-1")
        );
        assert_eq!(
            PortalService::resolve_service_agent_id(&ecosystem_service_only).as_deref(),
            Some("service-1")
        );

        let ecosystem_legacy_only = make_portal(
            Some(PortalDomain::Ecosystem),
            &["domain:ecosystem"],
            json!({ "domain": "ecosystem" }),
            None,
            None,
            Some("legacy-1"),
        );
        assert_eq!(
            PortalService::resolve_coding_agent_id(&ecosystem_legacy_only).as_deref(),
            Some("legacy-1")
        );
        assert_eq!(
            PortalService::resolve_service_agent_id(&ecosystem_legacy_only).as_deref(),
            Some("legacy-1")
        );
    }

    #[test]
    fn classify_ecosystem_service_agent_binding_matches_current_matrix() {
        assert_eq!(
            PortalService::classify_ecosystem_service_agent_binding(None, None),
            EcosystemServiceAgentBinding::GeneralTemplate
        );
        assert_eq!(
            PortalService::classify_ecosystem_service_agent_binding(Some("general"), Some("")),
            EcosystemServiceAgentBinding::GeneralTemplate
        );
        assert_eq!(
            PortalService::classify_ecosystem_service_agent_binding(
                Some("digital_avatar"),
                Some("service"),
            ),
            EcosystemServiceAgentBinding::AvatarService
        );
        assert_eq!(
            PortalService::classify_ecosystem_service_agent_binding(
                Some("digital_avatar"),
                Some("manager"),
            ),
            EcosystemServiceAgentBinding::AvatarManager
        );
        assert_eq!(
            PortalService::classify_ecosystem_service_agent_binding(
                Some("digital_avatar"),
                Some("runtime"),
            ),
            EcosystemServiceAgentBinding::AvatarOther
        );
        assert_eq!(
            PortalService::classify_ecosystem_service_agent_binding(
                Some("ecosystem_portal"),
                Some("service"),
            ),
            EcosystemServiceAgentBinding::EcosystemService
        );
        assert_eq!(
            PortalService::classify_ecosystem_service_agent_binding(
                Some("ecosystem_portal"),
                Some(""),
            ),
            EcosystemServiceAgentBinding::EcosystemService
        );
        assert_eq!(
            PortalService::classify_ecosystem_service_agent_binding(
                Some("ecosystem_portal"),
                Some("manager"),
            ),
            EcosystemServiceAgentBinding::EcosystemOther
        );
        assert_eq!(
            PortalService::classify_ecosystem_service_agent_binding(Some("unknown"), Some("")),
            EcosystemServiceAgentBinding::Unsupported
        );
    }

    #[test]
    fn classify_avatar_binding_shadow_issues_accepts_valid_avatar_pair() {
        let manager = make_policy_doc("manager-1", Some("digital_avatar"), Some("manager"), None);
        let service = make_policy_doc(
            "service-1",
            Some("digital_avatar"),
            Some("service"),
            Some("manager-1"),
        );

        let issues =
            PortalService::classify_avatar_binding_shadow_issues("manager-1", &manager, &service);

        assert!(issues.is_empty());
    }

    #[test]
    fn classify_avatar_binding_shadow_issues_flags_role_and_owner_mismatch() {
        let manager = make_policy_doc("manager-1", Some("general"), Some("default"), None);
        let service = make_policy_doc(
            "service-1",
            Some("digital_avatar"),
            Some("manager"),
            Some("someone-else"),
        );

        let issues =
            PortalService::classify_avatar_binding_shadow_issues("manager-1", &manager, &service);

        assert_eq!(
            issues,
            vec![
                AvatarBindingShadowIssue::ManagerRoleMismatch,
                AvatarBindingShadowIssue::ServiceRoleMismatch,
                AvatarBindingShadowIssue::OwnerManagerMismatch,
            ]
        );
    }

    #[test]
    fn avatar_binding_shadow_issue_messages_are_human_readable() {
        let manager = make_policy_doc("manager-1", Some("general"), Some("default"), None);
        let service = make_policy_doc(
            "service-1",
            Some("digital_avatar"),
            Some("manager"),
            Some("someone-else"),
        );
        let issues =
            PortalService::classify_avatar_binding_shadow_issues("manager-1", &manager, &service);
        let messages = PortalService::avatar_binding_shadow_issue_messages(
            "manager-1",
            &manager,
            &service,
            &issues,
        );
        assert_eq!(messages.len(), 3);
        assert!(messages[0].contains("digital_avatar:manager"));
        assert!(messages[1].contains("digital_avatar:service"));
        assert!(messages[2].contains("owner_manager_agent_id"));
    }

    #[test]
    fn validate_avatar_binding_policies_accepts_valid_pair_and_rejects_invalid_pair() {
        let valid_manager =
            make_policy_doc("manager-1", Some("digital_avatar"), Some("manager"), None);
        let valid_service = make_policy_doc(
            "service-1",
            Some("digital_avatar"),
            Some("service"),
            Some("manager-1"),
        );
        assert!(PortalService::validate_avatar_binding_policies(
            "manager-1",
            &valid_manager,
            &valid_service
        )
        .is_ok());

        let invalid_manager = make_policy_doc("manager-1", Some("general"), Some("default"), None);
        let invalid_service = make_policy_doc(
            "service-1",
            Some("digital_avatar"),
            Some("manager"),
            Some("someone-else"),
        );
        let error = PortalService::validate_avatar_binding_policies(
            "manager-1",
            &invalid_manager,
            &invalid_service,
        )
        .expect_err("invalid avatar binding should be rejected");
        let message = error.to_string();
        assert!(message.contains("digital_avatar:manager"));
        assert!(message.contains("digital_avatar:service"));
        assert!(message.contains("owner_manager_agent_id"));
    }

    #[test]
    fn resolve_require_human_for_publish_override_prefers_top_level_config() {
        let portal = make_portal(
            Some(PortalDomain::Avatar),
            &["digital-avatar"],
            json!({
                "digitalAvatarGovernanceConfig": {
                    "requireHumanForPublish": false
                },
                "digitalAvatarGovernance": {
                    "config": {
                        "requireHumanForPublish": true
                    }
                }
            }),
            None,
            None,
            None,
        );

        assert_eq!(
            PortalService::portal_governance_config_bool(&portal, "requireHumanForPublish"),
            Some(false)
        );
    }

    #[test]
    fn resolve_require_human_for_publish_override_falls_back_to_legacy_config() {
        let portal = make_portal(
            Some(PortalDomain::Avatar),
            &["digital-avatar"],
            json!({
                "digitalAvatarGovernance": {
                    "config": {
                        "requireHumanForPublish": false
                    }
                }
            }),
            None,
            None,
            None,
        );

        assert_eq!(
            PortalService::portal_governance_config_bool(&portal, "requireHumanForPublish"),
            Some(false)
        );
    }

    #[test]
    fn active_portal_partial_index_filter_matches_false_and_missing_rows() {
        assert_eq!(
            PortalService::active_portal_partial_index_filter(),
            doc! {
                "$or": [
                    { "is_deleted": false },
                    { "is_deleted": Bson::Null }
                ]
            }
        );
    }
}
