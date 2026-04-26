//! Document Tools — platform extension for Agent to read/create/search/list documents
//!
//! Implements McpClientTrait so it can be loaded by PlatformExtensionRunner.

use agime::agents::mcp_client::McpClientTrait;
use agime_team::db::MongoDb;
use agime_team::models::mongo::{
    AiWorkbenchGroup, DocumentCategory, DocumentOrigin, DocumentSourceSpaceType, DocumentStatus,
    DocumentSummary, PaginatedResponse,
};
use agime_team::services::mongo::{DocumentService, DocumentVersionService};
use anyhow::Result;
use mongodb::bson::{doc, oid::ObjectId, Document as BsonDocument};
use rmcp::model::*;
use rmcp::ServiceError;
use serde_json::json;
use std::collections::{HashMap, HashSet};
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use super::capability_policy::{DocumentScopeMode, DocumentWriteMode, ResolvedDocumentPolicy};
use super::chat_channels::ChatChannelService;
use super::context_injector::sanitize_filename;
use super::local_fs_workspace_store::LocalFsWorkspaceStore;
use super::workspace_physical_store::WorkspacePhysicalStore;
use super::workspace_types::{WorkspaceArtifactArea, WorkspaceArtifactRecord};

/// Provider of document tools for agents
pub struct DocumentToolsProvider {
    db: Arc<MongoDb>,
    team_id: String,
    session_id: Option<String>,
    agent_id: Option<String>,
    workspace_path: Option<String>,
    /// Session-scoped allowed documents (used for restricted sessions such as portal runtime).
    allowed_document_ids: Option<HashSet<String>>,
    /// If true, document read/list/search must be restricted to `allowed_document_ids`.
    restrict_to_allowed_documents: bool,
    /// Session document visibility range.
    scope_mode: DocumentScopeMode,
    /// Session write policy for document mutation tools.
    write_mode: DocumentWriteMode,
    /// Backward-compatible single-field policy value.
    legacy_document_access_mode: Option<String>,
    /// Document IDs read during this session (for automatic lineage tracking)
    read_doc_ids: Mutex<Vec<String>>,
    /// First materialized workspace path for each accessed document in this session.
    materialized_doc_paths: Mutex<HashMap<String, String>>,
}

