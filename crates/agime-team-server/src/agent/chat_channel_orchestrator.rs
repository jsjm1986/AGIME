use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use tracing::{info, warn};

use agime_team::MongoDb;

use super::chat_channels::{
    ChatChannelAgentAutonomyMode, ChatChannelAuthorType, ChatChannelDisplayStatus,
    ChatChannelMessageDoc, ChatChannelMessageSurface, ChatChannelService, ChatChannelThreadState,
};
use super::service_mongo::AgentService;

struct FormalProgressSnapshot {
    latest_actor: String,
    latest_activity_at: chrono::DateTime<Utc>,
    latest_activity_label: String,
    summary: String,
}

pub struct ChatChannelOrchestratorService {
    db: Arc<MongoDb>,
    agent_service: Arc<AgentService>,
}

impl ChatChannelOrchestratorService {
    pub fn new(db: Arc<MongoDb>, agent_service: Arc<AgentService>) -> Self {
        Self { db, agent_service }
    }

    fn cadence_for(mode: ChatChannelAgentAutonomyMode) -> Duration {
        match mode {
            ChatChannelAgentAutonomyMode::Standard => Duration::from_secs(60 * 60),
            ChatChannelAgentAutonomyMode::Proactive => Duration::from_secs(30 * 60),
            ChatChannelAgentAutonomyMode::AgentLead => Duration::from_secs(15 * 60),
        }
    }

    fn quiet_window_for(mode: ChatChannelAgentAutonomyMode) -> Duration {
        match mode {
            ChatChannelAgentAutonomyMode::Standard => Duration::from_secs(5 * 60),
            ChatChannelAgentAutonomyMode::Proactive => Duration::from_secs(3 * 60),
            ChatChannelAgentAutonomyMode::AgentLead => Duration::from_secs(2 * 60),
        }
    }

    fn stalled_threshold_for(mode: ChatChannelAgentAutonomyMode) -> chrono::Duration {
        match mode {
            ChatChannelAgentAutonomyMode::Standard => chrono::Duration::hours(24),
            ChatChannelAgentAutonomyMode::Proactive => chrono::Duration::hours(8),
            ChatChannelAgentAutonomyMode::AgentLead => chrono::Duration::hours(2),
        }
    }

    fn formal_progress_quiet_window() -> chrono::Duration {
        chrono::Duration::hours(1)
    }

    fn clamp_summary(value: &str, max_chars: usize) -> String {
        let trimmed = value.trim();
        if trimmed.chars().count() <= max_chars {
            return trimmed.to_string();
        }
        trimmed.chars().take(max_chars).collect::<String>() + "…"
    }

    fn is_due(
        last_heartbeat_at: Option<mongodb::bson::DateTime>,
        mode: ChatChannelAgentAutonomyMode,
    ) -> bool {
        let Some(last) = last_heartbeat_at else {
            return true;
        };
        let elapsed = Utc::now() - last.to_chrono();
        elapsed
            .to_std()
            .map(|value| value >= Self::cadence_for(mode))
            .unwrap_or(true)
    }

    fn recent_result_card_exists(messages: &[ChatChannelMessageDoc], root_id: &str) -> bool {
        messages.iter().any(|message| {
            matches!(message.surface, ChatChannelMessageSurface::Activity)
                && message
                    .metadata
                    .as_object()
                    .and_then(|meta| meta.get("card_purpose"))
                    .and_then(|value| value.as_str())
                    == Some("collaboration_result_sync")
                && message
                    .metadata
                    .as_object()
                    .and_then(|meta| meta.get("linked_collaboration_id"))
                    .and_then(|value| value.as_str())
                    == Some(root_id)
        })
    }

