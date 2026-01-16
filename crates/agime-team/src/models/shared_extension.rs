//! Shared extension model

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::{InstallStatus, ProtectionLevel, Visibility};

/// Extension type
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExtensionType {
    Stdio,
    Sse,
    Builtin,
}

impl Default for ExtensionType {
    fn default() -> Self {
        ExtensionType::Stdio
    }
}

impl std::fmt::Display for ExtensionType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExtensionType::Stdio => write!(f, "stdio"),
            ExtensionType::Sse => write!(f, "sse"),
            ExtensionType::Builtin => write!(f, "builtin"),
        }
    }
}

impl std::str::FromStr for ExtensionType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "stdio" => Ok(ExtensionType::Stdio),
            "sse" => Ok(ExtensionType::Sse),
            "builtin" => Ok(ExtensionType::Builtin),
            _ => Err(format!("Invalid extension type: {}", s)),
        }
    }
}

/// Extension configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtensionConfig {
    /// Command to run (for stdio type)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cmd: Option<String>,
    /// Arguments for the command
    #[serde(default)]
    pub args: Vec<String>,
    /// Environment variables
    #[serde(default)]
    pub env: std::collections::HashMap<String, String>,
    /// URL for SSE type
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// Working directory
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    /// Whether to bundle the extension
    #[serde(default)]
    pub bundled: bool,
    /// Additional metadata
    #[serde(default)]
    pub metadata: serde_json::Value,
}

impl Default for ExtensionConfig {
    fn default() -> Self {
        Self {
            cmd: None,
            args: Vec::new(),
            env: std::collections::HashMap::new(),
            url: None,
            cwd: None,
            bundled: false,
            metadata: serde_json::Value::Null,
        }
    }
}

/// Shared extension entity
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SharedExtension {
    pub id: String,
    pub team_id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub extension_type: ExtensionType,
    pub config: ExtensionConfig,
    pub author_id: String,
    pub version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous_version_id: Option<String>,
    pub visibility: Visibility,
    #[serde(default)]
    pub protection_level: ProtectionLevel,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub security_reviewed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub security_notes: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reviewed_by: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reviewed_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub use_count: u32,
    #[serde(default)]
    pub is_deleted: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl SharedExtension {
    /// Create a new shared extension
    pub fn new(
        team_id: String,
        name: String,
        extension_type: ExtensionType,
        config: ExtensionConfig,
        author_id: String,
    ) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4().to_string(),
            team_id,
            name,
            description: None,
            extension_type,
            config,
            author_id,
            version: "1.0.0".to_string(),
            previous_version_id: None,
            visibility: Visibility::Team,
            protection_level: ProtectionLevel::TeamInstallable,
            tags: Vec::new(),
            security_reviewed: false,
            security_notes: None,
            reviewed_by: None,
            reviewed_at: None,
            use_count: 0,
            is_deleted: false,
            created_at: now,
            updated_at: now,
        }
    }

    /// Increment version (patch version)
    pub fn increment_version(&mut self) {
        if let Ok(mut semver) = semver::Version::parse(&self.version) {
            semver.patch += 1;
            self.previous_version_id = Some(self.id.clone());
            self.id = Uuid::new_v4().to_string();
            self.version = semver.to_string();
            self.updated_at = Utc::now();
            // Reset security review on update
            self.security_reviewed = false;
            self.security_notes = None;
            self.reviewed_by = None;
            self.reviewed_at = None;
        }
    }

    /// Mark as security reviewed
    pub fn mark_reviewed(&mut self, reviewer_id: String, notes: Option<String>) {
        self.security_reviewed = true;
        self.reviewed_by = Some(reviewer_id);
        self.reviewed_at = Some(Utc::now());
        self.security_notes = notes;
    }
}

/// Request to share an extension
#[derive(Debug, Clone, Deserialize)]
pub struct ShareExtensionRequest {
    pub team_id: String,
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    pub extension_type: ExtensionType,
    pub config: ExtensionConfig,
    #[serde(default)]
    pub tags: Option<Vec<String>>,
    #[serde(default)]
    pub visibility: Option<Visibility>,
    /// BIZ-2 FIX: Add protection_level field
    #[serde(default)]
    pub protection_level: Option<ProtectionLevel>,
}

/// Request to update an extension
#[derive(Debug, Clone, Deserialize)]
pub struct UpdateExtensionRequest {
    /// New name for the extension (optional)
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub config: Option<ExtensionConfig>,
    #[serde(default)]
    pub tags: Option<Vec<String>>,
    #[serde(default)]
    pub visibility: Option<Visibility>,
    /// BIZ-2 FIX: Add protection_level field
    #[serde(default)]
    pub protection_level: Option<ProtectionLevel>,
}

/// Request to review an extension
#[derive(Debug, Clone, Deserialize)]
pub struct ReviewExtensionRequest {
    pub approved: bool,
    #[serde(default)]
    pub notes: Option<String>,
}

/// Extension list query parameters
#[derive(Debug, Clone, Deserialize)]
pub struct ListExtensionsQuery {
    #[serde(default = "super::default_page")]
    pub page: u32,
    #[serde(default = "super::default_limit")]
    pub limit: u32,
    #[serde(default)]
    pub team_id: Option<String>,
    #[serde(default)]
    pub search: Option<String>,
    #[serde(default)]
    pub extension_type: Option<ExtensionType>,
    #[serde(default)]
    pub tags: Option<Vec<String>>,
    #[serde(default)]
    pub author_id: Option<String>,
    #[serde(default)]
    pub reviewed_only: Option<bool>,
    #[serde(default = "default_sort")]
    pub sort: String,
}

fn default_sort() -> String {
    "updated_at".to_string()
}

/// Extension with additional info
#[derive(Debug, Clone, Serialize)]
pub struct ExtensionWithInfo {
    #[serde(flatten)]
    pub extension: SharedExtension,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reviewer_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub install_status: Option<InstallStatus>,
}

/// Extension version info
#[derive(Debug, Clone, Serialize)]
pub struct ExtensionVersion {
    pub id: String,
    pub version: String,
    pub created_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub security_reviewed: bool,
}
