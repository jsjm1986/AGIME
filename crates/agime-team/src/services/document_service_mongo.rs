//! Document service for MongoDB (simplified without GridFS)

use crate::db::{collections, MongoDb};
use crate::models::mongo::{
    ArchivedDocument, Document, DocumentCategory, DocumentOrigin, DocumentStatus, DocumentSummary,
    PaginatedResponse, SourceDocumentSnapshot,
};
use anyhow::{anyhow, Result};
use chrono::Utc;
use futures::TryStreamExt;
use mongodb::bson::{doc, oid::ObjectId, Binary, Regex as BsonRegex};
use mongodb::options::FindOptions;

pub struct DocumentService {
    db: MongoDb,
}

impl DocumentService {
    pub fn new(db: MongoDb) -> Self {
        Self { db }
    }

    /// Upload a document (stores content directly in MongoDB)
    pub async fn upload(
        &self,
        team_id: &str,
        user_id: &str,
        file_name: &str,
        data: Vec<u8>,
        mime_type: &str,
        folder_path: Option<String>,
    ) -> Result<Document> {
        let team_oid = ObjectId::parse_str(team_id)?;
        let now = Utc::now();
        let file_size = data.len() as i64;

        // Create document with binary content
        let doc = Document {
            id: None,
            team_id: team_oid,
            folder_path: folder_path.unwrap_or_else(|| "/".to_string()),
            name: file_name.to_string(),
            display_name: None,
            description: None,
            mime_type: mime_type.to_string(),
            file_size,
            content: Some(Binary {
                subtype: mongodb::bson::spec::BinarySubtype::Generic,
                bytes: data,
            }),
            tags: vec![],
            uploaded_by: user_id.to_string(),
            is_deleted: false,
            is_public: false,
            origin: DocumentOrigin::default(),
            status: DocumentStatus::default(),
            category: DocumentCategory::default(),
            source_document_ids: vec![],
            source_snapshots: vec![],
            source_session_id: None,
            source_mission_id: None,
            created_by_agent_id: None,
            supersedes_id: None,
            lineage_description: None,
            created_at: now,
            updated_at: now,
        };

        let coll = self.db.collection::<Document>("documents");
        let result = coll.insert_one(&doc, None).await?;

        let mut doc = doc;
        doc.id = result.inserted_id.as_object_id();
        Ok(doc)
    }

    /// List documents
    pub async fn list(
        &self,
        team_id: &str,
        folder_path: Option<&str>,
    ) -> Result<Vec<DocumentSummary>> {
        let team_oid = ObjectId::parse_str(team_id)?;
        let coll = self.db.collection::<Document>("documents");

        // Filter out soft-deleted documents
        let filter = if let Some(path) = folder_path {
            doc! { "team_id": team_oid, "folder_path": path, "is_deleted": { "$ne": true } }
        } else {
            doc! { "team_id": team_oid, "is_deleted": { "$ne": true } }
        };

        let cursor = coll.find(filter, None).await?;
        let docs: Vec<Document> = cursor.try_collect().await?;

        Ok(docs.into_iter().map(DocumentSummary::from).collect())
    }

    /// Download document
    pub async fn download(&self, team_id: &str, doc_id: &str) -> Result<(Vec<u8>, String, String)> {
        let team_oid = ObjectId::parse_str(team_id)?;
        let oid = ObjectId::parse_str(doc_id)?;
        let coll = self.db.collection::<Document>("documents");

        // Filter out soft-deleted documents
        let doc = coll
            .find_one(
                doc! { "_id": oid, "team_id": team_oid, "is_deleted": { "$ne": true } },
                None,
            )
            .await?
            .ok_or_else(|| anyhow!("Document not found"))?;

        let data = doc.content.map(|b| b.bytes).unwrap_or_default();

        Ok((data, doc.name, doc.mime_type))
    }

