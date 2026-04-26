//! Skill Registry tools — discover and import remote skills into the team skill library.
//!
//! Phase 1 scope:
//! - search skills via skills.sh
//! - preview a GitHub-backed skill package
//! - import a GitHub-backed skill package into Mongo skills

use agime::agents::mcp_client::McpClientTrait;
use agime_team::db::MongoDb;
use agime_team::models::mongo::{Skill, SkillFile};
use agime_team::services::mongo::SkillService;
use agime_team::services::package_service::PackageService;
use anyhow::{anyhow, Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use futures::TryStreamExt;
use mongodb::bson::{doc, oid::ObjectId};
use reqwest::Client;
use reqwest::Url;
use rmcp::model::*;
use rmcp::ServiceError;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::time::timeout;
use tokio_util::sync::CancellationToken;

const SKILLS_SH_SEARCH_URL: &str = "https://skills.sh/api/search";
const SKILLS_SH_ALL_TIME_URL: &str = "https://skills.sh/";
const SKILLS_SH_TRENDING_URL: &str = "https://skills.sh/trending";
const SKILLS_SH_HOT_URL: &str = "https://skills.sh/hot";
const DEFAULT_SEARCH_LIMIT: u64 = 10;
const MAX_SEARCH_LIMIT: u64 = 50;
const MAX_PREVIEW_SKILL_MD_BYTES: usize = 32 * 1024;
const HTTP_CONNECT_TIMEOUT_SECS: u64 = 8;
const HTTP_REQUEST_TIMEOUT_SECS: u64 = 25;
const PACKAGE_RESOLVE_TIMEOUT_SECS: u64 = 45;
const MAX_PACKAGE_FILES: usize = 80;
const MAX_PACKAGE_FILE_BYTES: usize = 256 * 1024;
const MAX_PACKAGE_TOTAL_BYTES: usize = 1024 * 1024;

fn build_skill_ref(skill_id: &str, name: &str, skill_class: &str, meta: &str) -> String {
    format!("[[skill:{}|{}|{}|{}]]", skill_id, name, skill_class, meta)
}

fn build_registry_display_line_zh(skill_ref: &str, source: &str) -> String {
    format!("{}（skills.sh registry，来源{}）", skill_ref, source)
}

fn build_registry_display_line_en(skill_ref: &str, source: &str) -> String {
    format!("{} (skills.sh registry, source {})", skill_ref, source)
}

fn build_registry_plain_line_zh(name: &str, source: &str) -> String {
    format!("{}（skills.sh registry，来源{}）", name, source)
}

fn build_registry_plain_line_en(name: &str, source: &str) -> String {
    format!("{} (skills.sh registry, source {})", name, source)
}

fn build_local_display_line_zh(skill_ref: &str, source: &str) -> String {
    format!("{}（本地来源，路径{}）", skill_ref, source)
}

fn build_local_display_line_en(skill_ref: &str, source: &str) -> String {
    format!("{} (local source, path {})", skill_ref, source)
}

fn build_local_plain_line_zh(name: &str, source: &str) -> String {
    format!("{}（本地来源，路径{}）", name, source)
}

fn build_local_plain_line_en(name: &str, source: &str) -> String {
    format!("{} (local source, path {})", name, source)
}

fn build_imported_display_line_zh(skill_ref: &str, version: &str) -> String {
    format!("{}（已导入 registry，v{}）", skill_ref, version)
}

fn build_imported_display_line_en(skill_ref: &str, version: &str) -> String {
    format!("{} (imported registry skill, v{})", skill_ref, version)
}

fn build_imported_plain_line_zh(name: &str, version: &str) -> String {
    format!("{}（已导入 registry，v{}）", name, version)
}

fn build_imported_plain_line_en(name: &str, version: &str) -> String {
    format!("{} (imported registry skill, v{})", name, version)
}

#[derive(Clone)]
pub struct SkillRegistryToolsProvider {
    db: Arc<MongoDb>,
    client: Client,
    team_id: String,
    actor_id: String,
    workspace_root: Option<PathBuf>,
    info: InitializeResult,
}

#[derive(Debug, Deserialize)]
struct SkillsShSearchResponse {
    #[serde(default)]
    skills: Vec<SkillsShSearchItem>,
}

#[derive(Debug, Deserialize)]
struct SkillsShSearchItem {
    id: String,
    #[serde(rename = "skillId")]
    skill_id: String,
    name: String,
    #[serde(default)]
    installs: u64,
    source: String,
    #[serde(rename = "isDuplicate", default)]
    is_duplicate: bool,
}

#[derive(Debug, Deserialize)]
struct SkillsShPopularItem {
    source: String,
    #[serde(rename = "skillId")]
    skill_id: String,
    name: String,
    #[serde(default)]
    installs: u64,
    #[serde(rename = "installsYesterday", default)]
    installs_yesterday: Option<u64>,
    #[serde(default)]
    change: Option<i64>,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
enum PopularListMode {
    AllTime,
    Trending,
    Hot,
}

impl PopularListMode {
    fn parse(raw: Option<&str>) -> Result<Self> {
        match raw
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or("all_time")
            .to_ascii_lowercase()
            .as_str()
        {
            "all" | "all_time" | "all-time" | "alltime" => Ok(Self::AllTime),
            "trending" => Ok(Self::Trending),
            "hot" => Ok(Self::Hot),
            other => Err(anyhow!(
                "Unsupported mode '{}'. Use all_time, trending, or hot.",
                other
            )),
        }
    }

    fn page_url(self) -> &'static str {
        match self {
            Self::AllTime => SKILLS_SH_ALL_TIME_URL,
            Self::Trending => SKILLS_SH_TRENDING_URL,
            Self::Hot => SKILLS_SH_HOT_URL,
        }
    }
}

#[derive(Debug, Deserialize)]
struct GitHubRepoResponse {
    default_branch: String,
}

#[derive(Debug, Deserialize)]
struct GitHubTreeResponse {
    sha: String,
    #[serde(default)]
    tree: Vec<GitHubTreeItem>,
    #[serde(default)]
    truncated: bool,
}

#[derive(Debug, Deserialize, Clone)]
struct GitHubTreeItem {
    path: String,
    #[serde(rename = "type")]
    item_type: String,
    sha: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GitHubContentResponse {
    content: String,
    encoding: String,
}

#[derive(Debug, Clone)]
struct SkillInstallSpec {
    owner: String,
    repo: String,
    skill_id: Option<String>,
    source_ref: Option<String>,
    skill_dir: Option<String>,
    local_path: Option<PathBuf>,
    raw: String,
}

impl SkillInstallSpec {
    fn source(&self) -> String {
        if let Some(path) = &self.local_path {
            return path.to_string_lossy().to_string();
        }
        format!("{}/{}", self.owner, self.repo)
    }

    fn canonical_for_skill(&self, skill_id: &str) -> String {
        if let Some(path) = &self.local_path {
            return build_local_install_spec(path, Some(skill_id));
        }
        format!("{}/{}@{}", self.owner, self.repo, skill_id)
    }
}

#[derive(Debug, Clone, Serialize)]
struct SkillCandidate {
    skill_id: String,
    name: String,
    source: String,
    source_ref: String,
    skill_dir: String,
    install_spec: String,
    skills_sh_url: String,
    skill_md_path: String,
}

enum SkillResolution {
    Resolved(ImportedSkillPackage),
    Multiple {
        install_spec: String,
        source: String,
        source_ref: String,
        candidate_count: usize,
        candidates: Vec<SkillCandidate>,
    },
    NotFound {
        install_spec: String,
        source: String,
        source_ref: String,
        message: String,
    },
}

#[derive(Debug, Clone)]
struct ImportedSkillPackage {
    source_type: String,
    source_repo: String,
    owner: String,
    repo: String,
    skill_id: String,
    skill_dir: String,
    install_spec: String,
    source_url: String,
    source_ref: String,
    source_commit: String,
    source_tree_sha: String,
    skill_md: String,
    description: String,
    body: String,
    tags: Vec<String>,
    files: Vec<SkillFile>,
    skipped_files: Vec<String>,
}

struct LocalSkillSource {
    root: PathBuf,
    cleanup_root: Option<PathBuf>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct ImportedSkillMetadata {
    source_type: String,
    source_repo: String,
    source_skill_path: String,
    install_spec: Option<String>,
    source_url: Option<String>,
    source_ref: String,
    source_commit: Option<String>,
    source_tree_sha: Option<String>,
    import_mode: Option<String>,
    registry_provider: Option<String>,
    import_actor: Option<String>,
    skipped_files: Option<Vec<String>>,
    visibility_override: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct ImportedRegistrySkillSummary {
    imported_skill_id: String,
    name: String,
    skill_ref: String,
    display_line_zh: String,
    display_line_en: String,
    plain_line_zh: String,
    plain_line_en: String,
    skill_class: String,
    description: Option<String>,
    version: String,
    visibility: String,
    source: String,
    skill_id: String,
    source_ref: String,
    source_commit: Option<String>,
    source_tree_sha: Option<String>,
    source_url: Option<String>,
    registry_provider: Option<String>,
    skipped_files: Vec<String>,
    updated_at: String,
}

#[derive(Debug, Clone, Serialize)]
struct RemoteUpdateInspection {
    imported_skill_id: String,
    name: String,
    skill_ref: String,
    display_line_zh: String,
    display_line_en: String,
    plain_line_zh: String,
    plain_line_en: String,
    skill_class: String,
    current_version: String,
    description: Option<String>,
    source: String,
    skill_id: String,
    source_ref: String,
    current_tree_sha: String,
    latest_tree_sha: String,
    latest_source_commit: String,
    has_update: bool,
    owner: String,
    repo: String,
}

impl SkillRegistryToolsProvider {
    pub fn new(db: Arc<MongoDb>, team_id: String, actor_id: String) -> Self {
        let info = InitializeResult {
            protocol_version: ProtocolVersion::V_2025_03_26,
            capabilities: ServerCapabilities {
                tools: Some(ToolsCapability {
                    list_changed: Some(false),
                }),
                resources: None,
                prompts: None,
                completions: None,
                experimental: None,
                logging: None,
                extensions: None,
                tasks: None,
            },
            server_info: Implementation {
                name: "skill_registry".to_string(),
                title: Some("Skill Registry".to_string()),
                version: "1.0.0".to_string(),
                description: None,
                icons: None,
                website_url: Some("https://skills.sh/".to_string()),
            },
            instructions: Some(
                "Discover remote skills from skills.sh. Start with list_popular_skills or search_skills, then use preview_skill to inspect a candidate before import_skill_to_team. Use list_imported_registry_skills and check_skill_updates to manage what is already installed."
                    .to_string(),
            ),
        };
        Self {
            db,
            client: Client::builder()
                .user_agent("agime-team-server/skill-registry")
                .connect_timeout(Duration::from_secs(HTTP_CONNECT_TIMEOUT_SECS))
                .timeout(Duration::from_secs(HTTP_REQUEST_TIMEOUT_SECS))
                .build()
                .expect("skill registry http client"),
            team_id,
            actor_id,
            workspace_root: None,
            info,
        }
    }

    pub fn with_workspace_root(mut self, workspace_root: Option<String>) -> Self {
        self.workspace_root = workspace_root
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .map(PathBuf::from);
        self
    }

    fn service(&self) -> SkillService {
        SkillService::new((*self.db).clone())
    }

    pub async fn search_registry(
        &self,
        query: &str,
        limit: Option<u64>,
    ) -> Result<serde_json::Value> {
        let mut args = JsonObject::new();
        args.insert(
            "query".to_string(),
            serde_json::Value::String(query.to_string()),
        );
        if let Some(limit) = limit {
            args.insert("limit".to_string(), serde_json::Value::Number(limit.into()));
        }
        Ok(serde_json::from_str(&self.handle_search(&args).await?)?)
    }

    pub async fn list_popular_registry_skills(
        &self,
        mode: Option<&str>,
        limit: Option<u64>,
    ) -> Result<serde_json::Value> {
        let mut args = JsonObject::new();
        if let Some(mode) = mode {
            args.insert(
                "mode".to_string(),
                serde_json::Value::String(mode.to_string()),
            );
        }
        if let Some(limit) = limit {
            args.insert("limit".to_string(), serde_json::Value::Number(limit.into()));
        }
        Ok(serde_json::from_str(
            &self.handle_list_popular(&args).await?,
        )?)
    }

    pub async fn preview_registry_skill(
        &self,
        source: &str,
        skill_id: &str,
        source_ref: Option<&str>,
    ) -> Result<serde_json::Value> {
        let mut args = JsonObject::new();
        args.insert(
            "source".to_string(),
            serde_json::Value::String(source.to_string()),
        );
        args.insert(
            "skill_id".to_string(),
            serde_json::Value::String(skill_id.to_string()),
        );
        if let Some(source_ref) = source_ref {
            args.insert(
                "source_ref".to_string(),
                serde_json::Value::String(source_ref.to_string()),
            );
        }
        Ok(serde_json::from_str(&self.handle_preview(&args).await?)?)
    }

    pub async fn preview_registry_install_spec(
        &self,
        install_spec: &str,
    ) -> Result<serde_json::Value> {
        let mut args = JsonObject::new();
        args.insert(
            "install_spec".to_string(),
            serde_json::Value::String(install_spec.to_string()),
        );
        Ok(serde_json::from_str(&self.handle_preview(&args).await?)?)
    }

    pub async fn import_registry_skill(
        &self,
        source: &str,
        skill_id: &str,
        source_ref: Option<&str>,
        visibility: Option<&str>,
    ) -> Result<serde_json::Value> {
        let mut args = JsonObject::new();
        args.insert(
            "source".to_string(),
            serde_json::Value::String(source.to_string()),
        );
        args.insert(
            "skill_id".to_string(),
            serde_json::Value::String(skill_id.to_string()),
        );
        if let Some(source_ref) = source_ref {
            args.insert(
                "source_ref".to_string(),
                serde_json::Value::String(source_ref.to_string()),
            );
        }
        if let Some(visibility) = visibility {
            args.insert(
                "visibility".to_string(),
                serde_json::Value::String(visibility.to_string()),
            );
        }
        Ok(serde_json::from_str(&self.handle_import(&args).await?)?)
    }

    pub async fn import_registry_install_spec(
        &self,
        install_spec: &str,
        visibility: Option<&str>,
    ) -> Result<serde_json::Value> {
        let mut args = JsonObject::new();
        args.insert(
            "install_spec".to_string(),
            serde_json::Value::String(install_spec.to_string()),
        );
        if let Some(visibility) = visibility {
            args.insert(
                "visibility".to_string(),
                serde_json::Value::String(visibility.to_string()),
            );
        }
        Ok(serde_json::from_str(&self.handle_import(&args).await?)?)
    }

    pub async fn check_registry_updates(
        &self,
        imported_skill_id: Option<&str>,
    ) -> Result<serde_json::Value> {
        let mut args = JsonObject::new();
        if let Some(imported_skill_id) = imported_skill_id {
            args.insert(
                "imported_skill_id".to_string(),
                serde_json::Value::String(imported_skill_id.to_string()),
            );
        }
        Ok(serde_json::from_str(
            &self.handle_check_updates(&args).await?,
        )?)
    }

    pub async fn list_imported_registry_skills(&self) -> Result<serde_json::Value> {
        Ok(serde_json::from_str(
            &self.handle_list_imported(&JsonObject::new()).await?,
        )?)
    }

    pub async fn upgrade_registry_skill(
        &self,
        imported_skill_id: &str,
        force: bool,
    ) -> Result<serde_json::Value> {
        let mut args = JsonObject::new();
        args.insert(
            "imported_skill_id".to_string(),
            serde_json::Value::String(imported_skill_id.to_string()),
        );
        if force {
            args.insert("force".to_string(), serde_json::Value::Bool(true));
        }
        Ok(serde_json::from_str(&self.handle_upgrade(&args).await?)?)
    }

    fn tool_definitions() -> Vec<Tool> {
        vec![
            Tool {
                name: "list_popular_skills".into(),
                title: None,
                description: Some(
                    "List popular skills from the skills.sh leaderboard. Use this for all_time, trending, or hot top skills before previewing or importing. When `skill_ref` / `display_line_zh` are present, Chinese answers must preserve them exactly only for concrete result items. In explanatory prose, generic format descriptions, or slash-separated examples, use `plain_line_zh` or the plain skill name instead of emitting `skill_ref`."
                        .into(),
                ),
                input_schema: serde_json::from_value(json!({
                    "type": "object",
                    "properties": {
                        "mode": {
                            "type": "string",
                            "description": "Leaderboard mode: all_time, trending, or hot"
                        },
                        "limit": {
                            "type": "integer",
                            "description": "Max results (default 10, max 50)"
                        }
                    },
                    "additionalProperties": false
                }))
                .unwrap_or_default(),
                output_schema: None,
                annotations: None,
                execution: None,
                icons: None,
                meta: None,
            },
            Tool {
                name: "search_skills".into(),
                title: None,
                description: Some(
                    "Search skills.sh for installable skills and return concise metadata. When `skill_ref` / `display_line_zh` are present, Chinese answers must preserve them exactly only for concrete result items. In explanatory prose, generic format descriptions, or slash-separated examples, use `plain_line_zh` or the plain skill name instead of `skill_ref`.".into(),
                ),
                input_schema: serde_json::from_value(json!({
                    "type": "object",
                    "properties": {
                        "query": { "type": "string", "description": "Search query" },
                        "limit": {
                            "type": "integer",
                            "description": "Max results (default 10, max 50)"
                        }
                    },
                    "required": ["query"],
                    "additionalProperties": false
                }))
                .unwrap_or_default(),
                output_schema: None,
                annotations: None,
                execution: None,
                icons: None,
                meta: None,
            },
            Tool {
                name: "preview_skill".into(),
                title: None,
                description: Some(
                    "Preview a GitHub-backed skill package before importing it into the team library. When `skill_ref` / `display_line_zh` are present, Chinese answers must preserve them exactly only for concrete preview/result items. In explanatory prose, use `plain_line_zh` or the plain skill name instead of `skill_ref`."
                        .into(),
                ),
                input_schema: serde_json::from_value(json!({
                    "type": "object",
                    "properties": {
                        "install_spec": {
                            "type": "string",
                            "description": "Optional standard install spec such as owner/repo@skill, a skills.sh URL, or a GitHub repo/tree URL. Prefer this when available."
                        },
                        "source": {
                            "type": "string",
                            "description": "Source repo from search result, usually owner/repo"
                        },
                        "skill_id": {
                            "type": "string",
                            "description": "skills.sh skillId value"
                        },
                        "source_ref": {
                            "type": "string",
                            "description": "Optional branch, tag, or commit override"
                        }
                    },
                    "additionalProperties": false
                }))
                .unwrap_or_default(),
                output_schema: None,
                annotations: None,
                execution: None,
                icons: None,
                meta: None,
            },
            Tool {
                name: "import_skill_to_team".into(),
                title: None,
                description: Some(
                    "Import a GitHub-backed skill package into this team's shared skills library. When `skill_ref` / `display_line_zh` are present, Chinese answers must preserve them exactly only for concrete result items. In explanatory prose, use `plain_line_zh` or the plain skill name instead of `skill_ref`."
                        .into(),
                ),
                input_schema: serde_json::from_value(json!({
                    "type": "object",
                    "properties": {
                        "install_spec": {
                            "type": "string",
                            "description": "Optional standard install spec such as owner/repo@skill, a skills.sh URL, or a GitHub repo/tree URL. Prefer this when available."
                        },
                        "source": {
                            "type": "string",
                            "description": "Source repo from search result, usually owner/repo"
                        },
                        "skill_id": {
                            "type": "string",
                            "description": "skills.sh skillId value"
                        },
                        "source_ref": {
                            "type": "string",
                            "description": "Optional branch, tag, or commit override"
                        },
                        "visibility": {
                            "type": "string",
                            "description": "Optional skill visibility override (default team)"
                        }
                    },
                    "additionalProperties": false
                }))
                .unwrap_or_default(),
                output_schema: None,
                annotations: None,
                execution: None,
                icons: None,
                meta: None,
            },
            Tool {
                name: "check_skill_updates".into(),
                title: None,
                description: Some(
                    "Check imported registry skills for upstream updates without modifying the team library. When `skill_ref` / `display_line_zh` are present, Chinese answers must preserve them exactly only for concrete result items. In explanatory prose, use `plain_line_zh` or the plain skill name instead of `skill_ref`."
                        .into(),
                ),
                input_schema: serde_json::from_value(json!({
                    "type": "object",
                    "properties": {
                        "imported_skill_id": {
                            "type": "string",
                            "description": "Optional imported team skill id to check; omitted checks all imported registry skills"
                        }
                    },
                    "additionalProperties": false
                }))
                .unwrap_or_default(),
                output_schema: None,
                annotations: None,
                execution: None,
                icons: None,
                meta: None,
            },
            Tool {
                name: "list_imported_registry_skills".into(),
                title: None,
                description: Some(
                    "List team skills that were previously imported from the external skill registry. When `skill_ref` / `display_line_zh` are present, Chinese answers must preserve them exactly only for concrete result items. In explanatory prose, use `plain_line_zh` or the plain skill name instead of `skill_ref`."
                        .into(),
                ),
                input_schema: serde_json::from_value(json!({
                    "type": "object",
                    "properties": {},
                    "additionalProperties": false
                }))
                .unwrap_or_default(),
                output_schema: None,
                annotations: None,
                execution: None,
                icons: None,
                meta: None,
            },
            Tool {
                name: "upgrade_imported_skill".into(),
                title: None,
                description: Some(
                    "Upgrade an already imported registry skill in place when upstream content changed. When `skill_ref` / `display_line_zh` are present, Chinese answers must preserve them exactly only for concrete result items. In explanatory prose, use `plain_line_zh` or the plain skill name instead of `skill_ref`."
                        .into(),
                ),
                input_schema: serde_json::from_value(json!({
                    "type": "object",
                    "properties": {
                        "imported_skill_id": {
                            "type": "string",
                            "description": "Existing team skill id created by import_skill_to_team"
                        },
                        "force": {
                            "type": "boolean",
                            "description": "Upgrade even when no upstream diff is detected"
                        }
                    },
                    "required": ["imported_skill_id"],
                    "additionalProperties": false
                }))
                .unwrap_or_default(),
                output_schema: None,
                annotations: None,
                execution: None,
                icons: None,
                meta: None,
            },
        ]
    }

    async fn handle_list_popular(&self, args: &JsonObject) -> Result<String> {
        let mode = PopularListMode::parse(args.get("mode").and_then(|v| v.as_str()))?;
        let limit = args
            .get("limit")
            .and_then(|v| v.as_u64())
            .unwrap_or(DEFAULT_SEARCH_LIMIT)
            .clamp(1, MAX_SEARCH_LIMIT);

        let html = self
            .client
            .get(mode.page_url())
            .send()
            .await
            .with_context(|| format!("failed to query leaderboard page {}", mode.page_url()))?
            .error_for_status()
            .with_context(|| {
                format!(
                    "skills.sh leaderboard page returned non-success status for {}",
                    mode.page_url()
                )
            })?
            .text()
            .await
            .context("failed to read skills.sh leaderboard page")?;

        let skills = extract_initial_skills(&html)?
            .into_iter()
            .take(limit as usize)
            .enumerate()
            .map(|(idx, item)| {
                let github_source = parse_github_source(&item.source).is_some();
                let skill_ref = build_skill_ref(
                    &format!("registry:{}", item.skill_id),
                    &item.name,
                    "registry",
                    &item.source,
                );
                let install_spec = build_install_spec(&item.source, &item.skill_id);
                let skills_sh_url = build_skills_sh_url(&item.source, &item.skill_id);
                json!({
                    "rank": idx + 1,
                    "skill_id": item.skill_id,
                    "name": item.name,
                    "skill_ref": skill_ref,
                    "install_spec": install_spec,
                    "skills_sh_url": skills_sh_url,
                    "display_line_zh": build_registry_display_line_zh(&skill_ref, &item.source),
                    "display_line_en": build_registry_display_line_en(&skill_ref, &item.source),
                    "plain_line_zh": build_registry_plain_line_zh(&item.name, &item.source),
                    "plain_line_en": build_registry_plain_line_en(&item.name, &item.source),
                    "skill_class": "registry",
                    "source": item.source,
                    "installs": item.installs,
                    "installs_yesterday": item.installs_yesterday,
                    "change": item.change,
                    "supports_preview": github_source,
                    "supports_import": github_source,
                })
            })
            .collect::<Vec<_>>();

        Ok(json!({
            "team_id": self.team_id,
            "mode": mode,
            "count": skills.len(),
            "skills": skills,
        })
        .to_string())
    }

    async fn handle_search(&self, args: &JsonObject) -> Result<String> {
        let query = args
            .get("query")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|q| !q.is_empty())
            .ok_or_else(|| anyhow!("query is required"))?;
        let limit = args
            .get("limit")
            .and_then(|v| v.as_u64())
            .unwrap_or(DEFAULT_SEARCH_LIMIT)
            .clamp(1, MAX_SEARCH_LIMIT);

        let response = self
            .client
            .get(SKILLS_SH_SEARCH_URL)
            .query(&[("q", query)])
            .send()
            .await
            .context("failed to query skills.sh")?
            .error_for_status()
            .context("skills.sh search returned non-success status")?
            .json::<SkillsShSearchResponse>()
            .await
            .context("failed to decode skills.sh search response")?;

        let skills = response
            .skills
            .into_iter()
            .take(limit as usize)
            .enumerate()
            .map(|(idx, item)| {
                let github_source = parse_github_source(&item.source).is_some();
                let skill_ref = build_skill_ref(
                    &format!("registry:{}", item.skill_id),
                    &item.name,
                    "registry",
                    &item.source,
                );
                let install_spec = build_install_spec(&item.source, &item.skill_id);
                let skills_sh_url = build_skills_sh_url(&item.source, &item.skill_id);
                json!({
                    "rank": idx + 1,
                    "id": item.id,
                    "skill_id": item.skill_id,
                    "name": item.name,
                    "skill_ref": skill_ref,
                    "install_spec": install_spec,
                    "skills_sh_url": skills_sh_url,
                    "display_line_zh": build_registry_display_line_zh(&skill_ref, &item.source),
                    "display_line_en": build_registry_display_line_en(&skill_ref, &item.source),
                    "plain_line_zh": build_registry_plain_line_zh(&item.name, &item.source),
                    "plain_line_en": build_registry_plain_line_en(&item.name, &item.source),
                    "skill_class": "registry",
                    "source": item.source,
                    "installs": item.installs,
                    "is_duplicate": item.is_duplicate,
                    "supports_preview": github_source,
                    "supports_import": github_source,
                })
            })
            .collect::<Vec<_>>();

        Ok(json!({
            "query": query,
            "team_id": self.team_id,
            "count": skills.len(),
            "skills": skills,
        })
        .to_string())
    }

    async fn handle_preview(&self, args: &JsonObject) -> Result<String> {
        let resolution = timeout(
            Duration::from_secs(PACKAGE_RESOLVE_TIMEOUT_SECS),
            self.resolve_skill_request(args),
        )
        .await
        .map_err(|_| {
            anyhow!(
                "Timed out previewing registry skill after {}s. The upstream GitHub package may be too large or slow; try another skill or retry later.",
                PACKAGE_RESOLVE_TIMEOUT_SECS
            )
        })??;
        let package = match resolution {
            SkillResolution::Resolved(package) => package,
            SkillResolution::Multiple {
                install_spec,
                source,
                source_ref,
                candidate_count,
                candidates,
            } => {
                return Ok(json!({
                    "team_id": self.team_id,
                    "resolution_status": "multiple_candidates",
                    "install_spec": install_spec,
                    "source": source,
                    "source_ref": source_ref,
                    "candidate_count": candidate_count,
                    "candidates": candidates,
                    "files": [],
                    "skipped_files": [],
                    "message": "Multiple skills were found in this repository. Choose one candidate and preview/import its install_spec.",
                })
                .to_string());
            }
            SkillResolution::NotFound {
                install_spec,
                source,
                source_ref,
                message,
            } => {
                return Ok(json!({
                    "team_id": self.team_id,
                    "resolution_status": "not_found",
                    "install_spec": install_spec,
                    "source": source,
                    "source_ref": source_ref,
                    "candidate_count": 0,
                    "candidates": [],
                    "files": [],
                    "skipped_files": [],
                    "message": message,
                })
                .to_string());
            }
        };
        let exists = self
            .service()
            .check_duplicate_name(&self.team_id, &package.skill_id, None)
            .await?;

        let mut preview = package.skill_md.clone();
        let truncated = preview.len() > MAX_PREVIEW_SKILL_MD_BYTES;
        if truncated {
            preview = preview
                .chars()
                .take(MAX_PREVIEW_SKILL_MD_BYTES)
                .collect::<String>();
        }

        let files = package
            .files
            .iter()
            .map(|file| json!({ "path": file.path }))
            .collect::<Vec<_>>();
        let source = if package.source_type == "local_skill_source" {
            package.source_repo.clone()
        } else {
            format!("{}/{}", package.owner, package.repo)
        };
        let skill_ref = build_skill_ref(
            &format!("registry:{}", package.skill_id),
            &package.skill_id,
            "registry",
            &source,
        );
        let (display_line_zh, display_line_en, plain_line_zh, plain_line_en) =
            if package.source_type == "local_skill_source" {
                (
                    build_local_display_line_zh(&skill_ref, &source),
                    build_local_display_line_en(&skill_ref, &source),
                    build_local_plain_line_zh(&package.skill_id, &source),
                    build_local_plain_line_en(&package.skill_id, &source),
                )
            } else {
                (
                    build_registry_display_line_zh(&skill_ref, &source),
                    build_registry_display_line_en(&skill_ref, &source),
                    build_registry_plain_line_zh(&package.skill_id, &source),
                    build_registry_plain_line_en(&package.skill_id, &source),
                )
            };

        Ok(json!({
            "team_id": self.team_id,
            "resolution_status": "resolved",
            "source": source,
            "skill_id": package.skill_id,
            "install_spec": package.install_spec,
            "skills_sh_url": package.source_url,
            "source_url": package.source_url,
            "candidate_count": 1,
            "candidates": [],
            "skill_ref": skill_ref,
            "display_line_zh": display_line_zh,
            "display_line_en": display_line_en,
            "plain_line_zh": plain_line_zh,
            "plain_line_en": plain_line_en,
            "skill_class": "registry",
            "source_ref": package.source_ref,
            "source_commit": package.source_commit,
            "skill_dir": package.skill_dir,
            "name": package.skill_id,
            "description": package.description,
            "tags": package.tags,
            "already_imported": exists,
            "skill_md": preview,
            "truncated": truncated,
            "files": files,
            "skipped_files": package.skipped_files,
        })
        .to_string())
    }

    async fn handle_import(&self, args: &JsonObject) -> Result<String> {
        let resolution = timeout(
            Duration::from_secs(PACKAGE_RESOLVE_TIMEOUT_SECS),
            self.resolve_skill_request(args),
        )
        .await
        .map_err(|_| {
            anyhow!(
                "Timed out importing registry skill after {}s. The upstream GitHub package may be too large or slow; no team skill was created.",
                PACKAGE_RESOLVE_TIMEOUT_SECS
            )
        })??;
        let package = match resolution {
            SkillResolution::Resolved(package) => package,
            SkillResolution::Multiple {
                candidate_count,
                candidates,
                ..
            } => {
                return Err(anyhow!(
                    "Multiple skills were found ({} candidates). Preview or import one candidate by its install_spec: {}",
                    candidate_count,
                    candidates
                        .iter()
                        .map(|candidate| candidate.install_spec.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }
            SkillResolution::NotFound { message, .. } => return Err(anyhow!(message)),
        };
        let visibility = args
            .get("visibility")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string);

        let skill = self
            .service()
            .create_package(
                &self.team_id,
                &self.actor_id,
                &package.skill_id,
                Some(package.description.clone()),
                package.skill_md.clone(),
                package.files.clone(),
                package.body.clone(),
                Some(package.tags.clone()),
                visibility.clone(),
            )
            .await?;
        let imported_skill_id = skill.id.map(|id| id.to_hex()).unwrap_or_default();
        let skill_ref = build_skill_ref(
            &format!("team:{}", imported_skill_id),
            &skill.name,
            "imported",
            &skill.version,
        );

        if let Some(skill_oid) = skill.id {
            self.persist_import_metadata(skill_oid, &package, visibility.as_deref())
                .await?;
        }
        let verification = self.verify_import(&imported_skill_id, &skill.name).await;

        Ok(json!({
            "team_id": self.team_id,
            "source": if package.source_type == "local_skill_source" {
                package.source_repo.clone()
            } else {
                format!("{}/{}", package.owner, package.repo)
            },
            "skill_id": package.skill_id,
            "install_spec": package.install_spec,
            "skills_sh_url": package.source_url,
            "source_url": package.source_url,
            "source_ref": package.source_ref,
            "source_commit": package.source_commit,
            "imported_skill_id": imported_skill_id,
            "name": skill.name,
            "skill_ref": skill_ref,
            "display_line_zh": build_imported_display_line_zh(&skill_ref, &skill.version),
            "display_line_en": build_imported_display_line_en(&skill_ref, &skill.version),
            "plain_line_zh": build_imported_plain_line_zh(&skill.name, &skill.version),
            "plain_line_en": build_imported_plain_line_en(&skill.name, &skill.version),
            "skill_class": "imported",
            "description": skill.description,
            "visibility": skill.visibility,
            "file_count": package.files.len(),
            "skipped_files": package.skipped_files,
            "verification": verification,
        })
        .to_string())
    }

    async fn handle_check_updates(&self, args: &JsonObject) -> Result<String> {
        let imported_skill_id = args
            .get("imported_skill_id")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty());
        let mut imported_skills = Vec::new();
        for skill in self.load_imported_skills(imported_skill_id).await? {
            if let Some(inspection) = self.inspect_update(skill).await? {
                imported_skills.push(inspection);
            }
        }

        Ok(json!({
            "team_id": self.team_id,
            "count": imported_skills.len(),
            "updates": imported_skills,
        })
        .to_string())
    }

    async fn handle_list_imported(&self, _args: &JsonObject) -> Result<String> {
        let imported_skills = self
            .load_imported_skills(None)
            .await?
            .into_iter()
            .filter_map(|skill| {
                let metadata = parse_imported_metadata(skill.metadata.as_ref())?;
                let imported_skill_id = skill.id.map(|id| id.to_hex()).unwrap_or_default();
                let skill_ref = build_skill_ref(
                    &format!("team:{}", imported_skill_id),
                    &skill.name,
                    "imported",
                    &skill.version,
                );
                Some(ImportedRegistrySkillSummary {
                    imported_skill_id,
                    name: skill.name.clone(),
                    skill_ref: skill_ref.clone(),
                    display_line_zh: build_imported_display_line_zh(&skill_ref, &skill.version),
                    display_line_en: build_imported_display_line_en(&skill_ref, &skill.version),
                    plain_line_zh: build_imported_plain_line_zh(&skill.name, &skill.version),
                    plain_line_en: build_imported_plain_line_en(&skill.name, &skill.version),
                    skill_class: "imported".to_string(),
                    description: skill.description.clone(),
                    version: skill.version.clone(),
                    visibility: skill.visibility.clone(),
                    source: metadata.source_repo.clone(),
                    skill_id: infer_imported_skill_id(&metadata, &skill.name),
                    source_ref: metadata.source_ref.clone(),
                    source_commit: metadata.source_commit.clone(),
                    source_tree_sha: metadata.source_tree_sha.clone(),
                    source_url: metadata.source_url.clone(),
                    registry_provider: metadata.registry_provider.clone(),
                    skipped_files: metadata.skipped_files.unwrap_or_default(),
                    updated_at: skill.updated_at.to_rfc3339(),
                })
            })
            .collect::<Vec<_>>();

        Ok(json!({
            "team_id": self.team_id,
            "count": imported_skills.len(),
            "skills": imported_skills,
        })
        .to_string())
    }

    async fn handle_upgrade(&self, args: &JsonObject) -> Result<String> {
        let imported_skill_id = args
            .get("imported_skill_id")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| anyhow!("imported_skill_id is required"))?;
        let force = args.get("force").and_then(|v| v.as_bool()).unwrap_or(false);

        let skill = self
            .load_imported_skills(Some(imported_skill_id))
            .await?
            .into_iter()
            .next()
            .ok_or_else(|| anyhow!("Imported registry skill not found"))?;
        let inspection = self
            .inspect_update(skill.clone())
            .await?
            .ok_or_else(|| anyhow!("Skill is missing registry import metadata"))?;

        if !inspection.has_update && !force {
            return Ok(json!({
                "team_id": self.team_id,
                "imported_skill_id": inspection.imported_skill_id,
                "name": inspection.name,
                "skill_ref": inspection.skill_ref,
                "display_line_zh": inspection.display_line_zh,
                "display_line_en": inspection.display_line_en,
                "plain_line_zh": inspection.plain_line_zh,
                "plain_line_en": inspection.plain_line_en,
                "skill_class": inspection.skill_class,
                "upgraded": false,
                "reason": "No upstream update detected",
                "current_version": inspection.current_version,
                "source_ref": inspection.source_ref,
                "source_tree_sha": inspection.current_tree_sha,
            })
            .to_string());
        }

        let source = format!("{}/{}", inspection.owner, inspection.repo);
        let package = timeout(
            Duration::from_secs(PACKAGE_RESOLVE_TIMEOUT_SECS),
            self.resolve_skill_package_from_values(
                &source,
                &inspection.skill_id,
                Some(inspection.source_ref.as_str()),
            ),
        )
        .await
        .map_err(|_| {
            anyhow!(
                "Timed out upgrading registry skill after {}s. The upstream GitHub package may be too large or slow; the existing skill was not changed.",
                PACKAGE_RESOLVE_TIMEOUT_SECS
            )
        })??;
        let metadata =
            build_import_metadata(&package, Some(skill.visibility.as_str()), &self.actor_id);
        let updated = self
            .service()
            .update_package(
                &inspection.imported_skill_id,
                Some(package.description.clone()),
                package.skill_md.clone(),
                package.files.clone(),
                package.body.clone(),
                Some(package.tags.clone()),
                Some(metadata),
            )
            .await?;

        Ok(json!({
            "team_id": self.team_id,
            "imported_skill_id": inspection.imported_skill_id,
            "name": updated.name,
            "skill_ref": build_skill_ref(
                &format!("team:{}", inspection.imported_skill_id),
                &updated.name,
                "imported",
                &updated.version,
            ),
            "display_line_zh": build_imported_display_line_zh(
                &build_skill_ref(
                    &format!("team:{}", inspection.imported_skill_id),
                    &updated.name,
                    "imported",
                    &updated.version,
                ),
                &updated.version,
            ),
            "display_line_en": build_imported_display_line_en(
                &build_skill_ref(
                    &format!("team:{}", inspection.imported_skill_id),
                    &updated.name,
                    "imported",
                    &updated.version,
                ),
                &updated.version,
            ),
            "plain_line_zh": build_imported_plain_line_zh(&updated.name, &updated.version),
            "plain_line_en": build_imported_plain_line_en(&updated.name, &updated.version),
            "skill_class": "imported",
            "upgraded": true,
            "previous_version": inspection.current_version,
            "current_version": updated.version,
            "source_ref": package.source_ref,
            "source_commit": package.source_commit,
            "source_tree_sha": package.source_tree_sha,
            "file_count": package.files.len(),
            "skipped_files": package.skipped_files,
        })
        .to_string())
    }

    async fn persist_import_metadata(
        &self,
        skill_oid: ObjectId,
        package: &ImportedSkillPackage,
        visibility: Option<&str>,
    ) -> Result<()> {
        let coll = self.db.collection::<Skill>("skills");
        let metadata = build_import_metadata(package, visibility, &self.actor_id);
        coll.update_one(
            doc! { "_id": skill_oid },
            doc! { "$set": { "metadata": mongodb::bson::to_bson(&metadata)? } },
            None,
        )
        .await?;
        Ok(())
    }

    async fn verify_import(&self, imported_skill_id: &str, skill_name: &str) -> serde_json::Value {
        let persisted = match (
            ObjectId::parse_str(imported_skill_id),
            ObjectId::parse_str(&self.team_id),
        ) {
            (Ok(skill_oid), Ok(team_oid)) => self
                .db
                .collection::<Skill>("skills")
                .find_one(
                    doc! {
                        "_id": skill_oid,
                        "team_id": team_oid,
                        "is_deleted": { "$ne": true },
                    },
                    None,
                )
                .await
                .ok()
                .flatten()
                .is_some(),
            _ => false,
        };
        let catalog_visible = self
            .service()
            .list(&self.team_id, Some(1), Some(10), Some(skill_name), None)
            .await
            .map(|page| page.items.iter().any(|item| item.name == skill_name))
            .unwrap_or(false);
        json!({
            "persisted": persisted,
            "catalog_visible": catalog_visible,
        })
    }

    async fn resolve_skill_request(&self, args: &JsonObject) -> Result<SkillResolution> {
        let spec = self.parse_request_spec(args)?;
        self.resolve_skill_resolution(spec).await
    }

    fn parse_request_spec(&self, args: &JsonObject) -> Result<SkillInstallSpec> {
        if let Some(raw) = args
            .get("install_spec")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            return parse_install_spec(raw);
        }
        let source = args
            .get("source")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| anyhow!("source is required"))?;
        let skill_id = args
            .get("skill_id")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty());
        let source_ref = args
            .get("source_ref")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        let (owner, repo) = parse_github_source(source).ok_or_else(|| {
            anyhow!(
                "Only GitHub-backed registry sources are supported. Unsupported source: {}",
                source
            )
        })?;
        Ok(SkillInstallSpec {
            owner,
            repo,
            skill_id: skill_id.map(str::to_string),
            source_ref,
            skill_dir: None,
            local_path: None,
            raw: match skill_id {
                Some(skill_id) => build_install_spec(source, skill_id),
                None => source.to_string(),
            },
        })
    }

    async fn resolve_skill_resolution(&self, spec: SkillInstallSpec) -> Result<SkillResolution> {
        if spec.local_path.is_some() {
            return self.resolve_local_skill_resolution(spec).await;
        }
        let resolved_ref = match spec.source_ref.as_deref() {
            Some(value) => value.to_string(),
            None => self.fetch_default_branch(&spec.owner, &spec.repo).await?,
        };
        let tree = self
            .fetch_git_tree(&spec.owner, &spec.repo, &resolved_ref)
            .await?;
        let candidates = discover_skill_candidates(&spec, &resolved_ref, &tree.tree);
        if candidates.is_empty() {
            return Ok(SkillResolution::NotFound {
                install_spec: spec.raw.clone(),
                source: spec.source(),
                source_ref: resolved_ref,
                message: format!(
                    "No SKILL.md matched '{}' in {}/{}",
                    spec.skill_id
                        .as_deref()
                        .or(spec.skill_dir.as_deref())
                        .unwrap_or("repository"),
                    spec.owner,
                    spec.repo
                ),
            });
        }
        if candidates.len() > 1 {
            return Ok(SkillResolution::Multiple {
                install_spec: spec.raw.clone(),
                source: spec.source(),
                source_ref: resolved_ref,
                candidate_count: candidates.len(),
                candidates,
            });
        }
        let candidate = candidates.into_iter().next().expect("candidate exists");
        let package = self
            .resolve_skill_package_from_candidate(&spec, candidate, &tree)
            .await?;
        Ok(SkillResolution::Resolved(package))
    }

    async fn resolve_local_skill_resolution(
        &self,
        spec: SkillInstallSpec,
    ) -> Result<SkillResolution> {
        let local = self.prepare_local_skill_source(&spec)?;
        let candidates = discover_local_skill_candidates(&spec, &local.root)?;
        if candidates.is_empty() {
            if let Some(cleanup_root) = &local.cleanup_root {
                let _ = std::fs::remove_dir_all(cleanup_root);
            }
            return Ok(SkillResolution::NotFound {
                install_spec: spec.raw.clone(),
                source: spec.source(),
                source_ref: "local".to_string(),
                message: format!("No SKILL.md matched local source '{}'", spec.source()),
            });
        }
        if candidates.len() > 1 {
            if let Some(cleanup_root) = &local.cleanup_root {
                let _ = std::fs::remove_dir_all(cleanup_root);
            }
            return Ok(SkillResolution::Multiple {
                install_spec: spec.raw.clone(),
                source: spec.source(),
                source_ref: "local".to_string(),
                candidate_count: candidates.len(),
                candidates,
            });
        }
        let candidate = candidates.into_iter().next().expect("candidate exists");
        let package = self.resolve_local_package_from_candidate(&spec, &local, candidate)?;
        if let Some(cleanup_root) = &local.cleanup_root {
            let _ = std::fs::remove_dir_all(cleanup_root);
        }
        Ok(SkillResolution::Resolved(package))
    }

    async fn resolve_skill_package_from_values(
        &self,
        source: &str,
        skill_id: &str,
        source_ref: Option<&str>,
    ) -> Result<ImportedSkillPackage> {
        let (owner, repo) =
            parse_github_source(source).ok_or_else(|| anyhow!("Unsupported source: {}", source))?;
        let spec = SkillInstallSpec {
            owner,
            repo,
            skill_id: Some(skill_id.to_string()),
            source_ref: source_ref.map(str::to_string),
            skill_dir: None,
            local_path: None,
            raw: build_install_spec(source, skill_id),
        };
        match self.resolve_skill_resolution(spec).await? {
            SkillResolution::Resolved(package) => Ok(package),
            SkillResolution::Multiple {
                candidate_count, ..
            } => Err(anyhow!(
                "Multiple skills were found ({} candidates); specify owner/repo@skill",
                candidate_count
            )),
            SkillResolution::NotFound { message, .. } => Err(anyhow!(message)),
        }
    }

    async fn resolve_skill_package_from_candidate(
        &self,
        spec: &SkillInstallSpec,
        candidate: SkillCandidate,
        tree: &GitHubTreeResponse,
    ) -> Result<ImportedSkillPackage> {
        let skill_md_path = candidate.skill_md_path.clone();
        let skill_md = self
            .fetch_raw_text(
                &spec.owner,
                &spec.repo,
                &candidate.source_ref,
                &skill_md_path,
            )
            .await
            .with_context(|| format!("failed to load {}", skill_md_path))?;
        let (frontmatter, body) = PackageService::parse_skill_md(&skill_md)
            .map_err(|e| anyhow!("invalid SKILL.md for '{}': {}", candidate.skill_id, e))?;

        let mut files = Vec::new();
        let mut skipped_files = Vec::new();
        let mut total_bytes = 0usize;
        for item in tree
            .tree
            .iter()
            .filter(|item| item.item_type == "blob")
            .filter(|item| candidate_contains_path(&candidate.skill_dir, &item.path))
            .filter(|item| item.path != skill_md_path)
        {
            let rel_path = item
                .path
                .strip_prefix(&(candidate.skill_dir.clone() + "/"))
                .unwrap_or(&item.path)
                .to_string();
            if files.len() >= MAX_PACKAGE_FILES {
                skipped_files.push(format!(
                    "{} (skipped: package file limit reached)",
                    rel_path
                ));
                continue;
            }
            match self
                .fetch_raw_bytes(&spec.owner, &spec.repo, &candidate.source_ref, &item.path)
                .await
            {
                Ok(bytes) => {
                    if bytes.len() > MAX_PACKAGE_FILE_BYTES {
                        skipped_files.push(format!(
                            "{} (skipped: file exceeds {} bytes)",
                            rel_path, MAX_PACKAGE_FILE_BYTES
                        ));
                        continue;
                    }
                    if total_bytes.saturating_add(bytes.len()) > MAX_PACKAGE_TOTAL_BYTES {
                        skipped_files.push(format!(
                            "{} (skipped: package exceeds {} bytes)",
                            rel_path, MAX_PACKAGE_TOTAL_BYTES
                        ));
                        continue;
                    }
                    match String::from_utf8(bytes) {
                        Ok(content) => {
                            total_bytes += content.len();
                            files.push(SkillFile {
                                path: rel_path,
                                content,
                            });
                        }
                        Err(_) => skipped_files.push(rel_path),
                    }
                }
                Err(_) => skipped_files.push(rel_path),
            }
        }

        let mut tags = frontmatter
            .metadata
            .as_ref()
            .map(|meta| meta.keywords.clone())
            .unwrap_or_default();
        tags.push("skills-sh".to_string());
        tags.push("registry-import".to_string());
        dedupe_tags(&mut tags);

        Ok(ImportedSkillPackage {
            source_type: "skills_sh_registry".to_string(),
            source_repo: format!("{}/{}", spec.owner, spec.repo),
            owner: spec.owner.clone(),
            repo: spec.repo.clone(),
            skill_id: candidate.skill_id.clone(),
            source_tree_sha: resolve_tree_sha(&candidate.skill_dir, &tree.tree)
                .unwrap_or_else(|| tree.sha.clone()),
            skill_dir: candidate.skill_dir.clone(),
            install_spec: candidate.install_spec,
            source_url: candidate.skills_sh_url,
            source_ref: candidate.source_ref,
            source_commit: tree.sha.clone(),
            skill_md,
            description: frontmatter.description,
            body,
            tags,
            files,
            skipped_files,
        })
    }

    fn prepare_local_skill_source(&self, spec: &SkillInstallSpec) -> Result<LocalSkillSource> {
        let path = spec
            .local_path
            .as_ref()
            .ok_or_else(|| anyhow!("local install source is required"))?;
        if !path.exists() {
            return Err(anyhow!(
                "Local skill source '{}' does not exist",
                path.display()
            ));
        }
        self.ensure_local_source_in_workspace(path)?;
        if path.is_dir() {
            return Ok(LocalSkillSource {
                root: path.clone(),
                cleanup_root: None,
            });
        }

        let ext = path
            .extension()
            .and_then(|value| value.to_str())
            .map(|value| value.to_ascii_lowercase());
        if matches!(
            ext.as_deref(),
            Some("zip")
                | Some("rar")
                | Some("7z")
                | Some("tar")
                | Some("gz")
                | Some("tgz")
                | Some("xz")
                | Some("bz2")
                | Some("zst")
        ) {
            return Err(anyhow!(
                "Archive sources like '{}' are not imported directly. First extract the archive into the workspace with developer shell or another local tool, then import the extracted directory or SKILL.md path.",
                path.display()
            ));
        }

        if ext.as_deref() == Some("md")
            && path
                .file_name()
                .and_then(|value| value.to_str())
                .map(|value| value.eq_ignore_ascii_case("SKILL.md"))
                .unwrap_or(false)
        {
            return Ok(LocalSkillSource {
                root: path
                    .parent()
                    .map(Path::to_path_buf)
                    .ok_or_else(|| anyhow!("SKILL.md path has no parent directory"))?,
                cleanup_root: None,
            });
        }
        Err(anyhow!(
            "Unsupported local skill source '{}'. Use a directory or a SKILL.md file.",
            path.display()
        ))
    }

    fn ensure_local_source_in_workspace(&self, path: &Path) -> Result<()> {
        ensure_local_source_in_workspace_root(self.workspace_root.as_deref(), path)
    }

    fn resolve_local_package_from_candidate(
        &self,
        spec: &SkillInstallSpec,
        local: &LocalSkillSource,
        candidate: SkillCandidate,
    ) -> Result<ImportedSkillPackage> {
        let skill_root = if candidate.skill_dir.is_empty() {
            local.root.clone()
        } else {
            local.root.join(&candidate.skill_dir)
        };
        let skill_md_path = skill_root.join("SKILL.md");
        let skill_md = std::fs::read_to_string(&skill_md_path)
            .with_context(|| format!("failed to load {}", skill_md_path.display()))?;
        let (frontmatter, body) = PackageService::parse_skill_md(&skill_md)
            .map_err(|e| anyhow!("invalid SKILL.md for '{}': {}", candidate.skill_id, e))?;

        let mut files = Vec::new();
        let mut skipped_files = Vec::new();
        let mut total_bytes = 0usize;
        for entry in std::fs::read_dir(&skill_root)? {
            let entry = entry?;
            let path = entry.path();
            if !path.is_file() || path == skill_md_path {
                continue;
            }
            let rel_path = path
                .strip_prefix(&skill_root)
                .ok()
                .and_then(|value| value.to_str())
                .map(str::to_string)
                .unwrap_or_else(|| {
                    path.file_name()
                        .and_then(|value| value.to_str())
                        .unwrap_or_default()
                        .to_string()
                });
            if files.len() >= MAX_PACKAGE_FILES {
                skipped_files.push(format!(
                    "{} (skipped: package file limit reached)",
                    rel_path
                ));
                continue;
            }
            let bytes = std::fs::read(&path)
                .with_context(|| format!("failed to read {}", path.display()))?;
            if bytes.len() > MAX_PACKAGE_FILE_BYTES {
                skipped_files.push(format!(
                    "{} (skipped: file exceeds {} bytes)",
                    rel_path, MAX_PACKAGE_FILE_BYTES
                ));
                continue;
            }
            if total_bytes.saturating_add(bytes.len()) > MAX_PACKAGE_TOTAL_BYTES {
                skipped_files.push(format!(
                    "{} (skipped: package exceeds {} bytes)",
                    rel_path, MAX_PACKAGE_TOTAL_BYTES
                ));
                continue;
            }
            match String::from_utf8(bytes) {
                Ok(content) => {
                    total_bytes += content.len();
                    files.push(SkillFile {
                        path: rel_path,
                        content,
                    });
                }
                Err(_) => skipped_files.push(rel_path),
            }
        }

        let mut tags = frontmatter
            .metadata
            .as_ref()
            .map(|meta| meta.keywords.clone())
            .unwrap_or_default();
        tags.push("local-import".to_string());
        tags.push("registry-import".to_string());
        dedupe_tags(&mut tags);

        let source_path = spec
            .local_path
            .as_ref()
            .map(|value| value.to_string_lossy().to_string())
            .unwrap_or_else(|| local.root.to_string_lossy().to_string());

        Ok(ImportedSkillPackage {
            source_type: "local_skill_source".to_string(),
            source_repo: source_path.clone(),
            owner: String::new(),
            repo: String::new(),
            skill_id: candidate.skill_id,
            skill_dir: candidate.skill_dir,
            install_spec: candidate.install_spec,
            source_url: build_local_install_spec(
                spec.local_path.as_deref().unwrap_or(local.root.as_path()),
                spec.skill_id.as_deref(),
            ),
            source_ref: "local".to_string(),
            source_commit: "local".to_string(),
            source_tree_sha: "local".to_string(),
            skill_md,
            description: frontmatter.description,
            body,
            tags,
            files,
            skipped_files,
        })
    }

    async fn load_imported_skills(&self, imported_skill_id: Option<&str>) -> Result<Vec<Skill>> {
        let team_oid = ObjectId::parse_str(&self.team_id)?;
        let coll = self.db.collection::<Skill>("skills");
        let mut filter = doc! {
            "team_id": team_oid,
            "is_deleted": { "$ne": true },
            "metadata.import_mode": "registry_import",
        };
        if let Some(skill_id) = imported_skill_id {
            filter.insert("_id", ObjectId::parse_str(skill_id)?);
        }
        let cursor = coll.find(filter, None).await?;
        Ok(cursor.try_collect().await?)
    }

    async fn inspect_update(&self, skill: Skill) -> Result<Option<RemoteUpdateInspection>> {
        let Some(metadata) = parse_imported_metadata(skill.metadata.as_ref()) else {
            return Ok(None);
        };
        if metadata.source_type != "skills_sh_registry" {
            return Ok(None);
        }
        let (owner, repo) = parse_github_source(&metadata.source_repo).ok_or_else(|| {
            anyhow!(
                "Imported skill '{}' has invalid source_repo '{}'",
                skill.name,
                metadata.source_repo
            )
        })?;
        let imported_skill_key = infer_imported_skill_id(&metadata, &skill.name);
        let package = timeout(
            Duration::from_secs(PACKAGE_RESOLVE_TIMEOUT_SECS),
            self.resolve_skill_package_from_values(
                &metadata.source_repo,
                &imported_skill_key,
                Some(&metadata.source_ref),
            ),
        )
        .await
        .map_err(|_| {
            anyhow!(
                "Timed out checking registry update for '{}' after {}s",
                skill.name,
                PACKAGE_RESOLVE_TIMEOUT_SECS
            )
        })??;
        let current_tree_sha = metadata
            .source_tree_sha
            .clone()
            .or(metadata.source_commit.clone())
            .unwrap_or_default();
        let has_update = current_tree_sha != package.source_tree_sha;
        let imported_skill_id = skill.id.map(|id| id.to_hex()).unwrap_or_default();
        let skill_ref = build_skill_ref(
            &format!("team:{}", imported_skill_id),
            &skill.name,
            "imported",
            &skill.version,
        );

        Ok(Some(RemoteUpdateInspection {
            imported_skill_id,
            name: skill.name.clone(),
            skill_ref: skill_ref.clone(),
            display_line_zh: build_imported_display_line_zh(&skill_ref, &skill.version),
            display_line_en: build_imported_display_line_en(&skill_ref, &skill.version),
            plain_line_zh: build_imported_plain_line_zh(&skill.name, &skill.version),
            plain_line_en: build_imported_plain_line_en(&skill.name, &skill.version),
            skill_class: "imported".to_string(),
            current_version: skill.version,
            description: skill.description,
            source: metadata.source_repo,
            skill_id: imported_skill_key,
            source_ref: metadata.source_ref,
            current_tree_sha,
            latest_tree_sha: package.source_tree_sha,
            latest_source_commit: package.source_commit,
            has_update,
            owner,
            repo,
        }))
    }

    async fn fetch_default_branch(&self, owner: &str, repo: &str) -> Result<String> {
        let url = format!("https://api.github.com/repos/{}/{}", owner, repo);
        let response = self
            .client
            .get(&url)
            .send()
            .await
            .with_context(|| {
                format!(
                    "failed to query GitHub repo metadata for {}/{}",
                    owner, repo
                )
            })?
            .error_for_status()
            .with_context(|| {
                format!(
                    "GitHub repo metadata returned non-success for {}/{}",
                    owner, repo
                )
            })?
            .json::<GitHubRepoResponse>()
            .await
            .context("failed to decode GitHub repo metadata response")?;
        Ok(response.default_branch)
    }

    async fn fetch_git_tree(
        &self,
        owner: &str,
        repo: &str,
        source_ref: &str,
    ) -> Result<GitHubTreeResponse> {
        let url = format!(
            "https://api.github.com/repos/{}/{}/git/trees/{}?recursive=1",
            owner, repo, source_ref
        );
        let response = self
            .client
            .get(&url)
            .send()
            .await
            .with_context(|| {
                format!(
                    "failed to query GitHub tree for {}/{}@{}",
                    owner, repo, source_ref
                )
            })?
            .error_for_status()
            .with_context(|| {
                format!(
                    "GitHub tree returned non-success for {}/{}@{}",
                    owner, repo, source_ref
                )
            })?
            .json::<GitHubTreeResponse>()
            .await
            .context("failed to decode GitHub tree response")?;

        if response.truncated {
            return Err(anyhow!(
                "Repository tree for {}/{}@{} is truncated; import is not supported for this repository yet",
                owner,
                repo,
                source_ref
            ));
        }

        Ok(response)
    }

    async fn fetch_raw_text(
        &self,
        owner: &str,
        repo: &str,
        source_ref: &str,
        path: &str,
    ) -> Result<String> {
        let bytes = self.fetch_raw_bytes(owner, repo, source_ref, path).await?;
        String::from_utf8(bytes).map_err(|_| anyhow!("{} is not valid UTF-8 text", path))
    }

    async fn fetch_raw_bytes(
        &self,
        owner: &str,
        repo: &str,
        source_ref: &str,
        path: &str,
    ) -> Result<Vec<u8>> {
        let url = format!(
            "https://raw.githubusercontent.com/{}/{}/{}/{}",
            owner, repo, source_ref, path
        );
        match self.client.get(&url).send().await {
            Ok(response) => match response.error_for_status() {
                Ok(response) => return Ok(response.bytes().await?.to_vec()),
                Err(err) => {
                    tracing::debug!("raw GitHub fetch failed for {}: {}", path, err);
                }
            },
            Err(err) => {
                tracing::debug!("raw GitHub fetch failed for {}: {}", path, err);
            }
        }

        let api_url = format!(
            "https://api.github.com/repos/{}/{}/contents/{}",
            owner, repo, path
        );
        let content = self
            .client
            .get(&api_url)
            .query(&[("ref", source_ref)])
            .send()
            .await
            .with_context(|| format!("failed to fetch {} via GitHub contents API", path))?
            .error_for_status()
            .with_context(|| {
                format!(
                    "GitHub contents API returned non-success for {}/{}@{}:{}",
                    owner, repo, source_ref, path
                )
            })?
            .json::<GitHubContentResponse>()
            .await
            .with_context(|| format!("failed to decode GitHub contents response for {}", path))?;
        if !content.encoding.eq_ignore_ascii_case("base64") {
            return Err(anyhow!(
                "GitHub contents response for {} used unsupported encoding '{}'",
                path,
                content.encoding
            ));
        }
        let normalized = content
            .content
            .chars()
            .filter(|ch| !ch.is_whitespace())
            .collect::<String>();
        STANDARD
            .decode(normalized)
            .with_context(|| format!("failed to decode GitHub base64 content for {}", path))
    }
}

