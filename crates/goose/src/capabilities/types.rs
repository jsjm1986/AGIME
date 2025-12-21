//! Core types for model capability definitions.
//!
//! This module defines the data structures used to describe model capabilities
//! in a configuration-driven way, replacing hardcoded model checks.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Thinking mode type - how the model exposes thinking/reasoning
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ThinkingType {
    /// Model does not support thinking mode
    #[default]
    None,
    /// Claude-style API parameter thinking (structured response)
    Api,
    /// Tag-based thinking (<think>...</think> in response)
    Tag,
}

/// Thinking mode configuration
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ThinkingConfig {
    /// Whether thinking mode is supported by this model
    #[serde(default)]
    pub supported: bool,

    /// Type of thinking: "api" (Claude) or "tag" (DeepSeek/Qwen)
    #[serde(default)]
    pub thinking_type: ThinkingType,

    /// API parameter name for enabling thinking (e.g., "thinking")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_param: Option<String>,

    /// Default budget tokens for thinking
    #[serde(default = "default_thinking_budget")]
    pub default_budget: u32,

    /// Minimum budget tokens
    #[serde(default = "default_min_budget")]
    pub min_budget: u32,

    /// Maximum budget tokens (None = unlimited/use max_tokens)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_budget: Option<u32>,

    /// Configuration for adding thinking parameters to requests
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_config: Option<ThinkingRequestConfig>,

    /// Configuration for parsing thinking content from responses
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_config: Option<ThinkingResponseConfig>,
}

fn default_thinking_budget() -> u32 {
    16000
}
fn default_min_budget() -> u32 {
    1024
}

/// Method for adding thinking parameters to requests
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum RequestMethod {
    /// Add as top-level API parameter (Claude style: {thinking: {type: enabled}})
    #[default]
    Parameter,
    /// Add in extra_body/additional_params (Qwen/DeepSeek style)
    ExtraBody,
    /// Add as request header
    Header,
    /// Automatically determine based on provider
    Auto,
}

/// Response type for thinking content
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ResponseType {
    /// Content block in message (Claude style: {type: "thinking", thinking: "..."})
    #[default]
    ContentBlock,
    /// Separate field in response (DeepSeek/Qwen: reasoning_content)
    Field,
    /// Inline tag in text (<think>...</think>)
    Tag,
}

/// Configuration for how to add thinking parameters to API requests
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ThinkingRequestConfig {
    /// Method to use for adding parameters
    #[serde(default)]
    pub method: RequestMethod,

    /// JSON path for the parameter (e.g., "thinking" or "extra_body.enable_thinking")
    #[serde(default)]
    pub param_path: String,

    /// Template for parameter value - supports ${budget} placeholder
    /// e.g., {"type": "enabled", "budget_tokens": "${budget}"}
    #[serde(skip_serializing_if = "Option::is_none")]
    pub param_template: Option<serde_json::Value>,

    /// Simple boolean or value when template not needed
    #[serde(skip_serializing_if = "Option::is_none")]
    pub param_value: Option<serde_json::Value>,

    /// How to adjust max_tokens when thinking is enabled
    /// "add_budget" = max_tokens + budget, "none" = no adjustment
    #[serde(default)]
    pub max_tokens_adjustment: String,
}

/// Configuration for how to parse thinking content from API responses
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ThinkingResponseConfig {
    /// Type of response format
    #[serde(default)]
    pub response_type: ResponseType,

    /// For ContentBlock: the block type to look for (e.g., "thinking")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub block_type: Option<String>,

    /// Field name containing the thinking content
    /// For ContentBlock: field within block (e.g., "thinking")
    /// For Field: field in message (e.g., "reasoning_content")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_field: Option<String>,

    /// Field name for thinking signature (Claude specific)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signature_field: Option<String>,

    /// Regex pattern for extracting tag-based thinking
    /// e.g., "<think>([\\s\\S]*?)</think>"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tag_pattern: Option<String>,

    /// Fallback tag pattern if primary method fails
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fallback_tag_pattern: Option<String>,
}

/// Configuration for reasoning effort API parameters
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ReasoningRequestConfig {
    /// Method to use for adding parameters
    #[serde(default)]
    pub method: RequestMethod,

    /// JSON path for the parameter (e.g., "reasoning.effort" or "reasoning_effort")
    #[serde(default)]
    pub param_path: String,

    /// Whether the parameter value is the effort level directly
    #[serde(default = "default_true")]
    pub use_effort_level: bool,
}

/// Reasoning effort configuration (for O-series and similar models)
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ReasoningConfig {
    /// Whether reasoning effort is supported
    #[serde(default)]
    pub supported: bool,

    /// Valid effort levels
    #[serde(default = "default_effort_levels")]
    pub effort_levels: Vec<String>,

    /// Default effort level
    #[serde(default = "default_effort")]
    pub default_effort: String,

    /// API parameter name (e.g., "reasoning_effort")
    #[serde(default = "default_reasoning_param")]
    pub api_param: String,

    /// Configuration for adding reasoning parameters to requests
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_config: Option<ReasoningRequestConfig>,
}

