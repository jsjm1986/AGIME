//! Model Capability Registry implementation.
//!
//! The registry loads capability definitions from:
//! 1. User config directory (~/.config/agime/model_capabilities.json or %APPDATA%/agime/)
//! 2. Bundled defaults (compiled into binary) - fallback
//! 3. User overrides (~/.config/agime/config.yaml)
//! 4. Environment variables (for backward compatibility)

use anyhow::{Context, Result};
use glob::Pattern;
use once_cell::sync::Lazy;
use std::collections::HashMap;
use std::io::Write;
use std::path::PathBuf;
use std::sync::RwLock;

use super::runtime::ResolvedCapabilities;
use super::types::*;
use crate::config::paths::Paths;

/// Minimum budget value (used for validation)
const MIN_THINKING_BUDGET: u32 = 1024;
/// Maximum budget value (used for validation)
const MAX_THINKING_BUDGET: u32 = 100000;

/// Config file name for model capabilities
const CAPABILITIES_FILE_NAME: &str = "model_capabilities.json";

/// UTF-8 BOM character
const UTF8_BOM: char = '\u{FEFF}';

/// Bundled default capabilities configuration (fallback)
const BUNDLED_CAPABILITIES: &str = include_str!("data/model_capabilities.json");

/// Global capability registry instance
static CAPABILITY_REGISTRY: Lazy<RwLock<CapabilityRegistry>> = Lazy::new(|| {
    let registry = CapabilityRegistry::new().expect("Failed to initialize capability registry");
    RwLock::new(registry)
});

/// Compiled capability entry with pre-parsed glob pattern
#[derive(Debug)]
struct CompiledCapability {
    pattern: Pattern,
    raw_pattern: String,
    capabilities: ModelCapabilities,
}

/// Model Capability Registry
///
/// Manages model capability definitions and provides resolution API.
///
/// Configuration priority (highest to lowest):
/// 1. Environment variables (CLAUDE_THINKING_ENABLED, CLAUDE_THINKING_BUDGET)
/// 2. User overrides (config.yaml -> model_overrides)
/// 3. User config file (~/.config/agime/model_capabilities.json or %APPDATA%)
/// 4. Bundled defaults (compiled into binary)
pub struct CapabilityRegistry {
    /// Model capability definitions (sorted by priority descending)
    capabilities: Vec<CompiledCapability>,

    /// User overrides from config (model_name -> override)
    user_overrides: HashMap<String, ModelOverride>,

    /// Cache of resolved capabilities (using RwLock for interior mutability)
    cache: RwLock<HashMap<String, ResolvedCapabilities>>,

    /// Source of the loaded capabilities (for debugging/logging)
    config_source: ConfigSource,
}

/// Source of the capabilities configuration
#[derive(Debug, Clone)]
pub enum ConfigSource {
    /// Loaded from user config file
    File(PathBuf),
    /// Using bundled defaults
    Bundled,
}

