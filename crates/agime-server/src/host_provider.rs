//! Provider factory (desktop side, value-type driven).
//!
//! Copied from `agime-runtime::provider_factory` to keep the desktop crate
//! free of `agime-runtime` as a dependency (per the dual-track plan). The
//! desktop's existing registry-based `agime::providers::create(name, model)`
//! continues to handle the 40+ pluggable providers that read API keys / URLs
//! from env + Config; this module fills the *per-session override* gap by
//! accepting a [`HostProviderConfig`] value object whose fields are populated
//! from a session record (Step 5).
//!
//! SOURCE: crates/agime-runtime/src/provider_factory.rs (mirror of
//! crates/agime-team-server/src/agent/provider_factory.rs at commit
//! 961109f). Keep in sync manually — see CLAUDE.md long-term maintenance
//! strategy.
//!
//! Coverage: Anthropic / OpenAI (incl. LiteLLM redirect) / LiteLLM / Local
//! OpenAI-compatible. Other providers (Google / Ollama / Bedrock / Tetrate /
//! Databricks / etc.) continue to flow through `agime::providers::create`.

#![allow(dead_code)]

use std::sync::Arc;

use agime::model::{CacheEditMode, ModelConfig, PromptCachingMode};
use agime::providers::anthropic::AnthropicProvider;
use agime::providers::api_client::{ApiClient, AuthMethod};
use agime::providers::base::Provider;
use agime::providers::litellm::LiteLLMProvider;
use agime::providers::openai::OpenAiProvider;
use anyhow::{anyhow, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HostApiFormat {
    #[default]
    OpenAI,
    Anthropic,
    LiteLLM,
    Local,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HostRuntimeOptimizationMode {
    #[default]
    Auto,
    Off,
    Prefer,
}

#[derive(Debug, Clone)]
pub struct HostProviderConfig {
    pub name: String,
    pub api_format: HostApiFormat,
    pub api_url: Option<String>,
    pub api_key: Option<String>,
    pub model: Option<String>,
    pub temperature: Option<f32>,
    pub max_tokens: Option<i32>,
    pub context_limit: Option<usize>,
    pub thinking_enabled: bool,
    pub thinking_budget: Option<u32>,
    pub reasoning_effort: Option<String>,
    pub output_reserve_tokens: Option<usize>,
    pub auto_compact_threshold: Option<f64>,
    pub supports_multimodal: bool,
    pub prompt_caching_mode: HostRuntimeOptimizationMode,
    pub cache_edit_mode: HostRuntimeOptimizationMode,
}

impl Default for HostProviderConfig {
    fn default() -> Self {
        Self {
            name: String::new(),
            api_format: HostApiFormat::default(),
            api_url: None,
            api_key: None,
            model: None,
            temperature: None,
            max_tokens: None,
            context_limit: None,
            thinking_enabled: false,
            thinking_budget: None,
            reasoning_effort: None,
            output_reserve_tokens: None,
            auto_compact_threshold: None,
            supports_multimodal: false,
            prompt_caching_mode: HostRuntimeOptimizationMode::default(),
            cache_edit_mode: HostRuntimeOptimizationMode::default(),
        }
    }
}

pub fn create_provider_for_config(cfg: &HostProviderConfig) -> Result<Arc<dyn Provider>> {
    let api_key = cfg.api_key.as_deref();

    let model_name = cfg.model.as_deref().unwrap_or(match cfg.api_format {
        HostApiFormat::Anthropic => "claude-sonnet-4-5",
        HostApiFormat::OpenAI => "gpt-4o",
        HostApiFormat::LiteLLM => "gpt-4o-mini",
        HostApiFormat::Local => "llama2",
    });

    let model = ModelConfig::new(model_name)
        .map_err(|e| anyhow!("Invalid model '{}': {}", model_name, e))?
        .with_temperature(cfg.temperature)
        .with_max_tokens(cfg.max_tokens)
        .with_context_limit(cfg.context_limit)
        .with_thinking(Some(cfg.thinking_enabled), cfg.thinking_budget)
        .with_reasoning_effort(cfg.reasoning_effort.clone())
        .with_output_reserve_tokens(cfg.output_reserve_tokens)
        .with_auto_compact_threshold(cfg.auto_compact_threshold)
        .with_supports_multimodal(cfg.supports_multimodal)
        .with_prompt_caching_mode(map_prompt_caching_mode(cfg.prompt_caching_mode))
        .with_cache_edit_mode(map_cache_edit_mode(cfg.cache_edit_mode));

    match cfg.api_format {
        HostApiFormat::Anthropic => {
            let api_key = api_key
                .ok_or_else(|| anyhow!("API key not configured for agent '{}'", cfg.name))?;
            let provider = create_anthropic_provider(cfg, api_key, model)?;
            Ok(Arc::new(provider))
        }
        HostApiFormat::OpenAI => {
            if should_use_litellm_provider(cfg) {
                let provider = create_litellm_provider(cfg, model)?;
                Ok(Arc::new(provider))
            } else {
                let api_key = api_key
                    .ok_or_else(|| anyhow!("API key not configured for agent '{}'", cfg.name))?;
                let provider = create_openai_provider(cfg, api_key, model)?;
                Ok(Arc::new(provider))
            }
        }
        HostApiFormat::LiteLLM => {
            let provider = create_litellm_provider(cfg, model)?;
            Ok(Arc::new(provider))
        }
        HostApiFormat::Local => {
            let provider = create_local_openai_compatible_provider(cfg, api_key, model)?;
            Ok(Arc::new(provider))
        }
    }
}

fn should_use_litellm_provider(cfg: &HostProviderConfig) -> bool {
    if matches!(cfg.api_format, HostApiFormat::LiteLLM) {
        return true;
    }
    let Some(url) = cfg.api_url.as_deref() else {
        return false;
    };
    let normalized = url.trim().to_ascii_lowercase();
    normalized.contains("litellm")
        || normalized.contains("api.litellm.ai")
        || normalized.contains("localhost:4000")
        || normalized.contains("127.0.0.1:4000")
}

fn map_prompt_caching_mode(mode: HostRuntimeOptimizationMode) -> PromptCachingMode {
    match mode {
        HostRuntimeOptimizationMode::Auto => PromptCachingMode::Auto,
        HostRuntimeOptimizationMode::Off => PromptCachingMode::Off,
        HostRuntimeOptimizationMode::Prefer => PromptCachingMode::Prefer,
    }
}

fn map_cache_edit_mode(mode: HostRuntimeOptimizationMode) -> CacheEditMode {
    match mode {
        HostRuntimeOptimizationMode::Auto => CacheEditMode::Auto,
        HostRuntimeOptimizationMode::Off => CacheEditMode::Off,
        HostRuntimeOptimizationMode::Prefer => CacheEditMode::Prefer,
    }
}

fn create_anthropic_provider(
    cfg: &HostProviderConfig,
    api_key: &str,
    model: ModelConfig,
) -> Result<AnthropicProvider> {
    let base_url = cfg
        .api_url
        .as_deref()
        .unwrap_or("https://api.anthropic.com");

    let is_volcengine = base_url.contains("ark.cn-beijing.volces.com");

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
    cfg: &HostProviderConfig,
    api_key: &str,
    model: ModelConfig,
) -> Result<OpenAiProvider> {
    let base_url = cfg.api_url.as_deref().unwrap_or("https://api.openai.com");

    let auth = AuthMethod::BearerToken(api_key.to_string());
    let api_client = ApiClient::new(base_url.to_string(), auth)?;

    Ok(OpenAiProvider::new(api_client, model))
}

fn create_local_openai_compatible_provider(
    cfg: &HostProviderConfig,
    api_key: Option<&str>,
    model: ModelConfig,
) -> Result<OpenAiProvider> {
    let raw_url = cfg
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
    let base_path = if path.is_empty() || path == "v1" {
        "v1/chat/completions".to_string()
    } else {
        path.to_string()
    };
    Ok((host, base_path))
}

fn create_litellm_provider(
    cfg: &HostProviderConfig,
    model: ModelConfig,
) -> Result<LiteLLMProvider> {
    if let Some(base_url) = cfg
        .api_url
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        return LiteLLMProvider::from_custom_endpoint(
            model,
            base_url,
            cfg.api_key.as_deref(),
            None,
        );
    }
    futures::executor::block_on(LiteLLMProvider::from_env(model))
}

/// Map a registry provider name to its [`HostApiFormat`]. Returns `None` for
/// providers that are *not* covered by [`create_provider_for_config`] (e.g.
/// google, ollama, bedrock, tetrate) — callers should fall back to
/// `agime::providers::create` for those.
pub fn host_api_format_from_provider_name(name: &str) -> Option<HostApiFormat> {
    match name.to_ascii_lowercase().as_str() {
        "anthropic" => Some(HostApiFormat::Anthropic),
        "openai" => Some(HostApiFormat::OpenAI),
        "litellm" => Some(HostApiFormat::LiteLLM),
        "local" | "local-openai-compatible" => Some(HostApiFormat::Local),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn provider_name_maps_to_known_formats() {
        assert_eq!(
            host_api_format_from_provider_name("anthropic"),
            Some(HostApiFormat::Anthropic)
        );
        assert_eq!(
            host_api_format_from_provider_name("OpenAI"),
            Some(HostApiFormat::OpenAI)
        );
        assert_eq!(
            host_api_format_from_provider_name("litellm"),
            Some(HostApiFormat::LiteLLM)
        );
        assert_eq!(
            host_api_format_from_provider_name("local"),
            Some(HostApiFormat::Local)
        );
        assert_eq!(host_api_format_from_provider_name("google"), None);
        assert_eq!(host_api_format_from_provider_name("ollama"), None);
    }

    #[test]
    fn litellm_detection_recognises_known_endpoints() {
        let mut cfg = HostProviderConfig {
            name: "test".into(),
            api_format: HostApiFormat::OpenAI,
            api_url: Some("https://api.litellm.ai/v1".into()),
            ..Default::default()
        };
        assert!(should_use_litellm_provider(&cfg));

        cfg.api_url = Some("http://localhost:4000".into());
        assert!(should_use_litellm_provider(&cfg));

        cfg.api_url = Some("https://api.openai.com/v1".into());
        assert!(!should_use_litellm_provider(&cfg));

        cfg.api_format = HostApiFormat::LiteLLM;
        assert!(should_use_litellm_provider(&cfg));
    }
}
