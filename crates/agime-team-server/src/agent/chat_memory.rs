use std::sync::Arc;

use agime_team::MongoDb;
use chrono::{DateTime, Utc};
use mongodb::bson::{doc, oid::ObjectId};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

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

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UserChatMemoryFields {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preferred_address: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role_hint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_focus: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub collaboration_preference: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserChatMemoryDoc {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub team_id: String,
    pub user_id: String,
    #[serde(flatten)]
    pub fields: UserChatMemoryFields,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
    pub updated_by_user_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UserChatMemorySuggestionStatus {
    Pending,
    Accepted,
    Dismissed,
    Expired,
}

impl Default for UserChatMemorySuggestionStatus {
    fn default() -> Self {
        Self::Pending
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserChatMemorySuggestionDoc {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub suggestion_id: String,
    pub team_id: String,
    pub user_id: String,
    pub session_id: String,
    pub source_message: String,
    #[serde(default)]
    pub proposed_patch: UserChatMemoryFields,
    pub reason: String,
    #[serde(default)]
    pub status: UserChatMemorySuggestionStatus,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "bson_datetime_option"
    )]
    pub resolved_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UpdateUserChatMemoryRequest {
    #[serde(default)]
    pub preferred_address: Option<Option<String>>,
    #[serde(default)]
    pub role_hint: Option<Option<String>>,
    #[serde(default)]
    pub current_focus: Option<Option<String>>,
    #[serde(default)]
    pub collaboration_preference: Option<Option<String>>,
    #[serde(default)]
    pub notes: Option<Option<String>>,
    #[serde(default)]
    pub session_id: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct UserChatMemoryPatch {
    pub preferred_address: Option<Option<String>>,
    pub role_hint: Option<Option<String>>,
    pub current_focus: Option<Option<String>>,
    pub collaboration_preference: Option<Option<String>>,
    pub notes: Option<Option<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserChatMemoryResponse {
    pub team_id: String,
    pub user_id: String,
    #[serde(flatten)]
    pub fields: UserChatMemoryFields,
    pub updated_at: DateTime<Utc>,
    pub updated_by_user_id: String,
}

impl From<UserChatMemoryDoc> for UserChatMemoryResponse {
    fn from(value: UserChatMemoryDoc) -> Self {
        Self {
            team_id: value.team_id,
            user_id: value.user_id,
            fields: value.fields,
            updated_at: value.updated_at,
            updated_by_user_id: value.updated_by_user_id,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserChatMemorySuggestionResponse {
    pub suggestion_id: String,
    pub team_id: String,
    pub user_id: String,
    pub session_id: String,
    pub source_message: String,
    pub proposed_patch: UserChatMemoryFields,
    pub reason: String,
    pub status: UserChatMemorySuggestionStatus,
    pub created_at: DateTime<Utc>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
    )]
    pub resolved_at: Option<DateTime<Utc>>,
}

impl From<UserChatMemorySuggestionDoc> for UserChatMemorySuggestionResponse {
    fn from(value: UserChatMemorySuggestionDoc) -> Self {
        Self {
            suggestion_id: value.suggestion_id,
            team_id: value.team_id,
            user_id: value.user_id,
            session_id: value.session_id,
            source_message: value.source_message,
            proposed_patch: value.proposed_patch,
            reason: value.reason,
            status: value.status,
            created_at: value.created_at,
            resolved_at: value.resolved_at,
        }
    }
}

fn sanitize_with_limit(value: Option<String>, limit: usize) -> Option<String> {
    value.and_then(|raw| {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.chars().take(limit).collect())
        }
    })
}

fn sanitize_patch_field(value: Option<Option<String>>, limit: usize) -> Option<Option<String>> {
    match value {
        Some(Some(raw)) => {
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                Some(None)
            } else {
                Some(Some(trimmed.chars().take(limit).collect()))
            }
        }
        Some(None) => Some(None),
        None => None,
    }
}

pub fn sanitize_memory_fields(fields: UserChatMemoryFields) -> UserChatMemoryFields {
    UserChatMemoryFields {
        preferred_address: sanitize_with_limit(fields.preferred_address, 32),
        role_hint: sanitize_with_limit(fields.role_hint, 32),
        current_focus: sanitize_with_limit(fields.current_focus, 120),
        collaboration_preference: sanitize_with_limit(fields.collaboration_preference, 120),
        notes: sanitize_with_limit(fields.notes, 160),
    }
}

pub fn sanitize_memory_patch(
    request: UpdateUserChatMemoryRequest,
) -> (UserChatMemoryPatch, Option<String>) {
    (
        UserChatMemoryPatch {
            preferred_address: sanitize_patch_field(request.preferred_address, 32),
            role_hint: sanitize_patch_field(request.role_hint, 32),
            current_focus: sanitize_patch_field(request.current_focus, 120),
            collaboration_preference: sanitize_patch_field(request.collaboration_preference, 120),
            notes: sanitize_patch_field(request.notes, 160),
        },
        sanitize_with_limit(request.session_id, 64),
    )
}

fn merge_memory_fields(
    current: UserChatMemoryFields,
    patch: UserChatMemoryPatch,
) -> UserChatMemoryFields {
    UserChatMemoryFields {
        preferred_address: match patch.preferred_address {
            Some(value) => value,
            None => current.preferred_address,
        },
        role_hint: match patch.role_hint {
            Some(value) => value,
            None => current.role_hint,
        },
        current_focus: match patch.current_focus {
            Some(value) => value,
            None => current.current_focus,
        },
        collaboration_preference: match patch.collaboration_preference {
            Some(value) => value,
            None => current.collaboration_preference,
        },
        notes: match patch.notes {
            Some(value) => value,
            None => current.notes,
        },
    }
}

impl From<UserChatMemoryFields> for UserChatMemoryPatch {
    fn from(value: UserChatMemoryFields) -> Self {
        Self {
            preferred_address: value.preferred_address.map(Some),
            role_hint: value.role_hint.map(Some),
            current_focus: value.current_focus.map(Some),
            collaboration_preference: value.collaboration_preference.map(Some),
            notes: value.notes.map(Some),
        }
    }
}

pub fn render_user_relationship_overlay(
    display_name: &str,
    memory: Option<&UserChatMemoryDoc>,
) -> String {
    let preferred_address = memory
        .and_then(|item| item.fields.preferred_address.clone())
        .unwrap_or_else(|| display_name.to_string());

    let mut lines = vec![
        "<user_relationship>".to_string(),
        format!("当前对话对象：{}", display_name),
        format!("优先称呼：{}", preferred_address),
    ];

    if let Some(value) = memory.and_then(|item| item.fields.role_hint.clone()) {
        lines.push(format!("团队角色：{}", value));
    }
    if let Some(value) = memory.and_then(|item| item.fields.current_focus.clone()) {
        lines.push(format!("最近关注：{}", value));
    }
    if let Some(value) = memory.and_then(|item| item.fields.collaboration_preference.clone()) {
        lines.push(format!("回复偏好：{}", value));
    }
    if let Some(value) = memory.and_then(|item| item.fields.notes.clone()) {
        lines.push(format!("稳定信息：{}", value));
    }
    lines.push(
        "默认可以直接使用优先称呼与用户交流，不需要解释这是不是实名或标识。用户问“我是谁”“你认识我吗”“我们在做什么”这类问题时，优先根据这些关系信息自然回答，而不是退回成元解释口吻。".to_string(),
    );
    lines.push(
        "用户打招呼、开启新话题、表达情绪、需要确认或被鼓励时，若语境自然，优先直接带上优先称呼；不要每次都硬叫，但也不要明明知道称呼却回得像陌生人。".to_string(),
    );
    lines.push(
        "如果这里已经有明确的回复偏好，普通对话里优先按这个人的回复偏好来交流；团队公共语境只作为背景，不覆盖当前用户的交流方式。".to_string(),
    );
    lines.push(
        "如果关系信息缺失但有助于更自然地交流，先通过对话自然确认；当用户明确提供稳定信息并希望记住时，可以使用 chat_memory__save_memory 保存；如果某些稳定信息值得保留但用户还没有明确要求保存，可以使用 chat_memory__propose_memory_update 提出建议。".to_string(),
    );
    lines.push("</user_relationship>".to_string());
    lines.join("\n")
}

fn build_preferred_address_suggestion(message: &str) -> Option<(UserChatMemoryFields, String)> {
    let text = message.trim();
    let patterns = [
        ("以后叫我", "preferred_address"),
        ("你以后叫我", "preferred_address"),
        ("你可以叫我", "preferred_address"),
        ("叫我", "preferred_address"),
    ];

    for (prefix, _) in patterns {
        if let Some(rest) = text.strip_prefix(prefix) {
            let cleaned = rest
                .trim()
                .trim_start_matches('：')
                .trim_start_matches(':')
                .trim()
                .trim_end_matches("就行")
                .trim_end_matches("就可以")
                .trim_end_matches("吧")
                .trim_end_matches("。")
                .trim_end_matches("！")
                .trim_end_matches("!")
                .trim();
            if !cleaned.is_empty() {
                return Some((
                    UserChatMemoryFields {
                        preferred_address: Some(cleaned.to_string()),
                        ..Default::default()
                    },
                    "用户明确指定了偏好的称呼方式".to_string(),
                ));
            }
        }
    }
    None
}

fn build_role_hint_suggestion(message: &str) -> Option<(UserChatMemoryFields, String)> {
    let text = message.trim();
    let patterns = [
        "我是团队里的",
        "我在团队里是",
        "我是负责",
        "我主要负责",
    ];
    for prefix in patterns {
        if let Some(rest) = text.strip_prefix(prefix) {
            let cleaned = rest
                .trim()
                .trim_start_matches('：')
                .trim_start_matches(':')
                .trim_end_matches('。')
                .trim_end_matches('！')
                .trim_end_matches('!')
                .trim();
            if !cleaned.is_empty() {
                return Some((
                    UserChatMemoryFields {
                        role_hint: Some(cleaned.to_string()),
                        ..Default::default()
                    },
                    "用户明确说明了自己在团队里的角色或职责".to_string(),
                ));
            }
        }
    }
    None
}

fn build_current_focus_suggestion(message: &str) -> Option<(UserChatMemoryFields, String)> {
    let text = message.trim();
    let patterns = ["我最近主要在做", "我现在主要在推进", "我最近在推进", "我现在在做"];
    for prefix in patterns {
        if let Some(rest) = text.strip_prefix(prefix) {
            let cleaned = rest
                .trim()
                .trim_start_matches('：')
                .trim_start_matches(':')
                .trim_end_matches('。')
                .trim_end_matches('！')
                .trim_end_matches('!')
                .trim();
            if !cleaned.is_empty() {
                return Some((
                    UserChatMemoryFields {
                        current_focus: Some(cleaned.to_string()),
                        ..Default::default()
                    },
                    "用户明确说明了最近正在推进的重点事项".to_string(),
                ));
            }
        }
    }
    None
}

fn build_collaboration_preference_suggestion(
    message: &str,
) -> Option<(UserChatMemoryFields, String)> {
    let text = message.trim();
    let patterns = ["你回答时", "以后回答", "跟我说话时", "和我沟通时"];
    for prefix in patterns {
        if let Some(rest) = text.strip_prefix(prefix) {
            let cleaned = rest
                .trim()
                .trim_start_matches('：')
                .trim_start_matches(':')
                .trim_end_matches('。')
                .trim_end_matches('！')
                .trim_end_matches('!')
                .trim();
            if !cleaned.is_empty() {
                return Some((
                    UserChatMemoryFields {
                        collaboration_preference: Some(cleaned.to_string()),
                        ..Default::default()
                    },
                    "用户明确表达了希望如何协作或回答".to_string(),
                ));
            }
        }
    }
    None
}

pub fn derive_memory_suggestions_from_message(
    message: &str,
    existing: Option<&UserChatMemoryDoc>,
) -> Vec<(UserChatMemoryFields, String)> {
    let mut out = Vec::new();
    let candidates = [
        build_preferred_address_suggestion(message),
        build_role_hint_suggestion(message),
        build_current_focus_suggestion(message),
        build_collaboration_preference_suggestion(message),
    ];

    for candidate in candidates.into_iter().flatten() {
        let sanitized = sanitize_memory_fields(candidate.0);
        let skip = if let Some(current) = existing {
            current.fields.preferred_address == sanitized.preferred_address
                && current.fields.role_hint == sanitized.role_hint
                && current.fields.current_focus == sanitized.current_focus
                && current.fields.collaboration_preference == sanitized.collaboration_preference
                && current.fields.notes == sanitized.notes
        } else {
            false
        };
        if !skip {
            out.push((sanitized, candidate.1));
        }
    }

    out
}

pub fn render_memory_update_notice(memory: &UserChatMemoryDoc) -> String {
    let mut lines = vec!["<user_memory_update>".to_string(), "当前用户关系记忆已更新。".to_string()];
    if let Some(value) = memory.fields.preferred_address.clone() {
        lines.push(format!("优先称呼：{}", value));
    }
    if let Some(value) = memory.fields.role_hint.clone() {
        lines.push(format!("团队角色：{}", value));
    }
    if let Some(value) = memory.fields.current_focus.clone() {
        lines.push(format!("最近关注：{}", value));
    }
    if let Some(value) = memory.fields.collaboration_preference.clone() {
        lines.push(format!("协作偏好：{}", value));
    }
    if let Some(value) = memory.fields.notes.clone() {
        lines.push(format!("稳定信息：{}", value));
    }
    lines.push("后续普通对话里自然使用这些信息，不要逐条解释来源。".to_string());
    lines.push("</user_memory_update>".to_string());
    lines.join("\n")
}

pub struct ChatMemoryService {
    db: Arc<MongoDb>,
}

impl ChatMemoryService {
    pub fn new(db: Arc<MongoDb>) -> Self {
        Self { db }
    }

    fn memories(&self) -> mongodb::Collection<UserChatMemoryDoc> {
        self.db.collection("user_chat_memories")
    }

    fn suggestions(&self) -> mongodb::Collection<UserChatMemorySuggestionDoc> {
        self.db.collection("user_chat_memory_suggestions")
    }

    pub async fn get_memory(
        &self,
        team_id: &str,
        user_id: &str,
    ) -> Result<Option<UserChatMemoryDoc>, mongodb::error::Error> {
        self.memories()
            .find_one(doc! { "team_id": team_id, "user_id": user_id }, None)
            .await
    }

    pub async fn upsert_memory(
        &self,
        team_id: &str,
        user_id: &str,
        patch: UserChatMemoryPatch,
        updated_by_user_id: &str,
    ) -> Result<UserChatMemoryDoc, mongodb::error::Error> {
        let now = Utc::now();
        let current = self.get_memory(team_id, user_id).await?;
        let merged = merge_memory_fields(
            current
                .as_ref()
                .map(|item| item.fields.clone())
                .unwrap_or_default(),
            patch,
        );
        let doc = UserChatMemoryDoc {
            id: current.as_ref().and_then(|item| item.id.clone()),
            team_id: team_id.to_string(),
            user_id: user_id.to_string(),
            fields: merged,
            updated_at: now,
            updated_by_user_id: updated_by_user_id.to_string(),
        };
        self.memories()
            .replace_one(
                doc! { "team_id": team_id, "user_id": user_id },
                &doc,
                mongodb::options::ReplaceOptions::builder()
                    .upsert(true)
                    .build(),
            )
            .await?;
        Ok(doc)
    }

    pub async fn list_pending_suggestions(
        &self,
        team_id: &str,
        user_id: &str,
        session_id: Option<&str>,
    ) -> Result<Vec<UserChatMemorySuggestionDoc>, mongodb::error::Error> {
        let mut filter = doc! {
            "team_id": team_id,
            "user_id": user_id,
            "status": "pending",
        };
        if let Some(value) = session_id {
            filter.insert("session_id", value);
        }

        let mut cursor = self
            .suggestions()
            .find(
                filter,
                mongodb::options::FindOptions::builder()
                    .sort(doc! { "created_at": -1 })
                    .limit(20)
                    .build(),
            )
            .await?;
        let mut out = Vec::new();
        while cursor.advance().await? {
            out.push(cursor.deserialize_current()?);
        }
        Ok(out)
    }

    pub async fn create_suggestion(
        &self,
        team_id: &str,
        user_id: &str,
        session_id: &str,
        source_message: &str,
        patch: UserChatMemoryFields,
        reason: &str,
    ) -> Result<UserChatMemorySuggestionDoc, mongodb::error::Error> {
        let now = Utc::now();
        let existing = self
            .suggestions()
            .find_one(
                doc! {
                    "team_id": team_id,
                    "user_id": user_id,
                    "session_id": session_id,
                    "status": "pending",
                    "proposed_patch.preferred_address": patch.preferred_address.clone(),
                    "proposed_patch.role_hint": patch.role_hint.clone(),
                    "proposed_patch.current_focus": patch.current_focus.clone(),
                    "proposed_patch.collaboration_preference": patch.collaboration_preference.clone(),
                    "proposed_patch.notes": patch.notes.clone(),
                },
                None,
            )
            .await?;
        if let Some(existing) = existing {
            return Ok(existing);
        }

        let doc = UserChatMemorySuggestionDoc {
            id: None,
            suggestion_id: Uuid::new_v4().to_string(),
            team_id: team_id.to_string(),
            user_id: user_id.to_string(),
            session_id: session_id.to_string(),
            source_message: source_message.to_string(),
            proposed_patch: patch,
            reason: reason.to_string(),
            status: UserChatMemorySuggestionStatus::Pending,
            created_at: now,
            resolved_at: None,
        };
        self.suggestions().insert_one(&doc, None).await?;
        Ok(doc)
    }

    pub async fn get_suggestion(
        &self,
        suggestion_id: &str,
    ) -> Result<Option<UserChatMemorySuggestionDoc>, mongodb::error::Error> {
        self.suggestions()
            .find_one(doc! { "suggestion_id": suggestion_id }, None)
            .await
    }

    pub async fn accept_suggestion(
        &self,
        suggestion_id: &str,
        actor_user_id: &str,
    ) -> Result<Option<(UserChatMemorySuggestionDoc, UserChatMemoryDoc)>, mongodb::error::Error> {
        let Some(mut suggestion) = self.get_suggestion(suggestion_id).await? else {
            return Ok(None);
        };
        let memory = self
            .upsert_memory(
                &suggestion.team_id,
                &suggestion.user_id,
                suggestion.proposed_patch.clone().into(),
                actor_user_id,
            )
            .await?;
        suggestion.status = UserChatMemorySuggestionStatus::Accepted;
        suggestion.resolved_at = Some(Utc::now());
        self.suggestions()
            .update_one(
                doc! { "suggestion_id": suggestion_id },
                doc! {
                    "$set": {
                        "status": "accepted",
                        "resolved_at": bson::DateTime::from_chrono(suggestion.resolved_at.unwrap()),
                    }
                },
                None,
            )
            .await?;
        Ok(Some((suggestion, memory)))
    }

    pub async fn dismiss_suggestion(
        &self,
        suggestion_id: &str,
    ) -> Result<Option<UserChatMemorySuggestionDoc>, mongodb::error::Error> {
        let Some(mut suggestion) = self.get_suggestion(suggestion_id).await? else {
            return Ok(None);
        };
        suggestion.status = UserChatMemorySuggestionStatus::Dismissed;
        suggestion.resolved_at = Some(Utc::now());
        self.suggestions()
            .update_one(
                doc! { "suggestion_id": suggestion_id },
                doc! {
                    "$set": {
                        "status": "dismissed",
                        "resolved_at": bson::DateTime::from_chrono(suggestion.resolved_at.unwrap()),
                    }
                },
                None,
            )
            .await?;
        Ok(Some(suggestion))
    }
}
