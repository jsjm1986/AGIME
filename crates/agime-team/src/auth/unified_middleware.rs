//! Unified Authentication Middleware
//! Supports multiple authentication methods: Session, API Key, Secret Key

use axum::{body::Body, http::Request};

/// Authentication method used
#[derive(Debug, Clone, PartialEq)]
pub enum AuthMethod {
    /// Session cookie authentication
    Session,
    /// API Key authentication (cloud servers)
    ApiKey,
    /// Secret Key authentication (local/LAN)
    SecretKey,
}

/// Authentication result
#[derive(Debug, Clone)]
pub struct AuthResult {
    pub user_id: String,
    pub method: AuthMethod,
}

/// Header names for authentication
pub const HEADER_API_KEY: &str = "X-API-Key";
pub const HEADER_SECRET_KEY: &str = "X-Secret-Key";

/// Extract API key from request headers
pub fn extract_api_key(request: &Request<Body>) -> Option<String> {
    request
        .headers()
        .get(HEADER_API_KEY)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
}

/// Extract secret key from request headers
pub fn extract_secret_key(request: &Request<Body>) -> Option<String> {
    request
        .headers()
        .get(HEADER_SECRET_KEY)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
}
