//! AGIME Team Server - Standalone team collaboration server
//!
//! This server provides centralized team data storage and synchronization.
//! Users connect via API Key authentication.

mod agent;
mod auth;
mod config;
mod state;

use agime::config::paths::Paths;
use agime_mcp::{
    mcp_server_runner::{serve, McpCommand},
    AutoVisualiserRouter, ComputerControllerServer, DeveloperServer, MemoryServer, TutorialServer,
};
use anyhow::Result;
use axum::extract::DefaultBodyLimit;
use axum::http::header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE};
use axum::http::HeaderValue;
use axum::{
    extract::State,
    http::StatusCode,
    response::Json,
    routing::{get, post},
    Router,
};
use clap::{Parser, Subcommand};
use std::net::SocketAddr;
use std::sync::Arc;
use tower_http::cors::CorsLayer;
use tower_http::services::{ServeDir, ServeFile};
use tower_http::trace::TraceLayer;
use tracing::info;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

// Database backends
use agime_team::MongoDb;
use sqlx::sqlite::SqlitePoolOptions;

use crate::config::{Config, DatabaseType};
use crate::state::{AppState, DatabaseBackend};

/// AGIME Team Server CLI
#[derive(Parser)]
#[command(name = "agime-team-server")]
#[command(about = "Standalone team collaboration server for AGIME")]
struct Cli {
    /// Port to listen on (overrides config)
    #[arg(short, long)]
    port: Option<u16>,
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Run built-in MCP server over stdio
    Mcp {
        #[arg(value_parser = clap::value_parser!(McpCommand))]
        server: McpCommand,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Some(Commands::Mcp { server }) => run_mcp(server).await,
        None => run_server(cli.port).await,
    }
}

async fn run_mcp(server: McpCommand) -> Result<()> {
    // MCP protocol uses stdout; keep logs on stderr.
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| format!("mcp_{}=info", server.name()).into()),
        )
        .with_writer(std::io::stderr)
        .try_init();

    match server {
        McpCommand::AutoVisualiser => serve(AutoVisualiserRouter::new()).await?,
        McpCommand::ComputerController => serve(ComputerControllerServer::new()).await?,
        McpCommand::Memory => serve(MemoryServer::new()).await?,
        McpCommand::Tutorial => serve(TutorialServer::new()).await?,
        McpCommand::Developer => {
            let bash_env = Paths::config_dir().join(".bash_env");
            serve(
                DeveloperServer::new()
                    .extend_path_with_shell(true)
                    .bash_env_file(Some(bash_env)),
            )
            .await?
        }
    }
    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = tokio::signal::ctrl_c();

    #[cfg(unix)]
    {
        let mut sigterm =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()).unwrap();
        tokio::select! {
            _ = ctrl_c => info!("Received Ctrl+C, shutting down..."),
            _ = sigterm.recv() => info!("Received SIGTERM, shutting down..."),
        }
    }

    #[cfg(not(unix))]
    {
        ctrl_c.await.ok();
        info!("Received Ctrl+C, shutting down...");
    }
}

