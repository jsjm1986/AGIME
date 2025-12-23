//! Runtime capability resolution.
//!
//! This module provides the resolved capability state for a specific model,
//! combining registry defaults with user/environment overrides.

use serde::{Deserialize, Serialize};

use super::types::{
    ReasoningRequestConfig, ThinkingRequestConfig, ThinkingResponseConfig, ThinkingType,
};

/// Resolved capabilities for a specific model at runtime.
/// This combines registry defaults with user/environment overrides.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolvedCapabilities {
    /// The original model name
    pub model_name: String,

    /// The matched pattern from registry (if any)
    pub matched_pattern: Option<String>,

    /// The inferred provider
    pub provider: Option<String>,

    // ===== Thinking =====
    /// Whether thinking mode is supported by this model
    pub thinking_supported: bool,

    /// Whether thinking mode is currently enabled
    pub thinking_enabled: bool,

    /// Thinking type (api or tag)
    pub thinking_type: ThinkingType,

    /// Thinking budget tokens (if thinking enabled)
    pub thinking_budget: Option<u32>,

    /// Configuration for adding thinking params to requests
    pub thinking_request_config: Option<ThinkingRequestConfig>,

    /// Configuration for parsing thinking from responses
    pub thinking_response_config: Option<ThinkingResponseConfig>,

    // ===== Reasoning =====
    /// Whether reasoning effort is supported
    pub reasoning_supported: bool,

    /// Reasoning effort level (if applicable)
    pub reasoning_effort: Option<String>,

    /// API parameter name for reasoning effort
    pub reasoning_param: String,

    /// Configuration for adding reasoning params to requests
    pub reasoning_request_config: Option<ReasoningRequestConfig>,

    // ===== Temperature =====
    /// Whether temperature is supported
    pub temperature_supported: bool,

    /// Fixed temperature value (if set)
    pub temperature_fixed: Option<f32>,

    /// Whether temperature is disabled due to thinking being enabled
    pub temperature_disabled_by_thinking: bool,

    // ===== System Role =====
    /// System role name to use ("system" or "developer")
    pub system_role: String,

    // ===== Headers =====
    /// Headers to include in requests
    pub headers: Vec<(String, String)>,

    // ===== Tool Format =====
    /// Whether tools are supported
    pub tools_supported: bool,

    /// Whether to use max_completion_tokens instead of max_tokens
    pub use_max_completion_tokens: bool,

    /// Schema processor to use for tools (e.g., "gemini")
    pub schema_processor: Option<String>,

    // ===== Context =====
    /// Context window size
    pub context_length: Option<usize>,

    /// Maximum completion tokens
    pub max_completion_tokens: Option<usize>,
}

impl Default for ResolvedCapabilities {
    fn default() -> Self {
        Self {
            model_name: String::new(),
            matched_pattern: None,
            provider: None,
            thinking_supported: false,
            thinking_enabled: false,
            thinking_type: ThinkingType::None,
            thinking_budget: None,
            thinking_request_config: None,
            thinking_response_config: None,
            reasoning_supported: false,
            reasoning_effort: None,
            reasoning_param: "reasoning_effort".into(),
            reasoning_request_config: None,
            temperature_supported: true,
            temperature_fixed: None,
            temperature_disabled_by_thinking: false,
            system_role: "system".into(),
            headers: Vec::new(),
            tools_supported: true,
            use_max_completion_tokens: false,
            schema_processor: None,
            context_length: None,
            max_completion_tokens: None,
        }
    }
}

impl ResolvedCapabilities {
    /// Create a new resolved capabilities for the given model name
    pub fn new(model_name: impl Into<String>) -> Self {
        Self {
            model_name: model_name.into(),
            ..Default::default()
        }
    }

    /// Check if this model uses API-based thinking (Claude style)
    pub fn uses_api_thinking(&self) -> bool {
        self.thinking_enabled && self.thinking_type == ThinkingType::Api
    }

    /// Check if this model uses tag-based thinking (DeepSeek style)
    pub fn uses_tag_thinking(&self) -> bool {
        self.thinking_enabled && self.thinking_type == ThinkingType::Tag
    }

    /// Get the effective temperature support status
    pub fn effective_temperature_supported(&self) -> bool {
        self.temperature_supported && !self.temperature_disabled_by_thinking
    }

    /// Check if this is a reasoning model (O-series, etc.)
    pub fn is_reasoning_model(&self) -> bool {
        self.reasoning_supported && self.reasoning_effort.is_some()
    }
}
