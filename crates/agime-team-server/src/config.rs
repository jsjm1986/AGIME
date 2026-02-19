//! Configuration management for the team server

use anyhow::{Context, Result};
use serde::Deserialize;
use std::str::FromStr;

/// Database backend type
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DatabaseType {
    #[default]
    MongoDB,
    SQLite,
}

impl FromStr for DatabaseType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "mongodb" | "mongo" => Ok(DatabaseType::MongoDB),
            "sqlite" | "sql" => Ok(DatabaseType::SQLite),
            _ => Err(format!("Unknown database type: {}", s)),
        }
    }
}

/// Server configuration
#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    /// Database type (default: mongodb)
    #[serde(default)]
    pub database_type: DatabaseType,

    /// Server host (default: 0.0.0.0)
    #[serde(default = "default_host")]
    pub host: String,

    /// Server port (default: 8080)
    #[serde(default = "default_port")]
    pub port: u16,

    /// Database URL (default: mongodb://localhost:27017)
    #[serde(default = "default_database_url")]
    pub database_url: String,

    /// Database name (default: agime_team)
    #[serde(default = "default_database_name")]
    pub database_name: String,

    /// Maximum database connections (default: 10)
    #[serde(default = "default_max_connections")]
    pub max_connections: u32,

    /// API Key for admin operations (optional)
    pub admin_api_key: Option<String>,

    /// Emails that are automatically assigned admin role (comma-separated env var)
    #[serde(default)]
    pub admin_emails: Vec<String>,

    /// Base URL for invite links (e.g., http://example.com:8080)
    /// If not set, will try to auto-detect from host and port
    pub base_url: Option<String>,
    /// Optional test URL base for published portal links (e.g., http://127.0.0.1:8080 or http://10.0.0.12:8080)
    /// Useful when BASE_URL points to domain but team still wants IP:port testing links.
    pub portal_test_base_url: Option<String>,

    /// CORS allowed origins (comma-separated). If empty, mirror_request is used (dev mode).
    pub cors_allowed_origins: Option<String>,

    /// Registration mode: "open" | "approval" | "disabled" (default: "open")
    #[serde(default = "default_registration_mode")]
    pub registration_mode: String,

    /// Maximum API keys per user (default: 10)
    #[serde(default = "default_max_api_keys_per_user")]
    pub max_api_keys_per_user: u32,

    /// Maximum login failures before lockout (default: 5)
    #[serde(default = "default_login_max_failures")]
    pub login_max_failures: u32,

    /// Lockout duration in minutes after max failures (default: 15)
    #[serde(default = "default_login_lockout_minutes")]
    pub login_lockout_minutes: u32,

    /// Session sliding window in hours - renew if remaining < this (default: 2)
    #[serde(default = "default_session_sliding_window_hours")]
    pub session_sliding_window_hours: u32,

    /// Whether to set Secure flag on cookies (default: false)
    #[serde(default)]
    pub secure_cookies: bool,

    /// AI Describe: dedicated API key (falls back to team agent config)
    pub ai_describe_api_key: Option<String>,
    /// AI Describe: model name (falls back to team agent config)
    pub ai_describe_model: Option<String>,
    /// AI Describe: API URL (falls back to team agent config)
    pub ai_describe_api_url: Option<String>,
    /// AI Describe: API format - "anthropic" or "openai" (falls back to team agent config)
    pub ai_describe_api_format: Option<String>,

    /// Root directory for per-mission/per-session workspace isolation
    #[serde(default = "default_workspace_root")]
    pub workspace_root: String,

    /// Team agent resource mode: explicit | auto
    #[serde(default = "default_team_agent_resource_mode")]
    pub team_agent_resource_mode: String,

    /// Team agent skill mode: assigned | on_demand
    #[serde(default = "default_team_agent_skill_mode")]
    pub team_agent_skill_mode: String,

    /// Auto extension policy in auto mode: reviewed_only | all
    #[serde(default = "default_team_agent_auto_extension_policy")]
    pub team_agent_auto_extension_policy: String,

    /// Whether stdio team extensions are auto-installed if command is missing
    #[serde(default = "default_team_agent_auto_install_extensions")]
    pub team_agent_auto_install_extensions: bool,

    /// Cache root for auto-installed team extensions
    #[serde(default = "default_team_agent_extension_cache_root")]
    pub team_agent_extension_cache_root: String,
}

