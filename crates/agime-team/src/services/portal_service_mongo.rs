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
use chrono::{Datelike, Utc};
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

    fn resolve_skill_display_names(
        skill_ids: &[String],
        service_agent: Option<&TeamAgentPolicyDoc>,
    ) -> Vec<String> {
        let mut by_id = HashMap::new();
        if let Some(agent) = service_agent {
            for skill in &agent.assigned_skills {
                if !skill.enabled {
                    continue;
                }
                by_id.insert(
                    skill.skill_id.trim().to_string(),
                    skill.name.trim().to_string(),
                );
            }
        }
        skill_ids
            .iter()
            .map(|skill_id| {
                let normalized = skill_id.trim();
                by_id
                    .get(normalized)
                    .filter(|name| !name.is_empty())
                    .cloned()
                    .unwrap_or_else(|| normalized.to_string())
            })
            .collect()
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
        let effective_allowed_skill_names =
            Self::resolve_skill_display_names(&effective_allowed_skill_ids, service_agent.as_ref());

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
            effective_allowed_skill_names,
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

    fn avatar_public_narrative_str(portal: &Portal, key: &str) -> Option<String> {
        portal
            .settings
            .get("avatarPublicNarrative")
            .and_then(serde_json::Value::as_object)
            .and_then(|config| config.get(key))
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToString::to_string)
    }

    fn avatar_public_narrative_list(portal: &Portal, key: &str) -> Vec<String> {
        portal
            .settings
            .get("avatarPublicNarrative")
            .and_then(serde_json::Value::as_object)
            .and_then(|config| config.get(key))
            .and_then(serde_json::Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(serde_json::Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    }

    fn resolve_avatar_header_kicker(team_name: Option<&str>) -> String {
        let team_name = team_name
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("");
        if team_name.is_empty() {
            "数字分身".to_string()
        } else {
            format!("{} 的数字分身", team_name)
        }
    }

    fn render_project_header_use_cases(items: &[String]) -> String {
        items
            .iter()
            .take(4)
            .map(|item| {
                format!(
                    r#"<span class="project-header-scene">{}</span>"#,
                    Self::escape_html(item)
                )
            })
            .collect::<Vec<_>>()
            .join("")
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
        items.push("可以围绕你的问题做梳理、问答、总结和结构化输出。".to_string());
        if portal.agent_enabled {
            items.push("支持连续对话，会结合当前会话上下文继续处理。".to_string());
        }
        if !portal.bound_document_ids.is_empty() {
            items.push(format!(
                "可以结合当前开放的 {} 份平台资料来回答问题或处理任务。",
                portal.bound_document_ids.len()
            ));
        } else {
            items.push("当前没有预置平台资料，你也可以先上传自己的资料再继续。".to_string());
        }
        match Self::resolve_effective_document_access_mode(portal) {
            crate::models::mongo::PortalDocumentAccessMode::ReadOnly => {
                items.push("更适合做阅读、梳理、提炼和答疑，不会改动文档内容。".to_string());
            }
            crate::models::mongo::PortalDocumentAccessMode::CoEditDraft => {
                items.push("需要修改文档时，会先生成新版本或协作草稿，再继续迭代。".to_string());
            }
            crate::models::mongo::PortalDocumentAccessMode::ControlledWrite => {
                items.push(
                    "需要修改文档时，可以直接在目标文档上继续处理，并保留版本记录。".to_string(),
                );
            }
        }
        items
            .into_iter()
            .map(|item| format!("<li>{}</li>", item))
            .collect::<Vec<_>>()
            .join("")
    }

    fn render_avatar_boundary_items(portal: &Portal) -> String {
        let mut items = vec![
            "只会使用当前开放的平台资料，以及你自己上传并加入本轮处理范围的资料。".to_string(),
            "如果资料不够、问题不清楚，或当前范围无法继续处理，我会直接告诉你还需要补充什么。".to_string(),
            "如果当前请求超出我的处理范围，我会先交给管理 Agent 判断；管理 Agent 未批准时，再交给人工审核。".to_string(),
        ];
        match Self::resolve_effective_document_access_mode(portal) {
            crate::models::mongo::PortalDocumentAccessMode::ReadOnly => {
                items.push("当前只会读取、检索和总结，不会改写文档内容。".to_string());
            }
            crate::models::mongo::PortalDocumentAccessMode::CoEditDraft => {
                items.push("当前会先生成新版本或协作草稿，不会直接覆盖正式文档。".to_string());
            }
            crate::models::mongo::PortalDocumentAccessMode::ControlledWrite => {
                items.push(
                    "当前可以直接更新目标文档；如果你有多份资料，建议先明确指定本轮要处理哪一份。"
                        .to_string(),
                );
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
                "先帮我梳理这个问题，告诉我建议的处理步骤。",
                "根据当前资料回答这个问题，并把结论讲清楚给我。",
                "我准备上传一份资料，请告诉我接下来最适合怎么处理。",
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
                    r#"<button type="button" class="prompt-chip" data-avatar-prompt="{}"{}>{}</button>"#,
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
            "当前没有预置平台资料。你可以先上传自己的资料，再让我结合资料继续处理。".to_string()
        } else {
            format!(
                "当前可参考 {} 份平台资料；除此之外，我还可以处理你自己上传并选中的资料。",
                portal.bound_document_ids.len()
            )
        };
        let write_policy = match portal.document_access_mode {
            crate::models::mongo::PortalDocumentAccessMode::ReadOnly => {
                "当前只做读取、检索和总结，不会改写文档。".to_string()
            }
            crate::models::mongo::PortalDocumentAccessMode::CoEditDraft => {
                "当前会先生成新版本或协作草稿，不会直接改动正式文档。".to_string()
            }
            crate::models::mongo::PortalDocumentAccessMode::ControlledWrite => {
                "当前支持直接写入目标文档，同时会保留版本记录，便于继续迭代。".to_string()
            }
        };
        let next_step =
            "如果请求超出当前范围，我会先交给管理 Agent 继续判断；若管理 Agent 未批准，再转给人工审核。你也可以先补充资料，或在资料频道切换当前处理目标。";
        [
            ("文档范围", doc_scope),
            ("写入策略", write_policy),
            ("继续方式", next_step.to_string()),
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
                "不会。我只会读取当前开放的 {} 份平台资料，以及你自己上传并加入本轮处理范围的资料。",
                portal.bound_document_ids.len()
            )
        };
        let privacy_answer = "是。你的上传资料会按当前访客身份或登录身份隔离，其他外部用户看不到。";
        let target_answer = "先到资料频道选中文档并设为当前处理目标，再回到对话频道发起请求即可。";
        let login_answer =
            "登录后，你上传的资料和处理进度会归到同一账号，换设备后重新登录也能继续使用。";
        let escalation_answer =
            "我会先明确告诉你当前缺少什么资料、权限或条件；如果仍超出我的处理范围，会先交给管理 Agent 判断，管理 Agent 未批准时再转给人工审核。";
        let faq_items = [
            ("你会读取所有资料吗？", doc_answer),
            ("你会直接修改正式文档吗？", write_answer),
            ("上传的资料只有我自己能看到吗？", privacy_answer.to_string()),
            (
                "如果我上传了多份资料，怎么指定处理哪一份？",
                target_answer.to_string(),
            ),
            (
                "如果你现在做不了，会怎么继续处理？",
                escalation_answer.to_string(),
            ),
            ("登录后有什么变化？", login_answer.to_string()),
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

    function isMobileLayout() {{
      return window.matchMedia
        ? window.matchMedia('(max-width: 760px)').matches
        : window.innerWidth <= 760;
    }}

    function scrollToBlock(el) {{
      if (!el) return;
      var offset = 12;
      if (isMobileLayout()) {{
        var nav = document.querySelector('.channel-nav');
        if (nav) {{
          offset += nav.getBoundingClientRect().height;
        }}
      }}
      var top = el.getBoundingClientRect().top + window.pageYOffset - offset;
      window.scrollTo({{
        top: Math.max(0, top),
        behavior: 'smooth'
      }});
    }}

    var channelButtons = Array.prototype.slice.call(document.querySelectorAll('[data-channel-button]'));
    var channelPanels = Array.prototype.slice.call(document.querySelectorAll('[data-channel-panel]'));
    var openChannelButtons = Array.prototype.slice.call(document.querySelectorAll('[data-open-channel]'));
    var activeChannel = 'chat';
    var docsSourceButtons = Array.prototype.slice.call(document.querySelectorAll('[data-docs-source-button]'));
    var docsSourcePanels = Array.prototype.slice.call(document.querySelectorAll('[data-docs-source-panel]'));
    var activeDocsSource = 'platform';

    function setChannel(key, options) {{
      var next = key || 'chat';
      activeChannel = next;
      channelButtons.forEach(function (button) {{
        button.classList.toggle('is-active', button.getAttribute('data-channel-button') === next);
      }});
      channelPanels.forEach(function (panel) {{
        panel.classList.toggle('is-active', panel.getAttribute('data-channel-panel') === next);
      }});
      if (options && options.scroll) {{
        var nextPanel = null;
        for (var i = 0; i < channelPanels.length; i += 1) {{
          if (channelPanels[i].getAttribute('data-channel-panel') === next) {{
            nextPanel = channelPanels[i];
            break;
          }}
        }}
        if (nextPanel) scrollToBlock(nextPanel);
      }}
    }}

    channelButtons.forEach(function (button) {{
      button.addEventListener('click', function () {{
        setChannel(button.getAttribute('data-channel-button') || 'chat', {{ scroll: isMobileLayout() }});
      }});
    }});

    openChannelButtons.forEach(function (button) {{
      button.addEventListener('click', function () {{
        setChannel(button.getAttribute('data-open-channel') || 'chat', {{ scroll: isMobileLayout() }});
      }});
    }});

    function setDocsSource(key) {{
      var next = key || 'platform';
      activeDocsSource = next;
      docsSourceButtons.forEach(function (button) {{
        button.classList.toggle('is-active', button.getAttribute('data-docs-source-button') === next);
      }});
      docsSourcePanels.forEach(function (panel) {{
        panel.classList.toggle('is-active', panel.getAttribute('data-docs-source-panel') === next);
      }});
    }}

    docsSourceButtons.forEach(function (button) {{
      button.addEventListener('click', function () {{
        setDocsSource(button.getAttribute('data-docs-source-button') || 'platform');
      }});
    }});

    detailsButtons.forEach(function (button) {{
      button.addEventListener('click', function () {{
        setChannel('guide', {{ scroll: isMobileLayout() }});
        if (detailsEl) {{
          detailsEl.open = true;
          scrollToBlock(detailsEl);
        }}
      }});
    }});

    setDocsSource('platform');

    if (previewMode) {{
      setDocText('管理预览不拉取实时文档清单，请重点检查结构、边界说明和文案口径。');
      setPlatformDocListText('管理预览不拉取实时平台资料，请重点检查资料频道的结构、说明和交互。');
    }} else if (docHolders.length > 0) {{
      fetch('./api/docs')
        .then(function (res) {{
          if (!res.ok) throw new Error(String(res.status));
          return res.json();
        }})
        .then(function (data) {{
          var docs = Array.isArray(data.documents) ? data.documents : [];
          platformDocs = docs;
          renderPlatformDocs();
          bindPlatformDocEvents();
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
          setPlatformDocListText('平台资料加载失败，请稍后重试。');
        }});
    }}

    if (!chatInteractive) {{
      var previewUploadList = document.getElementById('avatar-user-doc-list');
      var previewUploadHeading = document.getElementById('avatar-user-doc-heading');
      var previewUploadCopy = document.getElementById('avatar-user-doc-copy');
      var previewUploadTitle = document.getElementById('avatar-user-doc-preview-title');
      var previewUploadMeta = document.getElementById('avatar-user-doc-preview-meta');
      var previewUploadBody = document.getElementById('avatar-user-doc-preview-body');
      var previewAuthHeading = document.getElementById('avatar-auth-heading');
      var previewAuthCopy = document.getElementById('avatar-auth-copy');
      var previewAuthSession = document.getElementById('avatar-auth-session');
      var previewAuthSwitch = document.getElementById('avatar-auth-switch');
      var previewAuthForm = document.getElementById('avatar-auth-form');
      var previewAuthStatus = document.getElementById('avatar-auth-status');
      var disabledUploadMessage = previewMode
        ? '管理预览不连接真实匿名上传区，请重点检查上传入口、文档列表和当前处理目标的布局位置。'
        : '当前分身未启用对外对话能力，所以匿名上传区暂不开放。';
      if (previewUploadList) {{
        previewUploadList.innerHTML = '<p class="user-doc-empty">' + escapeHtml(disabledUploadMessage) + '</p>';
      }}
      if (previewUploadHeading) previewUploadHeading.textContent = previewMode ? '预览模式不连接真实身份' : '当前未开放访客资料区';
      if (previewUploadCopy) previewUploadCopy.textContent = previewMode
        ? '管理预览不会连接匿名访客或外部用户身份，请重点检查身份卡片、上传列表和文档预览的布局位置。'
        : '只有启用对外对话能力后，访客资料上传和外部用户登录才会开放。';
      if (previewUploadTitle) previewUploadTitle.textContent = previewMode ? '预览模式不读取真实资料内容' : '当前未开放访客资料区';
      if (previewUploadMeta) previewUploadMeta.textContent = previewMode ? '上线后这里会显示平台资料、匿名上传资料以及相关 AI 草稿的预览。' : '只有启用对话能力后，访客上传资料才会自动加入本轮会话；平台资料仍按当前开放范围展示。';
      if (previewUploadBody) previewUploadBody.textContent = previewMode ? '当前为管理预览，未连接真实 visitor 数据。' : '如需开放匿名资料上传，请先启用该分身的对外对话能力。';
      if (previewAuthHeading) previewAuthHeading.textContent = previewMode ? '预览模式不连接真实身份' : '当前未开放访客身份接入';
      if (previewAuthCopy) previewAuthCopy.textContent = previewMode
        ? '管理预览只用于检查登录/注册入口和身份说明的展示位置，不会连接真实外部用户。'
        : '只有启用对外对话能力后，外部用户注册、登录和跨设备身份同步才会开放。';
      if (previewAuthSession) previewAuthSession.hidden = true;
      if (previewAuthSwitch) previewAuthSwitch.hidden = true;
      if (previewAuthForm) previewAuthForm.hidden = true;
      if (previewAuthStatus) previewAuthStatus.textContent = previewMode
        ? '上线后这里会显示匿名访客与外部用户登录入口。'
        : '当前分身尚未启用对外对话能力，登录入口暂不开放。';
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
    var headerShellEl = document.getElementById('avatar-header-shell');
    var headerSummaryActionEl = headerShellEl ? headerShellEl.querySelector('.project-header-summary-action') : null;
    var feedEl = document.getElementById('avatar-chat-feed');
    var inputEl = document.getElementById('avatar-chat-input');
    var sendBtn = document.getElementById('avatar-chat-send');
    var stopBtn = document.getElementById('avatar-chat-stop');
    var clearBtn = document.getElementById('avatar-chat-clear');
    var voiceBtnEl = document.getElementById('avatar-chat-voice');
    var runtimeNoteEl = document.getElementById('avatar-chat-runtime-note');
    var statusBox = document.getElementById('avatar-chat-status');
    var statusTitleEl = document.getElementById('avatar-chat-status-title');
    var statusDetailEl = document.getElementById('avatar-chat-status-detail');
    var focusChatButtons = Array.prototype.slice.call(document.querySelectorAll('[data-focus-chat]'));
    var promptButtons = Array.prototype.slice.call(document.querySelectorAll('[data-avatar-prompt]'));
    var promptRowEl = document.getElementById('avatar-chat-prompts');
    var uploadInputEl = document.getElementById('avatar-user-doc-input');
    var uploadTriggerEl = document.getElementById('avatar-user-doc-trigger');
    var uploadRefreshEl = document.getElementById('avatar-user-doc-refresh');
    var platformDocListEl = document.getElementById('avatar-platform-doc-list');
    var userDocListEl = document.getElementById('avatar-user-doc-list');
    var userDocHeadingEl = document.getElementById('avatar-user-doc-heading');
    var userDocCopyEl = document.getElementById('avatar-user-doc-copy');
    var docsPreviewCardEl = document.getElementById('avatar-doc-preview-card');
    var userDocPreviewTitleEl = document.getElementById('avatar-user-doc-preview-title');
    var userDocPreviewMetaEl = document.getElementById('avatar-user-doc-preview-meta');
    var userDocPreviewBodyEl = document.getElementById('avatar-user-doc-preview-body');
    var previewChatJumpEl = document.getElementById('avatar-preview-chat-jump');
    var authHeadingEl = document.getElementById('avatar-auth-heading');
    var authCopyEl = document.getElementById('avatar-auth-copy');
    var authSessionEl = document.getElementById('avatar-auth-session');
    var authBadgeEl = document.getElementById('avatar-auth-badge');
    var authNameEl = document.getElementById('avatar-auth-name');
    var authMetaEl = document.getElementById('avatar-auth-meta');
    var authLogoutEl = document.getElementById('avatar-auth-logout');
    var authSwitchEl = document.getElementById('avatar-auth-switch');
    var authModeButtons = Array.prototype.slice.call(document.querySelectorAll('[data-auth-mode]'));
    var authFormEl = document.getElementById('avatar-auth-form');
    var authUsernameEl = document.getElementById('avatar-auth-username');
    var authDisplayNameFieldEl = document.getElementById('avatar-auth-display-name-field');
    var authDisplayNameEl = document.getElementById('avatar-auth-display-name');
    var authPhoneFieldEl = document.getElementById('avatar-auth-phone-field');
    var authPhoneEl = document.getElementById('avatar-auth-phone');
    var authPasswordEl = document.getElementById('avatar-auth-password');
    var authSubmitEl = document.getElementById('avatar-auth-submit');
    var authStatusEl = document.getElementById('avatar-auth-status');
    var targetBoxEl = document.getElementById('avatar-chat-target');
    var targetNameEl = document.getElementById('avatar-chat-target-name');
    var targetMetaEl = document.getElementById('avatar-chat-target-meta');
    var targetClearEl = document.getElementById('avatar-chat-target-clear');
    var busy = false;
    var authBusy = false;
    var authMode = 'login';
    var authUser = null;
    var activeControl = null;
    var pendingUserText = '';
    var liveAssistantText = '';
    var liveErrorText = '';
    var selectedTargetDoc = null;
    var platformDocs = [];
    var userDocFamilies = [];
    var SpeechRecognitionCtor = window.SpeechRecognition || window.webkitSpeechRecognition || null;
    var speechRecognition = null;
    var speechSupported = !!SpeechRecognitionCtor;
    var speechListening = false;
    var speechBaseValue = '';

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
      var showQuickPrompts = items.length === 0 && !pendingUserText && !liveAssistantText && !liveErrorText;
      if (promptRowEl) {{
        promptRowEl.hidden = !showQuickPrompts || promptButtons.length === 0;
      }}
      if (showQuickPrompts) {{
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

    function formatTime(raw) {{
      if (!raw) return '刚刚更新';
      var date = new Date(raw);
      if (Number.isNaN(date.getTime())) return '刚刚更新';
      return date.toLocaleString('zh-CN', {{
        month: '2-digit',
        day: '2-digit',
        hour: '2-digit',
        minute: '2-digit'
      }});
    }}

    function setUserDocListHtml(html) {{
      if (userDocListEl) userDocListEl.innerHTML = html;
    }}

    function setUserDocListText(text) {{
      if (!userDocListEl) return;
      userDocListEl.innerHTML = '<p class="user-doc-empty">' + escapeHtml(text) + '</p>';
    }}

    function setPlatformDocListText(text) {{
      if (!platformDocListEl) return;
      platformDocListEl.innerHTML = '<p class="user-doc-empty">' + escapeHtml(text) + '</p>';
    }}

    function setPreviewState(title, meta, body) {{
      if (userDocPreviewTitleEl) userDocPreviewTitleEl.textContent = title || '先从上传列表选择文档';
      if (userDocPreviewMetaEl) userDocPreviewMetaEl.textContent = meta || '支持查看平台资料、你上传的原始文档，以及这些原文衍生出来的 AI 工作草稿。';
      if (userDocPreviewBodyEl) userDocPreviewBodyEl.textContent = body || '尚未选择文档。';
    }}

    function scrollDocsPreviewIntoView() {{
      if (!isMobileLayout()) return;
      var target = docsPreviewCardEl || userDocPreviewBodyEl;
      if (!target) return;
      window.setTimeout(function () {{
        scrollToBlock(target);
      }}, 120);
    }}

    function syncHeaderShellLabel() {{
      if (!headerSummaryActionEl || !headerShellEl) return;
      headerSummaryActionEl.textContent = headerShellEl.open ? '收起说明' : '展开说明';
    }}

    function mergeComposerValue(base, addition) {{
      var left = String(base || '').trim();
      var right = String(addition || '').trim();
      if (!left) return right;
      if (!right) return left;
      return left + '\n' + right;
    }}

    function authDisplayName(user) {{
      if (!user) return '外部用户';
      return user.display_name || user.displayName || user.username || user.id || '外部用户';
    }}

    function updateRuntimeNote() {{
      if (!runtimeNoteEl) return;
      if (authUser) {{
        runtimeNoteEl.textContent = '当前已登录为 ' + authDisplayName(authUser) + '。你的上传资料会归属到该外部用户身份；页面里的本地聊天展示仍以当前浏览器为主。';
        return;
      }}
      runtimeNoteEl.textContent = '当前对话历史与匿名上传资料都保存在本机浏览器身份下；更换设备或清空浏览器数据后不会自动同步。';
    }}

    function updateUserDocCopy() {{
      if (!userDocHeadingEl || !userDocCopyEl) return;
      if (previewMode) {{
        userDocHeadingEl.textContent = '预览模式不连接真实身份';
        userDocCopyEl.textContent = '管理预览不会连接匿名访客或外部用户身份，请重点检查结构、说明和上传入口的展示位置。';
        return;
      }}
      if (authUser) {{
        userDocHeadingEl.textContent = '已登录，可跨设备继续使用';
        userDocCopyEl.textContent = '你上传的资料会进入独立的“用户上传文档”区域，并归属到当前外部用户身份。同一账号在其他设备登录后，也能继续查看和处理这些资料。';
        return;
      }}
      userDocHeadingEl.textContent = '仅当前匿名访客可见';
      userDocCopyEl.textContent = '上传资料会进入独立的“用户上传文档”区域，不会落到团队常规文档目录。换浏览器或换设备后不会自动同步。';
    }}

    function setAuthStatus(text) {{
      if (authStatusEl) authStatusEl.textContent = text || '';
    }}

    function setAuthBusy(nextBusy) {{
      authBusy = nextBusy;
      if (authUsernameEl) authUsernameEl.disabled = nextBusy;
      if (authDisplayNameEl) authDisplayNameEl.disabled = nextBusy;
      if (authPhoneEl) authPhoneEl.disabled = nextBusy;
      if (authPasswordEl) authPasswordEl.disabled = nextBusy;
      if (authSubmitEl) {{
        if (nextBusy) {{
          authSubmitEl.disabled = true;
          authSubmitEl.textContent = authMode === 'register' ? '注册中…' : '登录中…';
        }} else {{
          authSubmitEl.disabled = false;
          authSubmitEl.textContent = authMode === 'register' ? '注册并继续' : '登录并继续';
        }}
      }}
      authModeButtons.forEach(function (button) {{
        button.disabled = nextBusy || !!authUser;
      }});
      if (authLogoutEl) authLogoutEl.disabled = nextBusy;
    }}

    function setAuthMode(mode) {{
      authMode = mode === 'register' ? 'register' : 'login';
      authModeButtons.forEach(function (button) {{
        var isActive = button.getAttribute('data-auth-mode') === authMode;
        button.classList.toggle('is-active', isActive);
      }});
      if (authDisplayNameFieldEl) authDisplayNameFieldEl.hidden = authMode !== 'register';
      if (authPhoneFieldEl) authPhoneFieldEl.hidden = authMode !== 'register';
      if (authPasswordEl) authPasswordEl.autocomplete = authMode === 'register' ? 'new-password' : 'current-password';
      if (authSubmitEl && !authBusy) {{
        authSubmitEl.textContent = authMode === 'register' ? '注册并继续' : '登录并继续';
      }}
      if (!authUser) {{
        setAuthStatus(
          authMode === 'register'
            ? '注册后会自动登录，并把当前匿名访客资料迁移到你的外部用户身份。'
            : '登录后，你上传的资料会归属到当前外部用户身份。'
        );
      }}
    }}

    function renderAuthState() {{
      if (!authHeadingEl || !authCopyEl) return;
      if (previewMode) {{
        authHeadingEl.textContent = '预览模式不连接真实身份';
        authCopyEl.textContent = '管理预览只用于检查页面结构和交互位置，不会连接真实匿名访客或外部用户。';
        if (authSessionEl) authSessionEl.hidden = true;
        if (authSwitchEl) authSwitchEl.hidden = true;
        if (authFormEl) authFormEl.hidden = true;
        setAuthStatus('上线后这里会显示匿名访客与外部用户登录入口。');
        updateUserDocCopy();
        updateRuntimeNote();
        return;
      }}

      if (authUser) {{
        if (authSessionEl) authSessionEl.hidden = false;
        if (authSwitchEl) authSwitchEl.hidden = true;
        if (authFormEl) authFormEl.hidden = true;
        if (authBadgeEl) authBadgeEl.textContent = '已登录';
        if (authNameEl) authNameEl.textContent = authDisplayName(authUser);
        if (authMetaEl) {{
          authMetaEl.textContent = '用户名 ' + (authUser.username || authUser.id || 'external-user') + '。你上传的资料和后续会话会归属到同一身份。';
        }}
        authHeadingEl.textContent = '当前已登录，可继续使用同一身份';
        authCopyEl.textContent = '你现在的上传资料会归到外部用户身份；如果之前在当前浏览器里匿名上传过资料，登录后会自动迁移到该账号。';
        setAuthStatus('已登录。你可以在其他设备登录同一账号，继续查看和处理已上传资料。');
      }} else {{
        if (authSessionEl) authSessionEl.hidden = true;
        if (authSwitchEl) authSwitchEl.hidden = false;
        if (authFormEl) authFormEl.hidden = false;
        authHeadingEl.textContent = '当前以匿名访客模式使用';
        authCopyEl.textContent = '你现在可以直接对话和上传资料。登录后，当前浏览器里的匿名资料会迁移到你的外部用户身份，便于在其他设备继续使用。';
        if (!authBusy) {{
          setAuthStatus(
            authMode === 'register'
              ? '注册后会自动登录，并把当前匿名访客资料迁移到你的外部用户身份。'
              : '登录后，你上传的资料会归属到当前外部用户身份。'
          );
        }}
      }}
      updateUserDocCopy();
      updateRuntimeNote();
    }}

    function resolveSelectedDocById(docId) {{
      if (!docId) return null;
      for (var p = 0; p < platformDocs.length; p += 1) {{
        if (platformDocs[p] && platformDocs[p].id === docId) {{
          return {{
            id: platformDocs[p].id,
            name: platformDocs[p].name || platformDocs[p].id,
            kind: '平台资料',
            sourceDocumentIds: []
          }};
        }}
      }}
      for (var i = 0; i < userDocFamilies.length; i += 1) {{
        var source = userDocFamilies[i];
        if (source && source.id === docId) {{
          return {{
            id: source.id,
            name: source.name || source.id,
            kind: '上传原文',
            sourceDocumentIds: []
          }};
        }}
        var related = Array.isArray(source && source.related_ai_documents) ? source.related_ai_documents : [];
        for (var j = 0; j < related.length; j += 1) {{
          if (related[j] && related[j].id === docId) {{
            return {{
              id: related[j].id,
              name: related[j].name || related[j].id,
              kind: 'AI版本',
              sourceDocumentIds: Array.isArray(related[j].source_document_ids) ? related[j].source_document_ids : []
            }};
          }}
        }}
      }}
      return null;
    }}

    function renderSelectedTarget() {{
      if (!targetBoxEl || !targetNameEl || !targetMetaEl) return;
      if (!selectedTargetDoc) {{
        targetBoxEl.hidden = true;
        targetNameEl.textContent = '未指定';
        targetMetaEl.textContent = '当你上传多份资料时，可以先指定本轮要处理的文档。';
        if (targetClearEl) targetClearEl.disabled = true;
        return;
      }}
      targetBoxEl.hidden = false;
      targetNameEl.textContent = selectedTargetDoc.name;
      var parts = [selectedTargetDoc.kind];
      if (selectedTargetDoc.sourceDocumentIds && selectedTargetDoc.sourceDocumentIds.length > 0) {{
        parts.push('来源原文 ' + selectedTargetDoc.sourceDocumentIds.join(', '));
      }}
      targetMetaEl.textContent = parts.join(' · ');
      if (targetClearEl) targetClearEl.disabled = busy;
    }}

    function setSelectedTarget(doc) {{
      selectedTargetDoc = doc;
      renderSelectedTarget();
      renderPlatformDocs();
      bindPlatformDocEvents();
      renderUserDocFamilies();
      bindUserDocEvents();
    }}

    function renderPlatformDocs() {{
      if (!platformDocListEl) return;
      if (!platformDocs.length) {{
        setPlatformDocListText('当前还没有开放的平台资料。');
        return;
      }}
      var html = platformDocs.map(function (doc) {{
        var active = selectedTargetDoc && selectedTargetDoc.id === doc.id;
        var metaParts = ['平台资料'];
        if (doc.mime_type) metaParts.push(doc.mime_type);
        if (doc.file_size) metaParts.push(doc.file_size + ' B');
        return '<div class="user-doc-entry' + (active ? ' is-active' : '') + '">' +
          '<div class="user-doc-head">' +
            '<div class="user-doc-main">' +
              '<button type="button" class="user-doc-open" data-open-platform-doc="' + escapeHtml(doc.id) + '">' +
                escapeHtml(doc.name || doc.id || '未命名资料') +
              '</button>' +
              '<div class="user-doc-meta">' + escapeHtml(metaParts.join(' · ')) + '</div>' +
            '</div>' +
            '<div class="user-doc-actions">' +
              '<button type="button" class="mini-btn primary" data-target-platform-doc="' + escapeHtml(doc.id) + '">设为目标</button>' +
            '</div>' +
          '</div>' +
        '</div>';
      }}).join('');
      platformDocListEl.innerHTML = html;
    }}

    function bindPlatformDocEvents() {{
      if (!platformDocListEl) return;
      Array.prototype.slice.call(platformDocListEl.querySelectorAll('[data-open-platform-doc]')).forEach(function (button) {{
        button.addEventListener('click', function () {{
          openPlatformDoc(button.getAttribute('data-open-platform-doc') || '');
        }});
      }});
      Array.prototype.slice.call(platformDocListEl.querySelectorAll('[data-target-platform-doc]')).forEach(function (button) {{
        button.addEventListener('click', function () {{
          var doc = resolveSelectedDocById(button.getAttribute('data-target-platform-doc') || '');
          if (!doc) return;
          setSelectedTarget(doc);
          setChannel('chat', {{ scroll: isMobileLayout() }});
          window.setTimeout(function () {{
            focusComposer();
          }}, 120);
        }});
      }});
    }}

    function renderUserDocFamilies() {{
      if (!userDocListEl) return;
      if (!userDocFamilies.length) {{
        setUserDocListText(
          authUser
            ? '当前账号还没有上传资料。你在其他设备登录同一账号后上传的资料，也会显示在这里。'
            : '当前还没有上传资料。上传后仅当前浏览器里的匿名访客可见。'
        );
        return;
      }}
      var html = userDocFamilies.map(function (doc) {{
        var related = Array.isArray(doc.related_ai_documents) ? doc.related_ai_documents : [];
        var active = selectedTargetDoc && selectedTargetDoc.id === doc.id;
        var relatedHtml = related.length
          ? '<div class="user-doc-version-list">' + related.map(function (item) {{
              var itemActive = selectedTargetDoc && selectedTargetDoc.id === item.id;
              return '<div class="user-doc-version' + (itemActive ? ' is-active' : '') + '">' +
                '<div class="user-doc-version-main">' +
                  '<button type="button" class="user-doc-version-title" data-open-user-doc="' + escapeHtml(item.id) + '">' +
                    '<span class="user-doc-status">' + escapeHtml(String(item.status || 'draft')) + '</span>' +
                    '<span>' + escapeHtml(item.name || item.id || 'AI 草稿') + '</span>' +
                  '</button>' +
                  '<div class="user-doc-version-meta">更新于 ' + escapeHtml(formatTime(item.updated_at)) + '</div>' +
                '</div>' +
                '<button type="button" class="mini-btn primary" data-target-user-doc="' + escapeHtml(item.id) + '">设为目标</button>' +
              '</div>';
            }}).join('') + '</div>'
          : '<p class="user-doc-empty">当前还没有基于这份原文生成的 AI 工作草稿。</p>';
        return '<div class="user-doc-entry' + (active ? ' is-active' : '') + '">' +
          '<div class="user-doc-head">' +
            '<div class="user-doc-main">' +
              '<button type="button" class="user-doc-open" data-open-user-doc="' + escapeHtml(doc.id) + '">' + escapeHtml(doc.name || doc.id || '未命名文档') + '</button>' +
              '<div class="user-doc-meta">原始上传 · ' + escapeHtml(doc.mime_type || '未知类型') + ' · 更新于 ' + escapeHtml(formatTime(doc.updated_at)) + '</div>' +
            '</div>' +
            '<div class="user-doc-actions">' +
              '<button type="button" class="mini-btn primary" data-target-user-doc="' + escapeHtml(doc.id) + '">设为目标</button>' +
            '</div>' +
          '</div>' +
          '<div class="user-doc-related">' +
            '<div class="user-doc-related-label">相关 AI 版本</div>' +
            relatedHtml +
          '</div>' +
        '</div>';
      }}).join('');
      setUserDocListHtml(html);
    }}

    function bindUserDocEvents() {{
      if (!userDocListEl) return;
      Array.prototype.slice.call(userDocListEl.querySelectorAll('[data-open-user-doc]')).forEach(function (button) {{
        button.addEventListener('click', function () {{
          openUserDoc(button.getAttribute('data-open-user-doc') || '');
        }});
      }});
      Array.prototype.slice.call(userDocListEl.querySelectorAll('[data-target-user-doc]')).forEach(function (button) {{
        button.addEventListener('click', function () {{
          var doc = resolveSelectedDocById(button.getAttribute('data-target-user-doc') || '');
          if (!doc) return;
          setSelectedTarget(doc);
          setChannel('chat', {{ scroll: isMobileLayout() }});
          focusComposer();
        }});
      }});
    }}

    async function loadUserDocs() {{
      if (!userDocListEl) return;
      setUserDocListText(previewMode ? '管理预览不连接真实上传区，请重点检查页面结构和交互位置。' : '正在加载你的上传资料…');
      if (previewMode || !chatInteractive || !sdk.userDocs) {{
        return;
      }}
      try {{
        var payload = await sdk.userDocs.list();
        userDocFamilies = Array.isArray(payload && payload.documents) ? payload.documents : [];
        if (selectedTargetDoc) {{
          selectedTargetDoc = resolveSelectedDocById(selectedTargetDoc.id);
        }}
        renderSelectedTarget();
        renderUserDocFamilies();
        bindUserDocEvents();
        if (selectedTargetDoc) {{
          openUserDoc(selectedTargetDoc.id);
        }}
      }} catch (err) {{
        setUserDocListText('上传资料列表加载失败，请稍后刷新。');
      }}
    }}

    async function openPlatformDoc(docId) {{
      if (!docId) return;
      var selected = resolveSelectedDocById(docId);
      if (selected) setSelectedTarget(selected);
      setChannel('docs', {{ scroll: isMobileLayout() }});
      setDocsSource('platform');
      setPreviewState('正在加载资料…', '请稍候，正在读取平台资料。', '正在加载资料内容。');
      try {{
        var payload = await fetch('./api/docs/' + encodeURIComponent(docId))
          .then(function (res) {{
            if (!res.ok) throw new Error(String(res.status));
            return res.json();
          }});
        var matched = resolveSelectedDocById(docId);
        var title = matched ? matched.name : docId;
        var meta = matched ? matched.kind : '平台资料';
        if (matched && matched.kind === '平台资料') {{
          var platformDoc = null;
          for (var i = 0; i < platformDocs.length; i += 1) {{
            if (platformDocs[i] && platformDocs[i].id === docId) {{
              platformDoc = platformDocs[i];
              break;
            }}
          }}
          var parts = ['平台资料'];
          if (platformDoc && platformDoc.mime_type) parts.push(platformDoc.mime_type);
          if (platformDoc && platformDoc.file_size) parts.push(platformDoc.file_size + ' B');
          meta = parts.join(' · ');
        }}
        var text = payload && typeof payload.text === 'string' && payload.text.trim()
          ? payload.text
          : '当前资料暂无可展示文本，或该格式需要在后续版本中增强预览。';
        setPreviewState(title, meta, text);
        scrollDocsPreviewIntoView();
      }} catch (err) {{
        setPreviewState('资料预览失败', '请稍后重试。', '当前无法读取这份平台资料的内容。');
        scrollDocsPreviewIntoView();
      }}
    }}

    async function openUserDoc(docId) {{
      if (!docId || !sdk.userDocs) return;
      var selected = resolveSelectedDocById(docId);
      if (selected) setSelectedTarget(selected);
      setChannel('docs', {{ scroll: isMobileLayout() }});
      setDocsSource('uploads');
      setPreviewState('正在加载文档…', '请稍候，正在读取内容。', '正在加载文档内容。');
      try {{
        var payload = await sdk.userDocs.get(docId);
        var label = payload && payload.name ? payload.name : docId;
        var meta = [
          payload && payload.mime_type ? payload.mime_type : '未知类型',
          payload && payload.updated_at ? ('更新于 ' + formatTime(payload.updated_at)) : '刚刚更新'
        ].join(' · ');
        var text = payload && typeof payload.text === 'string' && payload.text.trim()
          ? payload.text
          : '当前文档暂无可展示文本，或该格式需要在后续版本中增强预览。';
        setPreviewState(label, meta, text);
        scrollDocsPreviewIntoView();
      }} catch (err) {{
        setPreviewState('文档预览失败', '请稍后重试。', '当前无法读取这份文档的内容。');
        scrollDocsPreviewIntoView();
      }}
    }}

    function setBusy(nextBusy) {{
      busy = nextBusy;
      if (inputEl) inputEl.disabled = nextBusy;
      if (sendBtn) sendBtn.disabled = nextBusy;
      if (stopBtn) stopBtn.disabled = !nextBusy;
      if (uploadTriggerEl) uploadTriggerEl.disabled = nextBusy;
      if (uploadRefreshEl) uploadRefreshEl.disabled = nextBusy;
      if (targetClearEl) targetClearEl.disabled = nextBusy || !selectedTargetDoc;
      promptButtons.forEach(function (button) {{
        button.disabled = nextBusy;
      }});
      if (voiceBtnEl) {{
        voiceBtnEl.disabled = nextBusy || !speechSupported;
      }}
      if (nextBusy && speechListening && speechRecognition) {{
        try {{
          speechRecognition.stop();
        }} catch (err) {{}}
      }}
      updateVoiceButtonState();
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
      inputEl.style.height = 'auto';
      var nextHeight = Math.min(Math.max(inputEl.scrollHeight, 72), 180);
      inputEl.style.height = nextHeight + 'px';
      inputEl.style.overflowY = inputEl.scrollHeight > 180 ? 'auto' : 'hidden';
    }}

    function updateVoiceButtonState() {{
      if (!voiceBtnEl) return;
      voiceBtnEl.classList.toggle('is-active', speechListening);
      if (!speechSupported) {{
        voiceBtnEl.textContent = '语音不可用';
        voiceBtnEl.title = '当前浏览器暂不支持语音输入';
        voiceBtnEl.disabled = true;
        return;
      }}
      voiceBtnEl.title = speechListening ? '点击结束当前语音录入' : '点击开始语音录入';
      voiceBtnEl.textContent = speechListening ? '结束听写' : '语音输入';
      voiceBtnEl.disabled = busy;
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
      var agentContent = content;
      if (selectedTargetDoc) {{
        var scopeNote = [
          '【当前处理目标】',
          '请优先处理以下文档，不要误改其他文档。',
          '目标文档ID: ' + selectedTargetDoc.id,
          '目标文档名称: ' + selectedTargetDoc.name,
          '目标文档类型: ' + selectedTargetDoc.kind
        ];
        if (selectedTargetDoc.sourceDocumentIds && selectedTargetDoc.sourceDocumentIds.length > 0) {{
          scopeNote.push('来源原文ID: ' + selectedTargetDoc.sourceDocumentIds.join(', '));
        }}
        scopeNote.push('如果该文档已经存在可继续的 AI 草稿，请优先沿用该版本。');
        scopeNote.push('');
        scopeNote.push(content);
        agentContent = scopeNote.join('\n');
      }}
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
        activeControl = await sdk.chat.sendAndStream(agentContent, {{
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
            loadUserDocs();
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

    async function syncAuthSession() {{
      if (!sdk.auth) {{
        authUser = null;
        renderAuthState();
        return;
      }}
      try {{
        var payload = await sdk.auth.session();
        authUser = payload && payload.authenticated ? (payload.user || null) : null;
      }} catch (err) {{
        authUser = null;
      }}
      renderAuthState();
    }}

    if (headerShellEl) {{
      syncHeaderShellLabel();
      headerShellEl.addEventListener('toggle', function () {{
        syncHeaderShellLabel();
      }});
    }}

    if (speechSupported) {{
      try {{
        speechRecognition = new SpeechRecognitionCtor();
        speechRecognition.lang = navigator.language || 'zh-CN';
        speechRecognition.interimResults = true;
        speechRecognition.continuous = false;
        speechRecognition.maxAlternatives = 1;
        speechRecognition.onstart = function () {{
          speechListening = true;
          speechBaseValue = inputEl ? String(inputEl.value || '') : '';
          updateVoiceButtonState();
          setStatus('正在语音录入', '请直接说出问题或资料处理目标，系统会写入输入框。');
        }};
        speechRecognition.onresult = function (event) {{
          if (!inputEl) return;
          var transcript = '';
          for (var index = event.resultIndex; index < event.results.length; index += 1) {{
            transcript += String(event.results[index][0] && event.results[index][0].transcript || '');
          }}
          inputEl.value = mergeComposerValue(speechBaseValue, transcript);
          autoResize();
        }};
        speechRecognition.onerror = function (event) {{
          speechListening = false;
          updateVoiceButtonState();
          setStatus('语音录入失败', event && event.error ? ('当前浏览器返回：' + event.error) : '请改用键盘输入，或检查麦克风权限后再试。');
        }};
        speechRecognition.onend = function () {{
          var hasText = !!(inputEl && String(inputEl.value || '').trim());
          speechListening = false;
          updateVoiceButtonState();
          if (hasText && !busy) {{
            setStatus('语音录入完成', '你可以继续补充文字，或直接发送。');
          }}
        }};
      }} catch (err) {{
        speechRecognition = null;
        speechSupported = false;
      }}
    }}

    updateVoiceButtonState();

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
    if (voiceBtnEl) {{
      voiceBtnEl.addEventListener('click', function () {{
        if (voiceBtnEl.disabled) return;
        if (!speechRecognition) {{
          setStatus('语音不可用', '当前浏览器暂不支持语音输入，请改用键盘输入。');
          return;
        }}
        if (speechListening) {{
          try {{
            speechRecognition.stop();
          }} catch (err) {{
            speechListening = false;
            updateVoiceButtonState();
          }}
          return;
        }}
        if (inputEl && inputEl.disabled) return;
        try {{
          speechBaseValue = inputEl ? String(inputEl.value || '') : '';
          speechRecognition.start();
        }} catch (err) {{
          setStatus('无法启动语音录入', err && err.message ? err.message : '请稍后重试，或检查浏览器的麦克风权限。');
        }}
      }});
    }}
    focusChatButtons.forEach(function (button) {{
      button.addEventListener('click', function () {{
        setChannel('chat', {{ scroll: isMobileLayout() }});
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
    if (uploadTriggerEl && uploadInputEl) {{
      uploadTriggerEl.addEventListener('click', function () {{
        if (uploadTriggerEl.disabled) return;
        uploadInputEl.click();
      }});
      uploadInputEl.addEventListener('change', function () {{
        var file = uploadInputEl.files && uploadInputEl.files[0];
        if (!file || !sdk.userDocs) return;
        setStatus('正在上传资料', '上传成功后会自动加入本轮对话可访问范围。');
        Promise.resolve()
          .then(function () {{
            return sdk.userDocs.upload(file);
          }})
          .then(function (payload) {{
            var doc = payload && payload.document ? payload.document : null;
            uploadInputEl.value = '';
            setStatus(
              '资料上传完成',
              doc && doc.name
                ? (doc.name + (authUser ? ' 已加入当前账号资料区。' : ' 已加入当前访客资料区。'))
                : '你可以直接让分身处理这份新资料。'
            );
            return loadUserDocs().then(function () {{
              if (doc && doc.id) {{
                var selected = resolveSelectedDocById(doc.id);
                if (selected) {{
                  setSelectedTarget(selected);
                  openUserDoc(doc.id);
                }}
              }}
            }});
          }})
          .catch(function (err) {{
            uploadInputEl.value = '';
            setStatus('资料上传失败', err && err.message ? err.message : '请稍后重试。');
          }});
      }});
    }}
    if (uploadRefreshEl) {{
      uploadRefreshEl.addEventListener('click', function () {{
        loadUserDocs();
      }});
    }}
    authModeButtons.forEach(function (button) {{
      button.addEventListener('click', function () {{
        if (authBusy || authUser) return;
        setAuthMode(button.getAttribute('data-auth-mode') || 'login');
      }});
    }});
    if (authFormEl) {{
      authFormEl.addEventListener('submit', function (event) {{
        event.preventDefault();
        if (authBusy || authUser || !sdk.auth) return;
        var username = authUsernameEl ? String(authUsernameEl.value || '').trim() : '';
        var password = authPasswordEl ? String(authPasswordEl.value || '') : '';
        if (!username || !password) {{
          setAuthStatus('请输入用户名和密码后再继续。');
          return;
        }}
        var payload = {{
          username: username,
          password: password
        }};
        if (authMode === 'register') {{
          if (authDisplayNameEl) payload.displayName = String(authDisplayNameEl.value || '').trim();
          if (authPhoneEl) payload.phone = String(authPhoneEl.value || '').trim();
        }}
        setAuthBusy(true);
        setAuthStatus(authMode === 'register' ? '正在注册并迁移当前匿名资料…' : '正在登录并恢复当前身份…');
        Promise.resolve()
          .then(function () {{
            return authMode === 'register' ? sdk.auth.register(payload) : sdk.auth.login(payload);
          }})
          .then(function (result) {{
            authUser = result && result.user ? result.user : null;
            if (authDisplayNameEl) authDisplayNameEl.value = '';
            if (authPhoneEl) authPhoneEl.value = '';
            if (authPasswordEl) authPasswordEl.value = '';
            renderAuthState();
            return loadUserDocs();
          }})
          .then(function () {{
            setAuthStatus(authMode === 'register' ? '注册成功，当前匿名资料已归入你的外部用户身份。' : '登录成功，已切换到外部用户身份。');
          }})
          .catch(function (err) {{
            setAuthStatus(err && err.message ? err.message : (authMode === 'register' ? '注册失败，请稍后重试。' : '登录失败，请稍后重试。'));
          }})
          .finally(function () {{
            setAuthBusy(false);
          }});
      }});
    }}
    if (authLogoutEl) {{
      authLogoutEl.addEventListener('click', function () {{
        if (authBusy || !sdk.auth) return;
        setAuthBusy(true);
        setAuthStatus('正在退出当前身份…');
        Promise.resolve()
          .then(function () {{
            return sdk.auth.logout();
          }})
          .then(function (result) {{
            authUser = result && result.authenticated ? (result.user || null) : null;
            return syncAuthSession();
          }})
          .then(function () {{
            sdk.chat.clearLocalSession();
            setSelectedTarget(null);
            renderAuthState();
            setPreviewState('先从左侧选择资料', '支持查看平台资料、你上传的原始文档，以及这些原文衍生出来的 AI 工作草稿。', '尚未选择文档。');
            return loadUserDocs();
          }})
          .then(function () {{
            setAuthMode('login');
            setAuthStatus('已退出登录，当前恢复为匿名访客模式。');
          }})
          .catch(function (err) {{
            setAuthStatus(err && err.message ? err.message : '退出失败，请稍后重试。');
          }})
          .finally(function () {{
            setAuthBusy(false);
          }});
      }});
    }}
    if (targetClearEl) {{
      targetClearEl.addEventListener('click', function () {{
        setSelectedTarget(null);
      }});
    }}
    if (previewChatJumpEl) {{
      previewChatJumpEl.addEventListener('click', function () {{
        setChannel('chat', {{ scroll: isMobileLayout() }});
        window.setTimeout(function () {{
          focusComposer();
        }}, 120);
      }});
    }}

    setAuthMode('login');
    setChannel('chat');
    renderSelectedTarget();
    renderAuthState();
    syncAuthSession().then(function () {{
      loadUserDocs();
    }});
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
        team_name: Option<&str>,
    ) -> String {
        let title = Self::escape_html(&portal.name);
        let header_kicker = Self::escape_html(&Self::resolve_avatar_header_kicker(team_name));
        let avatar_type = Self::resolve_avatar_type_label(portal);
        let header_intro = Self::avatar_public_narrative_str(portal, "heroIntro")
            .or_else(|| {
                portal
                    .description
                    .as_ref()
                    .map(|value| value.trim().to_string())
            })
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| {
                "这是一个面向外部用户的服务分身，你可以直接通过对话描述问题、目标或需要处理的资料。"
                    .to_string()
            });
        let header_use_cases = {
            let configured = Self::avatar_public_narrative_list(portal, "heroUseCases");
            if configured.is_empty() {
                let mut defaults = vec![
                    "梳理问题并给出下一步建议".to_string(),
                    "结合当前资料回答问题".to_string(),
                ];
                match Self::resolve_effective_document_access_mode(portal) {
                    crate::models::mongo::PortalDocumentAccessMode::ReadOnly => {
                        defaults.push("总结资料重点，不直接改写文档".to_string());
                    }
                    crate::models::mongo::PortalDocumentAccessMode::CoEditDraft => {
                        defaults.push("基于资料继续生成新版本或协作草稿".to_string());
                    }
                    crate::models::mongo::PortalDocumentAccessMode::ControlledWrite => {
                        defaults.push("继续处理目标文档，并保留版本记录".to_string());
                    }
                }
                defaults
            } else {
                configured
            }
        };
        let header_working_style = Self::avatar_public_narrative_str(portal, "heroWorkingStyle")
            .unwrap_or_else(|| "我会先结合当前开放资料处理；如果超出范围，会继续交给管理 Agent 判断，必要时再转给人工审核。".to_string());
        let header_cta_hint = Self::avatar_public_narrative_str(portal, "heroCtaHint")
            .unwrap_or_else(|| {
                "直接在对话频道描述问题；如果需要我结合资料处理，先去资料频道选中目标文档。"
                    .to_string()
            });
        let effective_doc_mode = effective
            .map(|config| config.effective_document_access_mode)
            .unwrap_or_else(|| Self::resolve_effective_document_access_mode(portal));
        let capability_items = Self::render_capability_items(portal);
        let boundary_items = Self::render_avatar_boundary_items(portal);
        let boundary_cards = Self::render_avatar_boundary_cards(portal);
        let example_items = Self::render_avatar_example_prompts(portal);
        let faq_items = Self::render_avatar_faq_items(portal);
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
            "管理预览模式，不建立真实访客会话。"
        } else if runtime_chat_interactive {
            "直接描述目标或问题，Enter 发送，Shift+Enter 换行。"
        } else {
            "当前未开放对外对话能力。"
        };
        let chat_placeholder = if runtime_chat_interactive {
            "先告诉我目标、背景和希望得到的结果"
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
            "直接在对话频道说明目标、问题、期望结果和限制条件即可；如果需要我处理某份资料，先在资料频道选中目标文档。"
        } else {
            "当前分身尚未启用对外对话能力，所以页面只展示说明信息。"
        };
        let docs_summary = if docs_count > 0 {
            format!("已开放 {} 份平台资料", docs_count)
        } else {
            "当前没有预置平台资料".to_string()
        };
        let upload_summary = if preview_mode {
            "管理预览仅检查上传区布局".to_string()
        } else if runtime_chat_interactive {
            "支持上传个人资料，并按当前身份隔离".to_string()
        } else {
            "当前未开放个人资料上传".to_string()
        };
        let sync_summary = if preview_mode {
            "预览模式不连接真实身份".to_string()
        } else {
            "登录后可跨设备继续使用".to_string()
        };
        let runtime_assets = Self::render_digital_avatar_runtime_assets(
            &portal.slug,
            welcome_message,
            preview_mode,
            runtime_chat_interactive,
        );
        let footer_year = Utc::now().year();
        let footer_website_url = "https://www.agiatme.com";
        let footer_website_label = "Agime Official Website";
        let footer_github_url = "https://github.com/jsjm1986/AGIME";

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
    [hidden] {{
      display: none !important;
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
      display: flex;
      flex-wrap: wrap;
      gap: 8px;
      padding: 14px 24px 0;
    }}
    .prompt-chip {{
      display: inline-flex;
      align-items: center;
      max-width: 100%;
      min-height: 38px;
      padding: 0 14px;
      border-radius: 999px;
      border: 1px solid rgba(148, 163, 184, 0.22);
      background: rgba(255,255,255,0.9);
      color: var(--muted-strong);
      cursor: pointer;
      text-align: center;
      touch-action: manipulation;
      white-space: nowrap;
      overflow: hidden;
      text-overflow: ellipsis;
      transition: transform 0.18s ease, border-color 0.18s ease, box-shadow 0.18s ease, color 0.18s ease;
    }}
    .prompt-chip:hover {{
      transform: translateY(-1px);
      border-color: rgba(20, 184, 166, 0.26);
      box-shadow: 0 10px 22px rgba(15, 23, 42, 0.06);
      color: var(--ink);
    }}
    .prompt-chip[disabled] {{
      cursor: not-allowed;
      color: #94a3b8;
      background: #f8fafc;
      box-shadow: none;
      transform: none;
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
      min-height: 72px;
      max-height: 180px;
      resize: none;
      overflow-y: hidden;
      overflow-x: hidden;
      border-radius: 20px;
      border: 1px solid rgba(148, 163, 184, 0.26);
      padding: 15px 17px;
      font: inherit;
      font-size: 15px;
      line-height: 1.55;
      color: var(--ink);
      outline: none;
      background: rgba(255,255,255,0.96);
      box-shadow: inset 0 1px 0 rgba(255,255,255,0.5);
      appearance: none;
      scrollbar-width: none;
      transition: border-color 0.18s ease, box-shadow 0.18s ease;
    }}
    .chat-composer textarea::-webkit-scrollbar {{
      width: 0;
      height: 0;
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
    .chat-target {{
      display: flex;
      align-items: center;
      justify-content: space-between;
      gap: 14px;
      margin-bottom: 14px;
      padding: 14px 16px;
      border-radius: 20px;
      border: 1px solid rgba(15, 118, 110, 0.18);
      background: linear-gradient(135deg, rgba(240, 253, 250, 0.96) 0%, rgba(255,255,255,0.96) 100%);
    }}
    .chat-target-copy {{
      min-width: 0;
    }}
    .chat-target-label {{
      display: inline-flex;
      align-items: center;
      gap: 6px;
      font-size: 11px;
      font-weight: 800;
      letter-spacing: 0.18em;
      text-transform: uppercase;
      color: var(--brand);
    }}
    .chat-target-copy strong {{
      display: block;
      margin-top: 6px;
      font-size: 15px;
      color: var(--ink);
    }}
    .chat-target-copy p {{
      margin: 6px 0 0;
      font-size: 12px;
      color: var(--muted);
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
    .action-btn-voice.is-active {{
      border-color: rgba(15, 118, 110, 0.24);
      background: linear-gradient(135deg, rgba(236, 254, 255, 0.96) 0%, rgba(220, 252, 231, 0.96) 100%);
      color: var(--brand);
      box-shadow: 0 12px 24px rgba(15, 118, 110, 0.12);
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
    .auth-session {{
      display: flex;
      align-items: center;
      justify-content: space-between;
      gap: 12px;
      margin-top: 14px;
      padding: 14px 15px;
      border-radius: 18px;
      border: 1px solid rgba(15, 118, 110, 0.18);
      background: linear-gradient(135deg, rgba(240, 253, 250, 0.96) 0%, rgba(255,255,255,0.96) 100%);
    }}
    .auth-session-main {{
      min-width: 0;
      flex: 1;
      display: grid;
      gap: 6px;
    }}
    .auth-session-main strong {{
      font-size: 15px;
      color: var(--ink);
    }}
    .auth-session-main p {{
      margin: 0;
      font-size: 12px;
      color: var(--muted);
    }}
    .auth-badge {{
      display: inline-flex;
      align-items: center;
      width: fit-content;
      min-height: 24px;
      padding: 0 10px;
      border-radius: 999px;
      background: rgba(15, 118, 110, 0.12);
      color: var(--brand);
      font-size: 11px;
      font-weight: 800;
      letter-spacing: 0.08em;
      text-transform: uppercase;
    }}
    .auth-switch {{
      display: flex;
      flex-wrap: wrap;
      gap: 8px;
      margin-top: 14px;
    }}
    .auth-switch .mini-btn.is-active {{
      border-color: rgba(15, 118, 110, 0.24);
      color: var(--brand);
      background: linear-gradient(135deg, rgba(236, 254, 255, 0.96) 0%, rgba(255,255,255,0.96) 100%);
    }}
    .auth-form {{
      margin-top: 14px;
      display: grid;
      gap: 10px;
    }}
    .auth-field {{
      display: grid;
      gap: 6px;
    }}
    .auth-field span {{
      font-size: 12px;
      font-weight: 700;
      color: var(--muted);
    }}
    .auth-field input {{
      width: 100%;
      min-height: 42px;
      border-radius: 14px;
      border: 1px solid rgba(148, 163, 184, 0.22);
      background: rgba(255,255,255,0.95);
      color: var(--ink);
      font: inherit;
      padding: 0 14px;
      outline: none;
      transition: border-color 0.18s ease, box-shadow 0.18s ease;
    }}
    .auth-field input:focus {{
      border-color: rgba(20, 184, 166, 0.34);
      box-shadow: 0 0 0 4px rgba(20, 184, 166, 0.08);
    }}
    .auth-field input:disabled {{
      background: #f8fafc;
      color: #94a3b8;
    }}
    .auth-form-actions {{
      display: flex;
      justify-content: flex-start;
      margin-top: 4px;
    }}
    .auth-status {{
      min-height: 18px;
      margin-top: 12px;
    }}
    .user-doc-list {{
      display: grid;
      gap: 12px;
      margin-top: 14px;
    }}
    .user-doc-empty {{
      margin: 0;
      padding: 14px 16px;
      border-radius: 18px;
      border: 1px dashed rgba(148, 163, 184, 0.36);
      color: var(--muted);
      font-size: 13px;
      background: rgba(248, 250, 252, 0.88);
    }}
    .user-doc-entry {{
      padding: 14px 15px;
      border-radius: 18px;
      border: 1px solid rgba(148, 163, 184, 0.18);
      background: linear-gradient(180deg, rgba(255,255,255,0.96) 0%, rgba(248,250,252,0.96) 100%);
      box-shadow: inset 0 1px 0 rgba(255,255,255,0.7);
    }}
    .user-doc-entry.is-active {{
      border-color: rgba(15, 118, 110, 0.28);
      box-shadow: 0 16px 26px rgba(15, 23, 42, 0.08);
    }}
    .user-doc-head {{
      display: flex;
      align-items: flex-start;
      justify-content: space-between;
      gap: 12px;
    }}
    .user-doc-main {{
      min-width: 0;
      flex: 1;
    }}
    .user-doc-open {{
      display: inline-flex;
      align-items: center;
      gap: 8px;
      padding: 0;
      border: none;
      background: none;
      color: var(--ink);
      font: inherit;
      font-size: 15px;
      font-weight: 800;
      cursor: pointer;
      text-align: left;
    }}
    .user-doc-open:hover {{
      color: var(--brand);
    }}
    .user-doc-meta {{
      margin-top: 6px;
      font-size: 12px;
      color: var(--muted);
    }}
    .user-doc-actions {{
      display: flex;
      flex-wrap: wrap;
      gap: 8px;
      justify-content: flex-end;
    }}
    .mini-btn {{
      display: inline-flex;
      align-items: center;
      justify-content: center;
      min-height: 32px;
      padding: 0 12px;
      border-radius: 999px;
      border: 1px solid rgba(148, 163, 184, 0.24);
      background: rgba(255,255,255,0.94);
      color: var(--ink);
      font-size: 12px;
      font-weight: 700;
      cursor: pointer;
      transition: border-color 0.18s ease, transform 0.18s ease;
    }}
    .mini-btn:hover {{
      transform: translateY(-1px);
      border-color: rgba(15, 118, 110, 0.26);
    }}
    .mini-btn.primary {{
      border-color: rgba(15, 118, 110, 0.24);
      color: var(--brand);
    }}
    .mini-btn[disabled] {{
      cursor: not-allowed;
      color: #94a3b8;
      transform: none;
      background: #f8fafc;
    }}
    .user-doc-related {{
      margin-top: 12px;
      padding-top: 12px;
      border-top: 1px dashed rgba(148, 163, 184, 0.24);
      display: grid;
      gap: 8px;
    }}
    .user-doc-related-label {{
      font-size: 11px;
      font-weight: 800;
      letter-spacing: 0.18em;
      text-transform: uppercase;
      color: var(--muted);
    }}
    .user-doc-version-list {{
      display: grid;
      gap: 8px;
    }}
    .user-doc-version {{
      display: flex;
      align-items: center;
      justify-content: space-between;
      gap: 10px;
      padding: 10px 12px;
      border-radius: 14px;
      border: 1px solid rgba(148, 163, 184, 0.16);
      background: rgba(248, 250, 252, 0.92);
    }}
    .user-doc-version-main {{
      min-width: 0;
      flex: 1;
    }}
    .user-doc-version-title {{
      display: inline-flex;
      align-items: center;
      gap: 8px;
      padding: 0;
      border: none;
      background: none;
      color: var(--ink);
      font: inherit;
      font-size: 13px;
      font-weight: 700;
      cursor: pointer;
      text-align: left;
    }}
    .user-doc-version-title:hover {{
      color: var(--brand);
    }}
    .user-doc-version-meta {{
      margin-top: 4px;
      font-size: 12px;
      color: var(--muted);
    }}
    .user-doc-status {{
      display: inline-flex;
      align-items: center;
      min-height: 22px;
      padding: 0 9px;
      border-radius: 999px;
      background: rgba(15, 23, 42, 0.08);
      color: var(--muted);
      font-size: 11px;
      font-weight: 800;
      letter-spacing: 0.08em;
      text-transform: uppercase;
    }}
    .user-doc-preview {{
      margin: 14px 0 0;
      min-height: 180px;
      max-height: 320px;
      overflow: auto;
      padding: 14px 15px;
      border-radius: 18px;
      border: 1px solid rgba(148, 163, 184, 0.16);
      background: rgba(248, 250, 252, 0.92);
      color: var(--ink);
      font-family: "IBM Plex Mono", "SFMono-Regular", Consolas, monospace;
      font-size: 12px;
      line-height: 1.65;
      white-space: pre-wrap;
      word-break: break-word;
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
    .mobile-quickbar,
    .mobile-rail-nav {{
      display: none;
    }}
    .mobile-quickbar {{
      grid-template-columns: repeat(2, minmax(0, 1fr));
      gap: 8px;
      padding: 0 24px 16px;
    }}
    .mobile-quickbar-btn,
    .mobile-rail-tab {{
      display: inline-flex;
      align-items: center;
      justify-content: center;
      min-height: 40px;
      padding: 0 14px;
      border-radius: 999px;
      border: 1px solid rgba(148, 163, 184, 0.22);
      background: rgba(255,255,255,0.94);
      color: var(--ink);
      font-size: 12px;
      font-weight: 800;
      cursor: pointer;
      touch-action: manipulation;
      white-space: nowrap;
      transition: transform 0.18s ease, border-color 0.18s ease, box-shadow 0.18s ease, background 0.18s ease;
    }}
    .mobile-quickbar-btn:hover,
    .mobile-rail-tab:hover {{
      transform: translateY(-1px);
      border-color: rgba(15, 118, 110, 0.22);
      box-shadow: 0 10px 18px rgba(15, 23, 42, 0.06);
    }}
    .mobile-quickbar-btn.is-active,
    .mobile-rail-tab.is-active {{
      border-color: rgba(15, 118, 110, 0.24);
      background: linear-gradient(135deg, rgba(236, 254, 255, 0.96) 0%, rgba(255,255,255,0.96) 100%);
      color: var(--brand);
    }}
    .mobile-quickbar-btn[disabled],
    .mobile-rail-tab[disabled] {{
      cursor: not-allowed;
      color: #94a3b8;
      background: #f8fafc;
      transform: none;
      box-shadow: none;
    }}
    .mobile-rail-nav {{
      gap: 8px;
      overflow-x: auto;
      padding: 2px 0 6px;
      scrollbar-width: none;
    }}
    .mobile-rail-nav::-webkit-scrollbar {{
      display: none;
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
    .project-shell {{
      display: grid;
      gap: 18px;
    }}
    .project-footer {{
      display: grid;
      gap: 10px;
      justify-items: center;
      padding: 4px 0 12px;
    }}
    .project-footer-links {{
      display: flex;
      flex-wrap: wrap;
      justify-content: center;
      gap: 12px 18px;
    }}
    .project-footer-link {{
      display: inline-flex;
      align-items: center;
      gap: 6px;
      color: var(--muted);
      font-size: 12px;
      font-weight: 600;
      text-decoration: none;
      transition: color 0.18s ease, transform 0.18s ease;
    }}
    .project-footer-link:hover {{
      color: var(--ink);
      transform: translateY(-1px);
    }}
    .project-footer-link svg {{
      width: 13px;
      height: 13px;
      flex: 0 0 auto;
      opacity: 0.76;
    }}
    .project-footer-copy {{
      color: rgba(92, 107, 128, 0.8);
      font-size: 11px;
      text-align: center;
      line-height: 1.5;
    }}
    .project-header {{
      position: relative;
      display: grid;
      gap: 12px;
      padding: 4px 2px 0;
    }}
    .project-header-shell {{
      display: grid;
      gap: 8px;
    }}
    .project-header-shell > summary {{
      display: flex;
      width: 100%;
      list-style: none;
    }}
    .project-header-shell > summary::-webkit-details-marker {{
      display: none;
    }}
    .project-header-shell:not([open]) .project-header-main {{
      display: none;
    }}
    .project-header-shell[open] .project-header-main {{
      padding: 4px 2px 0;
    }}
    .project-header-main {{
      display: flex;
      flex-wrap: wrap;
      align-items: flex-start;
      justify-content: space-between;
      gap: 8px 14px;
    }}
    .project-header-summary {{
      display: flex;
      align-items: center;
      justify-content: space-between;
      gap: 12px;
      padding: 14px 16px;
      border-radius: 22px;
      border: 1px solid rgba(148, 163, 184, 0.18);
      background: rgba(255,255,255,0.88);
      box-shadow: 0 14px 28px rgba(15, 23, 42, 0.04);
      cursor: pointer;
      touch-action: manipulation;
    }}
    .project-header-summary-copy {{
      min-width: 0;
      display: grid;
      gap: 4px;
    }}
    .project-header-summary-copy h1 {{
      margin: 0;
      font-size: clamp(22px, 2.6vw, 30px);
      line-height: 1.04;
      letter-spacing: -0.03em;
      color: var(--ink);
    }}
    .project-header-summary-action {{
      display: inline-flex;
      align-items: center;
      justify-content: center;
      min-height: 32px;
      padding: 0 12px;
      border-radius: 999px;
      border: 1px solid rgba(15, 118, 110, 0.16);
      background: rgba(236, 254, 255, 0.84);
      color: var(--brand);
      font-size: 12px;
      font-weight: 800;
      white-space: nowrap;
    }}
    .project-header-copy {{
      min-width: 0;
      flex: 1;
      max-width: 720px;
      display: grid;
      gap: 6px;
    }}
    .project-header-kicker {{
      margin: 0;
      font-size: 10px;
      font-weight: 800;
      letter-spacing: 0.14em;
      text-transform: uppercase;
      color: var(--brand);
    }}
    .project-header-copy h1 {{
      margin: 0;
      font-size: clamp(26px, 3vw, 34px);
      line-height: 1.02;
      letter-spacing: -0.03em;
      color: var(--ink);
    }}
    .project-header-copy p {{
      margin: 0;
      font-size: 15px;
      line-height: 1.6;
      color: var(--muted);
    }}
    .project-header-lead {{
      max-width: 66ch;
      color: var(--muted-strong);
    }}
    .project-header-scenes {{
      margin-top: 8px;
      display: flex;
      flex-wrap: wrap;
      gap: 8px;
    }}
    .project-header-scene {{
      display: inline-flex;
      align-items: center;
      min-height: 30px;
      padding: 0 12px;
      border-radius: 999px;
      border: 1px solid rgba(15, 118, 110, 0.16);
      background: rgba(236, 254, 255, 0.86);
      color: var(--brand);
      font-size: 12px;
      font-weight: 700;
      line-height: 1;
      white-space: nowrap;
    }}
    .project-header-note {{
      margin-top: 8px;
      max-width: 760px;
      font-size: 13px;
      line-height: 1.65;
      color: var(--muted-strong);
    }}
    .project-header-note strong {{
      color: var(--ink);
      font-weight: 800;
      margin-right: 6px;
    }}
    .project-header-meta {{
      display: flex;
      flex-wrap: wrap;
      gap: 6px;
      align-items: center;
      justify-content: flex-end;
    }}
    .project-status-chip {{
      display: inline-flex;
      align-items: center;
      justify-content: center;
      min-height: 30px;
      padding: 0 11px;
      border-radius: 999px;
      border: 1px solid rgba(148, 163, 184, 0.22);
      background: rgba(255,255,255,0.82);
      color: var(--muted-strong);
      font-size: 11px;
      font-weight: 800;
    }}
    .project-hero {{
      display: grid;
      grid-template-columns: minmax(0, 1.45fr) minmax(280px, 0.85fr);
      gap: 22px;
      align-items: end;
    }}
    .project-hero-main,
    .project-hero-side {{
      position: relative;
      z-index: 1;
      min-width: 0;
    }}
    .project-hero-actions {{
      margin-top: 20px;
      display: flex;
      flex-wrap: wrap;
      gap: 12px;
    }}
    .project-hero-summary {{
      padding: 18px;
      border-radius: 24px;
      background: linear-gradient(180deg, rgba(255,255,255,0.14) 0%, rgba(255,255,255,0.08) 100%);
      border: 1px solid rgba(255,255,255,0.12);
      backdrop-filter: blur(14px);
    }}
    .project-hero-summary h2 {{
      margin: 8px 0 10px;
      font-size: 24px;
      line-height: 1.15;
    }}
    .project-hero-summary p,
    .project-hero-summary li {{
      color: rgba(226, 232, 240, 0.84);
      font-size: 14px;
      line-height: 1.7;
    }}
    .project-hero-summary ul {{
      margin: 12px 0 0;
      padding-left: 18px;
      display: grid;
      gap: 8px;
    }}
    .project-hero-note {{
      margin: 0;
      padding: 16px 18px;
      border-radius: 20px;
      border: 1px solid rgba(255,255,255,0.12);
      background: rgba(255,255,255,0.08);
      color: rgba(226, 232, 240, 0.76);
      font-size: 13px;
      line-height: 1.65;
    }}
    .project-toolbar {{
      position: sticky;
      top: 14px;
      z-index: 6;
      display: flex;
      flex-wrap: wrap;
      align-items: center;
      justify-content: space-between;
      gap: 14px;
      padding: 14px 16px;
      border-radius: 24px;
      border: 1px solid rgba(148, 163, 184, 0.2);
      background: rgba(255,255,255,0.82);
      box-shadow: var(--shadow-soft);
      backdrop-filter: blur(16px);
    }}
    .project-toolbar-main {{
      min-width: 240px;
      flex: 1;
      display: grid;
      gap: 4px;
    }}
    .project-toolbar-eyebrow {{
      font-size: 11px;
      font-weight: 800;
      letter-spacing: 0.16em;
      text-transform: uppercase;
      color: var(--brand);
    }}
    .project-toolbar-main strong {{
      color: var(--ink);
      font-size: 16px;
    }}
    .project-toolbar-main p {{
      margin: 0;
      color: var(--muted);
      font-size: 13px;
      line-height: 1.65;
    }}
    .channel-nav {{
      display: inline-flex;
      flex-wrap: nowrap;
      gap: 8px;
      max-width: 100%;
      overflow-x: auto;
      padding: 4px;
      border-radius: 999px;
      border: 1px solid rgba(148, 163, 184, 0.18);
      background: rgba(255,255,255,0.78);
      box-shadow: 0 14px 28px rgba(15, 23, 42, 0.04);
      scrollbar-width: none;
    }}
    .channel-nav::-webkit-scrollbar {{
      display: none;
    }}
    .channel-btn {{
      display: inline-flex;
      align-items: center;
      justify-content: center;
      min-height: 38px;
      padding: 0 14px;
      border-radius: 999px;
      border: 1px solid transparent;
      background: transparent;
      color: var(--muted-strong);
      font-size: 13px;
      font-weight: 800;
      cursor: pointer;
      transition: transform 0.18s ease, border-color 0.18s ease, background 0.18s ease, color 0.18s ease;
      white-space: nowrap;
    }}
    .channel-btn:hover {{
      transform: translateY(-1px);
      border-color: rgba(15, 118, 110, 0.28);
      background: rgba(255,255,255,0.68);
      color: var(--ink);
    }}
    .channel-btn.is-active {{
      border-color: rgba(15, 118, 110, 0.24);
      background: linear-gradient(135deg, rgba(236, 254, 255, 0.96) 0%, rgba(255,255,255,0.96) 100%);
      color: var(--brand);
      box-shadow: 0 10px 22px rgba(15, 23, 42, 0.06);
    }}
    .channel-stage {{
      display: grid;
      gap: 18px;
    }}
    .channel-panel {{
      scroll-margin-top: 96px;
    }}
    .chat-stage-wide {{
      width: 100%;
    }}
    .chat-stage.chat-stage-wide {{
      padding-top: 8px;
    }}
    .chat-stage-head-compact {{
      display: flex;
      align-items: flex-start;
      justify-content: space-between;
      gap: 14px;
      flex-wrap: wrap;
    }}
    .chat-stage-copy {{
      min-width: 0;
      flex: 1;
    }}
    .chat-stage-links {{
      display: flex;
      flex-wrap: wrap;
      gap: 8px;
    }}
    .channel-panel {{
      display: none;
    }}
    .channel-panel.is-active {{
      display: block;
      animation: rise-in 0.35s ease-out both;
    }}
    .channel-grid,
    .account-grid {{
      display: grid;
      grid-template-columns: minmax(0, 1.45fr) minmax(280px, 0.8fr);
      gap: 18px;
      align-items: start;
    }}
    .channel-panel[data-channel-panel="chat"] .channel-grid {{
      grid-template-columns: 1fr;
    }}
    .guide-grid {{
      display: grid;
      grid-template-columns: minmax(0, 1.05fr) minmax(320px, 0.95fr);
      gap: 18px;
      align-items: start;
    }}
    .chat-sidebar,
    .docs-sidebar,
    .guide-stack,
    .account-sidebar {{
      display: grid;
      gap: 16px;
    }}
    .channel-card-head {{
      margin-bottom: 16px;
      display: grid;
      gap: 6px;
    }}
    .channel-card-head h2 {{
      margin: 0;
      font-size: 28px;
      line-height: 1.1;
      color: var(--ink);
    }}
    .channel-card-head p {{
      margin: 0;
      font-size: 14px;
      line-height: 1.7;
      color: var(--muted);
    }}
    .guide-narrative {{
      display: grid;
      gap: 12px;
      margin-bottom: 18px;
      padding: 18px 20px;
      border-radius: 22px;
      border: 1px solid rgba(15, 118, 110, 0.16);
      background: linear-gradient(180deg, rgba(240, 253, 250, 0.88) 0%, rgba(255,255,255,0.94) 100%);
      box-shadow: inset 0 1px 0 rgba(255,255,255,0.66);
    }}
    .guide-narrative-head {{
      display: grid;
      gap: 6px;
    }}
    .guide-narrative-head h3 {{
      margin: 0;
      font-size: 18px;
      line-height: 1.25;
      color: var(--ink);
    }}
    .guide-narrative-head p,
    .guide-narrative-note {{
      margin: 0;
      font-size: 14px;
      line-height: 1.7;
      color: var(--muted-strong);
    }}
    .guide-narrative .project-header-scenes {{
      margin-top: 0;
    }}
    .guide-narrative-note {{
      color: var(--muted);
    }}
    .panel-shell {{
      padding: 0;
      overflow: hidden;
    }}
    .panel-section {{
      padding: 24px 26px;
    }}
    .panel-divider {{
      height: 1px;
      background: linear-gradient(90deg, transparent 0%, rgba(148, 163, 184, 0.16) 12%, rgba(148, 163, 184, 0.16) 88%, transparent 100%);
    }}
    .panel-subsection h3 {{
      margin: 0 0 10px;
      font-size: 18px;
      line-height: 1.2;
      color: var(--ink);
    }}
    .panel-subsection p,
    .panel-subsection li {{
      color: var(--muted);
      font-size: 14px;
      line-height: 1.7;
    }}
    .panel-subsection ul {{
      margin: 0;
      padding-left: 18px;
      display: grid;
      gap: 8px;
    }}
    .guide-shell {{
      padding: 0;
    }}
    .guide-inline-grid {{
      display: grid;
      grid-template-columns: repeat(2, minmax(0, 1fr));
      gap: 18px;
    }}
    .account-layout {{
      display: grid;
      grid-template-columns: minmax(0, 1.05fr) minmax(280px, 0.95fr);
      gap: 24px;
      align-items: start;
    }}
    .account-auth-column,
    .account-copy-column {{
      display: grid;
      gap: 16px;
    }}
    .docs-stage,
    .account-grid > .card {{
      min-height: 100%;
    }}
    .docs-toolbar {{
      display: flex;
      flex-wrap: wrap;
      align-items: center;
      justify-content: space-between;
      gap: 12px;
    }}
    .docs-source-tabs,
    .docs-preview-actions {{
      display: flex;
      flex-wrap: wrap;
      gap: 10px;
    }}
    .docs-source-tabs .mini-btn.is-active {{
      border-color: rgba(15, 118, 110, 0.24);
      color: var(--brand);
      background: linear-gradient(135deg, rgba(236, 254, 255, 0.96) 0%, rgba(255,255,255,0.96) 100%);
    }}
    .docs-source-panel {{
      display: none;
      margin-top: 16px;
      gap: 14px;
    }}
    .docs-source-panel.is-active {{
      display: grid;
    }}
    .docs-source-head {{
      display: flex;
      flex-wrap: wrap;
      align-items: flex-start;
      justify-content: space-between;
      gap: 12px;
    }}
    .docs-source-head .hero-badge {{
      color: var(--brand);
      background: linear-gradient(135deg, rgba(236, 254, 255, 0.96) 0%, rgba(248, 250, 252, 0.98) 100%);
      border-color: rgba(15, 118, 110, 0.18);
      box-shadow: 0 10px 22px rgba(15, 23, 42, 0.06);
    }}
    .docs-source-head h3 {{
      margin: 0;
      font-size: 20px;
      line-height: 1.2;
      color: var(--ink);
    }}
    .docs-source-copy {{
      margin: 6px 0 0;
      font-size: 13px;
      line-height: 1.7;
      color: var(--muted);
    }}
    .docs-sidebar > .card:first-child {{
      position: sticky;
      top: 96px;
    }}
    .docs-sidebar > .card + .card {{
      position: static;
    }}
    @media (max-width: 1080px) {{
      .project-hero,
      .channel-grid,
      .guide-grid,
      .account-grid {{
        grid-template-columns: 1fr;
      }}
      .guide-inline-grid,
      .account-layout {{
        grid-template-columns: 1fr;
      }}
      .project-header {{
        padding: 0;
      }}
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
        padding-top: 12px;
      }}
      .chat-target,
      .auth-session,
      .user-doc-version,
      .user-doc-head {{
        flex-direction: column;
        align-items: stretch;
      }}
      .user-doc-actions {{
        justify-content: flex-start;
      }}
      .docs-sidebar > .card:first-child {{
        position: static;
      }}
    }}
    @media (max-width: 760px) {{
      .wrap {{
        padding: 14px 12px 24px;
      }}
      .project-shell {{
        gap: 14px;
      }}
      .panel-section {{
        padding: 16px;
      }}
      .project-toolbar {{
        top: 8px;
        padding: 12px;
      }}
      .project-header {{
        gap: 10px;
        padding: 0;
      }}
      .project-header-shell:not([open]) .project-header-main {{
        display: none;
      }}
      .project-header-shell[open] .project-header-main {{
        padding: 2px 2px 0;
      }}
      .project-header-main,
      .chat-stage-head-compact,
      .chat-stage-links {{
        flex-direction: column;
        align-items: stretch;
      }}
      .project-header-main {{
        gap: 6px;
      }}
      .project-header-copy {{
        gap: 4px;
      }}
      .project-header-meta {{
        justify-content: flex-start;
        gap: 6px;
      }}
      .channel-nav {{
        position: sticky;
        top: calc(env(safe-area-inset-top, 0px) + 8px);
        z-index: 9;
        width: 100%;
        justify-content: flex-start;
        padding: 5px;
        border-radius: 18px;
        background: rgba(255,255,255,0.92);
        backdrop-filter: blur(16px);
        box-shadow: 0 16px 28px rgba(15, 23, 42, 0.08);
      }}
      .channel-btn {{
        flex: 0 0 auto;
        white-space: nowrap;
      }}
      .channel-stage {{
        gap: 14px;
      }}
      .project-hero-summary h2,
      .channel-card-head h2 {{
        font-size: 22px;
      }}
      .project-header-copy h1 {{
        font-size: 22px;
      }}
      .project-header-summary-copy h1 {{
        font-size: 22px;
        line-height: 1.04;
      }}
      .project-header-copy p {{
        font-size: 13px;
        line-height: 1.55;
      }}
      .project-header-lead {{
        display: -webkit-box;
        -webkit-box-orient: vertical;
        -webkit-line-clamp: 3;
        overflow: hidden;
      }}
      .project-status-chip {{
        min-height: 28px;
        padding: 0 10px;
        font-size: 11px;
      }}
      .hero {{
        border-radius: 22px;
        padding: 22px 18px;
      }}
      .hero h1 {{
        font-size: 30px;
      }}
      .hero-panel h2,
      .chat-stage-head h2 {{
        font-size: 24px;
      }}
      .hero-badges {{
        justify-content: flex-start;
        flex-wrap: nowrap;
        overflow-x: auto;
        padding-bottom: 4px;
        scrollbar-width: none;
      }}
      .hero-badges::-webkit-scrollbar {{
        display: none;
      }}
      .hero-badge {{
        flex: 0 0 auto;
        white-space: nowrap;
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
      .prompt-row {{
        flex-wrap: nowrap;
        overflow-x: auto;
        padding-bottom: 2px;
        scrollbar-width: none;
      }}
      .prompt-row::-webkit-scrollbar {{
        display: none;
      }}
      .prompt-chip {{
        flex: 0 0 auto;
        max-width: 82vw;
      }}
      .chat-feed {{
        min-height: 44svh;
        max-height: min(56svh, 520px);
      }}
      .docs-source-head,
      .docs-toolbar,
      .project-hero-actions {{
        flex-direction: column;
        align-items: stretch;
      }}
      .docs-grid {{
        gap: 14px;
      }}
      .docs-sidebar {{
        gap: 12px;
      }}
      .docs-preview-actions .action-btn,
      .docs-preview-actions .detail-link {{
        width: 100%;
      }}
      .mobile-quickbar {{
        display: grid;
      }}
      .mobile-rail-nav {{
        display: flex;
      }}
      .summary-rail {{
        grid-template-columns: 1fr;
        gap: 12px;
      }}
      .summary-rail > .card[data-mobile-panel] {{
        display: none;
      }}
      .summary-rail > .card[data-mobile-panel].is-mobile-active {{
        display: block;
      }}
      .chat-composer {{
        position: sticky;
        bottom: 0;
        z-index: 4;
        padding-bottom: calc(18px + env(safe-area-inset-bottom, 0px));
        backdrop-filter: blur(14px);
        box-shadow: 0 -12px 24px rgba(15, 23, 42, 0.08);
      }}
      .chat-composer textarea,
      .auth-field input {{
        font-size: 16px;
      }}
      .chat-composer textarea {{
        min-height: 84px;
        padding: 16px 17px 18px;
      }}
      .chat-composer-foot {{
        flex-direction: column;
        align-items: stretch;
      }}
      .composer-note {{
        max-width: none;
      }}
      .composer-actions {{
        width: 100%;
        display: grid;
        grid-template-columns: repeat(2, minmax(0, 1fr));
        gap: 10px;
      }}
      .composer-actions .action-btn {{
        width: 100%;
        min-height: 44px;
      }}
      #avatar-chat-send {{
        grid-column: 1 / -1;
        min-height: 48px;
      }}
      .chat-status,
      .chat-disabled-note {{
        margin-left: 18px;
        margin-right: 18px;
      }}
      .user-doc-preview {{
        max-height: 240px;
      }}
      .guide-narrative {{
        gap: 10px;
        margin-bottom: 16px;
        padding: 16px;
      }}
      .guide-narrative-head h3 {{
        font-size: 17px;
      }}
      .guide-narrative-head p,
      .guide-narrative-note {{
        font-size: 13px;
      }}
      .guide-narrative .project-header-scenes {{
        flex-wrap: nowrap;
        overflow-x: auto;
        padding-bottom: 2px;
        scrollbar-width: none;
      }}
      .guide-narrative .project-header-scenes::-webkit-scrollbar {{
        display: none;
      }}
      .detail-shell > summary,
      .detail-shell-body {{
        padding-left: 18px;
        padding-right: 18px;
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
      .mobile-quickbar {{
        grid-template-columns: 1fr;
      }}
      .meta {{
        grid-template-columns: 1fr;
      }}
      .summary-rail {{
        grid-template-columns: 1fr;
      }}
      .mobile-rail-tab {{
        min-height: 38px;
        padding: 0 12px;
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
  <!-- AGIME_DIGITAL_AVATAR_DEFAULT_TEMPLATE_V6 -->
  <div class="wrap">
    <div class="project-shell">
      <section class="project-header">
        <details class="project-header-shell" id="avatar-header-shell">
          <summary class="project-header-summary">
            <div class="project-header-summary-copy">
              <p class="project-header-kicker">{header_kicker}</p>
              <h1>{title}</h1>
            </div>
            <span class="project-header-summary-action">展开说明</span>
          </summary>
          <div class="project-header-main">
            <div class="project-header-copy">
              <p class="project-header-lead">{header_intro}</p>
            </div>
            <div class="project-header-meta">
              <span class="project-status-chip">{avatar_type}</span>
              <span class="project-status-chip">{entry_state_badge}</span>
              {preview_badge}
            </div>
          </div>
        </details>
        <nav class="channel-nav" aria-label="数字分身频道">
          <button type="button" class="channel-btn is-active" data-channel-button="chat">对话</button>
          <button type="button" class="channel-btn" data-channel-button="docs">资料</button>
          <button type="button" class="channel-btn" data-channel-button="guide">说明</button>
          <button type="button" class="channel-btn" data-channel-button="account">账号</button>
        </nav>
      </section>

      {preview_banner}

      <div class="channel-stage">
        <section class="channel-panel is-active" data-channel-panel="chat" id="avatar-channel-chat">
          <section class="card chat-stage chat-stage-wide" id="avatar-chat-stage">
            <div class="chat-status" id="avatar-chat-status" hidden>
              <p class="chat-status-title" id="avatar-chat-status-title"></p>
              <p class="chat-status-detail" id="avatar-chat-status-detail"></p>
            </div>
            <div class="chat-feed" id="avatar-chat-feed"></div>
            <div class="chat-composer">
              <div class="prompt-row" id="avatar-chat-prompts">{prompt_buttons}</div>
              <div class="chat-target" id="avatar-chat-target" hidden>
                <div class="chat-target-copy">
                  <span class="chat-target-label">当前处理目标</span>
                  <strong id="avatar-chat-target-name">未指定</strong>
                  <p id="avatar-chat-target-meta">当你上传多份资料或开放多份平台资料时，可以先指定本轮要处理的文档。</p>
                </div>
                <button type="button" class="action-btn" id="avatar-chat-target-clear">取消指定</button>
              </div>
              <textarea id="avatar-chat-input" rows="1" placeholder="{chat_placeholder}"{send_disabled_attr}></textarea>
              <div class="chat-composer-foot">
                <p class="composer-note" id="avatar-chat-runtime-note">{runtime_note}</p>
                <div class="composer-actions">
                  <button type="button" class="action-btn" data-open-channel="docs">上传/选择资料</button>
                  <button type="button" class="action-btn action-btn-voice" id="avatar-chat-voice">语音输入</button>
                  <button type="button" class="action-btn" id="avatar-chat-clear"{clear_disabled_attr}>清空记录</button>
                  <button type="button" class="action-btn" id="avatar-chat-stop" disabled>停止生成</button>
                  <button type="button" class="action-btn action-btn-primary" id="avatar-chat-send"{send_disabled_attr}>发送</button>
                </div>
              </div>
            </div>
            {chat_disabled_notice}
          </section>
        </section>

        <section class="channel-panel" data-channel-panel="docs" id="avatar-channel-docs">
          <div class="channel-grid docs-grid">
            <section class="card docs-stage">
              <div class="channel-card-head">
                <p class="section-eyebrow">资料频道</p>
                <h2>平台资料与我的上传</h2>
                <p>平台资料用于参考，你自己的上传文档只对当前访客或当前外部用户可见。选中后可以直接设为本轮处理目标。</p>
              </div>

              <input type="file" id="avatar-user-doc-input" hidden />

              <div class="docs-toolbar">
                <div class="docs-source-tabs">
                  <button type="button" class="mini-btn is-active" data-docs-source-button="platform">平台资料</button>
                  <button type="button" class="mini-btn" data-docs-source-button="uploads">我的上传</button>
                </div>
                <div class="docs-preview-actions">
                  <button type="button" class="action-btn" id="avatar-user-doc-trigger"{send_disabled_attr}>上传资料</button>
                  <button type="button" class="action-btn" id="avatar-user-doc-refresh"{send_disabled_attr}>刷新上传</button>
                </div>
              </div>

              <section class="docs-source-panel is-active" data-docs-source-panel="platform">
                <div class="docs-source-head">
                  <div>
                    <h3>平台资料</h3>
                    <p class="docs-source-copy">这些资料由服务方开放给当前分身使用，可作为问答和处理的参考范围。</p>
                  </div>
                  <span class="hero-badge">{docs_count} 份开放资料</span>
                </div>
                <div class="user-doc-list" id="avatar-platform-doc-list">正在加载平台资料…</div>
              </section>

              <section class="docs-source-panel" data-docs-source-panel="uploads">
                <div class="docs-source-head">
                  <div>
                    <h3 id="avatar-user-doc-heading">仅当前匿名访客可见</h3>
                    <p class="docs-source-copy" id="avatar-user-doc-copy">上传资料会进入独立的“用户上传文档”区域，不会落到团队常规文档目录。换浏览器或换设备后不会自动同步。</p>
                  </div>
                </div>
                <div class="user-doc-list" id="avatar-user-doc-list">正在准备上传区…</div>
              </section>
            </section>

            <aside class="docs-sidebar">
              <section class="card" id="avatar-doc-preview-card">
                <p class="section-eyebrow">资料预览</p>
                <h2 id="avatar-user-doc-preview-title">先从左侧选择资料</h2>
                <p class="hint" id="avatar-user-doc-preview-meta">平台资料和你的上传都可以在这里预览，并设为当前处理目标。</p>
                <pre class="user-doc-preview" id="avatar-user-doc-preview-body">尚未选择文档。</pre>
                <div class="docs-preview-actions">
                  <button type="button" class="action-btn action-btn-primary" id="avatar-preview-chat-jump"{send_disabled_attr}>基于当前资料对话</button>
                </div>
              </section>

              <section class="card">
                <p class="section-eyebrow">资料说明</p>
                <h2>选择资料后再回到对话</h2>
                <p>当你需要明确处理哪一份文档时，先在这里预览并设为当前目标，再回到对话频道发起修改或问答。</p>
              </section>
            </aside>
          </div>
        </section>

        <section class="channel-panel" data-channel-panel="guide" id="avatar-channel-guide">
          <section class="card panel-shell guide-shell">
            <div class="panel-section">
              <div class="channel-card-head">
                <p class="section-eyebrow">说明频道</p>
                <h2>这里帮助你快速了解我适合处理什么，以及我会如何使用资料</h2>
                <p>首页只保留聊天入口。这里负责说明处理范围、资料边界、文档处理方式，以及第一次使用时最容易上手的方法。</p>
              </div>
              <section class="guide-narrative">
                <div class="guide-narrative-head">
                  <h3>这个分身主要帮你处理这些事</h3>
                  <p>{header_intro}</p>
                </div>
                <div class="project-header-scenes">{header_use_cases_html}</div>
                <p class="guide-narrative-note">如何配合：{header_working_style} {header_cta_hint}</p>
              </section>
              <div class="guide-inline-grid">
                <section class="panel-subsection good">
                  <h3>我能帮你做什么</h3>
                  <ul>{capability_items}</ul>
                </section>
                <section class="panel-subsection warn">
                  <h3>哪些事情我不会直接去做</h3>
                  <ul>{boundary_items}</ul>
                </section>
              </div>
            </div>

            <div class="panel-divider"></div>

            <div class="panel-section">
              <div class="guide-inline-grid">
                <section class="panel-subsection">
                  <h3>第一次使用时怎么描述任务</h3>
                  <p>建议直接说目标、问题、期望结果和限制条件。我会先在当前边界内推进，再告诉你下一步如何继续。</p>
                  <ul>{example_items}</ul>
                </section>
                <section class="panel-subsection">
                  <h3>当前资料与使用方式摘要</h3>
                  <div class="metric-grid">
                    <div class="metric-card"><div class="sub-title">文档权限</div><p>{doc_mode}</p></div>
                    <div class="metric-card"><div class="sub-title">平台资料</div><p>{docs_summary}</p></div>
                    <div class="metric-card"><div class="sub-title">个人上传</div><p>{upload_summary}</p></div>
                    <div class="metric-card"><div class="sub-title">身份同步</div><p>{sync_summary}</p></div>
                  </div>
                  <p class="hint">如果你上传了多份资料，可以先在资料频道选中本轮要处理的文档，再回到对话频道继续。</p>
                </section>
              </div>
            </div>

            <div class="panel-divider"></div>

            <div class="panel-section">
              <div class="channel-card-head">
                <p class="section-eyebrow">详细说明</p>
                <h2>常见问题与资料范围</h2>
                <p>如果你需要进一步确认资料边界、文档处理方式，或者上传与登录后的差异，可以展开查看。</p>
              </div>
              <details id="avatar-details" class="detail-shell">
                <summary>展开常见问题与资料范围</summary>
                <div class="detail-shell-body">
                  <div class="layout">
                    <div class="stack">
                      <section class="card">
                        <h2>我会怎么处理资料与文档</h2>
                        <p>我会先使用当前开放的平台资料，以及你自己上传并选中的资料来处理问题。需要继续修改文档时，会按照当前文档模式执行。</p>
                        <div class="boundary-grid">{boundary_cards}</div>
                        <div class="cta">
                          <strong>处理原则</strong>
                          {chat_entry_note}
                        </div>
                      </section>
                    </div>

                    <div class="stack">
                      <section class="card">
                        <h2>当前可参考的平台资料</h2>
                        <p>这些资料会作为当前分身可引用的公开范围。你自己的上传资料只会在“资料”频道里按当前身份单独展示。</p>
                        <div class="docs">
                          <div data-doc-list>正在加载文档清单...</div>
                        </div>
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
          </section>
        </section>

        <section class="channel-panel" data-channel-panel="account" id="avatar-channel-account">
          <section class="card panel-shell account-shell">
            <div class="panel-section">
              <div class="channel-card-head">
                <p class="section-eyebrow">账号频道</p>
                <h2 id="avatar-auth-heading">当前以匿名访客模式使用</h2>
                <p id="avatar-auth-copy">你现在可以直接对话和上传资料。登录后，当前浏览器里的匿名资料会迁移到你的外部用户身份，便于在其他设备继续使用。</p>
              </div>
              <div class="account-layout">
                <div class="account-auth-column">
                  <div class="auth-session" id="avatar-auth-session" hidden>
                    <div class="auth-session-main">
                      <span class="auth-badge" id="avatar-auth-badge">已登录</span>
                      <strong id="avatar-auth-name">外部用户</strong>
                      <p id="avatar-auth-meta">你的上传资料和后续会话会归属到同一身份。</p>
                    </div>
                    <button type="button" class="mini-btn" id="avatar-auth-logout">退出登录</button>
                  </div>
                  <div class="auth-switch" id="avatar-auth-switch">
                    <button type="button" class="mini-btn primary is-active" data-auth-mode="login">登录</button>
                    <button type="button" class="mini-btn" data-auth-mode="register">注册</button>
                  </div>
                  <form class="auth-form" id="avatar-auth-form">
                    <label class="auth-field">
                      <span>用户名</span>
                      <input type="text" id="avatar-auth-username" autocomplete="username" placeholder="请输入用户名" />
                    </label>
                    <label class="auth-field auth-field-optional" id="avatar-auth-display-name-field" hidden>
                      <span>显示名称</span>
                      <input type="text" id="avatar-auth-display-name" autocomplete="nickname" placeholder="注册时可填写，便于识别" />
                    </label>
                    <label class="auth-field auth-field-optional" id="avatar-auth-phone-field" hidden>
                      <span>手机号</span>
                      <input type="tel" id="avatar-auth-phone" autocomplete="tel" placeholder="可选，便于后续联系" />
                    </label>
                    <label class="auth-field">
                      <span>密码</span>
                      <input type="password" id="avatar-auth-password" autocomplete="current-password" placeholder="请输入密码" />
                    </label>
                    <div class="auth-form-actions">
                      <button type="submit" class="action-btn action-btn-primary" id="avatar-auth-submit">登录并继续</button>
                    </div>
                  </form>
                  <p class="hint auth-status" id="avatar-auth-status">登录后，你上传的资料会归属到外部用户身份；匿名模式下只保留在当前浏览器身份中。</p>
                </div>

                <div class="account-copy-column">
                  <section class="panel-subsection">
                    <h3>为什么登录</h3>
                    <ul>
                      <li>登录后，你上传的资料会归到外部用户身份，不再只保留在当前浏览器里。</li>
                      <li>换电脑、换手机或清空浏览器后，只要重新登录同一账号，就能继续查看资料。</li>
                      <li>匿名模式下也能直接开始对话，适合临时体验或一次性任务。</li>
                    </ul>
                  </section>
                  <section class="panel-subsection">
                    <h3>当前模式说明</h3>
                    <p>匿名模式适合临时处理；如果你需要长期保存上传资料、跨设备继续协作，建议在开始前注册或登录。</p>
                    <div class="docs-preview-actions">
                      <button type="button" class="action-btn" data-open-channel="chat" data-focus-chat>直接开始对话</button>
                      <button type="button" class="action-btn" data-open-channel="docs">先去上传资料</button>
                    </div>
                  </section>
                </div>
              </div>
            </div>
          </section>
        </section>
      </div>

      <footer class="project-footer">
        <div class="project-footer-links">
          <a class="project-footer-link" href="{footer_website_url}" target="_blank" rel="noopener noreferrer">
            <svg viewBox="0 0 24 24" fill="none" aria-hidden="true">
              <path d="M12 3a9 9 0 1 0 0 18a9 9 0 0 0 0-18Zm0 0c2.2 2.1 3.5 5.3 3.5 9s-1.3 6.9-3.5 9m0-18C9.8 5.1 8.5 8.3 8.5 12s1.3 6.9 3.5 9m-8.5-9h17M4.7 7.5h14.6M4.7 16.5h14.6" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"/>
            </svg>
            <span>{footer_website_label}</span>
          </a>
          <a class="project-footer-link" href="{footer_github_url}" target="_blank" rel="noopener noreferrer">
            <svg viewBox="0 0 24 24" fill="none" aria-hidden="true">
              <path d="M9 19c-4.5 1.4-4.5-2.2-6-2.7m12 5.4v-3.5a3 3 0 0 0-.8-2.2c2.6-.3 5.3-1.3 5.3-5.8A4.6 4.6 0 0 0 18.3 7A4.2 4.2 0 0 0 18.2 4s-1-.3-3.2 1.2a11 11 0 0 0-6 0C6.8 3.7 5.8 4 5.8 4A4.2 4.2 0 0 0 5.7 7 4.6 4.6 0 0 0 4.5 10.2c0 4.5 2.7 5.5 5.3 5.8A3 3 0 0 0 9 18.2v3.5" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"/>
            </svg>
            <span>GitHub</span>
          </a>
        </div>
        <p class="project-footer-copy">© {footer_year} Agime Team · Powered by Agime Team</p>
      </footer>
    </div>
  </div>

  {runtime_assets}
</body>
</html>"##,
            title = title,
            header_kicker = header_kicker,
            avatar_type = avatar_type,
            header_intro = Self::escape_html(&header_intro),
            header_use_cases_html = Self::render_project_header_use_cases(&header_use_cases),
            header_working_style = Self::escape_html(&header_working_style),
            header_cta_hint = Self::escape_html(&header_cta_hint),
            doc_mode = Self::resolve_doc_mode_label_for_mode(effective_doc_mode),
            docs_count = docs_count,
            docs_summary = Self::escape_html(&docs_summary),
            upload_summary = Self::escape_html(&upload_summary),
            sync_summary = Self::escape_html(&sync_summary),
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
            chat_entry_note = Self::escape_html(chat_entry_note),
            runtime_assets = runtime_assets,
            footer_year = footer_year,
            footer_website_url = footer_website_url,
            footer_website_label = footer_website_label,
            footer_github_url = footer_github_url,
            preview_badge = if preview_mode {
                r#"<span class="project-status-chip">管理预览</span>"#
            } else {
                ""
            },
            entry_state_badge = if runtime_chat_interactive {
                "可直接对话"
            } else if preview_mode {
                "管理预览"
            } else {
                "当前仅说明模式"
            },
            preview_banner = if preview_mode {
                r#"<div class="preview-banner"><strong>管理预览模式</strong>当前页面用于内部验收：重点核对对话主区、边界说明和详细信息是否按预期展示。请勿直接将该入口发送给外部访客。</div>"#
            } else {
                ""
            },
        )
    }

    pub fn render_digital_avatar_index_html(portal: &Portal) -> String {
        Self::default_digital_avatar_index_html(portal, false, None, None)
    }

    pub fn render_digital_avatar_preview_html(portal: &Portal) -> String {
        Self::default_digital_avatar_index_html(portal, true, None, None)
    }

    pub fn render_digital_avatar_index_html_with_effective(
        portal: &Portal,
        effective: &PortalEffectivePublicConfig,
        team_name: Option<&str>,
    ) -> String {
        Self::default_digital_avatar_index_html(portal, false, Some(effective), team_name)
    }

    pub fn render_digital_avatar_preview_html_with_effective(
        portal: &Portal,
        effective: &PortalEffectivePublicConfig,
        team_name: Option<&str>,
    ) -> String {
        Self::default_digital_avatar_index_html(portal, true, Some(effective), team_name)
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
            Self::default_digital_avatar_index_html(portal, false, None, None)
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
