//! Configuration-driven Thinking/Reasoning Handler
//!
//! This module provides a unified approach to handling thinking mode and reasoning
//! parameters across different AI providers. Instead of hardcoded logic, it uses
//! configuration from model_capabilities.json to:
//! 1. Add appropriate parameters to API requests
//! 2. Parse thinking content from API responses
//! 3. Extract thinking from tag-based formats (<think>...</think>)

use anyhow::{Context, Result};
use regex::Regex;
use serde_json::{json, Value};

use crate::capabilities::{
    ReasoningRequestConfig, RequestMethod, ResolvedCapabilities, ResponseType,
    ThinkingRequestConfig, ThinkingResponseConfig, ThinkingType,
};

/// Parsed thinking content from a response
#[derive(Debug, Clone)]
pub struct ThinkingContent {
    /// The thinking/reasoning text
    pub content: String,
    /// Optional signature (Claude-specific)
    pub signature: Option<String>,
}

/// Partial thinking content for streaming
#[derive(Debug, Clone)]
pub struct PartialThinking {
    /// Thinking content delta
    pub delta: String,
    /// Whether this is a signature delta
    pub is_signature: bool,
}

/// Streaming accumulator for thinking content
#[derive(Debug, Default)]
pub struct StreamingThinkingAccumulator {
    thinking_buffer: String,
    signature_buffer: Option<String>,
    in_thinking_block: bool,
    config: Option<ThinkingResponseConfig>,
}

impl StreamingThinkingAccumulator {
    /// Create a new accumulator with the given response config
    pub fn new(config: Option<ThinkingResponseConfig>) -> Self {
        Self {
            config,
            ..Default::default()
        }
    }

    /// Accumulate a chunk of thinking content
    pub fn accumulate(&mut self, chunk: &Value) -> Option<PartialThinking> {
        let config = self.config.clone()?;

        match config.response_type {
            ResponseType::ContentBlock => self.accumulate_content_block(chunk, &config),
            ResponseType::Field => self.accumulate_field(chunk, &config),
            ResponseType::Tag => {
                // Tag-based thinking is accumulated as text and parsed at the end
                None
            }
        }
    }

    fn accumulate_content_block(
        &mut self,
        chunk: &Value,
        config: &ThinkingResponseConfig,
    ) -> Option<PartialThinking> {
        // Handle content_block_start
        if let Some(content_block) = chunk.get("content_block") {
            let block_type = content_block.get("type")?.as_str()?;
            let expected_type = config.block_type.as_deref().unwrap_or("thinking");

            if block_type == expected_type {
                self.in_thinking_block = true;
                return None;
            }
        }

        // Handle content_block_delta
        if let Some(delta) = chunk.get("delta") {
            if self.in_thinking_block {
                let content_field = config.content_field.as_deref().unwrap_or("thinking");

                if let Some(thinking_delta) = delta.get(content_field).and_then(|v| v.as_str()) {
                    self.thinking_buffer.push_str(thinking_delta);
                    return Some(PartialThinking {
                        delta: thinking_delta.to_string(),
                        is_signature: false,
                    });
                }

                // Handle signature delta
                if let Some(sig_field) = &config.signature_field {
                    if let Some(sig_delta) = delta.get(sig_field).and_then(|v| v.as_str()) {
                        let sig_buf = self.signature_buffer.get_or_insert_with(String::new);
                        sig_buf.push_str(sig_delta);
                        return Some(PartialThinking {
                            delta: sig_delta.to_string(),
                            is_signature: true,
                        });
                    }
                }
            }
        }

        // Handle content_block_stop
        if let Some(event_type) = chunk.get("type").and_then(|v| v.as_str()) {
            if event_type == "content_block_stop" && self.in_thinking_block {
                self.in_thinking_block = false;
            }
        }

        None
    }

    fn accumulate_field(
        &mut self,
        chunk: &Value,
        config: &ThinkingResponseConfig,
    ) -> Option<PartialThinking> {
        let content_field = config
            .content_field
            .as_deref()
            .unwrap_or("reasoning_content");

        // Check in choices[0].delta
        if let Some(delta) = chunk
            .get("choices")
            .and_then(|c| c.get(0))
            .and_then(|c| c.get("delta"))
        {
            if let Some(reasoning) = delta.get(content_field).and_then(|v| v.as_str()) {
                if !reasoning.is_empty() {
                    self.thinking_buffer.push_str(reasoning);
                    return Some(PartialThinking {
                        delta: reasoning.to_string(),
                        is_signature: false,
                    });
                }
            }
        }

        None
    }

