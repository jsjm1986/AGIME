//! Shared skill model
//!
//! Supports both inline (simple text) and package (Agent Skills standard) formats.
//! See: https://agentskills.io/specification

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::{Dependency, Visibility};

/// Skill storage type
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum SkillStorageType {
    /// Simple inline text content (backward compatible)
    #[default]
    Inline,
    /// Full package with SKILL.md and supporting files
    Package,
}

impl std::fmt::Display for SkillStorageType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SkillStorageType::Inline => write!(f, "inline"),
            SkillStorageType::Package => write!(f, "package"),
        }
    }
}

impl std::str::FromStr for SkillStorageType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "inline" => Ok(SkillStorageType::Inline),
            "package" => Ok(SkillStorageType::Package),
            _ => Err(format!("Invalid storage type: {}", s)),
        }
    }
}

/// Protection level for shared resources
/// Controls how resources can be accessed and installed
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ProtectionLevel {
    /// Public: freely installable and copyable
    Public,
    /// Team Installable: can be installed locally, requires authorization
    /// User leaving team will have the resource automatically cleaned up
    #[default]
    TeamInstallable,
    /// Team Online Only: cannot be installed locally, online access only
    /// Content never stored on user's machine
    TeamOnlineOnly,
    /// Controlled: online only with full audit logging and usage limits
    Controlled,
}

impl ProtectionLevel {
    /// Check if local installation is allowed
    pub fn allows_local_install(&self) -> bool {
        matches!(self, ProtectionLevel::Public | ProtectionLevel::TeamInstallable)
    }

    /// Check if authorization is required
    pub fn requires_authorization(&self) -> bool {
        !matches!(self, ProtectionLevel::Public)
    }

    /// Check if full audit logging is required
    pub fn requires_full_audit(&self) -> bool {
        matches!(self, ProtectionLevel::Controlled)
    }
}

impl std::fmt::Display for ProtectionLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProtectionLevel::Public => write!(f, "public"),
            ProtectionLevel::TeamInstallable => write!(f, "team_installable"),
            ProtectionLevel::TeamOnlineOnly => write!(f, "team_online_only"),
            ProtectionLevel::Controlled => write!(f, "controlled"),
        }
    }
}

impl std::str::FromStr for ProtectionLevel {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "public" => Ok(ProtectionLevel::Public),
            "team_installable" => Ok(ProtectionLevel::TeamInstallable),
            "team_online_only" => Ok(ProtectionLevel::TeamOnlineOnly),
            "controlled" => Ok(ProtectionLevel::Controlled),
            _ => Err(format!("Invalid protection level: {}", s)),
        }
    }
}

/// File in a skill package
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillFile {
    /// Relative path within the package (e.g., "scripts/lint.py")
    pub path: String,
    /// File content (text for text files, Base64 for binary)
    pub content: String,
    /// MIME content type
    pub content_type: String,
    /// File size in bytes
    pub size: u64,
    /// Whether content is Base64 encoded
    #[serde(default)]
    pub is_binary: bool,
}

/// Skill package manifest
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SkillManifest {
    /// Script files (e.g., ["scripts/lint.py", "scripts/test.sh"])
    #[serde(default)]
    pub scripts: Vec<String>,
    /// Reference documentation files
    #[serde(default)]
    pub references: Vec<String>,
    /// Asset files (templates, images, etc.)
    #[serde(default)]
    pub assets: Vec<String>,
}

/// Extended skill metadata (from SKILL.md frontmatter)
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SkillMetadata {
    /// Author name
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    /// License (e.g., "MIT", "Apache-2.0")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub license: Option<String>,
    /// Homepage URL
    #[serde(skip_serializing_if = "Option::is_none")]
    pub homepage: Option<String>,
    /// Repository URL
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repository: Option<String>,
    /// Keywords for discovery
    #[serde(default)]
    pub keywords: Vec<String>,
    /// Estimated token usage for SKILL.md
    #[serde(skip_serializing_if = "Option::is_none")]
    pub estimated_tokens: Option<u32>,
}

