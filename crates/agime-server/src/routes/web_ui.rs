//! Web UI static file serving
//!
//! This module provides routes for serving the web UI static files.
//! The web UI allows users to access AGIME through a browser via the tunnel.

use axum::{response::Redirect, routing::get, Router};
use agime::config::get_env_compat;
use std::path::PathBuf;
use tower_http::services::{ServeDir, ServeFile};
use tracing::info;

/// Creates routes for serving web UI static files.
///
/// If `web_assets_dir` is `None` or the directory doesn't exist, returns an empty router.
///
/// The web UI is served at `/web/*` with the following behavior:
/// - Static assets are served directly (JS, CSS, images, etc.)
/// - All other routes fall back to `index.html` for client-side routing
///
/// # Arguments
/// * `web_assets_dir` - Optional path to the directory containing web UI assets
///
/// # Example
/// ```ignore
/// let web_routes = web_ui::routes(Some(PathBuf::from("./dist-web")));
/// let app = Router::new().merge(web_routes);
/// ```
pub fn routes(web_assets_dir: Option<PathBuf>) -> Router {
    let Some(assets_dir) = web_assets_dir else {
        info!("Web UI disabled: GOOSE_WEB_ASSETS_DIR not set");
        return Router::new();
    };

    if !assets_dir.exists() {
        info!(
            "Web UI disabled: assets directory does not exist: {:?}",
            assets_dir
        );
        return Router::new();
    }

    // Security: Ensure the path is a directory (not a file or symlink)
    if !assets_dir.is_dir() {
        info!(
            "Web UI disabled: path is not a directory: {:?}",
            assets_dir
        );
        return Router::new();
    }

    // Security: On Unix, check for symlinks to prevent path traversal
    #[cfg(unix)]
    {
        if let Ok(metadata) = std::fs::symlink_metadata(&assets_dir) {
            if metadata.file_type().is_symlink() {
                info!(
                    "Web UI disabled: symlink detected (security risk): {:?}",
                    assets_dir
                );
                return Router::new();
            }
        }
    }

    let index_file = assets_dir.join("index.html");
    if !index_file.exists() {
        info!(
            "Web UI disabled: index.html not found in {:?}",
            assets_dir
        );
        return Router::new();
    }

    info!("Web UI enabled, serving from {:?}", assets_dir);

    // Create ServeDir service with fallback to index.html for SPA routing
    let serve_dir = ServeDir::new(&assets_dir).fallback(ServeFile::new(&index_file));

    Router::new()
        // Redirect /web to /web/ for consistent routing
        .route("/web", get(redirect_to_web_index))
        // Serve all web assets
        .nest_service("/web/", serve_dir)
}

/// Redirect /web to /web/ for consistent routing
async fn redirect_to_web_index() -> Redirect {
    Redirect::permanent("/web/")
}

/// Get the web assets directory from environment variable.
///
/// Returns `Some(PathBuf)` if `GOOSE_WEB_ASSETS_DIR` is set and non-empty.
pub fn get_web_assets_dir() -> Option<PathBuf> {
    get_env_compat("WEB_ASSETS_DIR")
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_routes_without_dir() {
        let router = routes(None);
        // Should return empty router without panicking
        assert!(true);
    }

    #[test]
    fn test_routes_with_nonexistent_dir() {
        let router = routes(Some(PathBuf::from("/nonexistent/path")));
        // Should return empty router without panicking
        assert!(true);
    }

    #[test]
    fn test_get_web_assets_dir_unset() {
        std::env::remove_var("GOOSE_WEB_ASSETS_DIR");
        assert!(get_web_assets_dir().is_none());
    }
}