fn ensure_local_source_in_workspace_root(workspace_root: Option<&Path>, path: &Path) -> Result<()> {
    let Some(workspace_root) = workspace_root else {
        return Ok(());
    };
    let workspace_root = workspace_root.canonicalize().with_context(|| {
        format!(
            "workspace root '{}' is not accessible for local skill import",
            workspace_root.display()
        )
    })?;
    let source_path = path.canonicalize().with_context(|| {
        format!(
            "local skill source '{}' is not accessible for import",
            path.display()
        )
    })?;
    if source_path.starts_with(&workspace_root) {
        return Ok(());
    }
    Err(anyhow!(
        "Local skill source '{}' is outside the current workspace. Import the attachment or extracted skill into the workspace first, then use that workspace path.",
        path.display()
    ))
}

#[async_trait::async_trait]
impl McpClientTrait for SkillRegistryToolsProvider {
    async fn list_resources(
        &self,
        _next_cursor: Option<String>,
        _cancel_token: CancellationToken,
    ) -> std::result::Result<ListResourcesResult, ServiceError> {
        Ok(ListResourcesResult {
            resources: vec![],
            next_cursor: None,
            meta: None,
        })
    }

    async fn read_resource(
        &self,
        _uri: &str,
        _cancel_token: CancellationToken,
    ) -> std::result::Result<ReadResourceResult, ServiceError> {
        Err(ServiceError::TransportClosed)
    }

