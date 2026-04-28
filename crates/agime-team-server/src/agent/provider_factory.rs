//! Provider factory for creating agime Provider instances from TeamAgent config.
//!
//! Maps TeamAgent's (api_format, api_url, api_key, model) to the appropriate
//! Provider implementation (AnthropicProvider or OpenAiProvider).

use agime::model::{CacheEditMode, ModelConfig, PromptCachingMode};
use agime::providers::anthropic::AnthropicProvider;
use agime::providers::api_client::{ApiClient, AuthMethod};
use agime::providers::base::Provider;
use agime::providers::litellm::LiteLLMProvider;
use agime::providers::openai::OpenAiProvider;
use agime_team::models::{ApiFormat, RuntimeOptimizationMode, TeamAgent};
use anyhow::{anyhow, Result};
use std::sync::Arc;

/// Create a Provider instance from a TeamAgent's configuration.
///
/// - `ApiFormat::Anthropic` → `AnthropicProvider::new()`
/// - `ApiFormat::OpenAI` → `OpenAiProvider::new()` or `LiteLLMProvider::from_env()` in compat mode
/// - `ApiFormat::LiteLLM` → `LiteLLMProvider::from_env()`
/// - `ApiFormat::Local` → OpenAI-compatible local endpoint, without auth when no key is set
pub fn create_provider_for_agent(agent: &TeamAgent) -> Result<Arc<dyn Provider>> {
    let api_key = agent.api_key.as_deref();

    let model_name = agent.model.as_deref().unwrap_or(match agent.api_format {
        ApiFormat::Anthropic => "claude-sonnet-4-5",
        ApiFormat::OpenAI => "gpt-4o",
        ApiFormat::LiteLLM => "gpt-4o-mini",
        ApiFormat::Local => "llama2",
    });

    let model = ModelConfig::new(model_name)
        .map_err(|e| anyhow!("Invalid model '{}': {}", model_name, e))?
        .with_temperature(agent.temperature)
        .with_max_tokens(agent.max_tokens)
        .with_context_limit(agent.context_limit)
        .with_thinking(Some(agent.thinking_enabled), agent.thinking_budget)
        .with_reasoning_effort(agent.reasoning_effort.clone())
        .with_output_reserve_tokens(agent.output_reserve_tokens)
        .with_auto_compact_threshold(agent.auto_compact_threshold)
        .with_prompt_caching_mode(map_prompt_caching_mode(agent.prompt_caching_mode))
        .with_cache_edit_mode(map_cache_edit_mode(agent.cache_edit_mode));

    match agent.api_format {
        ApiFormat::Anthropic => {
            let api_key =
                api_key.ok_or_else(|| anyhow!("API key not configured for agent '{}'", agent.name))?;
            let provider = create_anthropic_provider(agent, api_key, model)?;
            Ok(Arc::new(provider))
        }
        ApiFormat::OpenAI => {
            if should_use_litellm_provider(agent) {
                let provider = create_litellm_provider(agent, model)?;
                Ok(Arc::new(provider))
            } else {
                let api_key = api_key
                    .ok_or_else(|| anyhow!("API key not configured for agent '{}'", agent.name))?;
                let provider = create_openai_provider(agent, api_key, model)?;
                Ok(Arc::new(provider))
            }
        }
        ApiFormat::LiteLLM => {
            let provider = create_litellm_provider(agent, model)?;
            Ok(Arc::new(provider))
        }
        ApiFormat::Local => {
            let provider = create_local_openai_compatible_provider(agent, api_key, model)?;
            Ok(Arc::new(provider))
        }
    }
}

fn should_use_litellm_provider(agent: &TeamAgent) -> bool {
    if matches!(agent.api_format, ApiFormat::LiteLLM) {
        return true;
    }
    let Some(url) = agent.api_url.as_deref() else {
        return false;
    };
    let normalized = url.trim().to_ascii_lowercase();
    normalized.contains("litellm")
        || normalized.contains("api.litellm.ai")
        || normalized.contains("localhost:4000")
        || normalized.contains("127.0.0.1:4000")
}

