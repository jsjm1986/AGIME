pub mod auth;
pub mod configuration;
#[cfg(feature = "desktop_harness_host")]
pub mod desktop_harness_host;
pub mod error;
#[cfg(feature = "desktop_harness_host")]
pub mod host_admission;
#[cfg(feature = "desktop_harness_host")]
pub mod host_capability;
#[cfg(feature = "desktop_harness_host")]
pub mod host_document_analysis;
pub mod host_helpers;
pub mod host_prompt;
pub mod host_provider;
#[cfg(feature = "desktop_harness_host")]
pub mod host_stream;
#[cfg(feature = "desktop_harness_host")]
pub mod host_task;
#[cfg(feature = "desktop_harness_host")]
pub mod host_tool_dispatch;
pub mod host_workspace;
#[cfg(feature = "desktop_harness_host")]
pub mod host_workspace_runtime;
#[cfg(feature = "desktop_harness_host")]
pub mod host_workspace_service;
#[cfg(feature = "desktop_harness_host")]
pub mod host_workspace_store;
#[cfg(feature = "desktop_harness_host")]
pub mod host_workspace_types;
pub mod openapi;
pub mod routes;
pub mod state;

#[cfg(feature = "team")]
pub mod team;

// Re-export commonly used items
pub use openapi::*;
pub use state::*;

#[cfg(feature = "team")]
pub use routes::TeamRoutesConfig;