impl DocumentToolsProvider {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        db: Arc<MongoDb>,
        team_id: String,
        session_id: Option<String>,
        agent_id: Option<String>,
        workspace_path: Option<String>,
        allowed_document_ids: Option<Vec<String>>,
        restrict_to_allowed_documents: bool,
        document_policy: ResolvedDocumentPolicy,
    ) -> Self {
        let normalized_allowed = allowed_document_ids
            .unwrap_or_default()
            .into_iter()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect::<HashSet<_>>();
        Self {
            db,
            team_id,
            session_id,
            agent_id,
            workspace_path,
            allowed_document_ids: if normalized_allowed.is_empty() {
                None
            } else {
                Some(normalized_allowed)
            },
            restrict_to_allowed_documents,
            scope_mode: Self::parse_scope_mode(
                document_policy.document_scope_mode.as_deref(),
                restrict_to_allowed_documents,
            ),
            write_mode: Self::parse_write_mode(
                document_policy.document_write_mode.as_deref(),
                restrict_to_allowed_documents,
            ),
            legacy_document_access_mode: document_policy.document_access_mode,
            read_doc_ids: Mutex::new(Vec::new()),
            materialized_doc_paths: Mutex::new(HashMap::new()),
        }
    }

    fn service(&self) -> DocumentService {
        DocumentService::new((*self.db).clone())
    }

    fn parse_scope_mode(
        document_scope_mode: Option<&str>,
        restrict_to_allowed_documents: bool,
    ) -> DocumentScopeMode {
        let mode = document_scope_mode
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| s.to_ascii_lowercase());
        match mode.as_deref() {
            Some("attached_only" | "attached-only" | "attachedonly") => {
                DocumentScopeMode::AttachedOnly
            }
            Some("channel_bound" | "channel-bound" | "channelbound") => {
                DocumentScopeMode::ChannelBound
            }
            Some("portal_bound" | "portal-bound" | "portalbound") => DocumentScopeMode::PortalBound,
            Some("full") => DocumentScopeMode::Full,
            _ => {
                if restrict_to_allowed_documents {
                    DocumentScopeMode::AttachedOnly
                } else {
                    DocumentScopeMode::Full
                }
            }
        }
    }

    fn parse_write_mode(
        document_write_mode: Option<&str>,
        restrict_to_allowed_documents: bool,
    ) -> DocumentWriteMode {
        let mode = document_write_mode
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| s.to_ascii_lowercase());
        match mode.as_deref() {
            Some("full_write" | "full-write" | "fullwrite") => DocumentWriteMode::FullWrite,
            Some("read_only") | Some("readonly") | Some("read-only") => DocumentWriteMode::ReadOnly,
            Some("draft_only") | Some("draft-only") | Some("draftonly") => {
                DocumentWriteMode::DraftOnly
            }
            Some("controlled_write") | Some("controlled-write") | Some("controlledwrite") => {
                DocumentWriteMode::ControlledWrite
            }
            _ => {
                if restrict_to_allowed_documents {
                    DocumentWriteMode::ReadOnly
                } else {
                    DocumentWriteMode::FullWrite
                }
            }
        }
    }

    fn can_write_documents(&self) -> bool {
        !matches!(self.write_mode, DocumentWriteMode::ReadOnly)
    }

    fn infer_ai_workbench_group(
        category: DocumentCategory,
        name: &str,
        mime_type: &str,
        lineage_description: Option<&str>,
    ) -> AiWorkbenchGroup {
        match category {
            DocumentCategory::Report => return AiWorkbenchGroup::Report,
            DocumentCategory::Summary => return AiWorkbenchGroup::Summary,
            DocumentCategory::Review => return AiWorkbenchGroup::Review,
            DocumentCategory::Code => return AiWorkbenchGroup::Code,
            DocumentCategory::Translation => return AiWorkbenchGroup::Draft,
            DocumentCategory::General | DocumentCategory::Other => {}
        }

        let haystack = format!(
            "{} {} {}",
            name.to_ascii_lowercase(),
            mime_type.to_ascii_lowercase(),
            lineage_description.unwrap_or_default().to_ascii_lowercase()
        );

        let contains_any =
            |patterns: &[&str]| patterns.iter().any(|pattern| haystack.contains(pattern));

        if contains_any(&[
            "roadmap", "plan", "proposal", "spec", "prd", "计划", "方案", "提案", "规划",
        ]) {
            return AiWorkbenchGroup::Plan;
        }
        if contains_any(&[
            "research",
            "investigation",
            "analysis",
            "study",
            "调研",
            "研究",
            "分析",
            "评估",
        ]) {
            return AiWorkbenchGroup::Research;
        }
        if contains_any(&["report", "日报", "周报", "月报", "报告"]) {
            return AiWorkbenchGroup::Report;
        }
        if contains_any(&["summary", "recap", "digest", "总结", "摘要", "汇总"]) {
            return AiWorkbenchGroup::Summary;
        }
        if contains_any(&["review", "审查", "审核", "检查", "qa"]) {
            return AiWorkbenchGroup::Review;
        }
        if contains_any(&[
            "application/pdf",
            "presentation",
            "spreadsheet",
            "powerpoint",
            "excel",
            ".ppt",
            ".pptx",
            ".xlsx",
            ".pdf",
        ]) {
            return AiWorkbenchGroup::Artifact;
        }
        if contains_any(&[
            "text/x-",
            "application/json",
            "typescript",
            "javascript",
            ".ts",
            ".tsx",
            ".js",
            ".jsx",
            ".py",
            ".rs",
            ".go",
            ".java",
            ".c",
            ".cpp",
        ]) {
            return AiWorkbenchGroup::Code;
        }

        AiWorkbenchGroup::Other
    }

    async fn resolve_ai_workbench_source_context(
        &self,
        group: AiWorkbenchGroup,
    ) -> (
        Option<DocumentSourceSpaceType>,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<AiWorkbenchGroup>,
    ) {
        let Some(session_id) = self.session_id.clone() else {
            return (
                Some(DocumentSourceSpaceType::Unknown),
                None,
                None,
                None,
                None,
                None,
                None,
                Some(group),
            );
        };

        let coll = self.db.collection::<BsonDocument>("agent_sessions");
        let session = match coll
            .find_one(doc! { "session_id": &session_id }, None)
            .await
        {
            Ok(Some(doc)) => doc,
            _ => {
                return (
                    Some(DocumentSourceSpaceType::Unknown),
                    Some(session_id),
                    None,
                    None,
                    None,
                    None,
                    None,
                    Some(group),
                );
            }
        };

        let session_source = session
            .get_str("session_source")
            .ok()
            .map(|s| s.trim().to_ascii_lowercase())
            .unwrap_or_else(|| "chat".to_string());

        match session_source.as_str() {
            "channel_runtime" | "channel_conversation" => (
                Some(DocumentSourceSpaceType::TeamChannel),
                session
                    .get_str("source_channel_id")
                    .ok()
                    .map(|s| s.to_string()),
                session
                    .get_str("source_channel_name")
                    .ok()
                    .map(|s| s.to_string()),
                session
                    .get_str("source_channel_id")
                    .ok()
                    .map(|s| s.to_string()),
                session
                    .get_str("source_channel_name")
                    .ok()
                    .map(|s| s.to_string()),
                session
                    .get_str("source_thread_root_id")
                    .ok()
                    .map(|s| s.to_string()),
                None,
                Some(group),
            ),
            "portal" | "portal_coding" | "portal_manager" => (
                Some(DocumentSourceSpaceType::Portal),
                session.get_str("portal_id").ok().map(|s| s.to_string()),
                session.get_str("portal_slug").ok().map(|s| s.to_string()),
                None,
                None,
                None,
                None,
                Some(group),
            ),
            "system" => (
                Some(DocumentSourceSpaceType::System),
                Some(session_id),
                Some("System".to_string()),
                None,
                None,
                None,
                None,
                Some(group),
            ),
            _ => (
                Some(DocumentSourceSpaceType::PersonalChat),
                Some(session_id),
                session
                    .get_str("title")
                    .ok()
                    .map(|s| s.to_string())
                    .or_else(|| Some("个人对话".to_string())),
                None,
                None,
                None,
                None,
                Some(group),
            ),
        }
    }

    fn write_denied_message(&self) -> &'static str {
        match self.write_mode {
            DocumentWriteMode::ReadOnly => {
                "Document write denied: this session is read-only. You can read/search/list documents only."
            }
            _ => "Document write denied by session policy.",
        }
    }

    fn write_mode_name(&self) -> &'static str {
        match self.write_mode {
            DocumentWriteMode::FullWrite => "full_write",
            DocumentWriteMode::ReadOnly => "read_only",
            DocumentWriteMode::DraftOnly => "draft_only",
            DocumentWriteMode::ControlledWrite => "controlled_write",
        }
    }

    fn scope_mode_name(&self) -> &'static str {
        self.scope_mode.as_str()
    }

    fn log_write_denied(&self, action: &str, reason: &str) {
        tracing::warn!(
            "document write denied: action={}, mode={}, team_id={}, session_id={:?}, agent_id={:?}, reason={}",
            action,
            self.write_mode_name(),
            self.team_id,
            self.session_id,
            self.agent_id,
            reason
        );
    }

    fn log_access_denied(&self, action: &str, doc_id: &str, reason: &str) {
        tracing::warn!(
            "document access denied: action={}, mode={}, team_id={}, session_id={:?}, agent_id={:?}, doc_id={}, restricted={}, reason={}",
            action,
            self.write_mode_name(),
            self.team_id,
            self.session_id,
            self.agent_id,
            doc_id,
            self.restrict_to_allowed_documents,
            reason
        );
    }

    /// Record a document ID as read for automatic lineage tracking.
    fn track_read(&self, doc_id: &str) {
        let mut tracked = self.read_doc_ids.lock().unwrap();
        if !tracked.iter().any(|id| id == doc_id) {
            tracked.push(doc_id.to_string());
        }
    }

    fn track_materialized_path(&self, doc_id: &str, file_path: &str) {
        self.track_read(doc_id);
        self.materialized_doc_paths
            .lock()
            .unwrap()
            .entry(doc_id.to_string())
            .or_insert_with(|| file_path.to_string());
    }

    fn existing_materialized_path(&self, doc_id: &str) -> Option<String> {
        self.materialized_doc_paths
            .lock()
            .unwrap()
            .get(doc_id)
            .cloned()
    }

    fn document_access_ledger_path(&self, doc_id: &str) -> Option<PathBuf> {
        let workspace = self.workspace_path.as_deref()?;
        Some(
            PathBuf::from(workspace)
                .join(".agime")
                .join("document_access")
                .join(format!("{doc_id}.json")),
        )
    }

    async fn existing_materialized_path_for_session(&self, doc_id: &str) -> Option<String> {
        if let Some(path) = self.existing_materialized_path(doc_id) {
            return Some(path);
        }
        let ledger_path = self.document_access_ledger_path(doc_id)?;
        let raw = tokio::fs::read_to_string(ledger_path).await.ok()?;
        let entry: DocumentAccessLedgerEntry = serde_json::from_str(&raw).ok()?;
        self.materialized_doc_paths
            .lock()
            .unwrap()
            .insert(doc_id.to_string(), entry.file_path.clone());
        Some(entry.file_path)
    }

    async fn persist_materialized_path_for_session(
        &self,
        doc_id: &str,
        file_path: &str,
    ) -> Result<()> {
        let Some(ledger_path) = self.document_access_ledger_path(doc_id) else {
            return Ok(());
        };
        if let Some(parent) = ledger_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let entry = DocumentAccessLedgerEntry {
            doc_id: doc_id.to_string(),
            file_path: file_path.to_string(),
        };
        tokio::fs::write(&ledger_path, serde_json::to_vec(&entry)?).await?;
        self.materialized_doc_paths
            .lock()
            .unwrap()
            .insert(doc_id.to_string(), file_path.to_string());
        Ok(())
    }

    fn repeated_access_redirect(
        &self,
        tool_name: &str,
        doc_id: &str,
        doc_name: &str,
        file_path: &str,
    ) -> String {
        json!({
            "type": "document_access_redirect",
            "doc_id": doc_id,
            "doc_name": doc_name,
            "file_path": file_path,
            "access_established": true,
            "content_accessed": false,
            "analysis_complete": false,
            "reason_code": "document_access_already_established",
            "next_action": "use_local_tool",
            "message": format!(
                "{} was already completed for document '{}' ({}). Reuse the existing workspace file at '{}' and continue with developer shell, MCP, or another local tool instead of calling another document access tool.",
                tool_name,
                doc_name,
                doc_id,
                file_path,
            ),
        })
        .to_string()
    }

    fn is_doc_allowed(&self, doc_id: &str) -> bool {
        if !self.restrict_to_allowed_documents {
            return true;
        }
        self.allowed_document_ids
            .as_ref()
            .map(|set| set.contains(doc_id))
            .unwrap_or(false)
    }

    fn can_access_doc_summary(&self, doc: &DocumentSummary) -> bool {
        if !self.restrict_to_allowed_documents {
            return true;
        }
        if self.is_doc_allowed(&doc.id) {
            return true;
        }
        let related_to_allowed_source = self
            .allowed_document_ids
            .as_ref()
            .map(|set| doc.source_document_ids.iter().any(|id| set.contains(id)))
            .unwrap_or(false);
        let created_in_this_session = self
            .session_id
            .as_deref()
            .zip(doc.source_session_id.as_deref())
            .map(|(sid, source_sid)| sid == source_sid)
            .unwrap_or(false);
        related_to_allowed_source || created_in_this_session
    }

    async fn get_accessible_doc_metadata(
        &self,
        doc_id: &str,
        action: &str,
    ) -> Result<DocumentSummary> {
        let svc = self.service();
        let meta = svc.get_metadata(&self.team_id, doc_id).await?;
        if self.can_access_doc_summary(&meta) {
            return Ok(meta);
        }
        let reason = "Document access denied: this session can only access bound documents and related AI drafts";
        self.log_access_denied(action, doc_id, reason);
        Err(anyhow::anyhow!(reason))
    }

    async fn list_related_ai_documents_for_scope(
        &self,
        page: Option<u64>,
        limit: Option<u64>,
    ) -> Result<PaginatedResponse<DocumentSummary>> {
        let Some(allowed) = &self.allowed_document_ids else {
            let resolved_page = page.unwrap_or(1).max(1);
            let resolved_limit = limit.unwrap_or(100).clamp(1, 500);
            return Ok(PaginatedResponse::new(
                Vec::new(),
                0,
                resolved_page,
                resolved_limit,
            ));
        };
        let source_ids = allowed.iter().cloned().collect::<Vec<_>>();
        self.service()
            .list_related_ai_documents(&self.team_id, &source_ids, page, limit)
            .await
    }

    async fn list_allowed_documents(&self) -> Result<Vec<DocumentSummary>> {
        let svc = self.service();
        let mut docs = Vec::new();
        let mut seen = HashSet::new();

        if let Some(allowed) = &self.allowed_document_ids {
            for doc_id in allowed {
                match svc.get_metadata(&self.team_id, doc_id).await {
                    Ok(meta) => {
                        if seen.insert(meta.id.clone()) {
                            docs.push(meta);
                        }
                    }
                    Err(err) => tracing::debug!(
                        "Skip unavailable restricted document {} for team {}: {}",
                        doc_id,
                        self.team_id,
                        err
                    ),
                }
            }
        }

        for doc in self.list_channel_folder_documents().await? {
            if seen.insert(doc.id.clone()) {
                docs.push(doc);
            }
        }
        docs.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        Ok(docs)
    }

    async fn get_current_channel_context(&self) -> Result<Option<(String, String)>> {
        let Some(session_id) = self.session_id.as_deref() else {
            return Ok(None);
        };

        let sessions = self.db.collection::<BsonDocument>("agent_sessions");
        let Some(session) = sessions
            .find_one(doc! { "session_id": session_id }, None)
            .await?
        else {
            return Ok(None);
        };

        let source = session
            .get_str("session_source")
            .ok()
            .map(|s| s.trim().to_ascii_lowercase())
            .unwrap_or_default();
        if source != "channel_runtime" && source != "channel_conversation" {
            return Ok(None);
        }

        let Some(channel_id) = session
            .get_str("source_channel_id")
            .ok()
            .map(|s| s.to_string())
        else {
            return Ok(None);
        };

        let channels = self.db.collection::<BsonDocument>("chat_channels");
        let Some(channel) = channels
            .find_one(doc! { "channel_id": &channel_id }, None)
            .await?
        else {
            return Ok(None);
        };

        let folder_path = channel
            .get_str("document_folder_path")
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());

        match folder_path {
            Some(folder) => Ok(Some((channel_id, folder))),
            None => Ok(None),
        }
    }

    async fn list_channel_folder_documents(&self) -> Result<Vec<DocumentSummary>> {
        let Some((_, folder_path)) = self.get_current_channel_context().await? else {
            return Ok(Vec::new());
        };
        let result = self
            .service()
            .list_paginated(
                &self.team_id,
                Some(folder_path.as_str()),
                Some(1),
                Some(500),
                None,
                None,
            )
            .await?;
        Ok(result.items)
    }

    async fn list_current_channel_ai_documents(
        &self,
        page: Option<u64>,
        limit: Option<u64>,
    ) -> Result<PaginatedResponse<DocumentSummary>> {
        let Some((channel_id, folder_path)) = self.get_current_channel_context().await? else {
            return Ok(PaginatedResponse::new(
                vec![],
                0,
                page.unwrap_or(1).max(1),
                limit.unwrap_or(100).min(500),
            ));
        };
        let mut result = self
            .service()
            .list_ai_workbench(
                &self.team_id,
                None,
                None,
                Some("team_channel"),
                Some(channel_id.as_str()),
                None,
                page,
                limit,
            )
            .await?;
        result
            .items
            .retain(|doc| !Self::is_promoted_ai_output(doc, Some(folder_path.as_str())));
        result.total = result.items.len() as u64;
        Ok(result)
    }

    fn is_promoted_ai_output(doc: &DocumentSummary, channel_folder_path: Option<&str>) -> bool {
        if matches!(
            doc.status,
            DocumentStatus::Accepted | DocumentStatus::Archived | DocumentStatus::Superseded
        ) {
            return true;
        }
        channel_folder_path
            .map(str::trim)
            .filter(|path| !path.is_empty())
            .map(|path| doc.folder_path == path)
            .unwrap_or(false)
    }

    fn matches_mime_filter(mime: &str, filter: Option<&str>) -> bool {
        let Some(filter) = filter.map(str::trim).filter(|s| !s.is_empty()) else {
            return true;
        };
        let mime_l = mime.to_ascii_lowercase();
        filter
            .split(',')
            .map(|s| s.trim().to_ascii_lowercase())
            .filter(|s| !s.is_empty())
            .any(|prefix| mime_l.starts_with(&prefix))
    }

    fn apply_page_limit(
        page: Option<u64>,
        limit: Option<u64>,
        total: usize,
    ) -> (usize, usize, u64, u64) {
        let page = page.unwrap_or(1).max(1);
        let limit = limit.unwrap_or(50).clamp(1, 500);
        let start = ((page - 1) * limit) as usize;
        let end = (start + limit as usize).min(total);
        (start, end, page, limit)
    }

    fn doc_matches_query(doc: &DocumentSummary, query_lower: &str) -> bool {
        if query_lower.is_empty() {
            return true;
        }
        doc.name.to_ascii_lowercase().contains(query_lower)
            || doc
                .display_name
                .as_deref()
                .unwrap_or("")
                .to_ascii_lowercase()
                .contains(query_lower)
            || doc
                .description
                .as_deref()
                .unwrap_or("")
                .to_ascii_lowercase()
                .contains(query_lower)
            || doc
                .tags
                .iter()
                .any(|tag| tag.to_ascii_lowercase().contains(query_lower))
    }

    fn status_key(status: DocumentStatus) -> &'static str {
        match status {
            DocumentStatus::Active => "active",
            DocumentStatus::Draft => "draft",
            DocumentStatus::Accepted => "accepted",
            DocumentStatus::Archived => "archived",
            DocumentStatus::Superseded => "superseded",
        }
    }

    fn status_label_zh(status: DocumentStatus) -> &'static str {
        match status {
            DocumentStatus::Active => "活跃",
            DocumentStatus::Draft => "草稿",
            DocumentStatus::Accepted => "已接受",
            DocumentStatus::Archived => "已归档",
            DocumentStatus::Superseded => "已取代",
        }
    }

    fn status_label_en(status: DocumentStatus) -> &'static str {
        match status {
            DocumentStatus::Active => "Active",
            DocumentStatus::Draft => "Draft",
            DocumentStatus::Accepted => "Accepted",
            DocumentStatus::Archived => "Archived",
            DocumentStatus::Superseded => "Superseded",
        }
    }

    fn status_display(status: DocumentStatus) -> serde_json::Value {
        json!({
            "key": Self::status_key(status),
            "label_zh": Self::status_label_zh(status),
            "label_en": Self::status_label_en(status),
        })
    }

    fn document_extension(name: &str) -> Option<String> {
        let trimmed = name.trim();
        let file_name = Path::new(trimmed)
            .file_name()
            .and_then(|value| value.to_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())?;
        let ext = file_name
            .rsplit_once('.')
            .map(|(_, ext)| ext.trim())
            .filter(|value| !value.is_empty())
            .filter(|value| value.len() <= 12)
            .filter(|value| value.chars().all(|ch| ch.is_ascii_alphanumeric()))?;
        Some(ext.to_ascii_lowercase())
    }

    fn document_class(name: &str, mime_type: &str) -> &'static str {
        let extension = Self::document_extension(name);
        let mime = mime_type.trim().to_ascii_lowercase();

        match extension.as_deref() {
            Some("md") | Some("markdown") => return "markdown",
            Some("pdf") => return "pdf",
            Some("zip") | Some("rar") | Some("7z") | Some("tar") | Some("gz") | Some("tgz")
            | Some("xz") | Some("bz2") | Some("zst") => return "archive",
            Some("xls") | Some("xlsx") | Some("xlsm") | Some("xlsb") | Some("csv")
            | Some("tsv") | Some("parquet") | Some("numbers") => return "spreadsheet",
            Some("ods") => return "spreadsheet",
            Some("ppt") | Some("pptx") | Some("pptm") | Some("pps") | Some("ppsx")
            | Some("ptx") | Some("odp") | Some("key") => return "slides",
            Some("html") | Some("htm") | Some("xml") | Some("xhtml") => return "web",
            Some("json") | Some("jsonl") | Some("ndjson") | Some("yaml") | Some("yml")
            | Some("toml") | Some("ini") | Some("ipynb") | Some("sql") => return "data",
            Some("doc") | Some("docx") | Some("docm") | Some("txt") | Some("rtf") | Some("odt")
            | Some("pages") | Some("epub") => return "office_doc",
            _ => {}
        }

        if mime.starts_with("text/markdown") {
            "markdown"
        } else if mime.starts_with("application/pdf") {
            "pdf"
        } else if mime.contains("zip")
            || mime.contains("rar")
            || mime.contains("7z")
            || mime.contains("tar")
            || mime.contains("gzip")
            || mime.contains("bzip")
            || mime.contains("xz")
            || mime.contains("zstd")
            || mime.contains("compressed")
            || mime.contains("archive")
        {
            "archive"
        } else if mime.contains("spreadsheet")
            || mime.contains("excel")
            || mime.contains("opendocument.spreadsheet")
            || mime.contains("numbers")
            || mime.starts_with("text/csv")
        {
            "spreadsheet"
        } else if mime.contains("presentation")
            || mime.contains("powerpoint")
            || mime.contains("opendocument.presentation")
            || mime.contains("keynote")
        {
            "slides"
        } else if mime.contains("html") || mime.contains("xml") {
            "web"
        } else if mime.contains("json")
            || mime.contains("jsonl")
            || mime.contains("yaml")
            || mime.contains("yml")
            || mime.contains("toml")
            || mime.contains("ini")
            || mime.contains("sqlite")
            || mime.contains("jupyter")
        {
            "data"
        } else if mime.contains("word")
            || mime.contains("document")
            || mime.contains("opendocument.text")
            || mime.starts_with("text/plain")
            || mime.contains("rtf")
            || mime.contains("epub")
        {
            "office_doc"
        } else {
            "document"
        }
    }

    fn document_ref(d: &DocumentSummary) -> String {
        format!(
            "[[doc:{}|{}|{}|{}]]",
            d.id,
            d.name,
            Self::status_key(d.status),
            Self::document_class(&d.name, &d.mime_type)
        )
    }

    fn display_line_zh(d: &DocumentSummary) -> String {
        let mut details = vec![Self::status_label_zh(d.status).to_string()];
        if !d.folder_path.trim().is_empty() && d.folder_path.trim() != "/" {
            details.push(format!("位于{}", d.folder_path.trim()));
        } else if d.folder_path.trim() == "/" {
            details.push("位于根目录".to_string());
        }

        format!("{}（{}）", Self::document_ref(d), details.join("，"))
    }

    fn display_line_en(d: &DocumentSummary) -> String {
        let mut details = vec![Self::status_label_en(d.status).to_string()];
        if !d.folder_path.trim().is_empty() && d.folder_path.trim() != "/" {
            details.push(format!("in {}", d.folder_path.trim()));
        } else if d.folder_path.trim() == "/" {
            details.push("in root".to_string());
        }

        format!("{} ({})", Self::document_ref(d), details.join(", "))
    }

    fn status_count_items(
        active: u64,
        draft: u64,
        accepted: u64,
        archived: u64,
        superseded: u64,
    ) -> Vec<serde_json::Value> {
        [
            (DocumentStatus::Active, active),
            (DocumentStatus::Draft, draft),
            (DocumentStatus::Accepted, accepted),
            (DocumentStatus::Archived, archived),
            (DocumentStatus::Superseded, superseded),
        ]
        .into_iter()
        .map(|(status, count)| {
            json!({
                "status": Self::status_key(status),
                "label_zh": Self::status_label_zh(status),
                "label_en": Self::status_label_en(status),
                "count": count,
            })
        })
        .collect()
    }

    fn as_document_list_item(d: &DocumentSummary) -> serde_json::Value {
        json!({
            "id": d.id,
            "name": d.name,
            "doc_ref": Self::document_ref(d),
            "display_line_zh": Self::display_line_zh(d),
            "display_line_en": Self::display_line_en(d),
            "document_class": Self::document_class(&d.name, &d.mime_type),
            "extension": Self::document_extension(&d.name),
            "mime_type": d.mime_type,
            "file_size": d.file_size,
            "folder_path": d.folder_path,
            "origin": d.origin,
            "status": d.status,
            "status_display": Self::status_display(d.status),
            "status_label_zh": Self::status_label_zh(d.status),
            "status_label_en": Self::status_label_en(d.status),
            "source_session_id": d.source_session_id,
        })
    }

    fn as_inventory_item(d: &DocumentSummary) -> serde_json::Value {
        json!({
            "id": d.id,
            "name": d.name,
            "doc_ref": Self::document_ref(d),
            "display_line_zh": Self::display_line_zh(d),
            "display_line_en": Self::display_line_en(d),
            "document_class": Self::document_class(&d.name, &d.mime_type),
            "extension": Self::document_extension(&d.name),
            "mime_type": d.mime_type,
            "file_size": d.file_size,
            "folder_path": d.folder_path,
            "origin": d.origin,
            "status": d.status,
            "status_display": Self::status_display(d.status),
            "status_label_zh": Self::status_label_zh(d.status),
            "status_label_en": Self::status_label_en(d.status),
            "source_session_id": d.source_session_id,
        })
    }

    fn tool_definitions() -> Vec<Tool> {
        vec![
            Tool {
                name: "document_inventory".into(),
                title: None,
                description: Some("Get a unified, fixed-format inventory for document overview questions. This tool returns both views in one response: Files (regular file library) and AI Workbench (agent-generated docs), with status counts to avoid confusion. Use this FIRST for queries like: '有哪些文档', 'AI工作台里有什么', '有没有草稿'. When answering in Chinese, you MUST list document items by reusing `display_line_zh` verbatim as the primary output. Preserve every `doc_ref` marker exactly as returned; do not rewrite, split, translate, or restyle it. If you need only the document name, keep the `doc_ref` marker intact.".into()),
                input_schema: serde_json::from_value(json!({
                    "type": "object",
                    "properties": {
                        "include_samples": { "type": "boolean", "description": "Whether to include sample document items in each view (default: true)" },
                        "sample_limit": { "type": "integer", "description": "Max sample items per view (default: 20, range: 1..100)" }
                    }
                })).unwrap_or_default(),
                output_schema: None,
                annotations: None,
                execution: None,
                icons: None,
                meta: None,
            },
            Tool {
                name: "read_document".into(),
                title: None,
                description: Some("Establish document-area access and materialize the selected document into the workspace. Use the returned workspace path with developer shell, MCP, or another local tool to inspect the actual content. Legacy chunking arguments are accepted for compatibility but ignored.".into()),
                input_schema: serde_json::from_value(json!({
                    "type": "object",
                    "properties": {
                        "doc_id": { "type": "string", "description": "Document ID" },
                        "offset": { "type": "integer", "description": "Legacy chunked-read offset. Ignored after the workspace-materialization migration." },
                        "limit": { "type": "integer", "description": "Legacy chunked-read limit. Ignored after the workspace-materialization migration." }
                    },
                    "required": ["doc_id"]
                })).unwrap_or_default(),
                output_schema: None,
                annotations: None,
                execution: None,
                icons: None,
                meta: None,
            },
            Tool {
                name: "export_document".into(),
                title: None,
                description: Some("Export any document to workspace path and return local file path. Prefer this for binary or mixed formats (zip/pdf/docx/pptx/etc.) and task delivery flows.".into()),
                input_schema: serde_json::from_value(json!({
                    "type": "object",
                    "properties": {
                        "doc_id": { "type": "string", "description": "Document ID" },
                        "output_dir": { "type": "string", "description": "Relative directory under workspace (default: documents). Example: output/docs_export" },
                        "output_name": { "type": "string", "description": "Optional output file name override" }
                    },
                    "required": ["doc_id"]
                })).unwrap_or_default(),
                output_schema: None,
                annotations: None,
                execution: None,
                icons: None,
                meta: None,
            },
            Tool {
                name: "import_document_to_workspace".into(),
                title: None,
                description: Some("Import a document from the formal document area into this conversation workspace. Defaults to attachments/ and records the imported file in workspace metadata.".into()),
                input_schema: serde_json::from_value(json!({
                    "type": "object",
                    "properties": {
                        "doc_id": { "type": "string", "description": "Document ID" },
                        "output_dir": { "type": "string", "description": "Relative directory under workspace (default: attachments)." },
                        "output_name": { "type": "string", "description": "Optional output file name override." }
                    },
                    "required": ["doc_id"]
                })).unwrap_or_default(),
                output_schema: None,
                annotations: None,
                execution: None,
                icons: None,
                meta: None,
            },
            Tool {
                name: "document_session_policy".into(),
                title: None,
                description: Some("Return current document access policy for this session, including read/write scope and restrictions.".into()),
                input_schema: serde_json::from_value(json!({
                    "type": "object",
                    "properties": {}
                })).unwrap_or_default(),
                output_schema: None,
                annotations: None,
                execution: None,
                icons: None,
                meta: None,
            },
            Tool {
                name: "create_document".into(),
                title: None,
                description: Some("Create a new text document in the team's document store. When creating a document derived from other documents (i.e. you have read source documents before), you MUST provide lineage_description explaining what you did and why.".into()),
                input_schema: serde_json::from_value(json!({
                    "type": "object",
                    "properties": {
                        "name": { "type": "string", "description": "File name" },
                        "content": { "type": "string", "description": "Text content" },
                        "mime_type": { "type": "string", "description": "MIME type (default: text/plain)" },
                        "folder_path": { "type": "string", "description": "Folder path" },
                        "source_document_ids": {
                            "type": "array", "items": { "type": "string" },
                            "description": "Source document IDs this was derived from"
                        },
                        "lineage_description": {
                            "type": "string",
                            "description": "REQUIRED when source documents exist. Describe specifically what you did: what content was extracted, translated, summarized, merged, or transformed, and why. Example: 'Translated the full document from Chinese to English, preserving original formatting and technical terms', 'Extracted financial data from the RAR archive (3 Excel files) and consolidated into a summary report'"
                        },
                        "category": {
                            "type": "string",
                            "enum": ["general","report","translation","summary","review","code","other"],
                            "description": "Document category"
                        }
                    },
                    "required": ["name", "content"]
                })).unwrap_or_default(),
                output_schema: None,
                annotations: None,
                execution: None,
                icons: None,
                meta: None,
            },
            Tool {
                name: "create_document_from_file".into(),
                title: None,
                description: Some("Create a new document from a workspace file path. Use this for binary deliverables like PPTX/PDF/XLSX generated during tasks. When source documents exist, lineage_description is required.".into()),
                input_schema: serde_json::from_value(json!({
                    "type": "object",
                    "properties": {
                        "file_path": { "type": "string", "description": "Relative workspace file path, e.g. output/result.ext" },
                        "name": { "type": "string", "description": "Document name in store (default: source file name)" },
                        "mime_type": { "type": "string", "description": "MIME type override (optional)" },
                        "folder_path": { "type": "string", "description": "Folder path" },
                        "source_document_ids": {
                            "type": "array", "items": { "type": "string" },
                            "description": "Source document IDs this was derived from"
                        },
                        "lineage_description": {
                            "type": "string",
                            "description": "REQUIRED when source documents exist. Describe what was transformed and why."
                        },
                        "category": {
                            "type": "string",
                            "enum": ["general","report","translation","summary","review","code","other"],
                            "description": "Document category"
                        }
                    },
                    "required": ["file_path"]
                })).unwrap_or_default(),
                output_schema: None,
                annotations: None,
                execution: None,
                icons: None,
                meta: None,
            },
            Tool {
                name: "publish_workspace_artifact".into(),
                title: None,
                description: Some("Publish a workspace artifact into the formal document area. Defaults to the current session's publish target when available, such as a channel document folder.".into()),
                input_schema: serde_json::from_value(json!({
                    "type": "object",
                    "properties": {
                        "file_path": { "type": "string", "description": "Relative workspace file path to publish." },
                        "name": { "type": "string", "description": "Optional document name override." },
                        "mime_type": { "type": "string", "description": "Optional MIME type override." },
                        "folder_path": { "type": "string", "description": "Optional explicit document folder path. If omitted, the current workspace publication target is used when available." },
                        "source_document_ids": {
                            "type": "array", "items": { "type": "string" },
                            "description": "Source document IDs this artifact derives from."
                        },
                        "lineage_description": {
                            "type": "string",
                            "description": "Required when source documents exist. Describe what was transformed and why."
                        },
                        "category": {
                            "type": "string",
                            "enum": ["general","report","translation","summary","review","code","other"],
                            "description": "Document category"
                        }
                    },
                    "required": ["file_path"]
                })).unwrap_or_default(),
                output_schema: None,
                annotations: None,
                execution: None,
                icons: None,
                meta: None,
            },
            Tool {
                name: "search_documents".into(),
                title: None,
                description: Some("Search documents in the file library by name, description or tags (this is not the AI Workbench view). 对应前端“文件”区域检索，不是“AI工作台”。When answering in Chinese, you MUST list each matched document by reusing `display_line_zh` verbatim. Preserve every `doc_ref` marker exactly as returned; do not rewrite, split, translate, or restyle it.".into()),
                input_schema: serde_json::from_value(json!({
                    "type": "object",
                    "properties": {
                        "query": { "type": "string", "description": "Search query" },
                        "mime_type": { "type": "string", "description": "Filter by MIME type prefix" },
                        "folder_path": { "type": "string", "description": "Filter by folder" }
                    },
                    "required": ["query"]
                })).unwrap_or_default(),
                output_schema: None,
                annotations: None,
                execution: None,
                icons: None,
                meta: None,
            },
            Tool {
                name: "list_documents".into(),
                title: None,
                description: Some("List documents in the team's file library (regular document view). 对应前端“文件”列表；AI Workbench（AI工作台）是另一视图，应使用 list_ai_workbench_documents。When answering in Chinese, you MUST list each document by reusing `display_line_zh` verbatim. Preserve every `doc_ref` marker exactly as returned; do not rewrite, split, translate, or restyle it.".into()),
                input_schema: serde_json::from_value(json!({
                    "type": "object",
                    "properties": {
                        "folder_path": { "type": "string", "description": "Filter by folder" },
                        "page": { "type": "integer", "description": "Page number" },
                        "limit": { "type": "integer", "description": "Items per page" }
                    }
                })).unwrap_or_default(),
                output_schema: None,
                annotations: None,
                execution: None,
                icons: None,
                meta: None,
            },
            Tool {
                name: "list_ai_workbench_documents".into(),
                title: None,
                description: Some("List AI Workbench documents (agent-generated documents). 对应前端“AI工作台”标签。AI Workbench is NOT a folder path; it is a source/status-based view. When answering in Chinese, you MUST list each document by reusing `display_line_zh` verbatim. Preserve every `doc_ref` marker exactly as returned; do not rewrite, split, translate, or restyle it.".into()),
                input_schema: serde_json::from_value(json!({
                    "type": "object",
                    "properties": {
                        "session_id": { "type": "string", "description": "Optional source chat session ID filter" },
                        "page": { "type": "integer", "description": "Page number" },
                        "limit": { "type": "integer", "description": "Items per page" }
                    }
                })).unwrap_or_default(),
                output_schema: None,
                annotations: None,
                execution: None,
                icons: None,
                meta: None,
            },
            Tool {
                name: "list_related_ai_documents".into(),
                title: None,
                description: Some("List AI Workbench documents related to one specific source document. Use this when the user wants to continue working on a particular bound document or one of its derived AI drafts, instead of browsing the whole AI Workbench. When answering in Chinese, you MUST list each document by reusing `display_line_zh` verbatim. Preserve every `doc_ref` marker exactly as returned; do not rewrite, split, translate, or restyle it.".into()),
                input_schema: serde_json::from_value(json!({
                    "type": "object",
                    "properties": {
                        "source_doc_id": { "type": "string", "description": "Source document ID" },
                        "page": { "type": "integer", "description": "Page number" },
                        "limit": { "type": "integer", "description": "Items per page" }
                    },
                    "required": ["source_doc_id"]
                })).unwrap_or_default(),
                output_schema: None,
                annotations: None,
                execution: None,
                icons: None,
                meta: None,
            },
            Tool {
                name: "update_document".into(),
                title: None,
                description: Some("Update the content of an existing document.".into()),
                input_schema: serde_json::from_value(json!({
                    "type": "object",
                    "properties": {
                        "doc_id": { "type": "string", "description": "Document ID" },
                        "content": { "type": "string", "description": "New text content" },
                        "message": { "type": "string", "description": "Version message describing the change" }
                    },
                    "required": ["doc_id", "content"]
                })).unwrap_or_default(),
                output_schema: None,
                annotations: None,
                execution: None,
                icons: None,
                meta: None,
            },
        ]
    }
}