    async fn list_tools(
        &self,
        _next_cursor: Option<String>,
        _cancel_token: CancellationToken,
    ) -> std::result::Result<ListToolsResult, ServiceError> {
        Ok(ListToolsResult {
            tools: Self::tool_definitions(),
            next_cursor: None,
            meta: None,
        })
    }

    async fn call_tool(
        &self,
        name: &str,
        arguments: Option<JsonObject>,
        _cancel_token: CancellationToken,
    ) -> std::result::Result<CallToolResult, ServiceError> {
        let args = arguments.unwrap_or_default();
        let result = match name {
            "list_popular_skills" => self.handle_list_popular(&args).await,
            "search_skills" => self.handle_search(&args).await,
            "preview_skill" => self.handle_preview(&args).await,
            "import_skill_to_team" => self.handle_import(&args).await,
            "list_imported_registry_skills" => self.handle_list_imported(&args).await,
            "check_skill_updates" => self.handle_check_updates(&args).await,
            "upgrade_imported_skill" => self.handle_upgrade(&args).await,
            _ => Err(anyhow!("Unknown tool: {}", name)),
        };

        match result {
            Ok(text) => Ok(CallToolResult::success(vec![Content::text(text)])),
            Err(err) => Ok(CallToolResult::error(vec![Content::text(err.to_string())])),
        }
    }