    /// Delete document (soft delete with archiving)
    pub async fn delete(&self, team_id: &str, doc_id: &str, deleted_by: &str) -> Result<()> {
        let team_oid = ObjectId::parse_str(team_id)?;
        let oid = ObjectId::parse_str(doc_id)?;
        let coll = self.db.collection::<Document>("documents");

        // Fetch full document for archiving (only if not already deleted)
        let doc = coll
            .find_one(doc! { "_id": oid, "team_id": team_oid, "is_deleted": { "$ne": true } }, None)
            .await?;

        // Archive metadata (without binary content) before soft-deleting
        if let Some(ref d) = doc {
            let archived = ArchivedDocument::from_document(d, deleted_by, None);
            let archive_coll = self
                .db
                .collection::<ArchivedDocument>(collections::ARCHIVED_DOCUMENTS);
            if let Err(e) = archive_coll.insert_one(&archived, None).await {
                tracing::warn!("Failed to archive document {}: {}", doc_id, e);
            }
        }

        // Soft-delete the original
        coll.update_one(
            doc! { "_id": oid, "team_id": team_oid },
            doc! { "$set": { "is_deleted": true, "updated_at": bson::DateTime::from_chrono(Utc::now()) } },
            None
        ).await?;
        Ok(())
    }

    /// List documents with pagination
    pub async fn list_paginated(
        &self,
        team_id: &str,
        folder_path: Option<&str>,
        page: Option<u64>,
        limit: Option<u64>,
    ) -> Result<PaginatedResponse<DocumentSummary>> {
        let team_oid = ObjectId::parse_str(team_id)?;
        let coll = self.db.collection::<Document>("documents");

        let mut filter = doc! { "team_id": team_oid, "is_deleted": { "$ne": true } };
        if let Some(path) = folder_path {
            filter.insert("folder_path", path);
        }

        let total = coll.count_documents(filter.clone(), None).await?;
        let page = page.unwrap_or(1).max(1);
        let limit = limit.unwrap_or(50).min(500);
        let skip = (page - 1) * limit;

        let options = FindOptions::builder()
            .skip(skip)
            .limit(limit as i64)
            .sort(doc! { "created_at": -1 })
            .build();

        let cursor = coll.find(filter, options).await?;
        let docs: Vec<Document> = cursor.try_collect().await?;
        let items = docs.into_iter().map(DocumentSummary::from).collect();

        Ok(PaginatedResponse::new(items, total, page, limit))
    }

    /// Get document metadata (without content)
    pub async fn get_metadata(&self, team_id: &str, doc_id: &str) -> Result<DocumentSummary> {
        let team_oid = ObjectId::parse_str(team_id)?;
        let oid = ObjectId::parse_str(doc_id)?;
        let coll = self.db.collection::<Document>("documents");

        let doc = coll
            .find_one(
                doc! { "_id": oid, "team_id": team_oid, "is_deleted": { "$ne": true } },
                None,
            )
            .await?
            .ok_or_else(|| anyhow!("Document not found"))?;

        Ok(DocumentSummary::from(doc))
    }

    /// Get text content of a document (for text-based files)
    /// Returns (text_content, mime_type)
    pub async fn get_text_content(&self, team_id: &str, doc_id: &str) -> Result<(String, String)> {
        let team_oid = ObjectId::parse_str(team_id)?;
        let oid = ObjectId::parse_str(doc_id)?;
        let coll = self.db.collection::<Document>("documents");

        let doc = coll
            .find_one(
                doc! { "_id": oid, "team_id": team_oid, "is_deleted": { "$ne": true } },
                None,
            )
            .await?
            .ok_or_else(|| anyhow!("Document not found"))?;

        let mime = &doc.mime_type;
        let is_text = mime.starts_with("text/")
            || mime == "application/json"
            || mime == "application/xml"
            || mime == "application/x-yaml"
            || mime == "application/javascript"
            || mime == "application/typescript";

        if !is_text {
            return Err(anyhow!("Document is not a text file"));
        }

        let data = doc.content.map(|b| b.bytes).unwrap_or_default();
        let text =
            String::from_utf8(data).map_err(|_| anyhow!("Document content is not valid UTF-8"))?;

        Ok((text, doc.mime_type))
    }

