use crate::conversation::message::Message;
use crate::prompt_template::render_global_file;
use crate::providers::base::Provider;
use crate::session::session_manager::MemoryFactDraft;
use anyhow::Result;
use serde::Deserialize;
use std::collections::HashMap;
use tracing::warn;

const CFPM_LLM_V2_SOURCE: &str = "cfpm_llm_v2";

#[derive(Deserialize)]
struct ExtractionResponse(HashMap<String, Vec<String>>);

/// Parse LLM extraction response JSON into MemoryFactDrafts.
pub fn parse_extraction_response(text: &str) -> Result<Vec<MemoryFactDraft>> {
    let trimmed = text.trim();

    // Try direct JSON parse first, then try extracting ```json block
    let json_str = if trimmed.starts_with('{') {
        trimmed.to_string()
    } else if let Some((_, after_json_fence)) = trimmed.split_once("```json") {
        let block = after_json_fence
            .split_once("```")
            .map(|(json, _)| json)
            .unwrap_or(after_json_fence);
        block.trim().to_string()
    } else if let Some((start, _)) = trimmed.char_indices().find(|(_, ch)| *ch == '{') {
        let end = trimmed
            .char_indices()
            .rfind(|(_, ch)| *ch == '}')
            .map(|(idx, _)| idx)
            .unwrap_or(start);
        trimmed
            .get(start..=end)
            .map(str::to_string)
            .ok_or_else(|| anyhow::anyhow!("No JSON found in extraction response"))?
    } else {
        return Err(anyhow::anyhow!("No JSON found in extraction response"));
    };

    let parsed: ExtractionResponse = serde_json::from_str(&json_str)?;
    let valid_categories = [
        "goal",
        "decision",
        "artifact",
        "open_item",
        "working_state",
        "invalid_path",
    ];

    let mut drafts = Vec::new();
    for (category, items) in &parsed.0 {
        if !valid_categories.contains(&category.as_str()) {
            continue;
        }
        for item in items.iter().take(3) {
            let content = item.chars().take(180).collect::<String>();
            if content.trim().is_empty() {
                continue;
            }
            drafts.push(MemoryFactDraft::new(
                category.clone(),
                content,
                CFPM_LLM_V2_SOURCE,
            ));
        }
    }
    Ok(drafts)
}

/// Extract memory facts from conversation using LLM.
pub async fn extract_memory_facts_via_llm(
    provider: &dyn Provider,
    messages: &[Message],
) -> Result<Vec<MemoryFactDraft>> {
    let system_prompt = render_global_file("cfpm_extract_v2.md", &serde_json::json!({}))?;

    let (response, _usage) = provider
        .complete_fast(&system_prompt, messages, &[])
        .await
        .map_err(|e| anyhow::anyhow!("LLM extraction failed: {}", e))?;

    let response_text = response.as_concat_text();
    if response_text.trim().is_empty() {
        return Ok(Vec::new());
    }

    match parse_extraction_response(&response_text) {
        Ok(drafts) => Ok(drafts),
        Err(e) => {
            warn!("Failed to parse CFPM V2 extraction response: {}", e);
            Ok(Vec::new())
        }
    }
}
