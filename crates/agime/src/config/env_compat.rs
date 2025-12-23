//! Environment Variable Compatibility Layer for AGIME/Goose Migration
//!
//! This module provides dual-prefix support for environment variables during
//! the brand migration from "Goose" to "AGIME". Variables can be accessed using
//! either AGIME_* prefix (preferred) or GOOSE_* prefix (legacy).
//!
//! # Priority Order
//! 1. AGIME_ prefix (new, preferred)
//! 2. GOOSE_ prefix (legacy, fallback)
//!
//! # Usage Examples
//! ```rust,ignore
//! use agime::config::env_compat::{get_env_compat, get_env_compat_or, env_compat_exists};
//!
//! // Get an optional environment variable
//! if let Some(value) = get_env_compat("PROVIDER") {
//!     // Checks AGIME_PROVIDER first, then GOOSE_PROVIDER
//! }
//!
//! // Get with default value
//! let mode = get_env_compat_or("MODE", "auto");
//!
//! // Check if variable exists under either prefix
//! if env_compat_exists("API_KEY") {
//!     // AGIME_API_KEY or GOOSE_API_KEY is set
//! }
//! ```

use std::env;
use std::str::FromStr;

/// The new AGIME prefix for environment variables (preferred)
pub const AGIME_PREFIX: &str = "AGIME_";

/// The legacy GOOSE prefix for environment variables (fallback)
pub const GOOSE_PREFIX: &str = "GOOSE_";

/// Get an environment variable with dual-prefix support.
///
/// Tries AGIME_ prefix first, then falls back to GOOSE_ prefix.
///
/// # Arguments
/// * `key` - The key without prefix (e.g., "PROVIDER" not "GOOSE_PROVIDER")
///
/// # Returns
/// * `Some(String)` if the variable is found under either prefix
/// * `None` if not found under any prefix
///
/// # Examples
/// ```rust,ignore
/// // With GOOSE_MODE=auto set:
/// assert_eq!(get_env_compat("MODE"), Some("auto".to_string()));
///
/// // With AGIME_MODE=manual set (takes priority):
/// assert_eq!(get_env_compat("MODE"), Some("manual".to_string()));
/// ```
pub fn get_env_compat(key: &str) -> Option<String> {
    // Tier 1: Try AGIME_ prefix (preferred)
    let agime_key = format!("{}{}", AGIME_PREFIX, key);
    if let Ok(value) = env::var(&agime_key) {
        return Some(value);
    }

    // Tier 2: Try GOOSE_ prefix (legacy fallback)
    let goose_key = format!("{}{}", GOOSE_PREFIX, key);
    if let Ok(value) = env::var(&goose_key) {
        return Some(value);
    }

    None
}

/// Get an environment variable with dual-prefix support, or return a default value.
///
/// # Arguments
/// * `key` - The key without prefix
/// * `default` - Default value if not found under either prefix
///
/// # Returns
/// The environment variable value, or the default if not found
pub fn get_env_compat_or(key: &str, default: &str) -> String {
    get_env_compat(key).unwrap_or_else(|| default.to_string())
}

/// Get and parse an environment variable with dual-prefix support.
///
/// # Arguments
/// * `key` - The key without prefix
///
/// # Returns
/// * `Ok(T)` if the variable is found and successfully parsed
/// * `Err` if not found or parsing fails
///
/// # Examples
/// ```rust,ignore
/// // With GOOSE_PORT=8080:
/// let port: u16 = get_env_compat_parsed("PORT")?;
/// assert_eq!(port, 8080);
/// ```
pub fn get_env_compat_parsed<T>(key: &str) -> Result<T, Box<dyn std::error::Error + Send + Sync>>
where
    T: FromStr,
    T::Err: std::error::Error + Send + Sync + 'static,
{
    let value = get_env_compat(key).ok_or_else(|| {
        format!(
            "Environment variable not found: {}_{} or {}_{}",
            AGIME_PREFIX, key, GOOSE_PREFIX, key
        )
    })?;
    value
        .parse::<T>()
        .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)
}

/// Get and parse an environment variable with dual-prefix support, or return a default.
///
/// # Arguments
/// * `key` - The key without prefix
/// * `default` - Default value if not found or parsing fails
pub fn get_env_compat_parsed_or<T>(key: &str, default: T) -> T
where
    T: FromStr,
    T::Err: std::error::Error + Send + Sync + 'static,
{
    get_env_compat_parsed(key).unwrap_or(default)
}

/// Check if an environment variable exists under either prefix.
///
/// # Arguments
/// * `key` - The key without prefix
///
/// # Returns
/// `true` if the variable exists under AGIME_ or GOOSE_ prefix
pub fn env_compat_exists(key: &str) -> bool {
    let agime_key = format!("{}{}", AGIME_PREFIX, key);
    let goose_key = format!("{}{}", GOOSE_PREFIX, key);
    env::var(&agime_key).is_ok() || env::var(&goose_key).is_ok()
}

