use serde::{Deserialize, Serialize};
use thiserror::Error;
use utoipa::ToSchema;

use crate::config::env_compat::get_env_compat;

const DEFAULT_CONTEXT_LIMIT: usize = 128_000;

fn default_supports_multimodal() -> bool {
    true
}

#[derive(Error, Debug)]
pub enum ConfigError {
    #[error("Environment variable '{0}' not found")]
    EnvVarMissing(String),
    #[error("Invalid value for '{0}': '{1}' - {2}")]
    InvalidValue(String, String, String),
    #[error("Value for '{0}' is out of valid range: {1}")]
    InvalidRange(String, String),
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ModelConfig {
    pub model_name: String,
    pub context_limit: Option<usize>,
    pub temperature: Option<f32>,
    pub max_tokens: Option<i32>,
    pub thinking_enabled: Option<bool>,
    pub thinking_budget: Option<u32>,
    pub reasoning_effort: Option<String>,
    pub output_reserve_tokens: Option<usize>,
    pub auto_compact_threshold: Option<f64>,
    #[serde(default = "default_supports_multimodal")]
    pub supports_multimodal: bool,
    #[serde(default)]
    pub prompt_caching_mode: PromptCachingMode,
    #[serde(default)]
    pub cache_edit_mode: CacheEditMode,
    pub toolshim: bool,
    pub toolshim_model: Option<String>,
    pub fast_model: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, ToSchema, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum PromptCachingMode {
    #[default]
    Auto,
    Off,
    Prefer,
}

impl PromptCachingMode {
    pub fn is_disabled(self) -> bool {
        matches!(self, Self::Off)
    }
}

impl std::str::FromStr for PromptCachingMode {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "auto" => Ok(Self::Auto),
            "off" => Ok(Self::Off),
            "prefer" => Ok(Self::Prefer),
            other => Err(format!("Invalid prompt caching mode: {}", other)),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, ToSchema, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum CacheEditMode {
    #[default]
    Auto,
    Off,
    Prefer,
}

impl CacheEditMode {
    pub fn is_disabled(self) -> bool {
        matches!(self, Self::Off)
    }
}

impl std::str::FromStr for CacheEditMode {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "auto" => Ok(Self::Auto),
            "off" => Ok(Self::Off),
            "prefer" => Ok(Self::Prefer),
            other => Err(format!("Invalid cache edit mode: {}", other)),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelLimitConfig {
    pub pattern: String,
    pub context_limit: usize,
}

impl ModelConfig {
    pub fn new(model_name: &str) -> Result<Self, ConfigError> {
        Self::new_with_context_env(model_name.to_string(), None)
    }

    pub fn new_with_context_env(
        model_name: String,
        context_env_var: Option<&str>,
    ) -> Result<Self, ConfigError> {
        let context_limit = Self::parse_context_limit(&model_name, None, context_env_var)?;
        let temperature = Self::parse_temperature()?;
        let toolshim = Self::parse_toolshim()?;
        let toolshim_model = Self::parse_toolshim_model()?;

        Ok(Self {
            model_name,
            context_limit,
            temperature,
            max_tokens: None,
            thinking_enabled: None,
            thinking_budget: None,
            reasoning_effort: None,
            output_reserve_tokens: None,
            auto_compact_threshold: None,
            supports_multimodal: Self::parse_supports_multimodal()?,
            prompt_caching_mode: PromptCachingMode::Auto,
            cache_edit_mode: CacheEditMode::Auto,
            toolshim,
            toolshim_model,
            fast_model: None,
        })
    }

    fn parse_context_limit(
        model_name: &str,
        fast_model: Option<&str>,
        custom_env_var: Option<&str>,
    ) -> Result<Option<usize>, ConfigError> {
        // First check if there's an explicit environment variable override
        if let Some(env_var) = custom_env_var {
            if let Ok(val) = std::env::var(env_var) {
                return Self::validate_context_limit(&val, env_var).map(Some);
            }
        }
        if let Some(val) = get_env_compat("CONTEXT_LIMIT") {
            return Self::validate_context_limit(&val, "CONTEXT_LIMIT").map(Some);
        }

        let _ = model_name;
        let _ = fast_model;
        Ok(None)
    }

    fn validate_context_limit(val: &str, env_var: &str) -> Result<usize, ConfigError> {
        let limit = val.parse::<usize>().map_err(|_| {
            ConfigError::InvalidValue(
                env_var.to_string(),
                val.to_string(),
                "must be a positive integer".to_string(),
            )
        })?;

        if limit < 4 * 1024 {
            return Err(ConfigError::InvalidRange(
                env_var.to_string(),
                "must be greater than 4K".to_string(),
            ));
        }

        Ok(limit)
    }

    fn parse_temperature() -> Result<Option<f32>, ConfigError> {
        if let Some(val) = get_env_compat("TEMPERATURE") {
            let temp = val.parse::<f32>().map_err(|_| {
                ConfigError::InvalidValue(
                    "TEMPERATURE".to_string(),
                    val.clone(),
                    "must be a valid number".to_string(),
                )
            })?;
            if temp < 0.0 {
                return Err(ConfigError::InvalidRange("TEMPERATURE".to_string(), val));
            }
            Ok(Some(temp))
        } else {
            Ok(None)
        }
    }

    fn parse_toolshim() -> Result<bool, ConfigError> {
        if let Some(val) = get_env_compat("TOOLSHIM") {
            match val.to_lowercase().as_str() {
                "1" | "true" | "yes" | "on" => Ok(true),
                "0" | "false" | "no" | "off" => Ok(false),
                _ => Err(ConfigError::InvalidValue(
                    "TOOLSHIM".to_string(),
                    val,
                    "must be one of: 1, true, yes, on, 0, false, no, off".to_string(),
                )),
            }
        } else {
            Ok(false)
        }
    }

    fn parse_toolshim_model() -> Result<Option<String>, ConfigError> {
        match get_env_compat("TOOLSHIM_OLLAMA_MODEL") {
            Some(val) if val.trim().is_empty() => Err(ConfigError::InvalidValue(
                "TOOLSHIM_OLLAMA_MODEL".to_string(),
                val,
                "cannot be empty if set".to_string(),
            )),
            Some(val) => Ok(Some(val)),
            None => Ok(None),
        }
    }

    /// Whether the model/endpoint should be sent image (multimodal) content.
    /// Defaults to `true`. The value is read with the standard dual-prefix
    /// precedence (`AGIME_MULTIMODAL` env, then `GOOSE_MULTIMODAL` env, then the
    /// `AGIME_MULTIMODAL` key in `config.yaml`) so the desktop UI toggle — which
    /// persists to `config.yaml` — is honored on the registry chat path. Set it
    /// to a falsey value to force images to be down-converted to text
    /// placeholders, which avoids 400 rejections from endpoints that only
    /// accept text.
    fn parse_supports_multimodal() -> Result<bool, ConfigError> {
        match crate::config::Config::global().get_param::<serde_json::Value>("MULTIMODAL") {
            Ok(value) => Self::interpret_multimodal_value(&value),
            Err(_) => Ok(true),
        }
    }

    fn interpret_multimodal_value(value: &serde_json::Value) -> Result<bool, ConfigError> {
        match value {
            serde_json::Value::Bool(b) => Ok(*b),
            serde_json::Value::String(s) => match s.trim().to_lowercase().as_str() {
                "1" | "true" | "yes" | "on" => Ok(true),
                "0" | "false" | "no" | "off" => Ok(false),
                _ => Err(ConfigError::InvalidValue(
                    "MULTIMODAL".to_string(),
                    s.clone(),
                    "must be one of: 1, true, yes, on, 0, false, no, off".to_string(),
                )),
            },
            // Numeric env values like `0` / `1` are JSON-parsed into numbers
            // before reaching here (see Config::parse_env_value).
            serde_json::Value::Number(n) => match n.as_i64() {
                Some(0) => Ok(false),
                Some(1) => Ok(true),
                _ => Err(ConfigError::InvalidValue(
                    "MULTIMODAL".to_string(),
                    n.to_string(),
                    "numeric value must be 0 or 1".to_string(),
                )),
            },
            other => Err(ConfigError::InvalidValue(
                "MULTIMODAL".to_string(),
                other.to_string(),
                "must be a boolean or one of: 1, true, yes, on, 0, false, no, off".to_string(),
            )),
        }
    }

    pub fn with_context_limit(mut self, limit: Option<usize>) -> Self {
        if limit.is_some() {
            self.context_limit = limit;
        }
        self
    }

    pub fn with_temperature(mut self, temp: Option<f32>) -> Self {
        self.temperature = temp;
        self
    }

    pub fn with_max_tokens(mut self, tokens: Option<i32>) -> Self {
        self.max_tokens = tokens;
        self
    }

    pub fn with_thinking(mut self, enabled: Option<bool>, budget: Option<u32>) -> Self {
        self.thinking_enabled = enabled;
        self.thinking_budget = budget;
        self
    }

    pub fn with_reasoning_effort(mut self, effort: Option<String>) -> Self {
        self.reasoning_effort = effort.filter(|value| !value.trim().is_empty());
        self
    }

    pub fn with_output_reserve_tokens(mut self, output_reserve_tokens: Option<usize>) -> Self {
        if output_reserve_tokens.is_some() {
            self.output_reserve_tokens = output_reserve_tokens;
        }
        self
    }

    pub fn with_auto_compact_threshold(mut self, threshold: Option<f64>) -> Self {
        if threshold.is_some() {
            self.auto_compact_threshold = threshold;
        }
        self
    }

    pub fn with_supports_multimodal(mut self, supports_multimodal: bool) -> Self {
        self.supports_multimodal = supports_multimodal;
        self
    }

    pub fn with_prompt_caching_mode(mut self, mode: PromptCachingMode) -> Self {
        self.prompt_caching_mode = mode;
        self
    }

    pub fn with_cache_edit_mode(mut self, mode: CacheEditMode) -> Self {
        self.cache_edit_mode = mode;
        self
    }

    pub fn with_toolshim(mut self, toolshim: bool) -> Self {
        self.toolshim = toolshim;
        self
    }

    pub fn with_toolshim_model(mut self, model: Option<String>) -> Self {
        self.toolshim_model = model;
        self
    }

    pub fn with_fast(mut self, fast_model: String) -> Self {
        self.fast_model = Some(fast_model);
        self
    }

    pub fn use_fast_model(&self) -> Self {
        if let Some(fast_model) = &self.fast_model {
            let mut config = self.clone();
            config.model_name = fast_model.clone();
            config
        } else {
            self.clone()
        }
    }

    pub fn context_limit(&self) -> usize {
        if let Some(limit) = self.context_limit {
            return limit;
        }
        DEFAULT_CONTEXT_LIMIT
    }

    pub fn new_or_fail(model_name: &str) -> ModelConfig {
        ModelConfig::new(model_name)
            .unwrap_or_else(|_| panic!("Failed to create model config for {}", model_name))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;
    use std::env;

    const AGIME_KEY: &str = "AGIME_MULTIMODAL";
    const GOOSE_KEY: &str = "GOOSE_MULTIMODAL";

    fn clear_multimodal_env() {
        env::remove_var(AGIME_KEY);
        env::remove_var(GOOSE_KEY);
    }

    #[test]
    #[serial]
    fn supports_multimodal_defaults_to_true_when_unset() {
        clear_multimodal_env();
        assert!(ModelConfig::parse_supports_multimodal().unwrap());
        let config = ModelConfig::new("test-model").unwrap();
        assert!(config.supports_multimodal);
    }

    #[test]
    #[serial]
    fn supports_multimodal_disabled_by_falsey_values() {
        for val in ["0", "false", "no", "off", "OFF", "False"] {
            clear_multimodal_env();
            env::set_var(AGIME_KEY, val);
            assert!(
                !ModelConfig::parse_supports_multimodal().unwrap(),
                "value {val:?} should disable multimodal"
            );
        }
        clear_multimodal_env();
    }

    #[test]
    #[serial]
    fn supports_multimodal_enabled_by_truthy_values() {
        for val in ["1", "true", "yes", "on", "ON", "True"] {
            clear_multimodal_env();
            env::set_var(AGIME_KEY, val);
            assert!(
                ModelConfig::parse_supports_multimodal().unwrap(),
                "value {val:?} should enable multimodal"
            );
        }
        clear_multimodal_env();
    }

    #[test]
    #[serial]
    fn supports_multimodal_rejects_garbage_values() {
        clear_multimodal_env();
        env::set_var(AGIME_KEY, "maybe");
        let err = ModelConfig::parse_supports_multimodal().unwrap_err();
        assert!(matches!(err, ConfigError::InvalidValue(name, _, _) if name == "MULTIMODAL"));
        clear_multimodal_env();
    }

    #[test]
    #[serial]
    fn supports_multimodal_honors_goose_legacy_prefix() {
        clear_multimodal_env();
        env::set_var(GOOSE_KEY, "false");
        assert!(!ModelConfig::parse_supports_multimodal().unwrap());
        let config = ModelConfig::new("test-model").unwrap();
        assert!(!config.supports_multimodal);
        clear_multimodal_env();
    }

    // Direct coverage for the config.yaml value shapes that
    // `Config::get_param` can return (native YAML bool, quoted string, number).
    // The desktop UI persists this key to config.yaml, so these shapes are the
    // real-world inputs the registry chat path must interpret.

    #[test]
    fn interprets_native_bool_from_config_yaml() {
        assert!(ModelConfig::interpret_multimodal_value(&serde_json::json!(true)).unwrap());
        assert!(!ModelConfig::interpret_multimodal_value(&serde_json::json!(false)).unwrap());
    }

    #[test]
    fn interprets_string_values_from_config_yaml() {
        for val in ["true", "yes", "on", "1", "True"] {
            assert!(
                ModelConfig::interpret_multimodal_value(&serde_json::json!(val)).unwrap(),
                "string {val:?} should enable multimodal"
            );
        }
        for val in ["false", "no", "off", "0", "False"] {
            assert!(
                !ModelConfig::interpret_multimodal_value(&serde_json::json!(val)).unwrap(),
                "string {val:?} should disable multimodal"
            );
        }
    }

    #[test]
    fn interprets_numeric_values_from_config_yaml() {
        assert!(ModelConfig::interpret_multimodal_value(&serde_json::json!(1)).unwrap());
        assert!(!ModelConfig::interpret_multimodal_value(&serde_json::json!(0)).unwrap());
        assert!(ModelConfig::interpret_multimodal_value(&serde_json::json!(2)).is_err());
    }

    #[test]
    fn rejects_invalid_config_yaml_shapes() {
        assert!(ModelConfig::interpret_multimodal_value(&serde_json::json!("maybe")).is_err());
        assert!(ModelConfig::interpret_multimodal_value(&serde_json::json!(["a"])).is_err());
        assert!(ModelConfig::interpret_multimodal_value(&serde_json::json!({"k": "v"})).is_err());
    }
}
