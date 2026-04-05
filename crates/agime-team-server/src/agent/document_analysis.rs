//! Document Analysis Trigger — auto-interpret uploaded documents via Agent
//!
//! When a document is uploaded, spawns an async task that:
//! 1. Creates a temporary chat session with the team's agent
//! 2. Sends a message asking the agent to analyze the document
//! 3. Agent uses its full toolset (DocumentTools, Developer shell, etc.)
//! 4. Extracts the agent's response and stores it as a smart log entry

use agime_team::models::mongo::{
    DocumentAnalysisContext, DocumentAnalysisSettings, DocumentAnalysisTrigger, SmartLogContext,
};
use agime_team::services::mongo::{SmartLogService, TeamService};
use agime_team::MongoDb;
use std::sync::Arc;
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;

use super::host_router::{DocumentAnalysisHostRouteRequest, HostExecutionRouter};
use super::runtime::{self, EventBroadcaster};
use super::server_harness_host::ServerHarnessHost;
use super::service_mongo::AgentService;
use super::session_mongo::CreateSessionRequest;
use super::task_manager::{StreamEvent, TaskManager};
use super::workspace_service::WorkspaceService;
use agime::agents::format_execution_host_completion_text;

const MAX_ANALYSIS_CHARS: usize = 3000;
const AGENT_TIMEOUT_SECS: u64 = 600;

fn derive_document_analysis_persistence(
    completion_report: Option<&agime::agents::ExecutionHostCompletionReport>,
    analysis_text: Option<&str>,
    doc_name: &str,
) -> (String, &'static str) {
    match (completion_report, analysis_text.map(str::trim)) {
        (Some(report), Some(text)) if !text.is_empty() && report.status == "completed" => {
            (text.chars().take(MAX_ANALYSIS_CHARS).collect(), "completed")
        }
        (Some(_report), Some(text)) if !text.is_empty() => {
            (text.chars().take(MAX_ANALYSIS_CHARS).collect(), "failed")
        }
        (Some(report), None) => (
            format_execution_host_completion_text(report)
                .trim()
                .chars()
                .take(MAX_ANALYSIS_CHARS)
                .collect(),
            "failed",
        ),
        (_, Some(text)) if !text.is_empty() => {
            (text.chars().take(MAX_ANALYSIS_CHARS).collect(), "failed")
        }
        _ => (format!("解读了文档「{}」", doc_name), "failed"),
    }
}

/// Discards all events. Used for background analysis with no SSE subscribers.
struct NoopBroadcaster;

impl EventBroadcaster for NoopBroadcaster {
    async fn broadcast(&self, _context_id: &str, _event: StreamEvent) {}
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
        let _ = smart_log_svc
            .cancel_pending_analysis(&ctx.team_id, &ctx.doc_id)
            .await;
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
            // document_tools for doc API; developer shell for fallback extraction.
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
            session_source: Some("system".to_string()),
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

    let mut user_message = build_analysis_prompt(&ctx);
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
    tracing::info!("[doc-analysis] llm_overrides={}, config: api_url={:?} api_key={:?} model={:?} api_format={:?}",
        if llm_overrides.is_some() { "present" } else { "none" },
        config.api_url.as_deref().map(|s| if s.is_empty() { "<empty>" } else { "<set>" }),
        config.api_key.as_deref().map(|s| if s.is_empty() { "<empty>" } else { "<set>" }),
        config.model.as_deref().map(|s| if s.is_empty() { "<empty>" } else { s }),
        config.api_format.as_deref().map(|s| if s.is_empty() { "<empty>" } else { s }),
    );

