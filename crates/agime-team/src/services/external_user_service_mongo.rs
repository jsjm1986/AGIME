use crate::db::{collections, MongoDb};
use crate::models::mongo::{
    Document, DocumentSummary, ExternalUser, ExternalUserDetail, ExternalUserEvent,
    ExternalUserEventResponse, ExternalUserPortalSessionSummary, ExternalUserSessionDoc,
    ExternalUserStatus, ExternalUserSummary, PaginatedResponse,
};
use anyhow::{anyhow, Result};
use argon2::{
    password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};
use chrono::{DateTime, Duration, Utc};
use futures::TryStreamExt;
use mongodb::bson::{doc, oid::ObjectId};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PortalSessionDocLite {
    pub session_id: String,
    pub user_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub portal_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub portal_slug: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_message_at: Option<bson::DateTime>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
    pub message_count: i32,
    #[serde(default)]
    pub is_processing: bool,
    #[serde(default)]
    pub portal_restricted: bool,
    pub team_id: String,
}

pub struct ExternalUserService {
    db: Arc<MongoDb>,
}

impl ExternalUserService {
    pub fn new(db: Arc<MongoDb>) -> Self {
        Self { db }
    }

    fn users(&self) -> mongodb::Collection<ExternalUser> {
        self.db.collection(collections::EXTERNAL_USERS)
    }

    fn sessions(&self) -> mongodb::Collection<ExternalUserSessionDoc> {
        self.db.collection(collections::EXTERNAL_USER_SESSIONS)
    }

    fn events(&self) -> mongodb::Collection<ExternalUserEvent> {
        self.db.collection(collections::EXTERNAL_USER_EVENTS)
    }

    fn documents(&self) -> mongodb::Collection<Document> {
        self.db.collection(collections::DOCUMENTS)
    }

    fn agent_sessions(&self) -> mongodb::Collection<PortalSessionDocLite> {
        self.db.collection("agent_sessions")
    }

    fn normalize_username(username: &str) -> Result<(String, String)> {
        let trimmed = username.trim();
        if trimmed.len() < 2 || trimmed.len() > 32 {
            return Err(anyhow!("Username must be between 2 and 32 characters"));
        }
        if trimmed.chars().any(|c| c.is_control() || c.is_whitespace()) {
            return Err(anyhow!(
                "Username cannot contain spaces or control characters"
            ));
        }
        Ok((trimmed.to_string(), trimmed.to_lowercase()))
    }

    fn validate_password(password: &str) -> Result<()> {
        if password.len() < 6 {
            return Err(anyhow!("Password must be at least 6 characters"));
        }
        if password.len() > 128 {
            return Err(anyhow!("Password is too long"));
        }
        Ok(())
    }

    fn hash_password(password: &str) -> Result<String> {
        let salt = SaltString::generate(&mut OsRng);
        Ok(Argon2::default()
            .hash_password(password.as_bytes(), &salt)
            .map_err(|e| anyhow!("Password hash failed: {}", e))?
            .to_string())
    }

    fn verify_password(password: &str, hash: &str) -> Result<bool> {
        let parsed =
            PasswordHash::new(hash).map_err(|e| anyhow!("Password hash parse failed: {}", e))?;
        Ok(Argon2::default()
            .verify_password(password.as_bytes(), &parsed)
            .is_ok())
    }

    async fn find_by_username(
        &self,
        team_id: &ObjectId,
        username_normalized: &str,
    ) -> Result<Option<ExternalUser>> {
        Ok(self
            .users()
            .find_one(
                doc! {
                    "team_id": team_id,
                    "username_normalized": username_normalized,
                },
                None,
            )
            .await?)
    }

    pub async fn get_user(
        &self,
        team_id: &str,
        external_user_id: &str,
    ) -> Result<Option<ExternalUser>> {
        let team_oid = ObjectId::parse_str(team_id)?;
        Ok(self
            .users()
            .find_one(
                doc! {
                    "team_id": team_oid,
                    "external_user_id": external_user_id,
                },
                None,
            )
            .await?)
    }

