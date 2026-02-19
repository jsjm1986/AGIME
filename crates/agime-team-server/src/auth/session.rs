//! Session management service

use anyhow::{anyhow, Result};
use chrono::{DateTime, Duration, Utc};
use sqlx::SqlitePool;
use std::sync::Arc;
use uuid::Uuid;

use super::service_sqlite::{AuthService, User};

/// Session entity
#[derive(Debug, Clone)]
pub struct Session {
    pub id: String,
    pub user_id: String,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

/// Session service for web authentication
pub struct SessionService {
    pool: Arc<SqlitePool>,
}

impl SessionService {
    pub fn new(pool: Arc<SqlitePool>) -> Self {
        Self { pool }
    }

    /// Create a new session for a user (login with API key)
    pub async fn create_session(&self, api_key: &str) -> Result<(Session, User)> {
        // Verify API key first
        let auth_service = AuthService::new(self.pool.clone());
        let user = auth_service.verify_api_key(api_key).await?;

        // Create session
        let session_id = Uuid::new_v4().to_string();
        let now = Utc::now();
        let expires_at = now + Duration::days(7); // 7 day session

        sqlx::query(
            "INSERT INTO sessions (id, user_id, created_at, expires_at) VALUES (?, ?, ?, ?)",
        )
        .bind(&session_id)
        .bind(&user.id)
        .bind(now.to_rfc3339())
        .bind(expires_at.to_rfc3339())
        .execute(self.pool.as_ref())
        .await?;

        // Update last login
        sqlx::query("UPDATE users SET last_login_at = ? WHERE id = ?")
            .bind(now.to_rfc3339())
            .bind(&user.id)
            .execute(self.pool.as_ref())
            .await?;

        let session = Session {
            id: session_id,
            user_id: user.id.clone(),
            created_at: now,
            expires_at,
        };

        Ok((session, user))
    }

    /// Validate a session and return the user
    pub async fn validate_session(&self, session_id: &str) -> Result<User> {
        let row: Option<(String, String, String)> =
            sqlx::query_as("SELECT user_id, created_at, expires_at FROM sessions WHERE id = ?")
                .bind(session_id)
                .fetch_optional(self.pool.as_ref())
                .await?;

        let (user_id, _, expires_at_str) = row.ok_or_else(|| anyhow!("Session not found"))?;

        // Check expiration
        let expires_at = DateTime::parse_from_rfc3339(&expires_at_str)?;
        if Utc::now() > expires_at {
            self.delete_session(session_id).await?;
            return Err(anyhow!("Session expired"));
        }

        // Fetch user
        let user: (String, String, String, String, Option<String>, bool) = sqlx::query_as(
            "SELECT id, email, display_name, created_at, last_login_at, is_active FROM users WHERE id = ? AND is_active = 1",
        )
        .bind(&user_id)
        .fetch_one(self.pool.as_ref())
        .await?;

        Ok(User {
            id: user.0,
            email: user.1,
            display_name: user.2,
            created_at: DateTime::parse_from_rfc3339(&user.3)?.with_timezone(&Utc),
            last_login_at: user
                .4
                .and_then(|s| DateTime::parse_from_rfc3339(&s).ok())
                .map(|dt| dt.with_timezone(&Utc)),
            is_active: user.5,
        })
    }

    /// Delete a session (logout)
    pub async fn delete_session(&self, session_id: &str) -> Result<()> {
        sqlx::query("DELETE FROM sessions WHERE id = ?")
            .bind(session_id)
            .execute(self.pool.as_ref())
            .await?;
        Ok(())
    }

    /// Clean up expired sessions
    pub async fn cleanup_expired(&self) -> Result<u64> {
        let now = Utc::now().to_rfc3339();
        let result = sqlx::query("DELETE FROM sessions WHERE expires_at < ?")
            .bind(&now)
            .execute(self.pool.as_ref())
            .await?;
        Ok(result.rows_affected())
    }
}
