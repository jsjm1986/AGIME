//! Package service for handling skill ZIP packages
//!
//! Implements the Agent Skills open standard for skill packaging.
//! See: https://agentskills.io/specification

use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use sha2::{Digest, Sha256};
use std::io::{Cursor, Read, Write};
use zip::read::ZipArchive;
use zip::write::ZipWriter;
use zip::CompressionMethod;

use crate::error::{TeamError, TeamResult};
use crate::models::{SharedSkill, SkillFile, SkillManifest, SkillMetadata, SkillStorageType};

/// Maximum package size (10 MB)
pub const MAX_PACKAGE_SIZE: u64 = 10 * 1024 * 1024;

/// Maximum number of files in a package
pub const MAX_FILE_COUNT: usize = 100;

/// Maximum single file size (5 MB)
pub const MAX_FILE_SIZE: u64 = 5 * 1024 * 1024;

/// SKILL.md filename
pub const SKILL_MD_FILENAME: &str = "SKILL.md";

/// Parsed skill package from ZIP
#[derive(Debug, Clone)]
pub struct ParsedSkillPackage {
    /// Raw SKILL.md content
    pub skill_md: String,
    /// Parsed frontmatter
    pub frontmatter: SkillFrontmatter,
    /// Markdown body (after frontmatter)
    pub body: String,
    /// Package files
    pub files: Vec<SkillFile>,
    /// Generated manifest
    pub manifest: SkillManifest,
    /// Total package size
    pub total_size: u64,
}

/// SKILL.md frontmatter structure
#[derive(Debug, Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct SkillFrontmatter {
    /// Skill name (required)
    pub name: String,
    /// Skill description (required)
    pub description: String,
    /// License
    #[serde(default)]
    pub license: Option<String>,
    /// Extended metadata
    #[serde(default)]
    pub metadata: Option<FrontmatterMetadata>,
}

/// Metadata from frontmatter
#[derive(Debug, Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct FrontmatterMetadata {
    #[serde(default)]
    pub author: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub homepage: Option<String>,
    #[serde(default)]
    pub repository: Option<String>,
    #[serde(default)]
    pub keywords: Vec<String>,
}

/// Package service for ZIP operations
pub struct PackageService;

impl PackageService {
    /// Parse a ZIP file into a skill package
    pub fn parse_zip(data: &[u8]) -> TeamResult<ParsedSkillPackage> {
        // Check total size
        if data.len() as u64 > MAX_PACKAGE_SIZE {
            return Err(TeamError::Validation(format!(
                "Package size {} bytes exceeds maximum {} bytes",
                data.len(),
                MAX_PACKAGE_SIZE
            )));
        }

        let cursor = Cursor::new(data);
        let mut archive = ZipArchive::new(cursor)
            .map_err(|e| TeamError::Validation(format!("Invalid ZIP archive: {}", e)))?;

        // Check file count
        if archive.len() > MAX_FILE_COUNT {
            return Err(TeamError::Validation(format!(
                "Package contains {} files, maximum is {}",
                archive.len(),
                MAX_FILE_COUNT
            )));
        }

        let mut skill_md: Option<String> = None;
        let mut files: Vec<SkillFile> = Vec::new();
        let mut manifest = SkillManifest::default();
        let mut total_size: u64 = 0;

        for i in 0..archive.len() {
            let mut file = archive.by_index(i).map_err(|e| {
                TeamError::Validation(format!("Failed to read archive entry: {}", e))
            })?;

            let name = file.name().to_string();

            // Skip directories
            if name.ends_with('/') {
                continue;
            }

            // Validate path
            Self::validate_file_path(&name)?;

            // Check file size
            let size = file.size();
            if size > MAX_FILE_SIZE {
                return Err(TeamError::Validation(format!(
                    "File '{}' size {} bytes exceeds maximum {} bytes",
                    name, size, MAX_FILE_SIZE
                )));
            }
            total_size += size;

            // Read file content
            let mut content = Vec::new();
            file.read_to_end(&mut content).map_err(|e| {
                TeamError::Validation(format!("Failed to read file '{}': {}", name, e))
            })?;

            // Handle SKILL.md
            if name == SKILL_MD_FILENAME || name.ends_with("/SKILL.md") {
                // Use the root SKILL.md or first found
                if skill_md.is_none() || name == SKILL_MD_FILENAME {
                    skill_md = Some(String::from_utf8(content).map_err(|_| {
                        TeamError::Validation("SKILL.md is not valid UTF-8".to_string())
                    })?);
                }
                continue;
            }

            // Determine content type and encoding
            let content_type = mime_guess::from_path(&name)
                .first_or_octet_stream()
                .to_string();
            let is_binary = !Self::is_text_content_type(&content_type);

            let (encoded_content, is_base64) = if is_binary {
                (BASE64.encode(&content), true)
            } else {
                match String::from_utf8(content) {
                    Ok(text) => (text, false),
                    Err(e) => (BASE64.encode(e.into_bytes()), true),
                }
            };

            // Update manifest based on path
            if name.starts_with("scripts/") {
                manifest.scripts.push(name.clone());
            } else if name.starts_with("references/") {
                manifest.references.push(name.clone());
            } else if name.starts_with("assets/") {
                manifest.assets.push(name.clone());
            }

            files.push(SkillFile {
                path: name,
                content: encoded_content,
                content_type,
                size,
                is_binary: is_base64,
            });
        }

        // Validate SKILL.md exists
        let skill_md = skill_md.ok_or_else(|| {
            TeamError::Validation("Package must contain a SKILL.md file".to_string())
        })?;

        // Parse frontmatter
        let (frontmatter, body) = Self::parse_skill_md(&skill_md)?;

        Ok(ParsedSkillPackage {
            skill_md,
            frontmatter,
            body,
            files,
            manifest,
            total_size,
        })
    }

