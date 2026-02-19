//! Portal service for MongoDB — CRUD, publish/unpublish, interactions

use crate::db::{collections, MongoDb};
use crate::models::{
    AgentExtensionConfig, AgentSkillConfig, BuiltinExtension, CustomExtensionConfig,
};
use crate::models::mongo::{
    CreatePortalRequest, PaginatedResponse, Portal, PortalInteraction, PortalStatus,
    PortalSummary, UpdatePortalRequest,
};
use anyhow::{anyhow, Result};
use chrono::Utc;
use futures::TryStreamExt;
use mongodb::bson::{doc, oid::ObjectId};
use mongodb::options::FindOptions;
use serde::Deserialize;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Deserialize)]
struct TeamAgentPolicyDoc {
    pub agent_id: String,
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
    if !slug.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-') {
        return Err(anyhow!("Slug may only contain lowercase letters, digits, and hyphens"));
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

    fn normalize_agent_id(value: Option<&str>) -> Option<String> {
        value
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(ToString::to_string)
    }

    fn resolve_coding_agent_id(portal: &Portal) -> Option<String> {
        Self::normalize_agent_id(portal.coding_agent_id.as_deref())
            .or_else(|| Self::normalize_agent_id(portal.agent_id.as_deref()))
            .or_else(|| Self::normalize_agent_id(portal.service_agent_id.as_deref()))
    }

    fn resolve_service_agent_id(portal: &Portal) -> Option<String> {
        Self::normalize_agent_id(portal.service_agent_id.as_deref())
            .or_else(|| Self::normalize_agent_id(portal.agent_id.as_deref()))
            .or_else(|| Self::normalize_agent_id(portal.coding_agent_id.as_deref()))
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

        // If an agent is specified, ensure it exists in this team even when allowlists are empty.
        if let Some(agent_id) = normalized_agent_id {
            let _ = self.load_team_agent_policy(team_id, agent_id).await?;
        }

        if !has_extension_constraints && !has_skill_constraints {
            return Ok(());
        }

        let agent_id = normalized_agent_id
            .ok_or_else(|| anyhow!("agent_id is required when setting allowlists"))?;

        let agent = self.load_team_agent_policy(team_id, agent_id).await?;

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

    // -----------------------------------------------------------------------
    // Portal CRUD
    // -----------------------------------------------------------------------

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
            tags,
            settings,
        } = req;

        let name = raw_name.trim().to_string();
        if name.is_empty() || name.len() > 200 {
            return Err(anyhow!("Portal name must be between 1 and 200 characters"));
        }

        let team_oid = ObjectId::parse_str(team_id)?;
        let now = Utc::now();
        let legacy_agent_id = Self::normalize_agent_id(raw_agent_id.as_deref());
        let mut coding_agent_id = Self::normalize_agent_id(raw_coding_agent_id.as_deref())
            .or_else(|| legacy_agent_id.clone());
        let mut service_agent_id = Self::normalize_agent_id(raw_service_agent_id.as_deref())
            .or_else(|| legacy_agent_id.clone());
        if coding_agent_id.is_none() {
            coding_agent_id = service_agent_id.clone();
        }
        if service_agent_id.is_none() {
            service_agent_id = coding_agent_id.clone();
        }
        let effective_agent_enabled = agent_enabled.unwrap_or(false);
        if effective_agent_enabled && service_agent_id.is_none() {
            return Err(anyhow!(
                "service_agent_id is required when agent_enabled is true"
            ));
        }

        self
            .validate_policy_scope(team_id, coding_agent_id.as_deref(), None, None)
            .await?;
        self
            .validate_policy_scope(
                team_id,
                service_agent_id.as_deref(),
                allowed_extensions.as_ref(),
                allowed_skill_ids.as_ref(),
            )
            .await?;

