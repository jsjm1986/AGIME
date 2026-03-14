//! Dedicated session management for the isolated system-admin console.

use agime_team::MongoDb;
use anyhow::{anyhow, Result};
use chrono::{DateTime, Duration, Utc};
use mongodb::bson::{doc, oid::ObjectId};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

use super::service_mongo::{SystemAdmin, SystemAdminResponse};

/// Separate cookie name for system-admin authentication.
pub const SYSTEM_ADMIN_SESSION_COOKIE_NAME: &str = "agime_system_admin_session";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemAdminSessionDoc {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub session_id: String,
    pub admin_id: String,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SystemAdminSession {
    pub id: String,
    pub admin_id: String,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

pub struct SystemAdminSessionService {
    db: Arc<MongoDb>,
}

impl SystemAdminSessionService {
    pub fn new(db: Arc<MongoDb>) -> Self {
        Self { db }
    }

    fn sessions(&self) -> mongodb::Collection<SystemAdminSessionDoc> {
        self.db.collection("system_admin_sessions")
    }

    fn system_admins(&self) -> mongodb::Collection<SystemAdmin> {
        self.db.collection("system_admins")
    }

    pub async fn create_session_for_admin(
        &self,
        admin: &SystemAdminResponse,
    ) -> Result<SystemAdminSession> {
        let session_id = Uuid::new_v4().to_string();
        let now = Utc::now();
        let expires_at = now + Duration::days(7);

        let session_doc = SystemAdminSessionDoc {
            id: None,
            session_id: session_id.clone(),
            admin_id: admin.id.clone(),
            created_at: now,
            expires_at,
        };

        self.sessions().insert_one(&session_doc, None).await?;

        Ok(SystemAdminSession {
            id: session_id,
            admin_id: admin.id.clone(),
            created_at: now,
            expires_at,
        })
    }

    pub async fn validate_session(&self, session_id: &str) -> Result<SystemAdminResponse> {
        let session_doc = self
            .sessions()
            .find_one(doc! { "session_id": session_id }, None)
            .await?
            .ok_or_else(|| anyhow!("Session not found"))?;

        if Utc::now() > session_doc.expires_at {
            self.delete_session(session_id).await?;
            return Err(anyhow!("Session expired"));
        }

        let admin = self
            .system_admins()
            .find_one(
                doc! { "admin_id": &session_doc.admin_id, "is_active": true },
                None,
            )
            .await?
            .ok_or_else(|| anyhow!("System admin not found"))?;

        Ok(admin.into())
    }

    pub async fn delete_session(&self, session_id: &str) -> Result<()> {
        self.sessions()
            .delete_one(doc! { "session_id": session_id }, None)
            .await?;
        Ok(())
    }

    pub async fn cleanup_expired(&self) -> Result<u64> {
        let now = bson::DateTime::from_chrono(Utc::now());
        let result = self
            .sessions()
            .delete_many(doc! { "expires_at": { "$lt": now } }, None)
            .await?;
        Ok(result.deleted_count)
    }

    pub async fn extend_session(&self, session_id: &str, days: i64) -> Result<()> {
        let new_expires = Utc::now() + Duration::days(days);
        self.sessions()
            .update_one(
                doc! { "session_id": session_id },
                doc! { "$set": { "expires_at": bson::DateTime::from_chrono(new_expires) } },
                None,
            )
            .await?;
        Ok(())
    }

    pub async fn try_extend_session(
        &self,
        session_id: &str,
        threshold_hours: i64,
        extend_days: i64,
    ) -> Result<()> {
        let session_doc = self
            .sessions()
            .find_one(doc! { "session_id": session_id }, None)
            .await?;

        if let Some(doc) = session_doc {
            let remaining = doc.expires_at - Utc::now();
            if remaining < Duration::hours(threshold_hours) {
                self.extend_session(session_id, extend_days).await?;
            }
        }

        Ok(())
    }
}