async fn run_server(port_override: Option<u16>) -> Result<()> {
    // Load .env file if present
    dotenvy::dotenv().ok();

    // Generate unique server instance ID for orphaned mission recovery
    let instance_id = uuid::Uuid::new_v4().to_string();
    std::env::set_var("TEAM_SERVER_INSTANCE_ID", &instance_id);

    // Initialize tracing
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "agime_team_server=info,tower_http=debug".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    // Load configuration
    let mut config = Config::from_env()?;
    // Apply CLI port override if provided
    if let Some(port) = port_override {
        config.port = port;
    }

    // Sync runtime feature flags to environment for executor/runtime modules.
    // This keeps TOML/file-based config consistent with env-based consumers.
    std::env::set_var("TEAM_AGENT_RESOURCE_MODE", &config.team_agent_resource_mode);
    std::env::set_var("TEAM_AGENT_SKILL_MODE", &config.team_agent_skill_mode);
    std::env::set_var(
        "TEAM_AGENT_AUTO_EXTENSION_POLICY",
        &config.team_agent_auto_extension_policy,
    );
    std::env::set_var(
        "TEAM_AGENT_AUTO_INSTALL_EXTENSIONS",
        if config.team_agent_auto_install_extensions {
            "true"
        } else {
            "false"
        },
    );
    std::env::set_var(
        "TEAM_AGENT_EXTENSION_CACHE_ROOT",
        &config.team_agent_extension_cache_root,
    );

    info!(
        "Starting AGIME Team Server on {}:{}",
        config.host, config.port
    );
    info!("Database type: {:?}", config.database_type);

    // Initialize database based on config
    let db = match config.database_type {
        DatabaseType::MongoDB => {
            // Redact credentials from the URL before logging
            let display_url = redact_url_credentials(&config.database_url);
            info!("Connecting to MongoDB: {}", display_url);
            let mongo = MongoDb::connect(&config.database_url, &config.database_name).await?;
            info!("Connected to MongoDB database: {}", config.database_name);

            // M12: Ensure chat session indexes
            let svc = agent::service_mongo::AgentService::new(Arc::new(mongo.clone()));
            svc.ensure_chat_indexes().await;

            // Mission Track: Ensure mission indexes
            svc.ensure_mission_indexes().await;

            DatabaseBackend::MongoDB(Arc::new(mongo))
        }
        DatabaseType::SQLite => {
            info!("Connecting to SQLite: {}", config.database_url);
            let pool = SqlitePoolOptions::new()
                .max_connections(config.max_connections)
                .connect(&config.database_url)
                .await?;
            // Run migrations
            sqlx::migrate!("../agime-team/src/migrations")
                .run(&pool)
                .await?;
            info!("Connected to SQLite database");
            DatabaseBackend::SQLite(Arc::new(pool))
        }
    };

    // Create app state with rate limiters and login guard
    let register_limiter = Some(Arc::new(agent::rate_limit::RateLimiter::new(5, 3600)));
    let login_limiter = Some(Arc::new(agent::rate_limit::RateLimiter::new(10, 60)));
    let login_guard = Some(Arc::new(auth::service_mongo::LoginGuard::new(
        config.login_max_failures,
        config.login_lockout_minutes,
    )));

    let state = Arc::new(AppState {
        db,
        config: config.clone(),
        register_limiter,
        login_limiter,
        login_guard,
    });

    // Build router
    let app = build_router(state);

    // Start server
    let addr = SocketAddr::new(config.host.parse()?, config.port);
    let listener = tokio::net::TcpListener::bind(addr).await?;
    info!("Server listening on {}", addr);

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    info!("Server shut down gracefully");
    Ok(())
}

