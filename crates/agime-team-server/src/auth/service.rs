//! Authentication service for user and API key management

use anyhow::{anyhow, Result};
use argon2::{
    password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use std::sync::Arc;
use uuid::Uuid;

use super::api_key::{extract_key_prefix, generate_api_key, validate_key_format};

/// User entity
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    pub id: String,
    pub email: String,
    pub display_name: String,
    pub created_at: DateTime<Utc>,
    pub last_login_at: Option<DateTime<Utc>>,
    pub is_active: bool,
}

/// API Key entity
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiKey {
    pub id: String,
    pub user_id: String,
    pub key_prefix: String,
    pub name: Option<String>,
    pub last_used_at: Option<DateTime<Utc>>,
    pub expires_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

/// Request to register a new user
#[derive(Debug, Deserialize)]
pub struct RegisterRequest {
    pub email: String,
    pub display_name: String,
}

/// Response after registration
#[derive(Debug, Serialize)]
pub struct RegisterResponse {
    pub user: User,
    pub api_key: String,
}

/// Request to create a new API key
#[derive(Debug, Deserialize)]
pub struct CreateApiKeyRequest {
    pub name: Option<String>,
    pub expires_in_days: Option<u32>,
}

/// Response after creating an API key
#[derive(Debug, Serialize)]
pub struct CreateApiKeyResponse {
    pub id: String,
    pub api_key: String,
    pub name: Option<String>,
    pub expires_at: Option<DateTime<Utc>>,
}

/// Authentication service
pub struct AuthService {
    pool: Arc<SqlitePool>,
}

impl AuthService {
    pub fn new(pool: Arc<SqlitePool>) -> Self {
        Self { pool }
    }

    /// Register a new user and generate an API key
    pub async fn register(&self, request: RegisterRequest) -> Result<RegisterResponse> {
        // Check if email already exists
        let existing = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM users WHERE email = ? AND is_active = 1",
        )
        .bind(&request.email)
        .fetch_one(self.pool.as_ref())
        .await?;

        if existing > 0 {
            return Err(anyhow!("Email already registered"));
        }

        // Create user
        let user_id = Uuid::new_v4().to_string();
        let now = Utc::now();

        sqlx::query(
            r#"
            INSERT INTO users (id, email, display_name, created_at, is_active)
            VALUES (?, ?, ?, ?, 1)
            "#,
        )
        .bind(&user_id)
        .bind(&request.email)
        .bind(&request.display_name)
        .bind(now.to_rfc3339())
        .execute(self.pool.as_ref())
        .await?;

        // Generate API key
        let api_key = generate_api_key(&user_id);
        let key_prefix = extract_key_prefix(&api_key).unwrap_or_default();
        let key_hash = self.hash_key(&api_key)?;

        let key_id = Uuid::new_v4().to_string();
        sqlx::query(
            r#"
            INSERT INTO api_keys (id, user_id, key_prefix, key_hash, name, created_at)
            VALUES (?, ?, ?, ?, 'Default Key', ?)
            "#,
        )
        .bind(&key_id)
        .bind(&user_id)
        .bind(&key_prefix)
        .bind(&key_hash)
        .bind(now.to_rfc3339())
        .execute(self.pool.as_ref())
        .await?;

        let user = User {
            id: user_id,
            email: request.email,
            display_name: request.display_name,
            created_at: now,
            last_login_at: None,
            is_active: true,
        };

        Ok(RegisterResponse { user, api_key })
    }

    /// Verify an API key and return the associated user
    pub async fn verify_api_key(&self, api_key: &str) -> Result<User> {
        if !validate_key_format(api_key) {
            return Err(anyhow!("Invalid API key format"));
        }

        let key_prefix =
            extract_key_prefix(api_key).ok_or_else(|| anyhow!("Invalid key prefix"))?;

        // Find keys matching the prefix
        let keys: Vec<(String, String, String, Option<String>)> = sqlx::query_as(
            r#"
            SELECT k.id, k.user_id, k.key_hash, k.expires_at
            FROM api_keys k
            JOIN users u ON k.user_id = u.id
            WHERE k.key_prefix = ? AND u.is_active = 1
            "#,
        )
        .bind(&key_prefix)
        .fetch_all(self.pool.as_ref())
        .await?;

        for (_key_id, user_id, key_hash, expires_at) in keys {
            // Check expiration
            if let Some(exp) = expires_at {
                let exp_time = DateTime::parse_from_rfc3339(&exp)?;
                if Utc::now() > exp_time {
                    continue;
                }
            }

            // Verify hash
            if self.verify_key(api_key, &key_hash)? {
                // Fetch user
                let user: (String, String, String, String, Option<String>, bool) = sqlx::query_as(
                    r#"
                    SELECT id, email, display_name, created_at, last_login_at, is_active
                    FROM users WHERE id = ?
                    "#,
                )
                .bind(&user_id)
                .fetch_one(self.pool.as_ref())
                .await?;

                return Ok(User {
                    id: user.0,
                    email: user.1,
                    display_name: user.2,
                    created_at: DateTime::parse_from_rfc3339(&user.3)?.with_timezone(&Utc),
                    last_login_at: user
                        .4
                        .and_then(|s| DateTime::parse_from_rfc3339(&s).ok())
                        .map(|dt| dt.with_timezone(&Utc)),
                    is_active: user.5,
                });
            }
        }

        Err(anyhow!("Invalid API key"))
    }

    /// Create a new API key for a user
    pub async fn create_api_key(
        &self,
        user_id: &str,
        request: CreateApiKeyRequest,
    ) -> Result<CreateApiKeyResponse> {
        let api_key = generate_api_key(user_id);
        let key_prefix = extract_key_prefix(&api_key).unwrap_or_default();
        let key_hash = self.hash_key(&api_key)?;

        let key_id = Uuid::new_v4().to_string();
        let now = Utc::now();
        let expires_at = request
            .expires_in_days
            .map(|days| now + chrono::Duration::days(days as i64));

        sqlx::query(
            r#"
            INSERT INTO api_keys (id, user_id, key_prefix, key_hash, name, expires_at, created_at)
            VALUES (?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(&key_id)
        .bind(user_id)
        .bind(&key_prefix)
        .bind(&key_hash)
        .bind(&request.name)
        .bind(expires_at.map(|dt| dt.to_rfc3339()))
        .bind(now.to_rfc3339())
        .execute(self.pool.as_ref())
        .await?;

        Ok(CreateApiKeyResponse {
            id: key_id,
            api_key,
            name: request.name,
            expires_at,
        })
    }

    /// List API keys for a user (without hashes)
    pub async fn list_api_keys(&self, user_id: &str) -> Result<Vec<ApiKey>> {
        let keys: Vec<(
            String,
            String,
            String,
            Option<String>,
            Option<String>,
            Option<String>,
            String,
        )> = sqlx::query_as(
            r#"
            SELECT id, user_id, key_prefix, name, last_used_at, expires_at, created_at
            FROM api_keys
            WHERE user_id = ?
            ORDER BY created_at DESC
            "#,
        )
        .bind(user_id)
        .fetch_all(self.pool.as_ref())
        .await?;

        let result: Vec<ApiKey> = keys
            .into_iter()
            .filter_map(|k| {
                Some(ApiKey {
                    id: k.0,
                    user_id: k.1,
                    key_prefix: k.2,
                    name: k.3,
                    last_used_at: k
                        .4
                        .and_then(|s| DateTime::parse_from_rfc3339(&s).ok())
                        .map(|dt| dt.with_timezone(&Utc)),
                    expires_at: k
                        .5
                        .and_then(|s| DateTime::parse_from_rfc3339(&s).ok())
                        .map(|dt| dt.with_timezone(&Utc)),
                    created_at: DateTime::parse_from_rfc3339(&k.6).ok()?.with_timezone(&Utc),
                })
            })
            .collect();

        Ok(result)
    }

    /// Revoke an API key
    pub async fn revoke_api_key(&self, user_id: &str, key_id: &str) -> Result<()> {
        let result = sqlx::query("DELETE FROM api_keys WHERE id = ? AND user_id = ?")
            .bind(key_id)
            .bind(user_id)
            .execute(self.pool.as_ref())
            .await?;

        if result.rows_affected() == 0 {
            return Err(anyhow!("API key not found"));
        }

        Ok(())
    }

    /// Update last used timestamp for an API key
    pub async fn update_key_last_used(&self, api_key: &str) -> Result<()> {
        let key_prefix =
            extract_key_prefix(api_key).ok_or_else(|| anyhow!("Invalid key prefix"))?;
        let now = Utc::now().to_rfc3339();

        sqlx::query("UPDATE api_keys SET last_used_at = ? WHERE key_prefix = ?")
            .bind(&now)
            .bind(&key_prefix)
            .execute(self.pool.as_ref())
            .await?;

        Ok(())
    }

    /// Hash an API key using Argon2
    fn hash_key(&self, key: &str) -> Result<String> {
        let salt = SaltString::generate(&mut OsRng);
        let argon2 = Argon2::default();
        let hash = argon2
            .hash_password(key.as_bytes(), &salt)
            .map_err(|e| anyhow!("Failed to hash key: {}", e))?;
        Ok(hash.to_string())
    }

    /// Verify an API key against a hash
    fn verify_key(&self, key: &str, hash: &str) -> Result<bool> {
        let parsed_hash =
            PasswordHash::new(hash).map_err(|e| anyhow!("Invalid hash format: {}", e))?;
        Ok(Argon2::default()
            .verify_password(key.as_bytes(), &parsed_hash)
            .is_ok())
    }
}
