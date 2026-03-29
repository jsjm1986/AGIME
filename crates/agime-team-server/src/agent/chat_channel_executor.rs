use std::sync::Arc;

use agime_team::MongoDb;
use anyhow::{anyhow, Result};
use tokio_util::sync::CancellationToken;

use super::chat_channel_manager::ChatChannelManager;
use super::chat_channels::{ChatChannelAuthorType, ChatChannelService};
use super::runtime;
use super::service_mongo::AgentService;
use super::task_manager::{StreamEvent, TaskManager};

pub struct ChatChannelExecutor {
    db: Arc<MongoDb>,
    channel_manager: Arc<ChatChannelManager>,
    agent_service: Arc<AgentService>,
    internal_task_manager: Arc<TaskManager>,
    workspace_root: String,
}

pub struct ExecuteChannelMessageRequest {
    pub channel_id: String,
    pub root_message_id: String,
    pub thread_root_id: Option<String>,
    pub assistant_parent_message_id: String,
    pub internal_session_id: String,
    pub agent_id: String,
    pub user_message: String,
}

impl ChatChannelExecutor {
    pub fn new(
        db: Arc<MongoDb>,
        channel_manager: Arc<ChatChannelManager>,
        workspace_root: String,
    ) -> Self {
        Self {
            agent_service: Arc::new(AgentService::new(db.clone())),
            internal_task_manager: Arc::new(TaskManager::new()),
            db,
            channel_manager,
            workspace_root,
        }
    }

    pub async fn execute_channel_message(
        &self,
        req: ExecuteChannelMessageRequest,
        cancel_token: CancellationToken,
    ) -> Result<()> {
        let exec_result = self.execute_channel_message_inner(&req, cancel_token).await;
        let (final_status, final_error) = match &exec_result {
            Ok(_) => ("completed", None),
            Err(error) => ("failed", Some(error.to_string())),
        };

        for attempt in 0..3 {
            match self
                .agent_service
                .update_session_execution_result(
                    &req.internal_session_id,
                    final_status,
                    final_error.as_deref(),
                )
                .await
            {
                Ok(_) => break,
                Err(error) => {
                    tracing::warn!(
                        "Failed to persist channel runtime session result for {} (attempt {}): {}",
                        req.internal_session_id,
                        attempt + 1,
                        error
                    );
                    if attempt < 2 {
                        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                    }
                }
            }
        }

        let channel_service = ChatChannelService::new(self.db.clone());
        let _ = channel_service
            .update_run_result(&req.channel_id, final_status, final_error.as_deref())
            .await;
        let _ = self
            .agent_service
            .delete_session_if_idle(&req.internal_session_id)
            .await;

        self.channel_manager
            .broadcast(
                &req.channel_id,
                StreamEvent::Done {
                    status: final_status.to_string(),
                    error: final_error.clone(),
                },
            )
            .await;
        self.channel_manager.complete(&req.channel_id).await;

        exec_result
    }

    async fn execute_channel_message_inner(
        &self,
        req: &ExecuteChannelMessageRequest,
        cancel_token: CancellationToken,
    ) -> Result<()> {
        let session = self
            .agent_service
            .get_session(&req.internal_session_id)
            .await
            .map_err(|e| anyhow!("DB error: {}", e))?
            .ok_or_else(|| anyhow!("Runtime session not found"))?;

        let workspace_path = if let Some(wp) = session.workspace_path.clone() {
            wp
        } else {
            let wp = runtime::create_workspace_dir(
                &self.workspace_root,
                &[
                    (&session.team_id, "team_id"),
                    ("channels", "category"),
                    (&req.channel_id, "channel_id"),
                ],
            )?;
            self.agent_service
                .set_session_workspace(&req.internal_session_id, &wp)
                .await
                .map_err(|e| anyhow!("Failed to set channel workspace: {}", e))?;
            wp
        };

        let exec_result = runtime::execute_via_bridge_with_turn_instruction(
            &self.db,
            &self.agent_service,
            &self.internal_task_manager,
            &self.channel_manager,
            &req.channel_id,
            &req.agent_id,
            &req.internal_session_id,
            &req.user_message,
            cancel_token,
            Some(&workspace_path),
            None,
            None,
            None,
            None,
            None,
        )
        .await;

        let channel_service = ChatChannelService::new(self.db.clone());
        let agent = self
            .agent_service
            .get_agent(&req.agent_id)
            .await
            .ok()
            .flatten();

        let reply_text = self
            .agent_service
            .get_session(&req.internal_session_id)
            .await
            .ok()
            .flatten()
            .and_then(|s| runtime::extract_last_assistant_text(&s.messages_json));

        match (&exec_result, reply_text) {
            (Ok(_), Some(text)) if !text.trim().is_empty() => {
                if let Some(channel) = channel_service.get_channel(&req.channel_id).await? {
                    let _ = channel_service
                        .create_message(
                            &channel,
                            req.thread_root_id.clone(),
                            Some(req.assistant_parent_message_id.clone()),
                            ChatChannelAuthorType::Agent,
                            None,
                            Some(req.agent_id.clone()),
                            agent
                                .as_ref()
                                .map(|item| item.name.clone())
                                .unwrap_or_else(|| req.agent_id.clone()),
                            Some(req.agent_id.clone()),
                            text.clone(),
                            serde_json::json!([{ "type": "text", "text": text }]),
                            serde_json::json!({
                                "channel_run_id": req.root_message_id,
                                "runtime_session_id": req.internal_session_id,
                            }),
                        )
                        .await;
                }
            }
            (Err(error), _) => {
                if let Some(channel) = channel_service.get_channel(&req.channel_id).await? {
                    let _ = channel_service
                        .create_message(
                            &channel,
                            req.thread_root_id.clone(),
                            Some(req.assistant_parent_message_id.clone()),
                            ChatChannelAuthorType::System,
                            None,
                            None,
                            "System".to_string(),
                            None,
                            format!("本轮执行失败：{}", error),
                            serde_json::json!([]),
                            serde_json::json!({
                                "channel_run_id": req.root_message_id,
                                "runtime_session_id": req.internal_session_id,
                            }),
                        )
                        .await;
                }
            }
            _ => {}
        }

        let _ = channel_service.refresh_channel_stats(&req.channel_id).await;
        exec_result
    }
}
