use crate::agents::extension::PlatformExtensionContext;
use crate::agents::mcp_client::{Error, McpClientTrait};
use crate::config::paths::Paths;
use anyhow::Result;
use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use indoc::indoc;
use rmcp::model::{
    CallToolResult, Content, GetPromptResult, Implementation, InitializeResult, JsonObject,
    ListPromptsResult, ListResourcesResult, ListToolsResult, ProtocolVersion, ReadResourceResult,
    ServerCapabilities, ServerNotification, Tool, ToolAnnotations, ToolsCapability,
};
use schemars::{schema_for, JsonSchema};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

pub static EXTENSION_NAME: &str = "skills";

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct LoadSkillParams {
    name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SkillMetadata {
    name: String,
    description: String,
}

/// Source metadata for tracking team-installed skills
/// This is read from .skill-meta.json alongside SKILL.md
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct SkillSourceMeta {
    /// Source of the skill: "local" or "team"
    #[serde(default)]
    pub source: Option<String>,
    /// Team ID (only for team skills)
    #[serde(alias = "teamId")]
    pub team_id: Option<String>,
    /// Resource ID in the team server
    #[serde(alias = "resourceId")]
    pub resource_id: Option<String>,
    /// When the skill was installed
    #[serde(alias = "installedAt")]
    pub installed_at: Option<String>,
    /// Version at installation time
    #[serde(alias = "installedVersion")]
    pub installed_version: Option<String>,
    /// Protection level of the skill
    #[serde(alias = "protectionLevel")]
    pub protection_level: Option<String>,
    /// User ID who installed the skill
    #[serde(alias = "userId")]
    pub user_id: Option<String>,
    /// Authorization information
    pub authorization: Option<SkillAuthorizationMeta>,
}

/// Authorization metadata for team skills
#[derive(Debug, Clone, Serialize, Deserialize)]
struct SkillAuthorizationMeta {
    /// Authorization token
    pub token: String,
    /// Token expiration time
    #[serde(alias = "expiresAt")]
    pub expires_at: String,
    /// Last time the authorization was verified
    #[serde(alias = "lastVerifiedAt")]
    pub last_verified_at: String,
}

#[derive(Debug, Clone)]
struct Skill {
    metadata: SkillMetadata,
    body: String,
    directory: PathBuf,
    supporting_files: Vec<PathBuf>,
    /// Optional source metadata for team-installed skills
    source_meta: Option<SkillSourceMeta>,
}

pub struct SkillsClient {
    info: InitializeResult,
    skills: HashMap<String, Skill>,
}

impl SkillsClient {
    pub fn new(_context: PlatformExtensionContext) -> Result<Self> {
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
                title: Some("Skills".to_string()),
                version: "1.0.0".to_string(),
                icons: None,
                website_url: None,
            },
            instructions: Some(String::new()),
        };

        let directories = Self::get_default_skill_directories()
            .into_iter()
            .filter(|d| d.exists())
            .collect::<Vec<_>>();
        let mut skills = Self::discover_skills_in_directories(&directories);

        // Add built-in skills (can be overridden by user skills with same name)
        let builtin_skills = Self::get_builtin_skills();
        for (name, skill) in builtin_skills {
            // Only add if not already present (user skills take priority)
            if !skills.contains_key(&name) {
                skills.insert(name, skill);
            }
        }

        let mut client = Self { info, skills };
        client.info.instructions = Some(client.generate_instructions());
        Ok(client)
    }

    /// Get built-in skills that are always available
    fn get_builtin_skills() -> HashMap<String, Skill> {
        let mut skills = HashMap::new();

        // Built-in: Team Onboarding Guide
        skills.insert(
            "team-onboarding".to_string(),
            Skill {
                metadata: SkillMetadata {
                    name: "team-onboarding".to_string(),
                    description: "团队协作入职指南 - 如何使用团队功能搜索、安装和分享资源"
                        .to_string(),
                },
                body: include_str!("builtin_skills/team_onboarding.md").to_string(),
                directory: PathBuf::from("builtin://team-onboarding"),
                supporting_files: vec![],
                source_meta: Some(SkillSourceMeta {
                    source: Some("builtin".to_string()),
                    team_id: None,
                    resource_id: None,
                    installed_at: None,
                    installed_version: Some("1.0.0".to_string()),
                    protection_level: Some("public".to_string()),
                    user_id: None,
                    authorization: None,
                }),
            },
        );

        // Built-in: Extension Security Review
        skills.insert(
            "extension-security-review".to_string(),
            Skill {
                metadata: SkillMetadata {
                    name: "extension-security-review".to_string(),
                    description: "MCP Extension 安全审核指南 - 评估和审核团队共享的扩展"
                        .to_string(),
                },
                body: include_str!("builtin_skills/extension_security_review.md").to_string(),
                directory: PathBuf::from("builtin://extension-security-review"),
                supporting_files: vec![],
                source_meta: Some(SkillSourceMeta {
                    source: Some("builtin".to_string()),
                    team_id: None,
                    resource_id: None,
                    installed_at: None,
                    installed_version: Some("1.0.0".to_string()),
                    protection_level: Some("public".to_string()),
                    user_id: None,
                    authorization: None,
                }),
            },
        );

        skills
    }

    fn get_default_skill_directories() -> Vec<PathBuf> {
        let mut dirs = Vec::new();

        // Check home directory for skills
        if let Some(home) = dirs::home_dir() {
            dirs.push(home.join(".claude/skills"));
            dirs.push(home.join(".agime/skills"));
            dirs.push(home.join(".goose/skills")); // backward compatibility
        }

        // Check AGIME config directory for skills
        dirs.push(Paths::config_dir().join("skills"));

        // Check team resources directory for team-installed skills
        // This is separate from personal skills for lifecycle management
        if let Some(data_local) = dirs::data_local_dir() {
            dirs.push(
                data_local
                    .join("agime")
                    .join("team-resources")
                    .join("skills"),
            );
        }

        // Check working directory for skills
        if let Ok(working_dir) = std::env::current_dir() {
            dirs.push(working_dir.join(".claude/skills"));
            dirs.push(working_dir.join(".agime/skills"));
            dirs.push(working_dir.join(".goose/skills")); // backward compatibility
        }

        dirs
    }

    fn parse_skill_file(path: &Path) -> Result<Skill> {
        let content = std::fs::read_to_string(path)?;

        let (metadata, body) = Self::parse_frontmatter(&content)?;

        let directory = path
            .parent()
            .ok_or_else(|| anyhow::anyhow!("Skill file has no parent directory"))?
            .to_path_buf();

        let supporting_files = Self::find_supporting_files(&directory, path)?;

        // Read source metadata if available
        let source_meta = Self::read_source_meta(&directory);

        Ok(Skill {
            metadata,
            body,
            directory,
            supporting_files,
            source_meta,
        })
    }

    /// Read .skill-meta.json if it exists in the skill directory
    fn read_source_meta(directory: &Path) -> Option<SkillSourceMeta> {
        let meta_path = directory.join(".skill-meta.json");
        if meta_path.exists() {
            match std::fs::read_to_string(&meta_path) {
                Ok(content) => match serde_json::from_str::<SkillSourceMeta>(&content) {
                    Ok(meta) => Some(meta),
                    Err(e) => {
                        tracing::warn!(
                            "Failed to parse .skill-meta.json in {}: {}",
                            directory.display(),
                            e
                        );
                        None
                    }
                },
                Err(e) => {
                    tracing::warn!(
                        "Failed to read .skill-meta.json in {}: {}",
                        directory.display(),
                        e
                    );
                    None
                }
            }
        } else {
            None
        }
    }

    fn parse_frontmatter(content: &str) -> Result<(SkillMetadata, String)> {
        let parts: Vec<&str> = content.split("---").collect();

        if parts.len() < 3 {
            return Err(anyhow::anyhow!("Invalid frontmatter format"));
        }

        let yaml_content = parts[1].trim();
        let metadata: SkillMetadata = serde_yaml::from_str(yaml_content)?;

        let body = parts[2..].join("---").trim().to_string();

        Ok((metadata, body))
    }

    fn find_supporting_files(directory: &Path, skill_file: &Path) -> Result<Vec<PathBuf>> {
        let mut files = Vec::new();

        if let Ok(entries) = std::fs::read_dir(directory) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_file() && path != skill_file {
                    files.push(path);
                } else if path.is_dir() {
                    if let Ok(sub_entries) = std::fs::read_dir(&path) {
                        for sub_entry in sub_entries.flatten() {
                            let sub_path = sub_entry.path();
                            if sub_path.is_file() {
                                files.push(sub_path);
                            }
                        }
                    }
                }
            }
        }

        Ok(files)
    }

    fn discover_skills_in_directories(directories: &[PathBuf]) -> HashMap<String, Skill> {
        let mut skills = HashMap::new();

        for dir in directories {
            if let Ok(entries) = std::fs::read_dir(dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.is_dir() {
                        let skill_file = path.join("SKILL.md");
                        if skill_file.exists() {
                            if let Ok(skill) = Self::parse_skill_file(&skill_file) {
                                // Generate key based on source
                                // Team skills use namespace: {team_id}/{name}
                                // Local skills use: {name}
                                let key = Self::generate_skill_key(&skill);
                                skills.insert(key, skill);
                            }
                        }
                    }
                }
            }
        }

        skills
    }

    /// Generate a unique key for a skill based on its source
    /// - Team skills: `{team_id}/{name}` (e.g., "abc123/commit-helper")
    /// - Local skills: `{name}` (e.g., "commit-helper")
    fn generate_skill_key(skill: &Skill) -> String {
        if let Some(ref source_meta) = skill.source_meta {
            if source_meta.source.as_deref() == Some("team") {
                if let Some(ref team_id) = source_meta.team_id {
                    return format!("{}/{}", team_id, skill.metadata.name);
                }
            }
        }
        // Default: use skill name directly (local skills)
        skill.metadata.name.clone()
    }

    fn generate_instructions(&self) -> String {
        if self.skills.is_empty() {
            return String::new();
        }

        let mut instructions = String::from("You have these skills at your disposal, when it is clear they can help you solve a problem or you are asked to use them:\n\n");

        let mut skill_list: Vec<_> = self.skills.iter().collect();
        skill_list.sort_by_key(|(name, _)| *name);

        for (name, skill) in skill_list {
            instructions.push_str(&format!("- {}: {}\n", name, skill.metadata.description));
        }

        instructions
    }

    /// Find a skill by name, supporting both exact key match and fuzzy name match
    /// - Exact match: `team-id/commit-helper` or `commit-helper`
    /// - Fuzzy match: If only one skill has the given base name, return it
    fn find_skill(&self, name: &str) -> Result<(&String, &Skill), String> {
        // First try exact match
        if let Some(skill) = self.skills.get(name) {
            return Ok((self.skills.get_key_value(name).unwrap().0, skill));
        }

        // If not found, try to find by base name (for backward compatibility)
        // This handles the case where user asks for "commit-helper" but the key is "team-id/commit-helper"
        let matches: Vec<_> = self
            .skills
            .iter()
            .filter(|(key, skill)| {
                // Match if the base name equals the search term
                skill.metadata.name == name ||
                // Or if the key ends with /{name}
                key.ends_with(&format!("/{}", name))
            })
            .collect();

        match matches.len() {
            0 => Err(format!("Skill '{}' not found", name)),
            1 => Ok((matches[0].0, matches[0].1)),
            _ => {
                // Multiple matches - list them for the user
                let options: Vec<_> = matches.iter().map(|(key, _)| key.as_str()).collect();
                Err(format!(
                    "Multiple skills found matching '{}'. Please specify the full name:\n{}",
                    name,
                    options
                        .iter()
                        .map(|k| format!("  - {}", k))
                        .collect::<Vec<_>>()
                        .join("\n")
                ))
            }
        }
    }

    async fn handle_load_skill(
        &self,
        arguments: Option<JsonObject>,
    ) -> Result<Vec<Content>, String> {
        let skill_name = arguments
            .as_ref()
            .ok_or("Missing arguments")?
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or("Missing required parameter: name")?;

        let (_skill_key, skill) = self.find_skill(skill_name)?;

        // Check authorization for team skills
        if let Some(ref source_meta) = skill.source_meta {
            if source_meta.source.as_deref() == Some("team") {
                // Verify authorization
                match Self::verify_team_skill_authorization(source_meta, &skill.directory) {
                    Ok(()) => {
                        tracing::debug!("Team skill '{}' authorization verified", skill_name);
                    }
                    Err(auth_error) => {
                        return Err(format!(
                            "Team skill '{}' authorization failed: {}. Please reinstall the skill using team_install.",
                            skill_name, auth_error
                        ));
                    }
                }
            }
        }

        let mut response = format!("# Skill: {}\n\n{}\n\n", skill.metadata.name, skill.body);

        // Add source information for team skills
        if let Some(ref source_meta) = skill.source_meta {
            if source_meta.source.as_deref() == Some("team") {
                response.push_str(&format!(
                    "## Source Information\n\n- **Source**: Team\n- **Team ID**: {}\n- **Installed**: {}\n- **Version**: {}\n\n",
                    source_meta.team_id.as_deref().unwrap_or("unknown"),
                    source_meta.installed_at.as_deref().unwrap_or("unknown"),
                    source_meta.installed_version.as_deref().unwrap_or("unknown")
                ));
            }
        }

        if !skill.supporting_files.is_empty() {
            response.push_str(&format!(
                "## Supporting Files\n\nSkill directory: {}\n\n",
                skill.directory.display()
            ));
            response.push_str("The following supporting files are available:\n");
            for file in &skill.supporting_files {
                if let Ok(relative) = file.strip_prefix(&skill.directory) {
                    response.push_str(&format!("- {}\n", relative.display()));
                }
            }
            response.push_str("\nUse the view file tools to access these files as needed, or run scripts as directed with dev extension.\n");
        }

        Ok(vec![Content::text(response)])
    }

    /// Verify that a team skill's authorization is still valid
    /// Returns Ok(()) if valid, Err with reason if not
    fn verify_team_skill_authorization(
        source_meta: &SkillSourceMeta,
        skill_directory: &Path,
    ) -> Result<(), String> {
        // Check if authorization exists
        let auth = source_meta
            .authorization
            .as_ref()
            .ok_or_else(|| "No authorization information found".to_string())?;

        // Check if token is empty (invalid)
        if auth.token.is_empty() {
            tracing::warn!(
                "Team skill in {} has empty authorization token",
                skill_directory.display()
            );
            // Allow with warning for backwards compatibility, but log it
            // In future, this should be a hard error
            return Ok(());
        }

        // Check expiration time
        if !auth.expires_at.is_empty() {
            if let Ok(expires_at) = DateTime::parse_from_rfc3339(&auth.expires_at) {
                let now = Utc::now();
                let expires_utc = expires_at.with_timezone(&Utc);

                if now > expires_utc {
                    // Check grace period (72 hours after expiration for offline use)
                    let grace_period = Duration::hours(72);
                    if now > expires_utc + grace_period {
                        return Err(format!(
                            "Authorization expired at {} (grace period exceeded)",
                            auth.expires_at
                        ));
                    } else {
                        tracing::warn!(
                            "Team skill authorization expired at {}, but within grace period",
                            auth.expires_at
                        );
                    }
                }
            } else {
                tracing::warn!(
                    "Could not parse authorization expiration time: {}",
                    auth.expires_at
                );
            }
        }

        // Check last verification time - warn if too old
        if !auth.last_verified_at.is_empty() {
            if let Ok(last_verified) = DateTime::parse_from_rfc3339(&auth.last_verified_at) {
                let now = Utc::now();
                let last_verified_utc = last_verified.with_timezone(&Utc);
                let age = now - last_verified_utc;

                // Warn if not verified in 7 days
                if age > Duration::days(7) {
                    tracing::info!(
                        "Team skill authorization was last verified {} days ago, consider re-verifying",
                        age.num_days()
                    );
                }
            }
        }

        Ok(())
    }

    fn get_tools() -> Vec<Tool> {
        let schema = schema_for!(LoadSkillParams);
        let schema_value =
            serde_json::to_value(schema).expect("Failed to serialize LoadSkillParams schema");

        let input_schema = schema_value
            .as_object()
            .expect("Schema should be an object")
            .clone();

        vec![Tool::new(
            "loadSkill".to_string(),
            indoc! {r#"
                Load a skill by name and return its content.

                This tool loads the specified skill and returns its body content along with
                information about any supporting files in the skill directory.
            "#}
            .to_string(),
            input_schema,
        )
        .annotate(ToolAnnotations {
            title: Some("Load skill".to_string()),
            read_only_hint: Some(true),
            destructive_hint: Some(false),
            idempotent_hint: Some(true),
            open_world_hint: Some(false),
        })]
    }
}

