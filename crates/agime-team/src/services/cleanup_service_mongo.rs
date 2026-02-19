//! Cleanup service for MongoDB
//! Handles purging soft-deleted resources and expired data

use crate::db::MongoDb;
use anyhow::Result;
use chrono::{Duration, Utc};
use mongodb::bson::{doc, Document as BsonDoc};
use serde::Serialize;

/// Report of cleanup operation.
/// Note: documents are excluded — document cleanup is handled by the archive mechanism.
#[derive(Debug, Clone, Default, Serialize)]
pub struct CleanupReport {
    pub skills: u64,
    pub recipes: u64,
    pub extensions: u64,
    pub folders: u64,
    pub stale_analyses: u64,
}

pub struct CleanupService {
    db: MongoDb,
}

impl CleanupService {
    pub fn new(db: MongoDb) -> Self {
        Self { db }
    }

    /// Purge soft-deleted resources older than given days
    pub async fn purge_deleted(&self, days_old: i64) -> Result<CleanupReport> {
        let cutoff = Utc::now() - Duration::days(days_old);
        let cutoff_bson = bson::DateTime::from_chrono(cutoff);

        let filter = doc! {
            "is_deleted": true,
            "updated_at": { "$lt": cutoff_bson }
        };

        let mut report = CleanupReport::default();

        for collection in &["skills", "recipes", "extensions", "folders"] {
            let coll = self.db.collection::<BsonDoc>(collection);
            let result = coll.delete_many(filter.clone(), None).await?;
            match *collection {
                "skills" => report.skills = result.deleted_count,
                "recipes" => report.recipes = result.deleted_count,
                "extensions" => report.extensions = result.deleted_count,
                "folders" => report.folders = result.deleted_count,
                _ => {}
            }
        }

        Ok(report)
    }

    /// Cancel smart_log AI analyses stuck in "pending" for over `hours` hours
    pub async fn cancel_stale_analyses(&self, hours: i64) -> Result<u64> {
        let cutoff = Utc::now() - Duration::hours(hours);
        let cutoff_bson = bson::DateTime::from_chrono(cutoff);

        let coll = self.db.collection::<BsonDoc>("smart_logs");
        let result = coll
            .update_many(
                doc! {
                    "ai_analysis_status": "pending",
                    "created_at": { "$lt": cutoff_bson }
                },
                doc! { "$set": { "ai_analysis_status": "cancelled" } },
                None,
            )
            .await?;
        Ok(result.modified_count)
    }
}
