//! Installed resource tracking model

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::{ResourceType, ProtectionLevel};

/// Installed resource entity
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstalledResource {
    pub id: String,
    pub resource_type: ResourceType,
    pub resource_id: String,
    pub team_id: String,
    pub resource_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub local_path: Option<String>,
    pub installed_version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latest_version: Option<String>,
    #[serde(default)]
    pub has_update: bool,
    pub installed_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_checked_at: Option<DateTime<Utc>>,

    // Authorization fields
    /// User who installed the resource
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
    /// Authorization token for accessing this resource
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authorization_token: Option<String>,
    /// When the authorization expires
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authorization_expires_at: Option<DateTime<Utc>>,
    /// Last time authorization was verified
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_verified_at: Option<DateTime<Utc>>,
    /// Protection level of the resource
    #[serde(default)]
    pub protection_level: ProtectionLevel,
}

impl InstalledResource {
    /// Create a new installed resource record
    pub fn new(
        resource_type: ResourceType,
        resource_id: String,
        team_id: String,
        resource_name: String,
        installed_version: String,
        local_path: Option<String>,
    ) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            resource_type,
            resource_id,
            team_id,
            resource_name,
            local_path,
            installed_version,
            latest_version: None,
            has_update: false,
            installed_at: Utc::now(),
            last_checked_at: None,
            user_id: None,
            authorization_token: None,
            authorization_expires_at: None,
            last_verified_at: None,
            protection_level: ProtectionLevel::TeamInstallable,
        }
    }

    /// Create a new installed resource with authorization
    pub fn new_with_auth(
        resource_type: ResourceType,
        resource_id: String,
        team_id: String,
        resource_name: String,
        installed_version: String,
        local_path: Option<String>,
        user_id: String,
        authorization_token: String,
        protection_level: ProtectionLevel,
    ) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4().to_string(),
            resource_type,
            resource_id,
            team_id,
            resource_name,
            local_path,
            installed_version,
            latest_version: None,
            has_update: false,
            installed_at: now,
            last_checked_at: None,
            user_id: Some(user_id),
            authorization_token: Some(authorization_token),
            authorization_expires_at: Some(now + Duration::hours(24)),
            last_verified_at: Some(now),
            protection_level,
        }
    }

    /// Check if there's an update available
    pub fn check_update(&mut self, latest_version: &str) -> bool {
        self.last_checked_at = Some(Utc::now());
        self.latest_version = Some(latest_version.to_string());

        // Compare versions using semver
        let has_update = match (
            semver::Version::parse(&self.installed_version),
            semver::Version::parse(latest_version),
        ) {
            (Ok(installed), Ok(latest)) => latest > installed,
            _ => self.installed_version != latest_version,
        };

        self.has_update = has_update;
        has_update
    }

    /// Update to new version
    pub fn update_version(&mut self, new_version: String, new_path: Option<String>) {
        self.installed_version = new_version.clone();
        self.latest_version = Some(new_version);
        self.has_update = false;
        if let Some(path) = new_path {
            self.local_path = Some(path);
        }
        self.last_checked_at = Some(Utc::now());
    }

    /// Check authorization status
    pub fn check_authorization(&self) -> AuthorizationStatus {
        // Public resources don't need authorization
        if self.protection_level == ProtectionLevel::Public {
            return AuthorizationStatus::NotRequired;
        }

        // Check if authorization exists
        let Some(expires_at) = self.authorization_expires_at else {
            return AuthorizationStatus::Missing;
        };

        let now = Utc::now();

        if now < expires_at {
            AuthorizationStatus::Valid
        } else {
            // Check grace period (72 hours from last verification)
            let grace_period = Duration::hours(72);
            if let Some(last_verified) = self.last_verified_at {
                if now < last_verified + grace_period {
                    return AuthorizationStatus::NeedsRefresh;
                }
            }
            AuthorizationStatus::Expired
        }
    }

    /// Refresh authorization with new token
    pub fn refresh_authorization(&mut self, new_token: String) {
        let now = Utc::now();
        self.authorization_token = Some(new_token);
        self.authorization_expires_at = Some(now + Duration::hours(24));
        self.last_verified_at = Some(now);
    }

    /// Check if authorization is valid or within grace period
    pub fn is_authorized(&self) -> bool {
        matches!(
            self.check_authorization(),
            AuthorizationStatus::Valid | AuthorizationStatus::NeedsRefresh | AuthorizationStatus::NotRequired
        )
    }
}