#[async_trait]
impl McpClientTrait for SkillsClient {
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
        let tools = if self.skills.is_empty() {
            Vec::new()
        } else {
            Self::get_tools()
        };
        Ok(ListToolsResult {
            tools,
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
            "loadSkill" => self.handle_load_skill(arguments).await,
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::team_extension::TeamClient;
    use serde_json::json;
    use serial_test::serial;
    use std::fs;
    use std::path::Path;
    use tempfile::TempDir;
    use wiremock::matchers::{body_json, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn first_text_content(result: &CallToolResult) -> String {
        result
            .content
            .iter()
            .find_map(|c| c.as_text().map(|t| t.text.to_string()))
            .unwrap_or_default()
    }

    struct EnvGuard {
        key: String,
        previous: Option<String>,
    }

    impl EnvGuard {
        fn set(key: &str, value: &str) -> Self {
            let previous = std::env::var(key).ok();
            std::env::set_var(key, value);
            Self {
                key: key.to_string(),
                previous,
            }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            if let Some(prev) = &self.previous {
                std::env::set_var(&self.key, prev);
            } else {
                std::env::remove_var(&self.key);
            }
        }
    }

    #[test]
    fn test_parse_frontmatter() {
        let content = r#"---
name: test-skill
description: A test skill
---

# Test Skill

This is the body of the skill.
"#;

        let (metadata, body) = SkillsClient::parse_frontmatter(content).unwrap();
        assert_eq!(metadata.name, "test-skill");
        assert_eq!(metadata.description, "A test skill");
        assert!(body.contains("# Test Skill"));
        assert!(body.contains("This is the body of the skill."));
    }

    #[test]
    fn test_parse_frontmatter_missing() {
        let content = "# No frontmatter here";
        assert!(SkillsClient::parse_frontmatter(content).is_err());
    }

    #[test]
    fn test_parse_frontmatter_unclosed() {
        let content = r#"---
name: test
description: test
"#;
        assert!(SkillsClient::parse_frontmatter(content).is_err());
    }

    #[test]
    fn test_parse_frontmatter_with_extra_fields() {
        let content = r#"---
name: test-skill
description: A test skill
author: Test Author
version: 1.0.0
tags:
  - test
  - example
extra_field: some value
---

# Test Skill

This is the body of the skill.
"#;

        let (metadata, body) = SkillsClient::parse_frontmatter(content).unwrap();
        assert_eq!(metadata.name, "test-skill");
        assert_eq!(metadata.description, "A test skill");
        assert!(body.contains("# Test Skill"));
        assert!(body.contains("This is the body of the skill."));
    }

    #[test]
    fn test_parse_skill_file() {
        let temp_dir = TempDir::new().unwrap();
        let skill_dir = temp_dir.path().join("test-skill");
        fs::create_dir(&skill_dir).unwrap();

        let skill_file = skill_dir.join("SKILL.md");
        fs::write(
            &skill_file,
            r#"---
name: test-skill
description: A test skill
---

# Test Skill Content
"#,
        )
        .unwrap();

        fs::write(skill_dir.join("helper.py"), "print('hello')").unwrap();
        fs::create_dir(skill_dir.join("templates")).unwrap();
        fs::write(skill_dir.join("templates/template.txt"), "template").unwrap();

        let skill = SkillsClient::parse_skill_file(&skill_file).unwrap();
        assert_eq!(skill.metadata.name, "test-skill");
        assert_eq!(skill.metadata.description, "A test skill");
        assert!(skill.body.contains("# Test Skill Content"));
        assert_eq!(skill.supporting_files.len(), 2);
    }

    #[test]
    fn test_discover_skills() {
        let temp_dir = TempDir::new().unwrap();
        let skills_dir = temp_dir.path().join("skills");
        fs::create_dir(&skills_dir).unwrap();

        let skill1_dir = skills_dir.join("test-skill-one-a1b2c3");
        fs::create_dir(&skill1_dir).unwrap();
        fs::write(
            skill1_dir.join("SKILL.md"),
            r#"---
name: test-skill-one-a1b2c3
description: First test skill
---
Body 1
"#,
        )
        .unwrap();

        let skill2_dir = skills_dir.join("test-skill-two-d4e5f6");
        fs::create_dir(&skill2_dir).unwrap();
        fs::write(
            skill2_dir.join("SKILL.md"),
            r#"---
name: test-skill-two-d4e5f6
description: Second test skill
---
Body 2
"#,
        )
        .unwrap();

        let skill3_dir = skills_dir.join("test-skill-three-g7h8i9");
        fs::create_dir(&skill3_dir).unwrap();
        fs::write(
            skill3_dir.join("SKILL.md"),
            r#"---
name: test-skill-three-g7h8i9
description: Third test skill
---
Body 3
"#,
        )
        .unwrap();

        let skills = SkillsClient::discover_skills_in_directories(&[skills_dir]);

        assert_eq!(skills.len(), 3);
        assert!(skills.contains_key("test-skill-one-a1b2c3"));
        assert!(skills.contains_key("test-skill-two-d4e5f6"));
        assert!(skills.contains_key("test-skill-three-g7h8i9"));
    }

    #[test]
    fn test_discover_skills_from_multiple_directories() {
        let temp_dir = TempDir::new().unwrap();

        let dir1 = temp_dir.path().join("dir1");
        fs::create_dir(&dir1).unwrap();
        let skill1_dir = dir1.join("skill-from-dir1");
        fs::create_dir(&skill1_dir).unwrap();
        fs::write(
            skill1_dir.join("SKILL.md"),
            r#"---
name: skill-from-dir1
description: Skill from directory 1
---
Content from dir1
"#,
        )
        .unwrap();

        let dir2 = temp_dir.path().join("dir2");
        fs::create_dir(&dir2).unwrap();
        let skill2_dir = dir2.join("skill-from-dir2");
        fs::create_dir(&skill2_dir).unwrap();
        fs::write(
            skill2_dir.join("SKILL.md"),
            r#"---
name: skill-from-dir2
description: Skill from directory 2
---
Content from dir2
"#,
        )
        .unwrap();

        let dir3 = temp_dir.path().join("dir3");
        fs::create_dir(&dir3).unwrap();
        let skill3_dir = dir3.join("skill-from-dir3");
        fs::create_dir(&skill3_dir).unwrap();
        fs::write(
            skill3_dir.join("SKILL.md"),
            r#"---
name: skill-from-dir3
description: Skill from directory 3
---
Content from dir3
"#,
        )
        .unwrap();

        let skills = SkillsClient::discover_skills_in_directories(&[dir1, dir2, dir3]);

        assert_eq!(skills.len(), 3);
        assert!(skills.contains_key("skill-from-dir1"));
        assert!(skills.contains_key("skill-from-dir2"));
        assert!(skills.contains_key("skill-from-dir3"));

        assert_eq!(
            skills.get("skill-from-dir1").unwrap().metadata.description,
            "Skill from directory 1"
        );
        assert_eq!(
            skills.get("skill-from-dir2").unwrap().metadata.description,
            "Skill from directory 2"
        );
        assert_eq!(
            skills.get("skill-from-dir3").unwrap().metadata.description,
            "Skill from directory 3"
        );
    }

    #[test]
    fn test_empty_instructions_when_no_skills() {
        let temp_dir = TempDir::new().unwrap();
        let empty_dir = temp_dir.path().join("empty");
        fs::create_dir(&empty_dir).unwrap();

        let skills = SkillsClient::discover_skills_in_directories(&[empty_dir]);
        assert_eq!(skills.len(), 0);

        let mut client = SkillsClient {
            info: InitializeResult {
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
                    title: Some("Skills".to_string()),
                    version: "1.0.0".to_string(),
                    icons: None,
                    website_url: None,
                },
                instructions: Some(String::new()),
            },
            skills,
        };

        let instructions = client.generate_instructions();
        assert_eq!(instructions, "");
        assert!(instructions.is_empty());

        client.info.instructions = Some(instructions);
        assert_eq!(client.info.instructions.as_ref().unwrap(), "");
    }

    #[tokio::test]
    async fn test_no_tools_when_no_skills() {
        let temp_dir = TempDir::new().unwrap();
        let empty_dir = temp_dir.path().join("empty");
        fs::create_dir(&empty_dir).unwrap();

        let skills = SkillsClient::discover_skills_in_directories(&[empty_dir]);
        assert_eq!(skills.len(), 0);

        let client = SkillsClient {
            info: InitializeResult {
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
                    title: Some("Skills".to_string()),
                    version: "1.0.0".to_string(),
                    icons: None,
                    website_url: None,
                },
                instructions: Some(String::new()),
            },
            skills,
        };

        let result = client
            .list_tools(None, CancellationToken::new())
            .await
            .unwrap();
        assert_eq!(result.tools.len(), 0);
    }

    #[tokio::test]
    async fn test_tools_available_when_skills_exist() {
        let temp_dir = TempDir::new().unwrap();
        let skills_dir = temp_dir.path().join("skills");
        fs::create_dir(&skills_dir).unwrap();

        let skill_dir = skills_dir.join("test-skill");
        fs::create_dir(&skill_dir).unwrap();
        fs::write(
            skill_dir.join("SKILL.md"),
            r#"---
name: test-skill
description: A test skill
---
Content
"#,
        )
        .unwrap();

        let skills = SkillsClient::discover_skills_in_directories(&[skills_dir]);
        assert_eq!(skills.len(), 1);

        let client = SkillsClient {
            info: InitializeResult {
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
                    title: Some("Skills".to_string()),
                    version: "1.0.0".to_string(),
                    icons: None,
                    website_url: None,
                },
                instructions: Some(String::new()),
            },
            skills,
        };

        let result = client
            .list_tools(None, CancellationToken::new())
            .await
            .unwrap();
        assert_eq!(result.tools.len(), 1);
        assert_eq!(result.tools[0].name, "loadSkill");
    }

