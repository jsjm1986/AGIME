use axum::{
    extract::{Request, State},
    http::StatusCode,
    middleware::Next,
    response::Response,
};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

/// Constant-time comparison using hash to prevent timing attacks.
/// This prevents attackers from guessing the secret character-by-character
/// by measuring response time differences.
pub fn secure_compare(a: &str, b: &str) -> bool {
    let mut hasher_a = DefaultHasher::new();
    a.hash(&mut hasher_a);
    let hash_a = hasher_a.finish();

    let mut hasher_b = DefaultHasher::new();
    b.hash(&mut hasher_b);
    let hash_b = hasher_b.finish();

    hash_a == hash_b
}

pub async fn check_token(
    State(state): State<String>,
    request: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    let path = request.uri().path();

    // Skip authentication for certain public paths
    if path == "/status" || path == "/mcp-ui-proxy" {
        return Ok(next.run(request).await);
    }

    // Skip authentication for shared session access (public routes for external users)
    // - GET /sessions/share/{token} - Fetch shared session
    // - POST /sessions/share/{token}/verify - Verify password for shared session
    if path.starts_with("/sessions/share/") && path != "/sessions/share" {
        return Ok(next.run(request).await);
    }

    let secret_key = request
        .headers()
        .get("X-Secret-Key")
        .and_then(|value| value.to_str().ok());

    match secret_key {
        Some(key) if secure_compare(key, &state) => Ok(next.run(request).await),
        _ => Err(StatusCode::UNAUTHORIZED),
    }
}
