use crate::conversation::message::{Message, MessageContent};
use crate::model::ModelConfig;
use crate::providers::base::{ProviderUsage, Usage};
use crate::providers::thinking_handler::ThinkingHandler;
use anyhow::{anyhow, Error};
use async_stream::try_stream;
use chrono;
use futures::Stream;
use rmcp::model::{object, CallToolRequestParams, RawContent, Role, Tool, ToolChoiceMode};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::ops::Deref;

#[derive(Debug, Serialize, Deserialize)]
pub struct ResponsesApiResponse {
    pub id: String,
    pub object: String,
    pub created_at: i64,
    pub status: String,
    pub model: String,
    pub output: Vec<ResponseOutputItem>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<ResponseReasoningInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<ResponseUsage>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
#[serde(rename_all = "snake_case")]
pub enum ResponseOutputItem {
    Reasoning {
        id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        summary: Option<Vec<String>>,
    },
    Message {
        id: String,
        status: String,
        role: String,
        content: Vec<ResponseContentBlock>,
    },
    FunctionCall {
        id: String,
        status: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        call_id: Option<String>,
        name: String,
        arguments: String,
    },
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
#[serde(rename_all = "snake_case")]
pub enum ResponseContentBlock {
    OutputText {
        text: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        annotations: Option<Vec<Value>>,
    },
    ToolCall {
        id: String,
        name: String,
        input: Value,
    },
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ResponseReasoningInfo {
    pub effort: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ResponseUsage {
    pub input_tokens: i32,
    pub output_tokens: i32,
    pub total_tokens: i32,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
#[serde(rename_all = "snake_case")]
pub enum ResponsesStreamEvent {
    #[serde(rename = "keepalive")]
    KeepAlive {
        #[serde(default)]
        sequence_number: Option<i32>,
    },
    #[serde(rename = "response.created")]
    ResponseCreated {
        sequence_number: i32,
        response: ResponseMetadata,
    },
    #[serde(rename = "response.in_progress")]
    ResponseInProgress {
        sequence_number: i32,
        response: ResponseMetadata,
    },
    #[serde(rename = "response.output_item.added")]
    OutputItemAdded {
        sequence_number: i32,
        output_index: i32,
        item: ResponseOutputItemInfo,
    },
    #[serde(rename = "response.content_part.added")]
    ContentPartAdded {
        sequence_number: i32,
        item_id: String,
        output_index: i32,
        content_index: i32,
        part: ContentPart,
    },
    #[serde(rename = "response.output_text.delta")]
    OutputTextDelta {
        sequence_number: i32,
        item_id: String,
        output_index: i32,
        content_index: i32,
        delta: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        logprobs: Option<Vec<Value>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        obfuscation: Option<String>,
    },
    #[serde(rename = "response.output_item.done")]
    OutputItemDone {
        sequence_number: i32,
        output_index: i32,
        item: ResponseOutputItemInfo,
    },
    #[serde(rename = "response.content_part.done")]
    ContentPartDone {
        sequence_number: i32,
        item_id: String,
        output_index: i32,
        content_index: i32,
        part: ContentPart,
    },
    #[serde(rename = "response.output_text.done")]
    OutputTextDone {
        sequence_number: i32,
        item_id: String,
        output_index: i32,
        content_index: i32,
        text: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        logprobs: Option<Vec<Value>>,
    },
    #[serde(rename = "response.completed")]
    ResponseCompleted {
        sequence_number: i32,
        response: ResponseMetadata,
    },
    #[serde(rename = "response.failed")]
    ResponseFailed { sequence_number: i32, error: Value },
    #[serde(rename = "response.function_call_arguments.delta")]
    FunctionCallArgumentsDelta {
        sequence_number: i32,
        item_id: String,
        output_index: i32,
        delta: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        obfuscation: Option<String>,
    },
    #[serde(rename = "response.function_call_arguments.done")]
    FunctionCallArgumentsDone {
        sequence_number: i32,
        item_id: String,
        output_index: i32,
        arguments: String,
    },
    #[serde(rename = "error")]
    Error {
        #[serde(default)]
        error: Option<Value>,
        #[serde(default)]
        code: Option<String>,
        #[serde(default)]
        message: Option<String>,
        #[serde(default)]
        sequence_number: Option<i32>,
    },
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ResponseMetadata {
    pub id: String,
    pub object: String,
    pub created_at: i64,
    pub status: String,
    pub model: String,
    pub output: Vec<ResponseOutputItemInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<ResponseUsage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<ResponseReasoningInfo>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(tag = "type")]
#[serde(rename_all = "snake_case")]
pub enum ResponseOutputItemInfo {
    Reasoning {
        id: String,
        summary: Vec<String>,
    },
    Message {
        id: String,
        status: String,
        role: String,
        content: Vec<ContentPart>,
    },
    FunctionCall {
        id: String,
        status: String,
        call_id: String,
        name: String,
        arguments: String,
    },
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(tag = "type")]
#[serde(rename_all = "snake_case")]
pub enum ContentPart {
    OutputText {
        text: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        annotations: Option<Vec<Value>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        logprobs: Option<Vec<Value>>,
    },
    ToolCall {
        id: String,
        name: String,
        arguments: String,
    },
}

fn add_conversation_history(input_items: &mut Vec<Value>, messages: &[Message]) {
    for message in messages.iter().filter(|m| m.is_agent_visible()) {
        let has_only_tool_content = message.content.iter().all(|c| {
            matches!(
                c,
                MessageContent::ToolRequest(_) | MessageContent::ToolResponse(_)
            )
        });

        if has_only_tool_content {
            continue;
        }

        if message.role != Role::User && message.role != Role::Assistant {
            continue;
        }

        let role = match message.role {
            Role::User => "user",
            Role::Assistant => "assistant",
        };

        let mut content_items = Vec::new();
        for content in &message.content {
            if let MessageContent::Text(text) = content {
                if !text.text.is_empty() {
                    let content_type = if message.role == Role::Assistant {
                        "output_text"
                    } else {
                        "input_text"
                    };
                    content_items.push(json!({
                        "type": content_type,
                        "text": text.text
                    }));
                }
            }
        }

        if !content_items.is_empty() {
            input_items.push(json!({
                "role": role,
                "content": content_items
            }));
        }
    }
}

fn add_function_calls(input_items: &mut Vec<Value>, messages: &[Message]) {
    for message in messages.iter().filter(|m| m.is_agent_visible()) {
        if message.role == Role::Assistant {
            for content in &message.content {
                if let MessageContent::ToolRequest(request) = content {
                    if let Ok(tool_call) = &request.tool_call {
                        let arguments_str = tool_call
                            .arguments
                            .as_ref()
                            .map(|args| {
                                serde_json::to_string(args).unwrap_or_else(|_| "{}".to_string())
                            })
                            .unwrap_or_else(|| "{}".to_string());

                        tracing::debug!(
                            "Replaying function_call with call_id: {}, name: {}",
                            request.id,
                            tool_call.name
                        );
                        input_items.push(json!({
                            "type": "function_call",
                            "call_id": request.id,
                            "name": tool_call.name,
                            "arguments": arguments_str
                        }));
                    }
                }
            }
        }
    }
}

fn add_function_call_outputs(input_items: &mut Vec<Value>, messages: &[Message]) {
    for message in messages.iter().filter(|m| m.is_agent_visible()) {
        for content in &message.content {
            if let MessageContent::ToolResponse(response) = content {
                match &response.tool_result {
                    Ok(contents) => {
                        let mut text_parts = Vec::new();
                        let mut has_images = false;

                        for c in &contents.content {
                            match c.deref() {
                                RawContent::Text(t) => {
                                    text_parts.push(t.text.clone());
                                }
                                RawContent::Image(_) => {
                                    has_images = true;
                                }
                                _ => {}
                            }
                        }

                        if has_images {
                            text_parts.push("[Image content included in tool result]".to_string());
                        }

                        if !text_parts.is_empty() {
                            tracing::debug!(
                                "Sending function_call_output with call_id: {}",
                                response.id
                            );
                            input_items.push(json!({
                                "type": "function_call_output",
                                "call_id": response.id,
                                "output": text_parts.join("\n")
                            }));
                        }
                    }
                    Err(error_data) => {
                        // Handle error responses - must send them back to the API
                        // to avoid "No tool output found" errors
                        tracing::debug!(
                            "Sending function_call_output error with call_id: {}",
                            response.id
                        );
                        input_items.push(json!({
                            "type": "function_call_output",
                            "call_id": response.id,
                            "output": format!("Error: {}", error_data.message)
                        }));
                    }
                }
            }
        }
    }
}

fn tool_choice_mode_to_openai_value(mode: Option<ToolChoiceMode>) -> Option<Value> {
    match mode {
        Some(ToolChoiceMode::Auto) => Some(json!("auto")),
        Some(ToolChoiceMode::Required) => Some(json!("required")),
        Some(ToolChoiceMode::None) => Some(json!("none")),
        None => None,
    }
}

pub fn create_responses_request_with_tool_choice(
    model_config: &ModelConfig,
    system: &str,
    messages: &[Message],
    tools: &[Tool],
    tool_choice_mode: Option<ToolChoiceMode>,
) -> anyhow::Result<Value, Error> {
    let caps = crate::capabilities::resolve_with_thinking_override(
        &model_config.model_name,
        model_config.thinking_enabled,
        model_config.thinking_budget,
    );

    let mut input_items = Vec::new();

    if !system.is_empty() {
        input_items.push(json!({
            "role": "system",
            "content": [{
                "type": "input_text",
                "text": system
            }]
        }));
    }

    add_conversation_history(&mut input_items, messages);
    add_function_calls(&mut input_items, messages);
    add_function_call_outputs(&mut input_items, messages);

    let mut payload = json!({
        "model": model_config.model_name,
        "input": input_items,
        "store": false,  // Don't store responses on server (we replay history ourselves)
    });

    if !tools.is_empty() {
        let tools_spec: Vec<Value> = tools
            .iter()
            .map(|tool| {
                json!({
                    "type": "function",
                    "name": tool.name,
                    "description": tool.description,
                    "parameters": tool.input_schema,
                })
            })
            .collect();

        payload
            .as_object_mut()
            .unwrap()
            .insert("tools".to_string(), json!(tools_spec));
        if let Some(tool_choice) = tool_choice_mode_to_openai_value(tool_choice_mode) {
            payload
                .as_object_mut()
                .unwrap()
                .insert("tool_choice".to_string(), tool_choice);
        }
    }

    if caps.effective_temperature_supported() {
        if let Some(temp) = model_config.temperature {
            payload
                .as_object_mut()
                .unwrap()
                .insert("temperature".to_string(), json!(temp));
        }
    }

    if let Some(tokens) = model_config.max_tokens {
        payload
            .as_object_mut()
            .unwrap()
            .insert("max_output_tokens".to_string(), json!(tokens));
    }

    ThinkingHandler::apply_request_params(&mut payload, &caps)?;

    Ok(payload)
}

pub fn create_responses_request(
    model_config: &ModelConfig,
    system: &str,
    messages: &[Message],
    tools: &[Tool],
) -> anyhow::Result<Value, Error> {
    create_responses_request_with_tool_choice(model_config, system, messages, tools, None)
}

pub fn responses_api_to_message(response: &ResponsesApiResponse) -> anyhow::Result<Message> {
    let mut content = Vec::new();

    for item in &response.output {
        match item {
            ResponseOutputItem::Reasoning { summary, .. } => {
                // Convert reasoning summary to thinking content for display
                if let Some(summaries) = summary {
                    let thinking_text = summaries.join("\n");
                    if !thinking_text.is_empty() {
                        content.push(MessageContent::thinking(thinking_text, String::new()));
                    }
                }
            }
            ResponseOutputItem::Message {
                content: msg_content,
                ..
            } => {
                for block in msg_content {
                    match block {
                        ResponseContentBlock::OutputText { text, .. } => {
                            if !text.is_empty() {
                                content.push(MessageContent::text(text));
                            }
                        }
                        ResponseContentBlock::ToolCall { id, name, input } => {
                            content.push(MessageContent::tool_request(
                                id.clone(),
                                Ok(CallToolRequestParams {
                                    name: name.clone().into(),
                                    arguments: Some(object(input.clone())),
                                    meta: None,
                                    task: None,
                                }),
                            ));
                        }
                    }
                }
            }
            ResponseOutputItem::FunctionCall {
                id,
                name,
                arguments,
                ..
            } => {
                tracing::debug!("Received FunctionCall with id: {}, name: {}", id, name);
                let parsed_args = if arguments.is_empty() {
                    json!({})
                } else {
                    serde_json::from_str(arguments).unwrap_or_else(|_| json!({}))
                };

                content.push(MessageContent::tool_request(
                    id.clone(),
                    Ok(CallToolRequestParams {
                        name: name.clone().into(),
                        arguments: Some(object(parsed_args)),
                        meta: None,
                        task: None,
                    }),
                ));
            }
        }
    }

    let mut message = Message::new(Role::Assistant, chrono::Utc::now().timestamp(), content);

    message = message.with_id(response.id.clone());

    Ok(message)
}

pub fn get_responses_usage(response: &ResponsesApiResponse) -> Usage {
    response.usage.as_ref().map_or_else(Usage::default, |u| {
        Usage::new(
            Some(u.input_tokens),
            Some(u.output_tokens),
            Some(u.total_tokens),
        )
    })
}

fn process_streaming_output_items(
    output_items: Vec<ResponseOutputItemInfo>,
    is_text_response: bool,
) -> Vec<MessageContent> {
    let mut content = Vec::new();

    for item in output_items {
        match item {
            ResponseOutputItemInfo::Reasoning { .. } => {
                // Skip reasoning items
            }
            ResponseOutputItemInfo::Message { content: parts, .. } => {
                for part in parts {
                    match part {
                        ContentPart::OutputText { text, .. } => {
                            if !text.is_empty() && !is_text_response {
                                content.push(MessageContent::text(&text));
                            }
                        }
                        ContentPart::ToolCall {
                            id,
                            name,
                            arguments,
                        } => {
                            let parsed_args = if arguments.is_empty() {
                                json!({})
                            } else {
                                serde_json::from_str(&arguments).unwrap_or_else(|_| json!({}))
                            };

                            content.push(MessageContent::tool_request(
                                id,
                                Ok(CallToolRequestParams {
                                    name: name.into(),
                                    arguments: Some(object(parsed_args)),
                                    meta: None,
                                    task: None,
                                }),
                            ));
                        }
                    }
                }
            }
            ResponseOutputItemInfo::FunctionCall {
                call_id,
                name,
                arguments,
                ..
            } => {
                let parsed_args = if arguments.is_empty() {
                    json!({})
                } else {
                    serde_json::from_str(&arguments).unwrap_or_else(|_| json!({}))
                };

                content.push(MessageContent::tool_request(
                    call_id,
                    Ok(CallToolRequestParams {
                        name: name.into(),
                        arguments: Some(object(parsed_args)),
                        meta: None,
                        task: None,
                    }),
                ));
            }
        }
    }

    content
}

pub fn responses_api_to_streaming_message<S>(
    mut stream: S,
) -> impl Stream<Item = anyhow::Result<(Option<Message>, Option<ProviderUsage>)>> + 'static
where
    S: Stream<Item = anyhow::Result<String>> + Unpin + Send + 'static,
{
    try_stream! {
        use futures::StreamExt;

        let mut accumulated_text = String::new();
        let mut response_id: Option<String> = None;
        let mut model_name: Option<String> = None;
        let mut final_usage: Option<ProviderUsage> = None;
        let mut output_items: Vec<ResponseOutputItemInfo> = Vec::new();
        let mut is_text_response = false;

        'outer: while let Some(response) = stream.next().await {
            let response_str = response?;

            // Skip empty lines
            if response_str.trim().is_empty() {
                continue;
            }

            // Parse SSE format: "event: <type>\ndata: <json>"
            // For now, we only care about the data line
            let data_line = if response_str.starts_with("data: ") {
                response_str.strip_prefix("data: ").unwrap()
            } else if response_str.starts_with("event: ") {
                // Skip event type lines
                continue;
            } else {
                // Try to parse as-is in case there's no prefix
                &response_str
            };

            if data_line == "[DONE]" {
                break 'outer;
            }

            let event: ResponsesStreamEvent = serde_json::from_str(data_line)
                .map_err(|e| anyhow!("Failed to parse Responses stream event: {}: {:?}", e, data_line))?;

            match event {
                ResponsesStreamEvent::KeepAlive { .. } => {
                    // Some OpenAI-compatible providers emit heartbeat frames between real
                    // Responses API events. They should be ignored rather than forcing a
                    // fallback to non-streaming completion.
                }
                ResponsesStreamEvent::ResponseCreated { response, .. } |
                ResponsesStreamEvent::ResponseInProgress { response, .. } => {
                    response_id = Some(response.id);
                    model_name = Some(response.model);
                }

                ResponsesStreamEvent::OutputTextDelta { delta, .. } => {
                    is_text_response = true;
                    accumulated_text.push_str(&delta);

                    // Yield incremental text updates for true streaming
                    let mut content = Vec::new();
                    if !delta.is_empty() {
                        content.push(MessageContent::text(&delta));
                    }
                    let mut msg = Message::new(Role::Assistant, chrono::Utc::now().timestamp(), content);

                    // Add ID so desktop client knows these deltas are part of the same message
                    if let Some(id) = &response_id {
                        msg = msg.with_id(id.clone());
                    }

                    yield (Some(msg), None);
                }

                ResponsesStreamEvent::OutputItemDone { item, .. } => {
                    output_items.push(item);
                }

                ResponsesStreamEvent::OutputTextDone { .. } => {
                    // Text is already complete from deltas, this is just a summary event
                }

                ResponsesStreamEvent::ResponseCompleted { response, .. } => {
                    let model = model_name.as_ref().unwrap_or(&response.model);
                    let usage = response.usage.as_ref().map_or_else(
                        Usage::default,
                        |u| Usage::new(
                            Some(u.input_tokens),
                            Some(u.output_tokens),
                            Some(u.total_tokens),
                        ),
                    );
                    final_usage = Some(ProviderUsage {
                        usage,
                        model: model.clone(),
                    });

                    // For complete output, use the response output items
                    if !response.output.is_empty() {
                        output_items = response.output;
                    }

                    break 'outer;
                }

                ResponsesStreamEvent::FunctionCallArgumentsDelta { .. } => {
                    // Function call arguments are being streamed, but we'll get the complete
                    // arguments in the OutputItemDone event, so we can ignore deltas for now
                }

                ResponsesStreamEvent::FunctionCallArgumentsDone { .. } => {
                    // Arguments are complete, will be in the OutputItemDone event
                }

                ResponsesStreamEvent::ResponseFailed { error, .. } => {
                    Err(anyhow!("Responses API failed: {:?}", error))?;
                }

                ResponsesStreamEvent::Error {
                    error,
                    code,
                    message,
                    sequence_number,
                } => {
                    let payload = error.unwrap_or_else(|| {
                        json!({
                            "code": code,
                            "message": message,
                            "sequence_number": sequence_number,
                        })
                    });
                    Err(anyhow!("Responses API error: {:?}", payload))?;
                }

                _ => {
                    // Ignore other event types (OutputItemAdded, ContentPartAdded, ContentPartDone)
                }
            }
        }

        // Process final output items and yield usage data
        let content = process_streaming_output_items(output_items, is_text_response);

        if !content.is_empty() {
            let mut message = Message::new(Role::Assistant, chrono::Utc::now().timestamp(), content);
            if let Some(id) = response_id {
                message = message.with_id(id);
            }
            yield (Some(message), final_usage);
        } else if let Some(usage) = final_usage {
            yield (None, Some(usage));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::{stream, StreamExt};
    use rmcp::model::ToolChoiceMode;

    #[test]
    fn test_create_responses_request_with_tool_choice_required() -> anyhow::Result<()> {
        let model_config = ModelConfig {
            model_name: "gpt-4.1".to_string(),
            context_limit: Some(4096),
            temperature: None,
            max_tokens: Some(1024),
            thinking_enabled: None,
            thinking_budget: None,
            toolshim: false,
            toolshim_model: None,
            fast_model: None,
        };

        let tool = Tool::new(
            "test_tool",
            "A test tool",
            rmcp::object!({
                "type": "object",
                "properties": {
                    "input": { "type": "string" }
                }
            }),
        );

        let payload = create_responses_request_with_tool_choice(
            &model_config,
            "system",
            &[],
            &[tool],
            Some(ToolChoiceMode::Required),
        )?;

        assert_eq!(payload["tool_choice"], "required");
        Ok(())
    }

    #[tokio::test]
    async fn test_responses_stream_ignores_keepalive_frames() {
        let stream = stream::iter(vec![
            Ok::<_, anyhow::Error>("data: {\"type\":\"keepalive\",\"sequence_number\":1}".to_string()),
            Ok::<_, anyhow::Error>("data: {\"type\":\"response.created\",\"sequence_number\":2,\"response\":{\"id\":\"resp_1\",\"object\":\"response\",\"created_at\":1,\"status\":\"in_progress\",\"model\":\"gpt-5.2\",\"output\":[]}}".to_string()),
            Ok::<_, anyhow::Error>("data: {\"type\":\"response.output_text.delta\",\"sequence_number\":3,\"item_id\":\"msg_1\",\"output_index\":0,\"content_index\":0,\"delta\":\"hello\"}".to_string()),
            Ok::<_, anyhow::Error>("data: {\"type\":\"response.completed\",\"sequence_number\":4,\"response\":{\"id\":\"resp_1\",\"object\":\"response\",\"created_at\":1,\"status\":\"completed\",\"model\":\"gpt-5.2\",\"output\":[],\"usage\":{\"input_tokens\":1,\"output_tokens\":1,\"total_tokens\":2}}}".to_string()),
            Ok::<_, anyhow::Error>("data: [DONE]".to_string()),
        ]);

        let results: Vec<_> = responses_api_to_streaming_message(stream).collect().await;
        assert_eq!(results.len(), 2);

        let first = results[0].as_ref().expect("first item should parse");
        let first_text = first
            .0
            .as_ref()
            .expect("first item should contain a message")
            .as_concat_text();
        assert_eq!(first_text, "hello");

        let second = results[1].as_ref().expect("final usage should parse");
        assert!(second.1.is_some());
    }

    #[tokio::test]
    async fn test_responses_stream_accepts_flat_error_events() {
        let stream = stream::iter(vec![Ok::<_, anyhow::Error>(
            "data: {\"type\":\"error\",\"code\":\"internal_server_error\",\"message\":\"unexpected EOF\",\"sequence_number\":0}".to_string(),
        )]);

        let results: Vec<_> = responses_api_to_streaming_message(stream).collect().await;
        assert_eq!(results.len(), 1);

        let err = results[0]
            .as_ref()
            .expect_err("flat error event should surface as provider error");
        let text = err.to_string();
        assert!(text.contains("internal_server_error"));
        assert!(text.contains("unexpected EOF"));
    }
}
