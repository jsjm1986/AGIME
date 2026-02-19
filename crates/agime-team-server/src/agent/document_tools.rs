//! Document Tools — platform extension for Agent to read/create/search/list documents
//!
//! Implements McpClientTrait so it can be loaded by PlatformExtensionRunner.

use agime::agents::mcp_client::McpClientTrait;
use agime_team::db::MongoDb;
use agime_team::models::mongo::{DocumentCategory, DocumentOrigin, DocumentStatus};
use agime_team::services::mongo::DocumentService;
use anyhow::Result;
use rmcp::model::*;
use rmcp::ServiceError;
use serde_json::json;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use super::context_injector::sanitize_filename;

/// Provider of document tools for agents
pub struct DocumentToolsProvider {
    db: Arc<MongoDb>,
    team_id: String,
    session_id: Option<String>,
    mission_id: Option<String>,
    agent_id: Option<String>,
    workspace_path: Option<String>,
    /// Document IDs read during this session (for automatic lineage tracking)
    read_doc_ids: Mutex<Vec<String>>,
}

impl DocumentToolsProvider {
    pub fn new(
        db: Arc<MongoDb>,
        team_id: String,
        session_id: Option<String>,
        mission_id: Option<String>,
        agent_id: Option<String>,
        workspace_path: Option<String>,
    ) -> Self {
        Self {
            db,
            team_id,
            session_id,
            mission_id,
            agent_id,
            workspace_path,
            read_doc_ids: Mutex::new(Vec::new()),
        }
    }

    fn service(&self) -> DocumentService {
        DocumentService::new((*self.db).clone())
    }

    /// Record a document ID as read for automatic lineage tracking.
    fn track_read(&self, doc_id: &str) {
        let mut tracked = self.read_doc_ids.lock().unwrap();
        if !tracked.iter().any(|id| id == doc_id) {
            tracked.push(doc_id.to_string());
        }
    }