fn default_host() -> String {
    "0.0.0.0".to_string()
}

fn default_port() -> u16 {
    8080
}

fn default_database_url() -> String {
    "mongodb://localhost:27017".to_string()
}

fn default_database_name() -> String {
    "agime_team".to_string()
}

fn default_max_connections() -> u32 {
    10
}

fn default_workspace_root() -> String {
    "./data/workspaces".to_string()
}

fn default_team_agent_resource_mode() -> String {
    "explicit".to_string()
}

fn default_team_agent_skill_mode() -> String {
    "on_demand".to_string()
}

fn default_team_agent_auto_extension_policy() -> String {
    "reviewed_only".to_string()
}

fn default_team_agent_auto_install_extensions() -> bool {
    true
}

fn default_team_agent_extension_cache_root() -> String {
    "./data/runtime/extensions".to_string()
}

fn default_registration_mode() -> String {
    "open".to_string()
}

fn default_max_api_keys_per_user() -> u32 {
    10
}

fn default_login_max_failures() -> u32 {
    5
}

fn default_login_lockout_minutes() -> u32 {
    15
}

fn default_session_sliding_window_hours() -> u32 {
    2
}

impl Config {
    /// Load configuration from environment variables
    pub fn from_env() -> Result<Self> {
        let database_type = std::env::var("DATABASE_TYPE")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or_default();
        let host = std::env::var("TEAM_SERVER_HOST").unwrap_or_else(|_| default_host());
        let port = std::env::var("TEAM_SERVER_PORT")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or_else(default_port);
        let database_url = std::env::var("DATABASE_URL")
            .or_else(|_| std::env::var("MONGODB_URL"))
            .unwrap_or_else(|_| default_database_url());
        let database_name = std::env::var("DATABASE_NAME")
            .or_else(|_| std::env::var("MONGODB_DATABASE"))
            .unwrap_or_else(|_| default_database_name());
        let max_connections = std::env::var("DATABASE_MAX_CONNECTIONS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or_else(default_max_connections);
        let admin_api_key = std::env::var("ADMIN_API_KEY").ok();
        let admin_emails: Vec<String> = std::env::var("ADMIN_EMAILS")
            .unwrap_or_default()
            .split(',')
            .map(|s| s.trim().to_lowercase())
            .filter(|s| !s.is_empty())
            .collect();
        let base_url = std::env::var("BASE_URL").ok();
        let portal_test_base_url = std::env::var("PORTAL_TEST_BASE_URL").ok();
        let cors_allowed_origins = std::env::var("CORS_ALLOWED_ORIGINS").ok();
        let registration_mode =
            std::env::var("REGISTRATION_MODE").unwrap_or_else(|_| "open".to_string());
        // Validate registration mode to prevent silent fallthrough on typos
        match registration_mode.as_str() {
            "open" | "approval" | "disabled" => {}
            other => anyhow::bail!(
                "Invalid REGISTRATION_MODE '{}'. Must be one of: open, approval, disabled",
                other
            ),
        }
        let max_api_keys_per_user = std::env::var("MAX_API_KEYS_PER_USER")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(10);
        let login_max_failures = std::env::var("LOGIN_MAX_FAILURES")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(5);
        let login_lockout_minutes = std::env::var("LOGIN_LOCKOUT_MINUTES")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(15);
        let session_sliding_window_hours = std::env::var("SESSION_SLIDING_WINDOW_HOURS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(2);
        let secure_cookies = std::env::var("SECURE_COOKIES")
            .map(|s| s.eq_ignore_ascii_case("true"))
            .unwrap_or(false);

        let ai_describe_api_key = std::env::var("AI_DESCRIBE_API_KEY").ok();
        let ai_describe_model = std::env::var("AI_DESCRIBE_MODEL").ok();
        let ai_describe_api_url = std::env::var("AI_DESCRIBE_API_URL").ok();
        let ai_describe_api_format = std::env::var("AI_DESCRIBE_API_FORMAT").ok();
        let workspace_root =
            std::env::var("WORKSPACE_ROOT").unwrap_or_else(|_| default_workspace_root());
        let team_agent_resource_mode = std::env::var("TEAM_AGENT_RESOURCE_MODE")
            .unwrap_or_else(|_| default_team_agent_resource_mode());
        let team_agent_skill_mode = std::env::var("TEAM_AGENT_SKILL_MODE")
            .unwrap_or_else(|_| default_team_agent_skill_mode());
        let team_agent_auto_extension_policy = std::env::var("TEAM_AGENT_AUTO_EXTENSION_POLICY")
            .unwrap_or_else(|_| default_team_agent_auto_extension_policy());
        let team_agent_auto_install_extensions =
            std::env::var("TEAM_AGENT_AUTO_INSTALL_EXTENSIONS")
                .map(|s| s.eq_ignore_ascii_case("true"))
                .unwrap_or_else(|_| default_team_agent_auto_install_extensions());
        let team_agent_extension_cache_root = std::env::var("TEAM_AGENT_EXTENSION_CACHE_ROOT")
            .unwrap_or_else(|_| default_team_agent_extension_cache_root());

        Ok(Self {
            database_type,
            host,
            port,
            database_url,
            database_name,
            max_connections,
            admin_api_key,
            admin_emails,
            base_url,
            portal_test_base_url,
            cors_allowed_origins,
            registration_mode,
            max_api_keys_per_user,
            login_max_failures,
            login_lockout_minutes,
            session_sliding_window_hours,
            secure_cookies,
            ai_describe_api_key,
            ai_describe_model,
            ai_describe_api_url,
            ai_describe_api_format,
            workspace_root,
            team_agent_resource_mode,
            team_agent_skill_mode,
            team_agent_auto_extension_policy,
            team_agent_auto_install_extensions,
            team_agent_extension_cache_root,
        })
    }

    /// Load configuration from a TOML file
    #[allow(dead_code)]
    pub fn from_file(path: &str) -> Result<Self> {
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read config file: {}", path))?;
        let config: Config =
            toml::from_str(&content).with_context(|| "Failed to parse config file")?;
        Ok(config)
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            database_type: DatabaseType::default(),
            host: default_host(),
            port: default_port(),
            database_url: default_database_url(),
            database_name: default_database_name(),
            max_connections: default_max_connections(),
            admin_api_key: None,
            admin_emails: Vec::new(),
            base_url: None,
            portal_test_base_url: None,
            cors_allowed_origins: None,
            registration_mode: default_registration_mode(),
            max_api_keys_per_user: default_max_api_keys_per_user(),
            login_max_failures: default_login_max_failures(),
            login_lockout_minutes: default_login_lockout_minutes(),
            session_sliding_window_hours: default_session_sliding_window_hours(),
            secure_cookies: false,
            ai_describe_api_key: None,
            ai_describe_model: None,
            ai_describe_api_url: None,
            ai_describe_api_format: None,
            workspace_root: default_workspace_root(),
            team_agent_resource_mode: default_team_agent_resource_mode(),
            team_agent_skill_mode: default_team_agent_skill_mode(),
            team_agent_auto_extension_policy: default_team_agent_auto_extension_policy(),
            team_agent_auto_install_extensions: default_team_agent_auto_install_extensions(),
            team_agent_extension_cache_root: default_team_agent_extension_cache_root(),
        }
    }
}