    /// Update document content
    pub async fn update_content(
        &self,
        team_id: &str,
        doc_id: &str,
        _user_id: &str,
        data: Vec<u8>,
        _message: &str,
    ) -> Result<Document> {
        let team_oid = ObjectId::parse_str(team_id)?;
        let oid = ObjectId::parse_str(doc_id)?;
        let coll = self.db.collection::<Document>("documents");
        let now = Utc::now();
        let file_size = data.len() as i64;

        coll.update_one(
            doc! { "_id": oid, "team_id": team_oid, "is_deleted": { "$ne": true } },
            doc! {
                "$set": {
                    "content": Binary { subtype: mongodb::bson::spec::BinarySubtype::Generic, bytes: data },
                    "file_size": file_size,
                    "updated_at": bson::DateTime::from_chrono(now),
                }
            },
            None,
        )
        .await?;

        let doc = coll
            .find_one(doc! { "_id": oid }, None)
            .await?
            .ok_or_else(|| anyhow!("Document not found after update"))?;

        Ok(doc)
    }

    /// Acquire a lock on a document
    pub async fn acquire_lock(
        &self,
        doc_id: &str,
        team_id: &str,
        user_id: &str,
        user_name: &str,
    ) -> Result<crate::models::mongo::DocumentLock> {
        use crate::models::mongo::DocumentLock;

        let doc_oid = ObjectId::parse_str(doc_id)?;
        let team_oid = ObjectId::parse_str(team_id)?;
        let coll = self.db.collection::<DocumentLock>("document_locks");
        let now = Utc::now();
        let expires = now + chrono::Duration::minutes(30);

        // Clean up expired locks first
        coll.delete_many(
            doc! { "document_id": doc_oid, "expires_at": { "$lte": bson::DateTime::from_chrono(now) } },
            None,
        )
        .await?;

        // Check for existing non-expired lock held by another user
        let existing = coll
            .find_one(
                doc! {
                    "document_id": doc_oid,
                    "expires_at": { "$gt": bson::DateTime::from_chrono(now) }
                },
                None,
            )
            .await?;

        if let Some(lock) = existing {
            if lock.locked_by != user_id {
                return Err(anyhow!("Document is locked by {}", lock.locked_by_name));
            }
            // Refresh lock for same user
            coll.update_one(
                doc! { "_id": lock.id },
                doc! { "$set": { "expires_at": bson::DateTime::from_chrono(expires) } },
                None,
            )
            .await?;
            return coll
                .find_one(doc! { "_id": lock.id }, None)
                .await?
                .ok_or_else(|| anyhow!("Lock disappeared after refresh"));
        }

        let lock = DocumentLock {
            id: None,
            document_id: doc_oid,
            team_id: team_oid,
            locked_by: user_id.to_string(),
            locked_by_name: user_name.to_string(),
            locked_at: now,
            expires_at: expires,
        };

        let result = coll.insert_one(&lock, None).await?;
        let mut lock = lock;
        lock.id = result.inserted_id.as_object_id();
        Ok(lock)
    }

    /// Release a lock on a document
    pub async fn release_lock(&self, doc_id: &str, user_id: &str) -> Result<()> {
        use crate::models::mongo::DocumentLock;

        let doc_oid = ObjectId::parse_str(doc_id)?;
        let coll = self.db.collection::<DocumentLock>("document_locks");

        coll.delete_many(doc! { "document_id": doc_oid, "locked_by": user_id }, None)
            .await?;

        Ok(())
    }

    /// Get lock status for a document
    pub async fn get_lock(
        &self,
        doc_id: &str,
    ) -> Result<Option<crate::models::mongo::DocumentLock>> {
        use crate::models::mongo::DocumentLock;

        let doc_oid = ObjectId::parse_str(doc_id)?;
        let coll = self.db.collection::<DocumentLock>("document_locks");
        let now = Utc::now();

        let lock = coll
            .find_one(
                doc! {
                    "document_id": doc_oid,
                    "expires_at": { "$gt": bson::DateTime::from_chrono(now) }
                },
                None,
            )
            .await?;

        Ok(lock)
    }

    /// Check if current user holds the lock
    pub async fn check_lock(&self, doc_id: &str, user_id: &str) -> Result<bool> {
        use crate::models::mongo::DocumentLock;

        let doc_oid = ObjectId::parse_str(doc_id)?;
        let coll = self.db.collection::<DocumentLock>("document_locks");
        let now = Utc::now();

        let lock = coll
            .find_one(
                doc! {
                    "document_id": doc_oid,
                    "locked_by": user_id,
                    "expires_at": { "$gt": bson::DateTime::from_chrono(now) }
                },
                None,
            )
            .await?;

        Ok(lock.is_some())
    }

