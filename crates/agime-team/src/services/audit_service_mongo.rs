//! Audit log service for MongoDB

use crate::db::MongoDb;
use crate::models::mongo::{AuditLog, AuditLogSummary, PaginatedResponse};
use anyhow::Result;
use chrono::Utc;
use futures::TryStreamExt;
use mongodb::bson::{doc, oid::ObjectId};
use mongodb::options::FindOptions;

pub struct AuditService {
    db: MongoDb,
}

impl AuditService {
    pub fn new(db: MongoDb) -> Self {
        Self { db }
    }

    /// Log an audit event
    pub async fn log(
        &self,
        team_id: &str,
        user_id: &str,
        user_name: Option<&str>,
        action: &str,
        resource_type: &str,
        resource_id: Option<&str>,
        resource_name: Option<&str>,
        details: Option<&str>,
    ) -> Result<()> {
        let team_oid = ObjectId::parse_str(team_id)?;
        let entry = AuditLog {
            id: None,
            team_id: team_oid,
            user_id: user_id.to_string(),
            user_name: user_name.map(|s| s.to_string()),
            action: action.to_string(),
            resource_type: resource_type.to_string(),
            resource_id: resource_id.map(|s| s.to_string()),
            resource_name: resource_name.map(|s| s.to_string()),
            details: details.map(|s| s.to_string()),
            ip_address: None,
            created_at: Utc::now(),
        };

        let coll = self.db.collection::<AuditLog>("audit_logs");
        coll.insert_one(&entry, None).await?;
        Ok(())
    }

    /// Query audit logs with pagination
    pub async fn query(
        &self,
        team_id: &str,
        action: Option<&str>,
        resource_type: Option<&str>,
        user_id: Option<&str>,
        page: Option<u64>,
        limit: Option<u64>,
    ) -> Result<PaginatedResponse<AuditLogSummary>> {
        let team_oid = ObjectId::parse_str(team_id)?;
        let coll = self.db.collection::<AuditLog>("audit_logs");

        let mut filter = doc! { "team_id": team_oid };
        if let Some(a) = action {
            filter.insert("action", a);
        }
        if let Some(rt) = resource_type {
            filter.insert("resource_type", rt);
        }
        if let Some(uid) = user_id {
            filter.insert("user_id", uid);
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
        let logs: Vec<AuditLog> = cursor.try_collect().await?;
        let items = logs.into_iter().map(AuditLogSummary::from).collect();

        Ok(PaginatedResponse::new(items, total, page, limit))
    }

    /// Get recent team activity (last N entries)
    pub async fn get_team_activity(
        &self,
        team_id: &str,
        limit: Option<u64>,
    ) -> Result<Vec<AuditLogSummary>> {
        let team_oid = ObjectId::parse_str(team_id)?;
        let coll = self.db.collection::<AuditLog>("audit_logs");
        let limit = limit.unwrap_or(20).min(100);

        let options = FindOptions::builder()
            .limit(limit as i64)
            .sort(doc! { "created_at": -1 })
            .build();

        let cursor = coll.find(doc! { "team_id": team_oid }, options).await?;
        let logs: Vec<AuditLog> = cursor.try_collect().await?;
        Ok(logs.into_iter().map(AuditLogSummary::from).collect())
    }
}