    tracing::info!(
        "[doc-analysis] Calling execution host for session={}",
        session_id
    );
    let router = HostExecutionRouter::from_env();
    let db_direct = db.clone();
    let db_bridge = db.clone();
    let agent_svc_direct = agent_svc.clone();
    let agent_svc_bridge = agent_svc.clone();
    let agent_svc_after = agent_svc.clone();
    let cancel_token_direct = cancel_token.clone();
    let cancel_token_bridge = cancel_token.clone();
    let llm_overrides_direct = llm_overrides.clone();
    let llm_overrides_bridge = llm_overrides.clone();
    let session_id_bridge = session_id.clone();
    let exec_result = router
        .route_document_analysis(
            DocumentAnalysisHostRouteRequest {
                session_id: session_id.clone(),
                agent_id: agent.id.clone(),
                user_message,
                workspace_path: workspace_path.clone(),
                llm_overrides,
                target_artifacts: vec![format!("document:{}", ctx.doc_id)],
                result_contract: vec![format!("document:{}", ctx.doc_id)],
                validation_mode: true,
            },
            |request| async move {
                let host = ServerHarnessHost::new(
                    db_direct,
                    agent_svc_direct,
                    Arc::new(TaskManager::new()),
                );
                tokio::time::timeout(
                    std::time::Duration::from_secs(AGENT_TIMEOUT_SECS),
                    host.execute_document_analysis_host(
                        &session,
                        &agent,
                        &request.user_message,
                        request.workspace_path,
                        request.target_artifacts,
                        request.result_contract,
                        request.validation_mode,
                        llm_overrides_direct,
                        cancel_token_direct,
                    ),
                )
                .await
                .map(|result| result)
                .map_err(anyhow::Error::from)?
            },
            |request| async move {
                let task_manager = Arc::new(TaskManager::new());
                let broadcaster = Arc::new(NoopBroadcaster);
                tokio::time::timeout(
                    std::time::Duration::from_secs(AGENT_TIMEOUT_SECS),
                    runtime::execute_bridge_request(
                        &db_bridge,
                        &agent_svc_bridge,
                        &task_manager,
                        &broadcaster,
                        runtime::BridgeExecutionRequest {
                            context_id: request.session_id.clone(),
                            agent_id: request.agent_id,
                            session_id: request.session_id,
                            user_message: request.user_message,
                            cancel_token: cancel_token_bridge,
                            workspace_path: Some(request.workspace_path),
                            llm_overrides: llm_overrides_bridge,
                            turn_system_instruction: None,
                        },
                    ),
                )
                .await
                .map_err(anyhow::Error::from)??;
                let updated_session = agent_svc_after
                    .get_session(&session_id_bridge)
                    .await?
                    .ok_or_else(|| anyhow::anyhow!("Session not found after bridge execution"))?;
                Ok(super::server_harness_host::ServerHarnessHostOutcome {
                    messages_json: updated_session.messages_json.clone(),
                    message_count: updated_session.message_count,
                    total_tokens: updated_session.total_tokens,
                    last_assistant_text: runtime::extract_last_assistant_text(
                        &updated_session.messages_json,
                    ),
                    completion_report: None,
                    events_emitted: 0,
                    signal_summary: None,
                    completion_outcome: None,
                })
            },
        )
        .await;

    let analysis_text = match &exec_result {
        Ok(outcome) => {
            tracing::info!("[doc-analysis] execution host OK, extracting response");
            let text = outcome
                .completion_report
                .as_ref()
                .map(format_execution_host_completion_text)
                .or_else(|| outcome.user_visible_summary());
            tracing::info!(
                "[doc-analysis] Extracted text length: {}",
                text.as_ref().map(|t| t.len()).unwrap_or(0)
            );
            text
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
            None
        }
    };

    // Cleanup session and workspace regardless of smart log outcome
    let _ = agent_svc.delete_session(&session_id).await;
    let _ = runtime::cleanup_workspace_dir(Some(&workspace_path));

    let doc_name = ctx.doc_name.clone();

    // Determine analysis text and status
    let completion_report = exec_result
        .as_ref()
        .ok()
        .and_then(|outcome| outcome.completion_report.as_ref());
    let (text, status) = derive_document_analysis_persistence(
        completion_report,
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
        assert!(text.contains("文档内容不可用"));
    }

    #[test]
    fn english_prompt_treats_document_tools_as_access_and_workspace_export_step() {
        let prompt = build_analysis_prompt(&DocumentAnalysisContext {
            team_id: "team".to_string(),
            doc_id: "doc-1".to_string(),
            doc_name: "demo.txt".to_string(),
            mime_type: "text/plain".to_string(),
            file_size: 12,
            user_id: "user".to_string(),
            lang: Some("en".to_string()),
            extra_instructions: None,
        });

        assert!(prompt.contains("workspace file path"));
        assert!(prompt.contains("developer shell, MCP, or another local tool"));
        assert!(!prompt.contains("read_document for direct text"));
    }