    fn tool_definitions() -> Vec<Tool> {
        vec![
            Tool {
                name: "read_document".into(),
                title: None,
                description: Some("Read document content. Supports chunked reading for large documents.".into()),
                input_schema: serde_json::from_value(json!({
                    "type": "object",
                    "properties": {
                        "doc_id": { "type": "string", "description": "Document ID" },
                        "offset": { "type": "integer", "description": "Byte offset for chunked reading" },
                        "limit": { "type": "integer", "description": "Max bytes to read" }
                    },
                    "required": ["doc_id"]
                })).unwrap_or_default(),
                output_schema: None,
                annotations: None,
                icons: None,
                meta: None,
            },
            Tool {
                name: "create_document".into(),
                title: None,
                description: Some("Create a new document in the team's document store. When creating a document derived from other documents (i.e. you have read source documents before), you MUST provide lineage_description explaining what you did and why.".into()),
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
                icons: None,
                meta: None,
            },
            Tool {
                name: "search_documents".into(),
                title: None,
                description: Some("Search documents by name, description or tags.".into()),
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
                icons: None,
                meta: None,
            },
            Tool {
                name: "list_documents".into(),
                title: None,
                description: Some("List documents in the team's document store.".into()),
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
            tools: Self::tool_definitions(),
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
            "read_document" => self.handle_read_document(&args).await,
            "create_document" => self.handle_create_document(&args).await,
            "search_documents" => self.handle_search_documents(&args).await,
            "list_documents" => self.handle_list_documents(&args).await,
            "update_document" => self.handle_update_document(&args).await,
            _ => Err(anyhow::anyhow!("Unknown tool: {}", name)),
        };

        match result {
            Ok(text) => Ok(CallToolResult::success(vec![Content::text(text)])),
            Err(e) => Ok(CallToolResult::error(vec![Content::text(e.to_string())])),
        }
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

// ── Tool handler implementations ──

impl DocumentToolsProvider {
    async fn handle_read_document(&self, args: &JsonObject) -> Result<String> {
        let doc_id = args
            .get("doc_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("doc_id is required"))?;

        let svc = self.service();
        let doc_meta = svc.get_metadata(&self.team_id, doc_id).await?;
        let mime_type = &doc_meta.mime_type;

        // Binary formats: export to workspace filesystem and return path
        if is_binary_format(mime_type) {
            return self
                .handle_read_binary(&svc, doc_id, &doc_meta.name, mime_type)
                .await;
        }

        // Text formats: return content directly
        let offset = args
            .get("offset")
            .and_then(|v| v.as_u64())
            .map(|v| v as usize);
        let limit = args
            .get("limit")
            .and_then(|v| v.as_u64())
            .map(|v| v as usize);

        let (text, mime_type, total_size) = svc
            .get_text_content_chunked(&self.team_id, doc_id, offset, limit)
            .await?;

        self.track_read(doc_id);

        Ok(json!({
            "text": text,
            "mime_type": mime_type,
            "total_size": total_size,
            "offset": offset.unwrap_or(0),
            "length": text.len(),
        })
        .to_string())
    }

    /// Export a binary document to workspace filesystem and return the file path.
    async fn handle_read_binary(
        &self,
        svc: &DocumentService,
        doc_id: &str,
        name: &str,
        mime_type: &str,
    ) -> Result<String> {
        let workspace = self.workspace_path.as_deref().ok_or_else(|| {
            anyhow::anyhow!(
                "Cannot read binary document '{}' ({}): no workspace available. \
                 Use shell tools to process this file format.",
                name,
                mime_type,
            )
        })?;

        let docs_dir = PathBuf::from(workspace).join("documents");
        tokio::fs::create_dir_all(&docs_dir).await?;

        let (data, _, _) = svc.download(&self.team_id, doc_id).await?;
        let file_size = data.len() as u64;

        let safe_name = sanitize_filename(name);
        let file_path = docs_dir.join(&safe_name);

        // Skip write if already exported with same size
        let needs_write = match tokio::fs::metadata(&file_path).await {
            Ok(meta) => meta.len() != file_size,
            Err(_) => true,
        };
        if needs_write {
            tokio::fs::write(&file_path, &data).await?;
        }

        self.track_read(doc_id);

        let path_str = file_path.to_string_lossy().to_string();
        Ok(json!({
            "type": "binary_export",
            "file_path": path_str,
            "name": name,
            "mime_type": mime_type,
            "file_size": file_size,
            "message": format!(
                "Binary file exported to workspace. Use shell/python to read: {}",
                path_str,
            ),
        })
        .to_string())
    }

    async fn handle_create_document(&self, args: &JsonObject) -> Result<String> {
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
                self.mission_id.clone(),
                self.agent_id.clone(),
                None,
                lineage_description,
            )
            .await?;

        // Clear tracked reads after successful create to prevent accumulation
        // across multiple creates within the same task.
        self.read_doc_ids.lock().unwrap().clear();

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

    async fn handle_search_documents(&self, args: &JsonObject) -> Result<String> {
        let query = args
            .get("query")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("query is required"))?;
        let mime_type = args.get("mime_type").and_then(|v| v.as_str());
        let folder_path = args.get("folder_path").and_then(|v| v.as_str());

        let svc = self.service();
        let result = svc
            .search(
                &self.team_id,
                query,
                Some(1),
                Some(20),
                mime_type,
                folder_path,
            )
            .await?;

        let items: Vec<serde_json::Value> = result
            .items
            .iter()
            .map(|d| {
                json!({
                    "id": d.id,
                    "name": d.name,
                    "mime_type": d.mime_type,
                    "file_size": d.file_size,
                    "folder_path": d.folder_path,
                })
            })
            .collect();

        Ok(json!({ "documents": items, "total": result.total }).to_string())
    }

    async fn handle_list_documents(&self, args: &JsonObject) -> Result<String> {
        let folder_path = args.get("folder_path").and_then(|v| v.as_str());
        let page = args.get("page").and_then(|v| v.as_u64());
        let limit = args.get("limit").and_then(|v| v.as_u64());

        let svc = self.service();
        let result = svc
            .list_paginated(&self.team_id, folder_path, page, limit)
            .await?;

        let items: Vec<serde_json::Value> = result
            .items
            .iter()
            .map(|d| {
                json!({
                    "id": d.id,
                    "name": d.name,
                    "mime_type": d.mime_type,
                    "file_size": d.file_size,
                    "folder_path": d.folder_path,
                })
            })
            .collect();

        Ok(json!({
            "documents": items,
            "total": result.total,
            "page": result.page,
        })
        .to_string())
    }

    async fn handle_update_document(&self, args: &JsonObject) -> Result<String> {
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
        let user_id = self.agent_id.as_deref().unwrap_or("agent");
        let doc = svc
            .update_content(&self.team_id, doc_id, user_id, content.as_bytes().to_vec(), message)
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

/// Check if a MIME type represents a binary format that cannot be meaningfully
/// returned as text content.
fn is_binary_format(mime_type: &str) -> bool {
    const BINARY_PREFIXES: &[&str] = &[
        "application/pdf",
        "application/msword",
        "application/vnd.openxmlformats",
        "application/vnd.ms-excel",
        "application/vnd.ms-powerpoint",
        "application/zip",
        "application/x-rar",
        "application/x-7z",
        "application/octet-stream",
        "image/",
        "audio/",
        "video/",
    ];
    BINARY_PREFIXES.iter().any(|p| mime_type.starts_with(p))
}