fn default_effort_levels() -> Vec<String> {
    vec!["low".into(), "medium".into(), "high".into()]
}
fn default_effort() -> String {
    "medium".into()
}
fn default_reasoning_param() -> String {
    "reasoning_effort".into()
}

/// Temperature constraints
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemperatureConfig {
    /// Whether temperature parameter is supported
    #[serde(default = "default_true")]
    pub supported: bool,

    /// Fixed temperature value (overrides user setting)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fixed_value: Option<f32>,

    /// Minimum allowed temperature
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_value: Option<f32>,

    /// Maximum allowed temperature
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_value: Option<f32>,

    /// Whether to disable temperature when thinking is enabled
    #[serde(default)]
    pub disabled_with_thinking: bool,
}

impl Default for TemperatureConfig {
    fn default() -> Self {
        Self {
            supported: true,
            fixed_value: None,
            min_value: None,
            max_value: None,
            disabled_with_thinking: false,
        }
    }
}

fn default_true() -> bool {
    true
}

/// System role configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemRoleConfig {
    /// The role name to use for system messages
    #[serde(default = "default_system_role")]
    pub role_name: String,
}

impl Default for SystemRoleConfig {
    fn default() -> Self {
        Self {
            role_name: "system".into(),
        }
    }
}

fn default_system_role() -> String {
    "system".into()
}

/// A single header configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HeaderConfig {
    pub name: String,
    pub value: String,
}

/// Beta headers configuration
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BetaHeadersConfig {
    /// Headers to always include for this model
    #[serde(default)]
    pub always: Vec<HeaderConfig>,

    /// Headers to include when thinking is enabled
    #[serde(default)]
    pub with_thinking: Vec<HeaderConfig>,
}

/// Tool format configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolFormatConfig {
    /// Whether tools/function calling is supported
    #[serde(default = "default_true")]
    pub supported: bool,

    /// Special schema processing required (e.g., "gemini", "anthropic")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schema_processor: Option<String>,

    /// Whether to use max_completion_tokens vs max_tokens
    #[serde(default)]
    pub use_max_completion_tokens: bool,
}

impl Default for ToolFormatConfig {
    fn default() -> Self {
        Self {
            supported: true,
            schema_processor: None,
            use_max_completion_tokens: false,
        }
    }
}

/// Complete model capabilities specification
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelCapabilities {
    /// Model ID pattern (supports wildcards: "claude-3-7-*", "o1-*", etc.)
    pub pattern: String,

    /// Human-readable description
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    /// The canonical provider for this model family
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,

    /// Thinking mode capabilities
    #[serde(default)]
    pub thinking: ThinkingConfig,

    /// Reasoning effort capabilities
    #[serde(default)]
    pub reasoning: ReasoningConfig,

    /// Temperature constraints
    #[serde(default)]
    pub temperature: TemperatureConfig,

    /// System role configuration
    #[serde(default)]
    pub system_role: SystemRoleConfig,

    /// Beta headers configuration
    #[serde(default)]
    pub beta_headers: BetaHeadersConfig,

    /// Tool format configuration
    #[serde(default)]
    pub tool_format: ToolFormatConfig,

    /// Context window size
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_length: Option<usize>,

    /// Maximum completion tokens
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_completion_tokens: Option<usize>,

    /// Priority (higher = matched first when multiple patterns match)
    #[serde(default)]
    pub priority: i32,

    /// Additional custom properties for extensibility
    #[serde(default)]
    pub extra: HashMap<String, serde_json::Value>,
}

impl Default for ModelCapabilities {
    fn default() -> Self {
        Self {
            pattern: "*".into(),
            description: None,
            provider: None,
            thinking: ThinkingConfig::default(),
            reasoning: ReasoningConfig::default(),
            temperature: TemperatureConfig::default(),
            system_role: SystemRoleConfig::default(),
            beta_headers: BetaHeadersConfig::default(),
            tool_format: ToolFormatConfig::default(),
            context_length: None,
            max_completion_tokens: None,
            priority: 0,
            extra: HashMap::new(),
        }
    }
}

/// Configuration file root structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilitiesConfig {
    /// Configuration version
    #[serde(default = "default_version")]
    pub version: u32,

    /// Model capability definitions
    #[serde(default)]
    pub models: Vec<ModelCapabilities>,
}

fn default_version() -> u32 {
    1
}

impl Default for CapabilitiesConfig {
    fn default() -> Self {
        Self {
            version: 1,
            models: Vec::new(),
        }
    }
}

/// User override for a specific model's capabilities
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ModelOverride {
    /// Thinking configuration override
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thinking: Option<ThinkingOverride>,

    /// Reasoning configuration override
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<ReasoningOverride>,

    /// Temperature override
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
}

/// User override for thinking mode
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ThinkingOverride {
    /// Whether thinking is enabled (user preference)
    #[serde(default)]
    pub enabled: bool,

    /// Custom budget
    #[serde(skip_serializing_if = "Option::is_none")]
    pub budget: Option<u32>,
}

/// User override for reasoning effort
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ReasoningOverride {
    /// Reasoning effort level
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
}
