//! Common types and shared defaults for MongoDB models

use serde::Serialize;

/// Paginated response wrapper
#[derive(Debug, Clone, Serialize)]
pub struct PaginatedResponse<T: Serialize> {
    pub items: Vec<T>,
    pub total: u64,
    pub page: u64,
    pub limit: u64,
    #[serde(rename = "totalPages")]
    pub total_pages: u64,
}

impl<T: Serialize> PaginatedResponse<T> {
    pub fn new(items: Vec<T>, total: u64, page: u64, limit: u64) -> Self {
        let total_pages = if limit > 0 {
            (total + limit - 1) / limit
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

/// Resource name validator
pub struct ResourceNameValidator;

impl ResourceNameValidator {
    /// Validate a resource name
    pub fn validate(name: &str) -> Result<(), String> {
        if name.trim().is_empty() {
            return Err("Name cannot be empty".to_string());
        }
        if name.len() > 200 {
            return Err("Name must be 200 characters or less".to_string());
        }
        // Check for path traversal
        if name.contains("..") || name.contains('/') || name.contains('\\') {
            return Err("Name contains invalid characters".to_string());
        }
        // Check for control characters
        if name.chars().any(|c| c.is_control()) {
            return Err("Name contains control characters".to_string());
        }
        Ok(())
    }

    /// Validate kebab-case name per Claude Code SKILL.md spec.
    /// Only allows [a-z0-9-], max 64 chars, no leading/trailing hyphens.
    pub fn validate_kebab_case(name: &str) -> Result<(), String> {
        if name.is_empty() {
            return Err("Name cannot be empty".to_string());
        }
        if name.len() > 64 {
            return Err("Name must be 64 characters or less".to_string());
        }
        if name.starts_with('-') || name.ends_with('-') {
            return Err("Name cannot start or end with a hyphen".to_string());
        }
        if !name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-') {
            return Err(
                "Name must be kebab-case: only lowercase letters, digits, and hyphens [a-z0-9-]"
                    .to_string(),
            );
        }
        if name.contains("--") {
            return Err("Name cannot contain consecutive hyphens".to_string());
        }
        Ok(())
    }
}

// --- Shared serde default functions used across MongoDB models ---

pub fn default_visibility() -> String {
    "team".to_string()
}

pub fn default_protection_level() -> String {
    "team_installable".to_string()
}

pub fn default_version() -> String {
    "1.0.0".to_string()
}

/// Serde helper for `Option<DateTime<Utc>>` stored as BSON datetime.
/// Used by models that have optional datetime fields in MongoDB.
pub mod bson_datetime_option {
    use chrono::{DateTime, Utc};
    use serde::{self, Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S>(date: &Option<DateTime<Utc>>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match date {
            Some(dt) => {
                let bson_dt = bson::DateTime::from_chrono(*dt);
                Serialize::serialize(&bson_dt, serializer)
            }
            None => serializer.serialize_none(),
        }
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Option<DateTime<Utc>>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let opt: Option<bson::DateTime> = Option::deserialize(deserializer)?;
        Ok(opt.map(|dt| dt.to_chrono()))
    }
}

/// Increment a semver patch version string (e.g., "1.0.0" -> "1.0.1")
pub fn increment_version(version: &str) -> String {
    if let Some((prefix, patch)) = version.rsplit_once('.') {
        if let Ok(n) = patch.parse::<u64>() {
            return format!("{}.{}", prefix, n + 1);
        }
    }
    // Fallback: append .1
    format!("{}.1", version)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_increment_version() {
        assert_eq!(increment_version("1.0.0"), "1.0.1");
        assert_eq!(increment_version("1.2.3"), "1.2.4");
        assert_eq!(increment_version("2.0.99"), "2.0.100");
    }

    #[test]
    fn test_validate_name() {
        assert!(ResourceNameValidator::validate("valid name").is_ok());
        assert!(ResourceNameValidator::validate("").is_err());
        assert!(ResourceNameValidator::validate("../hack").is_err());
        assert!(ResourceNameValidator::validate("a/b").is_err());
    }

    #[test]
    fn test_validate_kebab_case() {
        assert!(ResourceNameValidator::validate_kebab_case("my-skill").is_ok());
        assert!(ResourceNameValidator::validate_kebab_case("skill123").is_ok());
        assert!(ResourceNameValidator::validate_kebab_case("a").is_ok());
        assert!(ResourceNameValidator::validate_kebab_case("code-review-2").is_ok());
        // Failures
        assert!(ResourceNameValidator::validate_kebab_case("").is_err());
        assert!(ResourceNameValidator::validate_kebab_case("My-Skill").is_err());
        assert!(ResourceNameValidator::validate_kebab_case("my_skill").is_err());
        assert!(ResourceNameValidator::validate_kebab_case("-leading").is_err());
        assert!(ResourceNameValidator::validate_kebab_case("trailing-").is_err());
        assert!(ResourceNameValidator::validate_kebab_case("double--hyphen").is_err());
        assert!(ResourceNameValidator::validate_kebab_case(&"a".repeat(65)).is_err());
    }
}