    async fn list_tasks(
        &self,
        _cursor: Option<String>,
        _cancel_token: CancellationToken,
    ) -> std::result::Result<ListTasksResult, ServiceError> {
        Err(ServiceError::TransportClosed)
    }

    async fn get_task_info(
        &self,
        _task_id: &str,
        _cancel_token: CancellationToken,
    ) -> std::result::Result<GetTaskInfoResult, ServiceError> {
        Err(ServiceError::TransportClosed)
    }

    async fn get_task_result(
        &self,
        _task_id: &str,
        _cancel_token: CancellationToken,
    ) -> std::result::Result<TaskResult, ServiceError> {
        Err(ServiceError::TransportClosed)
    }

    async fn cancel_task(
        &self,
        _task_id: &str,
        _cancel_token: CancellationToken,
    ) -> std::result::Result<(), ServiceError> {
        Err(ServiceError::TransportClosed)
    }

    async fn list_prompts(
        &self,
        _next_cursor: Option<String>,
        _cancel_token: CancellationToken,
    ) -> std::result::Result<ListPromptsResult, ServiceError> {
        Ok(ListPromptsResult {
            prompts: vec![],
            next_cursor: None,
            meta: None,
        })
    }

    async fn get_prompt(
        &self,
        _name: &str,
        _arguments: serde_json::Value,
        _cancel_token: CancellationToken,
    ) -> std::result::Result<GetPromptResult, ServiceError> {
        Err(ServiceError::TransportClosed)
    }

