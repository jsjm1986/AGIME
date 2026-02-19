//! Smart Log service for MongoDB

use crate::db::MongoDb;
use crate::models::mongo::{
    build_fallback_summary, PaginatedResponse, SmartLogContext, SmartLogEntry, SmartLogSummary,
};
use anyhow::Result;
use chrono::Utc;
use futures::TryStreamExt;
use mongodb::bson::{doc, oid::ObjectId};
use mongodb::options::FindOptions;

pub struct SmartLogService {
    db: MongoDb,
}

impl SmartLogService {
    pub fn new(db: MongoDb) -> Self {
        Self { db }
    }

    /// Create a new smart log entry (initially pending)
    pub async fn create(&self, ctx: &SmartLogContext) -> Result<String> {
        let team_oid = ObjectId::parse_str(&ctx.team_id)?;
        let now = Utc::now();

        // Truncate content snapshot to 2000 chars
        let content_snapshot = ctx
            .content_for_ai
            .as_ref()
            .map(|c| c.chars().take(2000).collect::<String>());

        let status = if ctx.content_for_ai.is_some() {
            "pending"
        } else {
            "skipped"
        };

        let source = ctx.source.clone().unwrap_or_else(|| "human".to_string());

        let entry = SmartLogEntry {
            id: None,
            team_id: team_oid,
            user_id: ctx.user_id.clone(),
            user_name: ctx.user_name.clone(),
            action: ctx.action.clone(),
            resource_type: ctx.resource_type.clone(),
            resource_id: Some(ctx.resource_id.clone()),
            resource_name: Some(ctx.resource_name.clone()),
            ai_summary: if ctx.content_for_ai.is_none() {
                Some(build_fallback_summary(&ctx.action, &ctx.resource_type, &ctx.resource_name))
            } else {
                None
            },
            ai_summary_status: status.to_string(),
            content_snapshot,
            source,
            ai_analysis: None,
            ai_analysis_status: if ctx.has_pending_analysis {
                Some("pending".to_string())
            } else {
                None
            },
            created_at: now,
            ai_completed_at: if ctx.content_for_ai.is_none() {
                Some(now)
            } else {
                None
            },
        };

        let coll = self.db.collection::<SmartLogEntry>("smart_logs");
        let result = coll.insert_one(entry, None).await?;
        let id = result
            .inserted_id
            .as_object_id()
            .map(|oid| oid.to_hex())
            .unwrap_or_default();
        Ok(id)
    }

    /// Update AI summary after LLM completion
    pub async fn update_ai_summary(&self, id: &str, summary: &str, status: &str) -> Result<()> {
        let oid = ObjectId::parse_str(id)?;
        let coll = self.db.collection::<SmartLogEntry>("smart_logs");
        coll.update_one(
            doc! { "_id": oid },
            doc! {
                "$set": {
                    "ai_summary": summary,
                    "ai_summary_status": status,
                    "ai_completed_at": bson::DateTime::from_chrono(Utc::now()),
                }
            },
            None,
        )
        .await?;
        Ok(())
    }

    /// Query smart logs with pagination and optional resource_type/action/source filter
    pub async fn query(
        &self,
        team_id: &str,
        resource_type: Option<&str>,
        action: Option<&str>,
        source: Option<&str>,
        user_id: Option<&str>,
        page: Option<u64>,
        limit: Option<u64>,
    ) -> Result<PaginatedResponse<SmartLogSummary>> {
        let team_oid = ObjectId::parse_str(team_id)?;
        let page = page.unwrap_or(1).max(1);
        let limit = limit.unwrap_or(20).min(100);
        let skip = (page - 1) * limit;

        let mut filter = doc! { "team_id": team_oid };
        for (key, value) in [
            ("resource_type", resource_type),
            ("action", action),
            ("source", source),
            ("user_id", user_id),
        ] {
            if let Some(v) = value.filter(|s| !s.is_empty()) {
                filter.insert(key, v);
            }
        }

        let coll = self.db.collection::<SmartLogEntry>("smart_logs");
        let total = coll.count_documents(filter.clone(), None).await?;

        let options = FindOptions::builder()
            .sort(doc! { "created_at": -1 })
            .skip(skip)
            .limit(limit as i64)
            .build();

        let cursor = coll.find(filter, options).await?;
        let entries: Vec<SmartLogEntry> = cursor.try_collect().await?;
        let items: Vec<SmartLogSummary> = entries.into_iter().map(SmartLogSummary::from).collect();

        Ok(PaginatedResponse::new(items, total, page, limit))
    }

    /// Attach AI analysis result to the most recent smart log entry for a resource
    pub async fn attach_analysis(
        &self,
        team_id: &str,
        resource_id: &str,
        analysis: &str,
        status: &str,
    ) -> Result<bool> {
        let team_oid = ObjectId::parse_str(team_id)?;
        let coll = self.db.collection::<SmartLogEntry>("smart_logs");

        let filter = doc! {
            "team_id": team_oid,
            "resource_id": resource_id,
            "action": { "$ne": "delete" },
        };
        let options = mongodb::options::FindOneOptions::builder()
            .sort(doc! { "created_at": -1 })
            .build();

        let entry = coll.find_one(filter, options).await?;
        match entry {
            Some(e) => {
                let oid = e.id.ok_or_else(|| anyhow::anyhow!("Entry has no id"))?;
                coll.update_one(
                    doc! { "_id": oid },
                    doc! {
                        "$set": {
                            "ai_analysis": analysis,
                            "ai_analysis_status": status,
                        }
                    },
                    None,
                )
                .await?;
                Ok(true)
            }
            None => Ok(false),
        }
    }
    /// Cancel all pending AI analysis entries for a given resource (e.g. when document is deleted)
    pub async fn cancel_pending_analysis(&self, team_id: &str, resource_id: &str) -> Result<u64> {
        let team_oid = ObjectId::parse_str(team_id)?;
        let coll = self.db.collection::<SmartLogEntry>("smart_logs");
        let result = coll
            .update_many(
                doc! {
                    "team_id": team_oid,
                    "resource_id": resource_id,
                    "ai_analysis_status": "pending",
                },
                doc! {
                    "$set": { "ai_analysis_status": "cancelled" }
                },
                None,
            )
            .await?;
        Ok(result.modified_count)
    }

    /// Get recent smart logs (convenience method, no pagination)
    pub async fn get_recent(
        &self,
        team_id: &str,
        limit: Option<u64>,
    ) -> Result<Vec<SmartLogSummary>> {
        let team_oid = ObjectId::parse_str(team_id)?;
        let limit = limit.unwrap_or(10).min(50);

        let coll = self.db.collection::<SmartLogEntry>("smart_logs");
        let options = FindOptions::builder()
            .sort(doc! { "created_at": -1 })
            .limit(limit as i64)
            .build();

        let cursor = coll.find(doc! { "team_id": team_oid }, options).await?;
        let entries: Vec<SmartLogEntry> = cursor.try_collect().await?;
        Ok(entries.into_iter().map(SmartLogSummary::from).collect())
    }
}
