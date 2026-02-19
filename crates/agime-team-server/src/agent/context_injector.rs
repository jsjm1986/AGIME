//! Document Context Injector
//!
//! Exports attached documents from MongoDB to the workspace filesystem,
//! then injects file path information into the agent's system prompt.
//! The agent uses its own capabilities (shell, python, etc.) to read any format.

use agime_team::db::MongoDb;
use agime_team::services::mongo::DocumentService;
use serde::Serialize;
use std::path::{Path, PathBuf};

/// Exported document info for prompt injection
#[derive(Debug, Clone, Serialize)]
pub struct ExportedDocument {
    pub id: String,
    pub name: String,
    pub mime_type: String,
    pub file_size: i64,
    pub file_path: String,
}

pub struct DocumentContextInjector {
    db: MongoDb,
}

impl DocumentContextInjector {
    pub fn new(db: MongoDb) -> Self {
        Self { db }
    }

    /// Export documents to workspace filesystem and return path info.
    /// Creates `{workspace_path}/documents/` directory and writes each file there.
    pub async fn export_to_workspace(
        &self,
        team_id: &str,
        document_ids: &[String],
        workspace_path: &str,
    ) -> Vec<ExportedDocument> {
        let svc = DocumentService::new(self.db.clone());
        let docs_dir = PathBuf::from(workspace_path).join("documents");
        if let Err(e) = std::fs::create_dir_all(&docs_dir) {
            tracing::error!(
                "Failed to create documents dir {}: {}",
                docs_dir.display(),
                e
            );
            return Vec::new();
        }

        let mut exported = Vec::new();
        for doc_id in document_ids {
            match self.export_one(&svc, team_id, doc_id, &docs_dir).await {
                Ok(doc) => exported.push(doc),
                Err(e) => {
                    tracing::warn!("Failed to export document {}: {}", doc_id, e);
                }
            }
        }

        exported
    }

    async fn export_one(
        &self,
        svc: &DocumentService,
        team_id: &str,
        doc_id: &str,
        docs_dir: &Path,
    ) -> anyhow::Result<ExportedDocument> {
        let (data, name, mime_type) = svc.download(team_id, doc_id).await?;
        let file_size = data.len() as i64;

        // Sanitize filename to prevent path traversal
        let safe_name = sanitize_filename(&name);
        let file_path = docs_dir.join(&safe_name);

        // Skip if already exported (same size = likely same content)
        if file_path.exists() {
            if let Ok(meta) = std::fs::metadata(&file_path) {
                if meta.len() == file_size as u64 {
                    return Ok(ExportedDocument {
                        id: doc_id.to_string(),
                        name,
                        mime_type,
                        file_size,
                        file_path: file_path.to_string_lossy().to_string(),
                    });
                }
            }
        }

        std::fs::write(&file_path, &data)?;

        Ok(ExportedDocument {
            id: doc_id.to_string(),
            name,
            mime_type,
            file_size,
            file_path: file_path.to_string_lossy().to_string(),
        })
    }

    /// Format exported documents as a prompt section with file paths.
    pub fn format_as_prompt_section(docs: &[ExportedDocument]) -> String {
        if docs.is_empty() {
            return String::new();
        }

        let mut out = String::from(
            "\n<attached-documents>\n\
             The following documents have been saved to your workspace. \
             Use your tools (shell, python, etc.) to read and process them.\n",
        );
        for doc in docs {
            out.push_str(&format!(
                "- {} ({}, {} bytes)\n  Path: {}\n",
                doc.name, doc.mime_type, doc.file_size, doc.file_path,
            ));
        }
        out.push_str("</attached-documents>\n");
        out
    }
}

/// Sanitize a filename: strip path separators, replace unsafe chars.
pub(crate) fn sanitize_filename(name: &str) -> String {
    Path::new(name)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "document".to_string())
}