/// Shared skill entity
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SharedSkill {
    pub id: String,
    pub team_id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    // Storage type and content
    #[serde(default)]
    pub storage_type: SkillStorageType,

    /// Inline mode: simple text content (backward compatible)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,

    /// Package mode: SKILL.md content
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skill_md: Option<String>,

    /// Package mode: attached files
    #[serde(skip_serializing_if = "Option::is_none")]
    pub files: Option<Vec<SkillFile>>,

    /// Package mode: file manifest
    #[serde(skip_serializing_if = "Option::is_none")]
    pub manifest: Option<SkillManifest>,

    /// Package download URL (for large packages)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub package_url: Option<String>,

    /// Package hash for integrity verification (SHA-256)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub package_hash: Option<String>,

    /// Package size in bytes
    #[serde(skip_serializing_if = "Option::is_none")]
    pub package_size: Option<u64>,

    /// Extended metadata from SKILL.md frontmatter
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<SkillMetadata>,

    pub author_id: String,
    pub version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous_version_id: Option<String>,
    pub visibility: Visibility,
    /// Protection level controlling access and installation
    #[serde(default)]
    pub protection_level: ProtectionLevel,
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

impl SharedSkill {
    /// Create a new inline skill (backward compatible)
    pub fn new_inline(
        team_id: String,
        name: String,
        content: String,
        author_id: String,
    ) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4().to_string(),
            team_id,
            name,
            description: None,
            storage_type: SkillStorageType::Inline,
            content: Some(content),
            skill_md: None,
            files: None,
            manifest: None,
            package_url: None,
            package_hash: None,
            package_size: None,
            metadata: None,
            author_id,
            version: "1.0.0".to_string(),
            previous_version_id: None,
            visibility: Visibility::Team,
            protection_level: ProtectionLevel::TeamInstallable,
            tags: Vec::new(),
            dependencies: Vec::new(),
            use_count: 0,
            is_deleted: false,
            created_at: now,
            updated_at: now,
        }
    }

    /// Create a new package skill
    pub fn new_package(
        team_id: String,
        name: String,
        skill_md: String,
        author_id: String,
    ) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4().to_string(),
            team_id,
            name,
            description: None,
            storage_type: SkillStorageType::Package,
            content: None,
            skill_md: Some(skill_md),
            files: Some(Vec::new()),
            manifest: Some(SkillManifest::default()),
            package_url: None,
            package_hash: None,
            package_size: None,
            metadata: None,
            author_id,
            version: "1.0.0".to_string(),
            previous_version_id: None,
            visibility: Visibility::Team,
            protection_level: ProtectionLevel::TeamInstallable,
            tags: Vec::new(),
            dependencies: Vec::new(),
            use_count: 0,
            is_deleted: false,
            created_at: now,
            updated_at: now,
        }
    }

    /// Backward compatible constructor (creates inline skill)
    pub fn new(
        team_id: String,
        name: String,
        content: String,
        author_id: String,
    ) -> Self {
        Self::new_inline(team_id, name, content, author_id)
    }

    /// Get the effective content (SKILL.md for package, content for inline)
    pub fn get_effective_content(&self) -> Option<&str> {
        match self.storage_type {
            SkillStorageType::Package => self.skill_md.as_deref(),
            SkillStorageType::Inline => self.content.as_deref(),
        }
    }

    /// Check if this is a package skill
    pub fn is_package(&self) -> bool {
        self.storage_type == SkillStorageType::Package
    }

    /// Add a file to the package
    pub fn add_file(&mut self, file: SkillFile) {
        if self.files.is_none() {
            self.files = Some(Vec::new());
        }
        if self.manifest.is_none() {
            self.manifest = Some(SkillManifest::default());
        }

        if let Some(ref mut files) = self.files {
            // Update manifest based on file path
            if let Some(ref mut manifest) = self.manifest {
                if file.path.starts_with("scripts/") {
                    if !manifest.scripts.contains(&file.path) {
                        manifest.scripts.push(file.path.clone());
                    }
                } else if file.path.starts_with("references/") {
                    if !manifest.references.contains(&file.path) {
                        manifest.references.push(file.path.clone());
                    }
                } else if file.path.starts_with("assets/") {
                    if !manifest.assets.contains(&file.path) {
                        manifest.assets.push(file.path.clone());
                    }
                }
            }
            files.push(file);
        }
    }

    /// Remove a file from the package by path
    pub fn remove_file(&mut self, path: &str) -> bool {
        let mut removed = false;
        if let Some(ref mut files) = self.files {
            let len_before = files.len();
            files.retain(|f| f.path != path);
            removed = files.len() < len_before;
        }
        if removed {
            if let Some(ref mut manifest) = self.manifest {
                manifest.scripts.retain(|p| p != path);
                manifest.references.retain(|p| p != path);
                manifest.assets.retain(|p| p != path);
            }
        }
        removed
    }

    /// Get a file by path
    pub fn get_file(&self, path: &str) -> Option<&SkillFile> {
        self.files.as_ref()?.iter().find(|f| f.path == path)
    }

    /// Calculate total package size
    pub fn calculate_package_size(&mut self) {
        let mut total_size: u64 = 0;

        // Add SKILL.md size
        if let Some(ref skill_md) = self.skill_md {
            total_size += skill_md.len() as u64;
        }

        // Add files size
        if let Some(ref files) = self.files {
            for file in files {
                total_size += file.size;
            }
        }

        self.package_size = Some(total_size);
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

    /// Convert inline skill to package format
    pub fn convert_to_package(&mut self) {
        if self.storage_type == SkillStorageType::Inline {
            // Move content to skill_md with basic frontmatter
            if let Some(content) = self.content.take() {
                let skill_md = format!(
                    "---\nname: {}\ndescription: {}\n---\n\n{}",
                    self.name,
                    self.description.as_deref().unwrap_or(""),
                    content
                );
                self.skill_md = Some(skill_md);
            }
            self.storage_type = SkillStorageType::Package;
            self.files = Some(Vec::new());
            self.manifest = Some(SkillManifest::default());
            self.updated_at = Utc::now();
        }
    }
}