/// Authorization status for installed resources
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthorizationStatus {
    /// Authorization not required (public resource)
    NotRequired,
    /// Authorization is valid
    Valid,
    /// Authorization needs refresh (expired but within grace period)
    NeedsRefresh,
    /// Authorization has expired (beyond grace period)
    Expired,
    /// Authorization is missing
    Missing,
}

/// Install result
#[derive(Debug, Clone, Serialize)]
pub struct InstallResult {
    pub success: bool,
    pub resource_type: ResourceType,
    pub resource_id: String,
    pub installed_version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub local_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl InstallResult {
    pub fn success(
        resource_type: ResourceType,
        resource_id: String,
        installed_version: String,
        local_path: Option<String>,
    ) -> Self {
        Self {
            success: true,
            resource_type,
            resource_id,
            installed_version,
            local_path,
            error: None,
        }
    }

    pub fn failure(
        resource_type: ResourceType,
        resource_id: String,
        error: String,
    ) -> Self {
        Self {
            success: false,
            resource_type,
            resource_id,
            installed_version: String::new(),
            local_path: None,
            error: Some(error),
        }
    }
}

/// Uninstall result
#[derive(Debug, Clone, Serialize)]
pub struct UninstallResult {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Update info for a resource
#[derive(Debug, Clone, Serialize)]
pub struct UpdateInfo {
    pub resource_type: ResourceType,
    pub resource_id: String,
    pub resource_name: String,
    pub current_version: String,
    pub latest_version: String,
    pub has_update: bool,
}

/// Batch install request
#[derive(Debug, Clone, Deserialize)]
pub struct BatchInstallRequest {
    pub resources: Vec<ResourceRef>,
}

/// Resource reference for batch operations
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ResourceRef {
    #[serde(rename = "type")]
    pub resource_type: ResourceType,
    pub id: String,
}

/// Batch install result
#[derive(Debug, Clone, Serialize)]
pub struct BatchInstallResult {
    pub total: usize,
    pub successful: usize,
    pub failed: usize,
    pub results: Vec<InstallResult>,
}

impl BatchInstallResult {
    pub fn new(results: Vec<InstallResult>) -> Self {
        let total = results.len();
        let successful = results.iter().filter(|r| r.success).count();
        let failed = total - successful;
        Self {
            total,
            successful,
            failed,
            results,
        }
    }
}

/// Check updates request
#[derive(Debug, Clone, Deserialize)]
pub struct CheckUpdatesRequest {
    pub resource_ids: Vec<String>,
}

/// Check updates response
#[derive(Debug, Clone, Serialize)]
pub struct CheckUpdatesResponse {
    pub updates: Vec<UpdateInfo>,
}

/// Sync status
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncStatus {
    pub team_id: String,
    pub state: SyncState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_sync_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_commit_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
}

/// Sync state
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SyncState {
    Idle,
    Syncing,
    Error,
}

impl Default for SyncState {
    fn default() -> Self {
        SyncState::Idle
    }
}

impl std::fmt::Display for SyncState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SyncState::Idle => write!(f, "idle"),
            SyncState::Syncing => write!(f, "syncing"),
            SyncState::Error => write!(f, "error"),
        }
    }
}

impl std::str::FromStr for SyncState {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "idle" => Ok(SyncState::Idle),
            "syncing" => Ok(SyncState::Syncing),
            "error" => Ok(SyncState::Error),
            _ => Err(format!("Invalid sync state: {}", s)),
        }
    }
}
