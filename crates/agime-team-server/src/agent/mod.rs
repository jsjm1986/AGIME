//! Agent module for team task queue management
//!
//! This module provides:
//! - Team agent configuration management
//! - Task submission and approval workflow
//! - Task execution and result streaming
//! - Rate limiting for API protection
//! - Task manager for background task tracking

// NOTE: executor disabled - uses SQLite, needs MongoDB version
// pub mod executor;
// MongoDB version of executor
pub mod ai_describe;
pub mod executor_mongo;
// NOTE: full_executor disabled - requires agime crate
// pub mod full_executor;
pub mod context_injector;
pub mod developer_tools;
pub mod document_tools;
pub mod extension_installer;
pub mod extension_manager_client;
pub mod mcp_connector;
pub mod platform_runner;
pub mod provider_factory;
pub mod rate_limit;
pub mod resource_access;
pub mod team_skill_tools;
// NOTE: routes disabled - uses SQLite
// pub mod routes;
pub mod routes_mongo;
// NOTE: service disabled - uses SQLite
// pub mod service;
pub mod service_mongo;
pub mod session_mongo;
pub mod streamer;
pub mod task_manager;

// Shared runtime utilities for executor bridge pattern
pub mod runtime;

// Chat Track (Phase 1)
pub mod chat_executor;
pub mod chat_manager;
pub mod chat_routes;

// Mission Track (Phase 2)
pub mod adaptive_executor;
pub mod document_analysis;
pub mod mission_executor;
pub mod mission_manager;
pub mod mission_mongo;
pub mod mission_routes;
pub mod portal_public;
pub mod portal_tools;
pub mod smart_log;
// NOTE: These modules disabled - require agime/agime-mcp crates
// pub mod team_skills_extension;
// pub mod team_mcp_extension;
// pub mod team_tools_server;

// pub use team_tools_server::TeamToolsServer;

// pub use executor::TaskExecutor;
// pub use full_executor::FullAgentExecutor;
#[allow(unused_imports)]
pub use executor_mongo::TaskExecutor;
#[allow(unused_imports)]
pub use rate_limit::{default_rate_limiter, task_rate_limiter, RateLimiter};
pub use routes_mongo::router; // Use MongoDB version
#[allow(unused_imports)]
pub use service_mongo::AgentService; // Use MongoDB version
#[allow(unused_imports)]
pub use streamer::stream_task_results;
#[allow(unused_imports)]
pub use task_manager::{create_task_manager, TaskManager};

// Chat Track exports
#[allow(unused_imports)]
pub use chat_manager::ChatManager;
#[allow(unused_imports)]
pub use chat_routes::chat_router;

// Mission Track exports
#[allow(unused_imports)]
pub use mission_manager::{create_mission_manager, MissionManager};
#[allow(unused_imports)]
pub use mission_routes::mission_router;

// Shared path normalization utility
use std::path::PathBuf;

pub fn normalize_workspace_path(path: &str) -> String {
    let input = PathBuf::from(path);
    let absolute = if input.is_absolute() {
        input
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(input)
    };
    let canonical = absolute.canonicalize().unwrap_or(absolute);
    let display = canonical.to_string_lossy().to_string();
    #[cfg(windows)]
    {
        if let Some(stripped) = display.strip_prefix(r"\\?\") {
            stripped.to_string()
        } else {
            display
        }
    }
    #[cfg(not(windows))]
    {
        display
    }
}