/// Request to share a skill
#[derive(Debug, Clone, Deserialize)]
pub struct ShareSkillRequest {
    pub team_id: String,
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,

    // Storage type (defaults to inline for backward compatibility)
    #[serde(default)]
    pub storage_type: Option<SkillStorageType>,

    // Inline mode: simple text content (backward compatible)
    #[serde(default)]
    pub content: Option<String>,

    // Package mode: SKILL.md content
    #[serde(default)]
    pub skill_md: Option<String>,

    // Package mode: attached files
    #[serde(default)]
    pub files: Option<Vec<SkillFile>>,

    // Extended metadata
    #[serde(default)]
    pub metadata: Option<SkillMetadata>,

    #[serde(default)]
    pub tags: Option<Vec<String>>,
    #[serde(default)]
    pub dependencies: Option<Vec<Dependency>>,
    #[serde(default)]
    pub visibility: Option<Visibility>,
    /// Protection level (defaults to TeamInstallable)
    #[serde(default)]
    pub protection_level: Option<ProtectionLevel>,
}

impl ShareSkillRequest {
    /// Determine the effective storage type from request
    pub fn effective_storage_type(&self) -> SkillStorageType {
        if let Some(st) = self.storage_type {
            return st;
        }
        // Auto-detect: if skill_md is provided, it's a package
        if self.skill_md.is_some() || self.files.is_some() {
            SkillStorageType::Package
        } else {
            SkillStorageType::Inline
        }
    }

    /// Validate the request
    pub fn validate(&self) -> Result<(), String> {
        if self.name.trim().is_empty() {
            return Err("Name is required".to_string());
        }

        let storage_type = self.effective_storage_type();
        match storage_type {
            SkillStorageType::Inline => {
                if self.content.as_ref().map(|c| c.trim().is_empty()).unwrap_or(true) {
                    return Err("Content is required for inline skills".to_string());
                }
            }
            SkillStorageType::Package => {
                if self.skill_md.as_ref().map(|c| c.trim().is_empty()).unwrap_or(true) {
                    return Err("SKILL.md content is required for package skills".to_string());
                }
            }
        }

        // Validate file paths
        if let Some(ref files) = self.files {
            for file in files {
                if file.path.contains("..") {
                    return Err(format!("Invalid file path (path traversal): {}", file.path));
                }
                if file.path.starts_with('/') || file.path.starts_with('\\') {
                    return Err(format!("File path must be relative: {}", file.path));
                }
            }
        }

        Ok(())
    }
}

/// Request to update a skill
#[derive(Debug, Clone, Deserialize)]
pub struct UpdateSkillRequest {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,

    // Inline mode: content update
    #[serde(default)]
    pub content: Option<String>,

    // Package mode: SKILL.md content update
    #[serde(default)]
    pub skill_md: Option<String>,

    // Package mode: files to add/update
    #[serde(default)]
    pub files: Option<Vec<SkillFile>>,

    // Package mode: file paths to remove
    #[serde(default)]
    pub remove_files: Option<Vec<String>>,

    // Extended metadata update
    #[serde(default)]
    pub metadata: Option<SkillMetadata>,

    #[serde(default)]
    pub tags: Option<Vec<String>>,
    #[serde(default)]
    pub dependencies: Option<Vec<Dependency>>,
    #[serde(default)]
    pub visibility: Option<Visibility>,
    /// Protection level update
    #[serde(default)]
    pub protection_level: Option<ProtectionLevel>,

    /// Convert from inline to package format
    #[serde(default)]
    pub convert_to_package: Option<bool>,
}

/// Skill list query parameters
#[derive(Debug, Clone, Deserialize)]
pub struct ListSkillsQuery {
    #[serde(default = "super::default_page")]
    pub page: u32,
    #[serde(default = "super::default_limit")]
    pub limit: u32,
    #[serde(default)]
    pub team_id: Option<String>,
    #[serde(default)]
    pub search: Option<String>,
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

/// Skill with additional info
#[derive(Debug, Clone, Serialize)]
pub struct SkillWithInfo {
    #[serde(flatten)]
    pub skill: SharedSkill,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub install_status: Option<InstallStatus>,
}

/// Installation status
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstallStatus {
    pub installed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub installed_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub has_update: Option<bool>,
}

/// Skill version info
#[derive(Debug, Clone, Serialize)]
pub struct SkillVersion {
    pub id: String,
    pub version: String,
    pub created_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}