    #[test]
    fn chinese_prompt_treats_document_tools_as_access_and_workspace_export_step() {
        let prompt = build_analysis_prompt(&DocumentAnalysisContext {
            team_id: "team".to_string(),
            doc_id: "doc-1".to_string(),
            doc_name: "demo.txt".to_string(),
            mime_type: "text/plain".to_string(),
            file_size: 12,
            user_id: "user".to_string(),
            lang: Some("zh".to_string()),
            extra_instructions: None,
        });

        assert!(prompt.contains("workspace 文件路径"));
        assert!(prompt.contains("developer shell、MCP 或其他本地工具"));
        assert!(!prompt.contains("纯文本可 read_document"));
    }
}

// ─── Helper Functions ──────────────────────────────────

/// Check if a MIME type should be skipped based on configured prefixes.
fn should_skip_mime(mime: &str, skip_prefixes: &[String]) -> bool {
    let lower = mime.to_lowercase();
    skip_prefixes.iter().any(|p| lower.starts_with(p))
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
fn build_analysis_prompt(ctx: &DocumentAnalysisContext) -> String {
    let cat = mime_category(&ctx.mime_type);
    let en = ctx.lang.as_deref() == Some("en");

    let type_steps = match (cat, en) {
        ("pdf", false) => "1. 先用文档工具建立对文档区对象的访问（推荐 export_document 或 import_document_to_workspace）\n2. 再使用 developer shell、MCP 或其他本地工具读取导出文件的实际 PDF 内容（如 pdftotext 或 Python pdfplumber）\n3. 按页面/章节结构分析内容",
        ("pdf", true) => "1. First establish document-area access with a document tool (prefer export_document or import_document_to_workspace)\n2. Then use developer shell, MCP, or another local tool to read the exported PDF content (e.g. pdftotext or Python pdfplumber)\n3. Analyze content by page/chapter structure",
        ("archive", false) => "1. 先用文档工具建立对文档区对象的访问（推荐 export_document 或 import_document_to_workspace）\n2. 再使用 developer shell、MCP 或其他本地工具解压并读取导出文件内容\n3. 逐个分析解压后的文件",
        ("archive", true) => "1. First establish document-area access with a document tool (prefer export_document or import_document_to_workspace)\n2. Then use developer shell, MCP, or another local tool to extract and inspect the exported archive\n3. Analyze each extracted file",
        ("presentation", false) => "1. 先用文档工具建立对文档区对象的访问（推荐 export_document 或 import_document_to_workspace）\n2. 再使用 developer shell、MCP 或其他本地工具提取演示文稿内容（如 python-pptx）\n3. 逐页提取文字、标题和结构",
        ("presentation", true) => "1. First establish document-area access with a document tool (prefer export_document or import_document_to_workspace)\n2. Then use developer shell, MCP, or another local tool to extract slide content (e.g. python-pptx)\n3. Extract text and structure page by page",
        ("spreadsheet", false) => "1. 先用文档工具建立对文档区对象的访问并拿到 workspace 路径（可用 read_document、export_document 或 import_document_to_workspace）\n2. 再使用 developer shell、MCP 或其他本地工具读取导出文件中的实际表格内容\n3. 描述数据结构、字段含义并总结关键数据",
        ("spreadsheet", true) => "1. First establish document-area access and obtain a workspace path with a document tool (use read_document, export_document, or import_document_to_workspace)\n2. Then use developer shell, MCP, or another local tool to inspect the exported spreadsheet content\n3. Describe the data structure, field meanings, and key statistics",
        ("word", false) => "1. 先用文档工具建立对文档区对象的访问（推荐 export_document 或 import_document_to_workspace）\n2. 再使用 developer shell、MCP 或其他本地工具提取文档文本（如 python-docx）\n3. 按章节结构分析内容",
        ("word", true) => "1. First establish document-area access with a document tool (prefer export_document or import_document_to_workspace)\n2. Then use developer shell, MCP, or another local tool to extract text (e.g. python-docx)\n3. Analyze content by section structure",
        (_, false) => "1. 先用文档工具建立对文档区对象的访问并拿到 workspace 路径（可用 read_document、export_document 或 import_document_to_workspace）\n2. 再使用 developer shell、MCP 或其他本地工具读取导出文件中的实际内容\n3. 分析文本结构和关键信息",
        (_, true) => "1. First establish document-area access and obtain a workspace path with a document tool (use read_document, export_document, or import_document_to_workspace)\n2. Then use developer shell, MCP, or another local tool to inspect the exported content\n3. Analyze the text structure and key information",
    };

    if en {
        format!(
            "Analyze the document \"{name}\" (MIME: {mime}, Doc ID: {doc_id}).\n\n\
             You MUST first use one of these document tools to establish access to the formal document area: `read_document`, `export_document`, or `import_document_to_workspace`.\n\
             These tools establish access to the formal document area and should give you a workspace file path. They are not the final content-reading step by themselves.\n\
             Continue by using developer shell, MCP, or another local tool to inspect the exported workspace file itself.\n\
             If you do not successfully establish document-area access first, the run will be rejected as incomplete.\n\n\
             When you submit the final structured completion, set `content_accessed=true` only if you actually obtained document content or extracted readable content from the exported workspace file in this run.\n\
             Set `analysis_complete=true` only if the final analysis sections are fully written from the document content already gathered in this run.\n\
             If either condition is not met, you must return a blocked result with an accurate `reason_code`.\n\n\
             Never reply with future-intent text such as \"I need to read the document first\" or \"let me do that now\". Complete the read/analyze/final_output chain in this run.\n\n\
             ## Steps\n{steps}\n\n\
             ## Output Format (strict)\n\n\
             **Document Overview**\n\
             2-3 sentences: document type, topic, core purpose.\n\n\
             **Content Structure**\n\
             List chapters/pages/sheets for the overall framework.\n\n\
             **Key Content**\n\
             Expand key information by section, including important data, terms, and summaries.\n\n\
             **Key Takeaways**\n\
             3-5 most important points.\n\n\
             ## Strictly Forbidden\n\
             - No conversational phrases like \"let me know\" or \"hope this helps\"\n\
             - End with the last takeaway, no extra text",
            name = ctx.doc_name,
            mime = ctx.mime_type,
            doc_id = ctx.doc_id,
            steps = type_steps,
        )
    } else {
        format!(
            "请分析文档「{name}」(MIME: {mime}, 文档ID: {doc_id})。\n\n\
             在产出最终答案之前，你必须先成功调用以下任一文档工具来建立对正式文档区对象的访问：`read_document`、`export_document` 或 `import_document_to_workspace`。\n\
             这一步是文档区访问前置条件，并且应当拿到 workspace 文件路径；它本身不等于最终的内容读取。\n\
             后续还应继续使用 developer shell、MCP 或其他本地工具读取导出到 workspace 的实际文件内容。\n\
             如果没有先成功建立文档区访问，本轮会被判定为未完成。\n\n\
             在提交最终 structured completion 时，只有在本轮里确实拿到了文档正文或从导出文件中提取到了可读内容，才能设置 `content_accessed=true`。\n\
             只有在基于这些正文内容完成了最终分析各部分输出时，才能设置 `analysis_complete=true`。\n\
             只要任一条件不满足，就必须返回 blocked，并给出准确的 `reason_code`。\n\n\
             不要回复“我需要先读取文档”或“让我先做这个”之类的未来时描述。必须在本轮内完成读取、分析并给出最终结果。\n\n\
             ## 操作步骤\n{steps}\n\n\
             ## 输出格式（严格遵守）\n\n\
             **文档概述**\n\
             用2-3句话说明文档的类型、主题和核心目的。\n\n\
             **内容结构**\n\
             列出文档的章节/页面/Sheet结构，让读者了解整体框架。\n\n\
             **核心内容**\n\
             按章节或主题逐条展开关键信息，包括重要观点、数据、术语和图表摘要。\n\n\
             **要点总结**\n\
             3-5条最重要的信息。\n\n\
             ## 严格禁止\n\
             - 禁止末尾添加「如需进一步分析」等对话性语句\n\
             - 直接以要点总结的最后一条结束",
            name = ctx.doc_name,
            mime = ctx.mime_type,
            doc_id = ctx.doc_id,
            steps = type_steps,
        )
    }
}
