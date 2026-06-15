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

    // Desktop default: enable the native swarm tool so explicit "swarm/并行"
    // requests can actually delegate. `native_swarm_tool_enabled()` reads this
    // env lazily, so it must be set before the first request/tool-list build.
    // Set-if-unset preserves an explicit operator override. Team-server is a
    // separate binary and never runs this code path.
    #[cfg(feature = "desktop_harness_host")]
    {
        const NATIVE_SWARM_TOOL_ENV: &str = "AGIME_ENABLE_NATIVE_SWARM_TOOL";
        if std::env::var(NATIVE_SWARM_TOOL_ENV).is_err() {
            std::env::set_var(NATIVE_SWARM_TOOL_ENV, "1");
        }

        // Desktop-only: when the model announces a next action on the
        // Conversation surface but doesn't call the tool, nudge it to continue
        // instead of ending the turn. team-server is a separate binary without
        // this feature, so its chat behavior is unchanged.
        const PREAMBLE_NUDGE_ENV: &str = "AGIME_PREAMBLE_NUDGE";
        if std::env::var(PREAMBLE_NUDGE_ENV).is_err() {
            std::env::set_var(PREAMBLE_NUDGE_ENV, "1");
        }

        // Desktop-only: raise the per-reply recovery-compaction cap so long
        // turns aren't aborted after only 3 context overflows. team-server
        // keeps the core default (3).
        const MAX_COMPACTION_ATTEMPTS_ENV: &str = "AGIME_MAX_COMPACTION_ATTEMPTS";
        if std::env::var(MAX_COMPACTION_ATTEMPTS_ENV).is_err() {
            std::env::set_var(MAX_COMPACTION_ATTEMPTS_ENV, "6");
        }

        // Desktop-only: append a delegation guidance overlay to the system
        // prompt so the model actually uses its subagent/swarm tools in
        // conversation. The core prompt doesn't teach delegation and even
        // discourages claiming it, so without this overlay a desktop model
        // almost never delegates. Set `0` to revert. team-server is a separate
        // binary and never runs this code path.
        const DELEGATION_GUIDANCE_ENV: &str = "AGIME_DESKTOP_DELEGATION_GUIDANCE";
        if std::env::var(DELEGATION_GUIDANCE_ENV).is_err() {
            std::env::set_var(DELEGATION_GUIDANCE_ENV, "1");
        }
    }

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

    // Reconcile any user-level long-running tasks that were mid-flight when
    // the process last exited: terminal tasks are reloaded so the UI list
    // survives a restart; interrupted ones are marked failed. Best-effort —
    // never blocks server start.
    #[cfg(feature = "desktop_harness_host")]
    {
        app_state.task_manager().await.resume_queued_tasks().await;

        // Start the background scheduler that fires due scheduled tasks.
        let scheduler_tz = std::env::var("AGIME_SCHEDULER_TIMEZONE")
            .ok()
            .filter(|tz| !tz.trim().is_empty())
            .unwrap_or_else(|| "Asia/Shanghai".to_string());
        crate::scheduled_tasks::routes::start_scheduler(app_state.clone(), scheduler_tz);
    }

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
                tracing::warn!(
                    "Failed to initialize team database: {}. Team features will be disabled.",
                    e
                );
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
                tracing::warn!(
                    "Failed to create team resources directory: {}. Using current directory.",
                    e
                );
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