#[async_trait::async_trait]
impl McpClientTrait for DocumentToolsProvider {
    async fn list_resources(
        &self,
        _next_cursor: Option<String>,
        _cancel_token: CancellationToken,
    ) -> std::result::Result<ListResourcesResult, ServiceError> {
        Ok(ListResourcesResult {
            resources: vec![],
            next_cursor: None,
            meta: None,
        })
    }

    async fn read_resource(
        &self,
        _uri: &str,
        _cancel_token: CancellationToken,
    ) -> std::result::Result<ReadResourceResult, ServiceError> {
        Err(ServiceError::TransportClosed)
    }

    async fn list_tools(
        &self,
        _next_cursor: Option<String>,
        _cancel_token: CancellationToken,
    ) -> std::result::Result<ListToolsResult, ServiceError> {
        Ok(ListToolsResult {
            tools: {
                let mut tools = Self::tool_definitions();
                if !self.can_write_documents() {
                    tools.retain(|t| {
                        t.name != "create_document"
                            && t.name != "create_document_from_file"
                            && t.name != "update_document"
                    });
                }
                tools
            },
            next_cursor: None,
            meta: None,
        })
    }

    async fn call_tool(
        &self,
        name: &str,
        arguments: Option<JsonObject>,
        _cancel_token: CancellationToken,
    ) -> std::result::Result<CallToolResult, ServiceError> {
        let args = arguments.unwrap_or_default();
        let result = match name {
            "document_inventory" => self.handle_document_inventory(&args).await,
            "read_document" => self.handle_read_document(&args).await,
            "export_document" => self.handle_export_document(&args).await,
            "import_document_to_workspace" => self.handle_import_document_to_workspace(&args).await,
            "document_session_policy" => self.handle_document_session_policy(&args).await,
            "create_document" => self.handle_create_document(&args).await,
            "create_document_from_file" => self.handle_create_document_from_file(&args).await,
            "publish_workspace_artifact" => self.handle_publish_workspace_artifact(&args).await,
            "search_documents" => self.handle_search_documents(&args).await,
            "list_documents" => self.handle_list_documents(&args).await,
            "list_ai_workbench_documents" => self.handle_list_ai_workbench_documents(&args).await,
            "list_related_ai_documents" => self.handle_list_related_ai_documents(&args).await,
            "update_document" => self.handle_update_document(&args).await,
            _ => Err(anyhow::anyhow!("Unknown tool: {}", name)),
        };

        match result {
            Ok(text) => Ok(build_document_tool_call_result(text, false)),
            Err(e) => Ok(build_document_tool_call_result(e.to_string(), true)),
        }
    }

