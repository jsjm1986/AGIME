//! Team configuration module

use serde::{Deserialize, Serialize};

/// Team feature configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamConfig {
    /// Whether team features are enabled
    #[serde(default)]
    pub enabled: bool,

    /// Whether to allow installation of unreviewed extensions
    #[serde(default)]
    pub allow_unreviewed_extensions: bool,

    /// Whether to auto-install dependencies when installing a resource
    #[serde(default)]
    pub auto_install_dependencies: bool,

    /// Default visibility for shared resources
    #[serde(default = "default_visibility")]
    pub default_visibility: String,

    /// Maximum number of teams a user can create
    #[serde(default = "default_max_teams")]
    pub max_teams_per_user: usize,

    /// Maximum number of members per team
    #[serde(default = "default_max_members")]
    pub max_members_per_team: usize,

    /// Soft delete retention period in days (0 = forever)
    #[serde(default = "default_retention_days")]
    pub soft_delete_retention_days: u32,

    /// Git sync interval in seconds (0 = manual only)
    #[serde(default)]
    pub sync_interval_seconds: u64,
}

fn default_visibility() -> String {
    "team".to_string()
}

fn default_max_teams() -> usize {
    10
}

fn default_max_members() -> usize {
    100
}

fn default_retention_days() -> u32 {
    30
}

impl Default for TeamConfig {
    fn default() -> Self {
        Self {
            enabled: false, // Disabled by default
            allow_unreviewed_extensions: false,
            auto_install_dependencies: false,
            default_visibility: default_visibility(),
            max_teams_per_user: default_max_teams(),
            max_members_per_team: default_max_members(),
            soft_delete_retention_days: default_retention_days(),
            sync_interval_seconds: 0,
        }
    }
}

impl TeamConfig {
    /// Create a new team config with default values
    pub fn new() -> Self {
        Self::default()
    }

    /// Check if team features are enabled
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Enable team features
    pub fn enable(&mut self) {
        self.enabled = true;
    }

    /// Disable team features
    pub fn disable(&mut self) {
        self.enabled = false;
    }
}