    /// Update document metadata (display_name, description, tags)
    pub async fn update_metadata(
        &self,
        team_id: &str,
        doc_id: &str,
        display_name: Option<String>,
        description: Option<String>,
        tags: Option<Vec<String>>,
    ) -> Result<DocumentSummary> {
        let team_oid = ObjectId::parse_str(team_id)?;
        let oid = ObjectId::parse_str(doc_id)?;
        let coll = self.db.collection::<Document>("documents");
        let now = Utc::now();

        let mut set_doc = doc! { "updated_at": bson::DateTime::from_chrono(now) };
        if let Some(name) = display_name {
            set_doc.insert("display_name", name);
        }
        if let Some(desc) = description {
            set_doc.insert("description", desc);
        }
        if let Some(t) = tags {
            set_doc.insert("tags", t);
        }

        let result = coll
            .update_one(
                doc! { "_id": oid, "team_id": team_oid, "is_deleted": { "$ne": true } },
                doc! { "$set": set_doc },
                None,
            )
            .await?;

        if result.matched_count == 0 {
            return Err(anyhow!("Document not found"));
        }

        let doc = coll
            .find_one(doc! { "_id": oid }, None)
            .await?
            .ok_or_else(|| anyhow!("Document not found after update"))?;

        Ok(DocumentSummary::from(doc))
    }

    /// Search documents by name/description/tags
    pub async fn search(
        &self,
        team_id: &str,
        query: &str,
        page: Option<u64>,
        limit: Option<u64>,
        mime_type: Option<&str>,
        folder_path: Option<&str>,
    ) -> Result<PaginatedResponse<DocumentSummary>> {
        let team_oid = ObjectId::parse_str(team_id)?;
        let coll = self.db.collection::<Document>("documents");

        let escaped = regex::escape(query);
        let re = BsonRegex {
            pattern: escaped,
            options: "i".to_string(),
        };

        let mut filter = doc! {
            "team_id": team_oid,
            "is_deleted": { "$ne": true },
            "$or": [
                { "name": { "$regex": &re } },
                { "description": { "$regex": &re } },
                { "tags": { "$regex": &re } },
            ]
        };

        if let Some(mt) = mime_type {
            let mime_re = BsonRegex {
                pattern: format!("^{}", regex::escape(mt)),
                options: "i".to_string(),
            };
            filter.insert("mime_type", doc! { "$regex": mime_re });
        }
        if let Some(fp) = folder_path {
            filter.insert("folder_path", fp);
        }

        let total = coll.count_documents(filter.clone(), None).await?;
        let page = page.unwrap_or(1).max(1);
        let limit = limit.unwrap_or(50).min(500);
        let skip = (page - 1) * limit;

        let options = FindOptions::builder()
            .skip(skip)
            .limit(limit as i64)
            .sort(doc! { "created_at": -1 })
            .build();

        let cursor = coll.find(filter, options).await?;
        let docs: Vec<Document> = cursor.try_collect().await?;
        let items = docs.into_iter().map(DocumentSummary::from).collect();

        Ok(PaginatedResponse::new(items, total, page, limit))
    }

    /// List documents by origin (human or agent)
    pub async fn list_by_origin(
        &self,
        team_id: &str,
        origin: &str,
        page: Option<u64>,
        limit: Option<u64>,
    ) -> Result<PaginatedResponse<DocumentSummary>> {
        let team_oid = ObjectId::parse_str(team_id)?;
        let coll = self.db.collection::<Document>("documents");

        let filter = doc! {
            "team_id": team_oid,
            "is_deleted": { "$ne": true },
            "origin": origin,
        };

        let total = coll.count_documents(filter.clone(), None).await?;
        let page = page.unwrap_or(1).max(1);
        let limit = limit.unwrap_or(50).min(500);
        let skip = (page - 1) * limit;

        let options = FindOptions::builder()
            .skip(skip)
            .limit(limit as i64)
            .sort(doc! { "created_at": -1 })
            .build();

        let cursor = coll.find(filter, options).await?;
        let docs: Vec<Document> = cursor.try_collect().await?;
        let items = docs.into_iter().map(DocumentSummary::from).collect();

        Ok(PaginatedResponse::new(items, total, page, limit))
    }