    async fn list_tasks(
        &self,
        _cursor: Option<String>,
        _cancel_token: CancellationToken,
    ) -> std::result::Result<ListTasksResult, ServiceError> {
        Err(ServiceError::TransportClosed)
    }

    async fn get_task_info(
        &self,
        _task_id: &str,
        _cancel_token: CancellationToken,
    ) -> std::result::Result<GetTaskInfoResult, ServiceError> {
        Err(ServiceError::TransportClosed)
    }

    async fn get_task_result(
        &self,
        _task_id: &str,
        _cancel_token: CancellationToken,
    ) -> std::result::Result<TaskResult, ServiceError> {
        Err(ServiceError::TransportClosed)
    }

    async fn cancel_task(
        &self,
        _task_id: &str,
        _cancel_token: CancellationToken,
    ) -> std::result::Result<(), ServiceError> {
        Err(ServiceError::TransportClosed)
    }

    async fn list_prompts(
        &self,
        _next_cursor: Option<String>,
        _cancel_token: CancellationToken,
    ) -> std::result::Result<ListPromptsResult, ServiceError> {
        Ok(ListPromptsResult {
            prompts: vec![],
            next_cursor: None,
            meta: None,
        })
    }

    async fn get_prompt(
        &self,
        _name: &str,
        _arguments: serde_json::Value,
        _cancel_token: CancellationToken,
    ) -> std::result::Result<GetPromptResult, ServiceError> {
        Err(ServiceError::TransportClosed)
    }

