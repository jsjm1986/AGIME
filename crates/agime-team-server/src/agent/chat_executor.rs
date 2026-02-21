//! Chat executor for direct session-based chat (bypasses Task system)
//!
//! ChatExecutor handles chat message execution directly on sessions.
//! It internally creates a temporary task to reuse TaskExecutor's
//! proven execution logic, but the task is ephemeral and not persisted
//! to the agent_tasks collection.

use agime_team::MongoDb;
use anyhow::{anyhow, Result};

use std::sync::Arc;
use tokio_util::sync::CancellationToken;

use super::chat_manager::ChatManager;
use super::runtime;
use super::service_mongo::AgentService;
use super::task_manager::{StreamEvent, TaskManager};

/// Chat executor that reuses TaskExecutor logic for session-based chat
pub struct ChatExecutor {
    db: Arc<MongoDb>,
    chat_manager: Arc<ChatManager>,
    agent_service: Arc<AgentService>,
    /// Internal task manager used to drive TaskExecutor
    internal_task_manager: Arc<TaskManager>,
    workspace_root: String,
}

impl ChatExecutor {
    pub fn new(db: Arc<MongoDb>, chat_manager: Arc<ChatManager>, workspace_root: String) -> Self {
        let agent_service = Arc::new(AgentService::new(db.clone()));
        let internal_task_manager = Arc::new(TaskManager::new());
        Self {
            db,
            chat_manager,
            agent_service,
            internal_task_manager,
            workspace_root,
        }
    }

    /// Execute a chat message in a session (bypasses Task system).
    ///
    /// Strategy: We create a temporary task record in MongoDB, use
    /// TaskExecutor to run it, then bridge events from the internal
    /// TaskManager to the ChatManager. After completion, the temp
    /// task is cleaned up and session metadata is updated.
    ///
    /// C3 fix: All cleanup (is_processing, temp task, done event) is
    /// guaranteed via the inner method + cleanup-on-all-paths pattern.
    pub async fn execute_chat(
        &self,
        session_id: &str,
        agent_id: &str,
        user_message: &str,
        cancel_token: CancellationToken,
    ) -> Result<()> {
        // Run the inner logic; regardless of success/failure/panic,
        // we always clean up processing state and send done event.
        let exec_result = self
            .execute_chat_inner(session_id, agent_id, user_message, cancel_token)
            .await;

        // Guaranteed cleanup: clear processing flag (retry up to 3 times)
        for attempt in 0..3 {
            match self
                .agent_service
                .set_session_processing(session_id, false)
                .await
            {
                Ok(_) => break,
                Err(e) => {
                    tracing::warn!(
                        "Failed to clear is_processing for {} (attempt {}): {}",
                        session_id, attempt + 1, e
                    );
                    if attempt < 2 {
                        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                    }
                }
            }
        }

        // Send done event to chat subscribers
        match &exec_result {
            Ok(_) => {
                self.chat_manager
                    .broadcast(
                        session_id,
                        StreamEvent::Done {
                            status: "completed".to_string(),
                            error: None,
                        },
                    )
                    .await;
            }
            Err(e) => {
                self.chat_manager
                    .broadcast(
                        session_id,
                        StreamEvent::Done {
                            status: "failed".to_string(),
                            error: Some(e.to_string()),
                        },
                    )
                    .await;
            }
        }

        // Complete in chat manager (removes from active tracking)
        self.chat_manager.complete(session_id).await;

        exec_result
    }

    /// Inner execution logic, separated so that cleanup is guaranteed
    /// by the caller (`execute_chat`) regardless of how this returns.
    async fn execute_chat_inner(
        &self,
        session_id: &str,
        agent_id: &str,
        user_message: &str,
        cancel_token: CancellationToken,
    ) -> Result<()> {
        // Resolve workspace_path for this chat session
        let session = self
            .agent_service
            .get_session(session_id)
            .await
            .map_err(|e| anyhow!("DB error: {}", e))?
            .ok_or_else(|| anyhow!("Session not found"))?;

        let workspace_path = if let Some(wp) = session.workspace_path.clone() {
            wp
        } else {
            let wp = runtime::create_workspace_dir(
                &self.workspace_root,
                &[
                    (&session.team_id, "team_id"),
                    ("chats", "category"),
                    (session_id, "session_id"),
                ],
            )?;
            self.agent_service
                .set_session_workspace(session_id, &wp)
                .await
                .map_err(|e| anyhow!("Failed to set workspace: {}", e))?;
            wp
        };

        // Execute via shared bridge pattern
        let exec_result = runtime::execute_via_bridge(
            &self.db,
            &self.agent_service,
            &self.internal_task_manager,
            &self.chat_manager,
            session_id,
            agent_id,
            session_id,
            user_message,
            cancel_token,
            Some(&workspace_path),
            None,
            None, // no mission_context for chat
        )
        .await;

        // Chat-specific: update session metadata after execution
        self.update_session_metadata(session_id, user_message).await;

        exec_result
    }

    /// Update session metadata after message completion
    async fn update_session_metadata(&self, session_id: &str, user_message: &str) {
        // Load updated session to get latest messages
        let session = match self.agent_service.get_session(session_id).await {
            Ok(Some(s)) => s,
            _ => return,
        };

        // Auto-generate title from first user message if not set
        let title = if session.title.is_none() {
            // H2 fix: use chars() for safe Unicode slicing
            let preview: String = if user_message.chars().count() > 50 {
                let truncated: String = user_message.chars().take(47).collect();
                format!("{}...", truncated)
            } else {
                user_message.to_string()
            };
            Some(preview)
        } else {
            None
        };

        // Build last message preview from assistant response
        let preview = Self::extract_last_assistant_preview(&session.messages_json);

        let _ = self
            .agent_service
            .update_session_after_message(
                session_id,
                &session.messages_json,
                session.message_count,
                &preview,
                title.as_deref(),
                session.total_tokens,
            )
            .await;
    }

    /// Extract preview text from the last assistant message
    fn extract_last_assistant_preview(messages_json: &str) -> String {
        if let Ok(msgs) = serde_json::from_str::<Vec<serde_json::Value>>(messages_json) {
            for msg in msgs.iter().rev() {
                if msg.get("role").and_then(|r| r.as_str()) == Some("assistant") {
                    if let Some(content) = msg.get("content") {
                        if let Some(text) = content.as_str() {
                            return text.chars().take(200).collect();
                        }
                        // Handle array content format
                        if let Some(arr) = content.as_array() {
                            for item in arr {
                                if let Some(text) = item.get("text").and_then(|t| t.as_str()) {
                                    if !text.is_empty() {
                                        return text.chars().take(200).collect();
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        String::new()
    }
}
