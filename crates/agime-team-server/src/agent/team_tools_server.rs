//! Team Tools MCP Server
//!
//! Provides MCP tools for managing team Skills and MCP extensions.
//! This server is run as a subprocess and communicates via stdio.

use anyhow::Result;
use chrono::Utc;
use rmcp::{
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{
        CallToolResult, Content, ErrorCode, ErrorData, Implementation, ServerCapabilities,
        ServerInfo,
    },
    schemars::JsonSchema,
    tool, tool_router, ServerHandler,
};
// Re-export schemars for derive macro
use rmcp::schemars;
use serde::{Deserialize, Serialize};
use sqlx::sqlite::SqlitePoolOptions;
use sqlx::SqlitePool;
use std::sync::Arc;
use tracing::info;
use uuid::Uuid;

// Parameter structs for each tool

/// Parameters for install_skill tool
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct InstallSkillParams {
    /// The name of the skill
    pub name: String,
    /// The skill content (instructions, prompts, or code)
    pub content: String,
    /// A brief description of what the skill does
    #[serde(default)]
    pub description: Option<String>,
}

/// Parameters for list_skills tool (no parameters needed)
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct ListSkillsParams {}

/// Parameters for remove_skill tool
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct RemoveSkillParams {
    /// The name of the skill to remove
    pub name: String,
}

/// Parameters for get_skill tool
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct GetSkillParams {
    /// The name of the skill to retrieve
    pub name: String,
}

/// Parameters for install_mcp tool
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct InstallMcpParams {
    /// The name of the MCP extension
    pub name: String,
    /// The type: 'sse' or 'stdio'
    pub ext_type: String,
    /// For SSE: the URI. For Stdio: the command
    pub uri_or_cmd: String,
    /// Arguments for Stdio command
    #[serde(default)]
    pub args: Option<Vec<String>>,
}

/// Parameters for list_mcps tool (no parameters needed)
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct ListMcpsParams {}

/// Parameters for remove_mcp tool
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct RemoveMcpParams {
    /// The name of the MCP extension to remove
    pub name: String,
}

/// Team Tools MCP Server
#[derive(Clone)]
pub struct TeamToolsServer {
    tool_router: ToolRouter<Self>,
    pool: Arc<SqlitePool>,
    team_id: String,
    agent_id: String,
    user_id: String,
    instructions: String,
}

impl Default for TeamToolsServer {
    fn default() -> Self {
        panic!("Use TeamToolsServer::from_env() instead")
    }
}

impl TeamToolsServer {
    /// Create a new TeamToolsServer from environment variables
    pub async fn from_env() -> Result<Self> {
        let db_url = std::env::var("AGIME_TEAM_DB_URL")
            .unwrap_or_else(|_| "sqlite://./data/team.db?mode=rwc".to_string());
        let team_id = std::env::var("AGIME_TEAM_ID")
            .map_err(|_| anyhow::anyhow!("AGIME_TEAM_ID not set"))?;
        let agent_id = std::env::var("AGIME_AGENT_ID")
            .map_err(|_| anyhow::anyhow!("AGIME_AGENT_ID not set"))?;
        let user_id = std::env::var("AGIME_USER_ID")
            .unwrap_or_else(|_| "system".to_string());

        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .connect(&db_url)
            .await?;

        let instructions = r#"
Team Tools Extension

Use these tools to manage shared Skills and MCP extensions for the team.

Skills:
- install_skill: Install a new skill to share with team members
- list_skills: List all available skills
- get_skill: Get the content of a specific skill
- remove_skill: Remove a skill from the team

MCP Extensions:
- install_mcp: Install a new MCP extension for the agent
- list_mcps: List all MCP extensions
- remove_mcp: Remove an MCP extension
"#.to_string();

        Ok(Self {
            tool_router: ToolRouter::new(),
            pool: Arc::new(pool),
            team_id,
            agent_id,
            user_id,
            instructions,
        })
    }
}

// Implement the MCP server trait
impl ServerHandler for TeamToolsServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            server_info: Implementation {
                name: "team-tools".to_string(),
                version: env!("CARGO_PKG_VERSION").to_owned(),
                title: Some("Team Tools".to_string()),
                icons: None,
                website_url: None,
            },
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            instructions: Some(self.instructions.clone()),
            ..Default::default()
        }
    }
}

