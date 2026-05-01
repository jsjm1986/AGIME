//! Document Analysis Trigger — auto-interpret uploaded documents via Agent
//!
//! When a document is uploaded, spawns an async task that:
//! 1. Creates a temporary chat session with the team's agent
//! 2. Sends a message asking the agent to analyze the document
//! 3. Agent uses its full toolset (DocumentTools, Developer shell, etc.)
//! 4. Extracts the agent's response and stores it as a smart log entry

use agime_team::models::mongo::{
    is_image_document_type, DocumentAnalysisContext, DocumentAnalysisSettings,
    DocumentAnalysisTrigger, SmartLogContext, SmartLogEntry,
};
use agime_team::services::mongo::{DocumentService, SmartLogService, TeamService};
use agime_team::MongoDb;
use mongodb::bson::{doc, oid::ObjectId};
use mongodb::options::FindOneOptions;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;

use super::workspace_runtime;
use super::server_harness_host::ServerHarnessHost;
use super::service_mongo::AgentService;
use super::session_mongo::CreateSessionRequest;
use super::task_manager::create_task_manager;
use super::workspace_service::WorkspaceService;
use agime::agents::format_execution_host_completion_text;

const MAX_ANALYSIS_CHARS: usize = 3000;
const AGENT_TIMEOUT_SECS: u64 = 600;

#[derive(Debug, Clone)]
struct MaterializedAnalysisDocument {
    file_path: String,
    relative_path: String,
    file_name: String,
    mime_type: String,
    file_size: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct DocumentAnalysisJsonResult {
    status: String,
    summary: String,
    #[serde(default)]
    content_structure: Vec<String>,
    #[serde(default)]
    key_points: Vec<String>,
    #[serde(default)]
    file_observations: Vec<String>,
    #[serde(default)]
    limitations: Vec<String>,
}

fn derive_document_analysis_persistence(
    completion_report: Option<&agime::agents::ExecutionHostCompletionReport>,
    analysis_text: Option<&str>,
    doc_name: &str,
) -> (String, &'static str) {
    let candidate = analysis_text
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_string)
        .or_else(|| completion_report.map(|report| format_execution_host_completion_text(report)));

    let Some(text) = candidate else {
        return blocked_document_analysis_json(
            "文档解读未产生最终结果。",
            vec![format!("未能完成文档「{}」的解读。", doc_name)],
        );
    };

    if let Some(parsed) = parse_document_analysis_json(&text) {
        return persist_document_analysis_json(parsed);
    }

    if document_analysis_text_is_terminal(&text) {
        return completed_document_analysis_json(text);
    }

    blocked_document_analysis_json(
        "文档解读未完成。",
        vec![text.chars().take(MAX_ANALYSIS_CHARS).collect()],
    )
}

fn select_document_analysis_text(
    completion_report: Option<&agime::agents::ExecutionHostCompletionReport>,
    visible_text: Option<String>,
) -> Option<String> {
    if visible_text
        .as_deref()
        .is_some_and(document_analysis_text_is_terminal)
    {
        return visible_text;
    }

    if let Some(report) = completion_report.filter(|report| report.status == "completed") {
        return Some(report.summary.clone());
    }

    visible_text.or_else(|| completion_report.map(format_execution_host_completion_text))
}

fn document_analysis_text_is_terminal(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return false;
    }
    let normalized = trimmed.to_ascii_lowercase();
    let non_terminal_patterns = [
        "runtime exited without terminal completion evidence",
        "tool execution completed",
        "validation: failed",
        "blocking reason:",
        "no tool-backed progress",
        "let me ",
        "i need to ",
        "i will ",
        "i'll ",
        "need to extract",
        "need to read",
        "need to inspect",
        "not available",
        "cannot read",
        "could not read",
        "unable to read",
        "我需要",
        "我需要先",
        "让我",
        "我将",
        "下一步",
        "需要解压",
        "需要读取",
        "不可用",
        "无法读取",
        "无法访问",
        "无法解读",
    ];

    !non_terminal_patterns
        .iter()
        .any(|pattern| normalized.contains(pattern))
}

fn parse_document_analysis_json(text: &str) -> Option<DocumentAnalysisJsonResult> {
    let candidate = extract_json_object(text)?;
    serde_json::from_str::<DocumentAnalysisJsonResult>(&candidate)
        .ok()
        .or_else(|| repair_document_analysis_jsonish(&candidate))
}

fn extract_json_object(text: &str) -> Option<String> {
    let trimmed = text.trim();
    let unfenced = trimmed
        .strip_prefix("```json")
        .and_then(|value| value.strip_suffix("```"))
        .or_else(|| {
            trimmed
                .strip_prefix("```")
                .and_then(|value| value.strip_suffix("```"))
        })
        .map(str::trim)
        .unwrap_or(trimmed);

    if unfenced.starts_with('{') && unfenced.ends_with('}') {
        return Some(unfenced.to_string());
    }

    let start = unfenced.find('{')?;
    let end = unfenced.rfind('}')?;
    (start < end).then(|| unfenced[start..=end].to_string())
}

fn persist_document_analysis_json(result: DocumentAnalysisJsonResult) -> (String, &'static str) {
    let result = normalize_document_analysis_result(result);
    let blocked = !result.status.eq_ignore_ascii_case("completed")
        || !document_analysis_text_is_terminal(&result.summary);
    let status = if blocked { "failed" } else { "completed" };
    (serialize_document_analysis_json(result), status)
}