    async fn subscribe(&self) -> mpsc::Receiver<ServerNotification> {
        let (_tx, rx) = mpsc::channel(1);
        rx
    }

    fn get_info(&self) -> Option<&InitializeResult> {
        Some(&self.info)
    }
}

fn parse_github_source(source: &str) -> Option<(String, String)> {
    let trimmed = source.trim().trim_end_matches('/');
    let normalized = trimmed
        .strip_prefix("https://github.com/")
        .unwrap_or(trimmed)
        .trim_matches('/');
    let mut parts = normalized.split('/');
    let owner = parts.next()?.trim();
    let repo = parts.next()?.trim();
    if owner.is_empty() || repo.is_empty() || parts.next().is_some() {
        return None;
    }
    Some((owner.to_string(), repo.to_string()))
}

fn build_install_spec(source: &str, skill_id: &str) -> String {
    format!(
        "{}@{}",
        source.trim().trim_end_matches('/'),
        skill_id.trim()
    )
}

fn build_skills_sh_url(source: &str, skill_id: &str) -> String {
    let source = source
        .trim()
        .trim_end_matches('/')
        .strip_prefix("https://github.com/")
        .unwrap_or(source.trim().trim_end_matches('/'));
    format!("https://skills.sh/{}/{}", source, skill_id.trim())
}

