//! AGIME Team Server - Standalone team collaboration server
//!
//! This server provides centralized team data storage and synchronization.
//! Users connect via API Key authentication.

mod auth;
mod config;
mod state;

use anyhow::Result;
use axum::{
    extract::State,
    http::StatusCode,
    response::Json,
    routing::{get, post},
    Router,
};
use sqlx::sqlite::SqlitePoolOptions;
use std::net::SocketAddr;
use std::sync::Arc;
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::TraceLayer;
use tracing::info;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use crate::auth::middleware::auth_middleware;
use crate::config::Config;
use crate::state::AppState;

#[tokio::main]
async fn main() -> Result<()> {
    // Load .env file if present
    dotenvy::dotenv().ok();

    // Initialize tracing
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "agime_team_server=info,tower_http=debug".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    // Load configuration
    let config = Config::from_env()?;
    info!("Starting AGIME Team Server on {}:{}", config.host, config.port);

    // Initialize database
    let database_url = config.database_url.clone();
    info!("Connecting to database: {}", database_url);

    // Ensure parent directory exists for SQLite
    if database_url.starts_with("sqlite:") {
        let path = database_url.trim_start_matches("sqlite:");
        let path = path.trim_start_matches("//");
        // Remove query string for path extraction
        let path = path.split('?').next().unwrap_or(path);
        if let Some(parent) = std::path::Path::new(path).parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }
    }

    let pool = SqlitePoolOptions::new()
        .max_connections(config.max_connections)
        .connect(&database_url)
        .await?;

    // Run migrations
    info!("Running database migrations...");
    agime_team::migrations::run_migration(&pool).await?;

    // Run auth migrations
    auth::migrations::run_migration(&pool).await?;
    info!("Database migrations completed");

    // Create app state
    let state = Arc::new(AppState {
        pool: Arc::new(pool),
        config: config.clone(),
    });

    // Build router
    let app = build_router(state);

    // Start server
    let addr = SocketAddr::new(config.host.parse()?, config.port);
    let listener = tokio::net::TcpListener::bind(addr).await?;
    info!("Server listening on {}", addr);

    axum::serve(listener, app).await?;

    Ok(())
}

fn build_router(state: Arc<AppState>) -> Router {
    // CORS configuration
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    // Public routes (no auth required)
    let public_routes = Router::new()
        .route("/", get(root))
        .route("/health", get(health_check))
        .route("/api/auth/register", post(auth::routes::register))
        .with_state(state.clone());

    // Protected auth routes (require auth)
    let protected_auth_routes = auth::routes::protected_router()
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            auth_middleware,
        ))
        .with_state(state.clone());

    // Team API routes - use agime_team's configure function
    let team_routes = agime_team::routes::configure(
        state.pool.clone(),
        "anonymous".to_string(), // Will be replaced by middleware
        std::path::PathBuf::from("./data/resources"),
    );

    // Wrap team routes with auth middleware
    let protected_team_routes = team_routes
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            auth_middleware,
        ));

    // Combine all routes
    Router::new()
        .merge(public_routes)
        .nest("/api/auth", protected_auth_routes)
        .nest("/api/team", protected_team_routes)
        .layer(TraceLayer::new_for_http())
        .layer(cors)
}

async fn root() -> &'static str {
    "AGIME Team Server"
}

async fn health_check(State(state): State<Arc<AppState>>) -> Result<Json<serde_json::Value>, StatusCode> {
    // Check database connection
    match sqlx::query("SELECT 1").fetch_one(state.pool.as_ref()).await {
        Ok(_) => Ok(Json(serde_json::json!({
            "status": "healthy",
            "database": "connected",
            "version": env!("CARGO_PKG_VERSION")
        }))),
        Err(_) => Err(StatusCode::SERVICE_UNAVAILABLE),
    }
}