    /// Finalize and return the accumulated thinking content
    pub fn finalize(&self) -> Option<ThinkingContent> {
        if self.thinking_buffer.is_empty() {
            return None;
        }

        Some(ThinkingContent {
            content: self.thinking_buffer.clone(),
            signature: self.signature_buffer.clone(),
        })
    }
}

/// ThinkingHandler provides configuration-driven thinking/reasoning support
pub struct ThinkingHandler;

impl ThinkingHandler {
    /// Apply thinking parameters to the request payload based on configuration
    pub fn apply_request_params(payload: &mut Value, caps: &ResolvedCapabilities) -> Result<()> {
        if !caps.thinking_enabled || !caps.thinking_supported {
            return Ok(());
        }

        let budget = caps.thinking_budget.unwrap_or(16000);

        // Use configuration if available, otherwise fall back to type-based defaults
        if let Some(config) = &caps.thinking_request_config {
            Self::apply_configured_params(payload, config, budget)?;
        } else {
            // Fall back to type-based defaults for backward compatibility
            Self::apply_default_params(payload, caps, budget)?;
        }

        Ok(())
    }

    fn apply_configured_params(
        payload: &mut Value,
        config: &ThinkingRequestConfig,
        budget: u32,
    ) -> Result<()> {
        let param_value = if let Some(template) = &config.param_template {
            // Replace ${budget} placeholder in template
            // Handle both string "${budget}" and the value itself
            let template_str = serde_json::to_string(template)?;
            // Replace "${budget}" (with quotes) with the number (without quotes)
            // This converts string placeholder to actual number in JSON
            let replaced = template_str.replace("\"${budget}\"", &budget.to_string());
            // Also handle case where it might be used without quotes (shouldn't happen but be safe)
            let replaced = replaced.replace("${budget}", &budget.to_string());
            serde_json::from_str::<Value>(&replaced)?
        } else if let Some(value) = &config.param_value {
            value.clone()
        } else {
            // Default value based on method
            json!(true)
        };

        match config.method {
            RequestMethod::Parameter => {
                Self::set_nested_value(payload, &config.param_path, param_value);
            }
            RequestMethod::ExtraBody => {
                let extra_body = payload
                    .as_object_mut()
                    .context("Payload must be an object")?
                    .entry("extra_body")
                    .or_insert_with(|| json!({}));
                Self::set_nested_value(extra_body, &config.param_path, param_value);
            }
            RequestMethod::Header | RequestMethod::Auto => {
                // Headers are handled separately in provider code
                // Auto falls back to Parameter
                Self::set_nested_value(payload, &config.param_path, param_value);
            }
        }

        // Handle max_tokens adjustment
        if config.max_tokens_adjustment == "add_budget" {
            if let Some(max_tokens) = payload.get("max_tokens").and_then(|v| v.as_u64()) {
                payload["max_tokens"] = json!(max_tokens + budget as u64);
            }
        }

        Ok(())
    }

    fn apply_default_params(
        payload: &mut Value,
        caps: &ResolvedCapabilities,
        budget: u32,
    ) -> Result<()> {
        match caps.thinking_type {
            ThinkingType::Api => {
                // Claude-style thinking
                payload["thinking"] = json!({
                    "type": "enabled",
                    "budget_tokens": budget
                });

                // Adjust max_tokens
                if let Some(max_tokens) = payload.get("max_tokens").and_then(|v| v.as_u64()) {
                    payload["max_tokens"] = json!(max_tokens + budget as u64);
                }
            }
            ThinkingType::Tag => {
                // DeepSeek/Qwen style - add to extra_body
                let extra_body = payload
                    .as_object_mut()
                    .context("Payload must be an object")?
                    .entry("extra_body")
                    .or_insert_with(|| json!({}));

                if let Some(obj) = extra_body.as_object_mut() {
                    obj.insert("enable_thinking".to_string(), json!(true));
                    obj.insert("thinking_budget".to_string(), json!(budget));
                }
            }
            ThinkingType::None => {}
        }

        Ok(())
    }