    #[test]
    fn test_instructions_with_skills() {
        let temp_dir = TempDir::new().unwrap();
        let skills_dir = temp_dir.path().join("skills");
        fs::create_dir(&skills_dir).unwrap();

        let skill1_dir = skills_dir.join("alpha-skill");
        fs::create_dir(&skill1_dir).unwrap();
        fs::write(
            skill1_dir.join("SKILL.md"),
            r#"---
name: alpha-skill
description: First skill alphabetically
---
Content
"#,
        )
        .unwrap();

        let skill2_dir = skills_dir.join("beta-skill");
        fs::create_dir(&skill2_dir).unwrap();
        fs::write(
            skill2_dir.join("SKILL.md"),
            r#"---
name: beta-skill
description: Second skill alphabetically
---
Content
"#,
        )
        .unwrap();

        let skills = SkillsClient::discover_skills_in_directories(&[skills_dir]);
        assert_eq!(skills.len(), 2);

        let mut client = SkillsClient {
            info: InitializeResult {
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
                    title: Some("Skills".to_string()),
                    version: "1.0.0".to_string(),
                    icons: None,
                    website_url: None,
                },
                instructions: Some(String::new()),
            },
            skills,
        };

        let instructions = client.generate_instructions();
        assert!(!instructions.is_empty());
        assert!(instructions.contains("You have these skills at your disposal"));
        assert!(instructions.contains("alpha-skill: First skill alphabetically"));
        assert!(instructions.contains("beta-skill: Second skill alphabetically"));