    fn recent_card_exists(
        messages: &[ChatChannelMessageDoc],
        purpose: &str,
        root_id: Option<&str>,
        within: chrono::Duration,
    ) -> bool {
        let cutoff = Utc::now() - within;
        messages.iter().any(|message| {
            if message.created_at.to_chrono() < cutoff
                || !matches!(message.surface, ChatChannelMessageSurface::Activity)
            {
                return false;
            }
            let Some(meta) = message.metadata.as_object() else {
                return false;
            };
            if meta.get("card_purpose").and_then(|value| value.as_str()) != Some(purpose) {
                return false;
            }
            if let Some(root_id) = root_id {
                return meta
                    .get("linked_collaboration_id")
                    .and_then(|value| value.as_str())
                    == Some(root_id);
            }
            true
        })
    }

    fn recent_reminder_exists(
        messages: &[ChatChannelMessageDoc],
        root_id: &str,
        purpose: &str,
    ) -> bool {
        Self::recent_card_exists(messages, purpose, Some(root_id), chrono::Duration::hours(6))
    }

    fn recent_summary_exists(messages: &[ChatChannelMessageDoc]) -> bool {
        Self::recent_card_exists(
            messages,
            "discussion_summary",
            None,
            chrono::Duration::minutes(15),
        )
    }

    fn recent_suggestion_exists(messages: &[ChatChannelMessageDoc]) -> bool {
        [
            "manager_suggestion",
            "discussion_suggestion",
            "auto_started_notice",
            "auto_started_collaboration",
        ]
        .iter()
        .any(|purpose| {
            Self::recent_card_exists(messages, purpose, None, chrono::Duration::minutes(15))
        })
    }

    fn has_recent_formal_progress_card(
        messages: &[ChatChannelMessageDoc],
        root_id: &str,
        source_last_activity_at: &str,
    ) -> bool {
        messages.iter().any(|message| {
            if !matches!(message.surface, ChatChannelMessageSurface::Activity) {
                return false;
            }
            let Some(meta) = message.metadata.as_object() else {
                return false;
            };
            meta.get("card_purpose").and_then(|value| value.as_str())
                == Some("formal_collaboration_progress_sync")
                && meta
                    .get("linked_collaboration_id")
                    .and_then(|value| value.as_str())
                    == Some(root_id)
                && meta
                    .get("source_last_activity_at")
                    .and_then(|value| value.as_str())
                    == Some(source_last_activity_at)
        })
    }

    fn relative_activity_label(now: chrono::DateTime<Utc>, at: chrono::DateTime<Utc>) -> String {
        let elapsed = now - at;
        if elapsed.num_minutes() <= 1 {
            return "刚刚".to_string();
        }
        if elapsed.num_minutes() < 60 {
            return format!("{} 分钟前", elapsed.num_minutes());
        }
        if elapsed.num_hours() < 24 {
            return format!("{} 小时前", elapsed.num_hours());
        }
        format!("{} 天前", elapsed.num_days())
    }

    fn formal_progress_snapshot(
        root: &ChatChannelMessageDoc,
        thread: &super::chat_channels::ChatChannelThreadResponse,
    ) -> FormalProgressSnapshot {
        let latest_dialogue = thread
            .messages
            .iter()
            .rev()
            .find(|item| item.author_type != ChatChannelAuthorType::System)
            .unwrap_or(&thread.root_message);
        let latest_activity_at = chrono::DateTime::parse_from_rfc3339(&latest_dialogue.created_at)
            .map(|value| value.with_timezone(&Utc))
            .unwrap_or_else(|_| root.created_at.to_chrono());
        let latest_actor = latest_dialogue.author_name.clone();

        let summary_source = if !thread.root_message.summary_text.trim().is_empty() {
            thread.root_message.summary_text.as_str()
        } else if !latest_dialogue.content_text.trim().is_empty() {
            latest_dialogue.content_text.as_str()
        } else {
            root.content_text.as_str()
        };

        FormalProgressSnapshot {
            latest_actor,
            latest_activity_at,
            latest_activity_label: Self::relative_activity_label(Utc::now(), latest_activity_at),
            summary: Self::clamp_summary(summary_source, 160),
        }
    }

