//! API Key generation and management

use rand::Rng;

/// API Key prefix
const KEY_PREFIX: &str = "agime";

/// Generate a new API key
/// Format: agime_<user_prefix>_<random_32chars>
pub fn generate_api_key(user_id: &str) -> String {
    // Use char_indices to avoid panicking on non-ASCII user_id values
    let user_prefix: &str = if user_id.len() >= 6 {
        match user_id.char_indices().nth(6) {
            Some((byte_idx, _)) => &user_id[..byte_idx],
            None => user_id,
        }
    } else {
        user_id
    };

    let random_part: String = rand::thread_rng()
        .sample_iter(&rand::distributions::Alphanumeric)
        .take(32)
        .map(char::from)
        .collect();

    format!("{}_{}_{}", KEY_PREFIX, user_prefix, random_part)
}

/// Extract the key prefix (first 8 characters after agime_)
pub fn extract_key_prefix(api_key: &str) -> Option<String> {
    let parts: Vec<&str> = api_key.split('_').collect();
    if parts.len() >= 3 && parts[0] == KEY_PREFIX {
        // Return user_prefix + first 8 chars of random part (char-boundary safe)
        let random_part = parts[2];
        let end = random_part
            .char_indices()
            .nth(8)
            .map(|(i, _)| i)
            .unwrap_or(random_part.len());
        let prefix = format!("{}_{}", parts[1], &random_part[..end]);
        Some(prefix)
    } else {
        None
    }
}

/// Validate API key format
pub fn validate_key_format(api_key: &str) -> bool {
    let parts: Vec<&str> = api_key.split('_').collect();
    parts.len() >= 3 && parts[0] == KEY_PREFIX && parts[2].len() >= 32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_api_key() {
        let key = generate_api_key("user123456");
        assert!(key.starts_with("agime_user12_"));
        assert!(validate_key_format(&key));
    }

    #[test]
    fn test_extract_key_prefix() {
        let key = "agime_user12_a1b2c3d4e5f6g7h8i9j0k1l2m3n4o5p6";
        let prefix = extract_key_prefix(key);
        assert!(prefix.is_some());
        assert_eq!(prefix.unwrap(), "user12_a1b2c3d4");
    }

    #[test]
    fn test_validate_key_format() {
        assert!(validate_key_format(
            "agime_user12_a1b2c3d4e5f6g7h8i9j0k1l2m3n4o5p6"
        ));
        assert!(!validate_key_format("invalid_key"));
        assert!(!validate_key_format("agime_short_abc"));
    }
}