/// Get the actual key name that was found (for debugging/logging).
///
/// # Arguments
/// * `key` - The key without prefix
///
/// # Returns
/// * `Some((full_key, value))` with the actual prefixed key name and value
/// * `None` if not found
pub fn get_env_compat_with_source(key: &str) -> Option<(String, String)> {
    let agime_key = format!("{}{}", AGIME_PREFIX, key);
    if let Ok(value) = env::var(&agime_key) {
        return Some((agime_key, value));
    }

    let goose_key = format!("{}{}", GOOSE_PREFIX, key);
    if let Ok(value) = env::var(&goose_key) {
        return Some((goose_key, value));
    }

    None
}

/// Convert a legacy GOOSE_ prefixed key to the new AGIME_ prefix.
///
/// # Arguments
/// * `key` - A key that may have GOOSE_ prefix
///
/// # Returns
/// The key with AGIME_ prefix if it had GOOSE_ prefix, otherwise unchanged
pub fn migrate_key_to_agime(key: &str) -> String {
    if key.starts_with(GOOSE_PREFIX) {
        format!("{}{}", AGIME_PREFIX, &key[GOOSE_PREFIX.len()..])
    } else {
        key.to_string()
    }
}

/// Check if a full key (with prefix) is using the legacy GOOSE_ prefix.
pub fn is_legacy_key(key: &str) -> bool {
    key.starts_with(GOOSE_PREFIX)
}

/// Check if a full key (with prefix) is using the new AGIME_ prefix.
pub fn is_agime_key(key: &str) -> bool {
    key.starts_with(AGIME_PREFIX)
}

/// Macro for convenient environment variable access with dual-prefix support.
///
/// # Usage
/// ```rust,ignore
/// // Get optional value
/// let value = env_compat!("PROVIDER");
///
/// // Get with default
/// let mode = env_compat!("MODE", "auto");
/// ```
#[macro_export]
macro_rules! env_compat {
    ($key:expr) => {
        $crate::config::env_compat::get_env_compat($key)
    };
    ($key:expr, $default:expr) => {
        $crate::config::env_compat::get_env_compat_or($key, $default)
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    // Note: These tests manipulate environment variables, so they should be run
    // with --test-threads=1 to avoid race conditions

    #[test]
    fn test_goose_prefix_fallback() {
        let test_key = "TEST_COMPAT_GOOSE_ONLY";
        let goose_key = format!("{}{}", GOOSE_PREFIX, test_key);

        // Clean up
        env::remove_var(&goose_key);
        env::remove_var(&format!("{}{}", AGIME_PREFIX, test_key));

        // Set only GOOSE_ version
        env::set_var(&goose_key, "goose_value");

        assert_eq!(get_env_compat(test_key), Some("goose_value".to_string()));

        // Clean up
        env::remove_var(&goose_key);
    }

    #[test]
    fn test_agime_prefix_priority() {
        let test_key = "TEST_COMPAT_BOTH";
        let agime_key = format!("{}{}", AGIME_PREFIX, test_key);
        let goose_key = format!("{}{}", GOOSE_PREFIX, test_key);

        // Clean up
        env::remove_var(&agime_key);
        env::remove_var(&goose_key);

        // Set both
        env::set_var(&goose_key, "goose_value");
        env::set_var(&agime_key, "agime_value");

        // AGIME should take priority
        assert_eq!(get_env_compat(test_key), Some("agime_value".to_string()));

        // Clean up
        env::remove_var(&agime_key);
        env::remove_var(&goose_key);
    }

    #[test]
    fn test_not_found() {
        let test_key = "TEST_COMPAT_NOT_EXISTS_12345";
        assert_eq!(get_env_compat(test_key), None);
    }

    #[test]
    fn test_default_value() {
        let test_key = "TEST_COMPAT_DEFAULT_12345";
        assert_eq!(get_env_compat_or(test_key, "default"), "default");
    }

    #[test]
    fn test_env_compat_exists() {
        let test_key = "TEST_COMPAT_EXISTS";
        let goose_key = format!("{}{}", GOOSE_PREFIX, test_key);

        env::remove_var(&goose_key);
        assert!(!env_compat_exists(test_key));

        env::set_var(&goose_key, "value");
        assert!(env_compat_exists(test_key));

        env::remove_var(&goose_key);
    }

    #[test]
    fn test_parsed_value() {
        let test_key = "TEST_COMPAT_PARSED";
        let goose_key = format!("{}{}", GOOSE_PREFIX, test_key);

        env::set_var(&goose_key, "42");
        let value: u32 = get_env_compat_parsed(test_key).unwrap();
        assert_eq!(value, 42);

        env::remove_var(&goose_key);
    }

    #[test]
    fn test_migrate_key() {
        assert_eq!(migrate_key_to_agime("GOOSE_PROVIDER"), "AGIME_PROVIDER");
        assert_eq!(migrate_key_to_agime("AGIME_PROVIDER"), "AGIME_PROVIDER");
        assert_eq!(migrate_key_to_agime("OTHER_KEY"), "OTHER_KEY");
    }

    #[test]
    fn test_is_legacy_key() {
        assert!(is_legacy_key("GOOSE_PROVIDER"));
        assert!(!is_legacy_key("AGIME_PROVIDER"));
        assert!(!is_legacy_key("OTHER_KEY"));
    }
}
