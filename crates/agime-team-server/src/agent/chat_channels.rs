use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use agime_team::MongoDb;
use agime_team::services::mongo::FolderService;
use chrono::Utc;
use futures::TryStreamExt;
use mongodb::bson::{self, doc, oid::ObjectId};
use mongodb::options::{FindOneAndUpdateOptions, FindOptions, ReturnDocument};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ChatChannelVisibility {
    #[default]
    TeamPublic,
    TeamPrivate,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ChatChannelStatus {
    #[default]
    Active,
    Archived,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ChatChannelMemberRole {
    #[default]
    Owner,
    Manager,
    Member,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ChatChannelAuthorType {
    #[default]
    User,
    Agent,
    System,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatChannelDoc {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub channel_id: String,
    pub team_id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default)]
    pub visibility: ChatChannelVisibility,
    pub default_agent_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub document_folder_path: Option<String>,
    pub created_by_user_id: String,
    #[serde(default)]
    pub status: ChatChannelStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_message_preview: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_message_at: Option<bson::DateTime>,
    #[serde(default)]
    pub message_count: i32,
    #[serde(default)]
    pub thread_count: i32,
    #[serde(default)]
    pub is_processing: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_run_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_run_status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_run_error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_run_finished_at: Option<bson::DateTime>,
    pub created_at: bson::DateTime,
    pub updated_at: bson::DateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatChannelMemberDoc {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub channel_id: String,
    pub team_id: String,
    pub user_id: String,
    #[serde(default)]
    pub role: ChatChannelMemberRole,
    pub joined_at: bson::DateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatChannelMessageDoc {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub message_id: String,
    pub channel_id: String,
    pub team_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_root_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_message_id: Option<String>,
    #[serde(default)]
    pub author_type: ChatChannelAuthorType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author_user_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author_agent_id: Option<String>,
    #[serde(default)]
    pub author_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    pub content_text: String,
    #[serde(default)]
    pub content_blocks: serde_json::Value,
    #[serde(default)]
    pub metadata: serde_json::Value,
    #[serde(default)]
    pub visible: bool,
    pub created_at: bson::DateTime,
    pub updated_at: bson::DateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatChannelEventDoc {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub channel_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_root_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root_message_id: Option<String>,
    pub event_id: i64,
    pub event_type: String,
    pub payload: serde_json::Value,
    pub created_at: bson::DateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatChannelReadDoc {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub channel_id: String,
    pub team_id: String,
    pub user_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_read_message_id: Option<String>,
    pub last_read_at: bson::DateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatChannelSummary {
    pub channel_id: String,
    pub team_id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub visibility: ChatChannelVisibility,
    pub default_agent_id: String,
    #[serde(default)]
    pub default_agent_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub document_folder_path: Option<String>,
    pub created_by_user_id: String,
    pub status: ChatChannelStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_message_preview: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_message_at: Option<String>,
    pub message_count: i32,
    pub thread_count: i32,
    pub unread_count: i32,
    pub member_count: i32,
    pub is_processing: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_run_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_run_status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_run_error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_run_finished_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatChannelDetail {
    #[serde(flatten)]
    pub summary: ChatChannelSummary,
    pub current_user_role: ChatChannelMemberRole,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatChannelMemberResponse {
    pub channel_id: String,
    pub team_id: String,
    pub user_id: String,
    pub role: ChatChannelMemberRole,
    pub joined_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatChannelMessageResponse {
    pub message_id: String,
    pub channel_id: String,
    pub team_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_root_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_message_id: Option<String>,
    pub author_type: ChatChannelAuthorType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author_user_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author_agent_id: Option<String>,
    pub author_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    pub content_text: String,
    #[serde(default)]
    pub content_blocks: serde_json::Value,
    #[serde(default)]
    pub metadata: serde_json::Value,
    pub visible: bool,
    pub created_at: String,
    pub updated_at: String,
    #[serde(default)]
    pub reply_count: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatChannelThreadResponse {
    pub root_message: ChatChannelMessageResponse,
    pub messages: Vec<ChatChannelMessageResponse>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatChannelReadResponse {
    pub channel_id: String,
    pub user_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_read_message_id: Option<String>,
    pub last_read_at: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CreateChatChannelRequest {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub visibility: Option<ChatChannelVisibility>,
    pub default_agent_id: String,
    #[serde(default)]
    pub member_user_ids: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UpdateChatChannelRequest {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub description: Option<Option<String>>,
    #[serde(default)]
    pub visibility: Option<ChatChannelVisibility>,
    #[serde(default)]
    pub default_agent_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AddChatChannelMemberRequest {
    pub user_id: String,
    #[serde(default)]
    pub role: Option<ChatChannelMemberRole>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UpdateChatChannelMemberRequest {
    pub role: ChatChannelMemberRole,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ChannelMention {
    pub mention_type: String,
    pub target_id: String,
    #[serde(default)]
    pub label: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SendChatChannelMessageRequest {
    pub content: String,
    #[serde(default)]
    pub agent_id: Option<String>,
    #[serde(default)]
    pub thread_root_id: Option<String>,
    #[serde(default)]
    pub parent_message_id: Option<String>,
    #[serde(default)]
    pub mentions: Vec<ChannelMention>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MarkChatChannelReadRequest {
    #[serde(default)]
    pub last_read_message_id: Option<String>,
}

fn sanitize_text(value: &str, max_len: usize) -> String {
    value.trim().chars().take(max_len).collect()
}

fn sanitized_opt_text(value: Option<String>, max_len: usize) -> Option<String> {
    value.and_then(|raw| {
        let trimmed = sanitize_text(&raw, max_len);
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed)
        }
    })
}

fn sanitize_folder_segment(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.trim().chars() {
        match ch {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => out.push('-'),
            _ => out.push(ch),
        }
    }
    let collapsed = out
        .split_whitespace()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    collapsed.trim_matches('.').trim().chars().take(80).collect()
}

fn bson_now() -> bson::DateTime {
    bson::DateTime::from_chrono(Utc::now())
}

fn to_rfc3339(value: bson::DateTime) -> String {
    value.to_chrono().to_rfc3339()
}

pub struct ChatChannelService {
    db: Arc<MongoDb>,
}

impl ChatChannelService {
    pub fn new(db: Arc<MongoDb>) -> Self {
        Self { db }
    }

    fn channels(&self) -> mongodb::Collection<ChatChannelDoc> {
        self.db.collection("chat_channels")
    }

    fn members(&self) -> mongodb::Collection<ChatChannelMemberDoc> {
        self.db.collection("chat_channel_members")
    }

    fn messages(&self) -> mongodb::Collection<ChatChannelMessageDoc> {
        self.db.collection("chat_channel_messages")
    }

    fn events(&self) -> mongodb::Collection<ChatChannelEventDoc> {
        self.db.collection("chat_channel_events")
    }

    fn reads(&self) -> mongodb::Collection<ChatChannelReadDoc> {
        self.db.collection("chat_channel_reads")
    }

    async fn ensure_channel_document_folder(
        &self,
        team_id: &str,
        user_id: &str,
        channel_id: &str,
        channel_name: &str,
    ) -> Result<String, mongodb::error::Error> {
        let folder_service = FolderService::new((*self.db).clone());
        folder_service
            .ensure_system_folder(team_id, "团队频道", "/")
            .await
            .map_err(|err| mongodb::error::Error::custom(err.to_string()))?;

        let base_name = sanitize_folder_segment(channel_name);
        let folder_name = if base_name.is_empty() {
            format!("频道-{}", &channel_id[..8])
        } else {
            base_name
        };
        let primary_path = format!("/团队频道/{}", folder_name);
        let existing = folder_service
            .list(team_id, Some("/团队频道"))
            .await
            .map_err(|err| mongodb::error::Error::custom(err.to_string()))?;
        if existing.iter().any(|item| item.full_path == primary_path) {
            let fallback_name = format!("{}-{}", folder_name, &channel_id[..8]);
            let fallback_path = format!("/团队频道/{}", fallback_name);
            if !existing.iter().any(|item| item.full_path == fallback_path) {
                folder_service
                    .create(team_id, user_id, &fallback_name, "/团队频道")
                    .await
                    .map_err(|err| mongodb::error::Error::custom(err.to_string()))?;
                return Ok(fallback_path);
            }
            return Ok(fallback_path);
        }

        folder_service
            .create(team_id, user_id, &folder_name, "/团队频道")
            .await
            .map_err(|err| mongodb::error::Error::custom(err.to_string()))?;
        Ok(primary_path)
    }

    pub async fn ensure_indexes(&self) -> Result<(), mongodb::error::Error> {
        use mongodb::{options::IndexOptions, IndexModel};

        self.channels()
            .create_indexes(
                vec![
                    IndexModel::builder()
                        .keys(doc! { "channel_id": 1 })
                        .options(IndexOptions::builder().unique(true).build())
                        .build(),
                    IndexModel::builder()
                        .keys(doc! { "team_id": 1, "status": 1, "last_message_at": -1, "updated_at": -1 })
                        .build(),
                ],
                None,
            )
            .await?;
        self.members()
            .create_indexes(
                vec![
                    IndexModel::builder()
                        .keys(doc! { "channel_id": 1, "user_id": 1 })
                        .options(IndexOptions::builder().unique(true).build())
                        .build(),
                    IndexModel::builder()
                        .keys(doc! { "team_id": 1, "user_id": 1 })
                        .build(),
                ],
                None,
            )
            .await?;
        self.messages()
            .create_indexes(
                vec![
                    IndexModel::builder()
                        .keys(doc! { "message_id": 1 })
                        .options(IndexOptions::builder().unique(true).build())
                        .build(),
                    IndexModel::builder()
                        .keys(doc! { "channel_id": 1, "thread_root_id": 1, "created_at": 1 })
                        .build(),
                    IndexModel::builder()
                        .keys(doc! { "channel_id": 1, "parent_message_id": 1 })
                        .build(),
                ],
                None,
            )
            .await?;
        self.events()
            .create_indexes(
                vec![
                    IndexModel::builder()
                        .keys(doc! { "channel_id": 1, "event_id": 1 })
                        .build(),
                    IndexModel::builder()
                        .keys(doc! { "channel_id": 1, "run_id": 1, "event_id": 1 })
                        .build(),
                ],
                None,
            )
            .await?;
        self.reads()
            .create_indexes(
                vec![IndexModel::builder()
                    .keys(doc! { "channel_id": 1, "user_id": 1 })
                    .options(IndexOptions::builder().unique(true).build())
                    .build()],
                None,
            )
            .await?;
        Ok(())
    }

    pub async fn get_channel(
        &self,
        channel_id: &str,
    ) -> Result<Option<ChatChannelDoc>, mongodb::error::Error> {
        self.channels()
            .find_one(doc! { "channel_id": channel_id }, None)
            .await
    }

    pub async fn get_member_role(
        &self,
        channel_id: &str,
        user_id: &str,
    ) -> Result<Option<ChatChannelMemberRole>, mongodb::error::Error> {
        Ok(self
            .members()
            .find_one(doc! { "channel_id": channel_id, "user_id": user_id }, None)
            .await?
            .map(|doc| doc.role))
    }

    pub async fn create_channel(
        &self,
        team_id: &str,
        creator_user_id: &str,
        request: CreateChatChannelRequest,
    ) -> Result<ChatChannelDoc, mongodb::error::Error> {
        let now = bson_now();
        let channel_id = Uuid::new_v4().to_string();
        let document_folder_path = Some(
            self.ensure_channel_document_folder(
                team_id,
                creator_user_id,
                &channel_id,
                &request.name,
            )
            .await?,
        );
        let channel = ChatChannelDoc {
            id: None,
            channel_id,
            team_id: team_id.to_string(),
            name: sanitize_text(&request.name, 80),
            description: sanitized_opt_text(request.description, 280),
            visibility: request.visibility.unwrap_or_default(),
            default_agent_id: request.default_agent_id,
            document_folder_path,
            created_by_user_id: creator_user_id.to_string(),
            status: ChatChannelStatus::Active,
            last_message_preview: None,
            last_message_at: None,
            message_count: 0,
            thread_count: 0,
            is_processing: false,
            active_run_id: None,
            last_run_status: None,
            last_run_error: None,
            last_run_finished_at: None,
            created_at: now,
            updated_at: now,
        };
        self.channels().insert_one(&channel, None).await?;

        let mut seen = HashSet::from([creator_user_id.to_string()]);
        let mut members = vec![ChatChannelMemberDoc {
            id: None,
            channel_id: channel.channel_id.clone(),
            team_id: team_id.to_string(),
            user_id: creator_user_id.to_string(),
            role: ChatChannelMemberRole::Owner,
            joined_at: now,
        }];
        for user_id in request.member_user_ids {
            if seen.insert(user_id.clone()) {
                members.push(ChatChannelMemberDoc {
                    id: None,
                    channel_id: channel.channel_id.clone(),
                    team_id: team_id.to_string(),
                    user_id,
                    role: ChatChannelMemberRole::Member,
                    joined_at: now,
                });
            }
        }
        self.members().insert_many(members, None).await?;
        Ok(channel)
    }

    pub async fn list_members(
        &self,
        channel_id: &str,
    ) -> Result<Vec<ChatChannelMemberDoc>, mongodb::error::Error> {
        self.members()
            .find(
                doc! { "channel_id": channel_id },
                FindOptions::builder().sort(doc! { "joined_at": 1 }).build(),
            )
            .await?
            .try_collect()
            .await
    }

    pub async fn add_member(
        &self,
        channel: &ChatChannelDoc,
        user_id: &str,
        role: ChatChannelMemberRole,
    ) -> Result<ChatChannelMemberDoc, mongodb::error::Error> {
        let doc = ChatChannelMemberDoc {
            id: None,
            channel_id: channel.channel_id.clone(),
            team_id: channel.team_id.clone(),
            user_id: user_id.to_string(),
            role,
            joined_at: bson_now(),
        };
        self.members()
            .replace_one(
                doc! { "channel_id": &channel.channel_id, "user_id": user_id },
                &doc,
                mongodb::options::ReplaceOptions::builder()
                    .upsert(true)
                    .build(),
            )
            .await?;
        Ok(doc)
    }

    pub async fn update_member_role(
        &self,
        channel_id: &str,
        user_id: &str,
        role: ChatChannelMemberRole,
    ) -> Result<bool, mongodb::error::Error> {
        let result = self
            .members()
            .update_one(
                doc! { "channel_id": channel_id, "user_id": user_id },
                doc! { "$set": { "role": bson::to_bson(&role).unwrap_or(bson::Bson::String("member".into())) } },
                None,
            )
            .await?;
        Ok(result.matched_count > 0)
    }

    pub async fn remove_member(
        &self,
        channel_id: &str,
        user_id: &str,
    ) -> Result<bool, mongodb::error::Error> {
        let result = self
            .members()
            .delete_one(doc! { "channel_id": channel_id, "user_id": user_id }, None)
            .await?;
        Ok(result.deleted_count > 0)
    }

    fn to_summary(
        &self,
        channel: ChatChannelDoc,
        agent_name_by_id: &HashMap<String, String>,
        unread_count: i32,
        member_count: i32,
    ) -> ChatChannelSummary {
        ChatChannelSummary {
            channel_id: channel.channel_id.clone(),
            team_id: channel.team_id.clone(),
            name: channel.name.clone(),
            description: channel.description.clone(),
            visibility: channel.visibility,
            default_agent_id: channel.default_agent_id.clone(),
            default_agent_name: agent_name_by_id
                .get(&channel.default_agent_id)
                .cloned()
                .unwrap_or_else(|| channel.default_agent_id.clone()),
            document_folder_path: channel.document_folder_path.clone(),
            created_by_user_id: channel.created_by_user_id.clone(),
            status: channel.status,
            last_message_preview: channel.last_message_preview.clone(),
            last_message_at: channel.last_message_at.map(to_rfc3339),
            message_count: channel.message_count,
            thread_count: channel.thread_count,
            unread_count,
            member_count,
            is_processing: channel.is_processing,
            active_run_id: channel.active_run_id.clone(),
            last_run_status: channel.last_run_status.clone(),
            last_run_error: channel.last_run_error.clone(),
            last_run_finished_at: channel.last_run_finished_at.map(to_rfc3339),
            created_at: to_rfc3339(channel.created_at),
            updated_at: to_rfc3339(channel.updated_at),
        }
    }

    pub async fn list_channels_for_user(
        &self,
        team_id: &str,
        user_id: &str,
        is_team_admin: bool,
        agent_name_by_id: &HashMap<String, String>,
    ) -> Result<Vec<ChatChannelSummary>, mongodb::error::Error> {
        let private_channel_ids = if is_team_admin {
            HashSet::new()
        } else {
            self.members()
                .find(
                    doc! { "team_id": team_id, "user_id": user_id },
                    None,
                )
                .await?
                .try_collect::<Vec<ChatChannelMemberDoc>>()
                .await?
                .into_iter()
                .map(|item| item.channel_id)
                .collect::<HashSet<_>>()
        };

        let mut filter = doc! {
            "team_id": team_id,
            "status": "active",
        };
        if !is_team_admin {
            filter.insert(
                "$or",
                vec![
                    doc! { "visibility": "team_public" },
                    doc! { "channel_id": { "$in": private_channel_ids.iter().cloned().collect::<Vec<_>>() } },
                ],
            );
        }

        let channels: Vec<ChatChannelDoc> = self
            .channels()
            .find(
                filter,
                FindOptions::builder()
                    .sort(doc! { "last_message_at": -1, "updated_at": -1 })
                    .build(),
            )
            .await?
            .try_collect()
            .await?;

        let mut unread_by_channel = HashMap::new();
        let mut member_counts = HashMap::new();
        let channel_ids = channels.iter().map(|c| c.channel_id.clone()).collect::<Vec<_>>();
        if !channel_ids.is_empty() {
            let members: Vec<ChatChannelMemberDoc> = self
                .members()
                .find(doc! { "channel_id": { "$in": channel_ids.clone() } }, None)
                .await?
                .try_collect()
                .await?;
            for member in members {
                *member_counts.entry(member.channel_id).or_insert(0_i32) += 1;
            }
            for channel in &channels {
                unread_by_channel.insert(
                    channel.channel_id.clone(),
                    self.unread_count(&channel.channel_id, user_id).await?,
                );
            }
        }

        Ok(channels
            .into_iter()
            .map(|channel| {
                let channel_id = channel.channel_id.clone();
                self.to_summary(
                    channel,
                    agent_name_by_id,
                    unread_by_channel.get(&channel_id).copied().unwrap_or(0),
                    member_counts.get(&channel_id).copied().unwrap_or(0),
                )
            })
            .collect())
    }

    pub async fn build_detail(
        &self,
        channel: ChatChannelDoc,
        user_role: ChatChannelMemberRole,
        agent_name_by_id: &HashMap<String, String>,
        unread_count: i32,
        member_count: i32,
    ) -> ChatChannelDetail {
        ChatChannelDetail {
            summary: self.to_summary(channel, agent_name_by_id, unread_count, member_count),
            current_user_role: user_role,
        }
    }

    pub async fn update_channel(
        &self,
        channel_id: &str,
        request: UpdateChatChannelRequest,
    ) -> Result<Option<ChatChannelDoc>, mongodb::error::Error> {
        let mut set_doc = doc! {};
        if let Some(name) = request.name {
            let value = sanitize_text(&name, 80);
            if !value.is_empty() {
                set_doc.insert("name", value);
            }
        }
        if let Some(description) = request.description {
            set_doc.insert(
                "description",
                sanitized_opt_text(description, 280)
                    .map(bson::Bson::String)
                    .unwrap_or(bson::Bson::Null),
            );
        }
        if let Some(visibility) = request.visibility {
            set_doc.insert(
                "visibility",
                bson::to_bson(&visibility).unwrap_or(bson::Bson::String("team_public".into())),
            );
        }
        if let Some(default_agent_id) = request.default_agent_id {
            set_doc.insert("default_agent_id", default_agent_id);
        }
        if set_doc.is_empty() {
            return self.get_channel(channel_id).await;
        }
        set_doc.insert("updated_at", bson_now());
        self.channels()
            .find_one_and_update(
                doc! { "channel_id": channel_id },
                doc! { "$set": set_doc },
                FindOneAndUpdateOptions::builder()
                    .return_document(ReturnDocument::After)
                    .build(),
            )
            .await
    }

    pub async fn archive_channel(
        &self,
        channel_id: &str,
    ) -> Result<bool, mongodb::error::Error> {
        let result = self
            .channels()
            .update_one(
                doc! { "channel_id": channel_id },
                doc! { "$set": { "status": "archived", "updated_at": bson_now() } },
                None,
            )
            .await?;
        Ok(result.matched_count > 0)
    }

    pub async fn delete_channel(
        &self,
        channel_id: &str,
    ) -> Result<bool, mongodb::error::Error> {
        let deleted = self
            .channels()
            .delete_one(doc! { "channel_id": channel_id }, None)
            .await?;
        self.members()
            .delete_many(doc! { "channel_id": channel_id }, None)
            .await?;
        self.messages()
            .delete_many(doc! { "channel_id": channel_id }, None)
            .await?;
        self.events()
            .delete_many(doc! { "channel_id": channel_id }, None)
            .await?;
        self.reads()
            .delete_many(doc! { "channel_id": channel_id }, None)
            .await?;
        Ok(deleted.deleted_count > 0)
    }

    pub async fn get_message(
        &self,
        message_id: &str,
    ) -> Result<Option<ChatChannelMessageDoc>, mongodb::error::Error> {
        self.messages()
            .find_one(doc! { "message_id": message_id }, None)
            .await
    }

    pub async fn create_message(
        &self,
        channel: &ChatChannelDoc,
        thread_root_id: Option<String>,
        parent_message_id: Option<String>,
        author_type: ChatChannelAuthorType,
        author_user_id: Option<String>,
        author_agent_id: Option<String>,
        author_name: String,
        agent_id: Option<String>,
        content_text: String,
        content_blocks: serde_json::Value,
        metadata: serde_json::Value,
    ) -> Result<ChatChannelMessageDoc, mongodb::error::Error> {
        let now = bson_now();
        let doc = ChatChannelMessageDoc {
            id: None,
            message_id: Uuid::new_v4().to_string(),
            channel_id: channel.channel_id.clone(),
            team_id: channel.team_id.clone(),
            thread_root_id,
            parent_message_id,
            author_type,
            author_user_id,
            author_agent_id,
            author_name,
            agent_id,
            content_text: sanitize_text(&content_text, 32_000),
            content_blocks,
            metadata,
            visible: true,
            created_at: now,
            updated_at: now,
        };
        self.messages().insert_one(&doc, None).await?;
        Ok(doc)
    }

    fn to_message_response(
        &self,
        message: ChatChannelMessageDoc,
        reply_count: i32,
    ) -> ChatChannelMessageResponse {
        ChatChannelMessageResponse {
            message_id: message.message_id,
            channel_id: message.channel_id,
            team_id: message.team_id,
            thread_root_id: message.thread_root_id,
            parent_message_id: message.parent_message_id,
            author_type: message.author_type,
            author_user_id: message.author_user_id,
            author_agent_id: message.author_agent_id,
            author_name: message.author_name,
            agent_id: message.agent_id,
            content_text: message.content_text,
            content_blocks: message.content_blocks,
            metadata: message.metadata,
            visible: message.visible,
            created_at: to_rfc3339(message.created_at),
            updated_at: to_rfc3339(message.updated_at),
            reply_count,
        }
    }

    async fn reply_counts(
        &self,
        root_ids: &[String],
    ) -> Result<HashMap<String, i32>, mongodb::error::Error> {
        if root_ids.is_empty() {
            return Ok(HashMap::new());
        }
        let replies: Vec<ChatChannelMessageDoc> = self
            .messages()
            .find(doc! { "thread_root_id": { "$in": root_ids.to_vec() } }, None)
            .await?
            .try_collect()
            .await?;
        let mut counts = HashMap::new();
        for reply in replies {
            if let Some(root_id) = reply.thread_root_id {
                *counts.entry(root_id).or_insert(0) += 1;
            }
        }
        Ok(counts)
    }

    pub async fn list_root_messages(
        &self,
        channel_id: &str,
    ) -> Result<Vec<ChatChannelMessageResponse>, mongodb::error::Error> {
        let messages: Vec<ChatChannelMessageDoc> = self
            .messages()
            .find(
                doc! {
                    "channel_id": channel_id,
                    "$or": [
                        { "thread_root_id": { "$exists": false } },
                        { "thread_root_id": bson::Bson::Null }
                    ]
                },
                FindOptions::builder().sort(doc! { "created_at": 1 }).build(),
            )
            .await?
            .try_collect()
            .await?;

        let root_ids = messages
            .iter()
            .map(|item| item.message_id.clone())
            .collect::<Vec<_>>();
        let counts = self.reply_counts(&root_ids).await?;
        Ok(messages
            .into_iter()
            .map(|item| {
                let reply_count = counts.get(&item.message_id).copied().unwrap_or(0);
                self.to_message_response(item, reply_count)
            })
            .collect())
    }

    pub async fn list_thread_messages(
        &self,
        channel_id: &str,
        thread_root_id: &str,
    ) -> Result<Option<ChatChannelThreadResponse>, mongodb::error::Error> {
        let Some(root) = self
            .messages()
            .find_one(doc! { "channel_id": channel_id, "message_id": thread_root_id }, None)
            .await?
        else {
            return Ok(None);
        };
        let replies: Vec<ChatChannelMessageDoc> = self
            .messages()
            .find(
                doc! { "channel_id": channel_id, "thread_root_id": thread_root_id },
                FindOptions::builder().sort(doc! { "created_at": 1 }).build(),
            )
            .await?
            .try_collect()
            .await?;
        let reply_count = replies.len() as i32;
        Ok(Some(ChatChannelThreadResponse {
            root_message: self.to_message_response(root, reply_count),
            messages: replies
                .into_iter()
                .map(|item| self.to_message_response(item, 0))
                .collect(),
        }))
    }

    pub async fn mark_read(
        &self,
        channel: &ChatChannelDoc,
        user_id: &str,
        last_read_message_id: Option<String>,
    ) -> Result<ChatChannelReadDoc, mongodb::error::Error> {
        let doc = ChatChannelReadDoc {
            id: None,
            channel_id: channel.channel_id.clone(),
            team_id: channel.team_id.clone(),
            user_id: user_id.to_string(),
            last_read_message_id,
            last_read_at: bson_now(),
        };
        self.reads()
            .replace_one(
                doc! { "channel_id": &channel.channel_id, "user_id": user_id },
                &doc,
                mongodb::options::ReplaceOptions::builder()
                    .upsert(true)
                    .build(),
            )
            .await?;
        Ok(doc)
    }

    pub async fn unread_count(
        &self,
        channel_id: &str,
        user_id: &str,
    ) -> Result<i32, mongodb::error::Error> {
        let read = self
            .reads()
            .find_one(doc! { "channel_id": channel_id, "user_id": user_id }, None)
            .await?;
        let filter = if let Some(read_doc) = read {
            doc! {
                "channel_id": channel_id,
                "created_at": { "$gt": read_doc.last_read_at },
                "author_user_id": { "$ne": user_id }
            }
        } else {
            doc! {
                "channel_id": channel_id,
                "author_user_id": { "$ne": user_id }
            }
        };
        Ok(self.messages().count_documents(filter, None).await? as i32)
    }

    pub async fn set_processing(
        &self,
        channel_id: &str,
        run_id: &str,
        processing: bool,
    ) -> Result<(), mongodb::error::Error> {
        let now = bson_now();
        let mut set_doc = doc! {
            "is_processing": processing,
            "updated_at": now,
        };
        if processing {
            set_doc.insert("active_run_id", run_id);
            set_doc.insert("last_run_status", "running");
            set_doc.insert("last_run_error", bson::Bson::Null);
            set_doc.insert("last_run_finished_at", bson::Bson::Null);
        } else {
            set_doc.insert("active_run_id", bson::Bson::Null);
        }
        self.channels()
            .update_one(
                doc! { "channel_id": channel_id },
                doc! { "$set": set_doc },
                None,
            )
            .await?;
        Ok(())
    }

    pub async fn try_start_processing(
        &self,
        channel_id: &str,
        run_id: &str,
    ) -> Result<bool, mongodb::error::Error> {
        let result = self
            .channels()
            .update_one(
                doc! { "channel_id": channel_id, "is_processing": false, "status": "active" },
                doc! {
                    "$set": {
                        "is_processing": true,
                        "active_run_id": run_id,
                        "last_run_status": "running",
                        "last_run_error": bson::Bson::Null,
                        "last_run_finished_at": bson::Bson::Null,
                        "updated_at": bson_now(),
                    }
                },
                None,
            )
            .await?;
        Ok(result.matched_count > 0)
    }

    pub async fn update_run_result(
        &self,
        channel_id: &str,
        status: &str,
        error: Option<&str>,
    ) -> Result<(), mongodb::error::Error> {
        let now = bson_now();
        let mut set_doc = doc! {
            "is_processing": false,
            "active_run_id": bson::Bson::Null,
            "last_run_status": status,
            "last_run_finished_at": now,
            "updated_at": now,
        };
        set_doc.insert(
            "last_run_error",
            error
                .map(|value| bson::Bson::String(value.to_string()))
                .unwrap_or(bson::Bson::Null),
        );
        self.channels()
            .update_one(
                doc! { "channel_id": channel_id },
                doc! { "$set": set_doc },
                None,
            )
            .await?;
        Ok(())
    }

    pub async fn refresh_channel_stats(
        &self,
        channel_id: &str,
    ) -> Result<(), mongodb::error::Error> {
        let latest = self
            .messages()
            .find_one(
                doc! { "channel_id": channel_id },
                mongodb::options::FindOneOptions::builder()
                    .sort(doc! { "created_at": -1 })
                    .build(),
            )
            .await?;
        let message_count = self
            .messages()
            .count_documents(doc! { "channel_id": channel_id }, None)
            .await? as i32;
        let thread_count = self
            .messages()
            .count_documents(
                doc! {
                    "channel_id": channel_id,
                    "$or": [
                        { "thread_root_id": { "$exists": false } },
                        { "thread_root_id": bson::Bson::Null }
                    ]
                },
                None,
            )
            .await? as i32;
        let mut set_doc = doc! {
            "message_count": message_count,
            "thread_count": thread_count,
            "updated_at": bson_now(),
        };
        if let Some(message) = latest {
            set_doc.insert("last_message_preview", sanitize_text(&message.content_text, 200));
            set_doc.insert("last_message_at", message.created_at);
        }
        self.channels()
            .update_one(
                doc! { "channel_id": channel_id },
                doc! { "$set": set_doc },
                None,
            )
            .await?;
        Ok(())
    }

    pub async fn save_channel_events(
        &self,
        channel_id: &str,
        thread_root_id: Option<&str>,
        root_message_id: Option<&str>,
        records: &[(String, u64, super::task_manager::StreamEvent)],
    ) -> Result<(), mongodb::error::Error> {
        if records.is_empty() {
            return Ok(());
        }
        let docs = records
            .iter()
            .map(|(run_id, event_id, event)| ChatChannelEventDoc {
                id: None,
                channel_id: channel_id.to_string(),
                run_id: Some(run_id.clone()),
                thread_root_id: thread_root_id.map(|value| value.to_string()),
                root_message_id: root_message_id.map(|value| value.to_string()),
                event_id: (*event_id).try_into().unwrap_or(i64::MAX),
                event_type: event.event_type().to_string(),
                payload: serde_json::to_value(event)
                    .unwrap_or_else(|_| serde_json::json!({})),
                created_at: bson_now(),
            })
            .collect::<Vec<_>>();
        self.events().insert_many(docs, None).await?;
        Ok(())
    }

    pub async fn list_channel_events(
        &self,
        channel_id: &str,
        run_id: Option<&str>,
        limit: u32,
        descending: bool,
    ) -> Result<Vec<ChatChannelEventDoc>, mongodb::error::Error> {
        let mut filter = doc! { "channel_id": channel_id };
        if let Some(run_id) = run_id {
            filter.insert("run_id", run_id);
        }
        self.events()
            .find(
                filter,
                FindOptions::builder()
                    .sort(doc! { "event_id": if descending { -1 } else { 1 } })
                    .limit(limit as i64)
                    .build(),
            )
            .await?
            .try_collect()
            .await
    }
}
