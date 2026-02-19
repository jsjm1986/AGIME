//! Extension service for MongoDB

use crate::db::MongoDb;
use crate::models::mongo::{
    increment_version, Extension, ExtensionSummary, PaginatedResponse, ResourceNameValidator,
};
use anyhow::{anyhow, Result};
use chrono::Utc;
use futures::TryStreamExt;
use mongodb::bson::{doc, oid::ObjectId, Document as BsonDoc, Regex as BsonRegex};
use mongodb::options::FindOptions;

pub struct ExtensionService {
    db: MongoDb,
}

impl ExtensionService {
    pub fn new(db: MongoDb) -> Self {
        Self { db }
    }

    pub async fn check_duplicate_name(
        &self,
        team_id: &str,
        name: &str,
        exclude_id: Option<&str>,
    ) -> Result<bool> {
        let team_oid = ObjectId::parse_str(team_id)?;
        let coll = self.db.collection::<Extension>("extensions");
        let mut filter = doc! {
            "team_id": team_oid,
            "name": name,
            "is_deleted": { "$ne": true }
        };
        if let Some(eid) = exclude_id {
            filter.insert("_id", doc! { "$ne": ObjectId::parse_str(eid)? });
        }
        let count = coll.count_documents(filter, None).await?;
        Ok(count > 0)
    }

    pub async fn create(
        &self,
        team_id: &str,
        user_id: &str,
        name: &str,
        extension_type: &str,
        config: BsonDoc,
        description: Option<String>,
        tags: Option<Vec<String>>,
        visibility: Option<String>,
    ) -> Result<Extension> {
        ResourceNameValidator::validate(name).map_err(|e| anyhow!(e))?;

        if self.check_duplicate_name(team_id, name, None).await? {
            return Err(anyhow!(
                "An extension with name '{}' already exists in this team",
                name
            ));
        }

        let team_oid = ObjectId::parse_str(team_id)?;
        let now = Utc::now();

        let ext = Extension {
            id: None,
            team_id: team_oid,
            name: name.to_string(),
            description,
            extension_type: extension_type.to_string(),
            config,
            tags: tags.unwrap_or_default(),
            version: "1.0.0".to_string(),
            previous_version_id: None,
            visibility: visibility.unwrap_or_else(|| "team".to_string()),
            protection_level: "team_installable".to_string(),
            security_reviewed: false,
            security_notes: None,
            reviewed_by: None,
            reviewed_at: None,
            use_count: 0,
            is_deleted: false,
            created_by: user_id.to_string(),
            ai_description: None,
            ai_description_lang: None,
            ai_described_at: None,
            created_at: now,
            updated_at: now,
        };

        let coll = self.db.collection::<Extension>("extensions");
        let result = coll.insert_one(&ext, None).await?;

        let mut ext = ext;
        ext.id = result.inserted_id.as_object_id();
        Ok(ext)
    }

    pub async fn list(
        &self,
        team_id: &str,
        page: Option<u64>,
        limit: Option<u64>,
        search: Option<&str>,
        sort: Option<&str>,
    ) -> Result<PaginatedResponse<ExtensionSummary>> {
        let team_oid = ObjectId::parse_str(team_id)?;
        let coll = self.db.collection::<Extension>("extensions");

        let mut filter = doc! { "team_id": team_oid, "is_deleted": { "$ne": true } };

        if let Some(q) = search {
            if !q.is_empty() {
                let escaped = regex::escape(q);
                let re = BsonRegex {
                    pattern: escaped,
                    options: "i".to_string(),
                };
                filter.insert(
                    "$or",
                    vec![
                        doc! { "name": { "$regex": &re } },
                        doc! { "description": { "$regex": &re } },
                    ],
                );
            }
        }

        let total = coll.count_documents(filter.clone(), None).await?;

        let page = page.unwrap_or(1).max(1);
        let limit = limit.unwrap_or(100).min(1000);
        let skip = (page - 1) * limit;

        let sort_doc = match sort.unwrap_or("updated_at") {
            "name" => doc! { "name": 1 },
            "created_at" => doc! { "created_at": -1 },
            "use_count" => doc! { "use_count": -1 },
            _ => doc! { "updated_at": -1 },
        };

        let options = FindOptions::builder()
            .skip(skip)
            .limit(limit as i64)
            .sort(sort_doc)
            .build();

        let cursor = coll.find(filter, options).await?;
        let exts: Vec<Extension> = cursor.try_collect().await?;
        let items: Vec<ExtensionSummary> = exts.into_iter().map(ExtensionSummary::from).collect();

        Ok(PaginatedResponse::new(items, total, page, limit))
    }

    pub async fn delete(&self, ext_id: &str) -> Result<()> {
        let oid = ObjectId::parse_str(ext_id)?;
        let coll = self.db.collection::<Extension>("extensions");
        coll.update_one(
            doc! { "_id": oid },
            doc! { "$set": { "is_deleted": true, "updated_at": bson::DateTime::from_chrono(Utc::now()) } },
            None
        ).await?;
        Ok(())
    }

