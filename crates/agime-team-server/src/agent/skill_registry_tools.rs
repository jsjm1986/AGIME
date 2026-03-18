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
use futures::TryStreamExt;
use mongodb::bson::{doc, oid::ObjectId};
use reqwest::Client;
use rmcp::model::*;
use rmcp::ServiceError;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::BTreeSet;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

const SKILLS_SH_SEARCH_URL: &str = "https://skills.sh/api/search";
const SKILLS_SH_ALL_TIME_URL: &str = "https://skills.sh/";
const SKILLS_SH_TRENDING_URL: &str = "https://skills.sh/trending";
const SKILLS_SH_HOT_URL: &str = "https://skills.sh/hot";
const DEFAULT_SEARCH_LIMIT: u64 = 10;
const MAX_SEARCH_LIMIT: u64 = 50;
const MAX_PREVIEW_SKILL_MD_BYTES: usize = 32 * 1024;

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

#[derive(Debug, Clone)]
struct ImportedSkillPackage {
    owner: String,
    repo: String,
    skill_id: String,
    skill_dir: String,
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

#[derive(Debug, Clone, Deserialize, Serialize)]
struct ImportedSkillMetadata {
    source_type: String,
    source_repo: String,
    source_skill_path: String,
    source_url: Option<String>,
    source_ref: String,
    source_commit: Option<String>,
    source_tree_sha: Option<String>,
    import_mode: Option<String>,
    registry_provider: Option<String>,
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
                .build()
                .expect("skill registry http client"),
            team_id,
            actor_id,
            info,
        }
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
                    "required": ["source", "skill_id"],
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
                    "required": ["source", "skill_id"],
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
                json!({
                    "rank": idx + 1,
                    "skill_id": item.skill_id,
                    "name": item.name,
                    "skill_ref": skill_ref,
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
                json!({
                    "rank": idx + 1,
                    "id": item.id,
                    "skill_id": item.skill_id,
                    "name": item.name,
                    "skill_ref": skill_ref,
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
        let package = self.resolve_skill_package(args).await?;
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
        let source = format!("{}/{}", package.owner, package.repo);
        let skill_ref = build_skill_ref(
            &format!("registry:{}", package.skill_id),
            &package.skill_id,
            "registry",
            &source,
        );

        Ok(json!({
            "team_id": self.team_id,
            "source": source,
            "skill_id": package.skill_id,
            "skill_ref": skill_ref,
            "display_line_zh": build_registry_display_line_zh(&skill_ref, &source),
            "display_line_en": build_registry_display_line_en(&skill_ref, &source),
            "plain_line_zh": build_registry_plain_line_zh(&package.skill_id, &source),
            "plain_line_en": build_registry_plain_line_en(&package.skill_id, &source),
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
        let package = self.resolve_skill_package(args).await?;
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

        Ok(json!({
            "team_id": self.team_id,
            "source": format!("{}/{}", package.owner, package.repo),
            "skill_id": package.skill_id,
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
        let package = self
            .resolve_skill_package_from_values(
                &source,
                &inspection.skill_id,
                Some(inspection.source_ref.as_str()),
            )
            .await?;
        let metadata = build_import_metadata(&package, Some(skill.visibility.as_str()));
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
        let metadata = build_import_metadata(package, visibility);
        coll.update_one(
            doc! { "_id": skill_oid },
            doc! { "$set": { "metadata": mongodb::bson::to_bson(&metadata)? } },
            None,
        )
        .await?;
        Ok(())
    }

    async fn resolve_skill_package(&self, args: &JsonObject) -> Result<ImportedSkillPackage> {
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
            .filter(|s| !s.is_empty())
            .ok_or_else(|| anyhow!("skill_id is required"))?;
        let source_ref = args
            .get("source_ref")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string);

        self.resolve_skill_package_from_values(source, skill_id, source_ref.as_deref())
            .await
    }

    async fn resolve_skill_package_from_values(
        &self,
        source: &str,
        skill_id: &str,
        source_ref: Option<&str>,
    ) -> Result<ImportedSkillPackage> {
        let (owner, repo) = parse_github_source(source).ok_or_else(|| {
            anyhow!(
                "Only GitHub-backed registry sources are supported in Phase 1. Unsupported source: {}",
                source
            )
        })?;

        let resolved_ref = match source_ref {
            Some(value) => value.to_string(),
            None => self.fetch_default_branch(&owner, &repo).await?,
        };
        let tree = self.fetch_git_tree(&owner, &repo, &resolved_ref).await?;
        let skill_dir = resolve_skill_dir(skill_id, &tree.tree).ok_or_else(|| {
            anyhow!(
                "Unable to locate SKILL.md for '{}' in repository {}/{}",
                skill_id,
                owner,
                repo
            )
        })?;

        let skill_md_path = format!("{}/SKILL.md", skill_dir);
        let skill_md = self
            .fetch_raw_text(&owner, &repo, &resolved_ref, &skill_md_path)
            .await
            .with_context(|| format!("failed to load {}", skill_md_path))?;
        let (frontmatter, body) = PackageService::parse_skill_md(&skill_md)
            .map_err(|e| anyhow!("invalid SKILL.md for '{}': {}", skill_id, e))?;

        let mut files = Vec::new();
        let mut skipped_files = Vec::new();
        for item in tree
            .tree
            .iter()
            .filter(|item| item.item_type == "blob")
            .filter(|item| item.path.starts_with(&(skill_dir.clone() + "/")))
            .filter(|item| item.path != skill_md_path)
        {
            let rel_path = item
                .path
                .strip_prefix(&(skill_dir.clone() + "/"))
                .unwrap_or(&item.path)
                .to_string();
            match self
                .fetch_raw_bytes(&owner, &repo, &resolved_ref, &item.path)
                .await
            {
                Ok(bytes) => match String::from_utf8(bytes) {
                    Ok(content) => files.push(SkillFile {
                        path: rel_path,
                        content,
                    }),
                    Err(_) => skipped_files.push(rel_path),
                },
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
            owner,
            repo,
            skill_id: skill_id.to_string(),
            source_tree_sha: resolve_tree_sha(&skill_dir, &tree.tree)
                .unwrap_or_else(|| tree.sha.clone()),
            skill_dir,
            source_ref: resolved_ref,
            source_commit: tree.sha,
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
            "metadata.source_type": "skills_sh_registry",
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
        let (owner, repo) = parse_github_source(&metadata.source_repo).ok_or_else(|| {
            anyhow!(
                "Imported skill '{}' has invalid source_repo '{}'",
                skill.name,
                metadata.source_repo
            )
        })?;
        let imported_skill_key = infer_imported_skill_id(&metadata, &skill.name);
        let package = self
            .resolve_skill_package_from_values(
                &metadata.source_repo,
                &imported_skill_key,
                Some(&metadata.source_ref),
            )
            .await?;
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
        let response = self
            .client
            .get(&url)
            .send()
            .await
            .with_context(|| format!("failed to fetch {}", path))?
            .error_for_status()
            .with_context(|| format!("raw GitHub fetch returned non-success for {}", path))?;
        Ok(response.bytes().await?.to_vec())
    }
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
) -> serde_json::Value {
    json!({
        "source_type": "skills_sh_registry",
        "source_repo": format!("{}/{}", package.owner, package.repo),
        "source_skill_path": package.skill_dir,
        "source_url": format!("https://skills.sh/{}/{}/{}", package.owner, package.repo, package.skill_id),
        "source_ref": package.source_ref,
        "source_commit": package.source_commit,
        "source_tree_sha": package.source_tree_sha,
        "import_mode": "registry_import",
        "registry_provider": "skills.sh",
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
        extract_initial_skills, infer_imported_skill_id, parse_github_source,
        parse_imported_metadata, resolve_skill_dir, resolve_tree_sha, GitHubTreeItem,
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
            "source_tree_sha": "treesha"
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