    /// List AI workbench documents (agent-created), optionally filtered by session/mission
    pub async fn list_ai_workbench(
        &self,
        team_id: &str,
        session_id: Option<&str>,
        mission_id: Option<&str>,
        page: Option<u64>,
        limit: Option<u64>,
    ) -> Result<PaginatedResponse<DocumentSummary>> {
        let team_oid = ObjectId::parse_str(team_id)?;
        let coll = self.db.collection::<Document>("documents");

        let mut filter = doc! {
            "team_id": team_oid,
            "is_deleted": { "$ne": true },
            "origin": "agent",
        };
        if let Some(sid) = session_id {
            filter.insert("source_session_id", sid);
        }
        if let Some(mid) = mission_id {
            filter.insert("source_mission_id", mid);
        }

        let total = coll.count_documents(filter.clone(), None).await?;
        let page = page.unwrap_or(1).max(1);
        let limit = limit.unwrap_or(50).min(500);
        let skip = (page - 1) * limit;

        let options = FindOptions::builder()
            .skip(skip)
            .limit(limit as i64)
            .sort(doc! { "created_at": -1 })
            .build();

        let cursor = coll.find(filter, options).await?;
        let docs: Vec<Document> = cursor.try_collect().await?;
        let items = docs.into_iter().map(DocumentSummary::from).collect();

        Ok(PaginatedResponse::new(items, total, page, limit))
    }

    /// Update document status
    pub async fn update_status(
        &self,
        team_id: &str,
        doc_id: &str,
        status: DocumentStatus,
    ) -> Result<()> {
        let team_oid = ObjectId::parse_str(team_id)?;
        let oid = ObjectId::parse_str(doc_id)?;
        let coll = self.db.collection::<Document>("documents");
        let now = Utc::now();

        let status_str = serde_json::to_value(status)
            .ok()
            .and_then(|v| v.as_str().map(|s| s.to_string()))
            .unwrap_or_else(|| "active".to_string());

        let result = coll
            .update_one(
                doc! { "_id": oid, "team_id": team_oid, "is_deleted": { "$ne": true } },
                doc! { "$set": { "status": status_str, "updated_at": bson::DateTime::from_chrono(now) } },
                None,
            )
            .await?;

        if result.matched_count == 0 {
            return Err(anyhow!("Document not found"));
        }
        Ok(())
    }

    /// Create a document with full metadata (used by Agent tools)
    #[allow(clippy::too_many_arguments)]
    pub async fn create_with_metadata(
        &self,
        team_id: &str,
        creator_id: &str,
        name: &str,
        data: Vec<u8>,
        mime_type: &str,
        folder_path: Option<String>,
        origin: DocumentOrigin,
        status: DocumentStatus,
        category: DocumentCategory,
        source_document_ids: Vec<String>,
        source_snapshots: Vec<SourceDocumentSnapshot>,
        source_session_id: Option<String>,
        source_mission_id: Option<String>,
        created_by_agent_id: Option<String>,
        supersedes_id: Option<String>,
        lineage_description: Option<String>,
    ) -> Result<Document> {
        let team_oid = ObjectId::parse_str(team_id)?;
        let now = Utc::now();
        let file_size = data.len() as i64;

        let doc = Document {
            id: None,
            team_id: team_oid,
            folder_path: folder_path.unwrap_or_else(|| "/".to_string()),
            name: name.to_string(),
            display_name: None,
            description: None,
            mime_type: mime_type.to_string(),
            file_size,
            content: Some(Binary {
                subtype: mongodb::bson::spec::BinarySubtype::Generic,
                bytes: data,
            }),
            tags: vec![],
            uploaded_by: creator_id.to_string(),
            is_deleted: false,
            is_public: false,
            origin,
            status,
            category,
            source_document_ids,
            source_snapshots,
            source_session_id,
            source_mission_id,
            created_by_agent_id,
            supersedes_id,
            lineage_description,
            created_at: now,
            updated_at: now,
        };

        let coll = self.db.collection::<Document>("documents");
        let result = coll.insert_one(&doc, None).await?;

        let mut doc = doc;
        doc.id = result.inserted_id.as_object_id();
        Ok(doc)
    }

