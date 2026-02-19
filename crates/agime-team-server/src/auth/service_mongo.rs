//! Authentication service for user and API key management (MongoDB version)

use agime_team::MongoDb;
use anyhow::{anyhow, Result};
use argon2::{
    password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};
use chrono::{DateTime, Utc};
use mongodb::bson::{doc, oid::ObjectId};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

use super::api_key::{extract_key_prefix, generate_api_key, validate_key_format};

/// Custom serde module for Option<DateTime<Utc>> with BSON datetime
mod bson_datetime_option {
    use chrono::{DateTime, Utc};
    use serde::{self, Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S>(date: &Option<DateTime<Utc>>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match date {
            Some(dt) => {
                let bson_dt = bson::DateTime::from_chrono(*dt);
                Serialize::serialize(&bson_dt, serializer)
            }
            None => serializer.serialize_none(),
        }
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Option<DateTime<Utc>>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let opt: Option<bson::DateTime> = Option::deserialize(deserializer)?;
        Ok(opt.map(|dt| dt.to_chrono()))
    }
}

/// User entity
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub user_id: String,
    pub email: String,
    pub display_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub password_hash: Option<String>,
    #[serde(default = "default_role")]
    pub role: String,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "bson_datetime_option"
    )]
    pub last_login_at: Option<DateTime<Utc>>,
    #[serde(default = "default_true")]
    pub is_active: bool,
}

fn default_role() -> String {
    "user".to_string()
}

fn default_true() -> bool {
    true
}

/// API Key entity
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiKeyDoc {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub key_id: String,
    pub user_id: String,
    pub key_prefix: String,
    pub key_hash: String,
    pub name: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "bson_datetime_option"
    )]
    pub last_used_at: Option<DateTime<Utc>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "bson_datetime_option"
    )]
    pub expires_at: Option<DateTime<Utc>>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
}

/// API Key response (without hash)
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
    pub password: Option<String>,
}

/// Response after registration
#[derive(Debug, Serialize)]
pub struct RegisterResponse {
    pub user: UserResponse,
    pub api_key: String,
}

/// User response (for API)
#[derive(Debug, Clone, Serialize)]
pub struct UserResponse {
    pub id: String,
    pub email: String,
    pub display_name: String,
    pub role: String,
    pub created_at: DateTime<Utc>,
    pub last_login_at: Option<DateTime<Utc>>,
    pub is_active: bool,
}

impl From<User> for UserResponse {
    fn from(u: User) -> Self {
        Self {
            id: u.user_id,
            email: u.email,
            display_name: u.display_name,
            role: u.role,
            created_at: u.created_at,
            last_login_at: u.last_login_at,
            is_active: u.is_active,
        }
    }
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

/// Registration request document (for approval mode)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistrationRequestDoc {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub request_id: String,
    pub email: String,
    pub display_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub password_hash: Option<String>,
    pub status: String, // "pending" | "approved" | "rejected"
    pub reviewed_by: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "bson_datetime_option"
    )]
    pub reviewed_at: Option<DateTime<Utc>>,
    pub reject_reason: Option<String>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
}

/// Auth audit log entry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthAuditLog {
    pub action: String,
    pub user_id: Option<String>,
    pub email: Option<String>,
    pub ip_address: Option<String>,
    pub details: Option<String>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
}

/// Login guard - tracks failed login attempts per email
pub struct LoginGuard {
    failures: RwLock<HashMap<String, (u32, Instant)>>,
    max_failures: u32,
    lockout_duration: Duration,
}

impl LoginGuard {
    pub fn new(max_failures: u32, lockout_minutes: u32) -> Self {
        Self {
            failures: RwLock::new(HashMap::new()),
            max_failures,
            lockout_duration: Duration::from_secs(lockout_minutes as u64 * 60),
        }
    }

    /// Check if the email is currently locked out.
    /// Returns Ok(()) if allowed, Err(remaining_secs) if locked.
    pub async fn check_locked(&self, email: &str) -> std::result::Result<(), u64> {
        let failures = self.failures.read().await;
        if let Some((count, last_failure)) = failures.get(email) {
            if *count >= self.max_failures {
                let elapsed = last_failure.elapsed();
                if elapsed < self.lockout_duration {
                    let remaining = (self.lockout_duration - elapsed).as_secs();
                    return Err(remaining);
                }
            }
        }
        Ok(())
    }

