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

use super::runtime::{self, EventBroadcaster};
use super::service_mongo::AgentService;
use super::session_mongo::CreateSessionRequest;
use super::task_manager::{StreamEvent, TaskManager};

const MAX_ANALYSIS_CHARS: usize = 3000;
const AGENT_TIMEOUT_SECS: u64 = 600;

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
    tracing::info!("[doc-analysis] START doc={} team={} mime={} size={}", ctx.doc_id, ctx.team_id, ctx.mime_type, ctx.file_size);
    let team_svc = TeamService::new((*db).clone());
    let config = match team_svc.get_settings(&ctx.team_id).await {
        Ok(s) => {
            tracing::info!("[doc-analysis] Config loaded: enabled={} agent_id={:?} has_api_url={}", s.document_analysis.enabled, s.document_analysis.agent_id, s.document_analysis.api_url.is_some());
            s.document_analysis
        }
        Err(e) => {
            tracing::warn!("[doc-analysis] Failed to load settings, using defaults: {}", e);
            DocumentAnalysisSettings::default()
        }
    };

    let smart_log_svc = SmartLogService::new((*db).clone());

    // Check whether analysis should proceed; cancel the pending log entry on skip.
    let skip_reason = if !config.enabled {
        Some(format!("Document analysis disabled for team {}", ctx.team_id))
    } else if should_skip_mime(&ctx.mime_type, &config.skip_mime_prefixes) {
        Some(format!("Skipping analysis for MIME type: {}", ctx.mime_type))
    } else if ctx.file_size < config.min_file_size {
        Some(format!("Skipping analysis for tiny file ({} bytes)", ctx.file_size))
    } else if config.max_file_size.is_some_and(|max| ctx.file_size > max) {
        Some(format!("Skipping analysis for large file ({} bytes)", ctx.file_size))
    } else {
        None
    };
    if let Some(reason) = skip_reason {
        tracing::debug!("{}", reason);
        let _ = smart_log_svc.cancel_pending_analysis(&ctx.team_id, &ctx.doc_id).await;
        return Ok(());
    }

    // Agent selection: config.agent_id → fallback to first with key
    let agent = match &config.agent_id {
        Some(aid) => agent_svc.get_agent_with_key(aid).await?.filter(|a| a.api_key.is_some()),
        None => None,
    };
    let agent = match agent {
        Some(a) => a,
        None => match agent_svc.get_first_agent_with_key(&ctx.team_id).await? {
            Some(a) => a,
            None => {
                tracing::warn!("[doc-analysis] No agent with API key for team {}", ctx.team_id);
                let _ = smart_log_svc.cancel_pending_analysis(&ctx.team_id, &ctx.doc_id).await;
                return Ok(());
            }
        },
    };
    tracing::info!("[doc-analysis] Using agent={} api_format={:?} has_key={}", agent.id, agent.api_format, agent.api_key.is_some());

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
            allowed_extensions: Some(vec![
                "document_tools".to_string(),
                "developer".to_string(),
            ]),
            allowed_skill_ids: None,
            retry_config: None,
            max_turns: None,
            tool_timeout_seconds: None,
            max_portal_retry_rounds: None,
            require_final_report: false,
            portal_restricted: false,
        })
        .await?;

    let session_id = session.session_id;

    let workspace_path = runtime::create_workspace_dir(
        &workspace_root,
        &[
            (&ctx.team_id, "team_id"),
            ("doc-analysis", "category"),
            (&session_id, "session_id"),
        ],
    )?;
    if let Err(e) = agent_svc
        .set_session_workspace(&session_id, &workspace_path)
        .await
    {
        tracing::warn!("Failed to set workspace for session {}: {}", session_id, e);
    }

    let mut user_message = build_analysis_prompt(&ctx);
    if let Some(extra) = &ctx.extra_instructions {
        user_message.push_str("\n\n## 用户补充要求\n");
        user_message.push_str(extra);
    }

    let task_manager = Arc::new(TaskManager::new());
    let broadcaster = Arc::new(NoopBroadcaster);
    let cancel_token = CancellationToken::new();

    // Build LLM overrides only when a custom API URL is configured (standalone API mode).
    // Without api_url, the other fields (api_key, model, api_format) would incorrectly
    // override the selected agent's own credentials.
    let llm_overrides = match config.api_url.as_deref().filter(|s| !s.is_empty()) {
        Some(url) => {
            let mut ov = serde_json::Map::new();
            ov.insert("api_url".into(), serde_json::json!(url));
            if let Some(k) = config.api_key.as_deref().filter(|s| !s.is_empty()) { ov.insert("api_key".into(), serde_json::json!(k)); }
            if let Some(m) = config.model.as_deref().filter(|s| !s.is_empty()) { ov.insert("model".into(), serde_json::json!(m)); }
            if let Some(f) = config.api_format.as_deref().filter(|s| !s.is_empty()) { ov.insert("api_format".into(), serde_json::json!(f)); }
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

    tracing::info!("[doc-analysis] Calling execute_via_bridge for session={}", session_id);
    let exec_result = tokio::time::timeout(
        std::time::Duration::from_secs(AGENT_TIMEOUT_SECS),
        runtime::execute_via_bridge(
            &db,
            &agent_svc,
            &task_manager,
            &broadcaster,
            &session_id,
            &agent.id,
            &session_id,
            &user_message,
            cancel_token.clone(),
            Some(&workspace_path),
            llm_overrides,
            None,
        ),
    )
    .await;

    let analysis_text = match &exec_result {
        Ok(Ok(())) => {
            tracing::info!("[doc-analysis] execute_via_bridge OK, extracting response");
            if let Ok(Some(updated_session)) = agent_svc.get_session(&session_id).await {
                let text = runtime::extract_last_assistant_text(&updated_session.messages_json);
                tracing::info!("[doc-analysis] Extracted text length: {}", text.as_ref().map(|t| t.len()).unwrap_or(0));
                text
            } else {
                tracing::warn!("[doc-analysis] Session not found after execution");
                None
            }
        }
        Ok(Err(e)) => {
            tracing::warn!("[doc-analysis] Agent execution FAILED: {}", e);
            None
        }
        Err(_) => {
            tracing::warn!("[doc-analysis] Agent TIMED OUT ({}s)", AGENT_TIMEOUT_SECS);
            cancel_token.cancel();
            None
        }
    };

    // Cleanup session and workspace regardless of smart log outcome
    let _ = agent_svc.delete_session(&session_id).await;
    let _ = runtime::cleanup_workspace_dir(Some(&workspace_path));

    let doc_name = ctx.doc_name.clone();

    // Determine analysis text and status
    let (text, status) = match analysis_text {
        Some(ref t) if !t.trim().is_empty() => {
            let truncated: String = t.trim().chars().take(MAX_ANALYSIS_CHARS).collect();
            (truncated, "completed")
        }
        _ => (format!("解读了文档「{}」", doc_name), "failed"),
    };

    // Try to attach to the existing upload entry; if none exists, create a standalone AI entry
    let attached = smart_log_svc
        .attach_analysis(&ctx.team_id, &ctx.doc_id, &text, status)
        .await?;
    if !attached {
        let smart_ctx = SmartLogContext::ai_fallback(ctx.team_id, ctx.doc_id, ctx.doc_name);
        let entry_id = smart_log_svc.create(&smart_ctx).await?;
        smart_log_svc.update_ai_summary(&entry_id, &text, status).await?;
    }

    tracing::info!("Document analysis completed for '{}'", doc_name);
    Ok(())
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
    if m.contains("zip") || m.contains("rar") || m.contains("7z") || m.contains("tar") || m.contains("gzip") {
        return "archive";
    }
    if m.contains("presentation") || m.contains("powerpoint") || m.contains("pptx") || m.contains("ppt") {
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
        ("pdf", false) => "1. 使用 read_document 工具读取文档（会导出为文件）\n2. 使用 developer shell 工具提取 PDF 文本（如 pdftotext 或 Python pdfplumber）\n3. 按页面/章节结构分析内容",
        ("pdf", true) => "1. Use read_document tool (exports file to workspace)\n2. Use developer shell to extract PDF text (e.g. pdftotext or Python pdfplumber)\n3. Analyze content by page/chapter structure",
        ("archive", false) => "1. 使用 read_document 工具读取（会导出为文件）\n2. 使用 developer shell 解压文件\n3. 逐个分析解压后的文件内容",
        ("archive", true) => "1. Use read_document tool (exports file to workspace)\n2. Use developer shell to extract the archive\n3. Analyze each extracted file",
        ("presentation", false) => "1. 使用 read_document 工具读取（会导出为文件）\n2. 使用 developer shell 提取演示文稿内容（如 python-pptx）\n3. 逐页提取文字、标题和结构",
        ("presentation", true) => "1. Use read_document tool (exports file to workspace)\n2. Use developer shell to extract slides (e.g. python-pptx)\n3. Extract text and structure page by page",
        ("spreadsheet", false) => "1. 使用 read_document 工具读取文档内容\n2. 描述数据结构、字段含义\n3. 总结关键数据和统计信息",
        ("spreadsheet", true) => "1. Use read_document tool to read content\n2. Describe data structure and field meanings\n3. Summarize key data and statistics",
        ("word", false) => "1. 使用 read_document 工具读取（会导出为文件）\n2. 使用 developer shell 提取文档文本（如 python-docx）\n3. 按章节结构分析内容",
        ("word", true) => "1. Use read_document tool (exports file to workspace)\n2. Use developer shell to extract text (e.g. python-docx)\n3. Analyze content by section structure",
        (_, false) => "1. 使用 read_document 工具读取文档内容\n2. 分析文本结构和关键信息",
        (_, true) => "1. Use read_document tool to read the document content\n2. Analyze text structure and key information",
    };

    if en {
        format!(
            "Analyze the document \"{name}\" (MIME: {mime}, Doc ID: {doc_id}).\n\n\
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
            name = ctx.doc_name, mime = ctx.mime_type, doc_id = ctx.doc_id, steps = type_steps,
        )
    } else {
        format!(
            "请分析文档「{name}」(MIME: {mime}, 文档ID: {doc_id})。\n\n\
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
            name = ctx.doc_name, mime = ctx.mime_type, doc_id = ctx.doc_id, steps = type_steps,
        )
    }
}
