//! Team member model

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Member role
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MemberRole {
    Owner,
    Admin,
    Member,
}

impl Default for MemberRole {
    fn default() -> Self {
        MemberRole::Member
    }
}

impl std::fmt::Display for MemberRole {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MemberRole::Owner => write!(f, "owner"),
            MemberRole::Admin => write!(f, "admin"),
            MemberRole::Member => write!(f, "member"),
        }
    }
}

impl std::str::FromStr for MemberRole {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "owner" => Ok(MemberRole::Owner),
            "admin" => Ok(MemberRole::Admin),
            "member" => Ok(MemberRole::Member),
            _ => Err(format!("Invalid member role: {}", s)),
        }
    }
}

/// Member status
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MemberStatus {
    Active,
    Invited,
    Blocked,
}

impl Default for MemberStatus {
    fn default() -> Self {
        MemberStatus::Active
    }
}

impl std::fmt::Display for MemberStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MemberStatus::Active => write!(f, "active"),
            MemberStatus::Invited => write!(f, "invited"),
            MemberStatus::Blocked => write!(f, "blocked"),
        }
    }
}

impl std::str::FromStr for MemberStatus {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "active" => Ok(MemberStatus::Active),
            "invited" => Ok(MemberStatus::Invited),
            "blocked" => Ok(MemberStatus::Blocked),
            _ => Err(format!("Invalid member status: {}", s)),
        }
    }
}

/// Team member entity
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamMember {
    pub id: String,
    pub team_id: String,
    pub user_id: String,
    pub display_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub endpoint_url: Option<String>,
    pub role: MemberRole,
    pub status: MemberStatus,
    #[serde(default)]
    pub permissions: MemberPermissions,
    pub joined_at: DateTime<Utc>,
}

/// Fine-grained member permissions
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemberPermissions {
    /// Can share resources
    #[serde(default = "default_true")]
    pub can_share: bool,
    /// Can install resources
    #[serde(default = "default_true")]
    pub can_install: bool,
    /// Can delete own resources
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

impl TeamMember {
    /// Create a new team member
    pub fn new(team_id: String, user_id: String, display_name: String, role: MemberRole) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            team_id,
            user_id,
            display_name,
            endpoint_url: None,
            role,
            status: MemberStatus::Active,
            permissions: MemberPermissions::default(),
            joined_at: Utc::now(),
        }
    }

    /// Check if member is owner
    pub fn is_owner(&self) -> bool {
        self.role == MemberRole::Owner
    }

    /// Check if member is admin or owner
    pub fn is_admin_or_owner(&self) -> bool {
        matches!(self.role, MemberRole::Owner | MemberRole::Admin)
    }

    /// Check if member can manage other members
    pub fn can_manage_members(&self) -> bool {
        self.is_admin_or_owner()
    }

    /// Check if member can delete the team
    pub fn can_delete_team(&self) -> bool {
        self.is_owner()
    }

    /// Check if member can update team settings
    pub fn can_update_team(&self) -> bool {
        self.is_admin_or_owner()
    }

    /// Check if member can share resources
    pub fn can_share_resources(&self) -> bool {
        self.status == MemberStatus::Active && self.permissions.can_share
    }

    /// Check if member can install resources
    pub fn can_install_resources(&self) -> bool {
        self.status == MemberStatus::Active && self.permissions.can_install
    }

    /// Check if member can delete a resource
    pub fn can_delete_resource(&self, resource_author_id: &str) -> bool {
        if self.status != MemberStatus::Active {
            return false;
        }
        // Owner and Admin can delete any resource
        if self.is_admin_or_owner() {
            return true;
        }
        // Members can only delete their own resources
        self.user_id == resource_author_id && self.permissions.can_delete_own
    }

    /// Check if member can review extensions
    pub fn can_review_extensions(&self) -> bool {
        self.status == MemberStatus::Active && self.is_admin_or_owner()
    }

    /// Check if member can change roles
    pub fn can_change_roles(&self) -> bool {
        self.is_owner()
    }

    /// Check if member can remove another member
    pub fn can_remove_member(&self, target: &TeamMember) -> bool {
        if self.status != MemberStatus::Active {
            return false;
        }
        // Cannot remove the owner
        if target.is_owner() {
            return false;
        }
        // Owner can remove anyone
        if self.is_owner() {
            return true;
        }
        // Admin can remove non-owner members
        self.role == MemberRole::Admin
    }
}

/// Request to add a member
#[derive(Debug, Clone, Deserialize)]
pub struct AddMemberRequest {
    pub user_id: String,
    pub display_name: String,
    #[serde(default)]
    pub role: Option<MemberRole>,
    #[serde(default)]
    pub endpoint_url: Option<String>,
}

/// Request to update a member
#[derive(Debug, Clone, Deserialize)]
pub struct UpdateMemberRequest {
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub role: Option<MemberRole>,
    #[serde(default)]
    pub status: Option<MemberStatus>,
    #[serde(default)]
    pub endpoint_url: Option<String>,
    #[serde(default)]
    pub permissions: Option<MemberPermissions>,
}

/// Member list query parameters
#[derive(Debug, Clone, Deserialize)]
pub struct ListMembersQuery {
    #[serde(default = "super::default_page")]
    pub page: u32,
    #[serde(default = "default_member_limit")]
    pub limit: u32,
    #[serde(default)]
    pub status: Option<MemberStatus>,
    #[serde(default)]
    pub role: Option<MemberRole>,
}

fn default_member_limit() -> u32 {
    50
}