    pub async fn register(
        &self,
        team_id: &str,
        username: &str,
        password: &str,
        display_name: Option<String>,
        phone: Option<String>,
        visitor_id: Option<&str>,
        ip_address: Option<&str>,
        user_agent: Option<&str>,
    ) -> Result<ExternalUser> {
        let team_oid = ObjectId::parse_str(team_id)?;
        let (username, username_normalized) = Self::normalize_username(username)?;
        Self::validate_password(password)?;

        if self
            .find_by_username(&team_oid, &username_normalized)
            .await?
            .is_some()
        {
            return Err(anyhow!("Username already exists"));
        }

        let external_user = ExternalUser {
            id: None,
            external_user_id: format!("extu_{}", Uuid::new_v4().simple()),
            team_id: team_oid,
            username: username.clone(),
            username_normalized,
            password_hash: Self::hash_password(password)?,
            display_name: display_name
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty()),
            phone: phone
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty()),
            status: ExternalUserStatus::Active,
            linked_visitor_ids: visitor_id
                .map(|id| vec![id.to_string()])
                .unwrap_or_default(),
            tags: Vec::new(),
            notes: None,
            created_at: Utc::now(),
            last_login_at: None,
            last_seen_at: None,
        };

        self.users().insert_one(&external_user, None).await?;

        self.log_event(
            team_id,
            Some(&external_user.external_user_id),
            Some(&external_user.username),
            visitor_id,
            None,
            None,
            None,
            "user_registered",
            "success",
            ip_address,
            user_agent,
            None,
            serde_json::json!({
                "display_name": external_user.display_name,
                "phone": external_user.phone,
            }),
        )
        .await;

        if let Some(vid) = visitor_id {
            let _ = self
                .migrate_anonymous_state(team_id, &external_user.external_user_id, vid)
                .await;
        }

        self.get_user(team_id, &external_user.external_user_id)
            .await?
            .ok_or_else(|| anyhow!("Failed to reload created user"))
    }

    pub async fn authenticate(
        &self,
        team_id: &str,
        username: &str,
        password: &str,
        visitor_id: Option<&str>,
        ip_address: Option<&str>,
        user_agent: Option<&str>,
    ) -> Result<ExternalUser> {
        let team_oid = ObjectId::parse_str(team_id)?;
        let (_, username_normalized) = Self::normalize_username(username)?;

        let user = self
            .find_by_username(&team_oid, &username_normalized)
            .await?
            .ok_or_else(|| anyhow!("Invalid username or password"))?;

        if user.status != ExternalUserStatus::Active {
            self.log_event(
                team_id,
                Some(&user.external_user_id),
                Some(&user.username),
                visitor_id,
                None,
                None,
                None,
                "user_login",
                "disabled",
                ip_address,
                user_agent,
                None,
                serde_json::json!({}),
            )
            .await;
            return Err(anyhow!("Account is disabled"));
        }

        if !Self::verify_password(password, &user.password_hash)? {
            self.log_event(
                team_id,
                Some(&user.external_user_id),
                Some(&user.username),
                visitor_id,
                None,
                None,
                None,
                "user_login",
                "invalid_password",
                ip_address,
                user_agent,
                None,
                serde_json::json!({}),
            )
            .await;
            return Err(anyhow!("Invalid username or password"));
        }

        let now = Utc::now();
        let set = doc! {
            "last_login_at": bson::DateTime::from_chrono(now),
            "last_seen_at": bson::DateTime::from_chrono(now),
        };
        let mut add_to_set = doc! {};
        if let Some(vid) = visitor_id {
            add_to_set.insert("linked_visitor_ids", vid);
        }
        let mut update = doc! { "$set": set };
        if !add_to_set.is_empty() {
            update.insert("$addToSet", add_to_set);
        }
        self.users()
            .update_one(
                doc! {
                    "team_id": team_oid,
                    "external_user_id": &user.external_user_id,
                },
                update,
                None,
            )
            .await?;

        if let Some(vid) = visitor_id {
            let _ = self
                .migrate_anonymous_state(team_id, &user.external_user_id, vid)
                .await;
        }

        self.log_event(
            team_id,
            Some(&user.external_user_id),
            Some(&user.username),
            visitor_id,
            None,
            None,
            None,
            "user_login",
            "success",
            ip_address,
            user_agent,
            None,
            serde_json::json!({}),
        )
        .await;

        self.get_user(team_id, &user.external_user_id)
            .await?
            .ok_or_else(|| anyhow!("Failed to reload user"))
    }

    pub async fn create_session(
        &self,
        team_id: &str,
        external_user_id: &str,
        visitor_id: Option<&str>,
        ip_address: Option<&str>,
        user_agent: Option<&str>,
    ) -> Result<ExternalUserSessionDoc> {
        let team_oid = ObjectId::parse_str(team_id)?;
        let now = Utc::now();
        let session = ExternalUserSessionDoc {
            id: None,
            session_id: format!("exts_{}", Uuid::new_v4().simple()),
            team_id: team_oid,
            external_user_id: external_user_id.to_string(),
            visitor_id: visitor_id.map(ToString::to_string),
            ip_address: ip_address.map(ToString::to_string),
            user_agent: user_agent.map(ToString::to_string),
            created_at: now,
            expires_at: now + Duration::days(30),
            last_seen_at: now,
        };
        self.sessions().insert_one(&session, None).await?;
        Ok(session)
    }

    pub async fn validate_session(
        &self,
        team_id: &str,
        session_id: &str,
    ) -> Result<Option<ExternalUser>> {
        let team_oid = ObjectId::parse_str(team_id)?;
        let session = match self
            .sessions()
            .find_one(doc! { "team_id": team_oid, "session_id": session_id }, None)
            .await?
        {
            Some(session) => session,
            None => return Ok(None),
        };

        if Utc::now() > session.expires_at {
            let _ = self
                .sessions()
                .delete_one(doc! { "session_id": session_id }, None)
                .await;
            return Ok(None);
        }

        let user = self
            .get_user(team_id, &session.external_user_id)
            .await?
            .filter(|user| user.status == ExternalUserStatus::Active);

        if user.is_some() {
            let now = bson::DateTime::from_chrono(Utc::now());
            let _ = self
                .sessions()
                .update_one(
                    doc! { "session_id": session_id },
                    doc! { "$set": { "last_seen_at": now } },
                    None,
                )
                .await;
        }

        Ok(user)
    }

    pub async fn delete_session(&self, session_id: &str) -> Result<()> {
        self.sessions()
            .delete_one(doc! { "session_id": session_id }, None)
            .await?;
        Ok(())
    }

    pub async fn migrate_anonymous_state(
        &self,
        team_id: &str,
        external_user_id: &str,
        visitor_id: &str,
    ) -> Result<(u64, u64)> {
        let team_oid = ObjectId::parse_str(team_id)?;
        let synthetic_user_id = format!("portal_visitor_{visitor_id}");

        let doc_result = self
            .documents()
            .update_many(
                doc! {
                    "team_id": team_oid,
                    "uploaded_by": &synthetic_user_id,
                },
                doc! {
                    "$set": {
                        "uploaded_by": external_user_id,
                    }
                },
                None,
            )
            .await?;

        let session_result = self
            .agent_sessions()
            .update_many(
                doc! {
                    "team_id": team_id,
                    "user_id": &synthetic_user_id,
                    "portal_restricted": true,
                },
                doc! {
                    "$set": {
                        "user_id": external_user_id,
                    }
                },
                None,
            )
            .await?;

        self.users()
            .update_one(
                doc! {
                    "team_id": team_oid,
                    "external_user_id": external_user_id,
                },
                doc! {
                    "$addToSet": { "linked_visitor_ids": visitor_id },
                    "$set": { "last_seen_at": bson::DateTime::from_chrono(Utc::now()) },
                },
                None,
            )
            .await?;

        self.log_event(
            team_id,
            Some(external_user_id),
            None,
            Some(visitor_id),
            None,
            None,
            None,
            "visitor_linked",
            "success",
            None,
            None,
            None,
            serde_json::json!({
                "migrated_documents": doc_result.modified_count,
                "migrated_sessions": session_result.modified_count,
            }),
        )
        .await;

        Ok((doc_result.modified_count, session_result.modified_count))
    }

    pub async fn set_user_status(
        &self,
        team_id: &str,
        external_user_id: &str,
        status: ExternalUserStatus,
        actor_user_id: &str,
    ) -> Result<()> {
        let team_oid = ObjectId::parse_str(team_id)?;
        self.users()
            .update_one(
                doc! {
                    "team_id": team_oid,
                    "external_user_id": external_user_id,
                },
                doc! {
                    "$set": {
                        "status": bson::to_bson(&status)?,
                        "last_seen_at": bson::DateTime::from_chrono(Utc::now()),
                    }
                },
                None,
            )
            .await?;
        self.log_event(
            team_id,
            Some(external_user_id),
            None,
            None,
            None,
            None,
            None,
            "user_status_changed",
            "success",
            None,
            None,
            None,
            serde_json::json!({
                "status": status,
                "actor_user_id": actor_user_id,
            }),
        )
        .await;
        Ok(())
    }

    pub async fn reset_password(
        &self,
        team_id: &str,
        external_user_id: &str,
        new_password: &str,
        actor_user_id: &str,
    ) -> Result<()> {
        Self::validate_password(new_password)?;
        let team_oid = ObjectId::parse_str(team_id)?;
        self.users()
            .update_one(
                doc! {
                    "team_id": team_oid,
                    "external_user_id": external_user_id,
                },
                doc! {
                    "$set": {
                        "password_hash": Self::hash_password(new_password)?,
                        "last_seen_at": bson::DateTime::from_chrono(Utc::now()),
                    }
                },
                None,
            )
            .await?;
        self.sessions()
            .delete_many(
                doc! { "team_id": team_oid, "external_user_id": external_user_id },
                None,
            )
            .await?;
        self.log_event(
            team_id,
            Some(external_user_id),
            None,
            None,
            None,
            None,
            None,
            "password_reset_by_admin",
            "success",
            None,
            None,
            None,
            serde_json::json!({
                "actor_user_id": actor_user_id,
            }),
        )
        .await;
        Ok(())
    }

    pub async fn list_users(
        &self,
        team_id: &str,
        page: u32,
        limit: u32,
        search: Option<&str>,
        status: Option<ExternalUserStatus>,
    ) -> Result<PaginatedResponse<ExternalUserSummary>> {
        let team_oid = ObjectId::parse_str(team_id)?;
        let mut filter = doc! { "team_id": team_oid };
        if let Some(status) = status {
            filter.insert("status", bson::to_bson(&status)?);
        }
        if let Some(search) = search.map(str::trim).filter(|value| !value.is_empty()) {
            filter.insert(
                "$or",
                vec![
                    doc! { "username": { "$regex": search, "$options": "i" } },
                    doc! { "display_name": { "$regex": search, "$options": "i" } },
                    doc! { "phone": { "$regex": search, "$options": "i" } },
                ],
            );
        }

        let total = self.users().count_documents(filter.clone(), None).await?;
        let page = page.max(1);
        let limit = limit.clamp(1, 200);
        let skip = u64::from((page - 1) * limit);
        let options = mongodb::options::FindOptions::builder()
            .sort(doc! { "last_seen_at": -1, "created_at": -1 })
            .skip(skip)
            .limit(i64::from(limit))
            .build();
        let cursor = self.users().find(filter, options).await?;
        let users: Vec<ExternalUser> = cursor.try_collect().await?;

        let mut items = Vec::with_capacity(users.len());
        for user in users {
            items.push(self.build_summary(&user).await?);
        }

        Ok(PaginatedResponse::new(
            items,
            total,
            u64::from(page),
            u64::from(limit),
        ))
    }

    pub async fn get_user_detail(
        &self,
        team_id: &str,
        external_user_id: &str,
    ) -> Result<Option<ExternalUserDetail>> {
        let Some(user) = self.get_user(team_id, external_user_id).await? else {
            return Ok(None);
        };
        let summary = self.build_summary(&user).await?;
        let team_oid = ObjectId::parse_str(team_id)?;

        let upload_options = mongodb::options::FindOptions::builder()
            .sort(doc! { "updated_at": -1 })
            .limit(12)
            .build();
        let upload_cursor = self
            .documents()
            .find(
                doc! {
                    "team_id": team_oid,
                    "uploaded_by": external_user_id,
                    "is_deleted": { "$ne": true },
                },
                upload_options,
            )
            .await?;
        let recent_uploads: Vec<DocumentSummary> = upload_cursor
            .try_collect::<Vec<Document>>()
            .await?
            .into_iter()
            .map(Into::into)
            .collect();

        let session_options = mongodb::options::FindOptions::builder()
            .sort(doc! { "updated_at": -1 })
            .limit(12)
            .build();
        let session_cursor = self
            .agent_sessions()
            .find(
                doc! {
                    "team_id": team_id,
                    "user_id": external_user_id,
                    "portal_restricted": true,
                },
                session_options,
            )
            .await?;
        let recent_sessions: Vec<ExternalUserPortalSessionSummary> = session_cursor
            .try_collect::<Vec<PortalSessionDocLite>>()
            .await?
            .into_iter()
            .map(|session| ExternalUserPortalSessionSummary {
                session_id: session.session_id,
                portal_id: session.portal_id,
                portal_slug: session.portal_slug,
                title: session.title,
                last_message_at: session.last_message_at.map(|value| value.to_chrono()),
                updated_at: session.updated_at,
                message_count: session.message_count,
                is_processing: session.is_processing,
            })
            .collect();

        Ok(Some(ExternalUserDetail {
            user: summary,
            linked_visitor_ids: user.linked_visitor_ids,
            recent_uploads,
            recent_sessions,
        }))
    }

    pub async fn list_events(
        &self,
        team_id: &str,
        external_user_id: Option<&str>,
        event_type: Option<&str>,
        page: u32,
        limit: u32,
    ) -> Result<PaginatedResponse<ExternalUserEventResponse>> {
        let team_oid = ObjectId::parse_str(team_id)?;
        let mut filter = doc! { "team_id": team_oid };
        if let Some(user_id) = external_user_id.filter(|value| !value.trim().is_empty()) {
            filter.insert("external_user_id", user_id);
        }
        if let Some(event_type) = event_type.filter(|value| !value.trim().is_empty()) {
            filter.insert("event_type", event_type);
        }

        let total = self.events().count_documents(filter.clone(), None).await?;
        let page = page.max(1);
        let limit = limit.clamp(1, 500);
        let skip = u64::from((page - 1) * limit);
        let options = mongodb::options::FindOptions::builder()
            .sort(doc! { "created_at": -1 })
            .skip(skip)
            .limit(i64::from(limit))
            .build();
        let cursor = self.events().find(filter, options).await?;
        let events: Vec<ExternalUserEvent> = cursor.try_collect().await?;
        let items = events.into_iter().map(Into::into).collect();
        Ok(PaginatedResponse::new(
            items,
            total,
            u64::from(page),
            u64::from(limit),
        ))
    }

    async fn build_summary(&self, user: &ExternalUser) -> Result<ExternalUserSummary> {
        let upload_count = self
            .documents()
            .count_documents(
                doc! {
                    "team_id": user.team_id,
                    "uploaded_by": &user.external_user_id,
                    "is_deleted": { "$ne": true },
                },
                None,
            )
            .await?;
        let session_count = self
            .agent_sessions()
            .count_documents(
                doc! {
                    "team_id": user.team_id.to_hex(),
                    "user_id": &user.external_user_id,
                    "portal_restricted": true,
                },
                None,
            )
            .await?;
        let event_count = self
            .events()
            .count_documents(
                doc! {
                    "team_id": user.team_id,
                    "external_user_id": &user.external_user_id,
                },
                None,
            )
            .await?;

        Ok(ExternalUserSummary {
            id: user.external_user_id.clone(),
            team_id: user.team_id.to_hex(),
            username: user.username.clone(),
            display_name: user.display_name.clone(),
            phone: user.phone.clone(),
            status: user.status,
            linked_visitor_count: user.linked_visitor_ids.len(),
            upload_count,
            session_count,
            event_count,
            created_at: user.created_at,
            last_login_at: user.last_login_at,
            last_seen_at: user.last_seen_at,
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn log_event(
        &self,
        team_id: &str,
        external_user_id: Option<&str>,
        username: Option<&str>,
        visitor_id: Option<&str>,
        portal_id: Option<&str>,
        portal_slug: Option<&str>,
        session_id: Option<&str>,
        event_type: &str,
        result: &str,
        ip_address: Option<&str>,
        user_agent: Option<&str>,
        target_document_id: Option<&str>,
        metadata: serde_json::Value,
    ) {
        let team_oid = match ObjectId::parse_str(team_id) {
            Ok(value) => value,
            Err(e) => {
                tracing::warn!("external user log_event invalid team id {}: {}", team_id, e);
                return;
            }
        };
        let event = ExternalUserEvent {
            id: None,
            event_id: format!("eue_{}", Uuid::new_v4().simple()),
            team_id: team_oid,
            external_user_id: external_user_id.map(ToString::to_string),
            username: username.map(ToString::to_string),
            visitor_id: visitor_id.map(ToString::to_string),
            portal_id: portal_id.map(ToString::to_string),
            portal_slug: portal_slug.map(ToString::to_string),
            session_id: session_id.map(ToString::to_string),
            event_type: event_type.to_string(),
            result: result.to_string(),
            ip_address: ip_address.map(ToString::to_string),
            user_agent: user_agent.map(ToString::to_string),
            target_document_id: target_document_id.map(ToString::to_string),
            metadata,
            created_at: Utc::now(),
        };
        if let Err(err) = self.events().insert_one(event, None).await {
            tracing::warn!("Failed to write external user event log: {}", err);
        }
    }
}