fn normalize_document_analysis_result(
    mut result: DocumentAnalysisJsonResult,
) -> DocumentAnalysisJsonResult {
    if result.status.eq_ignore_ascii_case("blocked")
        && blocked_result_focuses_on_container(&result)
    {
        let original_summary = result.summary.trim().to_string();
        result.summary = "无法解压或读取压缩包内部的文档内容，因此无法生成文件内容解读。".to_string();
        result.content_structure.clear();
        result.key_points.clear();
        result.file_observations.clear();
        if !original_summary.is_empty()
            && !result
                .limitations
                .iter()
                .any(|item| item.contains(&original_summary))
        {
            result
                .limitations
                .insert(0, format!("读取受阻详情：{}", original_summary));
        }
    }
    result
}

fn blocked_result_focuses_on_container(result: &DocumentAnalysisJsonResult) -> bool {
    let combined = std::iter::once(result.summary.as_str())
        .chain(result.content_structure.iter().map(String::as_str))
        .chain(result.key_points.iter().map(String::as_str))
        .chain(result.file_observations.iter().map(String::as_str))
        .collect::<Vec<_>>()
        .join("\n")
        .to_ascii_lowercase();
    let container_terms = [
        "rar magic",
        "magic signature",
        "magic bytes",
        "file signature",
        "archive signature",
        "魔术签名",
        "文件签名",
        "rar 签名",
        "魔数",
        "ascii 明文",
        "not-a-real-rar",
        "synthetic test",
        "合成测试",
        "占位文件",
    ];
    container_terms
        .iter()
        .any(|term| combined.contains(&term.to_ascii_lowercase()))
}

fn repair_document_analysis_jsonish(text: &str) -> Option<DocumentAnalysisJsonResult> {
    if !text.contains("\"status\"") || !text.contains("\"summary\"") {
        return None;
    }

    let status = extract_jsonish_string_field(text, "status")
        .filter(|value| value.eq_ignore_ascii_case("completed") || value.eq_ignore_ascii_case("blocked"))
        .unwrap_or_else(|| "blocked".to_string());
    let summary = extract_jsonish_string_field_before(
        text,
        "summary",
        &[
            "\",\"content_structure\"",
            "\",\"key_points\"",
            "\",\"file_observations\"",
            "\",\"limitations\"",
        ],
    )
    .or_else(|| extract_jsonish_string_field(text, "summary"))?;

    Some(DocumentAnalysisJsonResult {
        status,
        summary,
        content_structure: Vec::new(),
        key_points: Vec::new(),
        file_observations: Vec::new(),
        limitations: Vec::new(),
    })
}

fn extract_jsonish_string_field(text: &str, field: &str) -> Option<String> {
    let marker = format!("\"{}\":\"", field);
    let start = text.find(&marker)? + marker.len();
    let rest = &text[start..];
    let end = rest.find('"')?;
    Some(unescape_jsonish_text(&rest[..end]))
}

fn extract_jsonish_string_field_before(
    text: &str,
    field: &str,
    delimiters: &[&str],
) -> Option<String> {
    let marker = format!("\"{}\":\"", field);
    let start = text.find(&marker)? + marker.len();
    let rest = &text[start..];
    let end = delimiters
        .iter()
        .filter_map(|delimiter| rest.find(delimiter))
        .min()?;
    Some(unescape_jsonish_text(&rest[..end]))
}

fn unescape_jsonish_text(value: &str) -> String {
    value
        .replace("\\\"", "\"")
        .replace("\\n", "\n")
        .replace("\\\\", "\\")
}

fn completed_document_analysis_json(summary: String) -> (String, &'static str) {
    let result = DocumentAnalysisJsonResult {
        status: "completed".to_string(),
        summary,
        content_structure: Vec::new(),
        key_points: Vec::new(),
        file_observations: Vec::new(),
        limitations: Vec::new(),
    };
    (serialize_document_analysis_json(result), "completed")
}

fn blocked_document_analysis_json(
    summary: &str,
    limitations: Vec<String>,
) -> (String, &'static str) {
    let result = DocumentAnalysisJsonResult {
        status: "blocked".to_string(),
        summary: summary.to_string(),
        content_structure: Vec::new(),
        key_points: Vec::new(),
        file_observations: Vec::new(),
        limitations,
    };
    (serialize_document_analysis_json(result), "failed")
}

fn serialize_document_analysis_json(result: DocumentAnalysisJsonResult) -> String {
    serde_json::to_string_pretty(&result)
        .unwrap_or_else(|_| "{\"status\":\"blocked\",\"summary\":\"文档解读结果序列化失败。\",\"content_structure\":[],\"key_points\":[],\"file_observations\":[],\"limitations\":[\"serialization failed\"]}".to_string())
        .chars()
        .take(MAX_ANALYSIS_CHARS)
        .collect()
}

pub struct DocumentAnalysisTriggerImpl {
    db: Arc<MongoDb>,
    agent_service: Arc<AgentService>,
    workspace_root: String,
    semaphore: Arc<Semaphore>,
}

impl DocumentAnalysisTriggerImpl {
    pub fn new(db: Arc<MongoDb>, agent_service: Arc<AgentService>, workspace_root: String) -> Self {
        Self {
            db,
            agent_service,
            workspace_root,
            semaphore: Arc::new(Semaphore::new(4)),
        }
    }
}

impl DocumentAnalysisTrigger for DocumentAnalysisTriggerImpl {
    fn trigger(&self, ctx: DocumentAnalysisContext) {
        let db = self.db.clone();
        let agent_svc = self.agent_service.clone();
        let sem = self.semaphore.clone();
        let workspace_root = self.workspace_root.clone();

        tokio::spawn(async move {
            if let Err(e) = process_document_analysis(db, agent_svc, sem, workspace_root, ctx).await
            {
                tracing::warn!("Document analysis failed: {}", e);
            }
        });
    }
}

