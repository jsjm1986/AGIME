use crate::configuration;
use crate::routes::web_ui;
use crate::state;
use anyhow::Result;
use axum::middleware;
use goose_server::auth::check_token;
use tower_http::cors::{Any, CorsLayer};
use tracing::info;

use goose::config::get_env_compat_or;
use goose::providers::pricing::initialize_pricing_cache;

// Graceful shutdown signal
#[cfg(unix)]
async fn shutdown_signal() {
    use tokio::signal::unix::{signal, SignalKind};

    let sigint = match signal(SignalKind::interrupt()) {
        Ok(s) => Some(s),
        Err(e) => {
            tracing::warn!("Failed to install SIGINT handler: {}", e);
            None
        }
    };

    let sigterm = match signal(SignalKind::terminate()) {
        Ok(s) => Some(s),
        Err(e) => {
            tracing::warn!("Failed to install SIGTERM handler: {}", e);
            None
        }
    };

    match (sigint, sigterm) {
        (Some(mut sigint), Some(mut sigterm)) => {
            tokio::select! {
                _ = sigint.recv() => {},
                _ = sigterm.recv() => {},
            }
        }
        (Some(mut sigint), None) => {
            sigint.recv().await;
        }
        (None, Some(mut sigterm)) => {
            sigterm.recv().await;
        }
        (None, None) => {
            // Fallback to ctrl_c if signal handlers failed
            let _ = tokio::signal::ctrl_c().await;
        }
    }
}

#[cfg(not(unix))]
async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
}

pub async fn run() -> Result<()> {
    crate::logging::setup_logging(Some("goosed"))?;

    let settings = configuration::Settings::new()?;

    if let Err(e) = initialize_pricing_cache().await {
        tracing::warn!(
            "Failed to initialize pricing cache: {}. Pricing data may not be available.",
            e
        );
    }

    let secret_key = get_env_compat_or("SERVER__SECRET_KEY", "test");
    if secret_key == "test" || secret_key.is_empty() {
        tracing::warn!(
            "GOOSE_SERVER__SECRET_KEY not set or empty - using insecure default. \
             Set this environment variable in production!"
        );
    }

    let app_state = state::AppState::new().await?;

    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    // API routes with authentication middleware
    let api_routes = crate::routes::configure(app_state.clone(), secret_key.clone())
        .layer(middleware::from_fn_with_state(
            secret_key.clone(),
            check_token,
        ))
        .layer(cors.clone());

    // Web UI routes (static files, no auth needed for serving HTML/JS/CSS)
    // The web app will include X-Secret-Key in API requests
    let web_assets_dir = web_ui::get_web_assets_dir();
    let web_routes = web_ui::routes(web_assets_dir);

    // Combine routes: web UI routes first (no auth), then API routes (with auth)
    let app = web_routes.merge(api_routes);

    let listener = tokio::net::TcpListener::bind(settings.socket_addr()).await?;
    info!("listening on {}", listener.local_addr()?);

    let tunnel_manager = app_state.tunnel_manager.clone();
    tokio::spawn(async move {
        tunnel_manager.check_auto_start().await;
    });

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    info!("server shutdown complete");
    Ok(())
}
