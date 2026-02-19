//! Team MCP Extension for Cloud Agent
//!
//! Provides tools for installing, listing, and removing MCP extensions
//! that are stored in the team agent's configuration.

use anyhow::Result;
use chrono::Utc;
use rmcp::model::{CallToolResult, Content, Tool};
use rmcp::object;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::SqlitePool;
use std::sync::Arc;
use tracing::info;

pub const EXTENSION_NAME: &str = "team_mcp";

/// Team MCP Extension Client
pub struct TeamMcpClient {
    pool: Arc<SqlitePool>,
    agent_id: String,
}

impl TeamMcpClient {
    pub fn new(pool: Arc<SqlitePool>, agent_id: String) -> Self {
        Self { pool, agent_id }
    }

    /// Get available tools
    pub fn tools(&self) -> Vec<Tool> {
        vec![
            Tool::new(
                "install_mcp",
                "Install a new MCP extension to the agent. Supports SSE and Stdio types.",
                object!({
                    "type": "object",
                    "properties": {
                        "name": {
                            "type": "string",
                            "description": "The name of the MCP extension"
                        },
                        "ext_type": {
                            "type": "string",
                            "enum": ["sse", "stdio"],
                            "description": "The type of MCP extension: 'sse' for Server-Sent Events, 'stdio' for standard I/O"
                        },
                        "uri_or_cmd": {
                            "type": "string",
                            "description": "For SSE: the URI endpoint. For Stdio: the command to execute"
                        },
                        "args": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "Arguments for Stdio command (optional)"
                        },
                        "envs": {
                            "type": "object",
                            "description": "Environment variables as key-value pairs (optional)"
                        }
                    },
                    "required": ["name", "ext_type", "uri_or_cmd"]
                }),
            ),
            Tool::new(
                "list_mcps",
                "List all MCP extensions configured for this agent.",
                object!({
                    "type": "object",
                    "properties": {}
                }),
            ),
            Tool::new(
                "remove_mcp",
                "Remove an MCP extension from the agent by name.",
                object!({
                    "type": "object",
                    "properties": {
                        "name": {
                            "type": "string",
                            "description": "The name of the MCP extension to remove"
                        }
                    },
                    "required": ["name"]
                }),
            ),
            Tool::new(
                "toggle_mcp",
                "Enable or disable an MCP extension.",
                object!({
                    "type": "object",
                    "properties": {
                        "name": {
                            "type": "string",
                            "description": "The name of the MCP extension"
                        },
                        "enabled": {
                            "type": "boolean",
                            "description": "Whether to enable (true) or disable (false) the extension"
                        }
                    },
                    "required": ["name", "enabled"]
                }),
            ),
        ]
    }

    /// Call a tool
    pub async fn call_tool(&self, name: &str, arguments: Value) -> Result<CallToolResult> {
        match name {
            "install_mcp" => self.install_mcp(arguments).await,
            "list_mcps" => self.list_mcps().await,
            "remove_mcp" => self.remove_mcp(arguments).await,
            "toggle_mcp" => self.toggle_mcp(arguments).await,
            _ => Ok(CallToolResult {
                content: vec![Content::text(format!("Unknown tool: {}", name))],
                is_error: Some(true),
                meta: None,
                structured_content: None,
            }),
        }
    }

    async fn get_custom_extensions(&self) -> Result<Vec<CustomExtension>> {
        let row: Option<(Option<String>,)> = sqlx::query_as(
            "SELECT custom_extensions FROM team_agents WHERE id = ?"
        )
        .bind(&self.agent_id)
        .fetch_optional(self.pool.as_ref())
        .await?;

        match row {
            Some((Some(json_str),)) => {
                Ok(serde_json::from_str(&json_str).unwrap_or_default())
            }
            _ => Ok(Vec::new()),
        }
    }

    async fn save_custom_extensions(&self, extensions: &[CustomExtension]) -> Result<()> {
        let json_str = serde_json::to_string(extensions)?;
        let now = Utc::now().to_rfc3339();

        sqlx::query(
            "UPDATE team_agents SET custom_extensions = ?, updated_at = ? WHERE id = ?"
        )
        .bind(&json_str)
        .bind(&now)
        .bind(&self.agent_id)
        .execute(self.pool.as_ref())
        .await?;

        Ok(())
    }

    async fn install_mcp(&self, args: Value) -> Result<CallToolResult> {
        let name = args.get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing 'name' parameter"))?;

        let ext_type = args.get("ext_type")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing 'ext_type' parameter"))?;

        let uri_or_cmd = args.get("uri_or_cmd")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing 'uri_or_cmd' parameter"))?;

        // Validate ext_type
        if ext_type != "sse" && ext_type != "stdio" {
            return Ok(CallToolResult {
                content: vec![Content::text(
                    "Invalid ext_type. Must be 'sse' or 'stdio'."
                )],
                is_error: Some(true),
                meta: None,
                structured_content: None,
            });
        }

        let args_array: Vec<String> = args.get("args")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();

        let envs: std::collections::HashMap<String, String> = args.get("envs")
            .and_then(|v| v.as_object())
            .map(|obj| {
                obj.iter()
                    .filter_map(|(k, v)| {
                        v.as_str().map(|s| (k.clone(), s.to_string()))
                    })
                    .collect()
            })
            .unwrap_or_default();

        let mut extensions = self.get_custom_extensions().await?;

        // Check if extension already exists
        if let Some(ext) = extensions.iter_mut().find(|e| e.name == name) {
            // Update existing
            ext.ext_type = ext_type.to_string();
            ext.uri_or_cmd = uri_or_cmd.to_string();
            ext.args = args_array;
            ext.envs = envs;
            ext.enabled = true;

            self.save_custom_extensions(&extensions).await?;
            info!("Updated MCP extension '{}' for agent {}", name, self.agent_id);

            Ok(CallToolResult {
                content: vec![Content::text(format!(
                    "Successfully updated MCP extension '{}'. It will be available on next task execution.",
                    name
                ))],
                is_error: Some(false),
                meta: None,
                structured_content: None,
            })
        } else {
            // Add new
            extensions.push(CustomExtension {
                name: name.to_string(),
                ext_type: ext_type.to_string(),
                uri_or_cmd: uri_or_cmd.to_string(),
                args: args_array,
                envs,
                enabled: true,
            });

            self.save_custom_extensions(&extensions).await?;
            info!("Installed MCP extension '{}' for agent {}", name, self.agent_id);

            Ok(CallToolResult {
                content: vec![Content::text(format!(
                    "Successfully installed MCP extension '{}'. It will be available on next task execution.",
                    name
                ))],
                is_error: Some(false),
                meta: None,
                structured_content: None,
            })
        }
    }

    async fn list_mcps(&self) -> Result<CallToolResult> {
        let extensions = self.get_custom_extensions().await?;

        if extensions.is_empty() {
            return Ok(CallToolResult {
                content: vec![Content::text(
                    "No custom MCP extensions installed for this agent."
                )],
                is_error: Some(false),
                meta: None,
                structured_content: None,
            });
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
            if !ext.args.is_empty() {
                output.push_str(&format!("  Args: {:?}\n", ext.args));
            }
        }

        Ok(CallToolResult {
            content: vec![Content::text(output)],
            is_error: Some(false),
            meta: None,
            structured_content: None,
        })
    }

    async fn remove_mcp(&self, args: Value) -> Result<CallToolResult> {
        let name = args.get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing 'name' parameter"))?;

        let mut extensions = self.get_custom_extensions().await?;
        let original_len = extensions.len();

        extensions.retain(|e| e.name != name);

        if extensions.len() < original_len {
            self.save_custom_extensions(&extensions).await?;
            info!("Removed MCP extension '{}' from agent {}", name, self.agent_id);

            Ok(CallToolResult {
                content: vec![Content::text(format!(
                    "Successfully removed MCP extension '{}'.",
                    name
                ))],
                is_error: Some(false),
                meta: None,
                structured_content: None,
            })
        } else {
            Ok(CallToolResult {
                content: vec![Content::text(format!(
                    "MCP extension '{}' not found.",
                    name
                ))],
                is_error: Some(true),
                meta: None,
                structured_content: None,
            })
        }
    }

    async fn toggle_mcp(&self, args: Value) -> Result<CallToolResult> {
        let name = args.get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing 'name' parameter"))?;

        let enabled = args.get("enabled")
            .and_then(|v| v.as_bool())
            .ok_or_else(|| anyhow::anyhow!("Missing 'enabled' parameter"))?;

        let mut extensions = self.get_custom_extensions().await?;

        if let Some(ext) = extensions.iter_mut().find(|e| e.name == name) {
            ext.enabled = enabled;
            self.save_custom_extensions(&extensions).await?;

            let status = if enabled { "enabled" } else { "disabled" };
            info!("Toggled MCP extension '{}' to {} for agent {}", name, status, self.agent_id);

            Ok(CallToolResult {
                content: vec![Content::text(format!(
                    "Successfully {} MCP extension '{}'.",
                    status, name
                ))],
                is_error: Some(false),
                meta: None,
                structured_content: None,
            })
        } else {
            Ok(CallToolResult {
                content: vec![Content::text(format!(
                    "MCP extension '{}' not found.",
                    name
                ))],
                is_error: Some(true),
                meta: None,
                structured_content: None,
            })
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomExtension {
    pub name: String,
    pub ext_type: String,
    pub uri_or_cmd: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub envs: std::collections::HashMap<String, String>,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
}

fn default_enabled() -> bool {
    true
}
