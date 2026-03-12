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
    /// Default governance policy for digital avatars
    #[serde(default)]
    pub avatar_governance: AvatarGovernanceSettings,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AvatarGovernanceSettings {
    #[serde(default = "default_avatar_auto_proposal_trigger_count")]
    pub auto_proposal_trigger_count: i64,
    #[serde(default = "default_avatar_manager_approval_mode")]
    pub manager_approval_mode: String,
    #[serde(default = "default_avatar_optimization_mode")]
    pub optimization_mode: String,
    #[serde(default = "default_avatar_low_risk_action")]
    pub low_risk_action: String,
    #[serde(default = "default_avatar_medium_risk_action")]
    pub medium_risk_action: String,
    #[serde(default = "default_avatar_high_risk_action")]
    pub high_risk_action: String,
    #[serde(default = "default_true")]
    pub auto_create_capability_requests: bool,
    #[serde(default = "default_true")]
    pub auto_create_optimization_tickets: bool,
    #[serde(default = "default_true")]
    pub require_human_for_publish: bool,
}

impl Default for AvatarGovernanceSettings {
    fn default() -> Self {
        Self {
            auto_proposal_trigger_count: default_avatar_auto_proposal_trigger_count(),
            manager_approval_mode: default_avatar_manager_approval_mode(),
            optimization_mode: default_avatar_optimization_mode(),
            low_risk_action: default_avatar_low_risk_action(),
            medium_risk_action: default_avatar_medium_risk_action(),
            high_risk_action: default_avatar_high_risk_action(),
            auto_create_capability_requests: true,
            auto_create_optimization_tickets: true,
            require_human_for_publish: true,
        }
    }
}

fn default_true() -> bool {
    true
}

fn default_visibility() -> String {
    "team".to_string()
}

fn default_avatar_auto_proposal_trigger_count() -> i64 {
    3
}

fn default_avatar_manager_approval_mode() -> String {
    "manager_decides".to_string()
}

fn default_avatar_optimization_mode() -> String {
    "dual_loop".to_string()
}

fn default_avatar_low_risk_action() -> String {
    "auto_execute".to_string()
}

fn default_avatar_medium_risk_action() -> String {
    "manager_review".to_string()
}

fn default_avatar_high_risk_action() -> String {
    "human_review".to_string()
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