fn build_router(state: Arc<AppState>) -> Router {
    // Set AGIME_TEAM_API_URL for invite links if base_url is configured
    if let Some(ref base_url) = state.config.base_url {
        std::env::set_var("AGIME_TEAM_API_URL", base_url);
    }

    // CORS configuration - configurable whitelist or mirror_request for dev
    let cors = if let Some(ref origins) = state.config.cors_allowed_origins {
        let allowed: Vec<HeaderValue> = origins
            .split(',')
            .filter_map(|s| s.trim().parse().ok())
            .collect();
        CorsLayer::new().allow_origin(allowed)
    } else {
        CorsLayer::new().allow_origin(tower_http::cors::AllowOrigin::mirror_request())
    }
    .allow_methods([
        axum::http::Method::GET,
        axum::http::Method::POST,
        axum::http::Method::PUT,
        axum::http::Method::DELETE,
        axum::http::Method::OPTIONS,
    ])
    .allow_headers([
        AUTHORIZATION,
        ACCEPT,
        CONTENT_TYPE,
        axum::http::header::COOKIE,
    ])
    .allow_credentials(true);

    // Public routes (no auth required)
    let public_routes = match &state.db {
        DatabaseBackend::MongoDB(_) => Router::new()
            .route("/", get(root))
            .route("/health", get(health_check))
            .route("/api/auth/register", post(auth::routes_mongo::register))
            .route("/api/auth/login", post(auth::routes_mongo::login))
            .route("/api/auth/login/password", post(auth::routes_mongo::login_with_password))
            .route("/api/auth/logout", post(auth::routes_mongo::logout))
            .route("/api/auth/session", get(auth::routes_mongo::get_session))
            .with_state(state.clone()),
        DatabaseBackend::SQLite(_) => Router::new()
            .route("/", get(root))
            .route("/health", get(health_check))
            .route("/api/auth/register", post(auth::routes_sqlite::register))
            .route("/api/auth/login", post(auth::routes_sqlite::login))
            .route("/api/auth/logout", post(auth::routes_sqlite::logout))
            .route("/api/auth/session", get(auth::routes_sqlite::get_session))
            .with_state(state.clone()),
    };

    // Protected auth routes (require auth)
    let protected_auth_routes = match &state.db {
        DatabaseBackend::MongoDB(_) => auth::routes_mongo::protected_router()
            .layer(axum::middleware::from_fn_with_state(
                state.clone(),
                auth::middleware_mongo::auth_middleware,
            ))
            .with_state(state.clone()),
        DatabaseBackend::SQLite(_) => auth::routes_sqlite::protected_router()
            .layer(axum::middleware::from_fn_with_state(
                state.clone(),
                auth::middleware_sqlite::auth_middleware,
            ))
            .with_state(state.clone()),
    };

    // Create AI triggers for smart log summaries and document analysis (MongoDB only)
    let (smart_log_trigger, doc_analysis_trigger): (
        Option<Arc<dyn agime_team::models::mongo::SmartLogTrigger>>,
        Option<Arc<dyn agime_team::models::mongo::DocumentAnalysisTrigger>>,
    ) = match &state.db {
        DatabaseBackend::MongoDB(db) => {
            let agent_service = Arc::new(agent::AgentService::new(db.clone()));
            let smart_log = Arc::new(agent::smart_log::SmartLogTriggerImpl::new(
                db.clone(),
                agent_service.clone(),
            ));
            let doc_analysis =
                Arc::new(agent::document_analysis::DocumentAnalysisTriggerImpl::new(
                    db.clone(),
                    agent_service,
                    state.config.workspace_root.clone(),
                ));
            (Some(smart_log), Some(doc_analysis))
        }
        DatabaseBackend::SQLite(_) => (None, None),
    };

    // Compute the portal base URL from config
    let portal_base_url_configured = state.config.base_url.is_some();
    let portal_base_url = state.config.base_url.clone().unwrap_or_else(|| {
        let host = if state.config.host == "0.0.0.0" {
            // 0.0.0.0 is not useful as a public URL; fall back to localhost
            "127.0.0.1".to_string()
        } else {
            state.config.host.clone()
        };
        format!("http://{}:{}", host, state.config.port)
    });
    let portal_test_base_url = state.config.portal_test_base_url.clone().or_else(|| {
        let bind_host = state.config.host.as_str();
        let host = if bind_host == "0.0.0.0" || bind_host == "::" {
            "127.0.0.1".to_string()
        } else {
            state.config.host.clone()
        };
        Some(format!("http://{}:{}", host, state.config.port))
    });

    // Expose portal base URL to executor/runtime modules via env var
    std::env::set_var("PORTAL_BASE_URL", &portal_base_url);

    // Team API routes based on database type
    let team_routes: Router = match &state.db {
        DatabaseBackend::MongoDB(db) => {
            let mongo_state = agime_team::routes::mongo::AppState {
                db: db.clone(),
                smart_log_trigger: smart_log_trigger.clone(),
                doc_analysis_trigger: doc_analysis_trigger.clone(),
                portal_base_url: portal_base_url.clone(),
                portal_base_url_configured,
                portal_test_base_url: portal_test_base_url.clone(),
                workspace_root: state.config.workspace_root.clone(),
            };
            agime_team::routes::mongo::configure(Arc::new(mongo_state))
        }
        DatabaseBackend::SQLite(pool) => {
            // SQLite routes use the existing agime-team routes
            agime_team::routes::configure_routes(pool.clone())
        }
    };

    // Wrap team routes with auth middleware
    let protected_team_routes = match &state.db {
        DatabaseBackend::MongoDB(_) => team_routes.layer(axum::middleware::from_fn_with_state(
            state.clone(),
            auth::middleware_mongo::auth_middleware,
        )),
        DatabaseBackend::SQLite(_) => team_routes.layer(axum::middleware::from_fn_with_state(
            state.clone(),
            auth::middleware_sqlite::auth_middleware,
        )),
    };

    // Agent routes (only available for MongoDB currently)
    let agent_routes = match &state.db {
        DatabaseBackend::MongoDB(db) => Some(agent::router(db.clone()).layer(
            axum::middleware::from_fn_with_state(
                state.clone(),
                auth::middleware_mongo::auth_middleware,
            ),
        )),
        DatabaseBackend::SQLite(_) => None,
    };

    // Shared ChatManager (used by both chat_routes and portal_public_routes)
    let chat_manager: Option<Arc<agent::ChatManager>> = match &state.db {
        DatabaseBackend::MongoDB(db) => {
            let cm = Arc::new(agent::ChatManager::new());

            // Startup: immediately reset any stuck is_processing sessions from previous run
            {
                let startup_db = db.clone();
                tokio::spawn(async move {
                    let svc = agent::service_mongo::AgentService::new(startup_db);
                    match svc.reset_stuck_processing(std::time::Duration::from_secs(0)).await {
                        Ok(n) if n > 0 => tracing::warn!("Startup: reset {} stuck chat sessions", n),
                        Err(e) => tracing::error!("Startup: failed to reset stuck sessions: {}", e),
                        _ => {}
                    }
                });
            }

            // M6: Spawn background task to clean up stale chat sessions
            // Recovers both in-memory ChatManager entries and DB is_processing flags
            {
                let cm2 = cm.clone();
                let cleanup_db = db.clone();
                tokio::spawn(async move {
                    let cleanup_interval_secs = std::env::var("TEAM_CHAT_CLEANUP_INTERVAL_SECS")
                        .ok()
                        .and_then(|v| v.parse::<u64>().ok())
                        .filter(|v| *v >= 15)
                        .unwrap_or(60);
                    let max_age_secs = std::env::var("TEAM_CHAT_STALE_MAX_AGE_SECS")
                        .ok()
                        .and_then(|v| v.parse::<u64>().ok())
                        .filter(|v| *v >= 300)
                        .unwrap_or(4 * 60 * 60); // default 4 hours inactivity
                    let cleanup_interval = std::time::Duration::from_secs(cleanup_interval_secs);
                    let max_age = std::time::Duration::from_secs(max_age_secs);
                    loop {
                        tokio::time::sleep(cleanup_interval).await;
                        let removed = cm2.cleanup_stale(max_age).await;
                        if removed > 0 {
                            tracing::info!(
                                "Background cleanup removed {} stale chat entries",
                                removed
                            );
                        }
                        let svc = agent::service_mongo::AgentService::new(cleanup_db.clone());
                        match svc.reset_stuck_processing(max_age).await {
                            Ok(n) if n > 0 => {
                                tracing::warn!("Reset {} stuck is_processing sessions in DB", n);
                            }
                            Err(e) => {
                                tracing::error!("Failed to reset stuck sessions: {}", e);
                            }
                            _ => {}
                        }
                    }
                });
            }

            // Spawn background task to clean up expired auth sessions
            {
                let session_db = db.clone();
                tokio::spawn(async move {
                    let interval = std::time::Duration::from_secs(600); // every 10 minutes
                    loop {
                        tokio::time::sleep(interval).await;
                        let ss = auth::session_mongo::SessionService::new(session_db.clone());
                        match ss.cleanup_expired().await {
                            Ok(n) if n > 0 => {
                                tracing::info!("Cleaned up {} expired auth sessions", n);
                            }
                            Err(e) => {
                                tracing::warn!("Session cleanup error: {}", e);
                            }
                            _ => {}
                        }
                    }
                });
            }

            Some(cm)
        }
        DatabaseBackend::SQLite(_) => None,
    };

    // Chat routes (Phase 1 - Chat Track, MongoDB only)
    let chat_routes = match (&state.db, &chat_manager) {
        (DatabaseBackend::MongoDB(db), Some(cm)) => Some(
            agent::chat_router(db.clone(), cm.clone(), state.config.workspace_root.clone()).layer(
                axum::middleware::from_fn_with_state(
                    state.clone(),
                    auth::middleware_mongo::auth_middleware,
                ),
            ),
        ),
        _ => None,
    };

    // AI Describe routes (only available for MongoDB)
    let ai_describe_routes = match &state.db {
        DatabaseBackend::MongoDB(db) => {
            let config = Arc::new(state.config.clone());
            Some(
                agent::routes_mongo::ai_describe_router(db.clone(), config).layer(
                    axum::middleware::from_fn_with_state(
                        state.clone(),
                        auth::middleware_mongo::auth_middleware,
                    ),
                ),
            )
        }
        DatabaseBackend::SQLite(_) => None,
    };

    // Mission routes (Phase 2 - Mission Track, MongoDB only)
    let mission_routes = match &state.db {
        DatabaseBackend::MongoDB(db) => {
            let mission_manager = Arc::new(agent::MissionManager::new());

            // Recover orphaned Running/Planning missions from previous server instance
            {
                let recovery_db = db.clone();
                let iid = std::env::var("TEAM_SERVER_INSTANCE_ID").unwrap_or_default();
                tokio::spawn(async move {
                    let svc = agent::service_mongo::AgentService::new(recovery_db);
                    match svc.recover_orphaned_missions(&iid).await {
                        Ok(n) if n > 0 => tracing::warn!("Recovered {} orphaned missions on startup", n),
                        Err(e) => tracing::error!("Failed to recover orphaned missions: {}", e),
                        _ => {}
                    }
                });
            }

            // Spawn background task to clean up stale missions
            {
                let mm = mission_manager.clone();
                tokio::spawn(async move {
                    let cleanup_interval = std::time::Duration::from_secs(120);
                    let max_age_secs = std::env::var("TEAM_MISSION_STALE_SECS")
                        .ok().and_then(|v| v.parse::<u64>().ok()).unwrap_or(3 * 60 * 60);
                    let max_age = std::time::Duration::from_secs(max_age_secs);
                    loop {
                        tokio::time::sleep(cleanup_interval).await;
                        let removed = mm.cleanup_stale(max_age).await;
                        if removed > 0 {
                            tracing::info!(
                                "Background cleanup removed {} stale mission entries",
                                removed
                            );
                        }
                    }
                });
            }

            // Spawn background task to cancel stale pending AI analyses
            {
                let cleanup_db = db.clone();
                tokio::spawn(async move {
                    let interval = std::time::Duration::from_secs(300);
                    loop {
                        tokio::time::sleep(interval).await;
                        let svc = agime_team::services::mongo::CleanupService::new(
                            cleanup_db.as_ref().clone(),
                        );
                        match svc.cancel_stale_analyses(1).await {
                            Ok(n) if n > 0 => {
                                tracing::info!("Cancelled {} stale pending AI analyses", n);
                            }
                            Err(e) => {
                                tracing::error!("Stale analysis cleanup error: {}", e);
                            }
                            _ => {}
                        }
                    }
                });
            }

            Some(
                agent::mission_router(
                    db.clone(),
                    mission_manager,
                    state.config.workspace_root.clone(),
                )
                .layer(axum::middleware::from_fn_with_state(
                    state.clone(),
                    auth::middleware_mongo::auth_middleware,
                )),
            )
        }
        DatabaseBackend::SQLite(_) => None,
    };

    // Portal public routes (no auth required, mounted before protected routes)
    let portal_public_routes = match (&state.db, &chat_manager) {
        (DatabaseBackend::MongoDB(db), Some(cm)) => {
            Some(agent::portal_public::portal_public_routes(
                db.clone(),
                cm.clone(),
                state.config.workspace_root.clone(),
            ))
        }
        (DatabaseBackend::MongoDB(db), None) => Some(agent::portal_public::portal_public_routes(
            db.clone(),
            Arc::new(agent::ChatManager::new()),
            state.config.workspace_root.clone(),
        )),
        _ => None,
    };

    // Combine all routes
    let admin_router = match &state.db {
        DatabaseBackend::MongoDB(_) => auth::routes_mongo::admin_router()
            .layer(axum::middleware::from_fn_with_state(
                state.clone(),
                auth::middleware_mongo::auth_middleware,
            ))
            .with_state(state.clone()),
        DatabaseBackend::SQLite(_) => Router::new().with_state(state.clone()),
    };

    let mut api_router = Router::new()
        .merge(public_routes)
        .nest("/api/auth", protected_auth_routes)
        .nest("/api/auth/admin", admin_router)
        .nest("/api/team", protected_team_routes);

    // Add portal public routes (no auth, before other middleware)
    if let Some(portal_router) = portal_public_routes {
        api_router = api_router.merge(portal_router);
    }

    // Add agent routes if available (MongoDB only)
    if let Some(agent_router) = agent_routes {
        api_router = api_router.nest("/api/team/agent", agent_router);
    }

    // Add AI describe routes if available (MongoDB only)
    if let Some(ai_router) = ai_describe_routes {
        api_router = api_router.nest("/api/teams", ai_router);
    }

    // Add chat routes if available (MongoDB only)
    if let Some(chat_router) = chat_routes {
        api_router = api_router.nest("/api/team/agent/chat", chat_router);
    }

    // Add mission routes if available (MongoDB only)
    if let Some(mission_router) = mission_routes {
        api_router = api_router.nest("/api/team/agent/mission", mission_router);
    }

    let api_router = api_router
        .layer(DefaultBodyLimit::max(16 * 1024 * 1024)) // 16MB for package uploads
        .layer(TraceLayer::new_for_http())
        .layer(cors);

    // Static file service for web admin
    serve_web_admin(api_router)
}

