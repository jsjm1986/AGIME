//! Provider factory (team-server side).
//!
//! The actual factory lives in [`agime_runtime::provider_factory`]; this file
//! adapts the team-server's `TeamAgent` to a `HostProviderConfig` so call
//! sites keep their existing `&TeamAgent` signature.

use agime::providers::base::Provider;
use agime_team::models::{ApiFormat, RuntimeOptimizationMode, TeamAgent};
use anyhow::Result;
use std::sync::Arc;

use agime_runtime::provider_factory::{
    create_provider_for_config, HostApiFormat, HostProviderConfig, HostRuntimeOptimizationMode,
};

fn map_api_format(value: ApiFormat) -> HostApiFormat {
    match value {
        ApiFormat::OpenAI => HostApiFormat::OpenAI,
        ApiFormat::Anthropic => HostApiFormat::Anthropic,
        ApiFormat::LiteLLM => HostApiFormat::LiteLLM,
        ApiFormat::Local => HostApiFormat::Local,
    }
}

fn map_optimization_mode(value: RuntimeOptimizationMode) -> HostRuntimeOptimizationMode {
    match value {
        RuntimeOptimizationMode::Auto => HostRuntimeOptimizationMode::Auto,
        RuntimeOptimizationMode::Off => HostRuntimeOptimizationMode::Off,
        RuntimeOptimizationMode::Prefer => HostRuntimeOptimizationMode::Prefer,
    }
}

fn host_config_from_agent(agent: &TeamAgent) -> HostProviderConfig {
    HostProviderConfig {
        name: agent.name.clone(),
        api_format: map_api_format(agent.api_format),
        api_url: agent.api_url.clone(),
        api_key: agent.api_key.clone(),
        model: agent.model.clone(),
        temperature: agent.temperature,
        max_tokens: agent.max_tokens,
        context_limit: agent.context_limit,
        thinking_enabled: agent.thinking_enabled,
        thinking_budget: agent.thinking_budget,
        reasoning_effort: agent.reasoning_effort.clone(),
        output_reserve_tokens: agent.output_reserve_tokens,
        auto_compact_threshold: agent.auto_compact_threshold,
        supports_multimodal: agent.supports_multimodal,
        prompt_caching_mode: map_optimization_mode(agent.prompt_caching_mode),
        cache_edit_mode: map_optimization_mode(agent.cache_edit_mode),
    }
}

/// Create a Provider instance from a TeamAgent's configuration.
pub fn create_provider_for_agent(agent: &TeamAgent) -> Result<Arc<dyn Provider>> {
    let cfg = host_config_from_agent(agent);
    create_provider_for_config(&cfg)
}