    /// Apply reasoning effort parameters to the request payload
    pub fn apply_reasoning_params(payload: &mut Value, caps: &ResolvedCapabilities) -> Result<()> {
        if !caps.reasoning_supported {
            return Ok(());
        }

        let effort = caps
            .reasoning_effort
            .as_ref()
            .map(|s: &String| s.as_str())
            .unwrap_or("medium");

        // Use configuration if available
        if let Some(config) = &caps.reasoning_request_config {
            Self::apply_reasoning_configured(payload, config, effort)?;
        } else {
            // Fall back to default api_param
            Self::set_nested_value(payload, &caps.reasoning_param, json!(effort));
        }

        Ok(())
    }

    fn apply_reasoning_configured(
        payload: &mut Value,
        config: &ReasoningRequestConfig,
        effort: &str,
    ) -> Result<()> {
        let value = if config.use_effort_level {
            json!(effort)
        } else {
            json!(true)
        };

        match config.method {
            RequestMethod::Parameter => {
                Self::set_nested_value(payload, &config.param_path, value);
            }
            RequestMethod::ExtraBody => {
                let extra_body = payload
                    .as_object_mut()
                    .context("Payload must be an object")?
                    .entry("extra_body")
                    .or_insert_with(|| json!({}));
                Self::set_nested_value(extra_body, &config.param_path, value);
            }
            RequestMethod::Header | RequestMethod::Auto => {
                Self::set_nested_value(payload, &config.param_path, value);
            }
        }

        Ok(())
    }

    /// Parse thinking content from a complete API response
    pub fn parse_response(
        response: &Value,
        caps: &ResolvedCapabilities,
    ) -> Option<ThinkingContent> {
        if !caps.thinking_supported {
            return None;
        }

        // Use configuration if available
        if let Some(config) = &caps.thinking_response_config {
            return Self::parse_configured_response(response, config);
        }

        // Fall back to type-based parsing
        match caps.thinking_type {
            ThinkingType::Api => Self::parse_content_block_response(response),
            ThinkingType::Tag => Self::parse_tag_response(response, None),
            ThinkingType::None => None,
        }
    }

    fn parse_configured_response(
        response: &Value,
        config: &ThinkingResponseConfig,
    ) -> Option<ThinkingContent> {
        match config.response_type {
            ResponseType::ContentBlock => Self::parse_content_block_with_config(response, config),
            ResponseType::Field => Self::parse_field_response(response, config),
            ResponseType::Tag => Self::parse_tag_response(response, config.tag_pattern.as_deref()),
        }
    }

    fn parse_content_block_with_config(
        response: &Value,
        config: &ThinkingResponseConfig,
    ) -> Option<ThinkingContent> {
        let block_type = config.block_type.as_deref().unwrap_or("thinking");
        let content_field = config.content_field.as_deref().unwrap_or("thinking");

        // Look in content array
        let content = response.get("content")?.as_array()?;

        for block in content {
            if block.get("type")?.as_str()? == block_type {
                let thinking = block.get(content_field)?.as_str()?.to_string();
                let signature = config
                    .signature_field
                    .as_ref()
                    .and_then(|f: &String| block.get(f))
                    .and_then(|v: &Value| v.as_str())
                    .map(|s: &str| s.to_string());

                return Some(ThinkingContent {
                    content: thinking,
                    signature,
                });
            }
        }

        None
    }

    fn parse_content_block_response(response: &Value) -> Option<ThinkingContent> {
        // Claude-style content blocks
        let content = response.get("content")?.as_array()?;

        for block in content {
            if block.get("type")?.as_str()? == "thinking" {
                return Some(ThinkingContent {
                    content: block.get("thinking")?.as_str()?.to_string(),
                    signature: block
                        .get("signature")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string()),
                });
            }
        }

