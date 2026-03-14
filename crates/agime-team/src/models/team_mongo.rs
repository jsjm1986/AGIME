//! Team model for MongoDB

use chrono::{DateTime, Utc};
use mongodb::bson::oid::ObjectId;
use serde::{Deserialize, Serialize};

use super::common_mongo::bson_datetime_option;

/// Team member embedded document
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamMember {
    pub user_id: String,
    #[serde(default)]
    pub email: String,
    #[serde(default)]
    pub display_name: String,
    pub role: String, // owner, admin, member
    #[serde(default = "default_member_status")]
    pub status: String, // active, invited, blocked
    #[serde(default)]
    pub permissions: MemberPermissions,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub joined_at: DateTime<Utc>,
}

fn default_member_status() -> String {
    "active".to_string()
}

/// Member permissions
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemberPermissions {
    #[serde(default = "default_true")]
    pub can_share: bool,
    #[serde(default = "default_true")]
    pub can_install: bool,
    #[serde(default = "default_true")]
    pub can_delete_own: bool,
}

impl Default for MemberPermissions {
    fn default() -> Self {
        Self {
            can_share: true,
            can_install: true,
            can_delete_own: true,
        }
    }
}

fn default_true() -> bool {
    true
}

/// Team settings
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamSettings {
    /// Whether to require security review for extensions (default: true, matching SQLite model)
    #[serde(default = "default_true")]
    pub require_extension_review: bool,
    /// Whether members can invite other members (default: false, matching SQLite model)
    #[serde(default)]
    pub members_can_invite: bool,
    #[serde(default = "default_visibility_setting")]
    pub default_visibility: String,
    #[serde(default)]
    pub document_analysis: DocumentAnalysisSettings,
    #[serde(default)]
    pub shell_security: ShellSecuritySettings,
    #[serde(default)]
    pub avatar_governance: AvatarGovernanceSettings,
}

impl Default for TeamSettings {
    fn default() -> Self {
        Self {
            require_extension_review: true,
            members_can_invite: false,
            default_visibility: "team".to_string(),
            document_analysis: DocumentAnalysisSettings::default(),
            shell_security: ShellSecuritySettings::default(),
            avatar_governance: AvatarGovernanceSettings::default(),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ShellSecurityMode {
    Off,
    Warn,
    Block,
}

impl Default for ShellSecurityMode {
    fn default() -> Self {
        Self::Block
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShellSecuritySettings {
    #[serde(default)]
    pub mode: ShellSecurityMode,
}

impl Default for ShellSecuritySettings {
    fn default() -> Self {
        Self {
            mode: ShellSecurityMode::default(),
        }
    }
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

/// Document analysis configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentAnalysisSettings {
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Standalone LLM API URL (priority 1)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// "openai" | "anthropic"
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_format: Option<String>,
    /// Use specific agent's config (priority 2; None = auto-select first with key)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    #[serde(default = "default_min_file_size")]
    pub min_file_size: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_file_size: Option<i64>,
    #[serde(default = "default_skip_mime_prefixes")]
    pub skip_mime_prefixes: Vec<String>,
}

impl Default for DocumentAnalysisSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            api_url: None,
            api_key: None,
            model: None,
            api_format: None,
            agent_id: None,
            min_file_size: 10,
            max_file_size: None,
            skip_mime_prefixes: default_skip_mime_prefixes(),
        }
    }
}

fn default_min_file_size() -> i64 {
    10
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

fn default_skip_mime_prefixes() -> Vec<String> {
    vec![
        "image/".to_string(),
        "audio/".to_string(),
        "video/".to_string(),
    ]
}

fn default_visibility_setting() -> String {
    "team".to_string()
}

/// Team document
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Team {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub name: String,
    pub description: Option<String>,
    pub repository_url: Option<String>,
    pub owner_id: String,
    pub members: Vec<TeamMember>,
    #[serde(default)]
    pub settings: TeamSettings,
    #[serde(default)]
    pub is_deleted: bool,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
}

/// Create team request
#[derive(Debug, Clone, Deserialize)]
pub struct CreateTeamRequest {
    pub name: String,
    pub description: Option<String>,
}

/// Team summary for list views (matches frontend Team interface)
#[derive(Debug, Clone, Serialize)]
pub struct TeamSummary {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    #[serde(rename = "repositoryUrl")]
    pub repository_url: Option<String>,
    #[serde(rename = "ownerId")]
    pub owner_id: String,
    #[serde(rename = "createdAt")]
    pub created_at: String,
    #[serde(rename = "updatedAt")]
    pub updated_at: String,
}

/// Team detail response with stats (matches frontend TeamSummaryResponse)
#[derive(Debug, Clone, Serialize)]
pub struct TeamDetailResponse {
    pub team: TeamSummary,
    #[serde(rename = "membersCount")]
    pub members_count: usize,
    #[serde(rename = "skillsCount")]
    pub skills_count: usize,
    #[serde(rename = "recipesCount")]
    pub recipes_count: usize,
    #[serde(rename = "extensionsCount")]
    pub extensions_count: usize,
    #[serde(rename = "currentUserId")]
    pub current_user_id: String,
    #[serde(rename = "currentUserRole")]
    pub current_user_role: String,
}

impl From<Team> for TeamSummary {
    fn from(team: Team) -> Self {
        Self {
            id: team.id.map(|id| id.to_hex()).unwrap_or_default(),
            name: team.name,
            description: team.description,
            repository_url: team.repository_url,
            owner_id: team.owner_id,
            created_at: team.created_at.to_rfc3339(),
            updated_at: team.updated_at.to_rfc3339(),
        }
    }
}

/// Team invite document
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamInvite {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub team_id: ObjectId,
    pub code: String,
    pub role: String,
    pub created_by: String,
    #[serde(default, with = "bson_datetime_option")]
    pub expires_at: Option<DateTime<Utc>>,
    pub max_uses: Option<i32>,
    pub used_count: i32,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn team_settings_bson_contains_shell_security() {
        let mut settings = TeamSettings::default();
        settings.shell_security.mode = ShellSecurityMode::Warn;

        let bson = mongodb::bson::to_bson(&settings).unwrap();
        let doc = bson.as_document().unwrap();

        assert_eq!(
            doc.get_document("shell_security")
                .unwrap()
                .get_str("mode")
                .unwrap(),
            "warn"
        );
    }
}