#[tool_router(router = tool_router)]
impl TeamToolsServer {
    /// Install a new skill to the team
    #[tool(
        name = "install_skill",
        description = "Install a new skill to the team. The skill will be available to all team members."
    )]
    pub async fn install_skill(
        &self,
        params: Parameters<InstallSkillParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let params = params.0;
        let desc = params.description.unwrap_or_default();
        let now = Utc::now().to_rfc3339();

        // Check if skill already exists
        let existing: Option<(String,)> = sqlx::query_as(
            "SELECT id FROM shared_skills WHERE team_id = ? AND name = ? AND is_deleted = 0"
        )
        .bind(&self.team_id)
        .bind(&params.name)
        .fetch_optional(self.pool.as_ref())
        .await
        .map_err(|e| ErrorData::new(ErrorCode::INTERNAL_ERROR, e.to_string(), None))?;

        if let Some((id,)) = existing {
            // Update existing skill
            sqlx::query(
                "UPDATE shared_skills SET content = ?, description = ?, updated_at = ? WHERE id = ?"
            )
            .bind(&params.content)
            .bind(&desc)
            .bind(&now)
            .bind(&id)
            .execute(self.pool.as_ref())
            .await
            .map_err(|e| ErrorData::new(ErrorCode::INTERNAL_ERROR, e.to_string(), None))?;

            info!("Updated skill '{}' in team {}", params.name, self.team_id);
            Ok(CallToolResult::success(vec![Content::text(format!(
                "Successfully updated skill '{}'. It is now available to all team members.",
                params.name
            ))]))
        } else {
            // Insert new skill
            let id = Uuid::new_v4().to_string();
            sqlx::query(
                "INSERT INTO shared_skills (id, team_id, name, description, content, shared_by, created_at, updated_at, is_deleted) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, 0)"
            )
            .bind(&id)
            .bind(&self.team_id)
            .bind(&params.name)
            .bind(&desc)
            .bind(&params.content)
            .bind(&self.user_id)
            .bind(&now)
            .bind(&now)
            .execute(self.pool.as_ref())
            .await
            .map_err(|e| ErrorData::new(ErrorCode::INTERNAL_ERROR, e.to_string(), None))?;

            info!("Installed new skill '{}' in team {}", params.name, self.team_id);
            Ok(CallToolResult::success(vec![Content::text(format!(
                "Successfully installed skill '{}'. It is now available to all team members.",
                params.name
            ))]))
        }
    }

    /// List all skills available in the team
    #[tool(
        name = "list_skills",
        description = "List all skills available in the team."
    )]
    pub async fn list_skills(
        &self,
        _params: Parameters<ListSkillsParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let skills: Vec<SkillRow> = sqlx::query_as(
            "SELECT id, name, description, shared_by, created_at FROM shared_skills \
             WHERE team_id = ? AND is_deleted = 0 ORDER BY name"
        )
        .bind(&self.team_id)
        .fetch_all(self.pool.as_ref())
        .await
        .map_err(|e| ErrorData::new(ErrorCode::INTERNAL_ERROR, e.to_string(), None))?;

        if skills.is_empty() {
            return Ok(CallToolResult::success(vec![Content::text(
                "No skills installed in this team yet."
            )]));
        }

        let mut output = format!("Found {} skills in the team:\n\n", skills.len());
        for skill in skills {
            output.push_str(&format!(
                "- **{}**: {}\n  (shared by: {}, created: {})\n",
                skill.name,
                skill.description.unwrap_or_default(),
                skill.shared_by,
                skill.created_at
            ));
        }

        Ok(CallToolResult::success(vec![Content::text(output)]))
    }

    /// Remove a skill from the team
    #[tool(
        name = "remove_skill",
        description = "Remove a skill from the team by name."
    )]
    pub async fn remove_skill(
        &self,
        params: Parameters<RemoveSkillParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let params = params.0;

        let result = sqlx::query(
            "UPDATE shared_skills SET is_deleted = 1, updated_at = ? \
             WHERE team_id = ? AND name = ? AND is_deleted = 0"
        )
        .bind(Utc::now().to_rfc3339())
        .bind(&self.team_id)
        .bind(&params.name)
        .execute(self.pool.as_ref())
        .await
        .map_err(|e| ErrorData::new(ErrorCode::INTERNAL_ERROR, e.to_string(), None))?;

        if result.rows_affected() > 0 {
            info!("Removed skill '{}' from team {}", params.name, self.team_id);
            Ok(CallToolResult::success(vec![Content::text(format!(
                "Successfully removed skill '{}'.",
                params.name
            ))]))
        } else {
            Ok(CallToolResult::error(vec![Content::text(format!(
                "Skill '{}' not found in the team.",
                params.name
            ))]))
        }
    }

    /// Get the content of a specific skill
    #[tool(
        name = "get_skill",
        description = "Get the content of a specific skill by name."
    )]
    pub async fn get_skill(
        &self,
        params: Parameters<GetSkillParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let params = params.0;

        let skill: Option<SkillContentRow> = sqlx::query_as(
            "SELECT name, description, content FROM shared_skills \
             WHERE team_id = ? AND name = ? AND is_deleted = 0"
        )
        .bind(&self.team_id)
        .bind(&params.name)
        .fetch_optional(self.pool.as_ref())
        .await
        .map_err(|e| ErrorData::new(ErrorCode::INTERNAL_ERROR, e.to_string(), None))?;

        match skill {
            Some(s) => {
                let output = format!(
                    "# Skill: {}\n\n{}\n\n## Content:\n\n{}",
                    s.name,
                    s.description.unwrap_or_default(),
                    s.content
                );
                Ok(CallToolResult::success(vec![Content::text(output)]))
            }
            None => Ok(CallToolResult::error(vec![Content::text(format!(
                "Skill '{}' not found.",
                params.name
            ))])),
        }
    }

    /// Install a new MCP extension to the agent
    #[tool(
        name = "install_mcp",
        description = "Install a new MCP extension to the agent. Supports SSE and Stdio types."
    )]
    pub async fn install_mcp(
        &self,
        params: Parameters<InstallMcpParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let params = params.0;

        if params.ext_type != "sse" && params.ext_type != "stdio" {
            return Ok(CallToolResult::error(vec![Content::text(
                "Invalid ext_type. Must be 'sse' or 'stdio'."
            )]));
        }

        let args_vec = params.args.unwrap_or_default();
        let now = Utc::now().to_rfc3339();

        // Get current custom_extensions
        let row: Option<(Option<String>,)> = sqlx::query_as(
            "SELECT custom_extensions FROM team_agents WHERE id = ?"
        )
        .bind(&self.agent_id)
        .fetch_optional(self.pool.as_ref())
        .await
        .map_err(|e| ErrorData::new(ErrorCode::INTERNAL_ERROR, e.to_string(), None))?;

        let mut extensions: Vec<CustomExtension> = match row {
            Some((Some(json_str),)) => serde_json::from_str(&json_str).unwrap_or_default(),
            _ => Vec::new(),
        };

        // Check if extension already exists
        let existing = extensions.iter_mut().find(|e| e.name == params.name);

        if let Some(ext) = existing {
            ext.ext_type = params.ext_type.clone();
            ext.uri_or_cmd = params.uri_or_cmd.clone();
            ext.args = args_vec;
            ext.enabled = true;
        } else {
            extensions.push(CustomExtension {
                name: params.name.clone(),
                ext_type: params.ext_type.clone(),
                uri_or_cmd: params.uri_or_cmd.clone(),
                args: args_vec,
                envs: std::collections::HashMap::new(),
                enabled: true,
            });
        }

        // Save back
        let json_str = serde_json::to_string(&extensions)
            .map_err(|e| ErrorData::new(ErrorCode::INTERNAL_ERROR, e.to_string(), None))?;

        sqlx::query(
            "UPDATE team_agents SET custom_extensions = ?, updated_at = ? WHERE id = ?"
        )
        .bind(&json_str)
        .bind(&now)
        .bind(&self.agent_id)
        .execute(self.pool.as_ref())
        .await
        .map_err(|e| ErrorData::new(ErrorCode::INTERNAL_ERROR, e.to_string(), None))?;

        info!("Installed MCP extension '{}' for agent {}", params.name, self.agent_id);
        Ok(CallToolResult::success(vec![Content::text(format!(
            "Successfully installed MCP extension '{}'. It will be available on next task execution.",
            params.name
        ))]))
    }

    /// List all MCP extensions configured for this agent
    #[tool(
        name = "list_mcps",
        description = "List all MCP extensions configured for this agent."
    )]
    pub async fn list_mcps(
        &self,
        _params: Parameters<ListMcpsParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let row: Option<(Option<String>,)> = sqlx::query_as(
            "SELECT custom_extensions FROM team_agents WHERE id = ?"
        )
        .bind(&self.agent_id)
        .fetch_optional(self.pool.as_ref())
        .await
        .map_err(|e| ErrorData::new(ErrorCode::INTERNAL_ERROR, e.to_string(), None))?;

        let extensions: Vec<CustomExtension> = match row {
            Some((Some(json_str),)) => serde_json::from_str(&json_str).unwrap_or_default(),
            _ => Vec::new(),
        };

        if extensions.is_empty() {
            return Ok(CallToolResult::success(vec![Content::text(
                "No custom MCP extensions installed for this agent."
            )]));
        }

        let mut output = format!("Found {} custom MCP extensions:\n\n", extensions.len());
        for ext in extensions {
            let status = if ext.enabled { "enabled" } else { "disabled" };
            output.push_str(&format!(
                "- **{}** ({})\n  Type: {}\n  {}: {}\n",
                ext.name,
                status,
                ext.ext_type,
                if ext.ext_type == "sse" { "URI" } else { "Command" },
                ext.uri_or_cmd
            ));
        }

        Ok(CallToolResult::success(vec![Content::text(output)]))
    }

    /// Remove an MCP extension from the agent
    #[tool(
        name = "remove_mcp",
        description = "Remove an MCP extension from the agent by name."
    )]
    pub async fn remove_mcp(
        &self,
        params: Parameters<RemoveMcpParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let params = params.0;
        let now = Utc::now().to_rfc3339();

        let row: Option<(Option<String>,)> = sqlx::query_as(
            "SELECT custom_extensions FROM team_agents WHERE id = ?"
        )
        .bind(&self.agent_id)
        .fetch_optional(self.pool.as_ref())
        .await
        .map_err(|e| ErrorData::new(ErrorCode::INTERNAL_ERROR, e.to_string(), None))?;

        let mut extensions: Vec<CustomExtension> = match row {
            Some((Some(json_str),)) => serde_json::from_str(&json_str).unwrap_or_default(),
            _ => Vec::new(),
        };

        let original_len = extensions.len();
        extensions.retain(|e| e.name != params.name);

        if extensions.len() < original_len {
            let json_str = serde_json::to_string(&extensions)
                .map_err(|e| ErrorData::new(ErrorCode::INTERNAL_ERROR, e.to_string(), None))?;

            sqlx::query(
                "UPDATE team_agents SET custom_extensions = ?, updated_at = ? WHERE id = ?"
            )
            .bind(&json_str)
            .bind(&now)
            .bind(&self.agent_id)
            .execute(self.pool.as_ref())
            .await
            .map_err(|e| ErrorData::new(ErrorCode::INTERNAL_ERROR, e.to_string(), None))?;

            info!("Removed MCP extension '{}' from agent {}", params.name, self.agent_id);
            Ok(CallToolResult::success(vec![Content::text(format!(
                "Successfully removed MCP extension '{}'.",
                params.name
            ))]))
        } else {
            Ok(CallToolResult::error(vec![Content::text(format!(
                "MCP extension '{}' not found.",
                params.name
            ))]))
        }
    }
}

#[derive(sqlx::FromRow)]
struct SkillRow {
    id: String,
    name: String,
    description: Option<String>,
    shared_by: String,
    created_at: String,
}

#[derive(sqlx::FromRow)]
struct SkillContentRow {
    name: String,
    description: Option<String>,
    content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CustomExtension {
    name: String,
    ext_type: String,
    uri_or_cmd: String,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    envs: std::collections::HashMap<String, String>,
    #[serde(default = "default_enabled")]
    enabled: bool,
}

fn default_enabled() -> bool {
    true
}