    async fn subscribe(&self) -> mpsc::Receiver<ServerNotification> {
        let (_tx, rx) = mpsc::channel(1);
        rx
    }

    fn get_info(&self) -> Option<&InitializeResult> {
        None
    }
}

fn build_document_tool_call_result(raw: String, is_error: bool) -> CallToolResult {
    let structured_content = serde_json::from_str::<serde_json::Value>(&raw).ok();
    let rendered = structured_content
        .as_ref()
        .map(render_document_tool_result_for_model)
        .unwrap_or_else(|| raw.clone());
    CallToolResult {
        content: vec![Content::text(rendered)],
        structured_content,
        is_error: Some(is_error),
        meta: None,
    }
}

fn render_document_tool_result_for_model(value: &serde_json::Value) -> String {
    match value.get("type").and_then(|item| item.as_str()) {
        Some("document_access") | Some("workspace_export") | Some("binary_export") => {
            let name = value
                .get("doc_name")
                .or_else(|| value.get("source_name"))
                .or_else(|| value.get("name"))
                .and_then(|item| item.as_str())
                .unwrap_or("document");
            let path = value
                .get("file_path")
                .and_then(|item| item.as_str())
                .unwrap_or("workspace export");
            let mime = value
                .get("mime_type")
                .and_then(|item| item.as_str())
                .unwrap_or("unknown");
            format!(
                "Document access established for {} (MIME: {}). Workspace path: {}. Use developer shell, MCP, or another local tool to read the file content from this path.",
                name, mime, path
            )
        }
        Some("document_access_redirect") => {
            let name = value
                .get("doc_name")
                .and_then(|item| item.as_str())
                .unwrap_or("document");
            let path = value
                .get("file_path")
                .and_then(|item| item.as_str())
                .unwrap_or("workspace export");
            format!(
                "Document access is already established for {} at {}. Do not call another document access tool. Use developer shell, MCP, or another local tool to inspect the workspace file content from that path.",
                name, path
            )
        }
        Some("text_read") => {
            let name = value
                .get("doc_name")
                .and_then(|item| item.as_str())
                .unwrap_or("document");
            let mime = value
                .get("mime_type")
                .and_then(|item| item.as_str())
                .unwrap_or("unknown");
            let text = value
                .get("text")
                .and_then(|item| item.as_str())
                .unwrap_or("")
                .trim();
            if text.is_empty() {
                format!(
                    "Read document {} (MIME: {}). The document content was empty.",
                    name, mime
                )
            } else {
                format!(
                    "Read document {} (MIME: {}).\n\nDocument text content:\n{}",
                    name, mime, text
                )
            }
        }
        _ => value.to_string(),
    }
}

#[derive(Debug, Clone)]
struct WorkspaceMaterializedDocument {
    output_name: String,
    output_dir: String,
    file_path: String,
    mime_type: String,
    file_size: u64,
    source_name: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct DocumentAccessLedgerEntry {
    doc_id: String,
    file_path: String,
}

// ── Tool handler implementations ──

impl DocumentToolsProvider {
    async fn handle_document_inventory(&self, args: &JsonObject) -> Result<String> {
        let include_samples = args
            .get("include_samples")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
        let sample_limit = args
            .get("sample_limit")
            .and_then(|v| v.as_u64())
            .unwrap_or(20)
            .clamp(1, 100);

        // Restricted sessions are computed from allowed documents only.
        if self.restrict_to_allowed_documents {
            let allowed = self.list_allowed_documents().await?;
            let related_ai = self
                .list_related_ai_documents_for_scope(Some(1), Some(500))
                .await?;
            let channel_ai = self
                .list_current_channel_ai_documents(Some(1), Some(500))
                .await?;
            let channel_folder_path = self
                .get_current_channel_context()
                .await?
                .map(|(_, folder_path)| folder_path);
            let files_docs: Vec<&DocumentSummary> = allowed
                .iter()
                .filter(|d| {
                    d.origin != DocumentOrigin::Agent
                        || Self::is_promoted_ai_output(d, channel_folder_path.as_deref())
                })
                .collect();
            let mut ai_docs: Vec<DocumentSummary> = allowed
                .iter()
                .filter(|d| {
                    d.origin == DocumentOrigin::Agent
                        && !Self::is_promoted_ai_output(d, channel_folder_path.as_deref())
                })
                .cloned()
                .collect();
            for doc in related_ai.items {
                if !ai_docs.iter().any(|existing| existing.id == doc.id) {
                    ai_docs.push(doc);
                }
            }
            for doc in channel_ai.items {
                if !ai_docs.iter().any(|existing| existing.id == doc.id) {
                    ai_docs.push(doc);
                }
            }
            ai_docs.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));

            let mut active = 0_u64;
            let mut draft = 0_u64;
            let mut accepted = 0_u64;
            let mut archived = 0_u64;
            let mut superseded = 0_u64;
            for d in &ai_docs {
                match d.status {
                    DocumentStatus::Active => active += 1,
                    DocumentStatus::Draft => draft += 1,
                    DocumentStatus::Accepted => accepted += 1,
                    DocumentStatus::Archived => archived += 1,
                    DocumentStatus::Superseded => superseded += 1,
                }
            }

            let files_sample: Vec<serde_json::Value> = if include_samples {
                files_docs
                    .iter()
                    .take(sample_limit as usize)
                    .map(|d| Self::as_inventory_item(d))
                    .collect()
            } else {
                Vec::new()
            };
            let ai_sample: Vec<serde_json::Value> = if include_samples {
                ai_docs
                    .iter()
                    .take(sample_limit as usize)
                    .map(Self::as_inventory_item)
                    .collect()
            } else {
                Vec::new()
            };

            return Ok(json!({
                "view": "document_inventory",
                "note": "Unified inventory for restricted session scope (bound documents plus related AI drafts).",
                "guidance": "When answering in Chinese, you MUST output document items by reusing `display_line_zh` verbatim. Preserve every `doc_ref` marker exactly as returned; do not rewrite, split, translate, or restyle it.",
                "files": {
                    "total": files_docs.len(),
                    "sample_count": files_sample.len(),
                    "sample": files_sample,
                },
                "ai_workbench": {
                    "total": ai_docs.len(),
                    "status_counts": {
                        "active": active,
                        "draft": draft,
                        "accepted": accepted,
                        "archived": archived,
                        "superseded": superseded,
                    },
                    "status_counts_display": Self::status_count_items(active, draft, accepted, archived, superseded),
                    "sample_count": ai_sample.len(),
                    "sample": ai_sample,
                },
                "policy": {
                    "mode": self.write_mode_name(),
                    "restrict_to_allowed_documents": true,
                    "allowed_document_ids_count": self.allowed_document_ids.as_ref().map(|s| s.len()).unwrap_or(0),
                }
            })
            .to_string());
        }

        let svc = self.service();
        let files_result = svc
            .list_paginated(&self.team_id, None, Some(1), Some(sample_limit), None, None)
            .await?;
        let ai_result = svc
            .list_ai_workbench(
                &self.team_id,
                None,
                None,
                None,
                None,
                None,
                Some(1),
                Some(sample_limit),
            )
            .await?;

        let files_sample: Vec<serde_json::Value> = if include_samples {
            files_result
                .items
                .iter()
                .map(Self::as_inventory_item)
                .collect()
        } else {
            Vec::new()
        };
        let ai_sample: Vec<serde_json::Value> = if include_samples {
            ai_result
                .items
                .iter()
                .map(Self::as_inventory_item)
                .collect()
        } else {
            Vec::new()
        };

        // Exact status counts for AI Workbench docs.
        let mut active = 0_u64;
        let mut draft = 0_u64;
        let mut accepted = 0_u64;
        let mut archived = 0_u64;
        let mut superseded = 0_u64;
        let mut ai_total = ai_result.total;
        let mut count_source = "service";

        if let Ok(team_oid) = ObjectId::parse_str(&self.team_id) {
            let coll = self.db.collection::<BsonDocument>("documents");
            let base = doc! {
                "team_id": team_oid,
                "is_deleted": { "$ne": true },
                "origin": "agent",
            };
            if let Ok(total) = coll.count_documents(base.clone(), None).await {
                ai_total = total;
                count_source = "mongo_exact";
            }
            if let Ok(v) = coll
                .count_documents(
                    doc! {
                        "team_id": team_oid,
                        "is_deleted": { "$ne": true },
                        "origin": "agent",
                        "status": "active",
                    },
                    None,
                )
                .await
            {
                active = v;
            }
            if let Ok(v) = coll
                .count_documents(
                    doc! {
                        "team_id": team_oid,
                        "is_deleted": { "$ne": true },
                        "origin": "agent",
                        "status": "draft",
                    },
                    None,
                )
                .await
            {
                draft = v;
            }
            if let Ok(v) = coll
                .count_documents(
                    doc! {
                        "team_id": team_oid,
                        "is_deleted": { "$ne": true },
                        "origin": "agent",
                        "status": "accepted",
                    },
                    None,
                )
                .await
            {
                accepted = v;
            }
            if let Ok(v) = coll
                .count_documents(
                    doc! {
                        "team_id": team_oid,
                        "is_deleted": { "$ne": true },
                        "origin": "agent",
                        "status": "archived",
                    },
                    None,
                )
                .await
            {
                archived = v;
            }
            if let Ok(v) = coll
                .count_documents(
                    doc! {
                        "team_id": team_oid,
                        "is_deleted": { "$ne": true },
                        "origin": "agent",
                        "status": "superseded",
                    },
                    None,
                )
                .await
            {
                superseded = v;
            }
        } else {
            // Fallback from sampled AI list.
            for d in &ai_result.items {
                match d.status {
                    DocumentStatus::Active => active += 1,
                    DocumentStatus::Draft => draft += 1,
                    DocumentStatus::Accepted => accepted += 1,
                    DocumentStatus::Archived => archived += 1,
                    DocumentStatus::Superseded => superseded += 1,
                }
            }
            count_source = "service_sampled";
        }

