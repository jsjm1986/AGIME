//! Model Capability Registry
//!
//! This module provides a configuration-driven system for managing model capabilities,
//! replacing hardcoded model-specific checks throughout the codebase.
//!
//! # Overview
//!
//! Different AI models have different capabilities:
//! - Some support "thinking" or "extended reasoning" (Claude 3.7+, DeepSeek, Qwen)
//! - Some have special API requirements (O-series reasoning effort, beta headers)
//! - Some have constraints (no temperature support, special tool formats)
//!
//! Instead of hardcoding these checks like:
//! ```ignore
//! if model_name.starts_with("claude-3-7-sonnet-") { ... }
//! ```
//!
//! We use a configuration-driven approach:
//! ```ignore
//! let caps = capabilities::resolve(&model_name);
//! if caps.thinking_enabled { ... }
//! ```
//!
//! # Configuration Sources
//!
//! Capabilities are resolved from (highest to lowest priority):
//! 1. Environment variables (backward compatibility)
//! 2. User overrides in config.yaml
//! 3. Bundled defaults (model_capabilities.json)
//!
//! # Usage
//!
//! ```ignore
//! use agime::capabilities;
//!
//! // Check if a model supports thinking
//! if capabilities::supports_thinking("claude-3-7-sonnet-latest") {
//!     // Enable thinking UI
//! }
//!
//! // Get resolved capabilities
//! let caps = capabilities::resolve("claude-3-7-sonnet-latest");
//! if caps.thinking_enabled {
//!     // Add thinking parameters to request
//! }
//!
//! // Get request headers
//! let headers = capabilities::get_headers("claude-3-7-sonnet-latest");
//! ```

mod registry;
mod runtime;
mod types;

pub use registry::{CapabilityRegistry, ConfigSource};
pub use runtime::ResolvedCapabilities;
pub use types::*;

use std::path::PathBuf;

// ============================================================================
// Internal helper for registry access
// ============================================================================

/// Helper to access registry with proper error handling.
/// Returns default capabilities if registry lock fails.
fn with_registry<T, F>(f: F) -> T
where
    F: FnOnce(&CapabilityRegistry) -> T,
    T: Default,
{
    match CapabilityRegistry::global().read() {
        Ok(registry) => f(&registry),
        Err(e) => {
            tracing::error!("Failed to acquire registry read lock: {}", e);
            T::default()
        }
    }
}

/// Helper to access registry for write operations.
fn with_registry_mut<T, F>(f: F) -> anyhow::Result<T>
where
    F: FnOnce(&mut CapabilityRegistry) -> anyhow::Result<T>,
{
    match CapabilityRegistry::global().write() {
        Ok(mut registry) => f(&mut registry),
        Err(e) => {
            tracing::error!("Failed to acquire registry write lock: {}", e);
            Err(anyhow::anyhow!(
                "Failed to acquire registry write lock: {}",
                e
            ))
        }
    }
}

// ============================================================================
// Convenience Functions
// ============================================================================

/// Resolve capabilities for a model.
///
/// This is the main entry point for capability queries.
/// Returns default capabilities if registry access fails.
pub fn resolve(model_name: &str) -> ResolvedCapabilities {
    with_registry(|r| r.resolve(model_name))
}

/// Check if a model supports thinking mode.
pub fn supports_thinking(model_name: &str) -> bool {
    with_registry(|r| r.supports_thinking(model_name))
}

/// Check if thinking mode is enabled for a model.
///
/// This considers both model support AND user/env configuration.
pub fn is_thinking_enabled(model_name: &str) -> bool {
    with_registry(|r| r.is_thinking_enabled(model_name))
}

/// Get thinking budget for a model (if thinking is enabled).
pub fn get_thinking_budget(model_name: &str) -> Option<u32> {
    with_registry(|r| r.get_thinking_budget(model_name))
}

/// Check if a model supports reasoning effort (O-series models).
pub fn supports_reasoning(model_name: &str) -> bool {
    with_registry(|r| r.supports_reasoning(model_name))
}

/// Get reasoning effort level for a model.
pub fn get_reasoning_effort(model_name: &str) -> Option<String> {
    with_registry(|r| r.get_reasoning_effort(model_name))
}

/// Get system role name for a model ("system" or "developer").
pub fn get_system_role(model_name: &str) -> String {
    with_registry(|r| r.get_system_role(model_name))
}

/// Get headers for a model request.
pub fn get_headers(model_name: &str) -> Vec<(String, String)> {
    with_registry(|r| r.get_headers(model_name))
}

/// Check if temperature is supported for a model.
pub fn supports_temperature(model_name: &str) -> bool {
    with_registry(|r| r.supports_temperature(model_name))
}

/// Check if tools are supported for a model.
pub fn supports_tools(model_name: &str) -> bool {
    with_registry(|r| r.supports_tools(model_name))
}

/// Get schema processor for tools (e.g., "gemini" for special handling).
pub fn get_schema_processor(model_name: &str) -> Option<String> {
    with_registry(|r| r.get_schema_processor(model_name))
}

/// Infer provider from model name.
pub fn infer_provider(model_name: &str) -> Option<String> {
    with_registry(|r| r.infer_provider(model_name))
}

/// Get all model patterns that support thinking.
pub fn get_thinking_models() -> Vec<String> {
    with_registry(|r| r.get_thinking_models())
}

/// Get all model patterns that support reasoning.
pub fn get_reasoning_models() -> Vec<String> {
    with_registry(|r| r.get_reasoning_models())
}

/// Reload capability configuration.
///
/// This clears the cache and reloads capabilities from file (or bundled) and user overrides.
pub fn reload() -> anyhow::Result<()> {
    with_registry_mut(|r| r.reload())
}

// ============================================================================
// Config File Management
// ============================================================================

/// Get the path to the model capabilities config file.
///
/// The file is located at the platform-specific user config directory:
/// - Linux/macOS: `~/.config/goose/model_capabilities.json`
/// - Windows: `%APPDATA%\goose\model_capabilities.json`
///
/// This file can be edited to customize model capabilities without recompiling.
/// Changes take effect after restarting the application.
pub fn get_config_file_path() -> PathBuf {
    CapabilityRegistry::get_config_file_path()
}

/// Ensure the config file exists, creating it with bundled defaults if necessary.
///
/// Returns the path to the config file.
pub fn ensure_config_file() -> anyhow::Result<PathBuf> {
    CapabilityRegistry::ensure_config_file()
}

/// Get the current config source (file or bundled).
pub fn get_config_source() -> Option<ConfigSource> {
    match CapabilityRegistry::global().read() {
        Ok(registry) => Some(registry.get_config_source().clone()),
        Err(_) => None,
    }
}
