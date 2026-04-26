//! Agent module for team task queue management
//!
//! This module provides:
//! - Team agent configuration management
//! - Task submission and approval workflow
//! - Task execution and result streaming
//! - Rate limiting for API protection
//! - Task manager for background task tracking

// Primary Mongo-backed task host
pub mod agent_prompt_composer;
pub mod ai_describe;
pub mod api_tools;
pub mod avatar_governance_tools;
pub mod capability_policy;
pub mod context_injector;
pub mod delegation_runtime;
pub mod developer_tools;
pub mod document_tools;
pub mod execution_admission;
pub mod executor_mongo;
pub mod extension_installer;
pub mod extension_manager_client;
pub mod harness_adapter;
pub mod harness_core;
pub mod hook_runtime;
pub mod host_router;
pub mod local_fs_workspace_store;
pub mod mcp_connector;
pub mod platform_runner;
pub mod provider_factory;
pub mod rate_limit;
pub mod resource_access;
pub mod routes_mongo;
pub mod server_harness_host;
pub mod service_mongo;
pub mod session_mongo;
pub mod skill_registry_routes;
pub mod skill_registry_tools;
pub mod streamer;
pub mod task_manager;
pub mod team_mcp_tools;
pub mod team_skill_tools;
pub mod workspace_physical_store;
pub mod workspace_service;
pub mod workspace_types;

// Shared runtime utilities for executor bridge pattern
pub mod runtime;
pub mod runtime_bridge;

// Chat Track (Phase 1)
pub mod channel_coding_cards;
pub mod channel_project_workspace;
pub mod channel_workspace_governance;
pub mod chat_channel_executor;
pub mod chat_channel_manager;
pub mod chat_channel_orchestrator;
pub mod chat_channels;
pub mod chat_delivery_tools;
pub mod chat_executor;
pub mod chat_manager;
pub mod chat_memory;
pub mod chat_memory_tools;
pub mod chat_routes;

pub mod document_analysis;
pub mod portal_public;
pub mod portal_tools;
pub mod prompt_profiles;
pub mod smart_log;
// NOTE: These modules disabled - require agime/agime-mcp crates
// pub mod team_skills_extension;
// pub mod team_mcp_extension;
// pub mod team_tools_server;
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
pub use chat_routes::chat_public_router;
#[allow(unused_imports)]
pub use chat_routes::chat_router;
#[allow(unused_imports)]
pub use skill_registry_routes::skill_registry_router;

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
