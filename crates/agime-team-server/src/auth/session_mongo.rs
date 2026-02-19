//! Session management service (MongoDB version)

use agime_team::MongoDb;
use anyhow::{anyhow, Result};
use chrono::{DateTime, Duration, Utc};
use mongodb::bson::{doc, oid::ObjectId};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

use super::service_mongo::{AuthService, UserResponse};

/// Session document
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionDoc {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub session_id: String,
    pub user_id: String,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub expires_at: DateTime<Utc>,
}

/// Session entity (for API response)
#[derive(Debug, Clone, Serialize)]
pub struct Session {
    pub id: String,
    pub user_id: String,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

/// Session service for web authentication (MongoDB)
pub struct SessionService {
    db: Arc<MongoDb>,
}

impl SessionService {
    pub fn new(db: Arc<MongoDb>) -> Self {
        Self { db }
    }

    fn sessions(&self) -> mongodb::Collection<SessionDoc> {
        self.db.collection("sessions")
    }

    fn users(&self) -> mongodb::Collection<super::service_mongo::User> {
        self.db.collection("users")
    }

    /// Create a new session for a user (login with API key)
    pub async fn create_session(&self, api_key: &str) -> Result<(Session, UserResponse)> {
        // Verify API key first
        let auth_service = AuthService::new(self.db.clone());
        let (user, key_id) = auth_service.verify_api_key(api_key).await?;

        // Update key last used (precise by key_id)
        let _ = auth_service.update_key_last_used_by_id(&key_id).await;

        // Create session
        let session_id = Uuid::new_v4().to_string();
        let now = Utc::now();
        let expires_at = now + Duration::days(7); // 7 day session

        let session_doc = SessionDoc {
            id: None,
            session_id: session_id.clone(),
            user_id: user.id.clone(),
            created_at: now,
            expires_at,
        };

        self.sessions().insert_one(&session_doc, None).await?;

        // Update last login
        self.users()
            .update_one(
                doc! { "user_id": &user.id },
                doc! { "$set": { "last_login_at": bson::DateTime::from_chrono(now) } },
                None,
            )
            .await?;

        let session = Session {
            id: session_id,
            user_id: user.id.clone(),
            created_at: now,
            expires_at,
        };

        Ok((session, user))
    }

    /// Create a session directly from a UserResponse (for password login)
    pub async fn create_session_for_user(&self, user: &UserResponse) -> Result<Session> {
        let session_id = Uuid::new_v4().to_string();
        let now = Utc::now();
        let expires_at = now + Duration::days(7);

        let session_doc = SessionDoc {
            id: None,
            session_id: session_id.clone(),
            user_id: user.id.clone(),
            created_at: now,
            expires_at,
        };

        self.sessions().insert_one(&session_doc, None).await?;

        // Update last login
        self.users()
            .update_one(
                doc! { "user_id": &user.id },
                doc! { "$set": { "last_login_at": bson::DateTime::from_chrono(now) } },
                None,
            )
            .await?;

        Ok(Session {
            id: session_id,
            user_id: user.id.clone(),
            created_at: now,
            expires_at,
        })
    }

    /// Validate a session and return the user
    pub async fn validate_session(&self, session_id: &str) -> Result<UserResponse> {
        let session_doc = self
            .sessions()
            .find_one(doc! { "session_id": session_id }, None)
            .await?
            .ok_or_else(|| anyhow!("Session not found"))?;

        // Check expiration
        if Utc::now() > session_doc.expires_at {
            self.delete_session(session_id).await?;
            return Err(anyhow!("Session expired"));
        }

        // Fetch user
        let user = self
            .users()
            .find_one(
                doc! { "user_id": &session_doc.user_id, "is_active": true },
                None,
            )
            .await?
            .ok_or_else(|| anyhow!("User not found"))?;

        Ok(user.into())
    }

    /// Delete a session (logout)
    pub async fn delete_session(&self, session_id: &str) -> Result<()> {
        self.sessions()
            .delete_one(doc! { "session_id": session_id }, None)
            .await?;
        Ok(())
    }

    /// Clean up expired sessions
    #[allow(dead_code)]
    pub async fn cleanup_expired(&self) -> Result<u64> {
        let now = bson::DateTime::from_chrono(Utc::now());
        let result = self
            .sessions()
            .delete_many(doc! { "expires_at": { "$lt": now } }, None)
            .await?;
        Ok(result.deleted_count)
    }

    /// Extend a session's expiration (sliding window)
    pub async fn extend_session(&self, session_id: &str, days: i64) -> Result<()> {
        let new_expires = Utc::now() + Duration::days(days);
        self.sessions()
            .update_one(
                doc! { "session_id": session_id },
                doc! { "$set": {
                    "expires_at": bson::DateTime::from_chrono(new_expires)
                }},
                None,
            )
            .await?;
        Ok(())
    }

    /// Try to extend a session if remaining time is below threshold
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

    /// Delete all sessions for a user
    pub async fn delete_user_sessions(&self, user_id: &str) -> Result<u64> {
        let result = self
            .sessions()
            .delete_many(doc! { "user_id": user_id }, None)
            .await?;
        Ok(result.deleted_count)
    }
}