    /// Record a failed login attempt. Returns the current failure count.
    pub async fn record_failure(&self, email: &str) -> u32 {
        let mut failures = self.failures.write().await;
        // Cleanup expired entries when map grows beyond threshold
        if failures.len() > 1000 {
            let lockout = self.lockout_duration;
            failures.retain(|_, (_, t)| t.elapsed() < lockout);
        }
        let entry = failures
            .entry(email.to_string())
            .or_insert((0, Instant::now()));
        // Reset if lockout period has passed
        if entry.1.elapsed() >= self.lockout_duration {
            entry.0 = 0;
        }
        entry.0 += 1;
        entry.1 = Instant::now();
        entry.0
    }

    /// Clear failure count on successful login.
    pub async fn clear(&self, email: &str) {
        let mut failures = self.failures.write().await;
        failures.remove(email);
    }
}

/// Validate and normalize an email address
pub fn validate_and_normalize_email(email: &str) -> Result<String> {
    let email = email.trim().to_lowercase();
    if email.len() > 254 {
        return Err(anyhow!("Invalid email format"));
    }
    if !email.contains('@') {
        return Err(anyhow!("Invalid email format"));
    }
    let parts: Vec<&str> = email.split('@').collect();
    if parts.len() != 2 || parts[0].is_empty() || parts[1].is_empty() {
        return Err(anyhow!("Invalid email format"));
    }
    if !parts[1].contains('.') {
        return Err(anyhow!("Invalid email format"));
    }
    Ok(email)
}

/// Authentication service (MongoDB)
pub struct AuthService {
    db: Arc<MongoDb>,
    admin_emails: Vec<String>,
}

impl AuthService {
    pub fn new(db: Arc<MongoDb>) -> Self {
        Self { db, admin_emails: Vec::new() }
    }

    pub fn with_admin_emails(mut self, emails: Vec<String>) -> Self {
        self.admin_emails = emails;
        self
    }

    fn users(&self) -> mongodb::Collection<User> {
        self.db.collection("users")
    }

    fn api_keys(&self) -> mongodb::Collection<ApiKeyDoc> {
        self.db.collection("api_keys")
    }

    fn registration_requests(&self) -> mongodb::Collection<RegistrationRequestDoc> {
        self.db.collection("registration_requests")
    }

    fn audit_logs(&self) -> mongodb::Collection<AuthAuditLog> {
        self.db.collection("auth_audit_logs")
    }

    /// Log an audit event (public). Failures are logged but do not block the main flow.
    pub async fn log_audit_public(
        &self,
        action: &str,
        user_id: Option<&str>,
        email: Option<&str>,
        ip_address: Option<&str>,
        details: Option<&str>,
    ) {
        let log = AuthAuditLog {
            action: action.to_string(),
            user_id: user_id.map(|s| s.to_string()),
            email: email.map(|s| s.to_string()),
            ip_address: ip_address.map(|s| s.to_string()),
            details: details.map(|s| s.to_string()),
            created_at: Utc::now(),
        };
        if let Err(e) = self.audit_logs().insert_one(&log, None).await {
            tracing::warn!("Failed to write auth audit log: {}", e);
        }
    }

    /// Determine role for a new user
    async fn determine_role(&self, email: &str) -> Result<String> {
        // Admin if email is in admin_emails list
        if self.admin_emails.iter().any(|e| e == email) {
            return Ok("admin".to_string());
        }
        // First user in the system becomes admin
        let count = self.users().count_documents(doc! {}, None).await?;
        if count == 0 {
            return Ok("admin".to_string());
        }
        Ok("user".to_string())
    }

