use std::fs;
use std::path::Path;
use std::sync::Arc;

use agime_team::MongoDb;
use anyhow::{anyhow, Context, Result};
use futures::TryStreamExt;
use mongodb::bson::{self, doc, oid::ObjectId, Regex};
use mongodb::options::{FindOneAndUpdateOptions, FindOneOptions, FindOptions, ReturnDocument};
use serde::{Deserialize, Serialize};

use super::chat_channels::{ChatChannelDoc, ChatChannelRepoStorageMode, ChatChannelType};
use super::session_mongo::AgentSessionDoc;

const DETACHED_RETENTION_DAYS: i64 = 30;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ChannelProjectWorkspaceLifecycleState {
    ActiveBound,
    Detached,
    Archived,
    PendingDelete,
    Deleted,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelProjectWorkspaceDoc {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub workspace_id: String,
    pub team_id: String,
    pub channel_id: String,
    pub channel_name_snapshot: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_display_name: Option<String>,
    pub workspace_path: String,
    pub repo_path: String,
    pub main_checkout_path: String,
    pub bare_repo_path: String,
    pub repo_default_branch: String,
    pub repo_storage_mode: ChatChannelRepoStorageMode,
    pub channel_type_snapshot: ChatChannelType,
    pub lifecycle_state: ChannelProjectWorkspaceLifecycleState,
    pub created_at: bson::DateTime,
    pub updated_at: bson::DateTime,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_bound_at: Option<bson::DateTime>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_activity_at: Option<bson::DateTime>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detached_at: Option<bson::DateTime>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archived_at: Option<bson::DateTime>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending_delete_at: Option<bson::DateTime>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deleted_at: Option<bson::DateTime>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retention_until: Option<bson::DateTime>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_detach_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatChannelWorkspaceGovernanceSummary {
    pub has_detached_workspace: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detached_workspace_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lifecycle_state: Option<ChannelProjectWorkspaceLifecycleState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_display_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub main_checkout_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo_default_branch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_detached_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retention_until: Option<String>,
}

#[derive(Clone)]
pub struct ChannelWorkspaceGovernanceService {
    db: Arc<MongoDb>,
}

impl ChannelWorkspaceGovernanceService {
    pub fn new(db: Arc<MongoDb>) -> Self {
        Self { db }
    }

    fn workspaces(&self) -> mongodb::Collection<ChannelProjectWorkspaceDoc> {
        self.db.collection("channel_project_workspaces")
    }

    fn sessions(&self) -> mongodb::Collection<AgentSessionDoc> {
        self.db.collection("agent_sessions")
    }

    fn channels(&self) -> mongodb::Collection<ChatChannelDoc> {
        self.db.collection("chat_channels")
    }

    pub async fn ensure_indexes(&self) -> Result<(), mongodb::error::Error> {
        use mongodb::{options::IndexOptions, IndexModel};

        self.workspaces()
            .create_indexes(
                vec![
                    IndexModel::builder()
                        .keys(doc! { "workspace_id": 1 })
                        .options(IndexOptions::builder().unique(true).build())
                        .build(),
                    IndexModel::builder()
                        .keys(doc! { "channel_id": 1, "lifecycle_state": 1, "updated_at": -1 })
                        .build(),
                    IndexModel::builder()
                        .keys(doc! { "team_id": 1, "lifecycle_state": 1, "updated_at": -1 })
                        .build(),
                ],
                None,
            )
            .await?;
        Ok(())
    }

    pub async fn register_active_binding(
        &self,
        channel: &ChatChannelDoc,
    ) -> Result<Option<ChannelProjectWorkspaceDoc>, mongodb::error::Error> {
        let Some(workspace_id) = channel
            .workspace_id
            .as_deref()
            .filter(|value| !value.trim().is_empty())
        else {
            return Ok(None);
        };
        let Some(workspace_path) = channel
            .workspace_path
            .as_deref()
            .filter(|value| !value.trim().is_empty())
        else {
            return Ok(None);
        };
        let Some(repo_path) = channel
            .repo_path
            .as_deref()
            .filter(|value| !value.trim().is_empty())
        else {
            return Ok(None);
        };
        let Some(main_checkout_path) = channel
            .main_checkout_path
            .as_deref()
            .or(channel.repo_root.as_deref())
            .filter(|value| !value.trim().is_empty())
        else {
            return Ok(None);
        };
        let bare_repo_path = Path::new(repo_path)
            .join("bare.git")
            .to_string_lossy()
            .to_string();
        let now = bson::DateTime::now();

        self.workspaces()
            .find_one_and_update(
                doc! { "workspace_id": workspace_id },
                doc! {
                    "$set": {
                        "team_id": &channel.team_id,
                        "channel_id": &channel.channel_id,
                        "channel_name_snapshot": &channel.name,
                        "workspace_display_name": channel.workspace_display_name.clone(),
                        "workspace_path": workspace_path,
                        "repo_path": repo_path,
                        "main_checkout_path": main_checkout_path,
                        "bare_repo_path": &bare_repo_path,
                        "repo_default_branch": channel.repo_default_branch.clone().unwrap_or_else(|| "main".to_string()),
                        "repo_storage_mode": bson::to_bson(&channel.repo_storage_mode.unwrap_or(ChatChannelRepoStorageMode::BareRepoWorktree)).unwrap_or(bson::Bson::String("bare_repo_worktree".to_string())),
                        "channel_type_snapshot": bson::to_bson(&channel.channel_type).unwrap_or(bson::Bson::String("coding".to_string())),
                        "lifecycle_state": bson::to_bson(&ChannelProjectWorkspaceLifecycleState::ActiveBound).unwrap_or(bson::Bson::String("active_bound".to_string())),
                        "updated_at": now,
                        "last_bound_at": now,
                        "last_activity_at": now,
                        "detached_at": bson::Bson::Null,
                        "archived_at": bson::Bson::Null,
                        "pending_delete_at": bson::Bson::Null,
                        "deleted_at": bson::Bson::Null,
                        "retention_until": bson::Bson::Null,
                        "last_detach_reason": bson::Bson::Null
                    },
                    "$setOnInsert": {
                        "created_at": now
                    }
                },
                FindOneAndUpdateOptions::builder()
                    .upsert(true)
                    .return_document(ReturnDocument::After)
                    .build(),
            )
            .await
    }

    pub async fn latest_restorable_workspace_for_channel(
        &self,
        channel_id: &str,
    ) -> Result<Option<ChannelProjectWorkspaceDoc>, mongodb::error::Error> {
        self.workspaces()
            .find_one(
                doc! {
                    "channel_id": channel_id,
                    "lifecycle_state": bson::to_bson(&ChannelProjectWorkspaceLifecycleState::Detached).unwrap_or(bson::Bson::String("detached".to_string()))
                },
                FindOneOptions::builder()
                    .sort(doc! { "detached_at": -1, "updated_at": -1 })
                    .build(),
            )
            .await
    }

    pub async fn get_workspace(
        &self,
        workspace_id: &str,
    ) -> Result<Option<ChannelProjectWorkspaceDoc>, mongodb::error::Error> {
        self.workspaces()
            .find_one(doc! { "workspace_id": workspace_id }, None)
            .await
    }

    pub async fn summarize_for_channel(
        &self,
        channel_id: &str,
    ) -> Result<Option<ChatChannelWorkspaceGovernanceSummary>, mongodb::error::Error> {
        let workspace = self
            .workspaces()
            .find_one(
                doc! {
                    "channel_id": channel_id,
                    "lifecycle_state": {
                        "$in": [
                            bson::to_bson(&ChannelProjectWorkspaceLifecycleState::Detached).unwrap_or(bson::Bson::String("detached".to_string())),
                            bson::to_bson(&ChannelProjectWorkspaceLifecycleState::Archived).unwrap_or(bson::Bson::String("archived".to_string())),
                            bson::to_bson(&ChannelProjectWorkspaceLifecycleState::PendingDelete).unwrap_or(bson::Bson::String("pending_delete".to_string()))
                        ]
                    }
                },
                FindOneOptions::builder()
                    .sort(doc! { "updated_at": -1 })
                    .build(),
            )
            .await?;

        Ok(workspace.map(|item| ChatChannelWorkspaceGovernanceSummary {
            has_detached_workspace: true,
            detached_workspace_id: Some(item.workspace_id),
            lifecycle_state: Some(item.lifecycle_state),
            workspace_display_name: item.workspace_display_name,
            workspace_path: Some(item.workspace_path),
            repo_path: Some(item.repo_path),
            main_checkout_path: Some(item.main_checkout_path),
            repo_default_branch: Some(item.repo_default_branch),
            last_detached_at: item.detached_at.map(to_rfc3339),
            retention_until: item.retention_until.map(to_rfc3339),
        }))
    }

    pub async fn list_team_workspaces(
        &self,
        team_id: &str,
        lifecycle_state: Option<ChannelProjectWorkspaceLifecycleState>,
    ) -> Result<Vec<ChannelProjectWorkspaceDoc>, mongodb::error::Error> {
        let mut filter = doc! { "team_id": team_id };
        if let Some(state) = lifecycle_state {
            filter.insert(
                "lifecycle_state",
                bson::to_bson(&state).unwrap_or(bson::Bson::String("detached".to_string())),
            );
        } else {
            filter.insert(
                "lifecycle_state",
                doc! {
                    "$ne": bson::to_bson(&ChannelProjectWorkspaceLifecycleState::Deleted).unwrap_or(bson::Bson::String("deleted".to_string()))
                },
            );
        }

        self.workspaces()
            .find(
                filter,
                FindOptions::builder()
                    .sort(doc! { "updated_at": -1 })
                    .build(),
            )
            .await?
            .try_collect()
            .await
    }

    pub async fn detach_workspace(
        &self,
        channel: &ChatChannelDoc,
        reason: Option<&str>,
    ) -> Result<Option<ChannelProjectWorkspaceDoc>, mongodb::error::Error> {
        let Some(workspace_id) = channel
            .workspace_id
            .as_deref()
            .filter(|value| !value.trim().is_empty())
        else {
            return Ok(None);
        };
        let now = bson::DateTime::now();
        let retention_until = bson::DateTime::from_chrono(
            chrono::Utc::now() + chrono::Duration::days(DETACHED_RETENTION_DAYS),
        );
        self.workspaces()
            .find_one_and_update(
                doc! { "workspace_id": workspace_id },
                doc! {
                    "$set": {
                        "channel_name_snapshot": &channel.name,
                        "channel_type_snapshot": bson::to_bson(&channel.channel_type).unwrap_or(bson::Bson::String("coding".to_string())),
                        "lifecycle_state": bson::to_bson(&ChannelProjectWorkspaceLifecycleState::Detached).unwrap_or(bson::Bson::String("detached".to_string())),
                        "detached_at": now,
                        "retention_until": retention_until,
                        "last_detach_reason": reason.unwrap_or("channel_downgraded"),
                        "updated_at": now
                    }
                },
                FindOneAndUpdateOptions::builder()
                    .return_document(ReturnDocument::After)
                    .build(),
            )
            .await
    }

    pub async fn archive_workspace(
        &self,
        workspace_id: &str,
        team_id: &str,
    ) -> Result<Option<ChannelProjectWorkspaceDoc>, mongodb::error::Error> {
        let now = bson::DateTime::now();
        self.workspaces()
            .find_one_and_update(
                doc! {
                    "workspace_id": workspace_id,
                    "team_id": team_id,
                    "lifecycle_state": {
                        "$in": [
                            bson::to_bson(&ChannelProjectWorkspaceLifecycleState::Detached).unwrap_or(bson::Bson::String("detached".to_string())),
                            bson::to_bson(&ChannelProjectWorkspaceLifecycleState::PendingDelete).unwrap_or(bson::Bson::String("pending_delete".to_string()))
                        ]
                    }
                },
                doc! {
                    "$set": {
                        "lifecycle_state": bson::to_bson(&ChannelProjectWorkspaceLifecycleState::Archived).unwrap_or(bson::Bson::String("archived".to_string())),
                        "archived_at": now,
                        "updated_at": now
                    }
                },
                FindOneAndUpdateOptions::builder()
                    .return_document(ReturnDocument::After)
                    .build(),
            )
            .await
    }

    pub async fn mark_workspace_pending_delete(
        &self,
        workspace_id: &str,
        team_id: &str,
    ) -> Result<Option<ChannelProjectWorkspaceDoc>, mongodb::error::Error> {
        let now = bson::DateTime::now();
        self.workspaces()
            .find_one_and_update(
                doc! {
                    "workspace_id": workspace_id,
                    "team_id": team_id,
                    "lifecycle_state": {
                        "$in": [
                            bson::to_bson(&ChannelProjectWorkspaceLifecycleState::Detached).unwrap_or(bson::Bson::String("detached".to_string())),
                            bson::to_bson(&ChannelProjectWorkspaceLifecycleState::Archived).unwrap_or(bson::Bson::String("archived".to_string()))
                        ]
                    }
                },
                doc! {
                    "$set": {
                        "lifecycle_state": bson::to_bson(&ChannelProjectWorkspaceLifecycleState::PendingDelete).unwrap_or(bson::Bson::String("pending_delete".to_string())),
                        "pending_delete_at": now,
                        "updated_at": now
                    }
                },
                FindOneAndUpdateOptions::builder()
                    .return_document(ReturnDocument::After)
                    .build(),
            )
            .await
    }

    pub async fn restore_workspace_binding(
        &self,
        workspace_id: &str,
        team_id: &str,
    ) -> Result<Option<ChannelProjectWorkspaceDoc>, mongodb::error::Error> {
        let now = bson::DateTime::now();
        self.workspaces()
            .find_one_and_update(
                doc! {
                    "workspace_id": workspace_id,
                    "team_id": team_id,
                    "lifecycle_state": {
                        "$in": [
                            bson::to_bson(&ChannelProjectWorkspaceLifecycleState::Detached).unwrap_or(bson::Bson::String("detached".to_string())),
                            bson::to_bson(&ChannelProjectWorkspaceLifecycleState::Archived).unwrap_or(bson::Bson::String("archived".to_string()))
                        ]
                    }
                },
                doc! {
                    "$set": {
                        "lifecycle_state": bson::to_bson(&ChannelProjectWorkspaceLifecycleState::ActiveBound).unwrap_or(bson::Bson::String("active_bound".to_string())),
                        "last_bound_at": now,
                        "last_activity_at": now,
                        "updated_at": now,
                        "detached_at": bson::Bson::Null,
                        "archived_at": bson::Bson::Null,
                        "pending_delete_at": bson::Bson::Null,
                        "deleted_at": bson::Bson::Null,
                        "retention_until": bson::Bson::Null,
                        "last_detach_reason": bson::Bson::Null
                    }
                },
                FindOneAndUpdateOptions::builder()
                    .return_document(ReturnDocument::After)
                    .build(),
            )
            .await
    }

    pub async fn delete_workspace(
        &self,
        workspace_id: &str,
        team_id: &str,
    ) -> Result<Option<ChannelProjectWorkspaceDoc>> {
        let Some(pending) = self
            .mark_workspace_pending_delete(workspace_id, team_id)
            .await
            .context("failed to mark workspace pending delete")?
        else {
            return Ok(None);
        };

        if self
            .workspace_has_active_channel_binding(workspace_id)
            .await?
        {
            return Err(anyhow!("workspace is still actively bound to a channel"));
        }
        let active_refs = self.count_active_session_refs(&pending).await?;
        if active_refs > 0 {
            return Err(anyhow!(
                "workspace still has active runtime session references"
            ));
        }

        ensure_safe_workspace_root(&pending.workspace_path)?;
        let workspace_root = Path::new(&pending.workspace_path);
        if workspace_root.exists() {
            fs::remove_dir_all(workspace_root).with_context(|| {
                format!(
                    "failed to delete detached workspace directory {}",
                    pending.workspace_path
                )
            })?;
        }

        let now = bson::DateTime::now();
        let deleted = self
            .workspaces()
            .find_one_and_update(
                doc! { "workspace_id": workspace_id, "team_id": team_id },
                doc! {
                    "$set": {
                        "lifecycle_state": bson::to_bson(&ChannelProjectWorkspaceLifecycleState::Deleted).unwrap_or(bson::Bson::String("deleted".to_string())),
                        "deleted_at": now,
                        "updated_at": now
                    }
                },
                FindOneAndUpdateOptions::builder()
                    .return_document(ReturnDocument::After)
                    .build(),
            )
            .await
            .context("failed to mark workspace deleted")?;
        Ok(deleted)
    }

    async fn workspace_has_active_channel_binding(&self, workspace_id: &str) -> Result<bool> {
        Ok(self
            .channels()
            .find_one(
                doc! {
                    "workspace_id": workspace_id,
                    "channel_type": bson::to_bson(&ChatChannelType::Coding).unwrap_or(bson::Bson::String("coding".to_string()))
                },
                None,
            )
            .await?
            .is_some())
    }

    async fn count_active_session_refs(
        &self,
        workspace: &ChannelProjectWorkspaceDoc,
    ) -> Result<u64> {
        let escaped_workspace = regex::escape(&workspace.workspace_path);
        let filter = doc! {
            "status": "active",
            "$or": [
                { "workspace_id": &workspace.workspace_id },
                { "workspace_path": Regex { pattern: format!("^{}($|/)", escaped_workspace), options: String::new() } },
                { "thread_repo_ref": &workspace.bare_repo_path }
            ]
        };
        Ok(self.sessions().count_documents(filter, None).await?)
    }
}

fn ensure_safe_workspace_root(path: &str) -> Result<()> {
    let root = Path::new(path);
    if !root.is_absolute() {
        return Err(anyhow!("workspace path must be absolute"));
    }
    let path_str = root.to_string_lossy();
    if !path_str.contains("/channels/") || !path_str.ends_with("/project") {
        return Err(anyhow!(
            "workspace path does not look like a managed channel project root"
        ));
    }
    Ok(())
}

fn to_rfc3339(value: bson::DateTime) -> String {
    value.to_chrono().to_rfc3339()
}
