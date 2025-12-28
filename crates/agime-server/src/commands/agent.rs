use crate::configuration;
use crate::routes::web_ui;
use crate::state;
use agime_server::auth::check_token;
use anyhow::Result;
use axum::middleware;
use tower_http::cors::{Any, CorsLayer};
use tracing::info;

use agime::config::get_env_compat_or;
use agime::providers::pricing::initialize_pricing_cache;

/// Generate a random 64-character hex secret key (32 bytes of entropy)
fn generate_random_secret() -> String {
    let bytes: [u8; 32] = rand::random();
    hex::encode(bytes)
}

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

    let env_secret = get_env_compat_or("SERVER__SECRET_KEY", "");
    let secret_key = if env_secret.is_empty() || env_secret == "test" {
        let generated = generate_random_secret();
        tracing::info!(
            "AGIME_SERVER__SECRET_KEY not set - generated random secret for this session. \
             Set this environment variable for persistent authentication."
        );
        generated
    } else {
        env_secret
    };

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

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    info!("server shutdown complete");
    Ok(())
}