    /// Get text content with chunking support (offset/limit in bytes)
    /// Returns (text_content, mime_type, total_size)
    pub async fn get_text_content_chunked(
        &self,
        team_id: &str,
        doc_id: &str,
        offset: Option<usize>,
        limit: Option<usize>,
    ) -> Result<(String, String, usize)> {
        let (full_text, mime_type) = self.get_text_content(team_id, doc_id).await?;
        let total_size = full_text.len();
        let offset = offset.unwrap_or(0).min(total_size);
        let limit = limit.unwrap_or(total_size);
        let end = (offset + limit).min(total_size);

        // Snap offset and end to valid UTF-8 char boundaries to avoid panics
        let safe_offset = if full_text.is_char_boundary(offset) {
            offset
        } else {
            full_text
                .char_indices()
                .map(|(i, _)| i)
                .take_while(|&i| i <= offset)
                .last()
                .unwrap_or(0)
        };
        let safe_end = if full_text.is_char_boundary(end) {
            end
        } else {
            full_text
                .char_indices()
                .map(|(i, _)| i)
                .take_while(|&i| i <= end)
                .last()
                .unwrap_or(safe_offset)
        };

        let text = &full_text[safe_offset..safe_end];
        Ok((text.to_string(), mime_type, total_size))
    }

    /// Get document lineage (source chain + derived documents).
    /// Falls back to archived_documents when a source document has been deleted.
    pub async fn get_lineage(&self, team_id: &str, doc_id: &str) -> Result<Vec<DocumentSummary>> {
        let team_oid = ObjectId::parse_str(team_id)?;
        let coll = self.db.collection::<Document>("documents");
        let archive_coll = self
            .db
            .collection::<ArchivedDocument>(collections::ARCHIVED_DOCUMENTS);
        let mut lineage = Vec::new();

        // Walk up the source chain
        let mut current_id = doc_id.to_string();
        let mut visited = std::collections::HashSet::new();
        loop {
            if !visited.insert(current_id.clone()) {
                break;
            }

            let oid = match ObjectId::parse_str(&current_id) {
                Ok(o) => o,
                Err(_) => break,
            };

            // Try live document first, then fall back to archived
            let (summary, source_ids) = if let Some(d) = coll
                .find_one(
                    doc! { "_id": oid, "team_id": &team_oid, "is_deleted": { "$ne": true } },
                    None,
                )
                .await?
            {
                let ids = d.source_document_ids.clone();
                (DocumentSummary::from(d), ids)
            } else if let Some(a) = archive_coll
                .find_one(doc! { "original_id": oid, "team_id": &team_oid }, None)
                .await?
            {
                let ids = a.source_document_ids.clone();
                (a.to_summary(), ids)
            } else {
                break;
            };

            lineage.push(summary);
            match source_ids.first() {
                Some(next) => current_id = next.clone(),
                None => break,
            }
        }

        // Reverse so source is first
        lineage.reverse();

        // Find derived documents (only active ones)
        let derived_filter = doc! {
            "team_id": &team_oid,
            "source_document_ids": doc_id,
            "is_deleted": { "$ne": true },
        };
        let options = FindOptions::builder()
            .sort(doc! { "created_at": 1 })
            .limit(50)
            .build();
        let cursor = coll.find(derived_filter, options).await?;
        let derived: Vec<Document> = cursor.try_collect().await?;

        for d in derived {
            let id_str = d.id.map(|id| id.to_hex()).unwrap_or_default();
            if !visited.contains(&id_str) {
                lineage.push(DocumentSummary::from(d));
            }
        }

        Ok(lineage)
    }

    /// List documents derived from a given document
    pub async fn list_derived(
        &self,
        team_id: &str,
        doc_id: &str,
        page: Option<u64>,
        limit: Option<u64>,
    ) -> Result<PaginatedResponse<DocumentSummary>> {
        let team_oid = ObjectId::parse_str(team_id)?;
        let coll = self.db.collection::<Document>("documents");

        let filter = doc! {
            "team_id": team_oid,
            "source_document_ids": doc_id,
            "is_deleted": { "$ne": true },
        };

        let total = coll.count_documents(filter.clone(), None).await?;
        let page = page.unwrap_or(1).max(1);
        let limit = limit.unwrap_or(50).min(500);
        let skip = (page - 1) * limit;

        let options = FindOptions::builder()
            .skip(skip)
            .limit(limit as i64)
            .sort(doc! { "created_at": -1 })
            .build();

        let cursor = coll.find(filter, options).await?;
        let docs: Vec<Document> = cursor.try_collect().await?;
        let items = docs.into_iter().map(DocumentSummary::from).collect();

        Ok(PaginatedResponse::new(items, total, page, limit))
    }