    /// Create a ZIP package from a skill
    pub fn create_zip(skill: &SharedSkill) -> TeamResult<Vec<u8>> {
        if skill.storage_type != SkillStorageType::Package {
            return Err(TeamError::Validation(
                "Cannot create ZIP from inline skill".to_string(),
            ));
        }

        let mut buffer = Vec::new();
        {
            let mut zip = ZipWriter::new(Cursor::new(&mut buffer));
            let options =
                zip::write::FileOptions::default().compression_method(CompressionMethod::Deflated);

            // Write SKILL.md
            if let Some(ref skill_md) = skill.skill_md {
                zip.start_file(SKILL_MD_FILENAME, options).map_err(|e| {
                    TeamError::Internal(format!("Failed to create SKILL.md in ZIP: {}", e))
                })?;
                zip.write_all(skill_md.as_bytes())
                    .map_err(|e| TeamError::Internal(format!("Failed to write SKILL.md: {}", e)))?;
            }

            // Write files
            if let Some(ref files) = skill.files {
                for file in files {
                    zip.start_file(&file.path, options).map_err(|e| {
                        TeamError::Internal(format!(
                            "Failed to create '{}' in ZIP: {}",
                            file.path, e
                        ))
                    })?;

                    let content = if file.is_binary {
                        BASE64.decode(&file.content).map_err(|e| {
                            TeamError::Internal(format!(
                                "Failed to decode base64 for '{}': {}",
                                file.path, e
                            ))
                        })?
                    } else {
                        file.content.as_bytes().to_vec()
                    };

                    zip.write_all(&content).map_err(|e| {
                        TeamError::Internal(format!("Failed to write '{}': {}", file.path, e))
                    })?;
                }
            }

            zip.finish()
                .map_err(|e| TeamError::Internal(format!("Failed to finalize ZIP: {}", e)))?;
        }

        Ok(buffer)
    }

    /// Parse SKILL.md content into frontmatter and body
    pub fn parse_skill_md(content: &str) -> TeamResult<(SkillFrontmatter, String)> {
        let content = content.trim();

        // Check for frontmatter delimiter
        if !content.starts_with("---") {
            return Err(TeamError::Validation(
                "SKILL.md must start with YAML frontmatter (---)".to_string(),
            ));
        }

        // Find end of frontmatter
        let rest = &content[3..];
        let end_pos = rest.find("\n---").ok_or_else(|| {
            TeamError::Validation("SKILL.md frontmatter is not properly closed".to_string())
        })?;

        let yaml_content = &rest[..end_pos].trim();
        let body = rest[end_pos + 4..].trim().to_string();

        // Parse YAML frontmatter
        let frontmatter: SkillFrontmatter = serde_yaml::from_str(yaml_content).map_err(|e| {
            TeamError::Validation(format!("Invalid SKILL.md frontmatter YAML: {}", e))
        })?;

        // Validate required fields
        if frontmatter.name.trim().is_empty() {
            return Err(TeamError::Validation(
                "SKILL.md frontmatter must contain 'name' field".to_string(),
            ));
        }
        if frontmatter.description.trim().is_empty() {
            return Err(TeamError::Validation(
                "SKILL.md frontmatter must contain 'description' field".to_string(),
            ));
        }

        Ok((frontmatter, body))
    }