async fn process_document_analysis(
    db: Arc<MongoDb>,
    agent_svc: Arc<AgentService>,
    sem: Arc<Semaphore>,
    workspace_root: String,
    ctx: DocumentAnalysisContext,
) -> anyhow::Result<()> {
    // Load team config (fallback to defaults on error)
    tracing::info!(
        "[doc-analysis] START doc={} team={} mime={} size={}",
        ctx.doc_id,
        ctx.team_id,
        ctx.mime_type,
        ctx.file_size
    );
    let team_svc = TeamService::new((*db).clone());
    let config = match team_svc.get_settings(&ctx.team_id).await {
        Ok(s) => {
            tracing::info!(
                "[doc-analysis] Config loaded: enabled={} agent_id={:?} has_api_url={}",
                s.document_analysis.enabled,
                s.document_analysis.agent_id,
                s.document_analysis.api_url.is_some()
            );
            s.document_analysis
        }
        Err(e) => {
            tracing::warn!(
                "[doc-analysis] Failed to load settings, using defaults: {}",
                e
            );
            DocumentAnalysisSettings::default()
        }
    };

    let smart_log_svc = SmartLogService::new((*db).clone());

    // Check whether analysis should proceed; cancel the pending log entry on skip.
    let skip_reason = if !config.enabled {
        Some(format!(
            "Document analysis disabled for team {}",
            ctx.team_id
        ))
    } else if is_image_document_type(&ctx.mime_type, &ctx.doc_name) {
        Some(format!(
            "Skipping automatic analysis for image document: {} ({})",
            ctx.doc_name, ctx.mime_type
        ))
    } else if should_skip_mime(&ctx.mime_type, &config.skip_mime_prefixes) {
        Some(format!(
            "Skipping analysis for MIME type: {}",
            ctx.mime_type
        ))
    } else if ctx.file_size < config.min_file_size {
        Some(format!(
            "Skipping analysis for tiny file ({} bytes)",
            ctx.file_size
        ))
    } else if config.max_file_size.is_some_and(|max| ctx.file_size > max) {
        Some(format!(
            "Skipping analysis for large file ({} bytes)",
            ctx.file_size
        ))
    } else {
        None
    };
    if let Some(reason) = skip_reason {
        tracing::debug!("{}", reason);
        let _ = if is_image_document_type(&ctx.mime_type, &ctx.doc_name) {
            smart_log_svc.clear_analysis(&ctx.team_id, &ctx.doc_id).await
        } else {
            smart_log_svc
                .cancel_pending_analysis(&ctx.team_id, &ctx.doc_id)
                .await
        };
        return Ok(());
    }

    // Agent selection: config.agent_id → fallback to first with key
    let agent = match &config.agent_id {
        Some(aid) => agent_svc
            .get_agent_with_key(aid)
            .await?
            .filter(|a| a.api_key.is_some()),
        None => None,
    };
    let agent = match agent {
        Some(a) => a,
        None => match agent_svc.get_first_agent_with_key(&ctx.team_id).await? {
            Some(a) => a,
            None => {
                tracing::warn!(
                    "[doc-analysis] No agent with API key for team {}",
                    ctx.team_id
                );
                let _ = smart_log_svc
                    .cancel_pending_analysis(&ctx.team_id, &ctx.doc_id)
                    .await;
                return Ok(());
            }
        },
    };
    tracing::info!(
        "[doc-analysis] Using agent={} api_format={:?} has_key={}",
        agent.id,
        agent.api_format,
        agent.api_key.is_some()
    );

    let _permit = sem.acquire().await?;

    let session = agent_svc
        .create_session(CreateSessionRequest {
            team_id: ctx.team_id.clone(),
            agent_id: agent.id.clone(),
            user_id: ctx.user_id.clone(),
            name: Some(format!("doc-analysis-{}", ctx.doc_id)),
            attached_document_ids: vec![ctx.doc_id.clone()],
            extra_instructions: None,
            // The document is pre-materialized below; document_tools stays available only
            // for optional inventory/reference lookups during deeper analysis.
            allowed_extensions: Some(vec!["document_tools".to_string(), "developer".to_string()]),
            allowed_skill_ids: None,
            retry_config: None,
            max_turns: None,
            tool_timeout_seconds: None,
            max_portal_retry_rounds: None,
            require_final_report: false,
            portal_restricted: false,
            document_access_mode: None,
            document_scope_mode: None,
            document_write_mode: None,
            delegation_policy_override: None,
            session_source: Some("document_analysis".to_string()),
            source_channel_id: None,
            source_channel_name: None,
            source_thread_root_id: None,
            hidden_from_chat_list: Some(true),
        })
        .await?;

    let session_id = session.session_id.clone();

    let workspace_binding = WorkspaceService::new(workspace_root.clone())
        .ensure_document_analysis_workspace(&session, &ctx)?;
    if let Err(e) = agent_svc
        .set_session_workspace_binding(&session_id, &workspace_binding)
        .await
    {
        tracing::warn!(
            "Failed to bind workspace metadata for session {}: {}",
            session_id,
            e
        );
    }
    let workspace_path = workspace_binding.root_path;

    let materialized_document = match materialize_document_for_analysis(&db, &ctx, &workspace_path)
        .await
    {
        Ok(value) => value,
        Err(error) => {
            let text = format!("文档导出到分析工作区失败：{}", error);
            let _ = agent_svc.delete_session(&session_id).await;
            let _ = workspace_runtime::cleanup_workspace_dir(Some(&workspace_path));
            let attached = smart_log_svc
                .attach_analysis(&ctx.team_id, &ctx.doc_id, &text, "failed")
                .await?;
            if !attached {
                let smart_ctx = SmartLogContext::ai_fallback(ctx.team_id, ctx.doc_id, ctx.doc_name);
                let entry_id = smart_log_svc.create(&smart_ctx).await?;
                smart_log_svc
                    .update_ai_summary(&entry_id, &text, "failed")
                    .await?;
            }
            return Ok(());
        }
    };

    let content_snapshot = load_latest_smart_log_content_snapshot(&db, &ctx)
        .await
        .unwrap_or_else(|error| {
            tracing::warn!(
                "[doc-analysis] failed to load smart log content snapshot for doc={}: {}",
                ctx.doc_id,
                error
            );
            None
        });
    let mut user_message = build_analysis_prompt(
        &ctx,
        Some(&materialized_document),
        content_snapshot.as_deref(),
    );
    if let Some(extra) = &ctx.extra_instructions {
        user_message.push_str("\n\n## 用户补充要求\n");
        user_message.push_str(extra);
    }

    let cancel_token = CancellationToken::new();

    // Build LLM overrides only when a custom API URL is configured (standalone API mode).
    // Without api_url, the other fields (api_key, model, api_format) would incorrectly
    // override the selected agent's own credentials.
    let llm_overrides = match config.api_url.as_deref().filter(|s| !s.is_empty()) {
        Some(url) => {
            let mut ov = serde_json::Map::new();
            ov.insert("api_url".into(), serde_json::json!(url));
            if let Some(k) = config.api_key.as_deref().filter(|s| !s.is_empty()) {
                ov.insert("api_key".into(), serde_json::json!(k));
            }
            if let Some(m) = config.model.as_deref().filter(|s| !s.is_empty()) {
                ov.insert("model".into(), serde_json::json!(m));
            }
            if let Some(f) = config.api_format.as_deref().filter(|s| !s.is_empty()) {
                ov.insert("api_format".into(), serde_json::json!(f));
            }
            Some(serde_json::Value::Object(ov))
        }
        None => None,
    };
    tracing::info!(
        "[doc-analysis] llm_overrides={}, config: api_url={:?} api_key={:?} model={:?} api_format={:?}",
        if llm_overrides.is_some() {
            "present"
        } else {
            "none"
        },
        config
            .api_url
            .as_deref()
            .map(|s| if s.is_empty() { "<empty>" } else { "<set>" }),
        config
            .api_key
            .as_deref()
            .map(|s| if s.is_empty() { "<empty>" } else { "<set>" }),
        config
            .model
            .as_deref()
            .map(|s| if s.is_empty() { "<empty>" } else { s }),
        config
            .api_format
            .as_deref()
            .map(|s| if s.is_empty() { "<empty>" } else { s }),
    );

    tracing::info!(
        "[doc-analysis] Calling execution host for session={}",
        session_id
    );
    let host = ServerHarnessHost::new(db.clone(), agent_svc.clone(), create_task_manager());
    let exec_result = tokio::time::timeout(
        std::time::Duration::from_secs(AGENT_TIMEOUT_SECS),
        host.execute_document_analysis_host(
            &session,
            &agent,
            &user_message,
            workspace_path.clone(),
            Vec::new(),
            Vec::new(),
            false,
            llm_overrides,
            cancel_token.clone(),
        )
    )
    .await
    .map_err(anyhow::Error::from)
    .and_then(|result| result);

    let (analysis_text, completion_report) = match exec_result {
        Ok(outcome) => {
            tracing::info!("[doc-analysis] execution host OK, extracting response");
            let text = select_document_analysis_text(
                outcome.completion_report.as_ref(),
                outcome.user_visible_summary(),
            );
            tracing::info!(
                "[doc-analysis] Extracted text length: {}",
                text.as_ref().map(|t| t.len()).unwrap_or(0)
            );
            (text, outcome.completion_report)
        }
        Err(e) => {
            if e.to_string()
                .to_ascii_lowercase()
                .contains("deadline has elapsed")
            {
                tracing::warn!("[doc-analysis] Agent TIMED OUT ({}s)", AGENT_TIMEOUT_SECS);
                cancel_token.cancel();
            } else {
                tracing::warn!("[doc-analysis] Agent execution FAILED: {}", e);
            }
            (None, None)
        }
    };

    // Cleanup session and workspace regardless of smart log outcome
    let _ = agent_svc.delete_session(&session_id).await;
    let _ = workspace_runtime::cleanup_workspace_dir(Some(&workspace_path));

    let doc_name = ctx.doc_name.clone();

    // Determine analysis text and status
    let (text, status) = derive_document_analysis_persistence(
        completion_report.as_ref(),
        analysis_text.as_deref(),
        &doc_name,
    );

    // Try to attach to the existing upload entry; if none exists, create a standalone AI entry
    let attached = smart_log_svc
        .attach_analysis(&ctx.team_id, &ctx.doc_id, &text, status)
        .await?;
    if !attached {
        let smart_ctx = SmartLogContext::ai_fallback(ctx.team_id, ctx.doc_id, ctx.doc_name);
        let entry_id = smart_log_svc.create(&smart_ctx).await?;
        smart_log_svc
            .update_ai_summary(&entry_id, &text, status)
            .await?;
    }

    tracing::info!("Document analysis completed for '{}'", doc_name);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn document_analysis_persistence_fails_closed_when_report_is_blocked() {
        let report = agime::agents::ExecutionHostCompletionReport {
            status: "blocked".to_string(),
            summary: "document content was not successfully accessed".to_string(),
            produced_artifacts: Vec::new(),
            accepted_artifacts: vec!["document:doc-1".to_string()],
            next_steps: vec!["retry".to_string()],
            validation_status: Some("failed".to_string()),
            blocking_reason: Some("document content was not successfully accessed".to_string()),
            reason_code: Some("document_analysis_incomplete".to_string()),
            content_accessed: Some(false),
            analysis_complete: Some(false),
        };

        let (text, status) =
            derive_document_analysis_persistence(Some(&report), Some("文档内容不可用"), "demo.txt");

        assert_eq!(status, "failed");
        assert!(text.contains("\"status\": \"blocked\""));
        assert!(text.contains("文档内容不可用"));
    }

    #[test]
    fn document_analysis_persistence_accepts_terminal_visible_analysis_text() {
        let report = agime::agents::ExecutionHostCompletionReport {
            status: "blocked".to_string(),
            summary: "Runtime exited without terminal completion evidence".to_string(),
            produced_artifacts: Vec::new(),
            accepted_artifacts: Vec::new(),
            next_steps: Vec::new(),
            validation_status: None,
            blocking_reason: Some("document content was not successfully accessed".to_string()),
            reason_code: None,
            content_accessed: Some(false),
            analysis_complete: Some(false),
        };

        let (text, status) = derive_document_analysis_persistence(
            Some(&report),
            Some("**文档概述**\n该文档用于验证 PPT 正文读取。\n\n**要点总结**\n1. 已包含页面标题和结论。"),
            "demo.pptx",
        );

        assert_eq!(status, "completed");
        assert!(text.contains("\"status\": \"completed\""));
        assert!(text.contains("文档概述"));
    }

    #[test]
    fn document_analysis_text_selection_prefers_terminal_visible_text_over_blocked_report() {
        let report = agime::agents::ExecutionHostCompletionReport {
            status: "blocked".to_string(),
            summary: "Runtime exited without terminal completion evidence".to_string(),
            produced_artifacts: Vec::new(),
            accepted_artifacts: Vec::new(),
            next_steps: Vec::new(),
            blocking_reason: Some("document content was not successfully accessed".to_string()),
            validation_status: Some("failed".to_string()),
            reason_code: None,
            content_accessed: Some(false),
            analysis_complete: Some(false),
        };

        let text = select_document_analysis_text(
            Some(&report),
            Some("**文档概述**\n该文档已经完成阅读并总结。".to_string()),
        )
        .unwrap();

        assert!(text.contains("文档概述"));
        assert!(!text.contains("Validation: failed"));
    }

    #[test]
    fn document_analysis_persistence_rejects_future_intent_text() {
        let (_text, status) = derive_document_analysis_persistence(
            None,
            Some("让我从工作区读取 Excel 文件以获取更完整的数据。"),
            "demo.xlsx",
        );

        assert_eq!(status, "failed");
    }

    #[test]
    fn document_analysis_persistence_accepts_json_contract() {
        let (text, status) = derive_document_analysis_persistence(
            None,
            Some(
                r#"{"status":"completed","summary":"已完成文档解读。","content_structure":["一页"],"key_points":["重点"],"file_observations":[],"limitations":[]}"#,
            ),
            "demo.txt",
        );

        assert_eq!(status, "completed");
        assert!(text.contains("\"content_structure\""));
    }

    #[test]
    fn document_analysis_persistence_blocks_json_contract() {
        let (text, status) = derive_document_analysis_persistence(
            None,
            Some(
                r#"{"status":"blocked","summary":"无法读取文档。","content_structure":[],"key_points":[],"file_observations":[],"limitations":["缺少 unrar 工具"]}"#,
            ),
            "demo.rar",
        );

        assert_eq!(status, "failed");
        assert!(text.contains("缺少 unrar 工具"));
    }

    #[test]
    fn document_analysis_persistence_rewrites_container_forensics_blocked_result() {
        let (text, status) = derive_document_analysis_persistence(
            None,
            Some(
                r#"{"status":"blocked","summary":"该文件是一个 38 字节的合成测试文件，虽然以有效的 RAR 魔术签名开头，但不是一个真实 RAR 压缩包。","content_structure":["RAR 魔术签名"],"key_points":["not-a-real-rar"],"file_observations":["ASCII 明文"],"limitations":[]}"#,
            ),
            "demo.rar",
        );

        assert_eq!(status, "failed");
        assert!(text.contains("无法解压或读取压缩包内部的文档内容"));
        assert!(text.contains("读取受阻详情"));
        assert!(!text.contains("\"content_structure\": [\n    \"RAR 魔术签名\""));
    }

    #[test]
    fn document_analysis_persistence_repairs_jsonish_summary_without_nesting() {
        let (text, status) = derive_document_analysis_persistence(
            None,
            Some(
                r#"{"status":"completed","summary":"该文档主题为"项目目标、风险、结论"，但内容较短。","content_structure":["一段"],"key_points":[],"file_observations":[],"limitations":[]}"#,
            ),
            "demo.txt",
        );

        assert_eq!(status, "completed");
        assert!(text.contains("\"summary\": \"该文档主题为\\\"项目目标、风险、结论\\\"，但内容较短。\""));
        assert!(!text.contains("\"summary\": \"{\\\"status\\\""));
    }

    #[test]
    fn english_prompt_uses_snapshot_as_auxiliary_context_and_requires_json() {
        let prompt = build_analysis_prompt(
            &DocumentAnalysisContext {
                team_id: "team".to_string(),
                doc_id: "doc-1".to_string(),
                doc_name: "demo.txt".to_string(),
                mime_type: "text/plain".to_string(),
                file_size: 12,
                user_id: "user".to_string(),
                lang: Some("en".to_string()),
                extra_instructions: None,
            },
            Some(&MaterializedAnalysisDocument {
                file_path: "/tmp/workspace/attachments/demo.txt".to_string(),
                relative_path: "attachments/demo.txt".to_string(),
                file_name: "demo.txt".to_string(),
                mime_type: "text/plain".to_string(),
                file_size: 12,
            }),
            Some("文档内容:\nhello world"),
        );

        assert!(prompt.contains("Preferred workspace-relative path"));
        assert!(prompt.contains("Available Text Snapshot (auxiliary)"));
        assert!(prompt.contains("strict JSON"));
        assert!(prompt.contains("\"status\":\"completed|blocked\""));
        assert!(prompt.contains("attachments/demo.txt"));
        assert!(prompt.contains("/tmp/workspace/attachments/demo.txt"));
    }

    #[test]
    fn chinese_prompt_uses_snapshot_as_auxiliary_context_and_requires_json() {
        let prompt = build_analysis_prompt(
            &DocumentAnalysisContext {
                team_id: "team".to_string(),
                doc_id: "doc-1".to_string(),
                doc_name: "demo.txt".to_string(),
                mime_type: "text/plain".to_string(),
                file_size: 12,
                user_id: "user".to_string(),
                lang: Some("zh".to_string()),
                extra_instructions: None,
            },
            Some(&MaterializedAnalysisDocument {
                file_path: "/tmp/workspace/attachments/demo.txt".to_string(),
                relative_path: "attachments/demo.txt".to_string(),
                file_name: "demo.txt".to_string(),
                mime_type: "text/plain".to_string(),
                file_size: 12,
            }),
            Some("文档内容:\nhello world"),
        );

        assert!(prompt.contains("首选 workspace 相对路径"));
        assert!(prompt.contains("可用文本摘录（辅助）"));
        assert!(prompt.contains("严格 JSON"));
        assert!(prompt.contains("\"status\":\"completed|blocked\""));
        assert!(prompt.contains("attachments/demo.txt"));
        assert!(prompt.contains("/tmp/workspace/attachments/demo.txt"));
    }

    #[test]
    fn chinese_prompt_uses_workspace_reader_when_excerpt_is_missing() {
        let prompt = build_analysis_prompt(
            &DocumentAnalysisContext {
                team_id: "team".to_string(),
                doc_id: "doc-1".to_string(),
                doc_name: "demo.pptx".to_string(),
                mime_type:
                    "application/vnd.openxmlformats-officedocument.presentationml.presentation"
                        .to_string(),
                file_size: 12,
                user_id: "user".to_string(),
                lang: Some("zh".to_string()),
                extra_instructions: None,
            },
            Some(&MaterializedAnalysisDocument {
                file_path: "/tmp/workspace/attachments/demo.pptx".to_string(),
                relative_path: "attachments/demo.pptx".to_string(),
                file_name: "demo.pptx".to_string(),
                mime_type:
                    "application/vnd.openxmlformats-officedocument.presentationml.presentation"
                        .to_string(),
                file_size: 12,
            }),
            None,
        );

        assert!(prompt.contains("使用文档引用和首选 workspace 相对路径"));
        assert!(prompt.contains("python-pptx"));
        assert!(prompt.contains("严格 JSON"));
    }
}