    /// List archived (deleted) documents for a team
    pub async fn list_archived(
        &self,
        team_id: &str,
        page: Option<u64>,
        limit: Option<u64>,
    ) -> Result<PaginatedResponse<ArchivedDocument>> {
        let team_oid = ObjectId::parse_str(team_id)?;
        let coll = self
            .db
            .collection::<ArchivedDocument>(collections::ARCHIVED_DOCUMENTS);

        let filter = doc! { "team_id": team_oid };
        let total = coll.count_documents(filter.clone(), None).await?;
        let page = page.unwrap_or(1).max(1);
        let limit = limit.unwrap_or(50).min(500);
        let skip = (page - 1) * limit;

        let options = FindOptions::builder()
            .skip(skip)
            .limit(limit as i64)
            .sort(doc! { "deleted_at": -1 })
            .build();

        let cursor = coll.find(filter, options).await?;
        let items: Vec<ArchivedDocument> = cursor.try_collect().await?;

        Ok(PaginatedResponse::new(items, total, page, limit))
    }

    /// Build source document snapshots for embedding in derived documents.
    /// Uses a single batch query instead of N individual lookups.
    pub async fn build_source_snapshots(
        &self,
        team_id: &str,
        source_ids: &[String],
    ) -> Result<Vec<SourceDocumentSnapshot>> {
        if source_ids.is_empty() {
            return Ok(vec![]);
        }

        let team_oid = ObjectId::parse_str(team_id)?;
        let oids: Vec<ObjectId> = source_ids
            .iter()
            .filter_map(|s| ObjectId::parse_str(s).ok())
            .collect();

        let coll = self.db.collection::<Document>("documents");
        let filter = doc! { "_id": { "$in": &oids }, "team_id": &team_oid };
        let cursor = coll.find(filter, None).await?;
        let docs: Vec<Document> = cursor.try_collect().await?;

        Ok(docs
            .into_iter()
            .map(|d| SourceDocumentSnapshot {
                id: d.id.map(|id| id.to_hex()).unwrap_or_default(),
                name: d.name,
                mime_type: d.mime_type,
                origin: d.origin,
                category: d.category,
            })
            .collect())
    }

    /// Move a document to a different folder
    pub async fn move_to_folder(
        &self,
        team_id: &str,
        doc_id: &str,
        new_folder_path: &str,
    ) -> Result<()> {
        let team_oid = ObjectId::parse_str(team_id)?;
        let oid = ObjectId::parse_str(doc_id)?;
        let coll = self.db.collection::<Document>("documents");
        let result = coll
            .update_one(
                doc! { "_id": oid, "team_id": team_oid, "is_deleted": { "$ne": true } },
                doc! { "$set": {
                    "folder_path": new_folder_path,
                    "updated_at": bson::DateTime::from_chrono(Utc::now()),
                }},
                None,
            )
            .await?;
        if result.matched_count == 0 {
            return Err(anyhow!("Document not found"));
        }
        Ok(())
    }

    /// Set the is_public flag on a document
    pub async fn set_public(&self, team_id: &str, doc_id: &str, is_public: bool) -> Result<()> {
        let team_oid = ObjectId::parse_str(team_id)?;
        let oid = ObjectId::parse_str(doc_id)?;
        let coll = self.db.collection::<Document>("documents");
        let result = coll
            .update_one(
                doc! { "_id": oid, "team_id": team_oid, "is_deleted": { "$ne": true } },
                doc! { "$set": {
                    "is_public": is_public,
                    "updated_at": bson::DateTime::from_chrono(Utc::now()),
                }},
                None,
            )
            .await?;
        if result.matched_count == 0 {
            return Err(anyhow!("Document not found"));
        }
        Ok(())
    }
}
