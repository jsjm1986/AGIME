//! Configuration management for the team server

use anyhow::{Context, Result};
use serde::Deserialize;

/// Server configuration
#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    /// Server host (default: 0.0.0.0)
    #[serde(default = "default_host")]
    pub host: String,

    /// Server port (default: 8080)
    #[serde(default = "default_port")]
    pub port: u16,

    /// Database URL (default: sqlite://./data/team.db)
    #[serde(default = "default_database_url")]
    pub database_url: String,

    /// Maximum database connections (default: 10)
    #[serde(default = "default_max_connections")]
    pub max_connections: u32,

    /// API Key for admin operations (optional)
    pub admin_api_key: Option<String>,

    /// Base URL for invite links (e.g., http://example.com:8080)
    /// If not set, will try to auto-detect from host and port
    pub base_url: Option<String>,
}

fn default_host() -> String {
    "0.0.0.0".to_string()
}

fn default_port() -> u16 {
    8080
}

fn default_database_url() -> String {
    "sqlite://./data/team.db?mode=rwc".to_string()
}

fn default_max_connections() -> u32 {
    10
}

impl Config {
    /// Load configuration from environment variables
    pub fn from_env() -> Result<Self> {
        let host = std::env::var("TEAM_SERVER_HOST").unwrap_or_else(|_| default_host());
        let port = std::env::var("TEAM_SERVER_PORT")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or_else(default_port);
        let database_url =
            std::env::var("DATABASE_URL").unwrap_or_else(|_| default_database_url());
        let max_connections = std::env::var("DATABASE_MAX_CONNECTIONS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or_else(default_max_connections);
        let admin_api_key = std::env::var("ADMIN_API_KEY").ok();
        let base_url = std::env::var("BASE_URL").ok();

        Ok(Self {
            host,
            port,
            database_url,
            max_connections,
            admin_api_key,
            base_url,
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
            host: default_host(),
            port: default_port(),
            database_url: default_database_url(),
            max_connections: default_max_connections(),
            admin_api_key: None,
            base_url: None,
        }
    }
}
