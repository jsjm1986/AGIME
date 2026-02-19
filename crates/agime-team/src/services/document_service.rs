//! Document service - simplified for agent-based processing
//!
//! Design: Store raw files only, let Agent process them on demand

use crate::models::{
    AgentFileResponse, CreateFolderRequest, DocumentSearchQuery, DocumentSearchResult,
    DocumentSummary, FolderTreeNode, TeamDocument, TeamFolder,
};
use anyhow::{anyhow, Result};
use chrono::Utc;
use sqlx::SqlitePool;
use std::path::{Path, PathBuf};
use tokio::fs;
use uuid::Uuid;

/// Document service for managing team files
pub struct DocumentService {
    storage_path: PathBuf,
}

impl DocumentService {
    pub fn new(storage_path: impl AsRef<Path>) -> Self {
        Self {
            storage_path: storage_path.as_ref().to_path_buf(),
        }
    }

    fn team_storage_path(&self, team_id: &str) -> PathBuf {
        self.storage_path
            .join("teams")
            .join(team_id)
            .join("documents")
    }

    // ========================================
    // Folder Operations
    // ========================================

    pub async fn create_folder(
        &self,
        pool: &SqlitePool,
        team_id: &str,
        user_id: &str,
        req: CreateFolderRequest,
    ) -> Result<TeamFolder> {
        let id = Uuid::new_v4().to_string();
        let now = Utc::now();

        let path = if let Some(ref parent_id) = req.parent_id {
            let parent = self
                .get_folder(pool, parent_id)
                .await?
                .ok_or_else(|| anyhow!("Parent folder not found"))?;
            format!("{}/{}", parent.path, req.name)
        } else {
            format!("/{}", req.name)
        };

        sqlx::query(
            "INSERT INTO team_folders (id, team_id, parent_id, name, path, description, created_by, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)"
        )
        .bind(&id)
        .bind(team_id)
        .bind(&req.parent_id)
        .bind(&req.name)
        .bind(&path)
        .bind(&req.description)
        .bind(user_id)
        .bind(now)
        .bind(now)
        .execute(pool)
        .await?;

        Ok(TeamFolder {
            id,
            team_id: team_id.to_string(),
            parent_id: req.parent_id,
            name: req.name,
            path,
            description: req.description,
            created_by: user_id.to_string(),
            created_at: now,
            updated_at: now,
            is_deleted: false,
        })
    }

    pub async fn get_folder(
        &self,
        pool: &SqlitePool,
        folder_id: &str,
    ) -> Result<Option<TeamFolder>> {
        let row = sqlx::query_as::<_, (String, String, Option<String>, String, String, Option<String>, String, String, String)>(
            "SELECT id, team_id, parent_id, name, path, description, created_by, created_at, updated_at
             FROM team_folders WHERE id = ? AND is_deleted = 0"
        )
        .bind(folder_id)
        .fetch_optional(pool)
        .await?;

        Ok(row.map(|r| TeamFolder {
            id: r.0,
            team_id: r.1,
            parent_id: r.2,
            name: r.3,
            path: r.4,
            description: r.5,
            created_by: r.6,
            created_at: r.7.parse().unwrap_or_default(),
            updated_at: r.8.parse().unwrap_or_default(),
            is_deleted: false,
        }))
    }

    pub async fn get_folder_tree(
        &self,
        pool: &SqlitePool,
        team_id: &str,
    ) -> Result<Vec<FolderTreeNode>> {
        let rows = sqlx::query_as::<_, (String, Option<String>, String, String, Option<String>)>(
            "SELECT id, parent_id, name, path, description FROM team_folders
             WHERE team_id = ? AND is_deleted = 0 ORDER BY path",
        )
        .bind(team_id)
        .fetch_all(pool)
        .await?;

        let nodes: Vec<FolderTreeNode> = rows
            .iter()
            .map(|r| FolderTreeNode {
                id: r.0.clone(),
                name: r.2.clone(),
                path: r.3.clone(),
                description: r.4.clone(),
                children: vec![],
                document_count: 0,
            })
            .collect();

        // Build tree (simplified - just return flat list for now)
        Ok(nodes)
    }

