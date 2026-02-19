//! Provider factory for creating agime Provider instances from TeamAgent config.
//!
//! Maps TeamAgent's (api_format, api_url, api_key, model) to the appropriate
//! Provider implementation (AnthropicProvider or OpenAiProvider).

use agime::model::ModelConfig;
use agime::providers::anthropic::AnthropicProvider;
use agime::providers::api_client::{ApiClient, AuthMethod};
use agime::providers::base::Provider;
use agime::providers::openai::OpenAiProvider;
use agime_team::models::{ApiFormat, TeamAgent};
use anyhow::{anyhow, Result};
use std::sync::Arc;

/// Create a Provider instance from a TeamAgent's configuration.
///
/// - `ApiFormat::Anthropic` → `AnthropicProvider::new()`
/// - `ApiFormat::OpenAI` → `OpenAiProvider::new()`
/// - `ApiFormat::Local` → returns error (Local uses direct HTTP, not Provider)
pub fn create_provider_for_agent(agent: &TeamAgent) -> Result<Arc<dyn Provider>> {
    let api_key = agent
        .api_key
        .as_deref()
        .ok_or_else(|| anyhow!("API key not configured for agent '{}'", agent.name))?;

    let model_name = agent.model.as_deref().unwrap_or(match agent.api_format {
        ApiFormat::Anthropic => "claude-sonnet-4-5",
        ApiFormat::OpenAI => "gpt-4o",
        ApiFormat::Local => "llama2",
    });

    let model = ModelConfig::new(model_name)
        .map_err(|e| anyhow!("Invalid model '{}': {}", model_name, e))?
        .with_temperature(agent.temperature)
        .with_max_tokens(agent.max_tokens)
        .with_context_limit(agent.context_limit);

    match agent.api_format {
        ApiFormat::Anthropic => {
            let provider = create_anthropic_provider(agent, api_key, model)?;
            Ok(Arc::new(provider))
        }
        ApiFormat::OpenAI => {
            let provider = create_openai_provider(agent, api_key, model)?;
            Ok(Arc::new(provider))
        }
        ApiFormat::Local => Err(anyhow!(
            "Local API format does not use Provider abstraction"
        )),
    }
}

fn create_anthropic_provider(
    agent: &TeamAgent,
    api_key: &str,
    model: ModelConfig,
) -> Result<AnthropicProvider> {
    let base_url = agent
        .api_url
        .as_deref()
        .unwrap_or("https://api.anthropic.com");

    let is_volcengine = base_url.contains("ark.cn-beijing.volces.com");

    // Volcengine uses Bearer token, native Anthropic uses x-api-key
    let auth = if is_volcengine {
        AuthMethod::BearerToken(api_key.to_string())
    } else {
        AuthMethod::ApiKey {
            header_name: "x-api-key".to_string(),
            key: api_key.to_string(),
        }
    };

    let mut api_client = ApiClient::new(base_url.to_string(), auth)?;

    if !is_volcengine {
        api_client = api_client.with_header("anthropic-version", "2023-06-01")?;
    }

    Ok(AnthropicProvider::new(api_client, model))
}

fn create_openai_provider(
    agent: &TeamAgent,
    api_key: &str,
    model: ModelConfig,
) -> Result<OpenAiProvider> {
    let base_url = agent.api_url.as_deref().unwrap_or("https://api.openai.com");

    let auth = AuthMethod::BearerToken(api_key.to_string());
    let api_client = ApiClient::new(base_url.to_string(), auth)?;

    Ok(OpenAiProvider::new(api_client, model))
}
