use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use serde_json::Value;
use tracing::{info, warn};

use agime_team::MongoDb;

use super::channel_coding_cards::{attach_card_domain_metadata, build_thread_coding_payload};
use super::chat_channels::{
    ChatChannelAgentAutonomyMode, ChatChannelAuthorType, ChatChannelCardEmissionFamily,
    ChatChannelCardEmissionRegistry, ChatChannelDisplayStatus, ChatChannelMessageDoc,
    ChatChannelMessageResponse, ChatChannelMessageSurface, ChatChannelOrchestratorStateDoc,
    ChatChannelService, ChatChannelThreadState,
};
use super::service_mongo::AgentService;

struct FormalProgressSnapshot {
    latest_actor: String,
    latest_activity_at: chrono::DateTime<Utc>,
    latest_activity_label: String,
    latest_activity_message_id: String,
    summary: String,
    summary_revision: String,
    progress_revision: String,
    result_revision: String,
    coding_payload: Option<Value>,
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

    fn registry_matches(
        state: &ChatChannelOrchestratorStateDoc,
        family: ChatChannelCardEmissionFamily,
        key: &str,
        revision: &str,
    ) -> bool {
        let normalized_key = ChatChannelCardEmissionRegistry::normalized_key(key);
        match family {
            ChatChannelCardEmissionFamily::Onboarding => {
                state.card_emission_registry.onboarding_revision.as_deref() == Some(revision)
            }
            ChatChannelCardEmissionFamily::Suggestion => state
                .card_emission_registry
                .suggestion_revisions
                .get(&normalized_key)
                .is_some_and(|value| value == revision),
            ChatChannelCardEmissionFamily::Progress => state
                .card_emission_registry
                .progress_revisions
                .get(&normalized_key)
                .is_some_and(|value| value == revision),
            ChatChannelCardEmissionFamily::Result => state
                .card_emission_registry
                .result_revisions
                .get(&normalized_key)
                .is_some_and(|value| value == revision),
            ChatChannelCardEmissionFamily::Reminder => state
                .card_emission_registry
                .reminder_revisions
                .get(&normalized_key)
                .is_some_and(|value| value == revision),
        }
    }

    fn normalize_revision_component(value: &str, max_chars: usize) -> String {
        Self::clamp_summary(
            &value.split_whitespace().collect::<Vec<_>>().join(" "),
            max_chars,
        )
    }

    fn is_template_summary_line(line: &str) -> bool {
        let normalized = line
            .trim()
            .trim_matches('#')
            .trim_matches('：')
            .trim_matches(':')
            .trim();
        matches!(normalized, "正式协作进度" | "协作结果同步")
    }

