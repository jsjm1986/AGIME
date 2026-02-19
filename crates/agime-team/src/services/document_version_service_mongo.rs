//! Document version service for MongoDB

use crate::db::MongoDb;
use crate::models::mongo::{DocumentVersion, DocumentVersionSummary, PaginatedResponse};
use anyhow::{anyhow, Result};
use chrono::Utc;
use futures::TryStreamExt;
use mongodb::bson::{doc, oid::ObjectId, Binary};
use mongodb::options::FindOptions;

pub struct DocumentVersionService {
    db: MongoDb,
}

impl DocumentVersionService {
    pub fn new(db: MongoDb) -> Self {
        Self { db }
    }

    /// Create a new version snapshot
    pub async fn create_version(
        &self,
        doc_id: &str,
        team_id: &str,
        user_id: &str,
        user_name: &str,
        content: Vec<u8>,
        message: &str,
    ) -> Result<DocumentVersion> {
        let doc_oid = ObjectId::parse_str(doc_id)?;
        let team_oid = ObjectId::parse_str(team_id)?;
        let now = Utc::now();
        let file_size = content.len() as i64;

        // Get next version number
        let coll = self.db.collection::<DocumentVersion>("document_versions");
        let latest = coll
            .find_one(
                doc! { "document_id": doc_oid },
                mongodb::options::FindOneOptions::builder()
                    .sort(doc! { "version_number": -1 })
                    .build(),
            )
            .await?;

        let version_number = latest.map(|v| v.version_number + 1).unwrap_or(1);

        let version = DocumentVersion {
            id: None,
            document_id: doc_oid,
            team_id: team_oid,
            version_number,
            content: Some(Binary {
                subtype: mongodb::bson::spec::BinarySubtype::Generic,
                bytes: content,
            }),
            file_size,
            message: message.to_string(),
            created_by: user_id.to_string(),
            created_by_name: user_name.to_string(),
            tag: None,
            created_at: now,
        };

        let result = coll.insert_one(&version, None).await?;
        let mut version = version;
        version.id = result.inserted_id.as_object_id();

        // Version retention policy: keep max 100 versions per document
        let max_versions: u64 = 100;
        let count = coll
            .count_documents(doc! { "document_id": doc_oid }, None)
            .await?;
        if count > max_versions {
            let to_remove = count - max_versions;
            let old_options = FindOptions::builder()
                .sort(doc! { "version_number": 1 })
                .limit(to_remove as i64)
                .build();
            let cursor = coll
                .find(doc! { "document_id": doc_oid }, old_options)
                .await?;
            let old_versions: Vec<DocumentVersion> = cursor.try_collect().await?;
            let old_ids: Vec<ObjectId> = old_versions.into_iter().filter_map(|v| v.id).collect();
            if !old_ids.is_empty() {
                coll.delete_many(doc! { "_id": { "$in": old_ids } }, None)
                    .await?;
            }
        }

        Ok(version)
    }

    /// List versions for a document (paginated, without content)
    pub async fn list_versions(
        &self,
        doc_id: &str,
        page: Option<u64>,
        limit: Option<u64>,
    ) -> Result<PaginatedResponse<DocumentVersionSummary>> {
        let doc_oid = ObjectId::parse_str(doc_id)?;
        let coll = self.db.collection::<DocumentVersion>("document_versions");

        let filter = doc! { "document_id": doc_oid };
        let total = coll.count_documents(filter.clone(), None).await?;
        let page = page.unwrap_or(1).max(1);
        let limit = limit.unwrap_or(50).min(500);
        let skip = (page - 1) * limit;

        let options = FindOptions::builder()
            .skip(skip)
            .limit(limit as i64)
            .sort(doc! { "version_number": -1 })
            .build();

        let cursor = coll.find(filter, options).await?;
        let versions: Vec<DocumentVersion> = cursor.try_collect().await?;
        let items = versions
            .into_iter()
            .map(DocumentVersionSummary::from)
            .collect();

        Ok(PaginatedResponse::new(items, total, page, limit))
    }

    /// Get a specific version (with content)
    pub async fn get_version(&self, version_id: &str) -> Result<DocumentVersion> {
        let oid = ObjectId::parse_str(version_id)?;
        let coll = self.db.collection::<DocumentVersion>("document_versions");

        coll.find_one(doc! { "_id": oid }, None)
            .await?
            .ok_or_else(|| anyhow!("Version not found"))
    }

    /// Get text content of a version
    pub async fn get_version_text(&self, version_id: &str) -> Result<(String, String)> {
        let version = self.get_version(version_id).await?;
        let data = version.content.map(|b| b.bytes).unwrap_or_default();
        let text =
            String::from_utf8(data).map_err(|_| anyhow!("Version content is not valid UTF-8"))?;

        // Get the document to find mime_type
        let doc_coll = self
            .db
            .collection::<crate::models::mongo::Document>("documents");
        let doc = doc_coll
            .find_one(doc! { "_id": version.document_id }, None)
            .await?
            .ok_or_else(|| anyhow!("Document not found"))?;

        Ok((text, doc.mime_type))
    }

    /// Rollback to a specific version (creates a new version)
    pub async fn rollback(
        &self,
        doc_id: &str,
        version_id: &str,
        user_id: &str,
        user_name: &str,
    ) -> Result<DocumentVersion> {
        let old_version = self.get_version(version_id).await?;
        let content = old_version.content.map(|b| b.bytes).unwrap_or_default();

        // Update the document's current content
        let doc_oid = ObjectId::parse_str(doc_id)?;
        let doc_coll = self
            .db
            .collection::<crate::models::mongo::Document>("documents");
        let now = Utc::now();

        doc_coll
            .update_one(
                doc! { "_id": doc_oid },
                doc! {
                    "$set": {
                        "content": Binary {
                            subtype: mongodb::bson::spec::BinarySubtype::Generic,
                            bytes: content.clone(),
                        },
                        "file_size": content.len() as i64,
                        "updated_at": bson::DateTime::from_chrono(now),
                    }
                },
                None,
            )
            .await?;

        // Get team_id from document
        let doc = doc_coll
            .find_one(doc! { "_id": doc_oid }, None)
            .await?
            .ok_or_else(|| anyhow!("Document not found"))?;

        let team_id = doc.team_id.to_hex();
        let message = format!("Rollback to version {}", old_version.version_number);

        self.create_version(doc_id, &team_id, user_id, user_name, content, &message)
            .await
    }

    /// Tag a version
    pub async fn tag_version(&self, version_id: &str, tag: &str) -> Result<()> {
        let oid = ObjectId::parse_str(version_id)?;
        let coll = self.db.collection::<DocumentVersion>("document_versions");

        let result = coll
            .update_one(doc! { "_id": oid }, doc! { "$set": { "tag": tag } }, None)
            .await?;

        if result.matched_count == 0 {
            return Err(anyhow!("Version not found"));
        }
        Ok(())
    }

    /// Delete a version
    pub async fn delete_version(&self, version_id: &str) -> Result<()> {
        let oid = ObjectId::parse_str(version_id)?;
        let coll = self.db.collection::<DocumentVersion>("document_versions");

        let result = coll.delete_one(doc! { "_id": oid }, None).await?;
        if result.deleted_count == 0 {
            return Err(anyhow!("Version not found"));
        }
        Ok(())
    }
}