fn map_prompt_caching_mode(mode: RuntimeOptimizationMode) -> PromptCachingMode {
    match mode {
        RuntimeOptimizationMode::Auto => PromptCachingMode::Auto,
        RuntimeOptimizationMode::Off => PromptCachingMode::Off,
        RuntimeOptimizationMode::Prefer => PromptCachingMode::Prefer,
    }
}

fn map_cache_edit_mode(mode: RuntimeOptimizationMode) -> CacheEditMode {
    match mode {
        RuntimeOptimizationMode::Auto => CacheEditMode::Auto,
        RuntimeOptimizationMode::Off => CacheEditMode::Off,
        RuntimeOptimizationMode::Prefer => CacheEditMode::Prefer,
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

fn create_local_openai_compatible_provider(
    agent: &TeamAgent,
    api_key: Option<&str>,
    model: ModelConfig,
) -> Result<OpenAiProvider> {
    let raw_url = agent
        .api_url
        .as_deref()
        .unwrap_or("http://127.0.0.1:11434/v1/chat/completions")
        .trim();
    let (host, base_path) = split_local_openai_url(raw_url)?;
    let auth = api_key
        .filter(|value| !value.trim().is_empty())
        .map(|value| AuthMethod::BearerToken(value.to_string()))
        .unwrap_or(AuthMethod::None);
    let api_client = ApiClient::new(host, auth)?;
    Ok(OpenAiProvider::new_with_base_path(
        api_client,
        model,
        base_path,
        true,
        "local-openai-compatible",
    ))
}

fn split_local_openai_url(raw_url: &str) -> Result<(String, String)> {
    let url = reqwest::Url::parse(raw_url)
        .map_err(|error| anyhow!("Invalid local API URL '{}': {}", raw_url, error))?;
    let host = if let Some(port) = url.port() {
        format!(
            "{}://{}:{}",
            url.scheme(),
            url.host_str().unwrap_or_default(),
            port
        )
    } else {
        format!("{}://{}", url.scheme(), url.host_str().unwrap_or_default())
    };
    let path = url.path().trim_start_matches('/').trim_end_matches('/');
    let base_path = if path.is_empty() {
        "v1/chat/completions".to_string()
    } else if path == "v1" {
        "v1/chat/completions".to_string()
    } else {
        path.to_string()
    };
    Ok((host, base_path))
}

fn create_litellm_provider(agent: &TeamAgent, model: ModelConfig) -> Result<LiteLLMProvider> {
    if let Some(base_url) = agent
        .api_url
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        return LiteLLMProvider::from_custom_endpoint(
            model,
            base_url,
            agent.api_key.as_deref(),
            None,
        );
    }
    futures::executor::block_on(LiteLLMProvider::from_env(model))
}

#[cfg(test)]
mod tests {
    use super::split_local_openai_url;

    #[test]
    fn local_url_defaults_to_openai_chat_completions_path() {
        let (host, path) = split_local_openai_url("http://127.0.0.1:11434").unwrap();
        assert_eq!(host, "http://127.0.0.1:11434");
        assert_eq!(path, "v1/chat/completions");
    }

    #[test]
    fn local_url_accepts_explicit_v1_base() {
        let (host, path) = split_local_openai_url("http://localhost:11434/v1").unwrap();
        assert_eq!(host, "http://localhost:11434");
        assert_eq!(path, "v1/chat/completions");
    }

    #[test]
    fn local_url_preserves_explicit_completion_path() {
        let (host, path) =
            split_local_openai_url("http://localhost:11434/v1/chat/completions").unwrap();
        assert_eq!(host, "http://localhost:11434");
        assert_eq!(path, "v1/chat/completions");
    }
}