    /// Register a new user and generate an API key
    pub async fn register(&self, request: RegisterRequest) -> Result<RegisterResponse> {
        let email = validate_and_normalize_email(&request.email)?;

        let existing = self
            .users()
            .find_one(doc! { "email": &email, "is_active": true }, None)
            .await?;
        if existing.is_some() {
            return Err(anyhow!("Email already registered"));
        }

        // Hash password if provided
        let password_hash = match &request.password {
            Some(pw) if !pw.is_empty() => {
                if pw.len() < 8 {
                    return Err(anyhow!("Password must be at least 8 characters"));
                }
                Some(self.hash_password(pw)?)
            }
            _ => None,
        };

        let role = self.determine_role(&email).await?;
        let user_id = uuid::Uuid::new_v4().to_string();
        let now = Utc::now();

        let user = User {
            id: None,
            user_id: user_id.clone(),
            email: email.clone(),
            display_name: request.display_name.clone(),
            password_hash,
            role: role.clone(),
            created_at: now,
            last_login_at: None,
            is_active: true,
        };

        self.users().insert_one(&user, None).await?;

        // Generate API key
        let api_key = generate_api_key(&user_id);
        let key_prefix =
            extract_key_prefix(&api_key).ok_or_else(|| anyhow!("Failed to extract key prefix"))?;
        let key_hash = self.hash_key(&api_key)?;

        let key_id = uuid::Uuid::new_v4().to_string();
        let api_key_doc = ApiKeyDoc {
            id: None,
            key_id: key_id.clone(),
            user_id: user_id.clone(),
            key_prefix,
            key_hash,
            name: Some("Default Key".to_string()),
            last_used_at: None,
            expires_at: None,
            created_at: now,
        };

        self.api_keys().insert_one(&api_key_doc, None).await?;

        self.log_audit_public("register", Some(&user_id), Some(&email), None, None)
            .await;

        Ok(RegisterResponse {
            user: UserResponse {
                id: user_id,
                email,
                display_name: request.display_name,
                role,
                created_at: now,
                last_login_at: None,
                is_active: true,
            },
            api_key,
        })
    }

    /// Verify an API key and return the associated user + key_id
    pub async fn verify_api_key(&self, api_key: &str) -> Result<(UserResponse, String)> {
        if !validate_key_format(api_key) {
            return Err(anyhow!("Invalid API key"));
        }

        let key_prefix = extract_key_prefix(api_key).ok_or_else(|| anyhow!("Invalid API key"))?;

        use futures::TryStreamExt;
        let mut cursor = self
            .api_keys()
            .find(doc! { "key_prefix": &key_prefix }, None)
            .await?;

        while let Some(key_doc) = cursor.try_next().await? {
            if let Some(exp) = key_doc.expires_at {
                if Utc::now() > exp {
                    continue;
                }
            }

            if self.verify_key(api_key, &key_doc.key_hash)? {
                let user = self
                    .users()
                    .find_one(
                        doc! { "user_id": &key_doc.user_id, "is_active": true },
                        None,
                    )
                    .await?
                    .ok_or_else(|| anyhow!("User not found or deactivated"))?;

                return Ok((user.into(), key_doc.key_id));
            }
        }

        Err(anyhow!("Invalid API key"))
    }

    /// Create a new API key for a user (with count limit)
    pub async fn create_api_key(
        &self,
        user_id: &str,
        request: CreateApiKeyRequest,
        max_keys: u32,
    ) -> Result<CreateApiKeyResponse> {
        // Check key count limit
        let existing_count = self
            .api_keys()
            .count_documents(doc! { "user_id": user_id }, None)
            .await?;
        if existing_count >= max_keys as u64 {
            return Err(anyhow!("Maximum API key limit reached"));
        }

        let api_key = generate_api_key(user_id);
        let key_prefix =
            extract_key_prefix(&api_key).ok_or_else(|| anyhow!("Failed to extract key prefix"))?;
        let key_hash = self.hash_key(&api_key)?;

        let key_id = uuid::Uuid::new_v4().to_string();
        let now = Utc::now();
        let expires_at = request
            .expires_in_days
            .map(|days| now + chrono::Duration::days(days as i64));

        let api_key_doc = ApiKeyDoc {
            id: None,
            key_id: key_id.clone(),
            user_id: user_id.to_string(),
            key_prefix,
            key_hash,
            name: request.name.clone(),
            last_used_at: None,
            expires_at,
            created_at: now,
        };

        self.api_keys().insert_one(&api_key_doc, None).await?;

        self.log_audit_public("key_created", Some(user_id), None, None, Some(&key_id))
            .await;

        Ok(CreateApiKeyResponse {
            id: key_id,
            api_key,
            name: request.name,
            expires_at,
        })
    }

    /// List API keys for a user (without hashes)
    pub async fn list_api_keys(&self, user_id: &str) -> Result<Vec<ApiKey>> {
        use futures::TryStreamExt;

        let options = mongodb::options::FindOptions::builder()
            .sort(doc! { "created_at": -1 })
            .build();
        let cursor = self
            .api_keys()
            .find(doc! { "user_id": user_id }, options)
            .await?;

        let keys: Vec<ApiKeyDoc> = cursor.try_collect().await?;

        Ok(keys
            .into_iter()
            .map(|k| ApiKey {
                id: k.key_id,
                user_id: k.user_id,
                key_prefix: k.key_prefix,
                name: k.name,
                last_used_at: k.last_used_at,
                expires_at: k.expires_at,
                created_at: k.created_at,
            })
            .collect())
    }