        None
    }

    fn parse_field_response(
        response: &Value,
        config: &ThinkingResponseConfig,
    ) -> Option<ThinkingContent> {
        let content_field = config
            .content_field
            .as_deref()
            .unwrap_or("reasoning_content");

        // Try direct field access
        if let Some(content) = response.get(content_field).and_then(|v| v.as_str()) {
            if !content.is_empty() {
                return Some(ThinkingContent {
                    content: content.to_string(),
                    signature: None,
                });
            }
        }

        // Try in choices[0].message
        if let Some(content) = response
            .get("choices")
            .and_then(|c| c.get(0))
            .and_then(|c| c.get("message"))
            .and_then(|m| m.get(content_field))
            .and_then(|v| v.as_str())
        {
            if !content.is_empty() {
                return Some(ThinkingContent {
                    content: content.to_string(),
                    signature: None,
                });
            }
        }

        // Fallback: try to extract from text content using fallback_tag_pattern
        if let Some(fallback_pattern) = &config.fallback_tag_pattern {
            if let Some(text) = Self::extract_text_content(response) {
                if let Some(thinking) = Self::extract_tag_thinking(&text, Some(fallback_pattern)) {
                    return Some(thinking);
                }
            }
        }

        None
    }

    fn parse_tag_response(response: &Value, pattern: Option<&str>) -> Option<ThinkingContent> {
        // Get the text content from the response
        let text = Self::extract_text_content(response)?;

        // Try to extract thinking from tags
        Self::extract_tag_thinking(&text, pattern)
    }

    fn extract_text_content(response: &Value) -> Option<String> {
        // Try content array (Claude-style)
        if let Some(content) = response.get("content").and_then(|c| c.as_array()) {
            for block in content {
                if block.get("type").and_then(|t| t.as_str()) == Some("text") {
                    if let Some(text) = block.get("text").and_then(|t| t.as_str()) {
                        return Some(text.to_string());
                    }
                }
            }
        }

        // Try choices[0].message.content (OpenAI-style)
        if let Some(content) = response
            .get("choices")
            .and_then(|c| c.get(0))
            .and_then(|c| c.get("message"))
            .and_then(|m| m.get("content"))
            .and_then(|c| c.as_str())
        {
            return Some(content.to_string());
        }

        None
    }

    /// Extract thinking content from text using tag pattern
    pub fn extract_tag_thinking(text: &str, pattern: Option<&str>) -> Option<ThinkingContent> {
        let default_pattern = r"<think>([\s\S]*?)</think>";
        let pattern_str = pattern.unwrap_or(default_pattern);

        let re = Regex::new(pattern_str).ok()?;

        if let Some(captures) = re.captures(text) {
            if let Some(thinking) = captures.get(1) {
                return Some(ThinkingContent {
                    content: thinking.as_str().trim().to_string(),
                    signature: None,
                });
            }
        }

        None
    }

    /// Remove thinking tags from text content (for displaying final output)
    pub fn strip_thinking_tags(text: &str, pattern: Option<&str>) -> String {
        let default_pattern = r"<think>[\s\S]*?</think>";
        let pattern_str = pattern.unwrap_or(default_pattern);

        if let Ok(re) = Regex::new(pattern_str) {
            re.replace_all(text, "").trim().to_string()
        } else {
            text.to_string()
        }
    }

    /// Set a nested value in a JSON object using dot-notation path
    fn set_nested_value(obj: &mut Value, path: &str, value: Value) {
        // Handle empty path - do nothing
        if path.is_empty() {
            return;
        }

        let parts: Vec<&str> = path.split('.').collect();

        if parts.len() == 1 {
            obj[parts[0]] = value;
            return;
        }

        let mut current = obj;
        for (i, part) in parts.iter().enumerate() {
            if i == parts.len() - 1 {
                current[*part] = value;
                return;
            }

            if current.get(*part).is_none() {
                current[*part] = json!({});
            }
            current = &mut current[*part];
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_tag_thinking() {
        let text =
            "Here is my response.\n<think>Let me think about this...</think>\nThe answer is 42.";
        let result = ThinkingHandler::extract_tag_thinking(text, None);

        assert!(result.is_some());
        let thinking = result.unwrap();
        assert_eq!(thinking.content, "Let me think about this...");
        assert!(thinking.signature.is_none());
    }

    #[test]
    fn test_extract_tag_thinking_custom_pattern() {
        let text = "Response\n<reasoning>My reasoning here</reasoning>\nAnswer";
        let result =
            ThinkingHandler::extract_tag_thinking(text, Some(r"<reasoning>([\s\S]*?)</reasoning>"));

        assert!(result.is_some());
        assert_eq!(result.unwrap().content, "My reasoning here");
    }

    #[test]
    fn test_strip_thinking_tags() {
        let text = "Hello <think>internal thinking</think> World";
        let result = ThinkingHandler::strip_thinking_tags(text, None);
        assert_eq!(result, "Hello  World");
    }

    #[test]
    fn test_set_nested_value() {
        let mut obj = json!({});
        ThinkingHandler::set_nested_value(&mut obj, "thinking.type", json!("enabled"));

        assert_eq!(obj["thinking"]["type"], "enabled");
    }

    #[test]
    fn test_set_nested_value_deep() {
        let mut obj = json!({});
        ThinkingHandler::set_nested_value(&mut obj, "a.b.c.d", json!(42));

        assert_eq!(obj["a"]["b"]["c"]["d"], 42);
    }

    #[test]
    fn test_set_nested_value_empty_path() {
        // Empty path should do nothing
        let mut obj = json!({"existing": "value"});
        ThinkingHandler::set_nested_value(&mut obj, "", json!("should not be set"));

        // Object should remain unchanged
        assert_eq!(obj, json!({"existing": "value"}));
    }

    #[test]
    fn test_parse_content_block_response() {
        let response = json!({
            "content": [
                {
                    "type": "thinking",
                    "thinking": "I am thinking deeply...",
                    "signature": "abc123"
                },
                {
                    "type": "text",
                    "text": "The answer is 42."
                }
            ]
        });

        let result = ThinkingHandler::parse_content_block_response(&response);
        assert!(result.is_some());

        let thinking = result.unwrap();
        assert_eq!(thinking.content, "I am thinking deeply...");
        assert_eq!(thinking.signature, Some("abc123".to_string()));
    }

    #[test]
    fn test_parse_field_response() {
        let response = json!({
            "choices": [{
                "message": {
                    "content": "The answer",
                    "reasoning_content": "Let me reason through this..."
                }
            }]
        });

        let config = ThinkingResponseConfig {
            response_type: ResponseType::Field,
            content_field: Some("reasoning_content".to_string()),
            ..Default::default()
        };

        let result = ThinkingHandler::parse_field_response(&response, &config);
        assert!(result.is_some());
        assert_eq!(result.unwrap().content, "Let me reason through this...");
    }

    #[test]
    fn test_budget_tokens_replaced_as_number() {
        // Test that ${budget} placeholder in param_template is replaced as a number, not a string
        let config = ThinkingRequestConfig {
            method: RequestMethod::Parameter,
            param_path: "thinking".to_string(),
            param_template: Some(json!({
                "type": "enabled",
                "budget_tokens": "${budget}"
            })),
            param_value: None,
            max_tokens_adjustment: String::new(),
        };

        let mut payload = json!({});
        ThinkingHandler::apply_configured_params(&mut payload, &config, 16000).unwrap();

        // budget_tokens should be a number (16000), not a string ("16000")
        assert!(payload["thinking"]["budget_tokens"].is_number());
        assert_eq!(payload["thinking"]["budget_tokens"], 16000);
        assert_eq!(payload["thinking"]["type"], "enabled");
    }

    #[test]
    fn test_fallback_tag_pattern() {
        // Test that fallback_tag_pattern is used when reasoning_content field is empty
        let response = json!({
            "choices": [{
                "message": {
                    "content": "Here is my answer. <think>My thinking process...</think> The result is 42.",
                    "reasoning_content": ""
                }
            }]
        });

        let config = ThinkingResponseConfig {
            response_type: ResponseType::Field,
            content_field: Some("reasoning_content".to_string()),
            fallback_tag_pattern: Some(r"<think>([\s\S]*?)</think>".to_string()),
            ..Default::default()
        };

        let result = ThinkingHandler::parse_field_response(&response, &config);
        assert!(result.is_some());
        assert_eq!(result.unwrap().content, "My thinking process...");
    }

    #[test]
    fn test_fallback_tag_pattern_not_used_when_field_exists() {
        // Test that fallback is NOT used when reasoning_content field has content
        let response = json!({
            "choices": [{
                "message": {
                    "content": "Answer with <think>tag content</think>",
                    "reasoning_content": "Field content takes priority"
                }
            }]
        });

        let config = ThinkingResponseConfig {
            response_type: ResponseType::Field,
            content_field: Some("reasoning_content".to_string()),
            fallback_tag_pattern: Some(r"<think>([\s\S]*?)</think>".to_string()),
            ..Default::default()
        };

        let result = ThinkingHandler::parse_field_response(&response, &config);
        assert!(result.is_some());
        // Should use field content, not tag content
        assert_eq!(result.unwrap().content, "Field content takes priority");
    }
}
