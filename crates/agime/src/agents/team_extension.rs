//! Team collaboration extension
//!
//! This module provides MCP tools for team collaboration, allowing users to:
//! - Search and discover shared Skills, Recipes, and Extensions
//! - Load team skills into the current context
//! - Share resources with team members
//! - Check for updates to installed resources
//! - Install team resources locally (cross-platform)
//! - Uninstall locally installed team resources

use crate::agents::extension::PlatformExtensionContext;
use crate::agents::mcp_client::{Error, McpClientTrait};
use crate::config::paths::Paths;
use anyhow::Result;
use async_trait::async_trait;
use chrono::Utc;
use indoc::indoc;
use reqwest::Client;
use rmcp::model::{
    CallToolResult, Content, GetPromptResult, Implementation, InitializeResult, JsonObject,
    ListPromptsResult, ListResourcesResult, ListToolsResult, ProtocolVersion, ReadResourceResult,
    ServerCapabilities, ServerNotification, Tool, ToolAnnotations, ToolsCapability,
};
use schemars::{schema_for, JsonSchema};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::env;
use std::fs;
use std::path::PathBuf;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::warn;

pub static EXTENSION_NAME: &str = "team";

/// Default API base URL
const DEFAULT_API_URL: &str = "http://localhost:3000";

/// Environment variable for API URL override
const API_URL_ENV: &str = "AGIME_TEAM_API_URL";

/// Environment variable for API Key authentication (for remote team server)
const API_KEY_ENV: &str = "AGIME_TEAM_API_KEY";