// ─── Helper Functions ──────────────────────────────────

/// Check if a MIME type should be skipped based on configured prefixes.
fn should_skip_mime(mime: &str, skip_prefixes: &[String]) -> bool {
    let lower = mime.to_lowercase();
    skip_prefixes
        .iter()
        .any(|p| lower.starts_with(&p.to_ascii_lowercase()))
}

async fn materialize_document_for_analysis(
    db: &Arc<MongoDb>,
    ctx: &DocumentAnalysisContext,
    workspace_path: &str,
) -> anyhow::Result<MaterializedAnalysisDocument> {
    let svc = DocumentService::new((**db).clone());
    let (data, _, _) = svc.download(&ctx.team_id, &ctx.doc_id).await?;
    let meta = svc.get_metadata(&ctx.team_id, &ctx.doc_id).await?;
    let output_dir = PathBuf::from(workspace_path).join("attachments");
    tokio::fs::create_dir_all(&output_dir).await?;

    let mut output_name = sanitize_analysis_filename(&meta.name);
    if output_name.is_empty() {
        output_name = format!(
            "document-{}",
            ctx.doc_id.chars().take(8).collect::<String>()
        );
    }
    let file_path = output_dir.join(output_name);
    tokio::fs::write(&file_path, &data).await?;
    let relative_path = format!(
        "attachments/{}",
        file_path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("document")
    );

    Ok(MaterializedAnalysisDocument {
        file_path: file_path.to_string_lossy().to_string(),
        relative_path,
        file_name: meta.name,
        mime_type: meta.mime_type,
        file_size: data.len() as u64,
    })
}

