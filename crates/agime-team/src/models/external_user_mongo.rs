use chrono::{DateTime, Utc};
use mongodb::bson::oid::ObjectId;
use serde::{Deserialize, Serialize};

use super::DocumentSummary;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ExternalUserStatus {
    #[default]
    Active,
    Disabled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExternalUser {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub external_user_id: String,
    pub team_id: ObjectId,
    pub username: String,
    pub username_normalized: String,
    pub password_hash: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phone: Option<String>,
    #[serde(default)]
    pub status: ExternalUserStatus,
    #[serde(default)]
    pub linked_visitor_ids: Vec<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::models::mongo::common_mongo::bson_datetime_option"
    )]
    pub last_login_at: Option<DateTime<Utc>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::models::mongo::common_mongo::bson_datetime_option"
    )]
    pub last_seen_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExternalUserSessionDoc {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub session_id: String,
    pub team_id: ObjectId,
    pub external_user_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visitor_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ip_address: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_agent: Option<String>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub expires_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub last_seen_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExternalUserEvent {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub event_id: String,
    pub team_id: ObjectId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_user_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visitor_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub portal_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub portal_slug: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    pub event_type: String,
    pub result: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ip_address: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_agent: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_document_id: Option<String>,
    #[serde(default)]
    pub metadata: serde_json::Value,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExternalUserSummary {
    pub id: String,
    pub team_id: String,
    pub username: String,
    pub display_name: Option<String>,
    pub phone: Option<String>,
    pub status: ExternalUserStatus,
    pub linked_visitor_count: usize,
    pub upload_count: u64,
    pub session_count: u64,
    pub event_count: u64,
    pub created_at: DateTime<Utc>,
    pub last_login_at: Option<DateTime<Utc>>,
    pub last_seen_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExternalUserPortalSessionSummary {
    pub session_id: String,
    pub portal_id: Option<String>,
    pub portal_slug: Option<String>,
    pub title: Option<String>,
    pub last_message_at: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
    pub message_count: i32,
    pub is_processing: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExternalUserDetail {
    pub user: ExternalUserSummary,
    pub linked_visitor_ids: Vec<String>,
    pub recent_uploads: Vec<DocumentSummary>,
    pub recent_sessions: Vec<ExternalUserPortalSessionSummary>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExternalUserPublicProfile {
    pub id: String,
    pub username: String,
    pub display_name: Option<String>,
    pub phone: Option<String>,
    pub status: ExternalUserStatus,
    pub linked_visitor_ids: Vec<String>,
    pub created_at: DateTime<Utc>,
    pub last_login_at: Option<DateTime<Utc>>,
    pub last_seen_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExternalUserEventResponse {
    pub id: String,
    pub event_id: String,
    pub team_id: String,
    pub external_user_id: Option<String>,
    pub username: Option<String>,
    pub visitor_id: Option<String>,
    pub portal_id: Option<String>,
    pub portal_slug: Option<String>,
    pub session_id: Option<String>,
    pub event_type: String,
    pub result: String,
    pub ip_address: Option<String>,
    pub user_agent: Option<String>,
    pub target_document_id: Option<String>,
    pub metadata: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

impl ExternalUser {
    pub fn to_public_profile(&self) -> ExternalUserPublicProfile {
        ExternalUserPublicProfile {
            id: self.external_user_id.clone(),
            username: self.username.clone(),
            display_name: self.display_name.clone(),
            phone: self.phone.clone(),
            status: self.status,
            linked_visitor_ids: self.linked_visitor_ids.clone(),
            created_at: self.created_at,
            last_login_at: self.last_login_at,
            last_seen_at: self.last_seen_at,
        }
    }
}

impl From<ExternalUserEvent> for ExternalUserEventResponse {
    fn from(value: ExternalUserEvent) -> Self {
        Self {
            id: value.id.map(|id| id.to_hex()).unwrap_or_default(),
            event_id: value.event_id,
            team_id: value.team_id.to_hex(),
            external_user_id: value.external_user_id,
            username: value.username,
            visitor_id: value.visitor_id,
            portal_id: value.portal_id,
            portal_slug: value.portal_slug,
            session_id: value.session_id,
            event_type: value.event_type,
            result: value.result,
            ip_address: value.ip_address,
            user_agent: value.user_agent,
            target_document_id: value.target_document_id,
            metadata: value.metadata,
            created_at: value.created_at,
        }
    }
}
