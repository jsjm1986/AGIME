//! Data models for team collaboration

mod team;
mod member;
mod shared_skill;
mod shared_recipe;
mod shared_extension;
mod installed_resource;
mod invite;

pub use team::*;
pub use member::*;
pub use shared_skill::*;
pub use shared_recipe::*;
pub use shared_extension::*;
pub use installed_resource::*;
pub use invite::*;

// Re-export skill package types for convenience
pub use shared_skill::{
    SkillStorageType,
    SkillFile,
    SkillManifest,
    SkillMetadata,
    ProtectionLevel,
};

/// Resource type enumeration
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ResourceType {
    Skill,
    Recipe,
    Extension,
}

impl std::fmt::Display for ResourceType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ResourceType::Skill => write!(f, "skill"),
            ResourceType::Recipe => write!(f, "recipe"),
            ResourceType::Extension => write!(f, "extension"),
        }
    }
}

impl std::str::FromStr for ResourceType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "skill" => Ok(ResourceType::Skill),
            "recipe" => Ok(ResourceType::Recipe),
            "extension" => Ok(ResourceType::Extension),
            _ => Err(format!("Invalid resource type: {}", s)),
        }
    }
}

/// Visibility enumeration
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Visibility {
    Team,
    Public,
}

impl Default for Visibility {
    fn default() -> Self {
        Visibility::Team
    }
}

impl std::fmt::Display for Visibility {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Visibility::Team => write!(f, "team"),
            Visibility::Public => write!(f, "public"),
        }
    }
}

impl std::str::FromStr for Visibility {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "team" => Ok(Visibility::Team),
            "public" => Ok(Visibility::Public),
            _ => Err(format!("Invalid visibility: {}", s)),
        }
    }
}

/// Dependency definition
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Dependency {
    #[serde(rename = "type")]
    pub dep_type: ResourceType,
    pub name: String,
    /// Version requirement (e.g., ">=1.0.0", "*", "1.2.3")
    #[serde(default = "default_version_req")]
    pub version: String,
}

fn default_version_req() -> String {
    "*".to_string()
}

/// Pagination parameters
#[derive(Debug, Clone, serde::Deserialize)]
pub struct PaginationParams {
    #[serde(default = "default_page")]
    pub page: u32,
    #[serde(default = "default_limit")]
    pub limit: u32,
}

fn default_page() -> u32 {
    1
}

fn default_limit() -> u32 {
    20
}

impl Default for PaginationParams {
    fn default() -> Self {
        Self {
            page: default_page(),
            limit: default_limit(),
        }
    }
}

/// Paginated response
#[derive(Debug, Clone, serde::Serialize)]
pub struct PaginatedResponse<T> {
    pub items: Vec<T>,
    pub total: u64,
    pub page: u32,
    pub limit: u32,
    pub total_pages: u32,
}

impl<T> PaginatedResponse<T> {
    pub fn new(items: Vec<T>, total: u64, page: u32, limit: u32) -> Self {
        let total_pages = if limit > 0 {
            ((total as f64) / (limit as f64)).ceil() as u32
        } else {
            0
        };
        Self {
            items,
            total,
            page,
            limit,
            total_pages,
        }
    }
}