fn build_local_install_spec(path: &Path, skill_id: Option<&str>) -> String {
    let mut spec = format!("file://{}", path.to_string_lossy().replace('\\', "/"));
    if let Some(skill_id) = skill_id.filter(|value| !value.trim().is_empty()) {
        spec.push_str("#skill=");
        spec.push_str(skill_id.trim());
    }
    spec
}

fn parse_install_spec(raw: &str) -> Result<SkillInstallSpec> {
    let trimmed = raw.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return Err(anyhow!("install_spec is required"));
    }

    let local_path = Path::new(trimmed);
    if local_path.is_absolute() {
        return Ok(SkillInstallSpec {
            owner: String::new(),
            repo: String::new(),
            skill_id: None,
            source_ref: None,
            skill_dir: None,
            local_path: Some(local_path.to_path_buf()),
            raw: trimmed.to_string(),
        });
    }

    if let Ok(url) = Url::parse(trimmed) {
        let host = url.host_str().unwrap_or_default().to_ascii_lowercase();
        if url.scheme().eq_ignore_ascii_case("file") {
            let path = url
                .to_file_path()
                .map_err(|_| anyhow!("Invalid local file URL '{}'", raw))?;
            let skill_id = url
                .fragment()
                .and_then(|fragment| fragment.strip_prefix("skill="))
                .map(str::to_string);
            return Ok(SkillInstallSpec {
                owner: String::new(),
                repo: String::new(),
                skill_id,
                source_ref: None,
                skill_dir: None,
                local_path: Some(path),
                raw: trimmed.to_string(),
            });
        }
        let segments = url
            .path_segments()
            .map(|items| items.filter(|item| !item.is_empty()).collect::<Vec<_>>())
            .unwrap_or_default();
        if host == "skills.sh" || host == "www.skills.sh" {
            if segments.len() >= 3 {
                return Ok(SkillInstallSpec {
                    owner: segments[0].to_string(),
                    repo: segments[1].to_string(),
                    skill_id: Some(segments[2].to_string()),
                    source_ref: None,
                    skill_dir: None,
                    local_path: None,
                    raw: trimmed.to_string(),
                });
            }
        }
        if host == "github.com" || host == "www.github.com" {
            if segments.len() >= 2 {
                let mut spec = SkillInstallSpec {
                    owner: segments[0].to_string(),
                    repo: segments[1].trim_end_matches(".git").to_string(),
                    skill_id: None,
                    source_ref: None,
                    skill_dir: None,
                    local_path: None,
                    raw: trimmed.to_string(),
                };
                if segments.get(2) == Some(&"tree") && segments.len() >= 4 {
                    spec.source_ref = Some(segments[3].to_string());
                    if segments.len() > 4 {
                        let path = segments[4..].join("/");
                        spec.skill_dir = Some(path.clone());
                        spec.skill_id = path.rsplit('/').next().map(str::to_string);
                    }
                }
                return Ok(spec);
            }
        }
    }

    if let Some((source, skill_id)) = trimmed.split_once('@') {
        let (owner, repo) = parse_github_source(source)
            .ok_or_else(|| anyhow!("Invalid install spec source '{}'", source))?;
        let skill_id = skill_id.trim();
        if skill_id.is_empty() {
            return Err(anyhow!("install spec '{}' is missing a skill name", raw));
        }
        return Ok(SkillInstallSpec {
            owner,
            repo,
            skill_id: Some(skill_id.to_string()),
            source_ref: None,
            skill_dir: None,
            local_path: None,
            raw: trimmed.to_string(),
        });
    }

    let (owner, repo) = parse_github_source(trimmed)
        .ok_or_else(|| anyhow!("Unsupported install spec '{}'", raw))?;
    Ok(SkillInstallSpec {
        owner,
        repo,
        skill_id: None,
        source_ref: None,
        skill_dir: None,
        local_path: None,
        raw: trimmed.to_string(),
    })
}