    pub async fn get(&self, ext_id: &str) -> Result<Option<Extension>> {
        let oid = ObjectId::parse_str(ext_id)?;
        let coll = self.db.collection::<Extension>("extensions");
        Ok(coll
            .find_one(doc! { "_id": oid, "is_deleted": { "$ne": true } }, None)
            .await?)
    }

    pub async fn update(
        &self,
        ext_id: &str,
        name: Option<String>,
        description: Option<String>,
        config: Option<BsonDoc>,
    ) -> Result<Extension> {
        let oid = ObjectId::parse_str(ext_id)?;
        let coll = self.db.collection::<Extension>("extensions");

        let current = self
            .get(ext_id)
            .await?
            .ok_or_else(|| anyhow!("Extension not found"))?;

        let mut set_doc = doc! { "updated_at": bson::DateTime::from_chrono(Utc::now()) };

        if let Some(ref n) = name {
            ResourceNameValidator::validate(n).map_err(|e| anyhow!(e))?;
            if self
                .check_duplicate_name(&current.team_id.to_hex(), n, Some(ext_id))
                .await?
            {
                return Err(anyhow!(
                    "An extension with name '{}' already exists in this team",
                    n
                ));
            }
            set_doc.insert("name", n.clone());
        }
        if let Some(d) = description {
            set_doc.insert("description", d);
        }

        // Config change: bump version and reset security review
        if let Some(c) = config {
            set_doc.insert("config", c);
            let new_version = increment_version(&current.version);
            set_doc.insert("version", new_version);
            set_doc.insert(
                "previous_version_id",
                current.id.map(|id| id.to_hex()).unwrap_or_default(),
            );
            // Reset security review on config change
            set_doc.insert("security_reviewed", false);
            set_doc.insert("security_notes", bson::Bson::Null);
            set_doc.insert("reviewed_by", bson::Bson::Null);
            set_doc.insert("reviewed_at", bson::Bson::Null);
        }

        coll.update_one(doc! { "_id": oid }, doc! { "$set": set_doc }, None)
            .await?;
        self.get(ext_id)
            .await?
            .ok_or_else(|| anyhow!("Extension not found"))
    }

    /// Review an extension (approve/reject)
    pub async fn review_extension(
        &self,
        ext_id: &str,
        approved: bool,
        notes: Option<String>,
        reviewer_id: &str,
    ) -> Result<Extension> {
        let oid = ObjectId::parse_str(ext_id)?;
        let coll = self.db.collection::<Extension>("extensions");

        let now = Utc::now();
        let mut set_doc = doc! {
            "security_reviewed": approved,
            "reviewed_by": reviewer_id,
            "reviewed_at": bson::DateTime::from_chrono(now),
            "updated_at": bson::DateTime::from_chrono(now),
        };
        if let Some(n) = notes {
            set_doc.insert("security_notes", n);
        }

        coll.update_one(doc! { "_id": oid }, doc! { "$set": set_doc }, None)
            .await?;
        self.get(ext_id)
            .await?
            .ok_or_else(|| anyhow!("Extension not found"))
    }

    /// List all security-reviewed, non-deleted extensions for a team.
    /// Used by Agent executor to auto-discover team shared extensions.
    pub async fn list_reviewed_for_team(&self, team_id: &str) -> Result<Vec<Extension>> {
        let team_oid = ObjectId::parse_str(team_id)?;
        let coll = self.db.collection::<Extension>("extensions");
        let filter = doc! {
            "team_id": team_oid,
            "security_reviewed": true,
            "is_deleted": { "$ne": true },
        };
        let cursor = coll.find(filter, None).await?;
        let exts: Vec<Extension> = cursor.try_collect().await?;
        Ok(exts)
    }

    /// List all active (non-deleted) extensions for a team.
    /// Used by Agent executor when auto-extension policy is "all".
    pub async fn list_active_for_team(&self, team_id: &str) -> Result<Vec<Extension>> {
        let team_oid = ObjectId::parse_str(team_id)?;
        let coll = self.db.collection::<Extension>("extensions");
        let filter = doc! {
            "team_id": team_oid,
            "is_deleted": { "$ne": true },
        };
        let cursor = coll.find(filter, None).await?;
        let exts: Vec<Extension> = cursor.try_collect().await?;
        Ok(exts)
    }

    pub async fn increment_use_count(&self, ext_id: &str) -> Result<()> {
        let oid = ObjectId::parse_str(ext_id)?;
        let coll = self.db.collection::<Extension>("extensions");
        coll.update_one(
            doc! { "_id": oid },
            doc! { "$inc": { "use_count": 1 } },
            None,
        )
        .await?;
        Ok(())
    }
}