impl CapabilityRegistry {
    /// Get the global registry instance
    pub fn global() -> &'static RwLock<CapabilityRegistry> {
        &CAPABILITY_REGISTRY
    }

    /// Get the path to the config file (in user config directory)
    ///
    /// Returns platform-specific path:
    /// - Linux/macOS: `~/.config/agime/model_capabilities.json`
    /// - Windows: `%APPDATA%\agime\model_capabilities.json`
    pub fn get_config_file_path() -> PathBuf {
        Paths::config_dir().join(CAPABILITIES_FILE_NAME)
    }

    /// Initialize with capabilities from file (preferred) or bundled defaults (fallback)
    pub fn new() -> Result<Self> {
        let mut registry = Self {
            capabilities: Vec::new(),
            user_overrides: HashMap::new(),
            cache: RwLock::new(HashMap::new()),
            config_source: ConfigSource::Bundled,
        };

        // Try to load from user config file first
        let config_path = Self::get_config_file_path();
        let (config, source) = if config_path.exists() {
            match Self::load_from_file(&config_path) {
                Ok(cfg) => {
                    tracing::info!(
                        "Loaded model capabilities from file: {}",
                        config_path.display()
                    );
                    (cfg, ConfigSource::File(config_path))
                }
                Err(e) => {
                    tracing::warn!(
                        "Failed to load capabilities from {}: {}. Using bundled defaults.",
                        config_path.display(),
                        e
                    );
                    (Self::load_bundled()?, ConfigSource::Bundled)
                }
            }
        } else {
            tracing::info!(
                "No user config file at {}. Using bundled defaults.",
                config_path.display()
            );
            // Optionally copy bundled to user config for easy editing
            if let Err(e) = Self::ensure_config_file() {
                tracing::debug!("Could not create default config file: {}", e);
            }
            (Self::load_bundled()?, ConfigSource::Bundled)
        };

        registry.config_source = source;
        registry.load_capabilities(config)?;

        // Load user overrides
        registry.load_user_overrides();

        Ok(registry)
    }

    /// Load capabilities from a file
    ///
    /// Handles UTF-8 BOM (Byte Order Mark) that Windows Notepad may add.
    fn load_from_file(path: &PathBuf) -> Result<CapabilitiesConfig> {
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read file: {}", path.display()))?;

        // Strip UTF-8 BOM if present (Windows Notepad adds this)
        let content = content.strip_prefix(UTF8_BOM).unwrap_or(&content);

        let config: CapabilitiesConfig = serde_json::from_str(content)
            .with_context(|| format!("Failed to parse JSON from: {}", path.display()))?;
        Ok(config)
    }

    /// Load bundled default capabilities
    fn load_bundled() -> Result<CapabilitiesConfig> {
        serde_json::from_str(BUNDLED_CAPABILITIES).context("Failed to parse bundled capabilities")
    }

    /// Ensure the config file exists (copy bundled defaults if not)
    ///
    /// Uses atomic write (temp file + rename) for safety.
    pub fn ensure_config_file() -> Result<PathBuf> {
        let config_path = Self::get_config_file_path();

        if !config_path.exists() {
            // Ensure parent directory exists
            if let Some(parent) = config_path.parent() {
                std::fs::create_dir_all(parent).with_context(|| {
                    format!("Failed to create config directory: {}", parent.display())
                })?;
            }

            // Use atomic write: write to temp file first, then rename
            let temp_path = config_path.with_extension("json.tmp");

            // Write bundled defaults to temp file
            let mut file = std::fs::File::create(&temp_path)
                .with_context(|| format!("Failed to create temp file: {}", temp_path.display()))?;
            file.write_all(BUNDLED_CAPABILITIES.as_bytes())
                .with_context(|| format!("Failed to write temp file: {}", temp_path.display()))?;
            file.sync_all()
                .with_context(|| format!("Failed to sync temp file: {}", temp_path.display()))?;
            drop(file);

            // Atomic rename
            std::fs::rename(&temp_path, &config_path).with_context(|| {
                format!(
                    "Failed to rename {} to {}",
                    temp_path.display(),
                    config_path.display()
                )
            })?;

            tracing::info!(
                "Created default model capabilities config at: {}",
                config_path.display()
            );
        }

        Ok(config_path)
    }

    /// Get the current config source
    pub fn get_config_source(&self) -> &ConfigSource {
        &self.config_source
    }

    /// Load capabilities from configuration
    fn load_capabilities(&mut self, config: CapabilitiesConfig) -> Result<()> {
        for caps in config.models {
            let pattern = Pattern::new(&caps.pattern)
                .with_context(|| format!("Invalid pattern: {}", caps.pattern))?;

            self.capabilities.push(CompiledCapability {
                pattern,
                raw_pattern: caps.pattern.clone(),
                capabilities: caps,
            });
        }

        // Sort by priority (descending)
        self.capabilities
            .sort_by(|a, b| b.capabilities.priority.cmp(&a.capabilities.priority));

        Ok(())
    }

    /// Load user overrides from config
    fn load_user_overrides(&mut self) {
        // Try to load from goose config
        if let Ok(config) = crate::config::Config::global()
            .get_param::<HashMap<String, ModelOverride>>("model_overrides")
        {
            self.user_overrides = config;
        }
    }

    /// Resolve capabilities for a specific model
    ///
    /// Returns cached result if available, otherwise computes and caches the result.
    /// Empty model names return default capabilities with a warning log.
    pub fn resolve(&self, model_name: &str) -> ResolvedCapabilities {
        // Handle empty model name
        if model_name.is_empty() {
            tracing::warn!("Empty model name provided to capabilities::resolve()");
            return ResolvedCapabilities::default();
        }

        // Normalize for cache key (lowercase)
        let cache_key = model_name.to_lowercase();

        // Check cache first (read lock)
        if let Ok(cache) = self.cache.read() {
            if let Some(cached) = cache.get(&cache_key) {
                return cached.clone();
            }
        }

        // Normalize model name (strip common prefixes)
        let normalized = self.normalize_model_name(model_name);

        // Find matching capability definition
        let base_caps = self.find_matching_capabilities(&normalized);

        // Build resolved capabilities
        let mut resolved = self.build_resolved(model_name, &base_caps);

        // Apply user overrides (with validation)
        self.apply_user_overrides(model_name, &mut resolved, &base_caps);

        // Apply environment variable overrides
        self.apply_env_overrides(&mut resolved, &base_caps);

        // Cache the result (write lock)
        if let Ok(mut cache) = self.cache.write() {
            cache.insert(cache_key, resolved.clone());
        }

        resolved
    }

    /// Resolve and cache capabilities (deprecated - use resolve() instead)
    #[deprecated(
        since = "0.1.0",
        note = "Use resolve() instead, which now caches automatically"
    )]
    pub fn resolve_cached(&mut self, model_name: &str) -> ResolvedCapabilities {
        self.resolve(model_name)
    }

    /// Find matching capabilities for a normalized model name
    fn find_matching_capabilities(&self, model_name: &str) -> ModelCapabilities {
        for compiled in &self.capabilities {
            if compiled.pattern.matches(model_name) {
                return compiled.capabilities.clone();
            }
        }

        // No match - return defaults
        ModelCapabilities::default()
    }

    /// Normalize model name by stripping common prefixes
    fn normalize_model_name(&self, model_name: &str) -> String {
        let prefixes = ["agime-", "databricks-", "azure-", "bedrock-"];

        let mut normalized = model_name.to_lowercase();
        for prefix in prefixes {
            if let Some(stripped) = normalized.strip_prefix(prefix) {
                normalized = stripped.to_string();
                break;
            }
        }

        normalized
    }

    /// Build resolved capabilities from base capabilities
    fn build_resolved(&self, model_name: &str, caps: &ModelCapabilities) -> ResolvedCapabilities {
        let mut resolved = ResolvedCapabilities::new(model_name);

        resolved.matched_pattern = Some(caps.pattern.clone());
        resolved.provider = caps.provider.clone();

        // Thinking
        resolved.thinking_supported = caps.thinking.supported;
        resolved.thinking_type = caps.thinking.thinking_type;
        resolved.thinking_request_config = caps.thinking.request_config.clone();
        resolved.thinking_response_config = caps.thinking.response_config.clone();
        // Note: thinking_enabled is set by user/env overrides

        // Reasoning
        resolved.reasoning_supported = caps.reasoning.supported;
        if caps.reasoning.supported {
            resolved.reasoning_effort = Some(caps.reasoning.default_effort.clone());
        }
        resolved.reasoning_param = caps.reasoning.api_param.clone();
        resolved.reasoning_request_config = caps.reasoning.request_config.clone();

        // Temperature
        resolved.temperature_supported = caps.temperature.supported;
        resolved.temperature_fixed = caps.temperature.fixed_value;

        // System role
        resolved.system_role = caps.system_role.role_name.clone();

        // Headers (always headers)
        for header in &caps.beta_headers.always {
            resolved
                .headers
                .push((header.name.clone(), header.value.clone()));
        }

        // Tool format
        resolved.tools_supported = caps.tool_format.supported;
        resolved.use_max_completion_tokens = caps.tool_format.use_max_completion_tokens;
        resolved.schema_processor = caps.tool_format.schema_processor.clone();

        // Context
        resolved.context_length = caps.context_length;
        resolved.max_completion_tokens = caps.max_completion_tokens;

        resolved
    }

    /// Apply user overrides to resolved capabilities (with validation)
    fn apply_user_overrides(
        &self,
        model_name: &str,
        resolved: &mut ResolvedCapabilities,
        base_caps: &ModelCapabilities,
    ) {
        if let Some(override_config) = self.user_overrides.get(model_name) {
            // Thinking override
            if let Some(thinking) = &override_config.thinking {
                if resolved.thinking_supported {
                    resolved.thinking_enabled = thinking.enabled;
                    if let Some(budget) = thinking.budget {
                        // Validate budget range
                        let validated_budget = budget
                            .max(base_caps.thinking.min_budget.max(MIN_THINKING_BUDGET))
                            .min(MAX_THINKING_BUDGET);
                        if validated_budget != budget {
                            tracing::warn!(
                                "User thinking budget {} clamped to {} for model {}",
                                budget,
                                validated_budget,
                                model_name
                            );
                        }
                        resolved.thinking_budget = Some(validated_budget);
                    }
                }
            }

            // Reasoning override
            if let Some(reasoning) = &override_config.reasoning {
                if resolved.reasoning_supported {
                    if let Some(effort) = &reasoning.effort {
                        // Validate effort is in allowed levels
                        if base_caps.reasoning.effort_levels.contains(effort) {
                            resolved.reasoning_effort = Some(effort.clone());
                        } else {
                            tracing::warn!(
                                "Invalid reasoning effort '{}' for model {}, using default",
                                effort,
                                model_name
                            );
                        }
                    }
                }
            }

            // Temperature override
            if let Some(_temp) = override_config.temperature {
                // Note: this is the user's preferred temperature,
                // actual application happens in the provider
            }
        }
    }

    /// Apply environment variable and config overrides for backward compatibility
    fn apply_env_overrides(&self, resolved: &mut ResolvedCapabilities, caps: &ModelCapabilities) {
        // Check new AGIME_ config first, then legacy GOOSE_ config, then env var
        let config_enabled = crate::config::Config::global()
            .get_param::<bool>("AGIME_THINKING_ENABLED")
            .or_else(|_| {
                crate::config::Config::global().get_param::<bool>("GOOSE_THINKING_ENABLED")
            })
            .unwrap_or(false);
        let env_enabled = std::env::var("CLAUDE_THINKING_ENABLED").is_ok();

        if config_enabled || env_enabled {
            if resolved.thinking_supported {
                resolved.thinking_enabled = true;

                // Get budget from new AGIME_ config first, then legacy GOOSE_ config, then env var, then default
                let budget = crate::config::Config::global()
                    .get_param::<u32>("AGIME_THINKING_BUDGET")
                    .ok()
                    .or_else(|| {
                        crate::config::Config::global()
                            .get_param::<u32>("GOOSE_THINKING_BUDGET")
                            .ok()
                    })
                    .or_else(|| {
                        std::env::var("CLAUDE_THINKING_BUDGET")
                            .ok()
                            .and_then(|s| s.parse::<u32>().ok())
                    })
                    .unwrap_or(caps.thinking.default_budget)
                    .max(caps.thinking.min_budget);
                resolved.thinking_budget = Some(budget);

                // Add thinking-specific headers
                for header in &caps.beta_headers.with_thinking {
                    resolved
                        .headers
                        .push((header.name.clone(), header.value.clone()));
                }

                // Check if temperature should be disabled
                if caps.temperature.disabled_with_thinking {
                    resolved.temperature_disabled_by_thinking = true;
                }
            }
        }
    }

    /// Reload configuration from disk
    ///
    /// Clears cache, reloads capabilities from file (or bundled), and reloads user overrides.
    pub fn reload(&mut self) -> Result<()> {
        // Clear cache
        if let Ok(mut cache) = self.cache.write() {
            cache.clear();
        }

        // Clear existing capabilities
        self.capabilities.clear();

        // Try to load from user config file first
        let config_path = Self::get_config_file_path();
        let (config, source) = if config_path.exists() {
            match Self::load_from_file(&config_path) {
                Ok(cfg) => {
                    tracing::info!(
                        "Reloaded model capabilities from file: {}",
                        config_path.display()
                    );
                    (cfg, ConfigSource::File(config_path))
                }
                Err(e) => {
                    tracing::warn!(
                        "Failed to reload capabilities from {}: {}. Using bundled defaults.",
                        config_path.display(),
                        e
                    );
                    (Self::load_bundled()?, ConfigSource::Bundled)
                }
            }
        } else {
            tracing::info!("No config file found, using bundled defaults");
            (Self::load_bundled()?, ConfigSource::Bundled)
        };

        self.config_source = source;
        self.load_capabilities(config)?;

        // Reload user overrides
        self.load_user_overrides();

        Ok(())
    }

    /// Clear the resolution cache
    pub fn clear_cache(&mut self) {
        if let Ok(mut cache) = self.cache.write() {
            cache.clear();
        }
    }

    /// Check if a model supports thinking
    pub fn supports_thinking(&self, model_name: &str) -> bool {
        self.resolve(model_name).thinking_supported
    }

    /// Check if thinking is enabled for a model
    pub fn is_thinking_enabled(&self, model_name: &str) -> bool {
        self.resolve(model_name).thinking_enabled
    }

    /// Get thinking budget for a model
    pub fn get_thinking_budget(&self, model_name: &str) -> Option<u32> {
        let resolved = self.resolve(model_name);
        if resolved.thinking_enabled {
            resolved.thinking_budget
        } else {
            None
        }
    }

    /// Check if a model supports reasoning effort
    pub fn supports_reasoning(&self, model_name: &str) -> bool {
        self.resolve(model_name).reasoning_supported
    }

    /// Get reasoning effort for a model
    pub fn get_reasoning_effort(&self, model_name: &str) -> Option<String> {
        self.resolve(model_name).reasoning_effort.clone()
    }

    /// Get system role name for a model
    pub fn get_system_role(&self, model_name: &str) -> String {
        self.resolve(model_name).system_role.clone()
    }

    /// Get headers for a model request
    pub fn get_headers(&self, model_name: &str) -> Vec<(String, String)> {
        self.resolve(model_name).headers.clone()
    }

    /// Check if temperature is supported for a model
    pub fn supports_temperature(&self, model_name: &str) -> bool {
        self.resolve(model_name).effective_temperature_supported()
    }

    /// Check if tools are supported for a model
    pub fn supports_tools(&self, model_name: &str) -> bool {
        self.resolve(model_name).tools_supported
    }

    /// Get schema processor for tools
    pub fn get_schema_processor(&self, model_name: &str) -> Option<String> {
        self.resolve(model_name).schema_processor.clone()
    }

    /// Infer provider from model name
    pub fn infer_provider(&self, model_name: &str) -> Option<String> {
        self.resolve(model_name).provider.clone()
    }

    /// Get all models that support thinking
    pub fn get_thinking_models(&self) -> Vec<String> {
        self.capabilities
            .iter()
            .filter(|c| c.capabilities.thinking.supported)
            .map(|c| c.raw_pattern.clone())
            .collect()
    }

    /// Get all models that support reasoning
    pub fn get_reasoning_models(&self) -> Vec<String> {
        self.capabilities
            .iter()
            .filter(|c| c.capabilities.reasoning.supported)
            .map(|c| c.raw_pattern.clone())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pattern_matching() {
        let pattern = Pattern::new("claude-3-7-*").unwrap();
        assert!(pattern.matches("claude-3-7-sonnet-20241022"));
        assert!(pattern.matches("claude-3-7-sonnet-latest"));
        assert!(!pattern.matches("claude-3-5-sonnet"));
    }

    #[test]
    fn test_normalize_model_name() {
        let registry = CapabilityRegistry::new().unwrap();
        assert_eq!(
            registry.normalize_model_name("agime-claude-3-7"),
            "claude-3-7"
        );
        assert_eq!(registry.normalize_model_name("databricks-gpt-4"), "gpt-4");
        assert_eq!(registry.normalize_model_name("claude-3-7"), "claude-3-7");
    }

    #[test]
    fn test_empty_model_name() {
        let registry = CapabilityRegistry::new().unwrap();
        let caps = registry.resolve("");
        // Should return default capabilities
        assert!(!caps.thinking_supported);
        assert!(!caps.reasoning_supported);
    }

    #[test]
    fn test_cache_case_insensitive() {
        let registry = CapabilityRegistry::new().unwrap();
        let caps1 = registry.resolve("Claude-3-7-Sonnet-20250219");
        let caps2 = registry.resolve("claude-3-7-sonnet-20250219");
        // Both should return the same capabilities (case insensitive cache)
        assert_eq!(caps1.thinking_supported, caps2.thinking_supported);
    }

    #[test]
    fn test_cache_actually_caches() {
        let registry = CapabilityRegistry::new().unwrap();
        // First call should compute
        let _caps1 = registry.resolve("claude-3-7-sonnet-20250219");
        // Second call should hit cache
        let caps2 = registry.resolve("claude-3-7-sonnet-20250219");
        assert!(caps2.thinking_supported);

        // Check cache contains the entry
        let cache = registry.cache.read().unwrap();
        assert!(cache.contains_key("claude-3-7-sonnet-20250219"));
    }

    #[test]
    fn test_budget_validation() {
        // Budget should be clamped between MIN and MAX
        let min = MIN_THINKING_BUDGET;
        let max = MAX_THINKING_BUDGET;

        // Test clamping
        assert_eq!(500_u32.max(min).min(max), 1024);
        assert_eq!(200000_u32.max(min).min(max), 100000);
        assert_eq!(16000_u32.max(min).min(max), 16000);
    }
}
