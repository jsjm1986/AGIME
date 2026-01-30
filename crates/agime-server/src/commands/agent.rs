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

/// Initialize team database (only available with 'team' feature)
#[cfg(feature = "team")]
async fn init_team_database() -> Result<std::sync::Arc<sqlx::SqlitePool>> {
    use sqlx::sqlite::SqlitePoolOptions;
    use std::sync::Arc;

    // Get team database path
    let team_db_path = dirs::data_local_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("agime")
        .join("team.db");

    // Ensure parent directory exists
    if let Some(parent) = team_db_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    info!("Initializing team database at {:?}", team_db_path);

    // Connect to database (create if not exists)
    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect(&format!("sqlite:{}?mode=rwc", team_db_path.display()))
        .await?;

    // Run migrations
    agime_team::migrations::run_migration(&pool).await?;

    info!("Team database initialized successfully");

    Ok(Arc::new(pool))
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
    #[cfg(feature = "team")]
    let api_routes = {
        // Set environment variables for team_extension to use
        // This allows team_extension (running as MCP tool) to connect to this server
        let api_url = format!("http://{}:{}", settings.host, settings.port);
        std::env::set_var("AGIME_TEAM_API_URL", &api_url);
        std::env::set_var("AGIME_API_HOST", &api_url);

        // Initialize team database
        let team_pool = match init_team_database().await {
            Ok(pool) => Some(pool),
            Err(e) => {
                tracing::warn!("Failed to initialize team database: {}. Team features will be disabled.", e);
                None
            }
        };

        if let Some(pool) = team_pool {
            // Get base path for installing team resources
            // Default to user's local data directory / agime / team-resources
            let base_path = dirs::data_local_dir()
                .unwrap_or_else(|| std::path::PathBuf::from("."))
                .join("agime")
                .join("team-resources");

            // Ensure the base path exists
            if let Err(e) = std::fs::create_dir_all(&base_path) {
                tracing::warn!("Failed to create team resources directory: {}. Using current directory.", e);
            }

            info!("Team resources base path: {:?}", base_path);

            let team_config = crate::routes::TeamRoutesConfig {
                pool,
                user_id: get_env_compat_or("USER_ID", "local-user").to_string(),
                base_path,
            };
            crate::routes::configure_with_team(app_state.clone(), secret_key.clone(), team_config)
        } else {
            crate::routes::configure(app_state.clone(), secret_key.clone())
        }
    };

    #[cfg(not(feature = "team"))]
    let api_routes = crate::routes::configure(app_state.clone(), secret_key.clone());

    let api_routes = api_routes
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