fn skill_id_from_dir(owner: &str, repo: &str, dir: &str) -> String {
    if dir.is_empty() {
        repo.to_string()
    } else {
        dir.rsplit('/').next().unwrap_or(repo).to_string()
    }
    .trim()
    .trim_end_matches(".git")
    .to_string()
    .if_empty_then(format!("{}-skill", owner))
}

trait EmptyFallback {
    fn if_empty_then(self, fallback: String) -> String;
}

impl EmptyFallback for String {
    fn if_empty_then(self, fallback: String) -> String {
        if self.is_empty() {
            fallback
        } else {
            self
        }
    }
}

fn candidate_contains_path(skill_dir: &str, path: &str) -> bool {
    if skill_dir.is_empty() {
        path != "SKILL.md" && !path.contains('/')
    } else {
        path.starts_with(&(skill_dir.to_string() + "/"))
    }
}

fn discover_skill_candidates(
    spec: &SkillInstallSpec,
    source_ref: &str,
    tree: &[GitHubTreeItem],
) -> Vec<SkillCandidate> {
    let requested_dir = spec
        .skill_dir
        .as_deref()
        .map(|value| value.trim().trim_matches('/').trim_end_matches("/SKILL.md"));
    let requested_skill = spec.skill_id.as_deref().map(str::trim);
    let source = spec.source();
    let mut candidates = tree
        .iter()
        .filter(|item| item.item_type == "blob")
        .filter(|item| item.path == "SKILL.md" || item.path.ends_with("/SKILL.md"))
        .filter_map(|item| {
            let dir = item
                .path
                .strip_suffix("/SKILL.md")
                .map(str::to_string)
                .or_else(|| (item.path == "SKILL.md").then(String::new))?;
            let skill_id = skill_id_from_dir(&spec.owner, &spec.repo, &dir);
            if let Some(requested_dir) = requested_dir {
                if dir != requested_dir {
                    return None;
                }
            }
            if let Some(requested_skill) = requested_skill {
                let path_matches = dir == requested_skill
                    || dir == format!("skills/{}", requested_skill)
                    || dir.rsplit('/').next() == Some(requested_skill);
                if skill_id != requested_skill && !path_matches {
                    return None;
                }
            }
            let install_spec = spec.canonical_for_skill(&skill_id);
            Some(SkillCandidate {
                skill_id: skill_id.clone(),
                name: skill_id.clone(),
                source: source.clone(),
                source_ref: source_ref.to_string(),
                skill_dir: dir.clone(),
                install_spec,
                skills_sh_url: build_skills_sh_url(&source, &skill_id),
                skill_md_path: item.path.clone(),
            })
        })
        .collect::<Vec<_>>();
    candidates.sort_by(|left, right| {
        candidate_rank(&left.skill_dir, requested_skill)
            .cmp(&candidate_rank(&right.skill_dir, requested_skill))
            .then(left.skill_dir.cmp(&right.skill_dir))
    });
    candidates
}

fn candidate_rank(skill_dir: &str, requested_skill: Option<&str>) -> u8 {
    if skill_dir.is_empty() {
        return 0;
    }
    if let Some(skill) = requested_skill {
        if skill_dir == format!("skills/{}", skill) {
            return 1;
        }
        if skill_dir == skill {
            return 2;
        }
        if skill_dir.rsplit('/').next() == Some(skill) {
            return 3;
        }
    }
    if skill_dir.starts_with("skills/") {
        4
    } else {
        5
    }
}

fn discover_local_skill_candidates(
    spec: &SkillInstallSpec,
    root: &Path,
) -> Result<Vec<SkillCandidate>> {
    let requested_dir = spec
        .skill_dir
        .as_deref()
        .map(|value| value.trim().trim_matches('/').trim_end_matches("/SKILL.md"));
    let requested_skill = spec.skill_id.as_deref().map(str::trim);
    let source = spec.source();
    let mut candidates = Vec::new();

    gather_local_skill_candidates(
        root,
        root,
        spec,
        &source,
        requested_dir,
        requested_skill,
        &mut candidates,
    )?;
    candidates.sort_by(|left, right| {
        candidate_rank(&left.skill_dir, requested_skill)
            .cmp(&candidate_rank(&right.skill_dir, requested_skill))
            .then(left.skill_dir.cmp(&right.skill_dir))
    });
    Ok(candidates)
}

fn gather_local_skill_candidates(
    root: &Path,
    current: &Path,
    spec: &SkillInstallSpec,
    source: &str,
    requested_dir: Option<&str>,
    requested_skill: Option<&str>,
    out: &mut Vec<SkillCandidate>,
) -> Result<()> {
    let skill_md_path = current.join("SKILL.md");
    if skill_md_path.is_file() {
        let rel_dir = current
            .strip_prefix(root)
            .ok()
            .and_then(|value| value.to_str())
            .unwrap_or_default()
            .replace('\\', "/");
        let rel_dir = rel_dir.trim_matches('/').to_string();
        let skill_id = if rel_dir.is_empty() {
            spec.skill_id
                .clone()
                .or_else(|| {
                    current
                        .file_name()
                        .and_then(|value| value.to_str())
                        .map(str::to_string)
                })
                .unwrap_or_else(|| "local-skill".to_string())
        } else {
            rel_dir
                .rsplit('/')
                .next()
                .unwrap_or("local-skill")
                .to_string()
        };
        if requested_dir.is_none_or(|value| value == rel_dir)
            && requested_skill.is_none_or(|value| {
                value == skill_id
                    || rel_dir == value
                    || rel_dir == format!("skills/{}", value)
                    || rel_dir.rsplit('/').next() == Some(value)
            })
        {
            out.push(SkillCandidate {
                skill_id: skill_id.clone(),
                name: skill_id.clone(),
                source: source.to_string(),
                source_ref: "local".to_string(),
                skill_dir: rel_dir.clone(),
                install_spec: spec.canonical_for_skill(&skill_id),
                skills_sh_url: build_local_install_spec(
                    spec.local_path.as_deref().unwrap_or(root),
                    Some(&skill_id),
                ),
                skill_md_path: skill_md_path.to_string_lossy().to_string(),
            });
        }
    }

    for entry in std::fs::read_dir(current)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            gather_local_skill_candidates(
                root,
                &path,
                spec,
                source,
                requested_dir,
                requested_skill,
                out,
            )?;
        }
    }
    Ok(())
}

fn extract_initial_skills(html: &str) -> Result<Vec<SkillsShPopularItem>> {
    let marker = r#"\"initialSkills\":"#;
    let marker_pos = html
        .find(marker)
        .ok_or_else(|| anyhow!("skills.sh leaderboard payload did not contain initialSkills"))?;
    let after_marker = &html[marker_pos + marker.len()..];
    let array_rel = after_marker
        .find('[')
        .ok_or_else(|| anyhow!("skills.sh leaderboard payload is missing initialSkills array"))?;
    let array_start = marker_pos + marker.len() + array_rel;
    let escaped_array = extract_initial_skills_fragment(html, array_start)
        .ok_or_else(|| anyhow!("failed to extract initialSkills JSON array"))?;
    let decoded_array = decode_embedded_json_fragment(escaped_array)?;
    serde_json::from_str::<Vec<SkillsShPopularItem>>(&decoded_array)
        .context("failed to decode skills.sh initialSkills payload")
}

fn extract_initial_skills_fragment(text: &str, array_start: usize) -> Option<&str> {
    const END_MARKER: &str = r#"],\"totalSkills\":"#;
    const LEGACY_END_MARKER: &str = r#"]}"#;

    // Current skills.sh pages embed `initialSkills` in a larger escaped JSON object.
    // Prefer the explicit `totalSkills` boundary because a generic bracket matcher can
    // accidentally consume the rest of the embedded object when quotes are escaped.
    let after_start = text.get(array_start..)?;
    if let Some(rel_end) = after_start.find(END_MARKER) {
        let end = array_start + rel_end + 1;
        return text.get(array_start..end);
    }
    if let Some(rel_end) = after_start.find(LEGACY_END_MARKER) {
        let end = array_start + rel_end + 1;
        return text.get(array_start..end);
    }

    extract_balanced_json_array(text, array_start)
}