    /// Revoke an API key
    pub async fn revoke_api_key(&self, user_id: &str, key_id: &str) -> Result<()> {
        let result = self
            .api_keys()
            .delete_one(doc! { "key_id": key_id, "user_id": user_id }, None)
            .await?;

        if result.deleted_count == 0 {
            return Err(anyhow!("API key not found"));
        }

        self.log_audit_public("key_revoked", Some(user_id), None, None, Some(key_id))
            .await;

        Ok(())
    }

    /// Update last used timestamp for an API key by key_id (precise)
    pub async fn update_key_last_used_by_id(&self, key_id: &str) -> Result<()> {
        let now = Utc::now();
        self.api_keys()
            .update_one(
                doc! { "key_id": key_id },
                doc! { "$set": { "last_used_at": bson::DateTime::from_chrono(now) } },
                None,
            )
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

    /// Hash a password using Argon2
    fn hash_password(&self, password: &str) -> Result<String> {
        self.hash_key(password)
    }

    /// Verify a password against a hash
    fn verify_password(&self, password: &str, hash: &str) -> Result<bool> {
        self.verify_key(password, hash)
    }

    /// Login with email and password
    pub async fn login_with_password(&self, email: &str, password: &str) -> Result<UserResponse> {
        let email = validate_and_normalize_email(email)?;
        let user = self
            .users()
            .find_one(doc! { "email": &email, "is_active": true }, None)
            .await?
            .ok_or_else(|| anyhow!("Invalid email or password"))?;

        let hash = user.password_hash.as_deref()
            .ok_or_else(|| anyhow!("Password login not enabled for this account"))?;

        if !self.verify_password(password, hash)? {
            return Err(anyhow!("Invalid email or password"));
        }

        // Update last login
        let now = Utc::now();
        let _ = self.users().update_one(
            doc! { "user_id": &user.user_id },
            doc! { "$set": { "last_login_at": bson::DateTime::from_chrono(now) } },
            None,
        ).await;

        self.log_audit_public("login_password", Some(&user.user_id), Some(&email), None, None).await;
        Ok(user.into())
    }

    /// Change password for a user
    pub async fn change_password(
        &self,
        user_id: &str,
        current_password: Option<&str>,
        new_password: &str,
    ) -> Result<()> {
        if new_password.len() < 8 {
            return Err(anyhow!("Password must be at least 8 characters"));
        }

        let user = self
            .users()
            .find_one(doc! { "user_id": user_id, "is_active": true }, None)
            .await?
            .ok_or_else(|| anyhow!("User not found"))?;

        // If user already has a password, verify current password
        if let Some(ref existing_hash) = user.password_hash {
            let current = current_password.ok_or_else(|| anyhow!("Current password required"))?;
            if !self.verify_password(current, existing_hash)? {
                return Err(anyhow!("Current password is incorrect"));
            }
        }

        let new_hash = self.hash_password(new_password)?;
        self.users().update_one(
            doc! { "user_id": user_id },
            doc! { "$set": { "password_hash": &new_hash } },
            None,
        ).await?;

        self.log_audit_public("password_changed", Some(user_id), None, None, None).await;
        Ok(())
    }

    /// Submit a registration request (approval mode)
    pub async fn submit_registration(&self, request: RegisterRequest) -> Result<String> {
        let email = validate_and_normalize_email(&request.email)?;

        // Check if already registered
        let existing = self
            .users()
            .find_one(doc! { "email": &email, "is_active": true }, None)
            .await?;
        if existing.is_some() {
            return Err(anyhow!("Email already registered"));
        }

        // Check for existing pending request
        let pending = self
            .registration_requests()
            .find_one(doc! { "email": &email, "status": "pending" }, None)
            .await?;
        if pending.is_some() {
            return Err(anyhow!("Registration request already pending"));
        }

        // Hash password if provided
        let password_hash = match &request.password {
            Some(pw) if !pw.is_empty() => {
                if pw.len() < 8 {
                    return Err(anyhow!("Password must be at least 8 characters"));
                }
                Some(self.hash_password(pw)?)
            }
            _ => None,
        };

        let request_id = uuid::Uuid::new_v4().to_string();
        let doc = RegistrationRequestDoc {
            id: None,
            request_id: request_id.clone(),
            email: email.clone(),
            display_name: request.display_name,
            password_hash,
            status: "pending".to_string(),
            reviewed_by: None,
            reviewed_at: None,
            reject_reason: None,
            created_at: Utc::now(),
        };

        self.registration_requests().insert_one(&doc, None).await?;
        self.log_audit_public("register_request", None, Some(&email), None, None)
            .await;

        Ok(request_id)
    }

    /// List pending registration requests (admin)
    pub async fn list_pending_registrations(&self) -> Result<Vec<RegistrationRequestDoc>> {
        use futures::TryStreamExt;
        let options = mongodb::options::FindOptions::builder()
            .sort(doc! { "created_at": -1 })
            .build();
        let cursor = self
            .registration_requests()
            .find(doc! { "status": "pending" }, options)
            .await?;
        let docs: Vec<RegistrationRequestDoc> = cursor.try_collect().await?;
        Ok(docs)
    }

    /// Approve a registration request (admin)
    pub async fn approve_registration(
        &self,
        request_id: &str,
        reviewed_by: &str,
    ) -> Result<RegisterResponse> {
        let req = self
            .registration_requests()
            .find_one(doc! { "request_id": request_id, "status": "pending" }, None)
            .await?
            .ok_or_else(|| anyhow!("Registration request not found"))?;

        // Update request status atomically (filter includes status to prevent double-approval)
        let now = Utc::now();
        let update_result = self
            .registration_requests()
            .update_one(
                doc! { "request_id": request_id, "status": "pending" },
                doc! { "$set": {
                    "status": "approved",
                    "reviewed_by": reviewed_by,
                    "reviewed_at": bson::DateTime::from_chrono(now),
                }},
                None,
            )
            .await?;

        if update_result.modified_count == 0 {
            return Err(anyhow!("Registration request already processed"));
        }

        // Create user + key via existing register flow (password already hashed in request doc)
        let response = self.register(RegisterRequest {
            email: req.email.clone(),
            display_name: req.display_name,
            password: None,
        }).await?;

        // If the registration request had a password_hash, apply it to the created user
        if let Some(ref ph) = req.password_hash {
            let _ = self.users().update_one(
                doc! { "user_id": &response.user.id },
                doc! { "$set": { "password_hash": ph } },
                None,
            ).await;
        }

        self.log_audit_public(
            "register_approved",
            Some(&response.user.id),
            Some(&req.email),
            None,
            Some(&format!("reviewed_by: {}", reviewed_by)),
        )
        .await;

        Ok(response)
    }

    /// Reject a registration request (admin)
    pub async fn reject_registration(
        &self,
        request_id: &str,
        reviewed_by: &str,
        reason: Option<&str>,
    ) -> Result<()> {
        let req = self
            .registration_requests()
            .find_one(doc! { "request_id": request_id, "status": "pending" }, None)
            .await?
            .ok_or_else(|| anyhow!("Registration request not found"))?;

        let now = Utc::now();
        let update_result = self
            .registration_requests()
            .update_one(
                doc! { "request_id": request_id, "status": "pending" },
                doc! { "$set": {
                    "status": "rejected",
                    "reviewed_by": reviewed_by,
                    "reviewed_at": bson::DateTime::from_chrono(now),
                    "reject_reason": reason.unwrap_or(""),
                }},
                None,
            )
            .await?;

        if update_result.modified_count == 0 {
            return Err(anyhow!("Registration request already processed"));
        }

        self.log_audit_public(
            "register_rejected",
            None,
            Some(&req.email),
            None,
            Some(&format!(
                "reviewed_by: {}, reason: {}",
                reviewed_by,
                reason.unwrap_or("none")
            )),
        )
        .await;

        Ok(())
    }

    /// Deactivate a user account
    pub async fn deactivate_user(&self, user_id: &str) -> Result<()> {
        let now = Utc::now();
        let result = self
            .users()
            .update_one(
                doc! { "user_id": user_id, "is_active": true },
                doc! { "$set": {
                    "is_active": false,
                    "deactivated_at": bson::DateTime::from_chrono(now),
                }},
                None,
            )
            .await?;

        if result.modified_count == 0 {
            return Err(anyhow!("User not found or already deactivated"));
        }

        // Revoke all API keys for this user
        let _ = self
            .api_keys()
            .delete_many(doc! { "user_id": user_id }, None)
            .await;

        self.log_audit_public("account_deactivated", Some(user_id), None, None, None)
            .await;
        Ok(())
    }

    /// Get remaining API key count for a user
    pub async fn get_user_key_count(&self, user_id: &str) -> Result<u64> {
        let count = self
            .api_keys()
            .count_documents(doc! { "user_id": user_id }, None)
            .await?;
        Ok(count)
    }
}