    async fn emit_formal_progress_card(
        &self,
        channel_service: &ChatChannelService,
        channel: &super::chat_channels::ChatChannelDoc,
        root: &ChatChannelMessageDoc,
        snapshot: &FormalProgressSnapshot,
    ) -> Result<(), mongodb::error::Error> {
        let default_agent_name = self
            .agent_service
            .get_agent(&channel.default_agent_id)
            .await
            .ok()
            .flatten()
            .map(|agent| agent.name)
            .unwrap_or_else(|| channel.default_agent_id.clone());
        let status_label = match root.collaboration_status {
            Some(ChatChannelDisplayStatus::AwaitingConfirmation) => "等你判断",
            Some(ChatChannelDisplayStatus::Adopted) => "已采用",
            Some(ChatChannelDisplayStatus::Rejected) => "未采用",
            Some(ChatChannelDisplayStatus::Proposed) => "建议",
            _ => "推进中",
        };
        let text = format!(
            "正式协作进度\n{}\n当前状态：{}\n最近推进：{} · {}\n这里先同步阶段进展，查看协作项可继续推进。",
            snapshot.summary,
            status_label,
            snapshot.latest_actor,
            snapshot.latest_activity_label,
        );
        let _ = channel_service
            .create_message(
                channel,
                None,
                Some(root.message_id.clone()),
                ChatChannelMessageSurface::Activity,
                ChatChannelThreadState::Active,
                root.collaboration_status,
                ChatChannelAuthorType::Agent,
                None,
                None,
                default_agent_name,
                root.agent_id.clone(),
                text.clone(),
                serde_json::json!([{ "type": "text", "text": text }]),
                serde_json::json!({
                    "discussion_card_kind": "progress",
                    "card_purpose": "formal_collaboration_progress_sync",
                    "linked_collaboration_id": root.message_id,
                    "summary_text": snapshot.summary,
                    "status_label": status_label,
                    "latest_actor": snapshot.latest_actor,
                    "source_last_activity_at": snapshot.latest_activity_at.to_rfc3339(),
                    "activity_age_minutes": (Utc::now() - snapshot.latest_activity_at).num_minutes(),
                }),
            )
            .await?;
        let _ = channel_service
            .record_orchestrator_heartbeat(
                channel,
                "scheduled_formal_progress_sync",
                Some(format!("progress:{}", root.message_id)),
            )
            .await;
        Ok(())
    }

    async fn emit_result_card(
        &self,
        channel_service: &ChatChannelService,
        channel: &super::chat_channels::ChatChannelDoc,
        root: &ChatChannelMessageDoc,
    ) -> Result<(), mongodb::error::Error> {
        let thread = channel_service
            .list_thread_messages(&channel.channel_id, &root.message_id)
            .await?;
        let Some(thread) = thread else {
            return Ok(());
        };
        let summary = if !thread.root_message.summary_text.trim().is_empty() {
            thread.root_message.summary_text.clone()
        } else {
            Self::clamp_summary(&thread.root_message.content_text, 160)
        };
        let latest_actor = thread
            .messages
            .iter()
            .rev()
            .find(|item| item.author_type != ChatChannelAuthorType::System)
            .map(|item| item.author_name.clone())
            .unwrap_or_else(|| thread.root_message.author_name.clone());
        let default_agent_name = self
            .agent_service
            .get_agent(&channel.default_agent_id)
            .await
            .ok()
            .flatten()
            .map(|agent| agent.name)
            .unwrap_or_else(|| channel.default_agent_id.clone());
        let text = format!(
            "协作结果同步\n{}\n当前状态：已采用\n最近推进者：{}",
            summary, latest_actor
        );
        let _ = channel_service
            .create_message(
                channel,
                None,
                Some(root.message_id.clone()),
                ChatChannelMessageSurface::Activity,
                ChatChannelThreadState::Active,
                Some(ChatChannelDisplayStatus::Adopted),
                ChatChannelAuthorType::Agent,
                None,
                None,
                default_agent_name,
                root.agent_id.clone(),
                text.clone(),
                serde_json::json!([{ "type": "text", "text": text }]),
                serde_json::json!({
                    "discussion_card_kind": "result",
                    "card_purpose": "collaboration_result_sync",
                    "linked_collaboration_id": root.message_id,
                    "summary_text": summary,
                    "status_label": "已采用",
                    "latest_actor": latest_actor,
                }),
            )
            .await?;
        let _ = channel_service
            .record_orchestrator_heartbeat(
                channel,
                "scheduled_result_sync",
                Some(format!("result:{}", root.message_id)),
            )
            .await;
        Ok(())
    }