        Ok(json!({
            "view": "document_inventory",
            "note": "Unified inventory for overview questions. Files and AI Workbench are different views.",
            "guidance": "When answering in Chinese, you MUST output document items by reusing `display_line_zh` verbatim. Preserve every `doc_ref` marker exactly as returned; do not rewrite, split, translate, or restyle it.",
            "files": {
                "total": files_result.total,
                "sample_count": files_sample.len(),
                "sample": files_sample,
            },
            "ai_workbench": {
                "total": ai_total,
                "status_counts": {
                    "active": active,
                    "draft": draft,
                    "accepted": accepted,
                    "archived": archived,
                    "superseded": superseded,
                },
                "status_counts_display": Self::status_count_items(active, draft, accepted, archived, superseded),
                "sample_count": ai_sample.len(),
                "sample": ai_sample,
                "count_source": count_source,
            },
            "policy": {
                "mode": self.write_mode_name(),
                "restrict_to_allowed_documents": false,
            },
            "guidance": [
                "For overview questions, use this inventory result directly.",
                "Files view excludes agent drafts; AI Workbench includes agent drafts."
            ]
        })
        .to_string())
    }

    async fn handle_document_session_policy(&self, _args: &JsonObject) -> Result<String> {
        let mut notes: Vec<&str> = Vec::new();
        match self.write_mode {
            DocumentWriteMode::ReadOnly => {
                notes.push("This session is read-only. Use read/list/search tools only.");
            }
            DocumentWriteMode::DraftOnly => {
                notes.push("Can create new draft documents.");
                notes.push("Can update only agent draft documents related to the current scope or created in this session.");
            }
            DocumentWriteMode::ControlledWrite => {
                notes.push("Write is enabled for documents inside the current scope and their related AI drafts.");
            }
            DocumentWriteMode::FullWrite => {
                notes.push("Full document read/write access according to team permissions.");
            }
        }
        if self.restrict_to_allowed_documents {
            notes.push("File-library list/search stay within bound documents. Related AI drafts are available through AI Workbench and related-ai tools.");
        }

        let mut allowed_document_ids = self
            .allowed_document_ids
            .as_ref()
            .map(|s| s.iter().cloned().collect::<Vec<_>>())
            .unwrap_or_default();
        allowed_document_ids.sort();

        Ok(json!({
            "mode": self.write_mode_name(),
            "scope_mode": self.scope_mode_name(),
            "legacy_document_access_mode": self.legacy_document_access_mode,
            "restrict_to_allowed_documents": self.restrict_to_allowed_documents,
            "can_create_documents": self.can_write_documents(),
            "can_update_documents": self.can_write_documents(),
            "allowed_document_ids_count": allowed_document_ids.len(),
            "allowed_document_ids": allowed_document_ids,
            "notes": notes,
        })
        .to_string())
    }

    async fn handle_read_document(&self, args: &JsonObject) -> Result<String> {
        let doc_id = args
            .get("doc_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("doc_id is required"))?;
        let doc_meta = self
            .get_accessible_doc_metadata(doc_id, "read_document")
            .await?;
        if let Some(existing_path) = self.existing_materialized_path_for_session(doc_id).await {
            return Ok(self.repeated_access_redirect(
                "read_document",
                doc_id,
                &doc_meta.name,
                &existing_path,
            ));
        }
        let materialized = self
            .materialize_document_to_workspace(doc_id, Some(&doc_meta.name), None, None)
            .await?;

        Ok(json!({
            "type": "document_access",
            "doc_id": doc_id,
            "doc_name": doc_meta.name,
            "output_name": materialized.output_name,
            "output_dir": materialized.output_dir,
            "file_path": materialized.file_path,
            "mime_type": materialized.mime_type,
            "file_size": materialized.file_size,
            "access_established": true,
            "content_accessed": false,
            "analysis_complete": false,
            "reason_code": "document_access_established",
            "message": "Document access established and the file was materialized into the workspace. Use developer shell, MCP, or another local tool to read the file content from the workspace path.",
        })
        .to_string())
    }

    async fn handle_export_document(&self, args: &JsonObject) -> Result<String> {
        let doc_id = args
            .get("doc_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("doc_id is required"))?;
        let doc_meta = self
            .get_accessible_doc_metadata(doc_id, "export_document")
            .await?;
        if let Some(existing_path) = self.existing_materialized_path_for_session(doc_id).await {
            return Ok(self.repeated_access_redirect(
                "export_document",
                doc_id,
                &doc_meta.name,
                &existing_path,
            ));
        }

        let output_dir = args
            .get("output_dir")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(ToString::to_string);

        let output_name = args
            .get("output_name")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(ToString::to_string);
        let materialized = self
            .materialize_document_to_workspace(
                doc_id,
                Some(&doc_meta.name),
                output_dir.as_deref(),
                output_name.as_deref(),
            )
            .await?;

        Ok(json!({
            "type": "workspace_export",
            "doc_id": doc_id,
            "doc_name": doc_meta.name,
            "source_name": materialized.source_name,
            "output_name": materialized.output_name,
            "output_dir": materialized.output_dir,
            "file_path": materialized.file_path,
            "mime_type": materialized.mime_type,
            "file_size": materialized.file_size,
            "access_established": true,
            "content_accessed": false,
            "analysis_complete": false,
            "reason_code": "document_exported_to_workspace",
            "message": "Document exported to workspace successfully. Use developer shell, MCP, or another local tool to inspect the file content.",
        })
        .to_string())
    }

    async fn handle_import_document_to_workspace(&self, args: &JsonObject) -> Result<String> {
        if let Some(doc_id) = args.get("doc_id").and_then(|v| v.as_str()) {
            if let Ok(doc_meta) = self
                .get_accessible_doc_metadata(doc_id, "import_document_to_workspace")
                .await
            {
                if let Some(existing_path) =
                    self.existing_materialized_path_for_session(doc_id).await
                {
                    return Ok(self.repeated_access_redirect(
                        "import_document_to_workspace",
                        doc_id,
                        &doc_meta.name,
                        &existing_path,
                    ));
                }
            }
        }
        let mut forwarded = args.clone();
        if !forwarded.contains_key("output_dir") {
            forwarded.insert("output_dir".to_string(), json!("attachments"));
        }
        let result = self.handle_export_document(&forwarded).await?;
        if let Some(output_dir) = forwarded
            .get("output_dir")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            if output_dir.eq_ignore_ascii_case("attachments") {
                if let Ok(value) = serde_json::from_str::<serde_json::Value>(&result) {
                    if let Some(output_name) = value.get("output_name").and_then(|v| v.as_str()) {
                        let _ = self.record_workspace_path(
                            WorkspaceArtifactArea::Attachments,
                            output_name,
                            value
                                .get("mime_type")
                                .and_then(|v| v.as_str())
                                .map(str::to_string),
                        );
                    }
                }
            }
        }
        Ok(result)
    }

    async fn materialize_document_to_workspace(
        &self,
        doc_id: &str,
        source_name: Option<&str>,
        output_dir_override: Option<&str>,
        output_name_override: Option<&str>,
    ) -> Result<WorkspaceMaterializedDocument> {
        let workspace = self.workspace_path.as_deref().ok_or_else(|| {
            anyhow::anyhow!(
                "Cannot materialize document '{}': no workspace available. Use shell tools only after a workspace-backed export path exists.",
                doc_id,
            )
        })?;

        let output_dir_rel =
            Self::normalize_relative_workspace_path(output_dir_override.unwrap_or("documents"))?;
        let target_dir = PathBuf::from(workspace).join(&output_dir_rel);
        tokio::fs::create_dir_all(&target_dir).await?;

        let svc = self.service();
        let (data, _, _) = svc.download(&self.team_id, doc_id).await?;
        let metadata = svc.get_metadata(&self.team_id, doc_id).await?;
        let file_size = data.len() as u64;

        let selected_source_name = source_name.unwrap_or(&metadata.name);
        let output_name = output_name_override
            .map(sanitize_filename)
            .unwrap_or_else(|| sanitize_filename(selected_source_name));
        let output_name = if output_name.is_empty() {
            format!("doc_{}", &doc_id.chars().take(8).collect::<String>())
        } else {
            output_name
        };
        let file_path = target_dir.join(&output_name);

        let needs_write = match tokio::fs::metadata(&file_path).await {
            Ok(meta) => meta.len() != file_size,
            Err(_) => true,
        };
        if needs_write {
            tokio::fs::write(&file_path, &data).await?;
        }

        let file_path_string = file_path.to_string_lossy().to_string();
        self.track_materialized_path(doc_id, &file_path_string);
        self.persist_materialized_path_for_session(doc_id, &file_path_string)
            .await?;

        Ok(WorkspaceMaterializedDocument {
            output_name,
            output_dir: output_dir_rel,
            file_path: file_path_string,
            mime_type: metadata.mime_type,
            file_size,
            source_name: metadata.name,
        })
    }

    fn normalize_relative_workspace_path(path: &str) -> Result<String> {
        let normalized = path.trim().replace('\\', "/");
        if normalized.is_empty() {
            return Err(anyhow::anyhow!("file_path is required"));
        }
        let parsed = Path::new(&normalized);
        if parsed.is_absolute() {
            return Err(anyhow::anyhow!("file_path must be relative to workspace"));
        }
        if !parsed
            .components()
            .all(|c| matches!(c, Component::Normal(_)))
        {
            return Err(anyhow::anyhow!(
                "file_path contains invalid segments (path traversal is not allowed)"
            ));
        }
        Ok(normalized)
    }

    async fn handle_create_document(&self, args: &JsonObject) -> Result<String> {
        if !self.can_write_documents() {
            let reason = self.write_denied_message();
            self.log_write_denied("create_document", reason);
            return Err(anyhow::anyhow!(reason));
        }

        let name = args
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("name is required"))?;
        let content = args
            .get("content")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("content is required"))?;
        let mime_type = args
            .get("mime_type")
            .and_then(|v| v.as_str())
            .unwrap_or("text/plain");
        let folder_path = args
            .get("folder_path")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let mut source_doc_ids: Vec<String> = args
            .get("source_document_ids")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();

        // Merge automatically tracked reads into source lineage (deduplicated)
        {
            let tracked = self.read_doc_ids.lock().unwrap();
            for id in tracked.iter() {
                if !source_doc_ids.contains(id) {
                    source_doc_ids.push(id.clone());
                }
            }
        }

        let lineage_description = args
            .get("lineage_description")
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());

        // Enforce: derived documents MUST have a meaningful lineage description
        if !source_doc_ids.is_empty() && lineage_description.is_none() {
            return Err(anyhow::anyhow!(
                "lineage_description is required when creating a document derived from source documents. \
                 Describe what you did: what content was extracted, translated, summarized, or transformed, and why."
            ));
        }

        let category = args
            .get("category")
            .and_then(|v| v.as_str())
            .and_then(|s| serde_json::from_value(json!(s)).ok())
            .unwrap_or(DocumentCategory::General);

        let svc = self.service();
        let source_context = self
            .resolve_ai_workbench_source_context(Self::infer_ai_workbench_group(
                category,
                name,
                mime_type,
                lineage_description.as_deref(),
            ))
            .await;

        // Build source snapshots for self-contained lineage
        let source_snapshots = if !source_doc_ids.is_empty() {
            svc.build_source_snapshots(&self.team_id, &source_doc_ids)
                .await
                .unwrap_or_default()
        } else {
            vec![]
        };

        let doc = svc
            .create_with_metadata(
                &self.team_id,
                self.agent_id.as_deref().unwrap_or("agent"),
                name,
                content.as_bytes().to_vec(),
                mime_type,
                folder_path,
                DocumentOrigin::Agent,
                DocumentStatus::Draft,
                category,
                source_doc_ids,
                source_snapshots,
                self.session_id.clone(),
                None,
                self.agent_id.clone(),
                source_context.0,
                source_context.1,
                source_context.2,
                source_context.3,
                source_context.4,
                source_context.5,
                source_context.6,
                source_context.7,
                None,
                lineage_description,
            )
            .await?;

        // Clear tracked reads after successful create to prevent accumulation
        // across multiple creates within the same task.
        self.read_doc_ids.lock().unwrap().clear();
        self.materialized_doc_paths.lock().unwrap().clear();
        if let Some(workspace) = self.workspace_path.as_deref() {
            let _ = std::fs::remove_dir_all(
                PathBuf::from(workspace)
                    .join(".agime")
                    .join("document_access"),
            );
        }

        let doc_id = doc.id.map(|id| id.to_hex()).unwrap_or_default();

        Ok(json!({
            "id": doc_id,
            "name": doc.name,
            "status": "draft",
            "origin": "agent",
            "source_document_ids": doc.source_document_ids,
            "lineage_description": doc.lineage_description,
        })
        .to_string())
    }

    async fn handle_create_document_from_file(&self, args: &JsonObject) -> Result<String> {
        if !self.can_write_documents() {
            let reason = self.write_denied_message();
            self.log_write_denied("create_document_from_file", reason);
            return Err(anyhow::anyhow!(reason));
        }

        let file_path = args
            .get("file_path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("file_path is required"))?;
        let relative_path = Self::normalize_relative_workspace_path(file_path)?;

        let workspace = self.workspace_path.as_deref().ok_or_else(|| {
            anyhow::anyhow!(
                "Cannot create document from file: no workspace is available for this session"
            )
        })?;
        let base = PathBuf::from(workspace);
        let full_path = base.join(&relative_path);
        if !full_path.exists() || !full_path.is_file() {
            return Err(anyhow::anyhow!(
                "workspace file does not exist: {}",
                relative_path
            ));
        }

        // Defense-in-depth: ensure canonicalized path still stays under workspace root.
        let canonical_base = std::fs::canonicalize(&base)?;
        let canonical_file = std::fs::canonicalize(&full_path)?;
        if !canonical_file.starts_with(&canonical_base) {
            return Err(anyhow::anyhow!(
                "file_path points outside workspace: {}",
                relative_path
            ));
        }

        let inferred_name = full_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("artifact.bin")
            .to_string();
        let name = args
            .get("name")
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or(inferred_name);

        let mime_type = args
            .get("mime_type")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .or_else(|| {
                mime_guess::from_path(&full_path)
                    .first_raw()
                    .map(|s| s.to_string())
            })
            .unwrap_or_else(|| "application/octet-stream".to_string());

        let folder_path = args
            .get("folder_path")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let mut source_doc_ids: Vec<String> = args
            .get("source_document_ids")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();

        {
            let tracked = self.read_doc_ids.lock().unwrap();
            for id in tracked.iter() {
                if !source_doc_ids.contains(id) {
                    source_doc_ids.push(id.clone());
                }
            }
        }

        let lineage_description = args
            .get("lineage_description")
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());

        if !source_doc_ids.is_empty() && lineage_description.is_none() {
            return Err(anyhow::anyhow!(
                "lineage_description is required when creating a document derived from source documents. \
                 Describe what you did: what content was extracted, translated, summarized, or transformed, and why."
            ));
        }

        let category = args
            .get("category")
            .and_then(|v| v.as_str())
            .and_then(|s| serde_json::from_value(json!(s)).ok())
            .unwrap_or(DocumentCategory::General);

        let content = tokio::fs::read(&full_path).await?;
        let svc = self.service();
        let source_context = self
            .resolve_ai_workbench_source_context(Self::infer_ai_workbench_group(
                category,
                &name,
                &mime_type,
                lineage_description.as_deref(),
            ))
            .await;
        let source_snapshots = if !source_doc_ids.is_empty() {
            svc.build_source_snapshots(&self.team_id, &source_doc_ids)
                .await
                .unwrap_or_default()
        } else {
            vec![]
        };

        let doc = svc
            .create_with_metadata(
                &self.team_id,
                self.agent_id.as_deref().unwrap_or("agent"),
                &name,
                content,
                &mime_type,
                folder_path,
                DocumentOrigin::Agent,
                DocumentStatus::Draft,
                category,
                source_doc_ids,
                source_snapshots,
                self.session_id.clone(),
                None,
                self.agent_id.clone(),
                source_context.0,
                source_context.1,
                source_context.2,
                source_context.3,
                source_context.4,
                source_context.5,
                source_context.6,
                source_context.7,
                None,
                lineage_description,
            )
            .await?;

        self.read_doc_ids.lock().unwrap().clear();
        self.materialized_doc_paths.lock().unwrap().clear();
        if let Some(workspace) = self.workspace_path.as_deref() {
            let _ = std::fs::remove_dir_all(
                PathBuf::from(workspace)
                    .join(".agime")
                    .join("document_access"),
            );
        }

        let doc_id = doc.id.map(|id| id.to_hex()).unwrap_or_default();
        Ok(json!({
            "id": doc_id,
            "name": doc.name,
            "status": "draft",
            "status_display": Self::status_display(DocumentStatus::Draft),
            "status_label_zh": Self::status_label_zh(DocumentStatus::Draft),
            "status_label_en": Self::status_label_en(DocumentStatus::Draft),
            "doc_ref": format!(
                "[[doc:{}|{}|{}|{}]]",
                doc_id,
                doc.name,
                Self::status_key(DocumentStatus::Draft),
                Self::document_class(&doc.name, &doc.mime_type)
            ),
            "document_class": Self::document_class(&doc.name, &doc.mime_type),
            "extension": Self::document_extension(&doc.name),
            "origin": "agent",
            "file_path": relative_path,
            "file_size": doc.file_size,
            "mime_type": doc.mime_type,
            "source_document_ids": doc.source_document_ids,
            "lineage_description": doc.lineage_description,
        })
        .to_string())
    }

    async fn handle_publish_workspace_artifact(&self, args: &JsonObject) -> Result<String> {
        let mut forwarded = args.clone();
        if !forwarded.contains_key("folder_path") {
            if let Some(folder_path) = self.default_publish_folder_path().await {
                forwarded.insert("folder_path".to_string(), json!(folder_path));
            }
        }
        let file_path = forwarded
            .get("file_path")
            .and_then(|value| value.as_str())
            .ok_or_else(|| anyhow::anyhow!("file_path is required"))?
            .to_string();
        let result = self.handle_create_document_from_file(&forwarded).await?;
        let _ = self.record_workspace_path(
            WorkspaceArtifactArea::Artifacts,
            &file_path,
            forwarded
                .get("mime_type")
                .and_then(|value| value.as_str())
                .map(str::to_string),
        );
        Ok(result)
    }

    async fn default_publish_folder_path(&self) -> Option<String> {
        let session_id = self.session_id.as_deref()?;
        let coll = self.db.collection::<BsonDocument>("agent_sessions");
        let session = coll
            .find_one(doc! { "session_id": session_id }, None)
            .await
            .ok()
            .flatten()?;

        let session_source = session
            .get_str("session_source")
            .ok()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("chat");
        if session_source.eq_ignore_ascii_case("channel_runtime")
            || session_source.eq_ignore_ascii_case("channel_conversation")
        {
            let channel_id = session
                .get_str("source_channel_id")
                .ok()
                .map(str::trim)
                .filter(|value| !value.is_empty())?;
            let service = ChatChannelService::new(self.db.clone());
            return service
                .get_channel(channel_id)
                .await
                .ok()
                .flatten()
                .and_then(|channel| channel.document_folder_path)
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty());
        }
        None
    }

    fn record_workspace_path(
        &self,
        area: WorkspaceArtifactArea,
        relative_path: &str,
        content_type: Option<String>,
    ) -> Result<()> {
        let workspace = self.workspace_path.as_deref().ok_or_else(|| {
            anyhow::anyhow!("Cannot record workspace artifact: no workspace is available")
        })?;
        let normalized = Self::normalize_relative_workspace_path(relative_path)?;
        let manifest_path = PathBuf::from(workspace).join("workspace.json");
        let store = LocalFsWorkspaceStore;
        let manifest_path_value = if normalized == area.as_rel_dir()
            || normalized.starts_with(&format!("{}/", area.as_rel_dir()))
            || normalized.starts_with("runs/")
        {
            normalized
        } else {
            format!("{}/{}", area.as_rel_dir(), normalized)
        };
        let _ = store.append_artifact_record(
            &manifest_path,
            WorkspaceArtifactRecord {
                path: manifest_path_value,
                content_type,
            },
        )?;
        Ok(())
    }

    async fn handle_search_documents(&self, args: &JsonObject) -> Result<String> {
        let query = args
            .get("query")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| anyhow::anyhow!("query is required"))?;
        let mime_type = args
            .get("mime_type")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty());
        let folder_path = args
            .get("folder_path")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty());

        if self.restrict_to_allowed_documents {
            let query_lower = query.trim().to_ascii_lowercase();
            let mut docs = self.list_allowed_documents().await?;
            docs.retain(|d| {
                let folder_ok = folder_path.map(|p| d.folder_path == p).unwrap_or(true);
                let mime_ok = Self::matches_mime_filter(&d.mime_type, mime_type);
                folder_ok && mime_ok && Self::doc_matches_query(d, &query_lower)
            });
            let total = docs.len() as u64;
            docs.truncate(20);
            let items: Vec<serde_json::Value> =
                docs.iter().map(Self::as_document_list_item).collect();
            return Ok(json!({
                "view": "restricted_bound_documents",
                "note": "Restricted session: only bound documents are searchable.",
                "guidance": "When answering in Chinese, you MUST output document items by reusing `display_line_zh` verbatim. Preserve every `doc_ref` marker exactly as returned; do not rewrite, split, translate, or restyle it.",
                "documents": items,
                "total": total,
                "page": 1,
            })
            .to_string());
        }

        let svc = self.service();
        let result = svc
            .search(
                &self.team_id,
                query,
                Some(1),
                Some(20),
                mime_type,
                folder_path,
                None,
            )
            .await?;

        let items: Vec<serde_json::Value> = result
            .items
            .iter()
            .map(Self::as_document_list_item)
            .collect();

        Ok(json!({
            "view": "files",
            "note": "This is file-library search. For AI Workbench documents, call list_ai_workbench_documents.",
            "guidance": "When answering in Chinese, you MUST output document items by reusing `display_line_zh` verbatim. Preserve every `doc_ref` marker exactly as returned; do not rewrite, split, translate, or restyle it.",
            "documents": items,
            "total": result.total
        }).to_string())
    }

    async fn handle_list_documents(&self, args: &JsonObject) -> Result<String> {
        let folder_path = args
            .get("folder_path")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty());
        let page = args.get("page").and_then(|v| v.as_u64());
        let limit = args.get("limit").and_then(|v| v.as_u64());

        if self.restrict_to_allowed_documents {
            let mut docs = self.list_allowed_documents().await?;
            docs.retain(|d| folder_path.map(|p| d.folder_path == p).unwrap_or(true));
            let total = docs.len() as u64;
            let (start, end, page, _) = Self::apply_page_limit(page, limit, docs.len());
            let docs = if start >= docs.len() {
                Vec::new()
            } else {
                docs[start..end].to_vec()
            };
            let items: Vec<serde_json::Value> =
                docs.iter().map(Self::as_document_list_item).collect();
            return Ok(json!({
                "view": "restricted_bound_documents",
                "note": "Restricted session: this list only includes bound documents, not the full file library.",
                "guidance": "When answering in Chinese, you MUST output document items by reusing `display_line_zh` verbatim. Preserve every `doc_ref` marker exactly as returned; do not rewrite, split, translate, or restyle it.",
                "documents": items,
                "total": total,
                "page": page,
            })
            .to_string());
        }

        let svc = self.service();
        let result = svc
            .list_paginated(&self.team_id, folder_path, page, limit, None, None)
            .await?;

        let items: Vec<serde_json::Value> = result
            .items
            .iter()
            .map(Self::as_document_list_item)
            .collect();

        Ok(json!({
            "view": "files",
            "note": "This is file-library listing. It may exclude draft agent documents. Use list_ai_workbench_documents for AI Workbench.",
            "guidance": "When answering in Chinese, you MUST output document items by reusing `display_line_zh` verbatim. Preserve every `doc_ref` marker exactly as returned; do not rewrite, split, translate, or restyle it.",
            "documents": items,
            "total": result.total,
            "page": result.page,
        })
        .to_string())
    }

    async fn handle_list_ai_workbench_documents(&self, args: &JsonObject) -> Result<String> {
        let session_id = args
            .get("session_id")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty());
        let page = args.get("page").and_then(|v| v.as_u64());
        let limit = args.get("limit").and_then(|v| v.as_u64());

        if self.restrict_to_allowed_documents {
            let allowed = self.list_allowed_documents().await?;
            let related = self
                .list_related_ai_documents_for_scope(None, Some(500))
                .await?;
            let channel_ai = self
                .list_current_channel_ai_documents(None, Some(500))
                .await?;
            let channel_folder_path = self
                .get_current_channel_context()
                .await?
                .map(|(_, folder_path)| folder_path);
            let mut docs: Vec<DocumentSummary> = allowed
                .into_iter()
                .filter(|d| {
                    d.origin == DocumentOrigin::Agent
                        && !Self::is_promoted_ai_output(d, channel_folder_path.as_deref())
                })
                .collect();
            for doc in related.items {
                if !Self::is_promoted_ai_output(&doc, channel_folder_path.as_deref())
                    && !docs.iter().any(|existing| existing.id == doc.id)
                {
                    docs.push(doc);
                }
            }
            for doc in channel_ai.items {
                if !docs.iter().any(|existing| existing.id == doc.id) {
                    docs.push(doc);
                }
            }
            if let Some(sid) = session_id {
                docs.retain(|d| d.source_session_id.as_deref() == Some(sid));
            }
            docs.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
            let total = docs.len() as u64;
            let (start, end, page, _) = Self::apply_page_limit(page, limit, docs.len());
            let docs = if start >= docs.len() {
                Vec::new()
            } else {
                docs[start..end].to_vec()
            };
            let items: Vec<serde_json::Value> = docs
                .iter()
                .map(|d| {
                    let mut item = Self::as_document_list_item(d);
                    if let Some(obj) = item.as_object_mut() {
                        obj.insert("category".to_string(), json!(d.category));
                        obj.insert(
                            "created_by_agent_id".to_string(),
                            json!(d.created_by_agent_id),
                        );
                    }
                    item
                })
                .collect();

            return Ok(json!({
                "view": "ai_workbench",
                "note": "AI Workbench in restricted session (bound documents plus related AI drafts).",
                "guidance": "When answering in Chinese, output document items by reusing `display_line_zh` verbatim. Preserve every `doc_ref` marker exactly as returned; do not rewrite, split, translate, or restyle it.",
                "documents": items,
                "total": total,
                "page": page,
            })
            .to_string());
        }

        let svc = self.service();
        let result = svc
            .list_ai_workbench(
                &self.team_id,
                session_id,
                None,
                None,
                None,
                None,
                page,
                limit,
            )
            .await?;

        let items: Vec<serde_json::Value> = result
            .items
            .iter()
            .filter(|d| self.is_doc_allowed(&d.id))
            .map(|d| {
                let mut item = Self::as_document_list_item(d);
                if let Some(obj) = item.as_object_mut() {
                    obj.insert("category".to_string(), json!(d.category));
                    obj.insert(
                        "created_by_agent_id".to_string(),
                        json!(d.created_by_agent_id),
                    );
                }
                item
            })
            .collect();
        let total = if self.restrict_to_allowed_documents {
            items.len() as u64
        } else {
            result.total
        };

        Ok(json!({
            "view": "ai_workbench",
            "note": "AI Workbench is a source/status-based view (not a folder path).",
            "guidance": "When answering in Chinese, output document items by reusing `display_line_zh` verbatim. Preserve every `doc_ref` marker exactly as returned; do not rewrite, split, translate, or restyle it.",
            "documents": items,
            "total": total,
            "page": result.page,
        })
        .to_string())
    }

    async fn handle_list_related_ai_documents(&self, args: &JsonObject) -> Result<String> {
        let source_doc_id = args
            .get("source_doc_id")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| anyhow::anyhow!("source_doc_id is required"))?;
        let page = args.get("page").and_then(|v| v.as_u64());
        let limit = args.get("limit").and_then(|v| v.as_u64());

        self.get_accessible_doc_metadata(source_doc_id, "list_related_ai_documents")
            .await?;

        let svc = self.service();
        let result = svc
            .list_related_ai_documents(&self.team_id, &[source_doc_id.to_string()], page, limit)
            .await?;

        let items: Vec<serde_json::Value> = result
            .items
            .iter()
            .filter(|d| self.can_access_doc_summary(d))
            .map(|d| {
                let mut item = Self::as_document_list_item(d);
                if let Some(obj) = item.as_object_mut() {
                    obj.insert("category".to_string(), json!(d.category));
                    obj.insert(
                        "source_document_ids".to_string(),
                        json!(d.source_document_ids),
                    );
                    obj.insert(
                        "created_by_agent_id".to_string(),
                        json!(d.created_by_agent_id),
                    );
                    obj.insert(
                        "lineage_description".to_string(),
                        json!(d.lineage_description),
                    );
                    obj.insert("updated_at".to_string(), json!(d.updated_at));
                }
                item
            })
            .collect();
        let total = if self.restrict_to_allowed_documents {
            items.len() as u64
        } else {
            result.total
        };

        Ok(json!({
            "view": "related_ai_documents",
            "source_doc_id": source_doc_id,
            "note": "AI documents related to the selected source document.",
            "guidance": "When answering in Chinese, output document items by reusing `display_line_zh` verbatim. Preserve every `doc_ref` marker exactly as returned; do not rewrite, split, translate, or restyle it.",
            "documents": items,
            "total": total,
            "page": result.page,
        })
        .to_string())
    }

    async fn handle_update_document(&self, args: &JsonObject) -> Result<String> {
        if !self.can_write_documents() {
            let reason = self.write_denied_message();
            self.log_write_denied("update_document", reason);
            return Err(anyhow::anyhow!(reason));
        }

        let doc_id = args
            .get("doc_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("doc_id is required"))?;
        let content = args
            .get("content")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("content is required"))?;
        let message = args
            .get("message")
            .and_then(|v| v.as_str())
            .unwrap_or("Updated by agent");

        let svc = self.service();
        if matches!(self.write_mode, DocumentWriteMode::DraftOnly) {
            let meta = self
                .get_accessible_doc_metadata(doc_id, "update_document")
                .await?;
            if meta.origin != DocumentOrigin::Agent || meta.status != DocumentStatus::Draft {
                self.log_write_denied(
                    "update_document",
                    "co_edit_draft mode only allows agent draft documents",
                );
                return Err(anyhow::anyhow!(
                    "Document write denied: co_edit_draft mode only allows updating agent draft documents"
                ));
            }
        } else {
            self.get_accessible_doc_metadata(doc_id, "update_document")
                .await?;
        }

        let user_id = self.agent_id.as_deref().unwrap_or("agent");
        let version_service = DocumentVersionService::new((*self.db).clone());
        if let Ok((old_data, _, _)) = svc.download(&self.team_id, doc_id).await {
            let _ = version_service
                .create_version(doc_id, &self.team_id, user_id, user_id, old_data, message)
                .await;
        }
        let doc = svc
            .update_content(
                &self.team_id,
                doc_id,
                user_id,
                content.as_bytes().to_vec(),
                message,
            )
            .await?;

        Ok(json!({
            "doc_id": doc_id,
            "name": doc.name,
            "file_size": doc.file_size,
            "updated": true,
        })
        .to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_document_access_result_for_model_points_to_workspace_path() {
        let rendered = render_document_tool_result_for_model(&json!({
            "type": "document_access",
            "doc_name": "demo.txt",
            "mime_type": "text/plain",
            "file_path": "workspace/documents/demo.txt"
        }));

        assert!(rendered.contains("demo.txt"));
        assert!(rendered.contains("workspace/documents/demo.txt"));
        assert!(rendered.contains("Use developer shell"));
        assert!(!rendered.contains("Document text content"));
    }

    #[test]
    fn render_workspace_export_result_for_model_points_to_workspace_path() {
        let rendered = render_document_tool_result_for_model(&json!({
            "type": "workspace_export",
            "doc_name": "report.pdf",
            "mime_type": "application/pdf",
            "file_path": "workspace/documents/report.pdf"
        }));

        assert!(rendered.contains("report.pdf"));
        assert!(rendered.contains("workspace/documents/report.pdf"));
        assert!(rendered.contains("Use developer shell"));
    }

    #[test]
    fn render_document_access_redirect_requires_local_tool() {
        let rendered = render_document_tool_result_for_model(&json!({
            "type": "document_access_redirect",
            "doc_name": "demo.txt",
            "file_path": "workspace/documents/demo.txt"
        }));

        assert!(rendered.contains("already established"));
        assert!(rendered.contains("workspace/documents/demo.txt"));
        assert!(rendered.contains("Do not call another document access tool"));
    }

    #[test]
    fn build_document_tool_call_result_preserves_structured_redirect_payload() {
        let result = build_document_tool_call_result(
            json!({
                "type": "document_access_redirect",
                "doc_name": "demo.txt",
                "file_path": "workspace/documents/demo.txt",
                "reason_code": "document_access_already_established"
            })
            .to_string(),
            false,
        );

        assert_eq!(result.is_error, Some(false));
        assert!(result.structured_content.is_some());
        let rendered = result
            .content
            .iter()
            .find_map(|content| content.as_text().map(|text| text.text.clone()))
            .unwrap_or_default();
        assert!(rendered.contains("Do not call another document access tool"));
    }
}