    fn normalize_card_summary(value: &str, max_chars: usize) -> String {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            return String::new();
        }
        let mut seen = std::collections::HashSet::new();
        let mut lines = Vec::new();
        for raw in trimmed.lines() {
            let line = raw.trim();
            if line.is_empty() || Self::is_template_summary_line(line) {
                continue;
            }
            let normalized_line = line.trim_start_matches('#').trim();
            if normalized_line.is_empty() || Self::is_template_summary_line(normalized_line) {
                continue;
            }
            if seen.insert(normalized_line.to_string()) {
                lines.push(normalized_line.to_string());
            }
        }
        let normalized = if lines.is_empty() {
            trimmed.to_string()
        } else {
            lines.join("\n")
        };
        Self::clamp_summary(&normalized, max_chars)
    }

    fn coding_payload_signature(payload: Option<&Value>) -> String {
        let Some(payload) = payload.and_then(Value::as_object) else {
            return "general".to_string();
        };
        let list = |key: &str| {
            payload
                .get(key)
                .and_then(Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(Value::as_str)
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                        .collect::<Vec<_>>()
                        .join("|")
                })
                .unwrap_or_default()
        };
        let next_action = payload
            .get("next_action")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("");
        format!(
            "files={};blockers={};next={}",
            list("changed_files"),
            list("blockers"),
            next_action
        )
    }

    fn latest_non_system_message<'a>(
        thread: &'a super::chat_channels::ChatChannelThreadResponse,
    ) -> &'a ChatChannelMessageResponse {
        thread
            .messages
            .iter()
            .rev()
            .find(|item| item.author_type != ChatChannelAuthorType::System)
            .unwrap_or(&thread.root_message)
    }

    fn reminder_revision(
        root: &ChatChannelMessageDoc,
        purpose: &str,
        latest_activity_message_id: &str,
    ) -> String {
        format!(
            "{}:{}:{}",
            purpose,
            root.collaboration_status
                .map(|status| format!("{status:?}"))
                .unwrap_or_else(|| "none".to_string()),
            latest_activity_message_id
        )
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

    async fn formal_progress_snapshot(
        &self,
        channel: &super::chat_channels::ChatChannelDoc,
        root: &ChatChannelMessageDoc,
        thread: &super::chat_channels::ChatChannelThreadResponse,
    ) -> FormalProgressSnapshot {
        let latest_dialogue = Self::latest_non_system_message(thread);
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
        let summary = Self::normalize_card_summary(summary_source, 160);
        let summary_revision = Self::normalize_revision_component(&summary, 180);
        let coding_payload = build_thread_coding_payload(
            self.agent_service.as_ref(),
            channel,
            &root.message_id,
            Some(thread),
        )
        .await;
        let coding_signature = Self::coding_payload_signature(coding_payload.as_ref());
        let status_token = root
            .collaboration_status
            .map(|status| format!("{status:?}"))
            .unwrap_or_else(|| "none".to_string());

        FormalProgressSnapshot {
            latest_actor,
            latest_activity_at,
            latest_activity_label: Self::relative_activity_label(Utc::now(), latest_activity_at),
            latest_activity_message_id: latest_dialogue.message_id.clone(),
            summary,
            summary_revision: summary_revision.clone(),
            progress_revision: format!(
                "progress:{}:{}:{}:{}",
                root.message_id, latest_dialogue.message_id, status_token, coding_signature
            ),
            result_revision: format!(
                "result:{}:{}:{}:{}",
                root.message_id, latest_dialogue.message_id, summary_revision, coding_signature
            ),
            coding_payload,
        }
    }

    async fn emit_formal_progress_card(
        &self,
        channel_service: &ChatChannelService,
        channel: &super::chat_channels::ChatChannelDoc,
        root: &ChatChannelMessageDoc,
        _thread: &super::chat_channels::ChatChannelThreadResponse,
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
            snapshot.summary, status_label, snapshot.latest_actor, snapshot.latest_activity_label,
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
                attach_card_domain_metadata(
                    channel,
                    serde_json::json!({
                        "discussion_card_kind": "progress",
                        "card_purpose": "formal_collaboration_progress_sync",
                        "linked_collaboration_id": root.message_id,
                        "summary_text": snapshot.summary,
                        "status_label": status_label,
                        "latest_actor": snapshot.latest_actor,
                        "source_message_id": snapshot.latest_activity_message_id,
                        "source_last_activity_id": snapshot.latest_activity_message_id,
                        "source_last_activity_at": snapshot.latest_activity_at.to_rfc3339(),
                        "source_revision": snapshot.progress_revision,
                        "summary_revision": snapshot.summary_revision,
                        "activity_age_minutes": (Utc::now() - snapshot.latest_activity_at).num_minutes(),
                    }),
                    snapshot.coding_payload.clone(),
                ),
            )
            .await?;
        let _ = channel_service
            .record_orchestrator_card_emission(
                channel,
                "scheduled_formal_progress_sync",
                ChatChannelCardEmissionFamily::Progress,
                &root.message_id,
                &snapshot.progress_revision,
                None,
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
        let snapshot = self.formal_progress_snapshot(channel, root, &thread).await;
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
            snapshot.summary, snapshot.latest_actor
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
                attach_card_domain_metadata(
                    channel,
                    serde_json::json!({
                        "discussion_card_kind": "result",
                        "card_purpose": "collaboration_result_sync",
                        "linked_collaboration_id": root.message_id,
                        "summary_text": snapshot.summary,
                        "status_label": "已采用",
                        "latest_actor": snapshot.latest_actor,
                        "source_message_id": snapshot.latest_activity_message_id,
                        "source_last_activity_id": snapshot.latest_activity_message_id,
                        "source_revision": snapshot.result_revision,
                        "summary_revision": snapshot.summary_revision,
                    }),
                    snapshot.coding_payload.clone(),
                ),
            )
            .await?;
        let _ = channel_service
            .record_orchestrator_card_emission(
                channel,
                "scheduled_result_sync",
                ChatChannelCardEmissionFamily::Result,
                &root.message_id,
                &snapshot.result_revision,
                None,
            )
            .await;
        Ok(())
    }

    async fn emit_summary_card(
        &self,
        channel_service: &ChatChannelService,
        channel: &super::chat_channels::ChatChannelDoc,
        discussion_messages: &[&ChatChannelMessageDoc],
        trigger_message_id: &str,
        revision: &str,
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
                    "source_message_id": trigger_message_id,
                    "source_revision": revision,
                    "next_actions": ["开始协作"],
                }),
            )
            .await?;
        Ok(())
    }

    async fn emit_reminder_card(
        &self,
        channel_service: &ChatChannelService,
        channel: &super::chat_channels::ChatChannelDoc,
        root: &ChatChannelMessageDoc,
        latest_activity_message_id: &str,
        revision: &str,
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
                    "source_last_activity_id": latest_activity_message_id,
                    "source_revision": revision,
                }),
            )
            .await?;
        let _ = channel_service
            .record_orchestrator_card_emission(
                channel,
                purpose,
                ChatChannelCardEmissionFamily::Reminder,
                &format!("{purpose}:{}", root.message_id),
                revision,
                None,
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
            let thread = channel_service
                .list_thread_messages(&channel.channel_id, &root.message_id)
                .await?;
            if matches!(root.surface, ChatChannelMessageSurface::Issue)
                && matches!(root.thread_state, ChatChannelThreadState::Active)
                && !matches!(
                    root.collaboration_status,
                    Some(ChatChannelDisplayStatus::Adopted | ChatChannelDisplayStatus::Rejected)
                )
            {
                if let Some(thread) = thread.as_ref() {
                    let snapshot = self.formal_progress_snapshot(channel, root, thread).await;
                    let silence = now - snapshot.latest_activity_at;
                    if silence >= Self::formal_progress_quiet_window()
                        && !Self::registry_matches(
                            &state,
                            ChatChannelCardEmissionFamily::Progress,
                            &root.message_id,
                            &snapshot.progress_revision,
                        )
                    {
                        let _ = self
                            .emit_formal_progress_card(
                                channel_service,
                                channel,
                                root,
                                &thread,
                                &snapshot,
                            )
                            .await;
                        continue;
                    }
                }
            }
            let latest_activity_message_id = thread
                .as_ref()
                .map(Self::latest_non_system_message)
                .map(|item| item.message_id.clone())
                .unwrap_or_else(|| root.message_id.clone());
            let latest_activity_at = thread
                .as_ref()
                .map(Self::latest_non_system_message)
                .and_then(|item| chrono::DateTime::parse_from_rfc3339(&item.created_at).ok())
                .map(|value| value.with_timezone(&Utc))
                .unwrap_or_else(|| root.updated_at.to_chrono());
            let age = now - latest_activity_at;
            if root.collaboration_status == Some(ChatChannelDisplayStatus::Adopted) {
                if let Some(thread) = thread.as_ref() {
                    let snapshot = self.formal_progress_snapshot(channel, root, thread).await;
                    if !Self::registry_matches(
                        &state,
                        ChatChannelCardEmissionFamily::Result,
                        &root.message_id,
                        &snapshot.result_revision,
                    ) {
                        let _ = self.emit_result_card(channel_service, channel, root).await;
                        continue;
                    }
                }
            }
            let awaiting_revision = Self::reminder_revision(
                root,
                "collaboration_awaiting_judgement_reminder",
                &latest_activity_message_id,
            );
            if root.collaboration_status == Some(ChatChannelDisplayStatus::AwaitingConfirmation)
                && age.num_minutes() >= 20
                && !Self::registry_matches(
                    &state,
                    ChatChannelCardEmissionFamily::Reminder,
                    &format!(
                        "{}:{}",
                        "collaboration_awaiting_judgement_reminder", root.message_id
                    ),
                    &awaiting_revision,
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
                        &latest_activity_message_id,
                        &awaiting_revision,
                        "collaboration_awaiting_judgement_reminder",
                        text,
                        ChatChannelDisplayStatus::AwaitingConfirmation,
                    )
                    .await;
                continue;
            }
            let stalled_revision = Self::reminder_revision(
                root,
                "collaboration_stalled_reminder",
                &latest_activity_message_id,
            );
            if root.collaboration_status == Some(ChatChannelDisplayStatus::Active)
                && age >= Self::stalled_threshold_for(state.agent_autonomy_mode)
                && !Self::registry_matches(
                    &state,
                    ChatChannelCardEmissionFamily::Reminder,
                    &format!("{}:{}", "collaboration_stalled_reminder", root.message_id),
                    &stalled_revision,
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
                        &latest_activity_message_id,
                        &stalled_revision,
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
                .maybe_emit_quiet_summary(channel_service, channel, &state, trigger_message_id)
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
        state: &ChatChannelOrchestratorStateDoc,
        trigger_message_id: &str,
    ) -> Result<(), mongodb::error::Error> {
        if state.last_seen_discussion_message_id.as_deref() != Some(trigger_message_id) {
            return Ok(());
        }
        let recent_roots = channel_service
            .list_recent_root_message_docs(&channel.channel_id, 80)
            .await?;
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
        let linked_ids = discussion_messages
            .iter()
            .map(|item| item.message_id.as_str())
            .collect::<Vec<_>>()
            .join("|");
        let revision = format!("discussion_summary:{}:{}", trigger_message_id, linked_ids);
        if Self::registry_matches(
            state,
            ChatChannelCardEmissionFamily::Suggestion,
            "discussion_summary",
            &revision,
        ) {
            return Ok(());
        }
        self.emit_summary_card(
            channel_service,
            channel,
            &discussion_messages,
            trigger_message_id,
            &revision,
        )
        .await?;
        let _ = channel_service
            .record_orchestrator_card_emission(
                channel,
                "discussion_summary",
                ChatChannelCardEmissionFamily::Suggestion,
                "discussion_summary",
                &revision,
                None,
            )
            .await;
        Ok(())
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
            let Some(state) = channel_service
                .get_orchestrator_state(&channel.channel_id)
                .await
                .ok()
                .flatten()
            else {
                return;
            };
            if let Err(error) = orchestrator
                .maybe_emit_quiet_summary(&channel_service, &channel, &state, &trigger_message_id)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::chat_channels::{ChatChannelCardEmissionRegistry, ChatChannelDisplayStatus};

    #[test]
    fn normalize_card_summary_drops_template_prefixes_and_duplicate_lines() {
        let raw =
            "正式协作进度\n正式协作进度\n### 当前已完成\n正式协作进度\n已开始落地。\n已开始落地。";
        let normalized = ChatChannelOrchestratorService::normalize_card_summary(raw, 160);
        assert!(!normalized.contains("###"));
        assert!(!normalized.lines().any(|line| line.trim() == "正式协作进度"));
        assert_eq!(normalized, "当前已完成\n已开始落地。");
    }

    #[test]
    fn registry_matches_same_revision_only_once() {
        let mut state = ChatChannelOrchestratorStateDoc {
            id: None,
            channel_id: "channel-1".to_string(),
            team_id: "team-1".to_string(),
            agent_autonomy_mode:
                crate::agent::chat_channels::ChatChannelAgentAutonomyMode::Standard,
            channel_goal: None,
            participant_notes: None,
            expected_outputs: None,
            collaboration_style: None,
            current_focus: None,
            active_collaboration_summaries: Vec::new(),
            recent_suggestion_fingerprints: Vec::new(),
            ignored_suggestion_fingerprints: Vec::new(),
            card_emission_registry: ChatChannelCardEmissionRegistry::default(),
            last_heartbeat_at: None,
            last_heartbeat_reason: None,
            last_summary_at: None,
            last_result_sync_at: None,
            last_seen_discussion_message_id: None,
            last_seen_document_change_at: None,
            last_seen_ai_output_change_at: None,
            created_at: mongodb::bson::DateTime::now(),
            updated_at: mongodb::bson::DateTime::now(),
        };
        state
            .card_emission_registry
            .progress_revisions
            .insert("root-1".to_string(), "rev-1".to_string());
        assert!(ChatChannelOrchestratorService::registry_matches(
            &state,
            ChatChannelCardEmissionFamily::Progress,
            "root-1",
            "rev-1",
        ));
        assert!(!ChatChannelOrchestratorService::registry_matches(
            &state,
            ChatChannelCardEmissionFamily::Progress,
            "root-1",
            "rev-2",
        ));
    }

    #[test]
    fn reminder_revision_changes_only_when_status_or_latest_activity_changes() {
        let root = ChatChannelMessageDoc {
            id: None,
            message_id: "root-1".to_string(),
            channel_id: "channel-1".to_string(),
            team_id: "team-1".to_string(),
            thread_root_id: None,
            parent_message_id: None,
            surface: ChatChannelMessageSurface::Issue,
            thread_state: ChatChannelThreadState::Active,
            collaboration_status: Some(ChatChannelDisplayStatus::AwaitingConfirmation),
            author_type: ChatChannelAuthorType::User,
            author_user_id: Some("user-1".to_string()),
            author_agent_id: None,
            author_name: "U".to_string(),
            agent_id: None,
            content_text: "需要判断".to_string(),
            content_blocks: serde_json::json!([]),
            metadata: serde_json::json!({}),
            visible: true,
            created_at: mongodb::bson::DateTime::now(),
            updated_at: mongodb::bson::DateTime::now(),
        };
        let base = ChatChannelOrchestratorService::reminder_revision(
            &root,
            "collaboration_awaiting_judgement_reminder",
            "msg-1",
        );
        let same = ChatChannelOrchestratorService::reminder_revision(
            &root,
            "collaboration_awaiting_judgement_reminder",
            "msg-1",
        );
        let changed = ChatChannelOrchestratorService::reminder_revision(
            &root,
            "collaboration_awaiting_judgement_reminder",
            "msg-2",
        );
        assert_eq!(base, same);
        assert_ne!(base, changed);
    }
}