    /// Generate SKILL.md content from frontmatter and body
    pub fn generate_skill_md(frontmatter: &SkillFrontmatter, body: &str) -> String {
        let yaml = serde_yaml::to_string(frontmatter).unwrap_or_default();
        format!("---\n{}---\n\n{}", yaml, body)
    }

    /// Calculate SHA-256 hash of data
    pub fn calculate_hash(data: &[u8]) -> String {
        let mut hasher = Sha256::new();
        hasher.update(data);
        let result = hasher.finalize();
        format!("{:x}", result)
    }

    /// Validate a file path for security
    fn validate_file_path(path: &str) -> TeamResult<()> {
        // Check for path traversal
        if path.contains("..") {
            return Err(TeamError::Validation(format!(
                "Invalid file path (path traversal): {}",
                path
            )));
        }

        // Check for absolute paths
        if path.starts_with('/') || path.starts_with('\\') {
            return Err(TeamError::Validation(format!(
                "File path must be relative: {}",
                path
            )));
        }

        // Check for Windows drive letters
        if path.len() >= 2 && path.chars().nth(1) == Some(':') {
            return Err(TeamError::Validation(format!(
                "File path must be relative: {}",
                path
            )));
        }

        // Check for null bytes
        if path.contains('\0') {
            return Err(TeamError::Validation(format!(
                "Invalid file path (null byte): {}",
                path
            )));
        }

        Ok(())
    }

    /// Check if a content type is text-based
    fn is_text_content_type(content_type: &str) -> bool {
        content_type.starts_with("text/")
            || content_type == "application/json"
            || content_type == "application/xml"
            || content_type == "application/javascript"
            || content_type == "application/x-yaml"
            || content_type == "application/toml"
            || content_type.ends_with("+json")
            || content_type.ends_with("+xml")
    }

    /// Convert parsed package to SharedSkill
    pub fn to_shared_skill(
        package: &ParsedSkillPackage,
        team_id: String,
        author_id: String,
    ) -> SharedSkill {
        let mut skill = SharedSkill::new_package(
            team_id,
            package.frontmatter.name.clone(),
            package.skill_md.clone(),
            author_id,
        );

        skill.description = Some(package.frontmatter.description.clone());
        skill.files = Some(package.files.clone());
        skill.manifest = Some(package.manifest.clone());
        skill.package_size = Some(package.total_size);

        // Extract metadata from frontmatter
        if let Some(ref fm_meta) = package.frontmatter.metadata {
            skill.metadata = Some(SkillMetadata {
                author: fm_meta.author.clone(),
                license: package.frontmatter.license.clone(),
                homepage: fm_meta.homepage.clone(),
                repository: fm_meta.repository.clone(),
                keywords: fm_meta.keywords.clone(),
                estimated_tokens: None,
            });

            // Use version from frontmatter if available
            if let Some(ref version) = fm_meta.version {
                skill.version = version.clone();
            }
        }

        skill
    }

    /// Validate a parsed package
    pub fn validate_package(package: &ParsedSkillPackage) -> TeamResult<()> {
        // Check total size
        if package.total_size > MAX_PACKAGE_SIZE {
            return Err(TeamError::Validation(format!(
                "Package size {} bytes exceeds maximum {} bytes",
                package.total_size, MAX_PACKAGE_SIZE
            )));
        }

        // Check file count
        if package.files.len() > MAX_FILE_COUNT {
            return Err(TeamError::Validation(format!(
                "Package contains {} files, maximum is {}",
                package.files.len(),
                MAX_FILE_COUNT
            )));
        }

        // Validate all file paths
        for file in &package.files {
            Self::validate_file_path(&file.path)?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_skill_md() {
        let content = r#"---
name: test-skill
description: A test skill
license: MIT
---

# Test Skill

This is the body.
"#;

        let (frontmatter, body) = PackageService::parse_skill_md(content).unwrap();
        assert_eq!(frontmatter.name, "test-skill");
        assert_eq!(frontmatter.description, "A test skill");
        assert_eq!(frontmatter.license, Some("MIT".to_string()));
        assert!(body.contains("# Test Skill"));
    }

    #[test]
    fn test_validate_file_path() {
        assert!(PackageService::validate_file_path("scripts/test.py").is_ok());
        assert!(PackageService::validate_file_path("../etc/passwd").is_err());
        assert!(PackageService::validate_file_path("/etc/passwd").is_err());
        assert!(PackageService::validate_file_path("C:\\Windows\\System32").is_err());
    }

    #[test]
    fn test_calculate_hash() {
        let data = b"hello world";
        let hash = PackageService::calculate_hash(data);
        // SHA-256 of "hello world"
        assert_eq!(
            hash,
            "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9"
        );
    }
}
