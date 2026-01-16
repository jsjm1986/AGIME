//! Team model

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Team entity
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Team {
    pub id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repository_url: Option<String>,
    pub owner_id: String,
    #[serde(default)]
    pub is_deleted: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    #[serde(default)]
    pub settings: TeamSettings,
}

/// Team settings
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TeamSettings {
    /// Whether to require security review for extensions
    #[serde(default = "default_true")]
    pub require_extension_review: bool,
    /// Whether members can invite other members
    #[serde(default)]
    pub members_can_invite: bool,
    /// Default visibility for resources shared to this team
    #[serde(default = "default_visibility")]
    pub default_visibility: String,
}

fn default_true() -> bool {
    true
}

fn default_visibility() -> String {
    "team".to_string()
}

impl Team {
    /// Create a new team
    pub fn new(name: String, owner_id: String) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4().to_string(),
            name,
            description: None,
            repository_url: None,
            owner_id,
            is_deleted: false,
            created_at: now,
            updated_at: now,
            settings: TeamSettings::default(),
        }
    }

    /// Create a team with description
    pub fn with_description(mut self, description: String) -> Self {
        self.description = Some(description);
        self
    }

    /// Create a team with repository URL
    pub fn with_repository(mut self, url: String) -> Self {
        self.repository_url = Some(url);
        self
    }
}

/// Request to create a team
#[derive(Debug, Clone, Deserialize)]
pub struct CreateTeamRequest {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub repository_url: Option<String>,
    #[serde(default)]
    pub settings: Option<TeamSettings>,
}

/// Request to update a team
#[derive(Debug, Clone, Deserialize)]
pub struct UpdateTeamRequest {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub repository_url: Option<String>,
    #[serde(default)]
    pub settings: Option<TeamSettings>,
}

/// Team summary with counts
#[derive(Debug, Clone, Serialize)]
pub struct TeamSummary {
    #[serde(flatten)]
    pub team: Team,
    pub members_count: u32,
    pub skills_count: u32,
    pub recipes_count: u32,
    pub extensions_count: u32,
}

/// Team list query parameters
#[derive(Debug, Clone, Deserialize)]
pub struct ListTeamsQuery {
    #[serde(default = "super::default_page")]
    pub page: u32,
    #[serde(default = "super::default_limit")]
    pub limit: u32,
    #[serde(default)]
    pub search: Option<String>,
    #[serde(default)]
    pub owner_id: Option<String>,
}