async fn load_latest_smart_log_content_snapshot(
    db: &Arc<MongoDb>,
    ctx: &DocumentAnalysisContext,
) -> anyhow::Result<Option<String>> {
    let team_oid = ObjectId::parse_str(&ctx.team_id)?;
    let coll = db.collection::<SmartLogEntry>("smart_logs");
    let filter = doc! {
        "team_id": team_oid,
        "resource_id": &ctx.doc_id,
        "action": { "$ne": "delete" },
    };
    let options = FindOneOptions::builder()
        .sort(doc! { "created_at": -1 })
        .build();
    Ok(coll
        .find_one(filter, options)
        .await?
        .and_then(|entry| entry.content_snapshot)
        .map(|value| value.trim().chars().take(2500).collect::<String>())
        .filter(|value| !value.is_empty()))
}

fn sanitize_analysis_filename(name: &str) -> String {
    let sanitized = name
        .chars()
        .map(|ch| match ch {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
            ch if ch.is_control() => '_',
            ch => ch,
        })
        .collect::<String>()
        .trim()
        .trim_matches('.')
        .to_string();
    if sanitized.is_empty() {
        String::new()
    } else {
        sanitized
    }
}

/// Classify MIME type into a category for prompt selection.
fn mime_category(mime: &str) -> &'static str {
    let m = mime.to_lowercase();
    if m.contains("pdf") {
        return "pdf";
    }
    if m.contains("zip")
        || m.contains("rar")
        || m.contains("7z")
        || m.contains("tar")
        || m.contains("gzip")
    {
        return "archive";
    }
    if m.contains("presentation")
        || m.contains("powerpoint")
        || m.contains("pptx")
        || m.contains("ppt")
    {
        return "presentation";
    }
    if m.contains("spreadsheet") || m.contains("excel") || m.contains("csv") {
        return "spreadsheet";
    }
    if m.contains("word") || (m.contains("document") && !m.contains("spreadsheet")) {
        return "word";
    }
    "text"
}