    async fn emit_summary_card(
        &self,
        channel_service: &ChatChannelService,
        channel: &super::chat_channels::ChatChannelDoc,
        discussion_messages: &[&ChatChannelMessageDoc],
    ) -> Result<(), mongodb::error::Error> {
        if discussion_messages.is_empty() {
            return Ok(());
        }
        let default_agent_name = self
            .agent_service
            .get_agent(&channel.default_agent_id)
            .await
            .ok()
            .flatten()
            .map(|agent| agent.name)
            .unwrap_or_else(|| channel.default_agent_id.clone());
        let snippets = discussion_messages
            .iter()
            .rev()
            .take(3)
            .map(|item| Self::clamp_summary(&item.content_text, 48))
            .collect::<Vec<_>>();
        let body = snippets
            .iter()
            .enumerate()
            .map(|(idx, item)| format!("{}. {}", idx + 1, item))
            .collect::<Vec<_>>()
            .join("\n");
        let text = format!(
            "我先帮大家收一下目前讨论：\n{}\n如果你愿意，我可以继续把这部分整理成一个协作项，继续补全方案和下一步。",
            body
        );
        let linked_ids = discussion_messages
            .iter()
            .map(|item| serde_json::Value::String(item.message_id.clone()))
            .collect::<Vec<_>>();
        let _ = channel_service
            .create_message(
                channel,
                None,
                discussion_messages
                    .last()
                    .map(|item| item.message_id.clone()),
                ChatChannelMessageSurface::Activity,
                ChatChannelThreadState::Active,
                Some(ChatChannelDisplayStatus::Proposed),
                ChatChannelAuthorType::Agent,
                None,
                None,
                default_agent_name,
                Some(channel.default_agent_id.clone()),
                text.clone(),
                serde_json::json!([{ "type": "text", "text": text }]),
                serde_json::json!({
                    "discussion_card_kind": "summary",
                    "card_purpose": "discussion_summary",
                    "linked_message_ids": linked_ids,
                    "next_actions": ["开始协作"],
                }),
            )
            .await?;
        let _ = channel_service
            .record_orchestrator_heartbeat(channel, "discussion_summary", None)
            .await;
        Ok(())
    }

    async fn emit_reminder_card(
        &self,
        channel_service: &ChatChannelService,
        channel: &super::chat_channels::ChatChannelDoc,
        root: &ChatChannelMessageDoc,
        purpose: &str,
        text: String,
        display_status: ChatChannelDisplayStatus,
    ) -> Result<(), mongodb::error::Error> {
        let default_agent_name = self
            .agent_service
            .get_agent(&channel.default_agent_id)
            .await
            .ok()
            .flatten()
            .map(|agent| agent.name)
            .unwrap_or_else(|| channel.default_agent_id.clone());
        let _ = channel_service
            .create_message(
                channel,
                None,
                Some(root.message_id.clone()),
                ChatChannelMessageSurface::Activity,
                ChatChannelThreadState::Active,
                Some(display_status),
                ChatChannelAuthorType::Agent,
                None,
                None,
                default_agent_name,
                root.agent_id.clone(),
                text.clone(),
                serde_json::json!([{ "type": "text", "text": text }]),
                serde_json::json!({
                    "discussion_card_kind": "summary",
                    "card_purpose": purpose,
                    "linked_collaboration_id": root.message_id,
                }),
            )
            .await?;
        let _ = channel_service
            .record_orchestrator_heartbeat(
                channel,
                purpose,
                Some(format!("reminder:{}", root.message_id)),
            )
            .await;
        Ok(())
    }

