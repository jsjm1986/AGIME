//! Shared recipe model

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::{Dependency, InstallStatus, ProtectionLevel, Visibility};

/// Shared recipe entity
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SharedRecipe {
    pub id: String,
    pub team_id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub content_yaml: String,
    pub author_id: String,
    pub version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous_version_id: Option<String>,
    pub visibility: Visibility,
    #[serde(default)]
    pub protection_level: ProtectionLevel,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub dependencies: Vec<Dependency>,
    #[serde(default)]
    pub use_count: u32,
    #[serde(default)]
    pub is_deleted: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl SharedRecipe {
    /// Create a new shared recipe
    pub fn new(
        team_id: String,
        name: String,
        content_yaml: String,
        author_id: String,
    ) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4().to_string(),
            team_id,
            name,
            description: None,
            content_yaml,
            author_id,
            version: "1.0.0".to_string(),
            previous_version_id: None,
            visibility: Visibility::Team,
            protection_level: ProtectionLevel::TeamInstallable,
            category: None,
            tags: Vec::new(),
            dependencies: Vec::new(),
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
        }
    }
}

/// Request to share a recipe
#[derive(Debug, Clone, Deserialize)]
pub struct ShareRecipeRequest {
    pub team_id: String,
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    pub content_yaml: String,
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default)]
    pub tags: Option<Vec<String>>,
    #[serde(default)]
    pub dependencies: Option<Vec<Dependency>>,
    #[serde(default)]
    pub visibility: Option<Visibility>,
    /// BIZ-1 FIX: Add protection_level field
    #[serde(default)]
    pub protection_level: Option<ProtectionLevel>,
}

/// Request to update a recipe
#[derive(Debug, Clone, Deserialize)]
pub struct UpdateRecipeRequest {
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub content_yaml: Option<String>,
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default)]
    pub tags: Option<Vec<String>>,
    #[serde(default)]
    pub dependencies: Option<Vec<Dependency>>,
    #[serde(default)]
    pub visibility: Option<Visibility>,
    /// BIZ-1 FIX: Add protection_level field
    #[serde(default)]
    pub protection_level: Option<ProtectionLevel>,
}

/// Recipe list query parameters
#[derive(Debug, Clone, Deserialize)]
pub struct ListRecipesQuery {
    #[serde(default = "super::default_page")]
    pub page: u32,
    #[serde(default = "super::default_limit")]
    pub limit: u32,
    #[serde(default)]
    pub team_id: Option<String>,
    #[serde(default)]
    pub search: Option<String>,
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default)]
    pub tags: Option<Vec<String>>,
    #[serde(default)]
    pub author_id: Option<String>,
    #[serde(default = "default_sort")]
    pub sort: String,
}

fn default_sort() -> String {
    "updated_at".to_string()
}

/// Recipe with additional info
#[derive(Debug, Clone, Serialize)]
pub struct RecipeWithInfo {
    #[serde(flatten)]
    pub recipe: SharedRecipe,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub install_status: Option<InstallStatus>,
}

/// Recipe version info
#[derive(Debug, Clone, Serialize)]
pub struct RecipeVersion {
    pub id: String,
    pub version: String,
    pub created_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// Recipe categories
pub mod categories {
    pub const AUTOMATION: &str = "automation";
    pub const DATA_PROCESSING: &str = "data-processing";
    pub const DEVELOPMENT: &str = "development";
    pub const DOCUMENTATION: &str = "documentation";
    pub const TESTING: &str = "testing";
    pub const DEPLOYMENT: &str = "deployment";
    pub const OTHER: &str = "other";

    pub fn all() -> Vec<&'static str> {
        vec![
            AUTOMATION,
            DATA_PROCESSING,
            DEVELOPMENT,
            DOCUMENTATION,
            TESTING,
            DEPLOYMENT,
            OTHER,
        ]
    }
}