// Tool parameter schemas
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct SearchResourcesParams {
    /// The type of resource to search: "skills", "recipes", or "extensions"
    pub resource_type: String,
    /// Optional search query
    pub query: Option<String>,
    /// Optional team ID to filter by
    pub team_id: Option<String>,
    /// Optional tags to filter by
    pub tags: Option<Vec<String>>,
    /// Maximum number of results (default: 20)
    pub limit: Option<u32>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct LoadSkillParams {
    /// The ID of the skill to load
    pub skill_id: String,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct ShareSkillParams {
    /// The team ID to share with
    pub team_id: String,
    /// The name of the skill
    pub name: String,
    /// The skill content (markdown)
    pub content: String,
    /// Optional description
    pub description: Option<String>,
    /// Optional tags
    pub tags: Option<Vec<String>>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct InstallResourceParams {
    /// The type of resource: "skill", "recipe", or "extension"
    pub resource_type: String,
    /// The ID of the resource to install
    pub resource_id: String,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct ListInstalledParams {
    /// Optional filter by resource type
    pub resource_type: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct UninstallLocalParams {
    /// The type of resource: "skill", "recipe", or "extension"
    pub resource_type: String,
    /// The ID of the resource to uninstall
    pub resource_id: String,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct ShareRecipeParams {
    /// The team ID to share with
    pub team_id: String,
    /// The name of the recipe
    pub name: String,
    /// The recipe content in YAML format
    pub content_yaml: String,
    /// Optional description
    pub description: Option<String>,
    /// Category: automation, data-processing, development, documentation, testing, deployment, other
    pub category: Option<String>,
    /// Optional tags
    pub tags: Option<Vec<String>>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct ShareExtensionParams {
    /// The team ID to share with
    pub team_id: String,
    /// The name of the extension
    pub name: String,
    /// The extension type: "stdio" or "sse"
    pub extension_type: String,
    /// Extension configuration as JSON string
    pub config: String,
    /// Optional description
    pub description: Option<String>,
    /// Optional tags
    pub tags: Option<Vec<String>>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct GetRecommendationsParams {
    /// Optional team ID to filter by
    pub team_id: Option<String>,
    /// Resource type filter: "skill", "recipe", or "extension"
    pub resource_type: Option<String>,
    /// Context keywords for content-based recommendations
    pub context: Option<String>,
    /// Preferred tags
    pub preferred_tags: Option<Vec<String>>,
    /// Maximum number of recommendations (default: 10)
    pub limit: Option<u32>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct ListTeamsParams {
    /// Include resource statistics for each team
    pub include_stats: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct GetStatsParams {
    /// Team ID to get stats for
    pub team_id: String,
    /// Resource type filter
    pub resource_type: Option<String>,
    /// Number of days to include (default: 30)
    pub days: Option<u32>,
}

/// Skill metadata for local storage (cross-platform)
/// This metadata file (.skill-meta.json) is stored alongside SKILL.md
/// to track team-sourced skills and their authorization status
#[derive(Debug, Serialize, Deserialize, Default)]
pub struct SkillMeta {
    /// Source of the skill: "local" or "team"
    #[serde(default)]
    pub source: Option<String>,
    /// Team ID (only for team skills)
    pub team_id: Option<String>,
    /// Resource ID in the team server
    pub resource_id: Option<String>,
    /// When the skill was installed
    pub installed_at: Option<String>,
    /// Version at installation time
    pub installed_version: Option<String>,
    /// Protection level of the skill
    pub protection_level: Option<String>,
    /// User ID who installed the skill
    pub user_id: Option<String>,
    /// Authorization information
    pub authorization: Option<SkillAuthorization>,
}

/// Authorization information for team skills
#[derive(Debug, Serialize, Deserialize)]
pub struct SkillAuthorization {
    /// Authorization token
    pub token: String,
    /// Token expiration time
    pub expires_at: String,
    /// Last time the authorization was verified
    pub last_verified_at: String,
}

/// Protection level enum matching the server definition
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProtectionLevel {
    Public,
    TeamInstallable,
    TeamOnlineOnly,
    Controlled,
}

impl ProtectionLevel {
    /// Check if this protection level allows local installation
    pub fn allows_local_install(&self) -> bool {
        matches!(self, ProtectionLevel::Public | ProtectionLevel::TeamInstallable)
    }
}

/// Sanitize skill name to prevent path traversal attacks
/// Only allows alphanumeric characters, underscores, and hyphens
fn sanitize_skill_name(name: &str) -> Result<String, String> {
    // Check for path traversal attempts
    if name.contains("..") || name.contains('/') || name.contains('\\') {
        return Err("Invalid skill name: path traversal characters not allowed".to_string());
    }

    // Check for empty or whitespace-only names
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err("Invalid skill name: name cannot be empty".to_string());
    }

    // Only allow alphanumeric, underscore, hyphen, and space
    // Also allow some common characters like dots (for versioned names)
    for c in trimmed.chars() {
        if !c.is_alphanumeric() && c != '_' && c != '-' && c != ' ' && c != '.' {
            return Err(format!(
                "Invalid skill name: character '{}' not allowed. Use only alphanumeric, underscore, hyphen, space, or dot.",
                c
            ));
        }
    }

    // Replace spaces with hyphens for filesystem compatibility
    let safe_name = trimmed.replace(' ', "-");

    // Ensure name doesn't start with a dot (hidden files)
    if safe_name.starts_with('.') {
        return Err("Invalid skill name: name cannot start with a dot".to_string());
    }

    Ok(safe_name)
}

/// HTTP client for team API
struct TeamApiClient {
    client: Client,
    base_url: String,
    api_key: Option<String>,
}

impl TeamApiClient {
    fn new() -> Self {
        let base_url = Self::get_base_url();
        let api_key = env::var(API_KEY_ENV).ok();
        Self {
            client: Client::new(),
            base_url,
            api_key,
        }
    }

    /// Get the base URL for team API
    /// Priority: AGIME_TEAM_API_URL > AGIME_API_HOST > default
    fn get_base_url() -> String {
        // 1. Explicit team API URL (highest priority)
        if let Ok(url) = env::var(API_URL_ENV) {
            if !url.is_empty() {
                return url;
            }
        }

        // 2. General API host (set by agimed at startup)
        if let Ok(host) = env::var("AGIME_API_HOST") {
            if !host.is_empty() {
                return host;
            }
        }

        // 3. Fallback to default (for standalone team server)
        DEFAULT_API_URL.to_string()
    }

    fn api_url(&self, path: &str) -> String {
        format!("{}/api/team{}", self.base_url, path)
    }

    /// Create a GET request with optional authentication
    fn get(&self, url: &str) -> reqwest::RequestBuilder {
        let mut req = self.client.get(url);
        if let Some(ref key) = self.api_key {
            req = req.header("X-API-Key", key);
        }
        req
    }

    /// Create a POST request with optional authentication
    fn post(&self, url: &str) -> reqwest::RequestBuilder {
        let mut req = self.client.post(url);
        if let Some(ref key) = self.api_key {
            req = req.header("X-API-Key", key);
        }
        req
    }
}

pub struct TeamClient {
    info: InitializeResult,
    #[allow(dead_code)]
    context: PlatformExtensionContext,
    api: TeamApiClient,
}

impl TeamClient {
    pub fn new(context: PlatformExtensionContext) -> Result<Self> {
        let api = TeamApiClient::new();
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
            },
            server_info: Implementation {
                name: EXTENSION_NAME.to_string(),
                title: Some("Team Collaboration".to_string()),
                version: "1.0.0".to_string(),
                icons: None,
                website_url: None,
            },
            instructions: Some(
                indoc! {r#"
                Team Collaboration Tools

                Use these tools to collaborate with your team by sharing and discovering resources:

                **Discover:**
                - team_list: List all teams you belong to
                - team_search: Find Skills, Recipes, or Extensions shared by your team
                - team_get_recommendations: Get personalized resource recommendations
                - team_get_stats: View usage statistics for team resources

                **Load and Use:**
                - team_load_skill: Load a skill's content into the conversation context
                - team_install: Install a resource locally for use

                **Share:**
                - team_share_skill: Share a skill with your team
                - team_share_recipe: Share a workflow recipe with your team
                - team_share_extension: Share an MCP extension configuration

                **Manage:**
                - team_list_installed: See all installed team resources
                - team_check_updates: Check for updates to installed resources
                - team_uninstall_local: Remove locally installed resources

                Tips:
                - Use team_list first to see which teams you belong to
                - Use team_get_recommendations to discover relevant resources
                - Search before creating to avoid duplicating existing resources
                - Always include descriptive tags when sharing
                - Check for updates regularly to get the latest improvements
                - Extensions require security review before team members can use them
            "#}
                .to_string(),
            ),
        };

        Ok(Self { info, context, api })
    }

    fn get_tools() -> Vec<Tool> {
        let mut tools = Vec::new();

        // Search resources tool
        let search_schema = schema_for!(SearchResourcesParams);
        let search_schema_value = serde_json::to_value(search_schema)
            .expect("Failed to serialize SearchResourcesParams schema");
        tools.push(
            Tool::new(
                "team_search".to_string(),
                indoc! {r#"
                    Search for Skills, Recipes, or Extensions shared by your team.

                    Use this to discover existing resources before creating new ones.
                    Specify the resource_type: "skills", "recipes", or "extensions".
                "#}
                .to_string(),
                search_schema_value.as_object().unwrap().clone(),
            )
            .annotate(ToolAnnotations {
                title: Some("Search Team Resources".to_string()),
                read_only_hint: Some(true),
                destructive_hint: Some(false),
                idempotent_hint: Some(true),
                open_world_hint: Some(false),
            }),
        );

        // Load skill tool
        let load_schema = schema_for!(LoadSkillParams);
        let load_schema_value =
            serde_json::to_value(load_schema).expect("Failed to serialize LoadSkillParams schema");
        tools.push(
            Tool::new(
                "team_load_skill".to_string(),
                indoc! {r#"
                    Load a team skill's content into the current conversation context.

                    This retrieves the full skill content so you can use it.
                "#}
                .to_string(),
                load_schema_value.as_object().unwrap().clone(),
            )
            .annotate(ToolAnnotations {
                title: Some("Load Team Skill".to_string()),
                read_only_hint: Some(true),
                destructive_hint: Some(false),
                idempotent_hint: Some(true),
                open_world_hint: Some(false),
            }),
        );

        // Share skill tool
        let share_schema = schema_for!(ShareSkillParams);
        let share_schema_value = serde_json::to_value(share_schema)
            .expect("Failed to serialize ShareSkillParams schema");
        tools.push(
            Tool::new(
                "team_share_skill".to_string(),
                indoc! {r#"
                    Share a skill with your team.

                    Provide the team_id, a descriptive name, and the skill content.
                    Adding tags helps others discover your skill.
                "#}
                .to_string(),
                share_schema_value.as_object().unwrap().clone(),
            )
            .annotate(ToolAnnotations {
                title: Some("Share Skill".to_string()),
                read_only_hint: Some(false),
                destructive_hint: Some(false),
                idempotent_hint: Some(false),
                open_world_hint: Some(false),
            }),
        );

        // Install resource tool
        let install_schema = schema_for!(InstallResourceParams);
        let install_schema_value = serde_json::to_value(install_schema)
            .expect("Failed to serialize InstallResourceParams schema");
        tools.push(
            Tool::new(
                "team_install".to_string(),
                indoc! {r#"
                    Install a team resource locally.

                    Specify the resource_type ("skill", "recipe", or "extension") and resource_id.
                "#}
                .to_string(),
                install_schema_value.as_object().unwrap().clone(),
            )
            .annotate(ToolAnnotations {
                title: Some("Install Resource".to_string()),
                read_only_hint: Some(false),
                destructive_hint: Some(false),
                idempotent_hint: Some(true),
                open_world_hint: Some(false),
            }),
        );

        // List installed tool
        let list_schema = schema_for!(ListInstalledParams);
        let list_schema_value = serde_json::to_value(list_schema)
            .expect("Failed to serialize ListInstalledParams schema");
        tools.push(
            Tool::new(
                "team_list_installed".to_string(),
                indoc! {r#"
                    List all installed team resources.

                    Optionally filter by resource_type: "skill", "recipe", or "extension".
                "#}
                .to_string(),
                list_schema_value.as_object().unwrap().clone(),
            )
            .annotate(ToolAnnotations {
                title: Some("List Installed Resources".to_string()),
                read_only_hint: Some(true),
                destructive_hint: Some(false),
                idempotent_hint: Some(true),
                open_world_hint: Some(false),
            }),
        );

        // Check updates tool (no params needed)
        tools.push(
            Tool::new(
                "team_check_updates".to_string(),
                "Check for updates to all installed team resources.".to_string(),
                serde_json::json!({
                    "type": "object",
                    "properties": {},
                    "required": []
                })
                .as_object()
                .unwrap()
                .clone(),
            )
            .annotate(ToolAnnotations {
                title: Some("Check for Updates".to_string()),
                read_only_hint: Some(true),
                destructive_hint: Some(false),
                idempotent_hint: Some(true),
                open_world_hint: Some(false),
            }),
        );

        // Uninstall local tool
        let uninstall_schema = schema_for!(UninstallLocalParams);
        let uninstall_schema_value = serde_json::to_value(uninstall_schema)
            .expect("Failed to serialize UninstallLocalParams schema");
        tools.push(
            Tool::new(
                "team_uninstall_local".to_string(),
                indoc! {r#"
                    Uninstall a team resource from local storage.

                    Removes the locally installed skill, recipe, or extension files.
                    Specify the resource_type ("skill", "recipe", or "extension") and resource_id.
                "#}
                .to_string(),
                uninstall_schema_value.as_object().unwrap().clone(),
            )
            .annotate(ToolAnnotations {
                title: Some("Uninstall Local Resource".to_string()),
                read_only_hint: Some(false),
                destructive_hint: Some(true),
                idempotent_hint: Some(true),
                open_world_hint: Some(false),
            }),
        );

        // Share recipe tool
        let share_recipe_schema = schema_for!(ShareRecipeParams);
        let share_recipe_schema_value = serde_json::to_value(share_recipe_schema)
            .expect("Failed to serialize ShareRecipeParams schema");
        tools.push(
            Tool::new(
                "team_share_recipe".to_string(),
                indoc! {r#"
                    Share a recipe (workflow automation) with your team.

                    Provide the team_id, a descriptive name, and the recipe content in YAML format.
                    Categories: automation, data-processing, development, documentation, testing, deployment, other.
                "#}
                .to_string(),
                share_recipe_schema_value.as_object().unwrap().clone(),
            )
            .annotate(ToolAnnotations {
                title: Some("Share Recipe".to_string()),
                read_only_hint: Some(false),
                destructive_hint: Some(false),
                idempotent_hint: Some(false),
                open_world_hint: Some(false),
            }),
        );

        // Share extension tool
        let share_extension_schema = schema_for!(ShareExtensionParams);
        let share_extension_schema_value = serde_json::to_value(share_extension_schema)
            .expect("Failed to serialize ShareExtensionParams schema");
        tools.push(
            Tool::new(
                "team_share_extension".to_string(),
                indoc! {r#"
                    Share an MCP extension configuration with your team.

                    Provide the team_id, name, extension_type ("stdio" or "sse"), and config (JSON).
                    Extensions allow team members to use shared tools and integrations.
                "#}
                .to_string(),
                share_extension_schema_value.as_object().unwrap().clone(),
            )
            .annotate(ToolAnnotations {
                title: Some("Share Extension".to_string()),
                read_only_hint: Some(false),
                destructive_hint: Some(false),
                idempotent_hint: Some(false),
                open_world_hint: Some(false),
            }),
        );

        // Get recommendations tool
        let recommendations_schema = schema_for!(GetRecommendationsParams);
        let recommendations_schema_value = serde_json::to_value(recommendations_schema)
            .expect("Failed to serialize GetRecommendationsParams schema");
        tools.push(
            Tool::new(
                "team_get_recommendations".to_string(),
                indoc! {r#"
                    Get personalized resource recommendations.

                    Returns recommended Skills, Recipes, and Extensions based on:
                    - Popularity among team members
                    - Your activity history
                    - Content similarity to your context
                    - Trending resources
                    - Newly added resources
                "#}
                .to_string(),
                recommendations_schema_value.as_object().unwrap().clone(),
            )
            .annotate(ToolAnnotations {
                title: Some("Get Recommendations".to_string()),
                read_only_hint: Some(true),
                destructive_hint: Some(false),
                idempotent_hint: Some(true),
                open_world_hint: Some(false),
            }),
        );

        // List teams tool
        let list_teams_schema = schema_for!(ListTeamsParams);
        let list_teams_schema_value = serde_json::to_value(list_teams_schema)
            .expect("Failed to serialize ListTeamsParams schema");
        tools.push(
            Tool::new(
                "team_list".to_string(),
                indoc! {r#"
                    List all teams you belong to.

                    Optionally include statistics (resource counts, member counts) for each team.
                "#}
                .to_string(),
                list_teams_schema_value.as_object().unwrap().clone(),
            )
            .annotate(ToolAnnotations {
                title: Some("List Teams".to_string()),
                read_only_hint: Some(true),
                destructive_hint: Some(false),
                idempotent_hint: Some(true),
                open_world_hint: Some(false),
            }),
        );

        // Get stats tool
        let stats_schema = schema_for!(GetStatsParams);
        let stats_schema_value = serde_json::to_value(stats_schema)
            .expect("Failed to serialize GetStatsParams schema");
        tools.push(
            Tool::new(
                "team_get_stats".to_string(),
                indoc! {r#"
                    Get usage statistics for a team's resources.

                    Returns metrics like resource usage counts, trending resources, and activity over time.
                "#}
                .to_string(),
                stats_schema_value.as_object().unwrap().clone(),
            )
            .annotate(ToolAnnotations {
                title: Some("Get Team Stats".to_string()),
                read_only_hint: Some(true),
                destructive_hint: Some(false),
                idempotent_hint: Some(true),
                open_world_hint: Some(false),
            }),
        );

        tools
    }

    /// Get the skills directory path (cross-platform)
    /// Uses etcetera for platform-specific paths with fallback to traditional ~/.agime/skills/
    fn get_skills_directory() -> PathBuf {
        let config_dir = Paths::config_dir();
        let skills_dir = config_dir.join("skills");

        // Ensure directory exists
        if !skills_dir.exists() {
            if let Err(e) = fs::create_dir_all(&skills_dir) {
                warn!("Failed to create skills directory: {}", e);
            }
        }

        skills_dir
    }

    /// Write skill files to local storage
    /// Creates SKILL.md and .skill-meta.json in the skill directory
    fn write_skill_files(
        install_path: &PathBuf,
        skill_name: &str,
        content: &str,
        meta: &SkillMeta,
    ) -> Result<(), String> {
        // ERR-2 FIX: Track whether we created the directory so we can clean up on failure
        let dir_existed = install_path.exists();

        // Create the skill directory
        fs::create_dir_all(install_path).map_err(|e| format!("Failed to create directory: {}", e))?;

        // Helper to clean up on failure
        let cleanup_on_error = |err_msg: String| -> String {
            if !dir_existed {
                // We created this directory, try to remove it on failure
                if let Err(cleanup_err) = fs::remove_dir_all(install_path) {
                    tracing::warn!(
                        "Failed to cleanup directory {} after failed installation: {}",
                        install_path.display(),
                        cleanup_err
                    );
                }
            }
            err_msg
        };

        // Write SKILL.md
        let skill_file = install_path.join("SKILL.md");
        if let Err(e) = fs::write(&skill_file, content) {
            return Err(cleanup_on_error(format!("Failed to write SKILL.md: {}", e)));
        }

        // Serialize metadata first (before writing, to catch serialization errors early)
        let meta_json = match serde_json::to_string_pretty(meta) {
            Ok(json) => json,
            Err(e) => return Err(cleanup_on_error(format!("Failed to serialize metadata: {}", e))),
        };

        // Write .skill-meta.json
        let meta_file = install_path.join(".skill-meta.json");
        if let Err(e) = fs::write(&meta_file, &meta_json) {
            return Err(cleanup_on_error(format!("Failed to write .skill-meta.json: {}", e)));
        }

        tracing::info!(
            "Installed skill '{}' to {}",
            skill_name,
            install_path.display()
        );

        Ok(())
    }

    /// Check if a protection level string allows local installation
    fn allows_local_install_str(protection_level: &str) -> bool {
        matches!(
            protection_level,
            "public" | "team_installable" | "Public" | "TeamInstallable"
        )
    }

    async fn handle_search(&self, arguments: Option<JsonObject>) -> Result<Vec<Content>, String> {
        let args = arguments.ok_or("Missing arguments")?;

        let resource_type = args
            .get("resource_type")
            .and_then(|v| v.as_str())
            .ok_or("Missing required parameter: resource_type")?;

        let query = args.get("query").and_then(|v| v.as_str());
        let team_id = args.get("team_id").and_then(|v| v.as_str());
        let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(20) as u32;

        // Build query parameters
        let mut params = vec![("limit", limit.to_string())];
        if let Some(q) = query {
            params.push(("search", q.to_string()));
        }
        if let Some(tid) = team_id {
            params.push(("teamId", tid.to_string()));
        }
        if let Some(tags) = args.get("tags").and_then(|v| v.as_array()) {
            let tags_str: Vec<String> = tags
                .iter()
                .filter_map(|t| t.as_str().map(|s| s.to_string()))
                .collect();
            if !tags_str.is_empty() {
                params.push(("tags", tags_str.join(",")));
            }
        }

        // Determine endpoint based on resource type
        let endpoint = match resource_type {
            "skills" => "/skills",
            "recipes" => "/recipes",
            "extensions" => "/extensions",
            _ => {
                return Err(format!(
                    "Invalid resource_type: {}. Use 'skills', 'recipes', or 'extensions'",
                    resource_type
                ))
            }
        };

        let url = self.api.api_url(endpoint);

        match self.api.get(&url).query(&params).send().await {
            Ok(response) => {
                if response.status().is_success() {
                    match response.json::<Value>().await {
                        Ok(data) => Ok(vec![Content::text(
                            serde_json::to_string_pretty(&data).unwrap(),
                        )]),
                        Err(e) => Err(format!("Failed to parse response: {}", e)),
                    }
                } else {
                    let status = response.status();
                    let error_text = response.text().await.unwrap_or_default();
                    Err(format!("API error ({}): {}", status, error_text))
                }
            }
            Err(e) => {
                warn!("Team API request failed: {}", e);
                // Return friendly message if server is not available
                let response = serde_json::json!({
                    "resource_type": resource_type,
                    "query": query,
                    "team_id": team_id,
                    "limit": limit,
                    "results": [],
                    "status": "unavailable",
                    "message": format!("Team server unavailable: {}. Make sure the server is running with the team feature enabled.", e)
                });
                Ok(vec![Content::text(
                    serde_json::to_string_pretty(&response).unwrap(),
                )])
            }
        }
    }

    async fn handle_load_skill(
        &self,
        arguments: Option<JsonObject>,
    ) -> Result<Vec<Content>, String> {
        let args = arguments.ok_or("Missing arguments")?;

        let skill_id = args
            .get("skill_id")
            .and_then(|v| v.as_str())
            .ok_or("Missing required parameter: skill_id")?;

        let url = self.api.api_url(&format!("/skills/{}", skill_id));

        match self.api.get(&url).send().await {
            Ok(response) => {
                if response.status().is_success() {
                    match response.json::<Value>().await {
                        Ok(data) => {
                            // Return the skill content in a format suitable for the AI
                            let result = serde_json::json!({
                                "skill_id": skill_id,
                                "status": "loaded",
                                "skill": data
                            });
                            Ok(vec![Content::text(
                                serde_json::to_string_pretty(&result).unwrap(),
                            )])
                        }
                        Err(e) => Err(format!("Failed to parse response: {}", e)),
                    }
                } else if response.status().as_u16() == 404 {
                    Err(format!("Skill not found: {}", skill_id))
                } else {
                    let status = response.status();
                    let error_text = response.text().await.unwrap_or_default();
                    Err(format!("API error ({}): {}", status, error_text))
                }
            }
            Err(e) => {
                warn!("Team API request failed: {}", e);
                let response = serde_json::json!({
                    "skill_id": skill_id,
                    "status": "unavailable",
                    "message": format!("Team server unavailable: {}", e)
                });
                Ok(vec![Content::text(
                    serde_json::to_string_pretty(&response).unwrap(),
                )])
            }
        }
    }

    async fn handle_share_skill(
        &self,
        arguments: Option<JsonObject>,
    ) -> Result<Vec<Content>, String> {
        let args = arguments.ok_or("Missing arguments")?;

        let team_id = args
            .get("team_id")
            .and_then(|v| v.as_str())
            .ok_or("Missing required parameter: team_id")?;
        let name = args
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or("Missing required parameter: name")?;
        let content = args
            .get("content")
            .and_then(|v| v.as_str())
            .ok_or("Missing required parameter: content")?;

        let description = args.get("description").and_then(|v| v.as_str());
        let tags: Option<Vec<String>> = args.get("tags").and_then(|v| {
            v.as_array().map(|arr| {
                arr.iter()
                    .filter_map(|t| t.as_str().map(|s| s.to_string()))
                    .collect()
            })
        });

        let request_body = serde_json::json!({
            "teamId": team_id,
            "name": name,
            "content": content,
            "description": description,
            "tags": tags
        });

        let url = self.api.api_url("/skills");

        match self.api.post(&url).json(&request_body).send().await {
            Ok(response) => {
                if response.status().is_success() {
                    match response.json::<Value>().await {
                        Ok(data) => {
                            let result = serde_json::json!({
                                "status": "shared",
                                "skill": data,
                                "message": format!("Skill '{}' has been shared with the team.", name)
                            });
                            Ok(vec![Content::text(
                                serde_json::to_string_pretty(&result).unwrap(),
                            )])
                        }
                        Err(e) => Err(format!("Failed to parse response: {}", e)),
                    }
                } else {
                    let status = response.status();
                    let error_text = response.text().await.unwrap_or_default();
                    Err(format!(
                        "Failed to share skill ({}): {}",
                        status, error_text
                    ))
                }
            }
            Err(e) => {
                warn!("Team API request failed: {}", e);
                let response = serde_json::json!({
                    "team_id": team_id,
                    "name": name,
                    "status": "unavailable",
                    "message": format!("Team server unavailable: {}", e)
                });
                Ok(vec![Content::text(
                    serde_json::to_string_pretty(&response).unwrap(),
                )])
            }
        }
    }

    async fn handle_install(&self, arguments: Option<JsonObject>) -> Result<Vec<Content>, String> {
        let args = arguments.ok_or("Missing arguments")?;

        let resource_type = args
            .get("resource_type")
            .and_then(|v| v.as_str())
            .ok_or("Missing required parameter: resource_type")?;
        let resource_id = args
            .get("resource_id")
            .and_then(|v| v.as_str())
            .ok_or("Missing required parameter: resource_id")?;

        // For skills, we also install locally
        if resource_type == "skill" {
            return self.handle_install_skill_local(resource_id).await;
        }

        // For recipes and extensions, just call server API (local install not yet supported)
        let endpoint = match resource_type {
            "recipe" => format!("/recipes/{}/install", resource_id),
            "extension" => format!("/extensions/{}/install", resource_id),
            _ => {
                return Err(format!(
                    "Invalid resource_type: {}. Use 'skill', 'recipe', or 'extension'",
                    resource_type
                ))
            }
        };

        let url = self.api.api_url(&endpoint);

        match self.api.post(&url).send().await {
            Ok(response) => {
                if response.status().is_success() {
                    match response.json::<Value>().await {
                        Ok(data) => {
                            let result = serde_json::json!({
                                "status": "installed",
                                "resource_type": resource_type,
                                "resource_id": resource_id,
                                "result": data,
                                "message": format!("{} '{}' has been installed successfully.", resource_type, resource_id)
                            });
                            Ok(vec![Content::text(
                                serde_json::to_string_pretty(&result).unwrap(),
                            )])
                        }
                        Err(e) => Err(format!("Failed to parse response: {}", e)),
                    }
                } else if response.status().as_u16() == 404 {
                    Err(format!(
                        "Resource not found: {} {}",
                        resource_type, resource_id
                    ))
                } else {
                    let status = response.status();
                    let error_text = response.text().await.unwrap_or_default();
                    Err(format!("Failed to install ({}): {}", status, error_text))
                }
            }
            Err(e) => {
                warn!("Team API request failed: {}", e);
                let response = serde_json::json!({
                    "resource_type": resource_type,
                    "resource_id": resource_id,
                    "status": "unavailable",
                    "message": format!("Team server unavailable: {}", e)
                });
                Ok(vec![Content::text(
                    serde_json::to_string_pretty(&response).unwrap(),
                )])
            }
        }
    }

    /// Install a skill locally with metadata
    async fn handle_install_skill_local(&self, skill_id: &str) -> Result<Vec<Content>, String> {
        // 1. Fetch skill details from server
        let skill_url = self.api.api_url(&format!("/skills/{}", skill_id));

        let skill_data: Value = match self.api.get(&skill_url).send().await {
            Ok(response) => {
                if response.status().is_success() {
                    response.json().await.map_err(|e| format!("Failed to parse skill data: {}", e))?
                } else if response.status().as_u16() == 404 {
                    return Err(format!("Skill not found: {}", skill_id));
                } else {
                    let status = response.status();
                    let error_text = response.text().await.unwrap_or_default();
                    return Err(format!("Failed to fetch skill ({}): {}", status, error_text));
                }
            }
            Err(e) => {
                return Err(format!("Team server unavailable: {}", e));
            }
        };

        // 2. Extract skill properties
        let name = skill_data.get("name")
            .and_then(|v| v.as_str())
            .ok_or("Skill has no name")?;

        // Validate and sanitize skill name to prevent path traversal
        let safe_name = sanitize_skill_name(name)?;

        let content = skill_data.get("content")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let protection_level = skill_data.get("protectionLevel")
            .and_then(|v| v.as_str())
            .unwrap_or("team_installable");
        let team_id = skill_data.get("teamId")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let version = skill_data.get("version")
            .and_then(|v| v.as_str())
            .unwrap_or("1.0.0");

        // 3. Check protection level
        if !Self::allows_local_install_str(protection_level) {
            return Err(format!(
                "Skill '{}' has protection level '{}' which does not allow local installation. Use team_load_skill for online access.",
                name, protection_level
            ));
        }

        // 4. Get authorization token from verify-access API
        // ERR-1 FIX: Don't silently ignore authorization failures for non-public skills
        let (auth_token, auth_expires) = match self.get_authorization_token(skill_id).await {
            Ok((token, expires)) => (token, expires),
            Err(e) => {
                // Only allow empty authorization for public skills
                if protection_level == "public" {
                    tracing::info!("Skipping authorization for public skill");
                    (String::new(), String::new())
                } else {
                    // For non-public skills, authorization is required
                    return Err(format!(
                        "Failed to authorize installation of skill '{}': {}. Please ensure you have access to this skill.",
                        name, e
                    ));
                }
            }
        };

        // 5. Prepare installation path using sanitized name
        let skills_dir = Self::get_skills_directory();
        let install_path = skills_dir.join(&safe_name);

        // 6. Create metadata with real authorization token
        let now = Utc::now().to_rfc3339();
        let meta = SkillMeta {
            source: Some("team".to_string()),
            team_id: Some(team_id.to_string()),
            resource_id: Some(skill_id.to_string()),
            installed_at: Some(now.clone()),
            installed_version: Some(version.to_string()),
            protection_level: Some(protection_level.to_string()),
            user_id: None, // Could be set from context if available
            authorization: Some(SkillAuthorization {
                token: auth_token,
                expires_at: auth_expires,
                last_verified_at: now,
            }),
        };

        // 7. Write files locally
        Self::write_skill_files(&install_path, &safe_name, content, &meta)?;

        // 8. Also notify server of installation (optional, for tracking)
        let server_install_url = self.api.api_url(&format!("/skills/{}/install", skill_id));
        let _ = self.api.post(&server_install_url).send().await;

        let result = serde_json::json!({
            "status": "installed",
            "resource_type": "skill",
            "resource_id": skill_id,
            "name": safe_name,
            "install_path": install_path.display().to_string(),
            "message": format!("Skill '{}' has been installed locally to {}. You can now use it with the skills extension.", safe_name, install_path.display())
        });

        Ok(vec![Content::text(serde_json::to_string_pretty(&result).unwrap())])
    }

    /// Get authorization token from verify-access API
    async fn get_authorization_token(&self, skill_id: &str) -> Result<(String, String), String> {
        let verify_url = self.api.api_url(&format!("/skills/{}/verify-access", skill_id));

        match self.api.post(&verify_url).send().await {
            Ok(response) => {
                if response.status().is_success() {
                    let data: Value = response.json().await
                        .map_err(|e| format!("Failed to parse verify-access response: {}", e))?;

                    let token = data.get("token")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let expires_at = data.get("expiresAt")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();

                    Ok((token, expires_at))
                } else {
                    let status = response.status();
                    let error_text = response.text().await.unwrap_or_default();
                    Err(format!("Failed to verify access ({}): {}", status, error_text))
                }
            }
            Err(e) => Err(format!("Failed to call verify-access API: {}", e)),
        }
    }

    /// Uninstall a locally installed team resource
    async fn handle_uninstall_local(
        &self,
        arguments: Option<JsonObject>,
    ) -> Result<Vec<Content>, String> {
        let args = arguments.ok_or("Missing arguments")?;

        let resource_type = args
            .get("resource_type")
            .and_then(|v| v.as_str())
            .ok_or("Missing required parameter: resource_type")?;
        let resource_id = args
            .get("resource_id")
            .and_then(|v| v.as_str())
            .ok_or("Missing required parameter: resource_id")?;

        match resource_type {
            "skill" => self.uninstall_skill_local(resource_id).await,
            "recipe" | "extension" => {
                // TODO: Implement for recipes and extensions
                Err(format!(
                    "Local uninstallation of {} is not yet supported",
                    resource_type
                ))
            }
            _ => Err(format!(
                "Invalid resource_type: {}. Use 'skill', 'recipe', or 'extension'",
                resource_type
            )),
        }
    }

    /// Uninstall a skill from local storage
    async fn uninstall_skill_local(&self, resource_id: &str) -> Result<Vec<Content>, String> {
        let skills_dir = Self::get_skills_directory();

        // Scan skills directory to find the skill with matching resource_id
        let entries = fs::read_dir(&skills_dir)
            .map_err(|e| format!("Failed to read skills directory: {}", e))?;

        for entry in entries {
            let entry = entry.map_err(|e| format!("Failed to read entry: {}", e))?;
            let skill_path = entry.path();

            if !skill_path.is_dir() {
                continue;
            }

            let meta_path = skill_path.join(".skill-meta.json");
            if !meta_path.exists() {
                continue;
            }

            // Read and parse metadata
            let meta_content = fs::read_to_string(&meta_path)
                .map_err(|e| format!("Failed to read metadata: {}", e))?;
            let meta: SkillMeta = serde_json::from_str(&meta_content)
                .map_err(|e| format!("Failed to parse metadata: {}", e))?;

            // Check if this is the skill we're looking for
            if meta.resource_id.as_deref() == Some(resource_id) {
                let skill_name = skill_path.file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("unknown");

                // Remove the entire skill directory
                fs::remove_dir_all(&skill_path)
                    .map_err(|e| format!("Failed to remove skill directory: {}", e))?;

                tracing::info!("Uninstalled skill '{}' from {}", skill_name, skill_path.display());

                let result = serde_json::json!({
                    "status": "uninstalled",
                    "resource_type": "skill",
                    "resource_id": resource_id,
                    "name": skill_name,
                    "message": format!("Skill '{}' has been uninstalled from local storage.", skill_name)
                });

                return Ok(vec![Content::text(serde_json::to_string_pretty(&result).unwrap())]);
            }
        }

        Err(format!(
            "Skill with resource_id '{}' not found in local storage",
            resource_id
        ))
    }

    async fn handle_list_installed(
        &self,
        arguments: Option<JsonObject>,
    ) -> Result<Vec<Content>, String> {
        let resource_type = arguments
            .as_ref()
            .and_then(|args| args.get("resource_type"))
            .and_then(|v| v.as_str());

        let mut params = vec![];
        if let Some(rt) = resource_type {
            params.push(("resourceType", rt.to_string()));
        }

        let url = self.api.api_url("/installed");

        match self.api.get(&url).query(&params).send().await {
            Ok(response) => {
                if response.status().is_success() {
                    match response.json::<Value>().await {
                        Ok(data) => Ok(vec![Content::text(
                            serde_json::to_string_pretty(&data).unwrap(),
                        )]),
                        Err(e) => Err(format!("Failed to parse response: {}", e)),
                    }
                } else {
                    let status = response.status();
                    let error_text = response.text().await.unwrap_or_default();
                    Err(format!("API error ({}): {}", status, error_text))
                }
            }
            Err(e) => {
                warn!("Team API request failed: {}", e);
                let response = serde_json::json!({
                    "resource_type": resource_type,
                    "resources": [],
                    "status": "unavailable",
                    "message": format!("Team server unavailable: {}", e)
                });
                Ok(vec![Content::text(
                    serde_json::to_string_pretty(&response).unwrap(),
                )])
            }
        }
    }

    async fn handle_check_updates(&self) -> Result<Vec<Content>, String> {
        let url = self.api.api_url("/resources/check-updates");

        match self
            .api
            .client
            .post(&url)
            .json(&serde_json::json!({}))
            .send()
            .await
        {
            Ok(response) => {
                if response.status().is_success() {
                    match response.json::<Value>().await {
                        Ok(data) => {
                            let result = serde_json::json!({
                                "status": "checked",
                                "result": data
                            });
                            Ok(vec![Content::text(
                                serde_json::to_string_pretty(&result).unwrap(),
                            )])
                        }
                        Err(e) => Err(format!("Failed to parse response: {}", e)),
                    }
                } else {
                    let status = response.status();
                    let error_text = response.text().await.unwrap_or_default();
                    Err(format!("API error ({}): {}", status, error_text))
                }
            }
            Err(e) => {
                warn!("Team API request failed: {}", e);
                let response = serde_json::json!({
                    "updates_available": 0,
                    "resources": [],
                    "status": "unavailable",
                    "message": format!("Team server unavailable: {}", e)
                });
                Ok(vec![Content::text(
                    serde_json::to_string_pretty(&response).unwrap(),
                )])
            }
        }
    }

    async fn handle_share_recipe(
        &self,
        arguments: Option<JsonObject>,
    ) -> Result<Vec<Content>, String> {
        let args = arguments.ok_or("Missing arguments")?;

        let team_id = args
            .get("team_id")
            .and_then(|v| v.as_str())
            .ok_or("Missing required parameter: team_id")?;
        let name = args
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or("Missing required parameter: name")?;
        let content_yaml = args
            .get("content_yaml")
            .and_then(|v| v.as_str())
            .ok_or("Missing required parameter: content_yaml")?;

        let description = args.get("description").and_then(|v| v.as_str());
        let category = args.get("category").and_then(|v| v.as_str());
        let tags: Option<Vec<String>> = args.get("tags").and_then(|v| {
            v.as_array().map(|arr| {
                arr.iter()
                    .filter_map(|t| t.as_str().map(|s| s.to_string()))
                    .collect()
            })
        });

        let request_body = serde_json::json!({
            "teamId": team_id,
            "name": name,
            "contentYaml": content_yaml,
            "description": description,
            "category": category.unwrap_or("other"),
            "tags": tags
        });

        let url = self.api.api_url("/recipes");

        match self.api.post(&url).json(&request_body).send().await {
            Ok(response) => {
                if response.status().is_success() {
                    match response.json::<Value>().await {
                        Ok(data) => {
                            let result = serde_json::json!({
                                "status": "shared",
                                "recipe": data,
                                "message": format!("Recipe '{}' has been shared with the team.", name)
                            });
                            Ok(vec![Content::text(
                                serde_json::to_string_pretty(&result).unwrap(),
                            )])
                        }
                        Err(e) => Err(format!("Failed to parse response: {}", e)),
                    }
                } else {
                    let status = response.status();
                    let error_text = response.text().await.unwrap_or_default();
                    Err(format!(
                        "Failed to share recipe ({}): {}",
                        status, error_text
                    ))
                }
            }
            Err(e) => {
                warn!("Team API request failed: {}", e);
                let response = serde_json::json!({
                    "team_id": team_id,
                    "name": name,
                    "status": "unavailable",
                    "message": format!("Team server unavailable: {}", e)
                });
                Ok(vec![Content::text(
                    serde_json::to_string_pretty(&response).unwrap(),
                )])
            }
        }
    }

    async fn handle_share_extension(
        &self,
        arguments: Option<JsonObject>,
    ) -> Result<Vec<Content>, String> {
        let args = arguments.ok_or("Missing arguments")?;

        let team_id = args
            .get("team_id")
            .and_then(|v| v.as_str())
            .ok_or("Missing required parameter: team_id")?;
        let name = args
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or("Missing required parameter: name")?;
        let extension_type = args
            .get("extension_type")
            .and_then(|v| v.as_str())
            .ok_or("Missing required parameter: extension_type")?;
        let config = args
            .get("config")
            .and_then(|v| v.as_str())
            .ok_or("Missing required parameter: config")?;

        // Validate extension type
        if extension_type != "stdio" && extension_type != "sse" {
            return Err("Invalid extension_type: must be 'stdio' or 'sse'".to_string());
        }

        // Validate config is valid JSON
        let config_value: Value = serde_json::from_str(config)
            .map_err(|e| format!("Invalid config JSON: {}", e))?;

        let description = args.get("description").and_then(|v| v.as_str());
        let tags: Option<Vec<String>> = args.get("tags").and_then(|v| {
            v.as_array().map(|arr| {
                arr.iter()
                    .filter_map(|t| t.as_str().map(|s| s.to_string()))
                    .collect()
            })
        });

        let request_body = serde_json::json!({
            "teamId": team_id,
            "name": name,
            "extensionType": extension_type,
            "config": config_value,
            "description": description,
            "tags": tags
        });

        let url = self.api.api_url("/extensions");

        match self.api.post(&url).json(&request_body).send().await {
            Ok(response) => {
                if response.status().is_success() {
                    match response.json::<Value>().await {
                        Ok(data) => {
                            let result = serde_json::json!({
                                "status": "shared",
                                "extension": data,
                                "message": format!("Extension '{}' has been shared with the team. Note: Extensions require security review before use.", name)
                            });
                            Ok(vec![Content::text(
                                serde_json::to_string_pretty(&result).unwrap(),
                            )])
                        }
                        Err(e) => Err(format!("Failed to parse response: {}", e)),
                    }
                } else {
                    let status = response.status();
                    let error_text = response.text().await.unwrap_or_default();
                    Err(format!(
                        "Failed to share extension ({}): {}",
                        status, error_text
                    ))
                }
            }
            Err(e) => {
                warn!("Team API request failed: {}", e);
                let response = serde_json::json!({
                    "team_id": team_id,
                    "name": name,
                    "status": "unavailable",
                    "message": format!("Team server unavailable: {}", e)
                });
                Ok(vec![Content::text(
                    serde_json::to_string_pretty(&response).unwrap(),
                )])
            }
        }
    }

    async fn handle_get_recommendations(
        &self,
        arguments: Option<JsonObject>,
    ) -> Result<Vec<Content>, String> {
        let args = arguments.unwrap_or_default();

        let team_id = args.get("team_id").and_then(|v| v.as_str());
        let resource_type = args.get("resource_type").and_then(|v| v.as_str());
        let context = args.get("context").and_then(|v| v.as_str());
        let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(10) as u32;
        let preferred_tags: Option<Vec<String>> = args.get("preferred_tags").and_then(|v| {
            v.as_array().map(|arr| {
                arr.iter()
                    .filter_map(|t| t.as_str().map(|s| s.to_string()))
                    .collect()
            })
        });

        let mut request_body = serde_json::json!({
            "limit": limit
        });

        if let Some(tid) = team_id {
            request_body["teamId"] = serde_json::json!(tid);
        }
        if let Some(rt) = resource_type {
            request_body["resourceType"] = serde_json::json!(rt);
        }
        if let Some(ctx) = context {
            request_body["context"] = serde_json::json!(ctx);
        }
        if let Some(tags) = preferred_tags {
            request_body["preferredTags"] = serde_json::json!(tags);
        }

        let url = self.api.api_url("/recommendations");

        match self.api.post(&url).json(&request_body).send().await {
            Ok(response) => {
                if response.status().is_success() {
                    match response.json::<Value>().await {
                        Ok(data) => {
                            let result = serde_json::json!({
                                "status": "success",
                                "recommendations": data
                            });
                            Ok(vec![Content::text(
                                serde_json::to_string_pretty(&result).unwrap(),
                            )])
                        }
                        Err(e) => Err(format!("Failed to parse response: {}", e)),
                    }
                } else {
                    let status = response.status();
                    let error_text = response.text().await.unwrap_or_default();
                    Err(format!("API error ({}): {}", status, error_text))
                }
            }
            Err(e) => {
                warn!("Team API request failed: {}", e);
                let response = serde_json::json!({
                    "recommendations": [],
                    "status": "unavailable",
                    "message": format!("Team server unavailable: {}", e)
                });
                Ok(vec![Content::text(
                    serde_json::to_string_pretty(&response).unwrap(),
                )])
            }
        }
    }

    async fn handle_list_teams(
        &self,
        arguments: Option<JsonObject>,
    ) -> Result<Vec<Content>, String> {
        let args = arguments.unwrap_or_default();
        let include_stats = args
            .get("include_stats")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let mut params = vec![];
        if include_stats {
            params.push(("includeStats", "true".to_string()));
        }

        let url = self.api.api_url("/teams");

        match self.api.get(&url).query(&params).send().await {
            Ok(response) => {
                if response.status().is_success() {
                    match response.json::<Value>().await {
                        Ok(data) => {
                            let result = serde_json::json!({
                                "status": "success",
                                "teams": data
                            });
                            Ok(vec![Content::text(
                                serde_json::to_string_pretty(&result).unwrap(),
                            )])
                        }
                        Err(e) => Err(format!("Failed to parse response: {}", e)),
                    }
                } else {
                    let status = response.status();
                    let error_text = response.text().await.unwrap_or_default();
                    Err(format!("API error ({}): {}", status, error_text))
                }
            }
            Err(e) => {
                warn!("Team API request failed: {}", e);
                let response = serde_json::json!({
                    "teams": [],
                    "status": "unavailable",
                    "message": format!("Team server unavailable: {}", e)
                });
                Ok(vec![Content::text(
                    serde_json::to_string_pretty(&response).unwrap(),
                )])
            }
        }
    }

    async fn handle_get_stats(
        &self,
        arguments: Option<JsonObject>,
    ) -> Result<Vec<Content>, String> {
        let args = arguments.ok_or("Missing arguments")?;

        let team_id = args
            .get("team_id")
            .and_then(|v| v.as_str())
            .ok_or("Missing required parameter: team_id")?;
        let resource_type = args.get("resource_type").and_then(|v| v.as_str());
        let days = args.get("days").and_then(|v| v.as_u64()).unwrap_or(30) as u32;

        let mut params = vec![("days", days.to_string())];
        if let Some(rt) = resource_type {
            params.push(("resourceType", rt.to_string()));
        }

        let url = self.api.api_url(&format!("/teams/{}/stats", team_id));

        match self.api.get(&url).query(&params).send().await {
            Ok(response) => {
                if response.status().is_success() {
                    match response.json::<Value>().await {
                        Ok(data) => {
                            let result = serde_json::json!({
                                "status": "success",
                                "team_id": team_id,
                                "stats": data
                            });
                            Ok(vec![Content::text(
                                serde_json::to_string_pretty(&result).unwrap(),
                            )])
                        }
                        Err(e) => Err(format!("Failed to parse response: {}", e)),
                    }
                } else if response.status().as_u16() == 404 {
                    Err(format!("Team not found: {}", team_id))
                } else {
                    let status = response.status();
                    let error_text = response.text().await.unwrap_or_default();
                    Err(format!("API error ({}): {}", status, error_text))
                }
            }
            Err(e) => {
                warn!("Team API request failed: {}", e);
                let response = serde_json::json!({
                    "team_id": team_id,
                    "stats": {},
                    "status": "unavailable",
                    "message": format!("Team server unavailable: {}", e)
                });
                Ok(vec![Content::text(
                    serde_json::to_string_pretty(&response).unwrap(),
                )])
            }
        }
    }
}

#[async_trait]
impl McpClientTrait for TeamClient {
    async fn list_resources(
        &self,
        _next_cursor: Option<String>,
        _cancellation_token: CancellationToken,
    ) -> Result<ListResourcesResult, Error> {
        Err(Error::TransportClosed)
    }

    async fn read_resource(
        &self,
        _uri: &str,
        _cancellation_token: CancellationToken,
    ) -> Result<ReadResourceResult, Error> {
        Err(Error::TransportClosed)
    }

    async fn list_tools(
        &self,
        _next_cursor: Option<String>,
        _cancellation_token: CancellationToken,
    ) -> Result<ListToolsResult, Error> {
        Ok(ListToolsResult {
            tools: Self::get_tools(),
            next_cursor: None,
            meta: None,
        })
    }

    async fn call_tool(
        &self,
        name: &str,
        arguments: Option<JsonObject>,
        _cancellation_token: CancellationToken,
    ) -> Result<CallToolResult, Error> {
        let content = match name {
            "team_search" => self.handle_search(arguments).await,
            "team_load_skill" => self.handle_load_skill(arguments).await,
            "team_share_skill" => self.handle_share_skill(arguments).await,
            "team_share_recipe" => self.handle_share_recipe(arguments).await,
            "team_share_extension" => self.handle_share_extension(arguments).await,
            "team_install" => self.handle_install(arguments).await,
            "team_list_installed" => self.handle_list_installed(arguments).await,
            "team_check_updates" => self.handle_check_updates().await,
            "team_uninstall_local" => self.handle_uninstall_local(arguments).await,
            "team_get_recommendations" => self.handle_get_recommendations(arguments).await,
            "team_list" => self.handle_list_teams(arguments).await,
            "team_get_stats" => self.handle_get_stats(arguments).await,
            _ => Err(format!("Unknown tool: {}", name)),
        };

        match content {
            Ok(content) => Ok(CallToolResult::success(content)),
            Err(error) => Ok(CallToolResult::error(vec![Content::text(format!(
                "Error: {}",
                error
            ))])),
        }
    }

    async fn list_prompts(
        &self,
        _next_cursor: Option<String>,
        _cancellation_token: CancellationToken,
    ) -> Result<ListPromptsResult, Error> {
        Err(Error::TransportClosed)
    }

    async fn get_prompt(
        &self,
        _name: &str,
        _arguments: Value,
        _cancellation_token: CancellationToken,
    ) -> Result<GetPromptResult, Error> {
        Err(Error::TransportClosed)
    }

    async fn subscribe(&self) -> mpsc::Receiver<ServerNotification> {
        mpsc::channel(1).1
    }

    fn get_info(&self) -> Option<&InitializeResult> {
        Some(&self.info)
    }

    async fn get_moim(&self) -> Option<String> {
        // Could return info about currently active team or installed resources
        None
    }
}