fn extract_balanced_json_array(text: &str, start: usize) -> Option<&str> {
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;

    for (offset, ch) in text[start..].char_indices() {
        if in_string {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }

        match ch {
            '"' => in_string = true,
            '[' => depth += 1,
            ']' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    let end = start + offset + ch.len_utf8();
                    return text.get(start..end);
                }
            }
            _ => {}
        }
    }

    None
}

fn decode_embedded_json_fragment(fragment: &str) -> Result<String> {
    serde_json::from_str::<String>(&format!("\"{}\"", fragment))
        .context("failed to unescape embedded JSON fragment from skills.sh")
}

fn resolve_skill_dir(skill_id: &str, tree: &[GitHubTreeItem]) -> Option<String> {
    let exact = [
        format!("skills/{}/SKILL.md", skill_id),
        format!("{}/SKILL.md", skill_id),
    ];
    for candidate in &exact {
        if tree
            .iter()
            .any(|item| item.item_type == "blob" && item.path == *candidate)
        {
            return candidate.rsplit_once('/').map(|(dir, _)| dir.to_string());
        }
    }

    let mut candidates = tree
        .iter()
        .filter(|item| item.item_type == "blob" && item.path.ends_with("/SKILL.md"))
        .filter_map(|item| {
            item.path
                .rsplit_once('/')
                .map(|(dir, _)| dir.to_string())
                .filter(|dir| dir.rsplit('/').next() == Some(skill_id))
        })
        .collect::<Vec<_>>();
    candidates.sort();
    candidates.dedup();
    candidates.into_iter().next()
}

fn resolve_tree_sha(skill_dir: &str, tree: &[GitHubTreeItem]) -> Option<String> {
    tree.iter()
        .find(|item| item.item_type == "tree" && item.path == skill_dir)
        .and_then(|item| item.sha.clone())
}

fn dedupe_tags(tags: &mut Vec<String>) {
    let unique = tags
        .drain(..)
        .map(|tag| tag.trim().to_ascii_lowercase())
        .filter(|tag| !tag.is_empty())
        .collect::<BTreeSet<_>>();
    tags.extend(unique);
}

fn build_import_metadata(
    package: &ImportedSkillPackage,
    visibility: Option<&str>,
    actor_id: &str,
) -> serde_json::Value {
    let registry_provider = if package.source_type == "local_skill_source" {
        "local"
    } else {
        "skills.sh"
    };
    json!({
        "source_type": package.source_type,
        "source_repo": package.source_repo,
        "source_skill_path": package.skill_dir,
        "install_spec": package.install_spec,
        "source_url": package.source_url,
        "source_ref": package.source_ref,
        "source_commit": package.source_commit,
        "source_tree_sha": package.source_tree_sha,
        "import_mode": "registry_import",
        "registry_provider": registry_provider,
        "import_actor": actor_id,
        "skipped_files": package.skipped_files,
        "visibility_override": visibility,
    })
}

fn parse_imported_metadata(metadata: Option<&serde_json::Value>) -> Option<ImportedSkillMetadata> {
    serde_json::from_value(metadata?.clone()).ok()
}

fn infer_imported_skill_id(metadata: &ImportedSkillMetadata, fallback_name: &str) -> String {
    metadata
        .source_skill_path
        .rsplit('/')
        .next()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(fallback_name)
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::{
        discover_skill_candidates, ensure_local_source_in_workspace_root, extract_initial_skills,
        infer_imported_skill_id, parse_github_source, parse_imported_metadata, parse_install_spec,
        resolve_skill_dir, resolve_tree_sha, GitHubTreeItem, SkillInstallSpec,
    };
    use serde_json::json;

    #[test]
    fn parse_github_repo_source_accepts_short_and_url_forms() {
        assert_eq!(
            parse_github_source("vercel-labs/skills"),
            Some(("vercel-labs".to_string(), "skills".to_string()))
        );
        assert_eq!(
            parse_github_source("https://github.com/vercel-labs/skills"),
            Some(("vercel-labs".to_string(), "skills".to_string()))
        );
    }

    #[test]
    fn parse_install_spec_accepts_cli_and_url_forms() {
        let spec = parse_install_spec("vercel-labs/skills@find-skills").expect("cli spec");
        assert_eq!(spec.owner, "vercel-labs");
        assert_eq!(spec.repo, "skills");
        assert_eq!(spec.skill_id.as_deref(), Some("find-skills"));

        let spec = parse_install_spec("https://skills.sh/vercel-labs/skills/find-skills")
            .expect("skills.sh url");
        assert_eq!(spec.owner, "vercel-labs");
        assert_eq!(spec.repo, "skills");
        assert_eq!(spec.skill_id.as_deref(), Some("find-skills"));

        let spec = parse_install_spec(
            "https://github.com/vercel-labs/skills/tree/main/skills/find-skills",
        )
        .expect("github tree url");
        assert_eq!(spec.source_ref.as_deref(), Some("main"));
        assert_eq!(spec.skill_dir.as_deref(), Some("skills/find-skills"));
        assert_eq!(spec.skill_id.as_deref(), Some("find-skills"));

        let spec =
            parse_install_spec("https://github.com/vercel-labs/skills").expect("github repo url");
        assert_eq!(spec.owner, "vercel-labs");
        assert_eq!(spec.repo, "skills");
        assert!(spec.skill_id.is_none());
    }

    #[test]
    fn local_skill_source_must_stay_inside_workspace_when_context_is_set() {
        let base =
            std::env::temp_dir().join(format!("agime-skill-boundary-{}", uuid::Uuid::new_v4()));
        let workspace = base.join("workspace");
        let inside = workspace.join("runs").join("run-1").join("skill");
        let outside = base.join("outside-skill");
        std::fs::create_dir_all(&inside).expect("inside dir");
        std::fs::create_dir_all(&outside).expect("outside dir");
        std::fs::write(inside.join("SKILL.md"), "# Inside").expect("inside skill");
        std::fs::write(outside.join("SKILL.md"), "# Outside").expect("outside skill");

        assert!(ensure_local_source_in_workspace_root(Some(&workspace), &inside).is_ok());
        let error =
            ensure_local_source_in_workspace_root(Some(&workspace), &outside).expect_err("outside");
        assert!(error.to_string().contains("outside the current workspace"));
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn resolve_skill_dir_prefers_skills_root() {
        let tree = vec![
            GitHubTreeItem {
                path: "packages/find-skills/SKILL.md".to_string(),
                item_type: "blob".to_string(),
                sha: None,
            },
            GitHubTreeItem {
                path: "skills/find-skills/SKILL.md".to_string(),
                item_type: "blob".to_string(),
                sha: None,
            },
        ];
        assert_eq!(
            resolve_skill_dir("find-skills", &tree),
            Some("skills/find-skills".to_string())
        );
    }

    #[test]
    fn discover_candidates_supports_root_skills_root_and_nested() {
        let spec = SkillInstallSpec {
            owner: "owner".to_string(),
            repo: "repo".to_string(),
            skill_id: None,
            source_ref: None,
            skill_dir: None,
            local_path: None,
            raw: "owner/repo".to_string(),
        };
        let tree = vec![
            GitHubTreeItem {
                path: "SKILL.md".to_string(),
                item_type: "blob".to_string(),
                sha: None,
            },
            GitHubTreeItem {
                path: "skills/find-skills/SKILL.md".to_string(),
                item_type: "blob".to_string(),
                sha: None,
            },
            GitHubTreeItem {
                path: "packages/review/SKILL.md".to_string(),
                item_type: "blob".to_string(),
                sha: None,
            },
        ];
        let candidates = discover_skill_candidates(&spec, "main", &tree);
        assert_eq!(candidates.len(), 3);
        assert_eq!(candidates[0].skill_id, "repo");
        assert_eq!(candidates[1].skill_id, "find-skills");
        assert_eq!(candidates[2].skill_id, "review");
    }

    #[test]
    fn discover_candidates_filters_requested_skill() {
        let spec = SkillInstallSpec {
            owner: "vercel-labs".to_string(),
            repo: "skills".to_string(),
            skill_id: Some("find-skills".to_string()),
            source_ref: None,
            skill_dir: None,
            local_path: None,
            raw: "vercel-labs/skills@find-skills".to_string(),
        };
        let tree = vec![
            GitHubTreeItem {
                path: "skills/find-skills/SKILL.md".to_string(),
                item_type: "blob".to_string(),
                sha: None,
            },
            GitHubTreeItem {
                path: "skills/other/SKILL.md".to_string(),
                item_type: "blob".to_string(),
                sha: None,
            },
        ];
        let candidates = discover_skill_candidates(&spec, "main", &tree);
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].install_spec, "vercel-labs/skills@find-skills");
    }

    #[test]
    fn resolve_tree_sha_uses_directory_tree_entry() {
        let tree = vec![GitHubTreeItem {
            path: "skills/find-skills".to_string(),
            item_type: "tree".to_string(),
            sha: Some("tree123".to_string()),
        }];
        assert_eq!(
            resolve_tree_sha("skills/find-skills", &tree),
            Some("tree123".to_string())
        );
    }

    #[test]
    fn parse_imported_metadata_accepts_registry_shape() {
        let metadata = json!({
            "source_type": "skills_sh_registry",
            "source_repo": "vercel-labs/skills",
            "source_skill_path": "skills/find-skills",
            "source_ref": "main",
            "source_commit": "rootsha",
            "source_tree_sha": "treesha",
            "install_spec": "vercel-labs/skills@find-skills",
            "import_actor": "user-1"
        });
        let parsed = parse_imported_metadata(Some(&metadata)).expect("metadata");
        assert_eq!(parsed.source_repo, "vercel-labs/skills");
        assert_eq!(parsed.source_tree_sha.as_deref(), Some("treesha"));
        assert_eq!(infer_imported_skill_id(&parsed, "fallback"), "find-skills");
    }

    #[test]
    fn extract_initial_skills_reads_embedded_next_payload() {
        let html = r#"<script>self.__next_f.push([1,"16:[\"$\",\"$L1e\",null,{\"initialSkills\":[{\"source\":\"vercel-labs/skills\",\"skillId\":\"find-skills\",\"name\":\"find-skills\",\"installs\":16899},{\"source\":\"microsoft/azure-skills\",\"skillId\":\"microsoft-foundry\",\"name\":\"microsoft-foundry\",\"installs\":5458}]}"])</script>"#;
        let skills = extract_initial_skills(html).expect("skills");
        assert_eq!(skills.len(), 2);
        assert_eq!(skills[0].skill_id, "find-skills");
        assert_eq!(skills[1].name, "microsoft-foundry");
    }

    #[test]
    fn extract_initial_skills_reads_current_leaderboard_shape() {
        let html = r#"<script>self.__next_f.push([1,"16:[\"$\",\"$L1e\",null,{\"initialSkills\":[{\"source\":\"vercel-labs/skills\",\"skillId\":\"find-skills\",\"name\":\"find-skills\",\"installs\":16899},{\"source\":\"microsoft/azure-skills\",\"skillId\":\"microsoft-foundry\",\"name\":\"microsoft-foundry\",\"installs\":5458}],\"totalSkills\":33065,\"allTimeTotal\":88117,\"view\":\"trending\"}]"])</script>"#;
        let skills = extract_initial_skills(html).expect("skills");
        assert_eq!(skills.len(), 2);
        assert_eq!(skills[0].skill_id, "find-skills");
        assert_eq!(skills[1].name, "microsoft-foundry");
    }
}