    pub async fn delete_folder(&self, pool: &SqlitePool, folder_id: &str) -> Result<()> {
        sqlx::query("UPDATE team_folders SET is_deleted = 1, updated_at = ? WHERE id = ?")
            .bind(Utc::now())
            .bind(folder_id)
            .execute(pool)
            .await?;
        Ok(())
    }

    // ========================================
    // Document Operations
    // ========================================

    pub async fn upload_document(
        &self,
        pool: &SqlitePool,
        team_id: &str,
        user_id: &str,
        file_name: &str,
        file_data: Vec<u8>,
        mime_type: &str,
        folder_id: Option<String>,
        description: Option<String>,
    ) -> Result<TeamDocument> {
        let id = Uuid::new_v4().to_string();
        let now = Utc::now();
        let file_size = file_data.len() as i64;

        // Always store on filesystem
        let storage_dir = self.team_storage_path(team_id);
        fs::create_dir_all(&storage_dir).await?;

        let file_path = storage_dir.join(&id);
        fs::write(&file_path, &file_data).await?;
        let file_path_str = file_path.to_string_lossy().to_string();

        sqlx::query(
            "INSERT INTO team_documents (id, team_id, folder_id, name, description, mime_type, file_size, file_path, uploaded_by, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"
        )
        .bind(&id)
        .bind(team_id)
        .bind(&folder_id)
        .bind(file_name)
        .bind(&description)
        .bind(mime_type)
        .bind(file_size)
        .bind(&file_path_str)
        .bind(user_id)
        .bind(now)
        .bind(now)
        .execute(pool)
        .await?;

        Ok(TeamDocument {
            id,
            team_id: team_id.to_string(),
            folder_id,
            name: file_name.to_string(),
            display_name: None,
            description,
            mime_type: mime_type.to_string(),
            file_size,
            file_path: file_path_str,
            uploaded_by: user_id.to_string(),
            created_at: now,
            updated_at: now,
        })
    }

    pub async fn list_documents(
        &self,
        pool: &SqlitePool,
        team_id: &str,
        folder_id: Option<&str>,
        page: u32,
        limit: u32,
    ) -> Result<(Vec<DocumentSummary>, i64)> {
        let offset = page.saturating_sub(1) * limit;

        let (items, total) = if let Some(fid) = folder_id {
            let rows = sqlx::query_as::<_, (String, String, Option<String>, String, i64, Option<String>, String, String)>(
                "SELECT id, name, display_name, mime_type, file_size, folder_id, uploaded_by, created_at
                 FROM team_documents WHERE team_id = ? AND folder_id = ? AND is_deleted = 0
                 ORDER BY created_at DESC LIMIT ? OFFSET ?"
            )
            .bind(team_id)
            .bind(fid)
            .bind(limit)
            .bind(offset)
            .fetch_all(pool)
            .await?;

            let count: (i64,) = sqlx::query_as(
                "SELECT COUNT(*) FROM team_documents WHERE team_id = ? AND folder_id = ? AND is_deleted = 0"
            )
            .bind(team_id)
            .bind(fid)
            .fetch_one(pool)
            .await?;

            (rows, count.0)
        } else {
            let rows = sqlx::query_as::<_, (String, String, Option<String>, String, i64, Option<String>, String, String)>(
                "SELECT id, name, display_name, mime_type, file_size, folder_id, uploaded_by, created_at
                 FROM team_documents WHERE team_id = ? AND is_deleted = 0
                 ORDER BY created_at DESC LIMIT ? OFFSET ?"
            )
            .bind(team_id)
            .bind(limit)
            .bind(offset)
            .fetch_all(pool)
            .await?;

            let count: (i64,) = sqlx::query_as(
                "SELECT COUNT(*) FROM team_documents WHERE team_id = ? AND is_deleted = 0",
            )
            .bind(team_id)
            .fetch_one(pool)
            .await?;

            (rows, count.0)
        };

        let summaries = items
            .into_iter()
            .map(|r| DocumentSummary {
                id: r.0,
                name: r.1,
                display_name: r.2,
                mime_type: r.3,
                file_size: r.4,
                folder_id: r.5,
                uploaded_by: r.6,
                created_at: r.7.parse().unwrap_or_default(),
            })
            .collect();

        Ok((summaries, total))
    }