    async fn evaluate_channel(
        &self,
        channel_service: &ChatChannelService,
        channel: &super::chat_channels::ChatChannelDoc,
    ) -> Result<(), mongodb::error::Error> {
        let Some(state) = channel_service
            .get_orchestrator_state(&channel.channel_id)
            .await?
        else {
            return Ok(());
        };
        if !Self::is_due(state.last_heartbeat_at, state.agent_autonomy_mode) {
            return Ok(());
        }
        let recent_roots = channel_service
            .list_recent_root_message_docs(&channel.channel_id, 80)
            .await?;

        let now = Utc::now();
        for root in recent_roots
            .iter()
            .filter(|item| !matches!(item.surface, ChatChannelMessageSurface::Activity))
        {
            if matches!(root.surface, ChatChannelMessageSurface::Issue)
                && matches!(root.thread_state, ChatChannelThreadState::Active)
                && !matches!(
                    root.collaboration_status,
                    Some(ChatChannelDisplayStatus::Adopted | ChatChannelDisplayStatus::Rejected)
                )
            {
                if let Some(thread) = channel_service
                    .list_thread_messages(&channel.channel_id, &root.message_id)
                    .await?
                {
                    let snapshot = Self::formal_progress_snapshot(root, &thread);
                    let silence = now - snapshot.latest_activity_at;
                    let source_last_activity_at = snapshot.latest_activity_at.to_rfc3339();
                    if silence >= Self::formal_progress_quiet_window()
                        && !Self::has_recent_formal_progress_card(
                            &recent_roots,
                            &root.message_id,
                            &source_last_activity_at,
                        )
                    {
                        let _ = self
                            .emit_formal_progress_card(channel_service, channel, root, &snapshot)
                            .await;
                        continue;
                    }
                }
            }
            let age = now - root.updated_at.to_chrono();
            if root.collaboration_status == Some(ChatChannelDisplayStatus::Adopted)
                && !Self::recent_result_card_exists(&recent_roots, &root.message_id)
            {
                let _ = self.emit_result_card(channel_service, channel, root).await;
                continue;
            }
            if root.collaboration_status == Some(ChatChannelDisplayStatus::AwaitingConfirmation)
                && age.num_minutes() >= 20
                && !Self::recent_reminder_exists(
                    &recent_roots,
                    &root.message_id,
                    "collaboration_awaiting_judgement_reminder",
                )
            {
                let text = format!(
                    "提醒：协作项「{}」还在等你判断。如果你已经有方向，可以直接进入协作项确认采用、未采用，或者继续推进。",
                    Self::clamp_summary(&root.content_text, 80)
                );
                let _ = self
                    .emit_reminder_card(
                        channel_service,
                        channel,
                        root,
                        "collaboration_awaiting_judgement_reminder",
                        text,
                        ChatChannelDisplayStatus::AwaitingConfirmation,
                    )
                    .await;
                continue;
            }
            if root.collaboration_status == Some(ChatChannelDisplayStatus::Active)
                && age >= Self::stalled_threshold_for(state.agent_autonomy_mode)
                && !Self::recent_reminder_exists(
                    &recent_roots,
                    &root.message_id,
                    "collaboration_stalled_reminder",
                )
            {
                let text = format!(
                    "提醒：协作项「{}」已经有一段时间没有新推进了。如果还要继续，我可以帮你梳理现状、补一版下一步，或提醒相关成员一起推进。",
                    Self::clamp_summary(&root.content_text, 80)
                );
                let _ = self
                    .emit_reminder_card(
                        channel_service,
                        channel,
                        root,
                        "collaboration_stalled_reminder",
                        text,
                        ChatChannelDisplayStatus::Active,
                    )
                    .await;
                continue;
            }
        }
        if let Some(trigger_message_id) = state.last_seen_discussion_message_id.as_deref() {
            let _ = self
                .maybe_emit_quiet_summary(channel_service, channel, trigger_message_id)
                .await;
        }
        let _ = channel_service
            .record_orchestrator_heartbeat(channel, "scheduled_sweep", None)
            .await;
        Ok(())
    }

