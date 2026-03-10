//! Portal service for MongoDB — CRUD, publish/unpublish, interactions

use crate::db::{collections, MongoDb};
use crate::models::mongo::{
    CreatePortalRequest, PaginatedResponse, Portal, PortalDomain, PortalInteraction,
    PortalInteractionResponse, PortalStatus, PortalSummary, UpdatePortalRequest,
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
use std::collections::HashSet;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Deserialize)]
struct TeamAgentPolicyDoc {
    pub agent_id: String,
    #[serde(default)]
    pub agent_domain: Option<String>,
    #[serde(default)]
    pub agent_role: Option<String>,
    #[serde(default)]
    pub enabled_extensions: Vec<AgentExtensionConfig>,
    #[serde(default)]
    pub custom_extensions: Vec<CustomExtensionConfig>,
    #[serde(default)]
    pub assigned_skills: Vec<AgentSkillConfig>,
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
                    .partial_filter_expression(doc! { "is_deleted": { "$ne": true } })
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
            .or_else(|| Self::normalize_agent_id(portal.agent_id.as_deref()))
            ;
        if explicit.is_some() {
            return explicit;
        }
        if Self::resolve_portal_domain(portal) == PortalDomain::Avatar {
            return None;
        }
        Self::normalize_agent_id(portal.service_agent_id.as_deref())
    }

    fn resolve_service_agent_id(portal: &Portal) -> Option<String> {
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
        let Some(service_agent_id) = service_agent_id.map(str::trim).filter(|s| !s.is_empty()) else {
            return Ok(());
        };

        if portal_domain != PortalDomain::Ecosystem {
            return Ok(());
        }

        let agent = self.load_team_agent_policy(team_id, service_agent_id).await?;
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

        if agent_domain.is_empty() || agent_domain == "general" {
            return Err(anyhow!(
                "ecosystem portals cannot bind a general agent directly as service_agent_id; clone it into an ecosystem service agent first"
            ));
        }

        if agent_domain == "digital_avatar" && agent_role == "manager" {
            return Err(anyhow!(
                "digital avatar manager agents cannot be used as ecosystem portal service agents"
            ));
        }

        if agent_domain == "digital_avatar" {
            if agent_role == "service" {
                return Ok(());
            }
            return Err(anyhow!(
                "only digital avatar service agents can be shared into ecosystem portals"
            ));
        }

        if agent_domain == "ecosystem_portal" {
            if agent_role.is_empty() || agent_role == "service" {
                return Ok(());
            }
            return Err(anyhow!(
                "ecosystem portal service agents must use the service role"
            ));
        }

        Err(anyhow!(
            "unsupported service agent domain '{}'; use an ecosystem service agent or a digital avatar service agent",
            agent_domain
        ))
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
        let coding_update = req_coding_agent_id.clone().or_else(|| legacy_update.clone());
        let service_update = req_service_agent_id.clone().or_else(|| legacy_update.clone());

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
        if touches_avatar_agent_binding && effective_agent_enabled && effective_coding_agent_id.is_none() {
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
            r#"<!DOCTYPE html>
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
</html>"#,
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
        match portal.document_access_mode {
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
        items.iter()
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
        match portal.document_access_mode {
            crate::models::mongo::PortalDocumentAccessMode::ReadOnly => {
                items.push("当前仅支持读取与检索，不会改写文档内容。".to_string());
            }
            crate::models::mongo::PortalDocumentAccessMode::CoEditDraft => {
                items.push("当前仅允许在协作草稿范围内写入，不会直接影响正式文档。".to_string());
            }
            crate::models::mongo::PortalDocumentAccessMode::ControlledWrite => {
                items.push("若涉及写入，会走受控写入与校验流程，不会跳过策略。".to_string());
            }
        }
        items
            .into_iter()
            .map(|item| format!("<li>{}</li>", item))
            .collect::<Vec<_>>()
            .join("")
    }

    fn render_avatar_example_prompts(portal: &Portal) -> String {
        let examples = if portal
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
        };

        examples
            .into_iter()
            .map(|item| format!("<li>{}</li>", Self::escape_html(item)))
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
                "如需写入，会经过受控写入和校验流程，不会跳过规则直接落库。".to_string()
            }
        };
        let escalation = "若当前能力或权限不足，会先说明缺口，再交由管理 Agent 决策，不会自行越权。";
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
                "不会直接跳过规则写入。涉及写入时会走受控写入和校验流程。".to_string()
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
            ("如果当前能力不够，你会怎么处理？", escalate_answer.to_string()),
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
        markers.iter().filter(|marker| html.contains(**marker)).count() >= 2
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

    #[allow(clippy::too_many_lines)]
    fn default_digital_avatar_index_html(portal: &Portal, preview_mode: bool) -> String {
        let title = Self::escape_html(&portal.name);
        let subtitle = Self::escape_html(
            portal
                .description
                .as_deref()
                .unwrap_or("这是一个面向真实业务协作场景的数字分身。"),
        );
        let avatar_type = Self::resolve_avatar_type_label(portal);
        let run_mode = Self::resolve_run_mode_label(portal);
        let doc_mode = Self::resolve_doc_mode_label(portal);
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
        let view_mode_items = Self::render_avatar_view_mode_cards(preview_mode);
        let capability_items = Self::render_capability_items(portal);
        let boundary_items = Self::render_avatar_boundary_items(portal);
        let boundary_cards = Self::render_avatar_boundary_cards(portal);
        let example_items = Self::render_avatar_example_prompts(portal);
        let faq_items = Self::render_avatar_faq_items(portal);
        let allowed_extensions = Self::render_badge_list(
            portal.allowed_extensions.as_deref().unwrap_or(&[]),
            "当前未开放额外扩展",
        );
        let allowed_skills = Self::render_badge_list(
            portal.allowed_skill_ids.as_deref().unwrap_or(&[]),
            "当前未开放专用技能",
        );
        let docs_count = portal.bound_document_ids.len();

        format!(
            r#"<!DOCTYPE html>
<html lang="zh-CN">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>{title}</title>
  <style>
    :root {{
      --bg: #f6f8fc;
      --card: #ffffff;
      --text: #0f172a;
      --muted: #475569;
      --line: #e2e8f0;
      --brand: #0f766e;
      --brand-soft: #ecfeff;
      --warn: #9a3412;
      --warn-soft: #fff7ed;
      --ink: #0b1220;
    }}
    * {{ box-sizing: border-box; }}
    body {{
      margin: 0;
      background:
        radial-gradient(circle at 0% 0%, rgba(45, 212, 191, 0.18) 0%, transparent 34%),
        linear-gradient(180deg, #f8fbff 0%, var(--bg) 100%);
      color: var(--text);
      font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", "PingFang SC", "Microsoft Yahei", sans-serif;
      line-height: 1.6;
    }}
    .wrap {{ max-width: 1120px; margin: 0 auto; padding: 28px 18px 42px; }}
    .hero {{
      background: linear-gradient(130deg, #0f172a 0%, #1e293b 65%, #155e75 100%);
      color: #f8fafc;
      border-radius: 16px;
      padding: 24px 24px;
      box-shadow: 0 14px 40px rgba(15, 23, 42, 0.22);
    }}
    .hero-top {{
      display: flex;
      flex-wrap: wrap;
      gap: 10px;
      align-items: center;
      justify-content: space-between;
    }}
    .hero h1 {{ margin: 0 0 8px; font-size: 30px; line-height: 1.18; }}
    .hero p {{ margin: 0; color: #cbd5e1; }}
    .hero-badges {{ display: flex; flex-wrap: wrap; gap: 8px; }}
    .hero-badge {{
      display: inline-flex;
      align-items: center;
      gap: 6px;
      border-radius: 999px;
      padding: 6px 10px;
      font-size: 12px;
      color: #dbeafe;
      background: rgba(15, 118, 110, 0.2);
      border: 1px solid rgba(148, 163, 184, 0.28);
    }}
    .meta {{
      margin-top: 14px;
      display: grid;
      grid-template-columns: repeat(auto-fit, minmax(150px, 1fr));
      gap: 10px;
    }}
    .meta-item {{
      background: rgba(15, 118, 110, 0.16);
      border: 1px solid rgba(148, 163, 184, 0.3);
      border-radius: 12px;
      padding: 10px 12px;
    }}
    .meta-label {{ font-size: 12px; color: #cbd5e1; }}
    .meta-value {{ margin-top: 2px; font-size: 14px; font-weight: 600; }}
    .layout {{
      display: grid;
      grid-template-columns: minmax(0, 1.6fr) minmax(280px, 0.9fr);
      gap: 14px;
      margin-top: 16px;
    }}
    .stack {{ display: grid; gap: 14px; min-width: 0; }}
    .grid {{
      display: grid;
      grid-template-columns: repeat(auto-fit, minmax(280px, 1fr));
      gap: 14px;
    }}
    .card {{
      background: var(--card);
      border: 1px solid var(--line);
      border-radius: 14px;
      padding: 16px;
      box-shadow: 0 10px 26px rgba(15, 23, 42, 0.06);
    }}
    .card h2 {{ margin: 0 0 8px; font-size: 17px; }}
    .card p {{ margin: 6px 0; color: var(--muted); }}
    ul {{ margin: 8px 0 0 18px; padding: 0; }}
    li {{ margin: 6px 0; color: var(--muted); }}
    .good {{ background: var(--brand-soft); border-color: #99f6e4; }}
    .warn {{ background: var(--warn-soft); border-color: #fdba74; }}
    .section-title {{
      margin: 20px 0 10px;
      font-size: 17px;
      font-weight: 700;
    }}
    .sub-title {{
      margin: 0 0 8px;
      font-size: 14px;
      font-weight: 700;
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
      border-radius: 12px;
      border: 1px solid var(--line);
      background: #f8fafc;
      padding: 12px;
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
      font-weight: 700;
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
      padding: 4px 8px;
      border-radius: 999px;
      background: rgba(15, 118, 110, 0.12);
      color: var(--brand);
      border: 1px solid rgba(15, 118, 110, 0.18);
      font-size: 11px;
      font-weight: 700;
      white-space: nowrap;
    }}
    .tag-chip {{
      display: inline-flex;
      align-items: center;
      padding: 5px 10px;
      border-radius: 999px;
      background: var(--brand-soft);
      border: 1px solid #a5f3fc;
      color: #155e75;
      font-size: 12px;
      font-weight: 600;
    }}
    .empty-badge {{
      display: inline-flex;
      align-items: center;
      padding: 5px 10px;
      border-radius: 999px;
      background: #f8fafc;
      border: 1px dashed var(--line);
      color: var(--muted);
      font-size: 12px;
    }}
    .steps {{
      display: grid;
      gap: 10px;
      margin-top: 8px;
    }}
    .step {{
      display: grid;
      grid-template-columns: 26px minmax(0, 1fr);
      gap: 10px;
      align-items: start;
    }}
    .step-no {{
      width: 26px;
      height: 26px;
      border-radius: 999px;
      background: #ecfeff;
      color: var(--brand);
      font-size: 12px;
      font-weight: 700;
      display: inline-flex;
      align-items: center;
      justify-content: center;
      border: 1px solid #99f6e4;
    }}
    .step p {{ margin: 0; }}
    .docs {{
      margin-top: 10px;
      border-top: 1px solid var(--line);
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
      border-radius: 12px;
      border: 1px solid var(--line);
      background: #f8fafc;
      padding: 12px;
    }}
    .faq-q {{
      margin: 0 0 6px;
      font-size: 14px;
      font-weight: 700;
      color: var(--ink);
    }}
    .faq-a {{
      margin: 0;
      font-size: 13px;
      color: var(--muted);
    }}
    .cta {{
      margin-top: 12px;
      padding: 12px 14px;
      border-radius: 12px;
      background: #0f172a;
      color: #f8fafc;
    }}
    .cta strong {{ display: block; margin-bottom: 4px; }}
    .preview-banner {{
      margin-top: 14px;
      padding: 12px 14px;
      border-radius: 12px;
      border: 1px solid #99f6e4;
      background: rgba(236, 254, 255, 0.92);
      color: #155e75;
    }}
    .preview-banner strong {{
      display: block;
      margin-bottom: 4px;
      color: #0f172a;
    }}
    @media (max-width: 880px) {{
      .layout {{ grid-template-columns: 1fr; }}
      .hero h1 {{ font-size: 26px; }}
    }}
  </style>
</head>
<body>
  <!-- AGIME_DIGITAL_AVATAR_DEFAULT_TEMPLATE_V3 -->
  <div class="wrap">
    <section class="hero">
      <div class="hero-top">
        <div>
          <h1>{title}</h1>
          <p>{subtitle}</p>
        </div>
        <div class="hero-badges">
          <span class="hero-badge">数字分身</span>
          <span class="hero-badge">{avatar_type}</span>
          <span class="hero-badge">由管理 Agent 治理</span>
          {preview_badge}
        </div>
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
          <p class="sub-title">允许扩展</p>
          <div class="tag-row">{allowed_extensions}</div>
          <p class="sub-title" style="margin-top: 14px;">允许技能</p>
          <div class="tag-row">{allowed_skills}</div>
        </section>

        <section class="card">
          <h2>已绑定文档</h2>
          <p>以下文档会作为当前分身的可访问知识范围展示给你。</p>
          <div class="docs">
            <div id="doc-list">正在加载文档清单...</div>
          </div>
        </section>

        <section class="card">
          <h2>使用提示</h2>
          <p>页面右下角为对话入口。你可以直接提出目标、问题或需要处理的事项。</p>
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

  <script>
  (function () {{
    async function loadDocs() {{
      var holder = document.getElementById('doc-list');
      if (!holder) return;
      try {{
        var res = await fetch('./api/docs');
        if (!res.ok) throw new Error(String(res.status));
        var data = await res.json();
        var docs = Array.isArray(data.documents) ? data.documents : [];
        if (docs.length === 0) {{
          holder.textContent = '当前未绑定文档。';
          return;
        }}
        holder.innerHTML = docs.map(function (doc) {{
          var name = String(doc.name || doc.id || '未命名文档');
          var size = doc.file_size ? ('（' + doc.file_size + ' B）') : '';
          return '<div>- ' + name + size + '</div>';
        }}).join('');
      }} catch (_err) {{
        holder.textContent = '文档清单加载失败，请稍后重试。';
      }}
    }}
    loadDocs();
  }})();
  </script>
</body>
</html>"#,
            title = title,
            subtitle = subtitle,
            avatar_type = avatar_type,
            run_mode = run_mode,
            doc_mode = doc_mode,
            docs_count = docs_count,
            extensions_count = portal.allowed_extensions.as_ref().map_or(0, Vec::len),
            skills_count = portal.allowed_skill_ids.as_ref().map_or(0, Vec::len),
            manager_note = manager_note,
            optimize_note = optimize_note,
            view_mode_items = view_mode_items,
            capability_items = capability_items,
            boundary_items = boundary_items,
            boundary_cards = boundary_cards,
            example_items = example_items,
            faq_items = faq_items,
            allowed_extensions = allowed_extensions,
            allowed_skills = allowed_skills,
            preview_badge = if preview_mode {
                r#"<span class="hero-badge">管理预览</span>"#
            } else {
                ""
            },
            preview_banner = if preview_mode {
                r#"<div class="preview-banner"><strong>管理预览模式</strong>当前页面用于内部验收：重点核对文档边界、能力说明、FAQ 与对话入口是否符合预期。请勿直接将该入口发送给外部访客。</div>"#
            } else {
                ""
            },
        )
    }

    pub fn render_digital_avatar_index_html(portal: &Portal) -> String {
        Self::default_digital_avatar_index_html(portal, false)
    }

    pub fn render_digital_avatar_preview_html(portal: &Portal) -> String {
        Self::default_digital_avatar_index_html(portal, true)
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
            Self::default_digital_avatar_index_html(portal, false)
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
}