async fn root() -> &'static str {
    "AGIME Team Server"
}

async fn health_check(
    State(state): State<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    // Check database connection based on backend type
    match &state.db {
        DatabaseBackend::MongoDB(db) => match db.ping().await {
            Ok(_) => Ok(Json(serde_json::json!({
                "status": "healthy",
                "database": "mongodb",
                "database_connected": true,
                "version": env!("CARGO_PKG_VERSION")
            }))),
            Err(_) => Err(StatusCode::SERVICE_UNAVAILABLE),
        },
        DatabaseBackend::SQLite(pool) => {
            match sqlx::query("SELECT 1").execute(pool.as_ref()).await {
                Ok(_) => Ok(Json(serde_json::json!({
                    "status": "healthy",
                    "database": "sqlite",
                    "database_connected": true,
                    "version": env!("CARGO_PKG_VERSION")
                }))),
                Err(_) => Err(StatusCode::SERVICE_UNAVAILABLE),
            }
        }
    }
}

/// Redact credentials from a database URL for safe logging.
/// Replaces `user:password@` with `***:***@` in URLs like `mongodb://user:pass@host`.
fn redact_url_credentials(url: &str) -> String {
    // Find the "://" scheme separator
    if let Some(scheme_end) = url.find("://") {
        let after_scheme = &url[scheme_end + 3..];
        // If there's an '@', credentials are present before it
        if let Some(at_pos) = after_scheme.find('@') {
            let host_part = &after_scheme[at_pos..]; // includes '@'
            return format!("{}://***:***{}", &url[..scheme_end], host_part);
        }
    }
    url.to_string()
}

fn serve_web_admin(api_router: Router) -> Router {
    // Try multiple possible locations for the web admin dist folder
    let possible_paths = [
        std::path::PathBuf::from("./web-admin/dist"),
        std::path::PathBuf::from("./crates/agime-team-server/web-admin/dist"),
    ];

    let web_admin_dir = possible_paths.iter().find(|p| p.exists());

    if let Some(dir) = web_admin_dir {
        info!("Serving web admin from {:?}", dir);
        let index_file = dir.join("index.html");
        let serve_dir = ServeDir::new(dir).fallback(ServeFile::new(index_file));
        Router::new()
            .merge(api_router)
            .nest_service("/admin", serve_dir)
    } else {
        info!("Web admin not found, skipping static file service");
        api_router
    }
}
