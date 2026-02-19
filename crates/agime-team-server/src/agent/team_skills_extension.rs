//! Team Skills Extension for Cloud Agent
//!
//! Provides tools for installing, listing, and removing Skills
//! that are stored in the team database.

use anyhow::Result;
use chrono::Utc;
use rmcp::model::{CallToolResult, Content, Tool};
use rmcp::object;
use serde_json::Value;
use sqlx::SqlitePool;
use std::sync::Arc;
use tracing::info;
use uuid::Uuid;

pub const EXTENSION_NAME: &str = "team_skills";

/// Team Skills Extension Client
pub struct TeamSkillsClient {
    pool: Arc<SqlitePool>,
    team_id: String,
    user_id: String,
}

impl TeamSkillsClient {
    pub fn new(pool: Arc<SqlitePool>, team_id: String, user_id: String) -> Self {
        Self { pool, team_id, user_id }
    }

    /// Get available tools
    pub fn tools(&self) -> Vec<Tool> {
        vec![
            Tool::new(
                "install_skill",
                "Install a new skill to the team. The skill will be available to all team members.",
                object!({
                    "type": "object",
                    "properties": {
                        "name": {
                            "type": "string",
                            "description": "The name of the skill"
                        },
                        "description": {
                            "type": "string",
                            "description": "A brief description of what the skill does"
                        },
                        "content": {
                            "type": "string",
                            "description": "The skill content (instructions, prompts, or code)"
                        }
                    },
                    "required": ["name", "content"]
                }),
            ),
            Tool::new(
                "list_skills",
                "List all skills available in the team.",
                object!({
                    "type": "object",
                    "properties": {}
                }),
            ),
            Tool::new(
                "remove_skill",
                "Remove a skill from the team by name.",
                object!({
                    "type": "object",
                    "properties": {
                        "name": {
                            "type": "string",
                            "description": "The name of the skill to remove"
                        }
                    },
                    "required": ["name"]
                }),
            ),
            Tool::new(
                "get_skill",
                "Get the content of a specific skill by name.",
                object!({
                    "type": "object",
                    "properties": {
                        "name": {
                            "type": "string",
                            "description": "The name of the skill to retrieve"
                        }
                    },
                    "required": ["name"]
                }),
            ),
        ]
    }

    /// Call a tool
    pub async fn call_tool(&self, name: &str, arguments: Value) -> Result<CallToolResult> {
        match name {
            "install_skill" => self.install_skill(arguments).await,
            "list_skills" => self.list_skills().await,
            "remove_skill" => self.remove_skill(arguments).await,
            "get_skill" => self.get_skill(arguments).await,
            _ => Ok(CallToolResult {
                content: vec![Content::text(format!("Unknown tool: {}", name))],
                is_error: Some(true),
                meta: None,
                structured_content: None,
            }),
        }
    }

    async fn install_skill(&self, args: Value) -> Result<CallToolResult> {
        let name = args.get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing 'name' parameter"))?;

        let content = args.get("content")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing 'content' parameter"))?;

        let description = args.get("description")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        // Check if skill already exists
        let existing: Option<(String,)> = sqlx::query_as(
            "SELECT id FROM shared_skills WHERE team_id = ? AND name = ? AND is_deleted = 0"
        )
        .bind(&self.team_id)
        .bind(name)
        .fetch_optional(self.pool.as_ref())
        .await?;

        let now = Utc::now().to_rfc3339();

        if let Some((id,)) = existing {
            // Update existing skill
            sqlx::query(
                "UPDATE shared_skills SET content = ?, description = ?, updated_at = ? WHERE id = ?"
            )
            .bind(content)
            .bind(description)
            .bind(&now)
            .bind(&id)
            .execute(self.pool.as_ref())
            .await?;

            info!("Updated skill '{}' in team {}", name, self.team_id);
            Ok(CallToolResult {
                content: vec![Content::text(format!(
                    "Successfully updated skill '{}'. It is now available to all team members.",
                    name
                ))],
                is_error: Some(false),
                meta: None,
                structured_content: None,
            })
        } else {
            // Insert new skill
            let id = Uuid::new_v4().to_string();
            sqlx::query(
                "INSERT INTO shared_skills (id, team_id, name, description, content, shared_by, created_at, updated_at, is_deleted) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, 0)"
            )
            .bind(&id)
            .bind(&self.team_id)
            .bind(name)
            .bind(description)
            .bind(content)
            .bind(&self.user_id)
            .bind(&now)
            .bind(&now)
            .execute(self.pool.as_ref())
            .await?;

            info!("Installed new skill '{}' in team {}", name, self.team_id);
            Ok(CallToolResult {
                content: vec![Content::text(format!(
                    "Successfully installed skill '{}'. It is now available to all team members.",
                    name
                ))],
                is_error: Some(false),
                meta: None,
                structured_content: None,
            })
        }
    }

    async fn list_skills(&self) -> Result<CallToolResult> {
        let skills: Vec<SkillRow> = sqlx::query_as(
            "SELECT id, name, description, shared_by, created_at FROM shared_skills \
             WHERE team_id = ? AND is_deleted = 0 ORDER BY name"
        )
        .bind(&self.team_id)
        .fetch_all(self.pool.as_ref())
        .await?;

        if skills.is_empty() {
            return Ok(CallToolResult {
                content: vec![Content::text("No skills installed in this team yet.")],
                is_error: Some(false),
                meta: None,
                structured_content: None,
            });
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

        Ok(CallToolResult {
            content: vec![Content::text(output)],
            is_error: Some(false),
            meta: None,
            structured_content: None,
        })
    }

    async fn remove_skill(&self, args: Value) -> Result<CallToolResult> {
        let name = args.get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing 'name' parameter"))?;

        let result = sqlx::query(
            "UPDATE shared_skills SET is_deleted = 1, updated_at = ? \
             WHERE team_id = ? AND name = ? AND is_deleted = 0"
        )
        .bind(Utc::now().to_rfc3339())
        .bind(&self.team_id)
        .bind(name)
        .execute(self.pool.as_ref())
        .await?;

        if result.rows_affected() > 0 {
            info!("Removed skill '{}' from team {}", name, self.team_id);
            Ok(CallToolResult {
                content: vec![Content::text(format!(
                    "Successfully removed skill '{}'.",
                    name
                ))],
                is_error: Some(false),
                meta: None,
                structured_content: None,
            })
        } else {
            Ok(CallToolResult {
                content: vec![Content::text(format!(
                    "Skill '{}' not found in the team.",
                    name
                ))],
                is_error: Some(true),
                meta: None,
                structured_content: None,
            })
        }
    }

    async fn get_skill(&self, args: Value) -> Result<CallToolResult> {
        let name = args.get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing 'name' parameter"))?;

        let skill: Option<SkillContentRow> = sqlx::query_as(
            "SELECT name, description, content FROM shared_skills \
             WHERE team_id = ? AND name = ? AND is_deleted = 0"
        )
        .bind(&self.team_id)
        .bind(name)
        .fetch_optional(self.pool.as_ref())
        .await?;

        match skill {
            Some(s) => {
                let output = format!(
                    "# Skill: {}\n\n{}\n\n## Content:\n\n{}",
                    s.name,
                    s.description.unwrap_or_default(),
                    s.content
                );
                Ok(CallToolResult {
                    content: vec![Content::text(output)],
                    is_error: Some(false),
                    meta: None,
                    structured_content: None,
                })
            }
            None => Ok(CallToolResult {
                content: vec![Content::text(format!("Skill '{}' not found.", name))],
                is_error: Some(true),
                meta: None,
                structured_content: None,
            }),
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
