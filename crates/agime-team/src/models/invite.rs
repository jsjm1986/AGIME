//! Team invite model for invitation system

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;

/// Role for invited member
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum InviteRole {
    Member,
    Admin,
}

impl Default for InviteRole {
    fn default() -> Self {
        InviteRole::Member
    }
}

impl std::fmt::Display for InviteRole {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            InviteRole::Member => write!(f, "member"),
            InviteRole::Admin => write!(f, "admin"),
        }
    }
}

impl std::str::FromStr for InviteRole {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "member" => Ok(InviteRole::Member),
            "admin" => Ok(InviteRole::Admin),
            _ => Err(format!("Invalid invite role: {}", s)),
        }
    }
}

/// Invite expiration duration
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum InviteExpiration {
    #[serde(rename = "24h")]
    Hours24,
    #[serde(rename = "7d")]
    Days7,
    #[serde(rename = "30d")]
    Days30,
    #[serde(rename = "never")]
    Never,
}

impl Default for InviteExpiration {
    fn default() -> Self {
        InviteExpiration::Days7
    }
}

impl InviteExpiration {
    /// Get the duration in seconds
    pub fn to_seconds(&self) -> Option<i64> {
        match self {
            InviteExpiration::Hours24 => Some(24 * 60 * 60),
            InviteExpiration::Days7 => Some(7 * 24 * 60 * 60),
            InviteExpiration::Days30 => Some(30 * 24 * 60 * 60),
            InviteExpiration::Never => None,
        }
    }

    /// Calculate expiration datetime from now
    pub fn expires_at(&self) -> Option<DateTime<Utc>> {
        self.to_seconds().map(|secs| Utc::now() + chrono::Duration::seconds(secs))
    }
}

/// Team invite
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
#[serde(rename_all = "camelCase")]
pub struct TeamInvite {
    /// Unique invite ID (also the code)
    pub id: String,
    /// Team ID this invite belongs to
    pub team_id: String,
    /// Role to assign to invitee
    pub role: String,
    /// Expiration time (null = never)
    pub expires_at: Option<DateTime<Utc>>,
    /// Maximum uses (null = unlimited)
    pub max_uses: Option<i32>,
    /// Current use count
    pub used_count: i32,
    /// User ID who created this invite
    pub created_by: String,
    /// Creation timestamp
    pub created_at: DateTime<Utc>,
    /// Soft delete flag
    #[serde(skip_serializing)]
    pub deleted: bool,
}

impl TeamInvite {
    /// Check if this invite is still valid
    pub fn is_valid(&self) -> bool {
        // Not deleted
        if self.deleted {
            return false;
        }

        // Not expired
        if let Some(expires_at) = self.expires_at {
            if Utc::now() > expires_at {
                return false;
            }
        }

        // Not exceeded max uses
        if let Some(max_uses) = self.max_uses {
            if self.used_count >= max_uses {
                return false;
            }
        }

        true
    }

    /// Get the role as enum
    pub fn get_role(&self) -> InviteRole {
        self.role.parse().unwrap_or_default()
    }

    /// Generate full invite URL
    pub fn get_url(&self, base_url: &str) -> String {
        format!("{}/join/{}", base_url.trim_end_matches('/'), self.id)
    }
}

/// Request to create an invite
#[derive(Debug, Clone, Deserialize)]
pub struct CreateInviteRequest {
    /// Expiration duration
    #[serde(default)]
    pub expires_in: InviteExpiration,
    /// Maximum uses (null = unlimited)
    pub max_uses: Option<i32>,
    /// Role for invitee
    #[serde(default)]
    pub role: InviteRole,
}

/// Response for created invite
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateInviteResponse {
    /// Invite code
    pub code: String,
    /// Full invite URL
    pub url: String,
    /// Expiration time
    pub expires_at: Option<DateTime<Utc>>,
    /// Maximum uses
    pub max_uses: Option<i32>,
    /// Current use count
    pub used_count: i32,
}

/// Response for validating an invite
#[derive(Debug, Clone, Serialize)]
pub struct ValidateInviteResponse {
    /// Whether the invite is valid
    pub valid: bool,
    /// Team ID
    pub team_id: Option<String>,
    /// Team name
    pub team_name: Option<String>,
    /// Team description
    pub team_description: Option<String>,
    /// Role to be assigned
    pub role: Option<String>,
    /// Name of the user who created the invite
    pub inviter_name: Option<String>,
    /// Expiration time
    pub expires_at: Option<DateTime<Utc>>,
    /// Error message if invalid
    pub error: Option<String>,
}

/// Response for accepting an invite
#[derive(Debug, Clone, Serialize)]
pub struct AcceptInviteResponse {
    /// Whether the accept was successful
    pub success: bool,
    /// Team ID joined
    pub team_id: Option<String>,
    /// Member ID created
    pub member_id: Option<String>,
    /// Error message if failed
    pub error: Option<String>,
}