    async fn maybe_emit_quiet_summary(
        &self,
        channel_service: &ChatChannelService,
        channel: &super::chat_channels::ChatChannelDoc,
        trigger_message_id: &str,
    ) -> Result<(), mongodb::error::Error> {
        let Some(state) = channel_service
            .get_orchestrator_state(&channel.channel_id)
            .await?
        else {
            return Ok(());
        };
        if state.last_seen_discussion_message_id.as_deref() != Some(trigger_message_id) {
            return Ok(());
        }
        let recent_roots = channel_service
            .list_recent_root_message_docs(&channel.channel_id, 80)
            .await?;
        if Self::recent_summary_exists(&recent_roots)
            || Self::recent_suggestion_exists(&recent_roots)
        {
            return Ok(());
        }
        let cutoff = Utc::now() - chrono::Duration::minutes(20);
        let discussion_messages = recent_roots
            .iter()
            .filter(|item| {
                matches!(item.surface, ChatChannelMessageSurface::Activity)
                    && matches!(item.author_type, ChatChannelAuthorType::User)
                    && item.created_at.to_chrono() >= cutoff
                    && item
                        .metadata
                        .as_object()
                        .and_then(|meta| meta.get("card_purpose"))
                        .is_none()
            })
            .collect::<Vec<_>>();
        let unique_users = discussion_messages
            .iter()
            .filter_map(|item| item.author_user_id.as_deref())
            .collect::<std::collections::HashSet<_>>();
        if discussion_messages.len() < 3 || unique_users.len() < 2 {
            return Ok(());
        }
        self.emit_summary_card(channel_service, channel, &discussion_messages)
            .await
    }

    pub fn spawn_quiet_window_summary(&self, channel_id: String, trigger_message_id: String) {
        let db = self.db.clone();
        let agent_service = self.agent_service.clone();
        tokio::spawn(async move {
            let channel_service = ChatChannelService::new(db.clone());
            let Some(channel) = channel_service
                .get_channel(&channel_id)
                .await
                .ok()
                .flatten()
            else {
                return;
            };
            let Some(state) = channel_service
                .get_orchestrator_state(&channel.channel_id)
                .await
                .ok()
                .flatten()
            else {
                return;
            };
            tokio::time::sleep(Self::quiet_window_for(state.agent_autonomy_mode)).await;
            let orchestrator = ChatChannelOrchestratorService::new(db, agent_service);
            if let Err(error) = orchestrator
                .maybe_emit_quiet_summary(&channel_service, &channel, &trigger_message_id)
                .await
            {
                warn!(
                    "chat channel orchestrator quiet-window summary failed for channel {}: {}",
                    channel.channel_id, error
                );
            }
        });
    }

    pub async fn run_sweep(&self) {
        let channel_service = ChatChannelService::new(self.db.clone());
        let channels = match channel_service.list_active_channels().await {
            Ok(channels) => channels,
            Err(error) => {
                warn!(
                    "chat channel orchestrator sweep failed to list active channels: {}",
                    error
                );
                return;
            }
        };
        for channel in channels {
            if let Err(error) = self.evaluate_channel(&channel_service, &channel).await {
                warn!(
                    "chat channel orchestrator sweep failed for channel {}: {}",
                    channel.channel_id, error
                );
            }
        }
        info!("chat channel orchestrator sweep completed");
    }
}