        let lines: Vec<&str> = instructions.lines().collect();
        let alpha_line = lines
            .iter()
            .position(|l| l.contains("alpha-skill"))
            .unwrap();
        let beta_line = lines.iter().position(|l| l.contains("beta-skill")).unwrap();
        assert!(alpha_line < beta_line);

        client.info.instructions = Some(instructions);
        assert!(!client.info.instructions.as_ref().unwrap().is_empty());
    }

    #[test]
    fn test_discover_skills_working_dir_overrides_global() {
        let temp_dir = TempDir::new().unwrap();

        // Simulate ~/.claude/skills (global, lowest priority)
        let global_claude = temp_dir.path().join("global-claude");
        fs::create_dir(&global_claude).unwrap();
        let skill_global_claude = global_claude.join("my-skill");
        fs::create_dir(&skill_global_claude).unwrap();
        fs::write(
            skill_global_claude.join("SKILL.md"),
            r#"---
name: my-skill
description: From global claude
---
Global claude content
"#,
        )
        .unwrap();

        // Simulate ~/.config/agime/skills (global, medium priority)
        let global_agime = temp_dir.path().join("global-agime");
        fs::create_dir(&global_agime).unwrap();
        let skill_global_agime = global_agime.join("my-skill");
        fs::create_dir(&skill_global_agime).unwrap();
        fs::write(
            skill_global_agime.join("SKILL.md"),
            r#"---
name: my-skill
description: From global agime config
---
Global agime config content
"#,
        )
        .unwrap();

        // Simulate $PWD/.claude/skills (working dir, higher priority)
        let working_claude = temp_dir.path().join("working-claude");
        fs::create_dir(&working_claude).unwrap();
        let skill_working_claude = working_claude.join("my-skill");
        fs::create_dir(&skill_working_claude).unwrap();
        fs::write(
            skill_working_claude.join("SKILL.md"),
            r#"---
name: my-skill
description: From working dir claude
---
Working dir claude content
"#,
        )
        .unwrap();

        // Simulate $PWD/.agime/skills (working dir, highest priority)
        let working_agime = temp_dir.path().join("working-agime");
        fs::create_dir(&working_agime).unwrap();
        let skill_working_agime = working_agime.join("my-skill");
        fs::create_dir(&skill_working_agime).unwrap();
        fs::write(
            skill_working_agime.join("SKILL.md"),
            r#"---
name: my-skill
description: From working dir agime
---
Working dir agime content
"#,
        )
        .unwrap();

        // Test priority order: global_claude < global_agime < working_claude < working_agime
        let skills = SkillsClient::discover_skills_in_directories(&[
            global_claude,
            global_agime,
            working_claude,
            working_agime,
        ]);

        assert_eq!(skills.len(), 1);
        assert!(skills.contains_key("my-skill"));
        // The last directory (working_agime) should win
        assert_eq!(
            skills.get("my-skill").unwrap().metadata.description,
            "From working dir agime"
        );
        assert!(skills
            .get("my-skill")
            .unwrap()
            .body
            .contains("Working dir agime content"));
    }

    #[tokio::test]
    async fn test_remote_non_public_team_skill_with_camelcase_meta_passes_authorization() {
        let temp_dir = TempDir::new().unwrap();
        let skills_dir = temp_dir.path().join("skills");
        fs::create_dir(&skills_dir).unwrap();

        let skill_dir = skills_dir.join("remote-secure-skill");
        fs::create_dir(&skill_dir).unwrap();
        fs::write(
            skill_dir.join("SKILL.md"),
            r#"---
name: remote-secure-skill
description: Team skill installed from remote
---
Sensitive team skill content
"#,
        )
        .unwrap();

        let now = Utc::now();
        let meta = serde_json::json!({
            "source": "team",
            "teamId": "team-remote-1",
            "resourceId": "skill-remote-1",
            "installedAt": now.to_rfc3339(),
            "installedVersion": "1.0.0",
            "protectionLevel": "team_installable",
            "userId": "user-remote-1",
            "authorization": {
                "token": "auth-token-123",
                "expiresAt": (now + Duration::hours(24)).to_rfc3339(),
                "lastVerifiedAt": now.to_rfc3339(),
            }
        });
        fs::write(
            skill_dir.join(".skill-meta.json"),
            serde_json::to_string_pretty(&meta).unwrap(),
        )
        .unwrap();

        let skills = SkillsClient::discover_skills_in_directories(&[skills_dir]);
        let discovered = skills
            .get("remote-secure-skill")
            .expect("team skill should be discovered");
        let source_meta = discovered
            .source_meta
            .as_ref()
            .expect("source metadata should be parsed");
        assert_eq!(source_meta.source.as_deref(), Some("team"));
        assert_eq!(source_meta.team_id.as_deref(), Some("team-remote-1"));
        assert!(source_meta.authorization.is_some());

        let client = SkillsClient {
            info: InitializeResult {
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
                    title: Some("Skills".to_string()),
                    version: "1.0.0".to_string(),
                    icons: None,
                    website_url: None,
                },
                instructions: Some(String::new()),
            },
            skills,
        };

        let args = serde_json::json!({ "name": "remote-secure-skill" });
        let result = client.handle_load_skill(args.as_object().cloned()).await;
        assert!(
            result.is_ok(),
            "team skill load should pass authorization verification, got: {:?}",
            result.err()
        );
    }

    #[tokio::test]
    #[serial]
    async fn test_remote_install_non_public_skill_then_load_passes_authorization() {
        let temp_dir = TempDir::new().unwrap();
        let path_root = temp_dir.path().join("agime-root");
        fs::create_dir_all(&path_root).unwrap();

        let server = MockServer::start().await;
        let skill_id = "skill-e2e-remote-1";
        let skill_name = "remote-secure-skill-e2e";
        let expires_at = (Utc::now() + Duration::hours(24)).to_rfc3339();

        Mock::given(method("GET"))
            .and(path(format!("/api/team/skills/{}", skill_id)))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": skill_id,
                "teamId": "team-e2e-1",
                "name": skill_name,
                "description": "remote secure team skill",
                "content": format!(
                    "---\nname: {}\ndescription: remote secure team skill\n---\nSensitive team skill content",
                    skill_name
                ),
                "protectionLevel": "team_installable",
                "version": "1.0.0"
            })))
            .expect(1)
            .mount(&server)
            .await;

        Mock::given(method("POST"))
            .and(path(format!("/api/team/skills/{}/verify-access", skill_id)))
            .and(body_json(json!({})))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "authorized": true,
                "token": "auth-token-e2e",
                "expiresAt": expires_at,
            })))
            .expect(1)
            .mount(&server)
            .await;

        Mock::given(method("POST"))
            .and(path(format!("/api/team/skills/{}/install", skill_id)))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "success": true
            })))
            .expect(1)
            .mount(&server)
            .await;

        let _api_guard = EnvGuard::set("AGIME_TEAM_API_URL", &server.uri());
        let _path_guard = EnvGuard::set("AGIME_PATH_ROOT", path_root.to_string_lossy().as_ref());

        let context = PlatformExtensionContext {
            session_id: None,
            extension_manager: None,
            tool_route_manager: None,
        };
        let team_client = TeamClient::new(context.clone()).expect("team client init should pass");

        let install_args = json!({
            "resource_type": "skill",
            "resource_id": skill_id,
        });
        let install_result = team_client
            .call_tool(
                "team_install",
                install_args.as_object().cloned(),
                CancellationToken::new(),
            )
            .await
            .expect("team_install call should return");
        let install_text = first_text_content(&install_result);
        assert!(
            !install_text.starts_with("Error:"),
            "team_install should succeed, got: {}",
            install_text
        );

        let install_dir = Path::new(&path_root).join("skills").join(skill_name);
        assert!(
            install_dir.join("SKILL.md").exists(),
            "installed skill file should exist"
        );
        assert!(
            install_dir.join(".skill-meta.json").exists(),
            "installed skill metadata should exist"
        );

        let skills_client =
            SkillsClient::new(context).expect("skills client init should discover installed skill");
        let load_args = json!({ "name": skill_name });
        let load_result = skills_client
            .call_tool(
                "loadSkill",
                load_args.as_object().cloned(),
                CancellationToken::new(),
            )
            .await
            .expect("loadSkill call should return");
        let load_text = first_text_content(&load_result);
        assert!(
            !load_text.starts_with("Error:"),
            "loadSkill should pass authorization checks, got: {}",
            load_text
        );
        assert!(
            load_text.contains("Sensitive team skill content"),
            "loadSkill response should contain skill content"
        );
    }

    #[tokio::test]
    #[serial]
    async fn test_remote_install_non_public_skill_denied_authorization_fails() {
        let temp_dir = TempDir::new().unwrap();
        let path_root = temp_dir.path().join("agime-root");
        fs::create_dir_all(&path_root).unwrap();

        let server = MockServer::start().await;
        let skill_id = "skill-e2e-denied-1";
        let skill_name = "remote-secure-skill-denied";

        Mock::given(method("GET"))
            .and(path(format!("/api/team/skills/{}", skill_id)))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": skill_id,
                "teamId": "team-e2e-1",
                "name": skill_name,
                "description": "remote secure team skill",
                "content": format!(
                    "---\nname: {}\ndescription: remote secure team skill\n---\nSensitive team skill content",
                    skill_name
                ),
                "protectionLevel": "team_installable",
                "version": "1.0.0"
            })))
            .expect(1)
            .mount(&server)
            .await;

        Mock::given(method("POST"))
            .and(path(format!("/api/team/skills/{}/verify-access", skill_id)))
            .and(body_json(json!({})))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "authorized": false,
                "error": "User is not a member of this team"
            })))
            .expect(1)
            .mount(&server)
            .await;

        let _api_guard = EnvGuard::set("AGIME_TEAM_API_URL", &server.uri());
        let _path_guard = EnvGuard::set("AGIME_PATH_ROOT", path_root.to_string_lossy().as_ref());

        let context = PlatformExtensionContext {
            session_id: None,
            extension_manager: None,
            tool_route_manager: None,
        };
        let team_client = TeamClient::new(context).expect("team client init should pass");

        let install_args = json!({
            "resource_type": "skill",
            "resource_id": skill_id,
        });
        let install_result = team_client
            .call_tool(
                "team_install",
                install_args.as_object().cloned(),
                CancellationToken::new(),
            )
            .await
            .expect("team_install call should return");
        let install_text = first_text_content(&install_result);

        assert!(
            install_text.starts_with("Error:"),
            "team_install should fail when authorization is denied, got: {}",
            install_text
        );
        assert!(
            install_text.contains("Failed to authorize installation")
                || install_text.contains("not a member"),
            "error should include authorization denial context, got: {}",
            install_text
        );
    }

    #[tokio::test]
    #[serial]
    async fn test_remote_install_recipe_uses_install_local_endpoint() {
        let server = MockServer::start().await;
        let recipe_id = "recipe-e2e-remote-1";
        let recipe_name = "remote-recipe-e2e";
        let recipe_yaml = "name: remote-recipe-e2e\nsteps:\n  - run: echo hello";

        Mock::given(method("GET"))
            .and(path(format!("/api/team/recipes/{}", recipe_id)))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": recipe_id,
                "teamId": "team-e2e-recipes",
                "name": recipe_name,
                "contentYaml": recipe_yaml,
                "version": "1.2.3",
                "protectionLevel": "team_installable"
            })))
            .expect(1)
            .mount(&server)
            .await;

        Mock::given(method("POST"))
            .and(path("/api/team/recipes/install-local"))
            .and(body_json(json!({
                "resourceId": recipe_id,
                "teamId": "team-e2e-recipes",
                "name": recipe_name,
                "contentYaml": recipe_yaml,
                "version": "1.2.3",
                "protectionLevel": "team_installable"
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "success": true,
                "resourceType": "recipe",
                "resourceId": recipe_id,
                "installedVersion": "1.2.3",
                "localPath": "/tmp/team-recipes/remote-recipe-e2e.yaml"
            })))
            .expect(1)
            .mount(&server)
            .await;

        // Ensure old fallback path is not used when local install succeeds.
        Mock::given(method("POST"))
            .and(path(format!("/api/team/recipes/{}/install", recipe_id)))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "success": true })))
            .expect(0)
            .mount(&server)
            .await;

        let _api_guard = EnvGuard::set("AGIME_TEAM_API_URL", &server.uri());
        let _local_api_guard = EnvGuard::set("AGIME_LOCAL_TEAM_API_URL", &server.uri());

        let context = PlatformExtensionContext {
            session_id: None,
            extension_manager: None,
            tool_route_manager: None,
        };
        let team_client = TeamClient::new(context).expect("team client init should pass");

        let install_args = json!({
            "resource_type": "recipe",
            "resource_id": recipe_id,
        });
        let install_result = team_client
            .call_tool(
                "team_install",
                install_args.as_object().cloned(),
                CancellationToken::new(),
            )
            .await
            .expect("team_install call should return");
        let install_text = first_text_content(&install_result);
        assert!(
            !install_text.starts_with("Error:"),
            "recipe team_install should succeed, got: {}",
            install_text
        );

        let install_json: Value =
            serde_json::from_str(&install_text).expect("install response should be valid JSON");
        assert_eq!(install_json["status"], "installed");
        assert_eq!(install_json["resource_type"], "recipe");
        assert_eq!(install_json["resource_id"], recipe_id);
    }

    #[tokio::test]
    #[serial]
    async fn test_remote_install_extension_uses_install_local_endpoint() {
        let server = MockServer::start().await;
        let extension_id = "extension-e2e-remote-1";
        let extension_name = "remote-extension-e2e";
        let extension_config = json!({
            "command": "npx",
            "args": ["-y", "@agime/sample-mcp"],
            "env": { "NODE_ENV": "production" }
        });

        Mock::given(method("GET"))
            .and(path(format!("/api/team/extensions/{}", extension_id)))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": extension_id,
                "teamId": "team-e2e-extensions",
                "name": extension_name,
                "extensionType": "stdio",
                "config": extension_config.clone(),
                "version": "2.0.0",
                "protectionLevel": "team_installable"
            })))
            .expect(1)
            .mount(&server)
            .await;

        Mock::given(method("POST"))
            .and(path("/api/team/extensions/install-local"))
            .and(body_json(json!({
                "resourceId": extension_id,
                "teamId": "team-e2e-extensions",
                "name": extension_name,
                "extensionType": "stdio",
                "config": extension_config,
                "version": "2.0.0",
                "protectionLevel": "team_installable"
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "success": true,
                "resourceType": "extension",
                "resourceId": extension_id,
                "installedVersion": "2.0.0",
                "localPath": "/tmp/team-extensions/remote-extension-e2e"
            })))
            .expect(1)
            .mount(&server)
            .await;

        // Ensure old fallback path is not used when local install succeeds.
        Mock::given(method("POST"))
            .and(path(format!(
                "/api/team/extensions/{}/install",
                extension_id
            )))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "success": true })))
            .expect(0)
            .mount(&server)
            .await;

        let _api_guard = EnvGuard::set("AGIME_TEAM_API_URL", &server.uri());
        let _local_api_guard = EnvGuard::set("AGIME_LOCAL_TEAM_API_URL", &server.uri());

        let context = PlatformExtensionContext {
            session_id: None,
            extension_manager: None,
            tool_route_manager: None,
        };
        let team_client = TeamClient::new(context).expect("team client init should pass");

        let install_args = json!({
            "resource_type": "extension",
            "resource_id": extension_id,
        });
        let install_result = team_client
            .call_tool(
                "team_install",
                install_args.as_object().cloned(),
                CancellationToken::new(),
            )
            .await
            .expect("team_install call should return");
        let install_text = first_text_content(&install_result);
        assert!(
            !install_text.starts_with("Error:"),
            "extension team_install should succeed, got: {}",
            install_text
        );

        let install_json: Value =
            serde_json::from_str(&install_text).expect("install response should be valid JSON");
        assert_eq!(install_json["status"], "installed");
        assert_eq!(install_json["resource_type"], "extension");
        assert_eq!(install_json["resource_id"], extension_id);
    }
}