        let slug = match raw_slug {
            Some(s) if !s.is_empty() => {
                validate_slug(&s)?;
                self.ensure_slug_available(&s).await?;
                s
            }
            _ => self.generate_slug(&name).await?,
        };

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
            tags: tags.unwrap_or_default(),
            settings: settings.unwrap_or(serde_json::Value::Object(Default::default())),
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
        coll.find_one(
            doc! { "slug": slug, "is_deleted": { "$ne": true } },
            None,
        )
        .await?
        .ok_or_else(|| anyhow!("Portal not found"))
    }

    pub async fn list(
        &self,
        team_id: &str,
        page: u64,
        limit: u64,
    ) -> Result<PaginatedResponse<PortalSummary>> {
        let team_oid = ObjectId::parse_str(team_id)?;
        let coll = self.db.collection::<Portal>(collections::PORTALS);
        let filter = doc! { "team_id": team_oid, "is_deleted": { "$ne": true } };

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
            tags,
            settings,
        } = req;

        let team_oid = ObjectId::parse_str(team_id)?;
        let portal_oid = ObjectId::parse_str(portal_id)?;
        let current = self.get(team_id, portal_id).await?;

        let legacy_update = req_legacy_agent_id.clone();
        let coding_update = req_coding_agent_id.or_else(|| legacy_update.clone());
        let service_update = req_service_agent_id.or_else(|| legacy_update.clone());

        let effective_coding_agent_id = match coding_update.as_ref() {
            Some(v) => Self::normalize_agent_id(v.as_deref()),
            None => Self::resolve_coding_agent_id(&current),
        };
        let effective_service_agent_id = match service_update.as_ref() {
            Some(v) => Self::normalize_agent_id(v.as_deref()),
            None => Self::resolve_service_agent_id(&current),
        };

        let clearing_service_agent = matches!(service_update.as_ref(), Some(None));
        let effective_allowed_extensions = if clearing_service_agent && allowed_extensions.is_none() {
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
        let effective_agent_enabled = agent_enabled.unwrap_or(current.agent_enabled);
        if effective_agent_enabled && effective_service_agent_id.is_none() {
            return Err(anyhow!(
                "service_agent_id is required when agent_enabled is true"
            ));
        }

        self
            .validate_policy_scope(
                team_id,
                effective_coding_agent_id.as_deref(),
                None,
                None,
            )
            .await?;
        self
            .validate_policy_scope(
                team_id,
                effective_service_agent_id.as_deref(),
                effective_allowed_extensions,
                effective_allowed_skill_ids,
            )
            .await?;

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
            let old_ids: HashSet<&str> = current.bound_document_ids.iter().map(|s| s.as_str()).collect();
            let new_set: HashSet<&str> = new_ids.iter().map(|s| s.as_str()).collect();

            let doc_svc = crate::services::mongo::DocumentService::new(self.db.clone());
            let folder_svc = crate::services::mongo::FolderService::new(self.db.clone());

            // Newly added docs → mark public + move to /公共文档
            let added: Vec<&str> = new_set.difference(&old_ids).copied().collect();
            if !added.is_empty() {
                folder_svc.ensure_system_folder(team_id, "公共文档", "/").await.ok();
                for doc_id in &added {
                    doc_svc.set_public(team_id, doc_id, true).await.ok();
                    doc_svc.move_to_folder(team_id, doc_id, "/公共文档").await.ok();
                }
            }

            // Removed docs → unmark if no other portal references them
            let removed: Vec<&str> = old_ids.difference(&new_set).copied().collect();
            for doc_id in &removed {
                let refs = self.find_portals_by_document_id(team_id, doc_id).await.unwrap_or_default();
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
        let result = coll
            .update_one(
                doc! { "_id": portal_oid, "team_id": team_oid, "is_deleted": { "$ne": true } },
                doc! { "$set": {
                    "status": "published",
                    "published_at": bson::DateTime::from_chrono(now),
                    "updated_at": bson::DateTime::from_chrono(now),
                }},
                None,
            )
            .await?;
        if result.matched_count == 0 {
            return Err(anyhow!("Portal not found"));
        }
        self.get(team_id, portal_id).await
    }

    pub async fn unpublish(&self, team_id: &str, portal_id: &str) -> Result<Portal> {
        let team_oid = ObjectId::parse_str(team_id)?;
        let portal_oid = ObjectId::parse_str(portal_id)?;
        let coll = self.db.collection::<Portal>(collections::PORTALS);
        let now = Utc::now();
        let result = coll
            .update_one(
                doc! { "_id": portal_oid, "team_id": team_oid, "is_deleted": { "$ne": true } },
                doc! { "$set": {
                    "status": "draft",
                    "updated_at": bson::DateTime::from_chrono(now),
                }},
                None,
            )
            .await?;
        if result.matched_count == 0 {
            return Err(anyhow!("Portal not found"));
        }
        self.get(team_id, portal_id).await
    }

    /// Set the project_path for a portal (called after creating the project folder)
    pub async fn set_project_path(&self, team_id: &str, portal_id: &str, project_path: &str) -> Result<()> {
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
        let path = workspace_abs
            .join("portals")
            .join(team_id)
            .join(slug);
        Self::normalize_project_path(path)
    }

    fn default_portal_index_html(name: &str) -> String {
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
            name, name
        )
    }

    fn write_portal_agent_scaffold(project_path: &str, slug: &str) -> Result<()> {
        let client_js = r#"// Portal SDK (generated by server scaffold)
// Unified client for chat, documents, data storage, config, and tracking.

class PortalSDK {
  constructor({ slug }) {
    this.slug = slug;
    this.visitorId = localStorage.getItem('portal_vid');
    if (!this.visitorId) {
      this.visitorId = 'v_' + Math.random().toString(36).substr(2, 9);
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
      method: 'PUT', headers: { 'Content-Type': 'application/json' },
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

## Usage
```html
<script src="portal-sdk.js"></script>
<script>
const sdk = new PortalSDK({{{{ slug: '{slug}' }}}});
</script>
```

## Chat
- `sdk.chat.createSession()` / `sendMessage(sid, text)` / `subscribe(sid)` / `cancel(sid)` / `listSessions()`

## Documents (read-only, bound)
- `sdk.docs.list()` / `get(docId)` / `getMeta(docId)` / `poll(docId, ms, cb)`

## Data Storage (_private/ key-value)
- `sdk.data.list()` / `get(key)` / `set(key, value)`

## Config & Tracking
- `sdk.config.get()` / `sdk.track(type, payload)`
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
        portal_id: &str,
        slug: &str,
        portal_name: &str,
        workspace_root: &str,
    ) -> Result<String> {
        let raw_project_path = Self::compute_project_path(workspace_root, team_id, slug);
        let base = Path::new(&raw_project_path);
        std::fs::create_dir_all(base)?;
        let project_path = Self::normalize_project_path(base.to_path_buf());
        let base = Path::new(&project_path);
        std::fs::write(base.join("index.html"), Self::default_portal_index_html(portal_name))?;
        Self::write_portal_agent_scaffold(&project_path, slug)?;
        self.set_project_path(team_id, portal_id, &project_path).await?;
        Ok(project_path)
    }

    // -----------------------------------------------------------------------
    // Slug helpers
    // -----------------------------------------------------------------------

    pub async fn generate_slug(&self, name: &str) -> Result<String> {
        let base = slugify(name);
        let base = if base.is_empty() {
            "portal".to_string()
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
                .find_one(doc! { "slug": &candidate, "is_deleted": { "$ne": true } }, None)
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
            return Err(anyhow!("Visitor ID exceeds maximum length of 64 characters"));
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
    ) -> Result<PaginatedResponse<PortalInteraction>> {
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
        let items: Vec<PortalInteraction> = cursor.try_collect().await?;

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
                        .and_then(|d| d.get_i32("count").ok())
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
    pub async fn find_portals_by_document_id(
        &self,
        team_id: &str,
        doc_id: &str,
    ) -> Result<Vec<PortalSummary>> {
        let team_oid = ObjectId::parse_str(team_id)?;
        let coll = self.db.collection::<Portal>(collections::PORTALS);
        let filter = doc! {
            "team_id": team_oid,
            "is_deleted": { "$ne": true },
            "bound_document_ids": doc_id,
        };
        let cursor = coll.find(filter, None).await?;
        let portals: Vec<Portal> = cursor.try_collect().await?;
        Ok(portals.into_iter().map(PortalSummary::from).collect())
    }
}