/// Build a MIME-type-aware analysis prompt.
fn build_analysis_prompt(
    ctx: &DocumentAnalysisContext,
    materialized: Option<&MaterializedAnalysisDocument>,
    content_snapshot: Option<&str>,
) -> String {
    let cat = mime_category(&ctx.mime_type);
    let en = ctx.lang.as_deref() == Some("en");
    let snapshot = content_snapshot
        .map(str::trim)
        .filter(|value| !value.is_empty());

    let input_rule = match (snapshot.is_some(), en) {
        (true, true) => {
            "A text snapshot is included as auxiliary context. Prefer the actual document reference and workspace file when deeper reading is needed. Use the snapshot only to speed up understanding; it is not a separate execution path."
        }
        (true, false) => {
            "下方文本摘录只是辅助上下文。需要更完整解读时，优先使用文档引用和 workspace 文件读取实际文档；不要把摘录当成另一条执行路径。"
        }
        (false, true) => {
            "Use the document reference and preferred workspace-relative path directly with the available agent tools. Read the document best-effort and return the required JSON result."
        }
        (false, false) => {
            "请使用文档引用和首选 workspace 相对路径，通过 agent 可用工具 best-effort 阅读实际文件，并返回要求的 JSON 解读结果。"
        }
    };

    let type_steps = match (snapshot.is_some(), cat, en) {
        (true, _, false) => {
            "推荐范式：先查看文档引用、workspace 路径和可用文本摘录；如果摘录足够，直接形成 JSON；如果不够，用可用工具读取 workspace 文件后再形成 JSON。"
        }
        (true, _, true) => {
            "Suggested pattern: inspect the document reference, workspace path, and available snapshot; if the snapshot is enough, produce JSON directly; otherwise read the workspace file with available tools and then produce JSON."
        }
        (false, "pdf", false) => {
            "推荐范式：直接使用已提供的 workspace 文件路径，再用 developer shell、MCP 或其他本地工具读取 PDF 正文（如 pdftotext 或 Python pdfplumber），最后按页面/章节结构分析。"
        }
        (false, "pdf", true) => {
            "Suggested pattern: use the provided workspace file path directly, then use developer shell, MCP, or another local tool to read the PDF content (for example pdftotext or Python pdfplumber), and finally analyze it by page/chapter structure."
        }
        (false, "archive", false) => {
            "推荐范式：直接使用已提供的 workspace 文件路径，再用 developer shell、MCP 或其他本地工具解压压缩包；只分析解压出来的内部文档内容。不要把压缩包容器、文件头、魔术签名、字节长度当成文档内容。如果无法解压或没有可读内部文件，返回 status=\"blocked\"，summary 说明无法读取压缩包内部文档内容。"
        }
        (false, "archive", true) => {
            "Suggested pattern: use the provided workspace file path directly, then use developer shell, MCP, or another local tool to extract the archive; only analyze the extracted inner documents. Do not treat the archive container, headers, magic signature, or byte length as document content. If extraction fails or no readable inner file exists, return status=\"blocked\" and say the inner document content could not be read."
        }
        (false, "presentation", false) => {
            "推荐范式：直接使用已提供的 workspace 文件路径。PPTX 可优先用 python-pptx；若库不可用，把 PPTX 当 zip 包读取 `ppt/slides/*.xml` 和 notes。旧 PPT/PPS 优先使用系统已有转换/文本提取工具，必要时做 best-effort 文本提取。拿到一次可读内容后直接逐页整理并输出最终结果。"
        }
        (false, "presentation", true) => {
            "Suggested pattern: use the provided workspace file path directly. For PPTX, prefer python-pptx; if unavailable, treat PPTX as a zip package and read `ppt/slides/*.xml` plus notes. For legacy PPT/PPS, use available conversion/text-extraction tools first, then best-effort text extraction if needed. After readable content is obtained once, analyze slide by slide and produce the final result."
        }
        (false, "spreadsheet", false) => {
            "推荐范式：直接使用已提供的 workspace 文件路径。XLSX 可优先用 openpyxl/pandas；若库不可用，把 XLSX 当 zip 包读取 `xl/sharedStrings.xml` 和 `xl/worksheets/*.xml`。旧 XLS 优先使用系统已有转换/文本提取工具，必要时做 best-effort 文本提取。拿到一次可读内容后直接总结 Sheet、字段和关键统计。"
        }
        (false, "spreadsheet", true) => {
            "Suggested pattern: use the provided workspace file path directly. For XLSX, prefer openpyxl/pandas; if unavailable, treat XLSX as a zip package and read `xl/sharedStrings.xml` and `xl/worksheets/*.xml`. For legacy XLS, use available conversion/text-extraction tools first, then best-effort text extraction if needed. After readable content is obtained once, summarize sheets, fields, and key statistics."
        }
        (false, "word", false) => {
            "推荐范式：直接使用已提供的 workspace 文件路径。DOCX 可优先用 python-docx；若库不可用，把 DOCX 当 zip 包读取 `word/document.xml`、headers、footers。旧 DOC 优先使用系统已有转换/文本提取工具，必要时做 best-effort 文本提取。拿到一次可读内容后直接按章节结构分析并输出最终结果。"
        }
        (false, "word", true) => {
            "Suggested pattern: use the provided workspace file path directly. For DOCX, prefer python-docx; if unavailable, treat DOCX as a zip package and read `word/document.xml`, headers, and footers. For legacy DOC, use available conversion/text-extraction tools first, then best-effort text extraction if needed. After readable content is obtained once, analyze by section and produce the final result."
        }
        (false, _, false) => {
            "推荐范式：直接使用已提供的 workspace 文件路径，再用 developer shell、MCP 或其他本地工具读取实际内容，最后分析文本结构和关键信息。"
        }
        (false, _, true) => {
            "Suggested pattern: use the provided workspace file path directly, then use developer shell, MCP, or another local tool to inspect the content, and finally analyze the text structure and key information."
        }
    };

    let mut prompt = if en {
        format!(
             "Analyze the document \"{name}\" (MIME: {mime}, Doc ID: {doc_id}).\n\n\
             This is an agent-driven document-analysis run. The server has already materialized the uploaded document into the workspace when a workspace path is provided below.\n\
             {input_rule}\n\n\
             First principle: summarize the user-facing document content. Do not analyze file containers, MIME labels, raw bytes, archive headers, magic signatures, filenames, or test markers as if they were the document content.\n\n\
             After you have obtained readable content once, do not repeatedly call file-reading tools. Move directly to the final JSON result.\n\n\
             Never reply with future-intent text such as \"I need to read the document first\" or \"let me do that now\". Complete the read/analyze chain in this run.\n\n\
             ## Suggested Reading Pattern\n{steps}\n\n\
             ## Output Format (strict JSON only)\n\
             Return exactly one JSON object with these keys and no Markdown fences:\n\
             {{\"status\":\"completed|blocked\",\"summary\":\"string\",\"content_structure\":[\"string\"],\"key_points\":[\"string\"],\"file_observations\":[\"string\"],\"limitations\":[\"string\"]}}\n\n\
             Use status=\"completed\" when you have read enough content to provide a useful analysis.\n\
             Use status=\"blocked\" only when the file cannot be read, the format has no available tool, the content is damaged, or the path/permission is unavailable.\n\n\
             ## Strictly Forbidden\n\
             - No Markdown fences or prose outside the JSON object\n\
             - Do not summarize archive/file metadata when no inner document content was read\n\
             - No conversational phrases like \"let me know\" or \"hope this helps\"",
            name = ctx.doc_name,
            mime = ctx.mime_type,
            doc_id = ctx.doc_id,
            input_rule = input_rule,
            steps = type_steps,
        )
    } else {
        format!(
             "请分析文档「{name}」(MIME: {mime}, 文档ID: {doc_id})。\n\n\
             这是一轮由 agent 自主完成的文档分析。如果下方提供了 workspace 文件路径，说明服务端已经把上传文档确定性放入本轮 workspace。\n\
             {input_rule}\n\n\
             第一性原则：总结用户真正关心的文档正文内容。不要把文件容器、MIME 标签、原始字节、压缩包头、魔术签名、文件名或测试标识当作文档内容来解读。\n\n\
             一旦已经拿到一次可读内容，不要反复调用文件读取工具，应直接进入最终 JSON 解读结果。\n\n\
             不要回复“我需要先读取文档”或“让我先做这个”之类的未来时描述。必须在本轮内完成读取和分析。\n\n\
             ## 建议范式\n{steps}\n\n\
             ## 输出格式（严格 JSON）\n\
             只返回一个 JSON 对象，不要 Markdown 代码块：\n\
             {{\"status\":\"completed|blocked\",\"summary\":\"string\",\"content_structure\":[\"string\"],\"key_points\":[\"string\"],\"file_observations\":[\"string\"],\"limitations\":[\"string\"]}}\n\n\
             当你已经读取到足够内容并能形成有效解读时，status 使用 \"completed\"。\n\
             只有文件无法读取、格式缺少可用工具、内容损坏、路径或权限不可访问时，status 才使用 \"blocked\"。\n\n\
             ## 严格禁止\n\
             - 禁止在 JSON 外输出解释性文字\n\
             - 未读取到压缩包内部文档内容时，禁止总结压缩包/文件元信息\n\
             - 禁止输出「我需要先读取」「让我先解压」等未来时描述",
            name = ctx.doc_name,
            mime = ctx.mime_type,
            doc_id = ctx.doc_id,
            input_rule = input_rule,
            steps = type_steps,
        )
    };

    if let Some(materialized) = materialized {
        if en {
            prompt.push_str(&format!(
                "\n\n## Workspace Input\n- Preferred workspace-relative path: `{}`\n- Absolute path for reference only: `{}`\n- File name: `{}`\n- MIME type: `{}`\n- File size: {} bytes",
                materialized.relative_path,
                materialized.file_path,
                materialized.file_name,
                materialized.mime_type,
                materialized.file_size
            ));
        } else {
            prompt.push_str(&format!(
                "\n\n## Workspace 输入\n- 首选 workspace 相对路径：`{}`\n- 绝对路径仅作参考：`{}`\n- 文件名：`{}`\n- MIME 类型：`{}`\n- 文件大小：{} bytes",
                materialized.relative_path,
                materialized.file_path,
                materialized.file_name,
                materialized.mime_type,
                materialized.file_size
            ));
        }
    }
    if let Some(snapshot) = snapshot {
        if en {
            prompt.push_str(&format!(
                "\n\n## Available Text Snapshot (auxiliary)\nThis snapshot may help you understand the document quickly. If it is insufficient, use the workspace file and available tools. The final answer must still be the strict JSON object:\n\n```text\n{}\n```",
                snapshot
            ));
        } else {
            prompt.push_str(&format!(
                "\n\n## 可用文本摘录（辅助）\n这段摘录可以帮助你快速理解文档。如果摘录不足，请使用 workspace 文件和可用工具读取实际内容。最终回答仍必须是严格 JSON 对象：\n\n```text\n{}\n```",
                snapshot
            ));
        }
    }

    prompt
}