    pub async fn get_document(
        &self,
        pool: &SqlitePool,
        doc_id: &str,
    ) -> Result<Option<TeamDocument>> {
        let row = sqlx::query_as::<_, (String, String, Option<String>, String, Option<String>, Option<String>, String, i64, String, String, String, String)>(
            "SELECT id, team_id, folder_id, name, display_name, description, mime_type, file_size, file_path, uploaded_by, created_at, updated_at
             FROM team_documents WHERE id = ? AND is_deleted = 0"
        )
        .bind(doc_id)
        .fetch_optional(pool)
        .await?;

        Ok(row.map(|r| TeamDocument {
            id: r.0,
            team_id: r.1,
            folder_id: r.2,
            name: r.3,
            display_name: r.4,
            description: r.5,
            mime_type: r.6,
            file_size: r.7,
            file_path: r.8,
            uploaded_by: r.9,
            created_at: r.10.parse().unwrap_or_default(),
            updated_at: r.11.parse().unwrap_or_default(),
        }))
    }

    pub async fn delete_document(&self, pool: &SqlitePool, doc_id: &str) -> Result<()> {
        // Get file path first
        if let Some(doc) = self.get_document(pool, doc_id).await? {
            // Delete file
            let _ = fs::remove_file(&doc.file_path).await;
        }

        sqlx::query("UPDATE team_documents SET is_deleted = 1, updated_at = ? WHERE id = ?")
            .bind(Utc::now())
            .bind(doc_id)
            .execute(pool)
            .await?;
        Ok(())
    }

    pub async fn download_document(
        &self,
        pool: &SqlitePool,
        doc_id: &str,
    ) -> Result<(Vec<u8>, String, String)> {
        let doc = self
            .get_document(pool, doc_id)
            .await?
            .ok_or_else(|| anyhow!("Document not found"))?;

        let data = fs::read(&doc.file_path).await?;
        Ok((data, doc.name, doc.mime_type))
    }

    /// Get file path for agent to process
    pub async fn get_file_for_agent(
        &self,
        pool: &SqlitePool,
        doc_id: &str,
    ) -> Result<AgentFileResponse> {
        let doc = self
            .get_document(pool, doc_id)
            .await?
            .ok_or_else(|| anyhow!("Document not found"))?;

        Ok(AgentFileResponse {
            id: doc.id,
            name: doc.name,
            mime_type: doc.mime_type,
            file_path: doc.file_path,
            file_size: doc.file_size,
        })
    }

    pub async fn search_documents(
        &self,
        pool: &SqlitePool,
        team_id: &str,
        query: &DocumentSearchQuery,
    ) -> Result<Vec<DocumentSearchResult>> {
        let search_pattern = format!("%{}%", query.query);
        let limit = query.limit.unwrap_or(20);

        let rows = sqlx::query_as::<_, (String, String, Option<String>, String, i64, Option<String>, String, String)>(
            "SELECT id, name, display_name, mime_type, file_size, folder_id, uploaded_by, created_at
             FROM team_documents
             WHERE team_id = ? AND is_deleted = 0 AND (name LIKE ? OR description LIKE ?)
             ORDER BY created_at DESC LIMIT ?"
        )
        .bind(team_id)
        .bind(&search_pattern)
        .bind(&search_pattern)
        .bind(limit)
        .fetch_all(pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|r| DocumentSearchResult {
                document: DocumentSummary {
                    id: r.0,
                    name: r.1,
                    display_name: r.2,
                    mime_type: r.3,
                    file_size: r.4,
                    folder_id: r.5,
                    uploaded_by: r.6,
                    created_at: r.7.parse().unwrap_or_default(),
                },
                score: 1.0,
            })
            .collect())
    }
}
