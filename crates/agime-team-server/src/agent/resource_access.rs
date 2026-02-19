//! Runtime resource access policy for team agent auto mode.
//!
//! Keeps visibility/protection-level checks centralized so extension auto-load
//! and team skill on-demand tools use the same allowlist rules.

/// Returns true if a resource can be exposed to runtime tools.
///
/// Current policy:
/// - visibility must be `team` or `public`
/// - protection_level must be one of known values:
///   `none` (legacy), `public`, `team_installable`, `team_online_only`, `controlled`
pub(crate) fn is_runtime_resource_allowed(visibility: &str, protection_level: &str) -> bool {
    is_runtime_visibility_allowed(visibility)
        && is_runtime_protection_level_allowed(protection_level)
}

pub(crate) fn is_runtime_visibility_allowed(visibility: &str) -> bool {
    matches!(normalize(visibility).as_str(), "team" | "public")
}

pub(crate) fn is_runtime_protection_level_allowed(protection_level: &str) -> bool {
    matches!(
        normalize(protection_level).as_str(),
        // `none` is kept for Mongo legacy/default compatibility.
        "none" | "public" | "team_installable" | "team_online_only" | "controlled"
    )
}

fn normalize(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_visibility_allowlist() {
        assert!(is_runtime_visibility_allowed("team"));
        assert!(is_runtime_visibility_allowed("public"));
        assert!(is_runtime_visibility_allowed("TEAM"));
        assert!(!is_runtime_visibility_allowed("private"));
        assert!(!is_runtime_visibility_allowed(""));
    }

    #[test]
    fn runtime_protection_allowlist() {
        assert!(is_runtime_protection_level_allowed("none"));
        assert!(is_runtime_protection_level_allowed("public"));
        assert!(is_runtime_protection_level_allowed("team_installable"));
        assert!(is_runtime_protection_level_allowed("team_online_only"));
        assert!(is_runtime_protection_level_allowed("controlled"));
        assert!(!is_runtime_protection_level_allowed("internal_only"));
        assert!(!is_runtime_protection_level_allowed(""));
    }

    #[test]
    fn runtime_resource_combined_policy() {
        assert!(is_runtime_resource_allowed("team", "none"));
        assert!(is_runtime_resource_allowed("public", "team_online_only"));
        assert!(!is_runtime_resource_allowed("private", "public"));
        assert!(!is_runtime_resource_allowed("team", "unknown"));
    }
}
