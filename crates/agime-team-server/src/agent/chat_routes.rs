//! Chat API routes (Phase 1 - Chat Track)
//!
//! These routes handle direct chat sessions that bypass the Task system.
//! Mounted at `/api/team/agent/chat`.

use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{
        sse::{Event, Sse},
        Json,
    },
    routing::{delete, get, patch, post, put},
    Extension, Router,
};
use futures::{stream::Stream, StreamExt};
use mongodb::bson::{doc, oid::ObjectId, Bson, Document};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;

use crate::auth::middleware::UserContext;
use crate::auth::service_mongo::ChatPersonaProfile;
use agime::agents::types::{RetryConfig, SuccessCheck};
use agime::session::{ExtensionState, SessionManager, TaskScope, TasksStateV2, TodoState};
use agime_team::models::mongo::PortalDetail;
use agime_team::models::{
    AgentSkillConfig, BuiltinExtension, CustomExtensionConfig, ListAgentsQuery, TeamAgent,
};
use agime_team::MongoDb;

use super::agent_prompt_composer::build_prompt_introspection_snapshot;
use super::capability_policy::AgentRuntimePolicyResolver;
use super::chat_channel_executor::{ChatChannelExecutor, ExecuteChannelMessageRequest};
use super::chat_channel_manager::ChatChannelManager;
use super::chat_channels::{
    AddChatChannelMemberRequest, ChatChannelAgentAutonomyMode, ChatChannelAuthorType,
    ChatChannelDetail, ChatChannelDisplayKind, ChatChannelDisplayStatus,
    ChatChannelInteractionMode, ChatChannelListFilters, ChatChannelMemberResponse,
    ChatChannelMemberRole, ChatChannelMessageSurface, ChatChannelReadResponse, ChatChannelService,
    ChatChannelSummary, ChatChannelThreadResponse, ChatChannelThreadState,
    ChatChannelUserPrefResponse, CreateChatChannelRequest, MarkChatChannelReadRequest,
    SendChatChannelMessageRequest, UpdateChatChannelMemberRequest, UpdateChatChannelPrefsRequest,
    UpdateChatChannelRequest,
};
use super::chat_executor::ChatExecutor;
use super::chat_manager::ChatManager;
use super::chat_memory::{
    render_memory_update_notice, render_user_relationship_overlay, sanitize_memory_patch,
    ChatMemoryService, UpdateUserChatMemoryRequest, UserChatMemoryResponse,
    UserChatMemorySuggestionResponse,
};
use super::normalize_workspace_path;
use super::prompt_profiles::{
    build_portal_coding_overlay, build_portal_manager_overlay, PortalCodingProfileInput,
};
use super::service_mongo::AgentService;
use super::session_mongo::{
    CreateChatSessionRequest, SendChatMessageRequest, SendMessageResponse, SessionListItem,
    UserSessionListQuery,
};
use agime_team::services::mongo::{PortalService, TeamService};

type ChatState = (
    Arc<AgentService>,
    Arc<MongoDb>,
    Arc<ChatManager>,
    Arc<ChatChannelManager>,
    String,
);

fn non_empty_text(value: Option<String>) -> Option<String> {
    value.and_then(|raw| {
        let trimmed = raw.trim().to_string();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed)
        }
    })
}

async fn load_runtime_tasks_snapshot(
    session_id: &str,
    runtime_session_id: Option<&str>,
) -> Option<TasksStateV2> {
    for candidate in std::iter::once(session_id).chain(runtime_session_id.into_iter()) {
        let runtime_session = match SessionManager::get_session(candidate, false).await {
            Ok(session) => session,
            Err(_) => continue,
        };
        if let Some(tasks) = TasksStateV2::from_extension_data(&runtime_session.extension_data) {
            return Some(tasks);
        }
        if let Some(legacy) = TodoState::from_extension_data(&runtime_session.extension_data) {
            return Some(TasksStateV2::migrate_from_legacy_todo(
                &legacy,
                TaskScope::Standalone,
                candidate.to_string(),
            ));
        }
    }

    None
}

async fn resolve_runtime_snapshot_for_session(
    service: &AgentService,
    db: &Arc<MongoDb>,
    session: &super::session_mongo::AgentSessionDoc,
) -> Option<super::capability_policy::RuntimeCapabilitySnapshot> {
    let agent = service.get_agent(&session.agent_id).await.ok().flatten()?;
    let portal_effective = if let Some(portal_id) = session
        .portal_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        let portal_svc = PortalService::new((**db).clone());
        match portal_svc.get(&session.team_id, portal_id).await {
            Ok(portal) => portal_svc
                .resolve_effective_public_config(&portal)
                .await
                .ok(),
            Err(_) => None,
        }
    } else {
        None
    };
    Some(AgentRuntimePolicyResolver::resolve(
        &agent,
        Some(session),
        portal_effective.as_ref(),
    ))
}

fn build_task_summary(state: &TasksStateV2) -> serde_json::Value {
    let pending = state
        .items
        .iter()
        .filter(|item| matches!(item.status, agime::session::TaskStatus::Pending))
        .count();
    let in_progress = state
        .items
        .iter()
        .filter(|item| matches!(item.status, agime::session::TaskStatus::InProgress))
        .count();
    let completed = state
        .items
        .iter()
        .filter(|item| matches!(item.status, agime::session::TaskStatus::Completed))
        .count();
    serde_json::json!({
        "board_id": state.board_id,
        "scope": state.scope,
        "task_count": state.items.len(),
        "pending_count": pending,
        "in_progress_count": in_progress,
        "completed_count": completed,
    })
}

fn build_assistant_identity_overlay() -> String {
    [
        "<assistant_identity>",
        "你是服务当前团队成员的 AI 助手。",
        "你的任务是理解问题、协助分析、推进任务，并在合适时完成复杂工作。",
        "保持自然、专业、可信，不要机械复述身份，也不要过度表演。",
        "</assistant_identity>",
    ]
    .join("\n")
}

fn build_assistant_persona_overlay(user: &UserContext) -> String {
    let profile = user.preferences.chat_persona_profile;
    let note = user.preferences.chat_persona_note.clone();

    let profile_text = match profile {
        ChatPersonaProfile::Default => {
            "自然、专业、有熟悉感。像一个可靠的内部协作伙伴，而不是客服或空泛助手。"
        }
        ChatPersonaProfile::Warm => "更温暖、更柔和，先理解用户感受，再进入事情本身。",
        ChatPersonaProfile::Supportive => "更支持型，适度鼓励和确认，帮助用户建立推进感和信心。",
        ChatPersonaProfile::Playful => "更有个性、更轻松、更有趣，但仍然亲切且克制，不做夸张表演。",
        ChatPersonaProfile::Direct => "更直接、更利落，减少绕弯，但保持礼貌、熟悉感和判断力。",
    };

    let mut lines = vec![
        "<assistant_persona>".to_string(),
        format!("当前人格：{}", profile_text),
        "共同原则：真有帮助，不装帮忙；可以有判断，不一味迎合；不客服化，不油腻，不重角色扮演。".to_string(),
        "允许少量 emoji 或语气词增强熟悉感，但不能喧宾夺主。".to_string(),
        "普通对话里，把用户默认视为当前团队内部熟悉成员，而不是陌生访客。".to_string(),
        "如果团队公共语境和当前用户的回复偏好不完全一致，优先贴合当前用户的称呼、关系和回复方式。".to_string(),
        "用户打招呼、闲聊或提关系型问题时，优先自然称呼对方，再进入内容，不要先进入免责声明口吻。".to_string(),
        "在普通对话里，只要语境自然，默认可以直接在开场、确认、鼓励、安慰和关系型回答中叫用户的称呼，让对话有熟悉感。".to_string(),
        "回答“我是谁”“你认识我吗”“我们在做什么”这类问题时，先用一两句自然的话直接回答，再按需要补充，不要一上来列清单或要求用户按格式补资料。".to_string(),
        "关系型问题和团队型问题，默认优先短答，不主动使用标题或清单；只有用户明确要结构化整理时再展开。".to_string(),
        "示例：'Tester，你是我们团队里的创始成员之一，我这边会先这样叫你。'".to_string(),
        "示例：'我们最近主要在打磨普通对话体验，让聊天更像团队内部沟通。'".to_string(),
    ];
    if let Some(value) = non_empty_text(note) {
        lines.push(format!("补充风格：{}", value));
    }
    lines.push("</assistant_persona>".to_string());
    lines.join("\n")
}

fn build_team_context_overlay(
    settings: &agime_team::models::mongo::TeamSettings,
) -> Option<String> {
    let chat = &settings.chat_assistant;
    let company_name = chat.company_name.clone();
    let department_name = chat.department_name.clone();
    let team_name = chat.team_name.clone();
    let team_summary = chat.team_summary.clone();
    let business_context = chat.business_context.clone();
    let tone_hint = chat.tone_hint.clone();

    if company_name.is_none()
        && department_name.is_none()
        && team_name.is_none()
        && team_summary.is_none()
        && business_context.is_none()
        && tone_hint.is_none()
    {
        return None;
    }

    let mut lines = vec!["<team_context>".to_string()];
    if let Some(value) = company_name {
        lines.push(format!("公司：{}", value));
    }
    if let Some(value) = department_name {
        lines.push(format!("部门：{}", value));
    }
    if let Some(value) = team_name {
        lines.push(format!("团队：{}", value));
    }
    if let Some(value) = team_summary {
        lines.push(format!("团队在做：{}", value));
    }
    if let Some(value) = business_context {
        lines.push(format!("常见语境：{}", value));
    }
    if let Some(value) = tone_hint {
        lines.push(format!("团队沟通氛围：{}", value));
    }
    lines
        .push("有这些信息时自然吸收并适度使用；没有涉及时保持通用表达，不要机械复述。".to_string());
    lines.push("与团队相关的话题，优先使用内部协作语气，例如“我们在做什么”“你这边在推进什么”，建立熟悉感，但不要生硬套近乎。".to_string());
    lines.push("这里提供的是团队公共语境，不替代你对当前用户的称呼和回复偏好。".to_string());
    lines.push("</team_context>".to_string());
    Some(lines.join("\n"))
}

fn build_personal_document_scope_overlay(attached_document_count: usize) -> String {
    let lines = vec![
        "<document_scope>".to_string(),
        "当前是普通个人对话。文档不是默认上下文。".to_string(),
        format!("本会话当前明确附加的文档数：{}", attached_document_count),
        "默认不要主动搜索、读取或引用团队文档库；只有当前明确附加的文档，才属于本轮默认文档上下文。".to_string(),
        "如果用户没有附加文档，也没有明确要求查资料/看文档/读团队资料，就不要主动调用 document_tools。".to_string(),
        "如果用户需要使用其他团队资料，应先让用户附加相应文档，再基于这些文档继续。".to_string(),
        "</document_scope>".to_string(),
    ];
    lines.join("\n")
}

fn build_channel_document_scope_overlay(attached_document_count: usize) -> String {
    [
        "<document_scope>".to_string(),
        "当前是团队共享频道。频道资料和团队文档库都不是每轮自动展开的默认上下文。".to_string(),
        format!("本轮当前明确附加的文档数：{}", attached_document_count),
        "默认只使用当前明确附加到这条消息或这次线程回复里的文档，不要主动搜索团队文档库。".to_string(),
        "即使频道已经绑定了资料目录，也不要因为目录存在就自动读取全部资料；需要用哪份资料，就先附加哪份资料。".to_string(),
        "如果用户想引用频道资料或团队文档，应先通过附件或频道资料入口明确加入当前消息上下文，再继续处理。".to_string(),
        "</document_scope>".to_string(),
    ]
    .join("\n")
}

fn channel_surface_requires_final_report(
    surface: ChatChannelMessageSurface,
    interaction_mode: ChatChannelInteractionMode,
) -> bool {
    matches!(
        (surface, interaction_mode),
        (
            ChatChannelMessageSurface::Issue,
            ChatChannelInteractionMode::Execution
        )
    )
}

fn channel_session_source_for_mode(interaction_mode: ChatChannelInteractionMode) -> &'static str {
    match interaction_mode {
        ChatChannelInteractionMode::Conversation => "channel_conversation",
        ChatChannelInteractionMode::Execution => "channel_runtime",
    }
}

fn build_channel_surface_execution_overlay(
    surface: ChatChannelMessageSurface,
    thread_state: ChatChannelThreadState,
    interaction_mode: ChatChannelInteractionMode,
) -> Option<String> {
    if matches!(interaction_mode, ChatChannelInteractionMode::Conversation) {
        return match surface {
            ChatChannelMessageSurface::Issue => Some(
                [
                    "<channel_conversation_contract>".to_string(),
                    "当前消息 surface=issue。这是一条正式协作线程。".to_string(),
                    "默认按持续对话推进：围绕问题澄清、补充判断、逐步修改文档、同步阶段进展，而不是把每一轮都当成立即收尾的任务。"
                        .to_string(),
                    "如果用户在探讨方案、来回调整、补上下文或持续协作，请自然继续对话，不要强行输出 blocked、final_output 或任务完成结论。"
                        .to_string(),
                    "</channel_conversation_contract>".to_string(),
                ]
                .join("\n"),
            ),
            ChatChannelMessageSurface::Temporary
                if matches!(thread_state, ChatChannelThreadState::Active) =>
            {
                Some(
                    [
                        "<channel_conversation_contract>".to_string(),
                        "当前消息 surface=temporary，表示临时协作线程。".to_string(),
                        "这里的目标是先聊明白、澄清问题、试探方向，默认按持续对话处理，不要把它理解成必须立刻完成的任务。"
                            .to_string(),
                        "如果上下文还不充分，就继续追问、收束和讨论；不要为了凑结果而硬性给出 blocked 或任务完成结论。"
                            .to_string(),
                        "</channel_conversation_contract>".to_string(),
                    ]
                    .join("\n"),
                )
            }
            _ => None,
        };
    }

    match surface {
        ChatChannelMessageSurface::Issue => Some(
            [
                "<channel_execution_contract>".to_string(),
                "当前消息 surface=issue。这是一条正式协作线程里的显式执行回合。".to_string(),
                "本轮应集中执行当前这一步，并给出清晰的执行结果、阻塞点或下一步。".to_string(),
                "不要复用频道 onboarding 话术，也不要输出未来时废话。".to_string(),
                "</channel_execution_contract>".to_string(),
            ]
            .join("\n"),
        ),
        ChatChannelMessageSurface::Temporary
            if matches!(thread_state, ChatChannelThreadState::Active) =>
        {
            Some(
                [
                    "<channel_execution_contract>".to_string(),
                    "当前消息 surface=temporary，表示临时协作线程里的显式执行回合。".to_string(),
                    "只执行用户当前明确要求的这一步；如果还缺上下文，就用简短中文说明阻塞点和下一步。"
                        .to_string(),
                    "不要退回成频道欢迎语或泛泛的协作开场白。".to_string(),
                    "</channel_execution_contract>".to_string(),
                ]
                .join("\n"),
            )
        }
        _ => None,
    }
}

fn merge_chat_extra_instructions(
    parts: Vec<String>,
    user_supplied: Option<String>,
) -> Option<String> {
    let mut chunks = parts
        .into_iter()
        .filter(|value| !value.trim().is_empty())
        .collect::<Vec<_>>();
    if let Some(extra) = non_empty_text(user_supplied) {
        chunks.push(extra);
    }
    if chunks.is_empty() {
        None
    } else {
        Some(chunks.join("\n\n"))
    }
}

fn should_auto_start_collaboration(content: &str) -> bool {
    let lowered = content.to_ascii_lowercase();
    lowered.contains("帮我")
        || lowered.contains("请帮")
        || lowered.contains("需要")
        || lowered.contains("整理")
        || lowered.contains("方案")
        || lowered.contains("计划")
        || lowered.contains("推进")
}

async fn maybe_emit_manager_agent_follow_up(
    service: &AgentService,
    channel_service: &ChatChannelService,
    channel: &super::chat_channels::ChatChannelDoc,
    user_message: &super::chat_channels::ChatChannelMessageDoc,
) -> Result<(), StatusCode> {
    let Some(state) = channel_service
        .get_orchestrator_state(&channel.channel_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
    else {
        return Ok(());
    };
    if matches!(
        state.agent_autonomy_mode,
        ChatChannelAgentAutonomyMode::Standard
    ) {
        return Ok(());
    }
    let fingerprint = user_message
        .content_text
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if state
        .recent_suggestion_fingerprints
        .iter()
        .any(|item| item == &fingerprint)
    {
        return Ok(());
    }
    if state
        .ignored_suggestion_fingerprints
        .iter()
        .any(|item| item == &fingerprint)
    {
        return Ok(());
    }
    let agent = service
        .get_agent(&channel.default_agent_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    let content = user_message.content_text.trim();
    if content.len() < 10 {
        return Ok(());
    }
    if matches!(
        state.agent_autonomy_mode,
        ChatChannelAgentAutonomyMode::AgentLead
    ) && should_auto_start_collaboration(content)
    {
        let collaboration_text = format!("围绕「{}」继续推进", clamp_summary(content, 120));
        let collaboration = channel_service
            .create_message(
                channel,
                None,
                None,
                ChatChannelMessageSurface::Temporary,
                ChatChannelThreadState::Active,
                Some(ChatChannelDisplayStatus::Active),
                ChatChannelAuthorType::Agent,
                None,
                Some(agent.id.clone()),
                agent.name.clone(),
                Some(agent.id.clone()),
                collaboration_text.clone(),
                serde_json::json!([{ "type": "text", "text": collaboration_text }]),
                serde_json::json!({
                    "discussion_card_kind": "suggestion",
                    "card_purpose": "auto_started_collaboration",
                    "source_message_id": user_message.message_id,
                }),
            )
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        let note = format!(
            "我先为这段讨论开了一个协作项，方便我继续整理方案、提炼下一步，并在需要时 @相关成员。"
        );
        let _ = channel_service
            .create_message(
                channel,
                None,
                Some(collaboration.message_id.clone()),
                ChatChannelMessageSurface::Activity,
                ChatChannelThreadState::Active,
                Some(ChatChannelDisplayStatus::Proposed),
                ChatChannelAuthorType::Agent,
                None,
                Some(agent.id.clone()),
                agent.name.clone(),
                Some(agent.id.clone()),
                note.clone(),
                serde_json::json!([{ "type": "text", "text": note }]),
                serde_json::json!({
                    "discussion_card_kind": "suggestion",
                    "card_purpose": "auto_started_notice",
                    "source_message_id": user_message.message_id,
                    "linked_collaboration_id": collaboration.message_id,
                }),
            )
            .await;
    } else {
        let suggestion = format!(
            "我注意到你们在讨论「{}」。如果你愿意，我可以帮你们把现状、方案和下一步整理成一个协作项。",
            clamp_summary(content, 120)
        );
        let _ = channel_service
            .create_message(
                channel,
                None,
                Some(user_message.message_id.clone()),
                ChatChannelMessageSurface::Activity,
                ChatChannelThreadState::Active,
                Some(ChatChannelDisplayStatus::Proposed),
                ChatChannelAuthorType::Agent,
                None,
                Some(agent.id.clone()),
                agent.name.clone(),
                Some(agent.id.clone()),
                suggestion.clone(),
                serde_json::json!([{ "type": "text", "text": suggestion }]),
                serde_json::json!({
                    "discussion_card_kind": "suggestion",
                    "card_purpose": "manager_suggestion",
                    "source_message_id": user_message.message_id,
                    "next_actions": ["开始协作", "先忽略"]
                }),
            )
            .await;
    }
    let _ = channel_service
        .record_orchestrator_heartbeat(channel, "discussion_follow_up", Some(fingerprint))
        .await;
    Ok(())
}

fn build_channel_context_overlay(
    channel: &super::chat_channels::ChatChannelDoc,
    default_agent_name: &str,
    thread_summary: Option<&str>,
) -> String {
    let mut lines = vec![
        "<channel_context>".to_string(),
        format!("频道名称：{}", channel.name),
        format!(
            "频道用途：{}",
            channel.description.as_deref().unwrap_or("团队协作频道")
        ),
        format!("默认 Agent：{}", default_agent_name),
        "这是团队共享频道，不是个人私聊空间。优先围绕频道目标、线程上下文和团队公共语境来协作。"
            .to_string(),
        "这里不要读取或套用任何个人私有记忆、个人称呼偏好或个人回复偏好。".to_string(),
    ];
    if let Some(folder_path) = channel
        .document_folder_path
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        lines.push(format!("当前频道资料目录：{}", folder_path));
        lines.push("当用户询问当前频道有哪些文档/资料时，优先围绕这个频道资料目录和当前频道 AI 产出来回答。".to_string());
    }
    if let Some(summary) = thread_summary.filter(|value| !value.trim().is_empty()) {
        lines.push(format!("当前线程围绕：{}", summary));
    }
    lines.push("</channel_context>".to_string());
    lines.join("\n")
}

async fn build_agent_name_map_for_team(
    service: &AgentService,
    team_id: &str,
) -> Result<HashMap<String, String>, StatusCode> {
    let agents = service
        .list_agents(ListAgentsQuery {
            team_id: team_id.to_string(),
            page: 1,
            limit: 200,
        })
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(agents
        .items
        .into_iter()
        .map(|agent| (agent.id, agent.name))
        .collect())
}

async fn ensure_channel_access(
    service: &AgentService,
    channel_service: &ChatChannelService,
    user: &UserContext,
    channel_id: &str,
) -> Result<
    (
        super::chat_channels::ChatChannelDoc,
        ChatChannelMemberRole,
        bool,
    ),
    StatusCode,
> {
    let channel = channel_service
        .get_channel(channel_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    let is_member = service
        .is_team_member(&user.user_id, &channel.team_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if !is_member {
        return Err(StatusCode::FORBIDDEN);
    }

    let is_admin = service
        .is_team_admin(&user.user_id, &channel.team_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if is_admin {
        return Ok((channel, ChatChannelMemberRole::Owner, true));
    }

    let member_role = channel_service
        .get_member_role(channel_id, &user.user_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    match channel.visibility {
        super::chat_channels::ChatChannelVisibility::TeamPublic => Ok((
            channel,
            member_role.unwrap_or(ChatChannelMemberRole::Member),
            false,
        )),
        super::chat_channels::ChatChannelVisibility::TeamPrivate => {
            if let Some(role) = member_role {
                Ok((channel, role, false))
            } else {
                Err(StatusCode::FORBIDDEN)
            }
        }
    }
}

async fn derive_suggestion_fingerprint(
    channel_service: &ChatChannelService,
    root: &super::chat_channels::ChatChannelMessageDoc,
) -> Result<Option<String>, mongodb::error::Error> {
    if let Some(source_message_id) = root
        .metadata
        .as_object()
        .and_then(|meta| meta.get("source_message_id"))
        .and_then(|value| value.as_str())
    {
        if let Some(source_message) = channel_service.get_message(source_message_id).await? {
            let fingerprint = collapse_whitespace(&source_message.content_text);
            if !fingerprint.is_empty() {
                return Ok(Some(fingerprint));
            }
        }
    }

    let fingerprint = collapse_whitespace(&root.content_text);
    if fingerprint.is_empty() {
        Ok(None)
    } else {
        Ok(Some(fingerprint))
    }
}

fn can_manage_channel(role: ChatChannelMemberRole, is_admin: bool) -> bool {
    is_admin
        || matches!(
            role,
            ChatChannelMemberRole::Owner | ChatChannelMemberRole::Manager
        )
}

async fn ensure_chat_agent_access(
    service: &AgentService,
    db: &Arc<MongoDb>,
    team_id: &str,
    agent_id: &str,
    user_id: &str,
) -> Result<(), StatusCode> {
    let user_group_ids =
        agime_team::services::mongo::user_group_service_mongo::UserGroupService::new(
            (**db).clone(),
        )
        .get_user_group_ids(team_id, user_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let has_agent_access = service
        .check_agent_access(agent_id, user_id, &user_group_ids)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if has_agent_access {
        Ok(())
    } else {
        Err(StatusCode::FORBIDDEN)
    }
}

async fn build_channel_extra_instructions(
    db: &Arc<MongoDb>,
    user: &UserContext,
    team_id: &str,
    channel: &super::chat_channels::ChatChannelDoc,
    default_agent_name: &str,
    thread_summary: Option<&str>,
    attached_document_count: usize,
    surface: ChatChannelMessageSurface,
    thread_state: ChatChannelThreadState,
    interaction_mode: ChatChannelInteractionMode,
) -> Option<String> {
    let team_settings = TeamService::new((**db).clone())
        .get_settings(team_id)
        .await
        .ok();
    merge_chat_extra_instructions(
        std::iter::once(build_assistant_identity_overlay())
            .chain(std::iter::once(build_assistant_persona_overlay(user)))
            .chain(
                team_settings
                    .as_ref()
                    .and_then(build_team_context_overlay)
                    .into_iter(),
            )
            .chain(std::iter::once(build_channel_context_overlay(
                channel,
                default_agent_name,
                thread_summary,
            )))
            .chain(std::iter::once(build_channel_document_scope_overlay(
                attached_document_count,
            )))
            .chain(
                build_channel_surface_execution_overlay(surface, thread_state, interaction_mode)
                    .into_iter(),
            )
            .collect(),
        None,
    )
}

async fn find_or_create_channel_runtime_session(
    service: &AgentService,
    channel: &super::chat_channels::ChatChannelDoc,
    user_id: &str,
    agent_id: &str,
    thread_root_id: &str,
    attached_document_ids: Vec<String>,
    extra_instructions: Option<String>,
    require_final_report: bool,
    session_source: &str,
) -> Result<super::session_mongo::AgentSessionDoc, StatusCode> {
    if let Some(existing) = service
        .find_active_channel_session(
            &channel.channel_id,
            thread_root_id,
            agent_id,
            session_source,
        )
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
    {
        service
            .update_channel_session_runtime_context(
                &existing.session_id,
                attached_document_ids,
                extra_instructions,
                require_final_report,
            )
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        service
            .set_session_channel_context(
                &existing.session_id,
                &channel.channel_id,
                &channel.name,
                Some(thread_root_id),
            )
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        return Ok(existing);
    }

    let internal_session = service
        .create_chat_session(
            &channel.team_id,
            agent_id,
            user_id,
            attached_document_ids,
            extra_instructions,
            None,
            None,
            None,
            None,
            None,
            None,
            require_final_report,
            false,
            Some("attached_only".to_string()),
            None,
            Some(session_source.to_string()),
            Some(true),
        )
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    service
        .set_session_channel_context(
            &internal_session.session_id,
            &channel.channel_id,
            &channel.name,
            Some(thread_root_id),
        )
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(internal_session)
}

#[derive(serde::Deserialize)]
struct StreamQuery {
    last_event_id: Option<u64>,
}

#[derive(serde::Deserialize)]
struct EventListQuery {
    #[serde(default)]
    run_id: Option<String>,
    #[serde(default)]
    after_event_id: Option<u64>,
    #[serde(default)]
    before_event_id: Option<u64>,
    #[serde(default)]
    order: Option<String>,
    #[serde(default)]
    limit: Option<u32>,
}

#[derive(serde::Deserialize)]
struct ChatMemoryQuery {
    team_id: String,
    #[serde(default)]
    session_id: Option<String>,
}

#[derive(serde::Deserialize)]
struct ChannelQuery {
    team_id: String,
}

#[derive(serde::Deserialize)]
struct ChannelEventListQuery {
    #[serde(default)]
    run_id: Option<String>,
    #[serde(default)]
    order: Option<String>,
    #[serde(default)]
    limit: Option<u32>,
}

#[derive(serde::Serialize)]
struct ChatMemoryEnvelope {
    memory: Option<UserChatMemoryResponse>,
}

#[derive(serde::Serialize)]
struct ChatMemorySuggestionsEnvelope {
    suggestions: Vec<UserChatMemorySuggestionResponse>,
}

#[derive(serde::Serialize)]
struct ChatChannelsEnvelope {
    channels: Vec<ChatChannelSummary>,
}

#[derive(serde::Serialize)]
struct ChatChannelEnvelope {
    channel: ChatChannelDetail,
}

#[derive(serde::Serialize)]
struct ChatChannelMembersEnvelope {
    members: Vec<ChatChannelMemberResponse>,
}

#[derive(serde::Serialize)]
struct ChatChannelPrefsEnvelope {
    prefs: ChatChannelUserPrefResponse,
}

#[derive(serde::Serialize)]
struct ChatChannelMessagesEnvelope {
    messages: Vec<super::chat_channels::ChatChannelMessageResponse>,
}

#[derive(Debug, Clone, serde::Deserialize)]
struct ChannelMessagesQuery {
    #[serde(default)]
    surface: Option<ChatChannelMessageSurface>,
    #[serde(default)]
    thread_state: Option<ChatChannelThreadState>,
    #[serde(default)]
    display_kind: Option<ChatChannelDisplayKind>,
    #[serde(default)]
    display_status: Option<ChatChannelDisplayStatus>,
}

#[derive(Debug, Clone, serde::Deserialize)]
struct DeleteChannelQuery {
    #[serde(default)]
    mode: Option<super::chat_channels::ChatChannelDeleteMode>,
}

#[derive(Debug, Clone, serde::Serialize)]
struct ComposerCapabilitySkill {
    id: String,
    name: String,
    version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    summary_text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    detail_text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    detail_lang: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    detail_source: Option<String>,
    skill_ref: String,
    display_line_zh: String,
    plain_line_zh: String,
}

#[derive(Debug, Clone, serde::Serialize)]
struct ComposerCapabilityExtension {
    runtime_name: String,
    display_name: String,
    class: String,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    ext_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    summary_text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    detail_text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    detail_lang: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    detail_source: Option<String>,
    ext_ref: String,
    display_line_zh: String,
    plain_line_zh: String,
}

#[derive(Debug, Clone, serde::Serialize)]
struct ComposerHiddenCapabilityExtension {
    runtime_name: String,
    display_name: String,
    reason: String,
}

#[derive(Debug, Clone, serde::Serialize)]
struct ComposerCapabilitiesCatalog {
    skills: Vec<ComposerCapabilitySkill>,
    extensions: Vec<ComposerCapabilityExtension>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    hidden_extensions: Vec<ComposerHiddenCapabilityExtension>,
}

#[derive(Debug, Clone)]
struct ComposerExtensionEntry {
    runtime_name: String,
    display_name: String,
    class: String,
    ext_type: Option<String>,
    description: Option<String>,
    ext_ref: String,
    display_line_zh: String,
    plain_line_zh: String,
    source_extension_id: Option<String>,
    builtin_lookup_id: Option<String>,
}

#[derive(Debug, Clone, Default)]
struct ComposerDescriptionText {
    summary_text: Option<String>,
    detail_text: Option<String>,
    detail_lang: Option<String>,
    detail_source: Option<String>,
}

fn normalize_name(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

fn normalize_skill_id(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

fn build_skill_ref(skill_id: &str, name: &str, skill_class: &str, meta: &str) -> String {
    format!("[[skill:{}|{}|{}|{}]]", skill_id, name, skill_class, meta)
}

fn build_extension_ref(
    extension_id: &str,
    name: &str,
    extension_class: &str,
    meta: &str,
) -> String {
    format!(
        "[[ext:{}|{}|{}|{}]]",
        extension_id, name, extension_class, meta
    )
}

fn skill_display_line_zh(skill_ref: &str, version: &str) -> String {
    format!("{}（团队技能，v{}）", skill_ref, version)
}

fn skill_plain_line_zh(name: &str, version: &str) -> String {
    format!("{}（团队技能，v{}）", name, version)
}

fn builtin_extension_display_name(extension: BuiltinExtension) -> &'static str {
    match extension {
        BuiltinExtension::Skills => "Skills",
        BuiltinExtension::SkillRegistry => "Skill Registry",
        BuiltinExtension::Tasks => "Tasks",
        BuiltinExtension::ExtensionManager => "Extension Manager",
        BuiltinExtension::Team => "Team",
        BuiltinExtension::ChatRecall => "Chat Recall",
        BuiltinExtension::DocumentTools => "Document Tools",
        BuiltinExtension::Developer => "Developer",
        BuiltinExtension::Memory => "Memory",
        BuiltinExtension::ComputerController => "Computer Controller",
        BuiltinExtension::AutoVisualiser => "Auto Visualiser",
        BuiltinExtension::Tutorial => "Tutorial",
    }
}

fn extension_display_line_zh(extension_ref: &str, extension_class: &str) -> String {
    let label = match extension_class {
        "builtin" => "内置扩展",
        "team" => "团队扩展",
        _ => "自定义扩展",
    };
    format!("{}（{}）", extension_ref, label)
}

fn extension_plain_line_zh(display_name: &str, extension_class: &str) -> String {
    let label = match extension_class {
        "builtin" => "内置扩展",
        "team" => "团队扩展",
        _ => "自定义扩展",
    };
    format!("{}（{}）", display_name, label)
}

fn document_scope_label_zh(scope_mode: Option<&str>) -> &'static str {
    match scope_mode {
        Some("attached_only") => "仅当前附加文档",
        Some("channel_bound") => "频道资料 + 当前附加文档",
        Some("portal_bound") => "当前绑定资料",
        Some("full") => "团队文档库",
        _ => "按当前会话限制",
    }
}

fn document_write_label_zh(write_mode: Option<&str>) -> &'static str {
    match write_mode {
        Some("read_only") => "只读",
        Some("draft_only") => "生成草稿",
        Some("controlled_write") => "受控写入",
        Some("full_write") => "完整写入",
        _ => "按当前会话限制",
    }
}

fn document_tools_runtime_description_zh(
    scope_mode: Option<&str>,
    write_mode: Option<&str>,
) -> String {
    format!(
        "文档范围：{}；文档操作：{}。",
        document_scope_label_zh(scope_mode),
        document_write_label_zh(write_mode)
    )
}

fn is_runtime_visible_builtin(extension: BuiltinExtension) -> bool {
    matches!(
        extension,
        BuiltinExtension::SkillRegistry
            | BuiltinExtension::Tasks
            | BuiltinExtension::DocumentTools
            | BuiltinExtension::Developer
            | BuiltinExtension::Memory
            | BuiltinExtension::ComputerController
            | BuiltinExtension::AutoVisualiser
            | BuiltinExtension::Tutorial
    )
}

fn builtin_runtime_names(extension: BuiltinExtension) -> Vec<String> {
    let mut names = vec![normalize_name(extension.name())];
    if let Some(mcp_name) = extension.mcp_name() {
        let normalized = normalize_name(mcp_name);
        if !names.contains(&normalized) {
            names.push(normalized);
        }
    }
    names
}

fn extension_class_from_config(extension: &CustomExtensionConfig) -> &'static str {
    if extension.source.as_deref() == Some("team") || extension.source_extension_id.is_some() {
        "team"
    } else {
        "custom"
    }
}

fn build_skill_catalog_item(skill: &AgentSkillConfig) -> ComposerCapabilitySkill {
    let version = if skill.version.trim().is_empty() {
        "1.0.0".to_string()
    } else {
        skill.version.trim().to_string()
    };
    let skill_ref = build_skill_ref(
        &format!("team:{}", skill.skill_id),
        &skill.name,
        "team",
        &version,
    );
    ComposerCapabilitySkill {
        id: skill.skill_id.clone(),
        name: skill.name.clone(),
        version: version.clone(),
        description: skill.description.clone(),
        summary_text: None,
        detail_text: None,
        detail_lang: None,
        detail_source: None,
        skill_ref: skill_ref.clone(),
        display_line_zh: skill_display_line_zh(&skill_ref, &version),
        plain_line_zh: skill_plain_line_zh(&skill.name, &version),
    }
}

fn build_builtin_extension_entry(extension: BuiltinExtension) -> ComposerExtensionEntry {
    let runtime_name = extension
        .mcp_name()
        .map(normalize_name)
        .unwrap_or_else(|| normalize_name(extension.name()));
    let display_name = builtin_extension_display_name(extension).to_string();
    let ext_ref = build_extension_ref(
        &format!("builtin:{}", runtime_name),
        &display_name,
        "builtin",
        &runtime_name,
    );
    ComposerExtensionEntry {
        runtime_name,
        display_name: display_name.clone(),
        class: "builtin".to_string(),
        ext_type: if extension.mcp_name().is_some() {
            Some("stdio".to_string())
        } else {
            None
        },
        description: Some(extension.description().to_string()),
        ext_ref: ext_ref.clone(),
        display_line_zh: extension_display_line_zh(&ext_ref, "builtin"),
        plain_line_zh: extension_plain_line_zh(&display_name, "builtin"),
        source_extension_id: None,
        builtin_lookup_id: Some(extension.name().to_string()),
    }
}

fn build_custom_extension_entry(extension: &CustomExtensionConfig) -> ComposerExtensionEntry {
    let runtime_name = normalize_name(&extension.name);
    let class = extension_class_from_config(extension).to_string();
    let ext_ref = build_extension_ref(
        &match class.as_str() {
            "team" => format!(
                "team:{}",
                extension
                    .source_extension_id
                    .clone()
                    .unwrap_or_else(|| runtime_name.clone())
            ),
            _ => format!("custom:{}", runtime_name),
        },
        &extension.name,
        &class,
        extension.ext_type.as_str(),
    );
    ComposerExtensionEntry {
        runtime_name,
        display_name: extension.name.clone(),
        class: class.clone(),
        ext_type: Some(extension.ext_type.clone()),
        description: None,
        ext_ref: ext_ref.clone(),
        display_line_zh: extension_display_line_zh(&ext_ref, &class),
        plain_line_zh: extension_plain_line_zh(&extension.name, &class),
        source_extension_id: extension.source_extension_id.clone(),
        builtin_lookup_id: None,
    }
}

fn build_attached_team_extension_entry(
    extension: &agime_team::models::AttachedTeamExtensionRef,
) -> Option<ComposerExtensionEntry> {
    let runtime_name = extension
        .runtime_name
        .as_deref()
        .map(normalize_name)
        .filter(|value| !value.is_empty())?;
    let display_name = extension
        .display_name
        .clone()
        .unwrap_or_else(|| runtime_name.clone());
    let transport = extension
        .transport
        .clone()
        .unwrap_or_else(|| "mcp".to_string());
    let ext_ref = build_extension_ref(
        &format!("team:{}", extension.extension_id),
        &display_name,
        "team",
        &transport,
    );
    Some(ComposerExtensionEntry {
        runtime_name,
        display_name: display_name.clone(),
        class: "team".to_string(),
        ext_type: Some(transport),
        description: None,
        ext_ref: ext_ref.clone(),
        display_line_zh: extension_display_line_zh(&ext_ref, "team"),
        plain_line_zh: extension_plain_line_zh(&display_name, "team"),
        source_extension_id: Some(extension.extension_id.clone()),
        builtin_lookup_id: None,
    })
}

fn build_hidden_builtin_extension(
    extension: BuiltinExtension,
) -> Option<ComposerHiddenCapabilityExtension> {
    let runtime_name = extension
        .mcp_name()
        .map(normalize_name)
        .unwrap_or_else(|| normalize_name(extension.name()));
    let display_name = builtin_extension_display_name(extension).to_string();
    let reason = match extension {
        BuiltinExtension::Skills => "skill_runtime",
        BuiltinExtension::ExtensionManager | BuiltinExtension::ChatRecall => "system_assist",
        BuiltinExtension::Team => "legacy_hidden",
        _ => return None,
    };
    Some(ComposerHiddenCapabilityExtension {
        runtime_name,
        display_name,
        reason: reason.to_string(),
    })
}

fn find_extension_entry_from_agent(
    agent: &TeamAgent,
    runtime_name: &str,
) -> Option<ComposerExtensionEntry> {
    let normalized = normalize_name(runtime_name);
    if let Some(custom) = agent
        .custom_extensions
        .iter()
        .find(|ext| normalize_name(&ext.name) == normalized)
    {
        let mut cfg = custom.clone();
        cfg.enabled = true;
        return Some(build_custom_extension_entry(&cfg));
    }

    for ext in &agent.enabled_extensions {
        let runtime_names = builtin_runtime_names(ext.extension);
        if runtime_names.iter().any(|name| name == &normalized)
            && is_runtime_visible_builtin(ext.extension)
        {
            return Some(build_builtin_extension_entry(ext.extension));
        }
    }

    None
}

fn resolve_composer_extensions(
    agent: &TeamAgent,
    session: Option<&super::session_mongo::AgentSessionDoc>,
) -> Vec<ComposerExtensionEntry> {
    let runtime_snapshot = AgentRuntimePolicyResolver::resolve(agent, session, None);
    let mut entries = BTreeMap::<String, ComposerExtensionEntry>::new();

    for ext in runtime_snapshot
        .extensions
        .builtin_capabilities
        .iter()
        .filter(|ext| ext.enabled && ext.registry.editable)
    {
        if is_runtime_visible_builtin(ext.extension) {
            let mut entry = build_builtin_extension_entry(ext.extension);
            if entry.runtime_name == "document_tools" {
                entry.description = Some(document_tools_runtime_description_zh(
                    runtime_snapshot.document_scope_mode.as_deref(),
                    runtime_snapshot.document_write_mode.as_deref(),
                ));
            }
            entries.insert(entry.runtime_name.clone(), entry);
        }
    }

    for ext in &runtime_snapshot.extensions.custom_extensions {
        let entry = build_custom_extension_entry(ext);
        entries.insert(entry.runtime_name.clone(), entry);
    }

    for ext in &runtime_snapshot.extensions.attached_team_extensions {
        if let Some(entry) = build_attached_team_extension_entry(ext) {
            entries.insert(entry.runtime_name.clone(), entry);
        }
    }

    if !runtime_snapshot
        .extensions
        .effective_allowed_extension_names
        .is_empty()
    {
        let allowed: HashSet<String> = runtime_snapshot
            .extensions
            .effective_allowed_extension_names
            .iter()
            .map(|value| normalize_name(value))
            .collect();
        entries.retain(|runtime_name, _| allowed.contains(runtime_name));
    }

    entries.into_values().collect()
}

fn resolve_hidden_composer_extensions(agent: &TeamAgent) -> Vec<ComposerHiddenCapabilityExtension> {
    let runtime_snapshot = AgentRuntimePolicyResolver::resolve(agent, None, None);
    let mut hidden = BTreeMap::<String, ComposerHiddenCapabilityExtension>::new();
    for ext in runtime_snapshot
        .extensions
        .builtin_capabilities
        .iter()
        .filter(|ext| ext.enabled)
    {
        if is_runtime_visible_builtin(ext.extension) {
            continue;
        }
        if let Some(entry) = build_hidden_builtin_extension(ext.extension) {
            hidden.insert(entry.runtime_name.clone(), entry);
        }
    }
    hidden.into_values().collect()
}

fn resolve_composer_skills(
    agent: &TeamAgent,
    session: Option<&super::session_mongo::AgentSessionDoc>,
) -> Vec<ComposerCapabilitySkill> {
    let runtime_snapshot = AgentRuntimePolicyResolver::resolve(agent, session, None);
    let allowed_skill_ids: Option<HashSet<String>> = runtime_snapshot
        .skills
        .effective_allowed_skill_ids
        .as_ref()
        .map(|ids| {
            ids.iter()
                .map(|value| normalize_skill_id(value))
                .collect::<HashSet<_>>()
        });

    runtime_snapshot
        .skills
        .assigned_skills
        .iter()
        .filter(|skill| skill.enabled)
        .filter(|skill| {
            allowed_skill_ids
                .as_ref()
                .map(|ids| ids.contains(&normalize_skill_id(&skill.skill_id)))
                .unwrap_or(true)
        })
        .map(build_skill_catalog_item)
        .collect()
}

fn collapse_whitespace(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn trim_optional_text(value: Option<String>) -> Option<String> {
    value.and_then(|text| {
        let normalized = collapse_whitespace(&text);
        if normalized.is_empty() {
            None
        } else {
            Some(normalized)
        }
    })
}

fn first_summary_chunk(value: &str) -> String {
    let normalized = collapse_whitespace(value);
    if normalized.is_empty() {
        return String::new();
    }

    for separator in ["\n\n", "。", ".", "；", ";", "！", "!", "？", "?"] {
        if let Some((head, _)) = normalized.split_once(separator) {
            let trimmed = head.trim();
            if trimmed.len() >= 18 {
                return trimmed.to_string();
            }
        }
    }

    normalized
}

fn clamp_summary(value: &str, max_chars: usize) -> String {
    let chunk = first_summary_chunk(value);
    if chunk.chars().count() <= max_chars {
        return chunk;
    }

    let mut end = 0usize;
    for (idx, _) in chunk.char_indices() {
        if chunk[..idx].chars().count() >= max_chars {
            break;
        }
        end = idx;
    }

    let candidate = if end == 0 {
        chunk.chars().take(max_chars).collect::<String>()
    } else {
        chunk[..end].trim_end().to_string()
    };
    format!(
        "{}…",
        candidate.trim_end_matches(['。', '.', ';', '；', '!', '！', '?', '？'])
    )
}

fn build_description_text(
    ai_description: Option<String>,
    ai_lang: Option<String>,
    raw_description: Option<String>,
    ai_source: &str,
) -> ComposerDescriptionText {
    let ai_description = trim_optional_text(ai_description);
    let raw_description = trim_optional_text(raw_description);

    if let Some(detail_text) = ai_description {
        return ComposerDescriptionText {
            summary_text: Some(clamp_summary(&detail_text, 96)),
            detail_text: Some(detail_text),
            detail_lang: trim_optional_text(ai_lang),
            detail_source: Some(ai_source.to_string()),
        };
    }

    if let Some(detail_text) = raw_description {
        return ComposerDescriptionText {
            summary_text: Some(clamp_summary(&detail_text, 96)),
            detail_text: Some(detail_text),
            detail_lang: None,
            detail_source: Some("raw_description".to_string()),
        };
    }

    ComposerDescriptionText::default()
}

async fn enrich_composer_skills(
    db: &MongoDb,
    team_id: &str,
    skills: &mut [ComposerCapabilitySkill],
) {
    let Ok(team_oid) = ObjectId::parse_str(team_id) else {
        return;
    };

    let skill_ids: Vec<ObjectId> = skills
        .iter()
        .filter_map(|skill| ObjectId::parse_str(&skill.id).ok())
        .collect();
    if skill_ids.is_empty() {
        return;
    }

    let coll = db.collection::<Document>("skills");
    let filter = doc! {
        "team_id": team_oid,
        "_id": { "$in": Bson::Array(skill_ids.into_iter().map(Bson::ObjectId).collect()) },
        "is_deleted": false,
    };
    let Ok(mut cursor) = coll.find(filter, None).await else {
        return;
    };

    let mut docs_by_id = BTreeMap::<String, Document>::new();
    while let Some(Ok(doc)) = cursor.next().await {
        if let Ok(id) = doc.get_object_id("_id") {
            docs_by_id.insert(id.to_hex(), doc);
        }
    }

    for skill in skills.iter_mut() {
        let source_doc = docs_by_id.get(&skill.id);
        let raw_description = source_doc
            .and_then(|doc| doc.get_str("description").ok().map(str::to_string))
            .or_else(|| skill.description.clone());
        let ai_description =
            source_doc.and_then(|doc| doc.get_str("ai_description").ok().map(str::to_string));
        let ai_lang =
            source_doc.and_then(|doc| doc.get_str("ai_description_lang").ok().map(str::to_string));
        let detail = build_description_text(
            ai_description,
            ai_lang,
            raw_description.clone(),
            "ai_description",
        );
        skill.description = trim_optional_text(raw_description);
        skill.summary_text = detail.summary_text;
        skill.detail_text = detail.detail_text;
        skill.detail_lang = detail.detail_lang;
        skill.detail_source = detail.detail_source;
    }
}

fn composer_skill_dedupe_key(skill: &ComposerCapabilitySkill) -> String {
    format!(
        "{}::{}",
        normalize_name(&skill.name),
        skill.version.trim().to_ascii_lowercase()
    )
}

fn composer_detail_source_rank(source: Option<&str>) -> u8 {
    match source.unwrap_or_default() {
        "ai_description" => 3,
        "builtin_cache" => 2,
        "raw_description" => 1,
        _ => 0,
    }
}

fn composer_detail_lang_rank(lang: Option<&str>) -> u8 {
    let normalized = lang.unwrap_or_default().trim().to_ascii_lowercase();
    if normalized.starts_with("zh") {
        2
    } else if normalized.is_empty() {
        0
    } else {
        1
    }
}

fn normalize_lang_tag(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

fn preferred_lang_from_headers(headers: &HeaderMap) -> Option<String> {
    let raw = headers.get("accept-language")?.to_str().ok()?.trim();
    if raw.is_empty() {
        return None;
    }

    raw.split(',')
        .map(|part| part.split(';').next().unwrap_or_default().trim())
        .find(|part| !part.is_empty())
        .map(normalize_lang_tag)
}

fn composer_doc_lang_preference_rank(doc_lang: Option<&str>, preferred_lang: Option<&str>) -> u8 {
    let normalized = normalize_lang_tag(doc_lang.unwrap_or_default());
    let preferred = preferred_lang
        .map(normalize_lang_tag)
        .unwrap_or_else(|| "zh".to_string());

    if normalized.is_empty() {
        return 0;
    }
    if normalized == preferred || normalized.starts_with(&format!("{}-", preferred)) {
        return 4;
    }
    if preferred.starts_with("zh") && normalized.starts_with("zh") {
        return 3;
    }
    if normalized.starts_with("zh") {
        return 2;
    }
    if normalized.starts_with("en") {
        return 1;
    }
    1
}

fn composer_skill_rank(skill: &ComposerCapabilitySkill) -> (u8, u8, usize, usize) {
    (
        composer_detail_source_rank(skill.detail_source.as_deref()),
        composer_detail_lang_rank(skill.detail_lang.as_deref()),
        skill.detail_text.as_deref().map(str::len).unwrap_or(0),
        skill.description.as_deref().map(str::len).unwrap_or(0),
    )
}

fn dedupe_composer_skills(skills: Vec<ComposerCapabilitySkill>) -> Vec<ComposerCapabilitySkill> {
    let mut order = Vec::<String>::new();
    let mut deduped = HashMap::<String, ComposerCapabilitySkill>::new();

    for skill in skills {
        let key = composer_skill_dedupe_key(&skill);
        match deduped.get(&key) {
            Some(existing) if composer_skill_rank(&skill) <= composer_skill_rank(existing) => {}
            Some(_) => {
                deduped.insert(key, skill);
            }
            None => {
                order.push(key.clone());
                deduped.insert(key, skill);
            }
        }
    }

    order
        .into_iter()
        .filter_map(|key| deduped.remove(&key))
        .collect()
}

async fn enrich_composer_extensions(
    db: &MongoDb,
    team_id: &str,
    entries: Vec<ComposerExtensionEntry>,
    preferred_lang: Option<&str>,
) -> Vec<ComposerCapabilityExtension> {
    let Ok(team_oid) = ObjectId::parse_str(team_id) else {
        return entries
            .into_iter()
            .map(|entry| ComposerCapabilityExtension {
                runtime_name: entry.runtime_name,
                display_name: entry.display_name,
                class: entry.class,
                ext_type: entry.ext_type,
                description: entry.description,
                summary_text: None,
                detail_text: None,
                detail_lang: None,
                detail_source: None,
                ext_ref: entry.ext_ref,
                display_line_zh: entry.display_line_zh,
                plain_line_zh: entry.plain_line_zh,
            })
            .collect();
    };

    let source_extension_ids: Vec<ObjectId> = entries
        .iter()
        .filter_map(|entry| entry.source_extension_id.as_deref())
        .filter_map(|value| ObjectId::parse_str(value).ok())
        .collect();
    let builtin_ids: Vec<String> = entries
        .iter()
        .filter_map(|entry| entry.builtin_lookup_id.clone())
        .collect();

    let mut extension_docs = BTreeMap::<String, Document>::new();
    if !source_extension_ids.is_empty() {
        let coll = db.collection::<Document>("extensions");
        let filter = doc! {
            "team_id": team_oid,
            "_id": { "$in": Bson::Array(source_extension_ids.into_iter().map(Bson::ObjectId).collect()) },
            "is_deleted": false,
        };
        if let Ok(mut cursor) = coll.find(filter, None).await {
            while let Some(Ok(doc)) = cursor.next().await {
                if let Ok(id) = doc.get_object_id("_id") {
                    extension_docs.insert(id.to_hex(), doc);
                }
            }
        }
    }

    let mut builtin_docs = HashMap::<String, Document>::new();
    if !builtin_ids.is_empty() {
        let coll = db.collection::<Document>("builtin_extension_descriptions");
        let filter = doc! {
            "team_id": team_oid,
            "extension_id": { "$in": Bson::Array(builtin_ids.into_iter().map(Bson::String).collect()) },
        };
        if let Ok(mut cursor) = coll.find(filter, None).await {
            while let Some(Ok(doc)) = cursor.next().await {
                if let Ok(id) = doc.get_str("extension_id") {
                    let doc_lang = doc.get_str("ai_description_lang").ok();
                    let should_replace = match builtin_docs.get(id) {
                        Some(existing) => {
                            let existing_lang = existing.get_str("ai_description_lang").ok();
                            composer_doc_lang_preference_rank(doc_lang, preferred_lang)
                                > composer_doc_lang_preference_rank(existing_lang, preferred_lang)
                        }
                        None => true,
                    };
                    if should_replace {
                        builtin_docs.insert(id.to_string(), doc);
                    }
                }
            }
        }
    }

    entries
        .into_iter()
        .map(|entry| {
            let source_doc = entry
                .source_extension_id
                .as_ref()
                .and_then(|id| extension_docs.get(id));
            let builtin_doc = entry
                .builtin_lookup_id
                .as_ref()
                .and_then(|id| builtin_docs.get(id));

            let raw_description = source_doc
                .and_then(|doc| doc.get_str("description").ok().map(str::to_string))
                .or_else(|| entry.description.clone());

            let (ai_description, ai_lang, ai_source) = if let Some(doc) = builtin_doc {
                (
                    doc.get_str("ai_description").ok().map(str::to_string),
                    doc.get_str("ai_description_lang").ok().map(str::to_string),
                    "builtin_cache",
                )
            } else if let Some(doc) = source_doc {
                (
                    doc.get_str("ai_description").ok().map(str::to_string),
                    doc.get_str("ai_description_lang").ok().map(str::to_string),
                    "ai_description",
                )
            } else {
                (None, None, "raw_description")
            };

            let detail =
                build_description_text(ai_description, ai_lang, raw_description.clone(), ai_source);
            ComposerCapabilityExtension {
                runtime_name: entry.runtime_name,
                display_name: entry.display_name,
                class: entry.class,
                ext_type: entry.ext_type,
                description: trim_optional_text(raw_description),
                summary_text: detail.summary_text,
                detail_text: detail.detail_text,
                detail_lang: detail.detail_lang,
                detail_source: detail.detail_source,
                ext_ref: entry.ext_ref,
                display_line_zh: entry.display_line_zh,
                plain_line_zh: entry.plain_line_zh,
            }
        })
        .collect()
}

async fn build_composer_capability_catalog(
    db: &MongoDb,
    team_id: &str,
    agent: &TeamAgent,
    session: Option<&super::session_mongo::AgentSessionDoc>,
    preferred_lang: Option<&str>,
) -> ComposerCapabilitiesCatalog {
    let mut skills = resolve_composer_skills(agent, session);
    enrich_composer_skills(db, team_id, &mut skills).await;
    let skills = dedupe_composer_skills(skills);

    ComposerCapabilitiesCatalog {
        skills,
        extensions: enrich_composer_extensions(
            db,
            team_id,
            resolve_composer_extensions(agent, session),
            preferred_lang,
        )
        .await,
        hidden_extensions: resolve_hidden_composer_extensions(agent),
    }
}

fn default_portal_retry_config() -> RetryConfig {
    let check_command = if cfg!(windows) {
        "if exist index.html (exit /b 0) else (echo index.html not found & exit /b 1)".to_string()
    } else {
        "[ -f index.html ]".to_string()
    };
    RetryConfig {
        max_retries: 6,
        checks: vec![SuccessCheck::Shell {
            command: check_command,
        }],
        on_failure: None,
        timeout_seconds: Some(180),
        on_failure_timeout_seconds: Some(300),
    }
}

fn manager_message_mentions_skill_inventory(user_content: &str) -> bool {
    let lowered = user_content.to_ascii_lowercase();
    lowered.contains("available skill")
        || lowered.contains("available skills")
        || lowered.contains("installed skill")
        || lowered.contains("installed skills")
        || lowered.contains("current skill")
        || lowered.contains("current skills")
        || lowered.contains("my skills")
        || lowered.contains("usable skill")
        || lowered.contains("usable skills")
        || lowered.contains("what skills")
        || user_content.contains("有哪些技能")
        || user_content.contains("什么技能")
        || user_content.contains("可用技能")
        || user_content.contains("当前技能")
        || user_content.contains("已安装技能")
        || user_content.contains("已安装并启用")
        || user_content.contains("安装并启用")
        || user_content.contains("启用的skills")
        || user_content.contains("启用的 skills")
        || user_content.contains("我已安装并启用的 skills")
        || user_content.contains("我已安装并启用的skills")
        || user_content.contains("目前能用")
        || user_content.contains("找一下你目前能用的skills")
        || user_content.contains("找一下你目前能用的 skills")
}

fn manager_message_mentions_registry(user_content: &str) -> bool {
    let lowered = user_content.to_ascii_lowercase();
    lowered.contains("skills.sh")
        || lowered.contains("skill_registry")
        || lowered.contains("registry")
        || lowered.contains("trending")
        || lowered.contains("all_time")
        || lowered.contains("all time")
        || lowered.contains("hot")
        || user_content.contains("热门")
        || user_content.contains("技能")
}

fn manager_message_mentions_extension_inventory(user_content: &str) -> bool {
    let lowered = user_content.to_ascii_lowercase();
    lowered.contains("available extension")
        || lowered.contains("available extensions")
        || lowered.contains("current extension")
        || lowered.contains("current extensions")
        || lowered.contains("enabled extension")
        || lowered.contains("enabled extensions")
        || lowered.contains("builtin extension")
        || lowered.contains("builtin extensions")
        || lowered.contains("mcp")
        || user_content.contains("可用扩展")
        || user_content.contains("当前扩展")
        || user_content.contains("已启用扩展")
        || user_content.contains("启用的扩展")
        || user_content.contains("有哪些扩展")
        || user_content.contains("什么扩展")
        || user_content.contains("哪些扩展")
        || user_content.contains("MCP")
        || user_content.contains("mcp")
}

fn message_mentions_mcp_install(user_content: &str) -> bool {
    let lowered = user_content.to_ascii_lowercase();
    lowered.contains("install mcp")
        || lowered.contains("install extension")
        || lowered.contains("uninstall mcp")
        || lowered.contains("remove mcp")
        || lowered.contains("delete mcp")
        || lowered.contains("delete extension")
        || lowered.contains("remove extension")
        || lowered.contains("uninstall extension")
        || lowered.contains("detach mcp")
        || lowered.contains("detach extension")
        || lowered.contains("custom extension")
        || lowered.contains("mcp server")
        || lowered.contains("stdio mcp")
        || lowered.contains("sse mcp")
        || lowered.contains("streamable http")
        || lowered.contains("streamable_http")
        || lowered.contains("playwright-mcp")
        || user_content.contains("安装mcp")
        || user_content.contains("安装 MCP")
        || user_content.contains("安装扩展")
        || user_content.contains("安装一个新的 MCP")
        || user_content.contains("卸载mcp")
        || user_content.contains("卸载 MCP")
        || user_content.contains("卸载扩展")
        || user_content.contains("删除mcp")
        || user_content.contains("删除 MCP")
        || user_content.contains("删除扩展")
        || user_content.contains("移除mcp")
        || user_content.contains("移除 MCP")
        || user_content.contains("移除扩展")
        || user_content.contains("删掉mcp")
        || user_content.contains("删掉 MCP")
        || user_content.contains("删掉扩展")
        || user_content.contains("自定义扩展")
        || user_content.contains("stdio")
        || user_content.contains("SSE")
        || user_content.contains("streamable")
}

async fn build_portal_manager_turn_notice(
    db: &MongoDb,
    team_id: &str,
    portal_id: Option<&str>,
    user_content: &str,
) -> Option<String> {
    let mentions_registry = manager_message_mentions_registry(user_content);
    let mentions_extension_inventory = manager_message_mentions_extension_inventory(user_content);

    let mentions_mcp_install = message_mentions_mcp_install(user_content);

    if !mentions_registry && !mentions_extension_inventory && !mentions_mcp_install {
        return None;
    }

    let mut skill_registry_allowed = false;
    if let Some(portal_id) = portal_id.filter(|value| !value.trim().is_empty()) {
        let portal_svc = PortalService::new(db.clone());
        if let Ok(portal) = portal_svc.get(team_id, portal_id).await {
            if let Ok(effective) = portal_svc.resolve_effective_public_config(&portal).await {
                skill_registry_allowed = effective
                    .effective_allowed_extensions
                    .iter()
                    .any(|ext| ext == "skill_registry");
            }
        }
    }

    let mut parts = Vec::new();
    if mentions_registry {
        if skill_registry_allowed {
            parts.push(
                "特别提醒：当前用户正在询问 skills.sh / registry / 热门技能相关问题，并且当前数字分身上下文允许 `skill_registry`。本轮应优先调用 `skill_registry__list_popular_skills`、`skill_registry__search_skills` 或 `skill_registry__preview_skill` 等工具完成查询；只有在工具真实返回上游错误时，才能说明外部接口失败，不能再说“没有该能力”或要求再次申请。"
                    .to_string(),
            );
        } else {
            parts.push(
                "特别提醒：当前用户正在询问 skills.sh / registry / 热门技能相关问题。如果本轮实际不可用 `skill_registry` 工具，应如实说明当前会话未开放 registry 能力或仍需治理审批；禁止再说“401 Missing API key”或把问题归因到缺少 API key。"
                    .to_string(),
            );
        }
    }

    if mentions_extension_inventory {
        parts.push(
            "特别提醒：当前用户正在询问“当前可用的扩展/MCP/已启用扩展”。在 portal_manager 管理会话中，这类问题必须先调用 `portal_tools__get_portal_service_capability_profile`，并优先使用 `profile.serviceAgent.enabledBuiltinExtensionDetails`、`enabledCustomExtensionDetails`、`catalog.teamExtensions` 中的 `display_line_zh` 逐条列出当前真实可用的扩展/MCP。禁止自行概括成 `xxx ✓`、只列原始内部名，或跳过 profile 直接凭记忆回答。若 profile 没有返回可列举项，再明确说明当前没有可显示的扩展详情。".to_string(),
        );
    }

    if mentions_mcp_install {
        parts.push(
                "特别提醒：当前用户正在要求正式管理 MCP/自定义扩展。禁止把 `git clone`、`npm install`、把代码放进当前 workspace、或在 shell 里临时把 server 跑起来描述成“系统已经安装”。在 portal_manager 管理会话中，正式安装必须优先走 `team_mcp__install_team_mcp` 写入团队扩展库；若用户还要求立即给某个 Agent/分身可用，再调用 `team_mcp__attach_team_mcp` 挂载到目标 Agent。更新必须走 `team_mcp__update_team_mcp`，卸载必须走 `team_mcp__remove_team_mcp`。如果用户给的是网页、README、仓库或教程链接，应先使用当前会话已有的网页阅读能力（如 developer / playwright）提取真实安装命令、服务地址和必要 envs，再调用 `team_mcp__plan_install_team_mcp` 归一化并校验安装计划；只有在 `ready_to_install=true` 时才能执行正式安装。若用户要求卸载/删除/移除某个 MCP，必须先调用 `team_mcp__list_installed` 确认精确目标，再用 `team_mcp__remove_team_mcp` 正式卸载，并默认保持 `detach_attached=true`，确保所有已挂载 Agent 一并摘除。若涉及当前数字分身，还应在安装/挂载后调用 `portal_tools__get_portal_service_capability_profile` 回读，确认扩展已出现在 `catalog.teamExtensions`、`enabledCustomExtensionDetails` 或运行边界里。".to_string(),
        );
    }

    Some(parts.join(" "))
}

async fn build_general_turn_notice(
    service: &AgentService,
    team_id: &str,
    agent_id: &str,
    user_content: &str,
) -> Option<String> {
    let mentions_extension_inventory = manager_message_mentions_extension_inventory(user_content);
    let mentions_mcp_install = message_mentions_mcp_install(user_content);

    if !mentions_mcp_install && !mentions_extension_inventory {
        return None;
    }

    let mut notices = Vec::new();

    if mentions_extension_inventory {
        notices.push(
            "特别提醒：当前用户正在询问“当前 Agent 现在能用哪些扩展 / MCP / 工具 / 能力”。这类问题必须先调用 `team_mcp__inspect_runtime_capabilities`，再基于工具结果回答，并把 `enabled_builtin_capabilities`、`attached_team_mcps`、`attached_custom_extensions`、`team_library_mcp` 明确分开说明。若工具结果返回 `render_ready_sections_zh` / `render_ready_markdown_zh`，在列能力清单时必须优先逐条原样使用其中的 `display_line_zh`，不要自己重写成普通名称列表。只有在正文解释、原因分析或泛化说明里，才改用 `plain_line_zh`。禁止只调用 `team_mcp__list_installed` 后就说“当前只有这些 MCP”，因为 `list_installed` 只代表团队扩展库，不代表当前 Agent 的全部运行时能力。禁止根据模型自身记忆、其它产品里的工具名（如 `read_file`、`read_dir`）或泛化经验臆测“当前会话可用能力”；如果 `team_mcp__inspect_runtime_capabilities` 真正调用失败，必须明确说明是该工具调用失败，而不是主观推断工具不可用。".to_string(),
        );
    }

    let agent = match service.get_agent(agent_id).await {
        Ok(Some(agent)) if agent.team_id == team_id => agent,
        _ => {
            if mentions_mcp_install {
                notices.push(
                    "特别提醒：当前用户正在要求管理 MCP/自定义扩展。禁止把 clone 仓库、npm install、把代码放进 workspace 或临时跑通 server 描述成“系统已经安装”或“已经卸载”。如果当前会话没有正式的扩展管理工具能力，应明确说明需要改用 MCP 工作区 `/teams/{teamId}/mcp/workspace` 或数字分身管理会话完成正式安装/卸载。"
                        .to_string(),
                );
            }
            return Some(notices.join(" "));
        }
    };

    if mentions_mcp_install && has_manager_tooling(&agent) {
        notices.push(
            "特别提醒：当前用户正在要求正式管理 MCP/自定义扩展。禁止把 clone 仓库、npm install、把代码放进 workspace、或临时跑通 server 当成“系统已经安装”或“已经卸载”。正式安装必须优先走 `team_mcp__install_team_mcp` 写入团队扩展库；若需要立即给某个 Agent/分身可用，再调用 `team_mcp__attach_team_mcp`。更新走 `team_mcp__update_team_mcp`，卸载走 `team_mcp__remove_team_mcp`。如果用户给的是网页、README、仓库或教程链接，应先使用当前会话已有的网页阅读能力（如 developer / playwright）提取真实安装命令、服务地址和必要 envs，再调用 `team_mcp__plan_install_team_mcp` 归一化并校验安装计划；只有在 `ready_to_install=true` 时才能执行正式安装。若用户要求卸载/删除/移除某个 MCP，必须先调用 `team_mcp__list_installed` 确认精确目标，再用 `team_mcp__remove_team_mcp` 正式卸载，并默认保持 `detach_attached=true`，确保所有已挂载 Agent 一并摘除。如果当前会话明确绑定了数字分身/服务 Agent，则在挂载后再调用 `portal_tools__get_portal_service_capability_profile` 回读确认；若用户只是想通过 UI 完成安装，则引导使用 MCP 工作区 `/teams/{teamId}/mcp/workspace`。".to_string(),
        );
        return Some(notices.join(" "));
    }

    if mentions_mcp_install {
        notices.push(
            "特别提醒：当前用户正在要求管理 MCP/自定义扩展，但本会话不具备正式的扩展管理工具能力。禁止把 workspace 里的临时安装/删除描述成系统已安装或已卸载；应明确说明需要切换到具备管理能力的会话，或直接使用 MCP 工作区 `/teams/{teamId}/mcp/workspace` 完成正式安装/卸载。".to_string(),
        );
    }
    Some(notices.join(" "))
}

/// Create chat router
pub fn chat_router(
    db: Arc<MongoDb>,
    chat_manager: Arc<ChatManager>,
    workspace_root: String,
) -> Router {
    let service = Arc::new(AgentService::new(db.clone()));
    let channel_manager = Arc::new(ChatChannelManager::new_with_event_persistence(db.clone()));
    {
        let db = db.clone();
        tokio::spawn(async move {
            let _ = ChatChannelService::new(db).ensure_indexes().await;
        });
    }

    Router::new()
        .route(
            "/agents/{agent_id}/composer-capabilities",
            get(get_agent_composer_capabilities),
        )
        .route(
            "/memory/me",
            get(get_my_chat_memory).patch(update_my_chat_memory),
        )
        .route("/memory/suggestions", get(list_my_chat_memory_suggestions))
        .route(
            "/memory/suggestions/{id}/accept",
            post(accept_chat_memory_suggestion),
        )
        .route(
            "/memory/suggestions/{id}/dismiss",
            post(dismiss_chat_memory_suggestion),
        )
        .route("/channels", get(list_channels).post(create_channel))
        .route(
            "/channels/{channel_id}",
            get(get_channel_detail)
                .patch(update_channel)
                .delete(delete_channel),
        )
        .route("/channels/{channel_id}/prefs", patch(update_channel_prefs))
        .route("/channels/{channel_id}/archive", post(archive_channel))
        .route(
            "/channels/{channel_id}/members",
            get(list_channel_members).post(add_channel_member),
        )
        .route(
            "/channels/{channel_id}/members/{user_id}",
            patch(update_channel_member).delete(remove_channel_member),
        )
        .route(
            "/channels/{channel_id}/messages",
            get(list_channel_messages).post(send_channel_message),
        )
        .route(
            "/channels/{channel_id}/messages/{message_id}/promote-to-issue",
            post(promote_channel_message_to_issue),
        )
        .route(
            "/channels/{channel_id}/messages/{message_id}/archive-thread",
            post(archive_channel_thread),
        )
        .route(
            "/channels/{channel_id}/messages/{message_id}/status",
            post(update_channel_collaboration_status),
        )
        .route(
            "/channels/{channel_id}/messages/{message_id}/sync-result",
            post(sync_channel_collaboration_result),
        )
        .route(
            "/channels/{channel_id}/threads/{thread_root_id}",
            get(get_channel_thread),
        )
        .route(
            "/channels/{channel_id}/threads/{thread_root_id}/messages",
            post(send_channel_thread_message),
        )
        .route("/channels/{channel_id}/stream", get(stream_channel))
        .route("/channels/{channel_id}/events", get(list_channel_events))
        .route("/channels/{channel_id}/read", post(mark_channel_read))
        .route("/sessions", get(list_sessions))
        .route("/sessions", post(create_session))
        .route(
            "/sessions/portal-coding",
            post(create_portal_coding_session),
        )
        .route(
            "/sessions/portal-manager",
            post(create_portal_manager_session),
        )
        .route("/sessions/{id}", get(get_session))
        .route(
            "/sessions/{id}/composer-capabilities",
            get(get_session_composer_capabilities),
        )
        .route("/sessions/{id}", put(update_session))
        .route("/sessions/{id}", delete(delete_session))
        .route("/sessions/{id}/messages", post(send_message))
        .route("/sessions/{id}/stream", get(stream_chat))
        .route("/sessions/{id}/events", get(list_session_events))
        .route("/sessions/{id}/cancel", post(cancel_chat))
        .route("/sessions/{id}/archive", post(archive_session))
        // Phase 2: Document attachment
        .route(
            "/sessions/{id}/documents",
            get(list_attached_documents)
                .post(attach_documents)
                .delete(detach_documents),
        )
        .with_state((service, db, chat_manager, channel_manager, workspace_root))
}

async fn ensure_team_member_for_chat_memory(
    service: &AgentService,
    user: &UserContext,
    team_id: &str,
) -> Result<(), StatusCode> {
    let is_member = service
        .is_team_member(&user.user_id, team_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if !is_member {
        return Err(StatusCode::FORBIDDEN);
    }
    Ok(())
}

async fn get_my_chat_memory(
    State((service, db, _, _, _)): State<ChatState>,
    Extension(user): Extension<UserContext>,
    Query(query): Query<ChatMemoryQuery>,
) -> Result<Json<ChatMemoryEnvelope>, StatusCode> {
    ensure_team_member_for_chat_memory(service.as_ref(), &user, &query.team_id).await?;
    let memory = ChatMemoryService::new(db)
        .get_memory(&query.team_id, &user.user_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .map(Into::into);
    Ok(Json(ChatMemoryEnvelope { memory }))
}

async fn update_my_chat_memory(
    State((service, db, _, _, _)): State<ChatState>,
    Extension(user): Extension<UserContext>,
    Query(query): Query<ChatMemoryQuery>,
    Json(request): Json<UpdateUserChatMemoryRequest>,
) -> Result<Json<ChatMemoryEnvelope>, StatusCode> {
    ensure_team_member_for_chat_memory(service.as_ref(), &user, &query.team_id).await?;
    let (patch, session_id) = sanitize_memory_patch(request);
    let memory_service = ChatMemoryService::new(db);
    let memory = memory_service
        .upsert_memory(&query.team_id, &user.user_id, patch, &user.user_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    if let Some(target_session_id) = session_id.or(query.session_id) {
        if let Ok(Some(session)) = service.get_session(&target_session_id).await {
            if session.user_id == user.user_id
                && session.session_source.eq_ignore_ascii_case("chat")
                && !session.portal_restricted
            {
                let _ = service
                    .append_hidden_session_notice(
                        &target_session_id,
                        &render_memory_update_notice(&memory),
                    )
                    .await;
            }
        }
    }

    Ok(Json(ChatMemoryEnvelope {
        memory: Some(memory.into()),
    }))
}

async fn list_my_chat_memory_suggestions(
    State((service, db, _, _, _)): State<ChatState>,
    Extension(user): Extension<UserContext>,
    Query(query): Query<ChatMemoryQuery>,
) -> Result<Json<ChatMemorySuggestionsEnvelope>, StatusCode> {
    ensure_team_member_for_chat_memory(service.as_ref(), &user, &query.team_id).await?;
    let suggestions = ChatMemoryService::new(db)
        .list_pending_suggestions(&query.team_id, &user.user_id, query.session_id.as_deref())
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .into_iter()
        .map(Into::into)
        .collect();
    Ok(Json(ChatMemorySuggestionsEnvelope { suggestions }))
}

async fn accept_chat_memory_suggestion(
    State((service, db, _, _, _)): State<ChatState>,
    Extension(user): Extension<UserContext>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let memory_service = ChatMemoryService::new(db);
    let Some(suggestion) = memory_service
        .get_suggestion(&id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
    else {
        return Err(StatusCode::NOT_FOUND);
    };
    if suggestion.user_id != user.user_id {
        return Err(StatusCode::FORBIDDEN);
    }
    ensure_team_member_for_chat_memory(service.as_ref(), &user, &suggestion.team_id).await?;
    let Some((suggestion, memory)) = memory_service
        .accept_suggestion(&id, &user.user_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
    else {
        return Err(StatusCode::NOT_FOUND);
    };
    if let Ok(Some(session)) = service.get_session(&suggestion.session_id).await {
        if session.user_id == user.user_id
            && session.session_source.eq_ignore_ascii_case("chat")
            && !session.portal_restricted
        {
            let _ = service
                .append_hidden_session_notice(
                    &suggestion.session_id,
                    &render_memory_update_notice(&memory),
                )
                .await;
        }
    }
    Ok(Json(serde_json::json!({
        "suggestion": UserChatMemorySuggestionResponse::from(suggestion),
        "memory": UserChatMemoryResponse::from(memory),
    })))
}

async fn dismiss_chat_memory_suggestion(
    State((service, db, _, _, _)): State<ChatState>,
    Extension(user): Extension<UserContext>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let memory_service = ChatMemoryService::new(db);
    let Some(existing) = memory_service
        .get_suggestion(&id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
    else {
        return Err(StatusCode::NOT_FOUND);
    };
    if existing.user_id != user.user_id {
        return Err(StatusCode::FORBIDDEN);
    }
    ensure_team_member_for_chat_memory(service.as_ref(), &user, &existing.team_id).await?;
    let Some(suggestion) = memory_service
        .dismiss_suggestion(&id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
    else {
        return Err(StatusCode::NOT_FOUND);
    };
    Ok(Json(serde_json::json!({
        "suggestion": UserChatMemorySuggestionResponse::from(suggestion),
    })))
}

async fn list_channels(
    State((service, db, _, _, _)): State<ChatState>,
    Extension(user): Extension<UserContext>,
    Query(query): Query<ChannelQuery>,
) -> Result<Json<ChatChannelsEnvelope>, StatusCode> {
    ensure_team_member_for_chat_memory(service.as_ref(), &user, &query.team_id).await?;
    let is_admin = service
        .is_team_admin(&user.user_id, &query.team_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let agent_names = build_agent_name_map_for_team(service.as_ref(), &query.team_id).await?;
    let channels = ChatChannelService::new(db)
        .list_channels_for_user(&query.team_id, &user.user_id, is_admin, &agent_names)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(ChatChannelsEnvelope { channels }))
}

async fn create_channel(
    State((service, db, _, _, _)): State<ChatState>,
    Extension(user): Extension<UserContext>,
    Query(query): Query<ChannelQuery>,
    Json(mut request): Json<CreateChatChannelRequest>,
) -> Result<Json<ChatChannelEnvelope>, StatusCode> {
    ensure_team_member_for_chat_memory(service.as_ref(), &user, &query.team_id).await?;
    let agent = service
        .get_agent(&request.default_agent_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    if agent.team_id != query.team_id {
        return Err(StatusCode::FORBIDDEN);
    }
    ensure_chat_agent_access(
        service.as_ref(),
        &db,
        &query.team_id,
        &request.default_agent_id,
        &user.user_id,
    )
    .await?;

    request.member_user_ids.retain(|item| item != &user.user_id);
    for member_user_id in &request.member_user_ids {
        let is_member = service
            .is_team_member(member_user_id, &query.team_id)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        if !is_member {
            return Err(StatusCode::BAD_REQUEST);
        }
    }

    let channel_service = ChatChannelService::new(db);
    let channel = channel_service
        .create_channel(&query.team_id, &user.user_id, request)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let onboarding_text = "频道启动卡\n请先告诉我四件事：1. 这个频道主要目标是什么；2. 主要谁会参与协作、谁是关键判断人；3. 最终希望产出什么；4. 这里更偏讨论、方案、执行还是评审。";
    let _ = channel_service
        .create_message(
            &channel,
            None,
            None,
            ChatChannelMessageSurface::Activity,
            ChatChannelThreadState::Active,
            Some(ChatChannelDisplayStatus::Proposed),
            ChatChannelAuthorType::Agent,
            None,
            Some(agent.id.clone()),
            agent.name.clone(),
            Some(agent.id.clone()),
            onboarding_text.to_string(),
            serde_json::json!([
                { "type": "text", "text": onboarding_text }
            ]),
            serde_json::json!({
                "discussion_card_kind": "suggestion",
                "card_purpose": "channel_onboarding",
                "prompts": [
                    "频道目标",
                    "参与人",
                    "产出物",
                    "协作方式"
                ]
            }),
        )
        .await;
    let pref = channel_service
        .touch_last_visited(&channel, &user.user_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let agent_names = build_agent_name_map_for_team(service.as_ref(), &query.team_id).await?;
    let unread_count = channel_service
        .unread_count(&channel.channel_id, &user.user_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let member_count = channel_service
        .list_members(&channel.channel_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .len() as i32;
    let detail = channel_service
        .build_detail(
            channel,
            ChatChannelMemberRole::Owner,
            &agent_names,
            unread_count,
            member_count,
            Some(&pref),
        )
        .await;
    Ok(Json(ChatChannelEnvelope { channel: detail }))
}

async fn get_channel_detail(
    State((service, db, _, _, _)): State<ChatState>,
    Extension(user): Extension<UserContext>,
    Path(channel_id): Path<String>,
) -> Result<Json<ChatChannelEnvelope>, StatusCode> {
    let channel_service = ChatChannelService::new(db.clone());
    let (channel, role, is_admin) =
        ensure_channel_access(service.as_ref(), &channel_service, &user, &channel_id).await?;
    let pref = channel_service
        .touch_last_visited(&channel, &user.user_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let agent_names = build_agent_name_map_for_team(service.as_ref(), &channel.team_id).await?;
    let unread_count = channel_service
        .unread_count(&channel.channel_id, &user.user_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let member_count = if is_admin {
        channel_service
            .list_members(&channel.channel_id)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
            .len() as i32
    } else {
        channel_service
            .list_members(&channel.channel_id)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
            .len() as i32
    };
    let detail = channel_service
        .build_detail(
            channel,
            role,
            &agent_names,
            unread_count,
            member_count,
            Some(&pref),
        )
        .await;
    Ok(Json(ChatChannelEnvelope { channel: detail }))
}

async fn update_channel(
    State((service, db, _, _, _)): State<ChatState>,
    Extension(user): Extension<UserContext>,
    Path(channel_id): Path<String>,
    Json(request): Json<UpdateChatChannelRequest>,
) -> Result<Json<ChatChannelEnvelope>, StatusCode> {
    let channel_service = ChatChannelService::new(db.clone());
    let (channel, role, is_admin) =
        ensure_channel_access(service.as_ref(), &channel_service, &user, &channel_id).await?;
    if !can_manage_channel(role, is_admin) {
        return Err(StatusCode::FORBIDDEN);
    }
    let requested_mode = request.agent_autonomy_mode;
    if let Some(agent_id) = request.default_agent_id.as_deref() {
        let agent = service
            .get_agent(agent_id)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
            .ok_or(StatusCode::NOT_FOUND)?;
        if agent.team_id != channel.team_id {
            return Err(StatusCode::FORBIDDEN);
        }
    }
    let orchestrator_update_requested = requested_mode.is_some()
        || request.channel_goal.is_some()
        || request.participant_notes.is_some()
        || request.expected_outputs.is_some()
        || request.collaboration_style.is_some();
    let request_for_orchestrator = request.clone();
    let updated = channel_service
        .update_channel(&channel_id, request)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    if orchestrator_update_requested {
        channel_service
            .update_orchestrator_profile(&updated, &request_for_orchestrator)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    }
    let agent_names = build_agent_name_map_for_team(service.as_ref(), &updated.team_id).await?;
    let unread_count = channel_service
        .unread_count(&updated.channel_id, &user.user_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let member_count = channel_service
        .list_members(&updated.channel_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .len() as i32;
    let pref = channel_service
        .get_pref_for_user(&updated.channel_id, &user.user_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let detail = channel_service
        .build_detail(
            updated,
            role,
            &agent_names,
            unread_count,
            member_count,
            pref.as_ref(),
        )
        .await;
    Ok(Json(ChatChannelEnvelope { channel: detail }))
}

async fn update_channel_prefs(
    State((service, db, _, _, _)): State<ChatState>,
    Extension(user): Extension<UserContext>,
    Path(channel_id): Path<String>,
    Json(request): Json<UpdateChatChannelPrefsRequest>,
) -> Result<Json<ChatChannelPrefsEnvelope>, StatusCode> {
    let channel_service = ChatChannelService::new(db);
    let (channel, _, _) =
        ensure_channel_access(service.as_ref(), &channel_service, &user, &channel_id).await?;
    let pref = channel_service
        .upsert_user_pref(&channel, &user.user_id, request)
        .await
        .map_err(|error| {
            tracing::error!(
                "Failed to update channel prefs for channel {} and user {}: {:?}",
                channel_id,
                user.user_id,
                error
            );
            StatusCode::INTERNAL_SERVER_ERROR
        })?;
    Ok(Json(ChatChannelPrefsEnvelope {
        prefs: ChatChannelUserPrefResponse {
            channel_id: pref.channel_id,
            team_id: pref.team_id,
            user_id: pref.user_id,
            pinned: pref.pinned,
            muted: pref.muted,
            last_visited_at: pref
                .last_visited_at
                .map(|value| value.to_chrono().to_rfc3339()),
        },
    }))
}

async fn archive_channel(
    State((service, db, _, _, _)): State<ChatState>,
    Extension(user): Extension<UserContext>,
    Path(channel_id): Path<String>,
) -> Result<StatusCode, StatusCode> {
    let channel_service = ChatChannelService::new(db);
    let (_, role, is_admin) =
        ensure_channel_access(service.as_ref(), &channel_service, &user, &channel_id).await?;
    if !can_manage_channel(role, is_admin) {
        return Err(StatusCode::FORBIDDEN);
    }
    let archived = channel_service
        .archive_channel(&channel_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if archived {
        Ok(StatusCode::OK)
    } else {
        Err(StatusCode::NOT_FOUND)
    }
}

async fn delete_channel(
    State((service, db, _, _, _)): State<ChatState>,
    Extension(user): Extension<UserContext>,
    Path(channel_id): Path<String>,
    Query(query): Query<DeleteChannelQuery>,
) -> Result<StatusCode, StatusCode> {
    let channel_service = ChatChannelService::new(db);
    let (_, role, is_admin) =
        ensure_channel_access(service.as_ref(), &channel_service, &user, &channel_id).await?;
    if !(is_admin || role == ChatChannelMemberRole::Owner) {
        return Err(StatusCode::FORBIDDEN);
    }
    let deleted = channel_service
        .delete_channel(
            &channel_id,
            &user.user_id,
            query
                .mode
                .unwrap_or(super::chat_channels::ChatChannelDeleteMode::PreserveDocuments),
        )
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if deleted {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(StatusCode::NOT_FOUND)
    }
}

async fn list_channel_members(
    State((service, db, _, _, _)): State<ChatState>,
    Extension(user): Extension<UserContext>,
    Path(channel_id): Path<String>,
) -> Result<Json<ChatChannelMembersEnvelope>, StatusCode> {
    let channel_service = ChatChannelService::new(db);
    let (channel, _, _) =
        ensure_channel_access(service.as_ref(), &channel_service, &user, &channel_id).await?;
    let members = channel_service
        .list_members(&channel.channel_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .into_iter()
        .map(|item| ChatChannelMemberResponse {
            channel_id: item.channel_id,
            team_id: item.team_id,
            user_id: item.user_id,
            role: item.role,
            joined_at: item.joined_at.to_chrono().to_rfc3339(),
        })
        .collect();
    Ok(Json(ChatChannelMembersEnvelope { members }))
}

async fn add_channel_member(
    State((service, db, _, _, _)): State<ChatState>,
    Extension(user): Extension<UserContext>,
    Path(channel_id): Path<String>,
    Json(request): Json<AddChatChannelMemberRequest>,
) -> Result<Json<ChatChannelMembersEnvelope>, StatusCode> {
    let channel_service = ChatChannelService::new(db.clone());
    let (channel, role, is_admin) =
        ensure_channel_access(service.as_ref(), &channel_service, &user, &channel_id).await?;
    if !can_manage_channel(role, is_admin) {
        return Err(StatusCode::FORBIDDEN);
    }
    let is_member = service
        .is_team_member(&request.user_id, &channel.team_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if !is_member {
        return Err(StatusCode::BAD_REQUEST);
    }
    channel_service
        .add_member(
            &channel,
            &request.user_id,
            request.role.unwrap_or(ChatChannelMemberRole::Member),
        )
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let members = channel_service
        .list_members(&channel.channel_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .into_iter()
        .map(|item| ChatChannelMemberResponse {
            channel_id: item.channel_id,
            team_id: item.team_id,
            user_id: item.user_id,
            role: item.role,
            joined_at: item.joined_at.to_chrono().to_rfc3339(),
        })
        .collect();
    Ok(Json(ChatChannelMembersEnvelope { members }))
}

async fn update_channel_member(
    State((service, db, _, _, _)): State<ChatState>,
    Extension(user): Extension<UserContext>,
    Path((channel_id, member_user_id)): Path<(String, String)>,
    Json(request): Json<UpdateChatChannelMemberRequest>,
) -> Result<Json<ChatChannelMembersEnvelope>, StatusCode> {
    let channel_service = ChatChannelService::new(db.clone());
    let (channel, role, is_admin) =
        ensure_channel_access(service.as_ref(), &channel_service, &user, &channel_id).await?;
    if !can_manage_channel(role, is_admin) {
        return Err(StatusCode::FORBIDDEN);
    }
    let updated = channel_service
        .update_member_role(&channel.channel_id, &member_user_id, request.role)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if !updated {
        return Err(StatusCode::NOT_FOUND);
    }
    let members = channel_service
        .list_members(&channel.channel_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .into_iter()
        .map(|item| ChatChannelMemberResponse {
            channel_id: item.channel_id,
            team_id: item.team_id,
            user_id: item.user_id,
            role: item.role,
            joined_at: item.joined_at.to_chrono().to_rfc3339(),
        })
        .collect();
    Ok(Json(ChatChannelMembersEnvelope { members }))
}

async fn remove_channel_member(
    State((service, db, _, _, _)): State<ChatState>,
    Extension(user): Extension<UserContext>,
    Path((channel_id, member_user_id)): Path<(String, String)>,
) -> Result<Json<ChatChannelMembersEnvelope>, StatusCode> {
    let channel_service = ChatChannelService::new(db.clone());
    let (channel, role, is_admin) =
        ensure_channel_access(service.as_ref(), &channel_service, &user, &channel_id).await?;
    if !can_manage_channel(role, is_admin) {
        return Err(StatusCode::FORBIDDEN);
    }
    if channel.created_by_user_id == member_user_id && !is_admin {
        return Err(StatusCode::CONFLICT);
    }
    let removed = channel_service
        .remove_member(&channel.channel_id, &member_user_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if !removed {
        return Err(StatusCode::NOT_FOUND);
    }
    let members = channel_service
        .list_members(&channel.channel_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .into_iter()
        .map(|item| ChatChannelMemberResponse {
            channel_id: item.channel_id,
            team_id: item.team_id,
            user_id: item.user_id,
            role: item.role,
            joined_at: item.joined_at.to_chrono().to_rfc3339(),
        })
        .collect();
    Ok(Json(ChatChannelMembersEnvelope { members }))
}

async fn list_channel_messages(
    State((service, db, _, _, _)): State<ChatState>,
    Extension(user): Extension<UserContext>,
    Path(channel_id): Path<String>,
    Query(query): Query<ChannelMessagesQuery>,
) -> Result<Json<ChatChannelMessagesEnvelope>, StatusCode> {
    let channel_service = ChatChannelService::new(db);
    let _ = ensure_channel_access(service.as_ref(), &channel_service, &user, &channel_id).await?;
    let messages = channel_service
        .list_root_messages(
            &channel_id,
            ChatChannelListFilters {
                surface: query.surface,
                thread_state: query.thread_state,
                display_kind: query.display_kind,
                display_status: query.display_status,
            },
        )
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(ChatChannelMessagesEnvelope { messages }))
}

async fn get_channel_thread(
    State((service, db, _, _, _)): State<ChatState>,
    Extension(user): Extension<UserContext>,
    Path((channel_id, thread_root_id)): Path<(String, String)>,
) -> Result<Json<ChatChannelThreadResponse>, StatusCode> {
    let channel_service = ChatChannelService::new(db);
    let _ = ensure_channel_access(service.as_ref(), &channel_service, &user, &channel_id).await?;
    let thread = channel_service
        .list_thread_messages(&channel_id, &thread_root_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    Ok(Json(thread))
}

async fn send_channel_message_internal(
    service: Arc<AgentService>,
    db: Arc<MongoDb>,
    channel_manager: Arc<ChatChannelManager>,
    workspace_root: String,
    user: UserContext,
    channel_id: String,
    request: SendChatChannelMessageRequest,
    forced_thread_root_id: Option<String>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let channel_service = ChatChannelService::new(db.clone());
    let (channel, _, _) =
        ensure_channel_access(service.as_ref(), &channel_service, &user, &channel_id).await?;
    let content = request.content.trim().to_string();
    if content.is_empty() || content.len() > 100_000 {
        return Err(StatusCode::BAD_REQUEST);
    }
    let attached_document_ids = request
        .attached_document_ids
        .iter()
        .filter_map(|raw| {
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        })
        .collect::<Vec<_>>();

    let requested_surface = request
        .surface
        .unwrap_or(ChatChannelMessageSurface::Activity);

    let thread_root_id = forced_thread_root_id.or(request.thread_root_id.clone());
    let thread_root = if let Some(root_id) = thread_root_id.as_deref() {
        let root = channel_service
            .get_message(root_id)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
            .ok_or(StatusCode::NOT_FOUND)?;
        if root.channel_id != channel.channel_id {
            return Err(StatusCode::FORBIDDEN);
        }
        Some(root)
    } else {
        None
    };
    let effective_surface = thread_root
        .as_ref()
        .map(|root| root.surface)
        .unwrap_or(requested_surface);
    let interaction_mode = request
        .interaction_mode
        .unwrap_or(ChatChannelInteractionMode::Conversation);
    let effective_thread_state = thread_root
        .as_ref()
        .map(|root| root.thread_state)
        .unwrap_or(ChatChannelThreadState::Active);

    if matches!(effective_thread_state, ChatChannelThreadState::Archived) {
        return Err(StatusCode::CONFLICT);
    }

    if thread_root.is_some() && matches!(effective_surface, ChatChannelMessageSurface::Activity) {
        return Err(StatusCode::BAD_REQUEST);
    }

    if let Some(parent_message_id) = request.parent_message_id.as_deref() {
        let parent = channel_service
            .get_message(parent_message_id)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
            .ok_or(StatusCode::NOT_FOUND)?;
        if parent.channel_id != channel.channel_id {
            return Err(StatusCode::FORBIDDEN);
        }
        if let Some(root_id) = thread_root_id.as_deref() {
            if parent
                .thread_root_id
                .as_deref()
                .unwrap_or(parent.message_id.as_str())
                != root_id
            {
                return Err(StatusCode::BAD_REQUEST);
            }
        }
    }

    if thread_root.is_none() && matches!(effective_surface, ChatChannelMessageSurface::Activity) {
        let user_message = channel_service
            .create_message(
                &channel,
                None,
                request.parent_message_id.clone(),
                ChatChannelMessageSurface::Activity,
                ChatChannelThreadState::Active,
                None,
                ChatChannelAuthorType::User,
                Some(user.user_id.clone()),
                None,
                user.display_name.clone(),
                None,
                content.clone(),
                serde_json::json!([{ "type": "text", "text": content }]),
                serde_json::json!({
                    "mentions": request.mentions,
                    "attached_document_ids": attached_document_ids,
                }),
            )
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        let _ = channel_service
            .note_orchestrator_discussion_activity(&channel, &user_message.message_id)
            .await;
        channel_service
            .mark_read(
                &channel,
                &user.user_id,
                Some(user_message.message_id.clone()),
            )
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        channel_service
            .refresh_channel_stats(&channel.channel_id)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        let _ = maybe_emit_manager_agent_follow_up(
            service.as_ref(),
            &channel_service,
            &channel,
            &user_message,
        )
        .await;
        let orchestrator = super::chat_channel_orchestrator::ChatChannelOrchestratorService::new(
            db.clone(),
            service.clone(),
        );
        orchestrator.spawn_quiet_window_summary(
            channel.channel_id.clone(),
            user_message.message_id.clone(),
        );
        return Ok(Json(serde_json::json!({
            "channel_id": channel.channel_id,
            "message_id": user_message.message_id,
            "thread_root_id": serde_json::Value::Null,
            "root_message_id": user_message.message_id,
            "run_id": serde_json::Value::Null,
            "streaming": false,
            "surface": ChatChannelMessageSurface::Activity,
            "thread_state": ChatChannelThreadState::Active,
        })));
    }

    let agent_id = request
        .agent_id
        .clone()
        .unwrap_or_else(|| channel.default_agent_id.clone());
    let agent = service
        .get_agent(&agent_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    if agent.team_id != channel.team_id {
        return Err(StatusCode::FORBIDDEN);
    }
    ensure_chat_agent_access(
        service.as_ref(),
        &db,
        &channel.team_id,
        &agent_id,
        &user.user_id,
    )
    .await?;

    let extra_instructions = build_channel_extra_instructions(
        &db,
        &user,
        &channel.team_id,
        &channel,
        &agent.name,
        thread_root.as_ref().map(|item| item.content_text.as_str()),
        attached_document_ids.len(),
        effective_surface,
        effective_thread_state,
        interaction_mode,
    )
    .await;
    let require_final_report =
        channel_surface_requires_final_report(effective_surface, interaction_mode);
    let session_source = channel_session_source_for_mode(interaction_mode);
    let pending_run = if let Some(scope_id) = thread_root_id.as_deref() {
        Some(
            channel_manager
                .register(
                    &channel.channel_id,
                    scope_id.to_string(),
                    Some(scope_id.to_string()),
                    Some(scope_id.to_string()),
                )
                .await
                .ok_or(StatusCode::CONFLICT)?,
        )
    } else {
        None
    };

    let user_message = channel_service
        .create_message(
            &channel,
            thread_root_id.clone(),
            request.parent_message_id.clone(),
            effective_surface,
            effective_thread_state,
            thread_root
                .as_ref()
                .and_then(|item| item.collaboration_status)
                .or(Some(super::chat_channels::ChatChannelDisplayStatus::Active)),
            ChatChannelAuthorType::User,
            Some(user.user_id.clone()),
            None,
            user.display_name.clone(),
            Some(agent_id.clone()),
            content.clone(),
            serde_json::json!([{ "type": "text", "text": content }]),
            serde_json::json!({
                "mentions": request.mentions,
                "selected_agent_id": agent_id,
                "selected_agent_name": agent.name,
                "attached_document_ids": attached_document_ids,
            }),
        )
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let effective_root_message_id = thread_root_id
        .clone()
        .unwrap_or_else(|| user_message.message_id.clone());
    let internal_session = find_or_create_channel_runtime_session(
        service.as_ref(),
        &channel,
        &user.user_id,
        &agent_id,
        &effective_root_message_id,
        attached_document_ids.clone(),
        extra_instructions,
        require_final_report,
        session_source,
    )
    .await?;
    let (cancel_token, _stream_tx, run_id) = if let Some(run) = pending_run {
        run
    } else {
        channel_manager
            .register(
                &channel.channel_id,
                effective_root_message_id.clone(),
                Some(effective_root_message_id.clone()),
                Some(effective_root_message_id.clone()),
            )
            .await
            .ok_or(StatusCode::CONFLICT)?
    };
    if matches!(interaction_mode, ChatChannelInteractionMode::Execution) {
        let claimed = channel_service
            .try_start_processing(&channel.channel_id, &run_id)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        if !claimed {
            channel_manager
                .unregister(&channel.channel_id, Some(&effective_root_message_id))
                .await;
            let _ = service
                .delete_session_if_idle(&internal_session.session_id)
                .await;
            return Err(StatusCode::CONFLICT);
        }
    }
    channel_service
        .mark_read(
            &channel,
            &user.user_id,
            Some(user_message.message_id.clone()),
        )
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let executor = ChatChannelExecutor::new(db.clone(), channel_manager.clone(), workspace_root);
    let request_for_exec = ExecuteChannelMessageRequest {
        channel_id: channel.channel_id.clone(),
        root_message_id: effective_root_message_id.clone(),
        run_scope_id: effective_root_message_id.clone(),
        thread_root_id: thread_root_id.clone(),
        assistant_parent_message_id: user_message.message_id.clone(),
        surface: effective_surface,
        thread_state: effective_thread_state,
        interaction_mode,
        internal_session_id: internal_session.session_id.clone(),
        agent_id: agent_id.clone(),
        user_message: content,
    };
    tokio::spawn(async move {
        if let Err(error) = executor
            .execute_channel_message(request_for_exec, cancel_token)
            .await
        {
            tracing::error!(
                "Channel message execution failed for {}: {}",
                channel_id,
                error
            );
        }
    });

    Ok(Json(serde_json::json!({
        "channel_id": channel.channel_id,
        "message_id": user_message.message_id,
        "thread_root_id": thread_root_id,
        "root_message_id": effective_root_message_id,
        "run_id": run_id,
        "streaming": true,
        "surface": effective_surface,
        "thread_state": effective_thread_state,
    })))
}

async fn send_channel_message(
    State((service, db, _, channel_manager, workspace_root)): State<ChatState>,
    Extension(user): Extension<UserContext>,
    Path(channel_id): Path<String>,
    Json(request): Json<SendChatChannelMessageRequest>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    send_channel_message_internal(
        service,
        db,
        channel_manager,
        workspace_root,
        user,
        channel_id,
        request,
        None,
    )
    .await
}

async fn send_channel_thread_message(
    State((service, db, _, channel_manager, workspace_root)): State<ChatState>,
    Extension(user): Extension<UserContext>,
    Path((channel_id, thread_root_id)): Path<(String, String)>,
    Json(request): Json<SendChatChannelMessageRequest>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    send_channel_message_internal(
        service,
        db,
        channel_manager,
        workspace_root,
        user,
        channel_id,
        request,
        Some(thread_root_id),
    )
    .await
}

async fn promote_channel_message_to_issue(
    State((service, db, _, _, _)): State<ChatState>,
    Extension(user): Extension<UserContext>,
    Path((channel_id, message_id)): Path<(String, String)>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let channel_service = ChatChannelService::new(db);
    let _ = ensure_channel_access(service.as_ref(), &channel_service, &user, &channel_id).await?;
    let root = channel_service
        .get_message(&message_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    if root.channel_id != channel_id {
        return Err(StatusCode::FORBIDDEN);
    }
    if root.thread_root_id.is_some() {
        return Err(StatusCode::BAD_REQUEST);
    }
    if matches!(root.surface, ChatChannelMessageSurface::Issue) {
        return Ok(Json(serde_json::json!({
            "channel_id": channel_id,
            "message_id": message_id,
            "surface": ChatChannelMessageSurface::Issue,
            "thread_state": root.thread_state,
        })));
    }
    if !matches!(root.surface, ChatChannelMessageSurface::Temporary) {
        return Err(StatusCode::BAD_REQUEST);
    }
    let updated = channel_service
        .update_thread_classification(
            &channel_id,
            &message_id,
            Some(ChatChannelMessageSurface::Issue),
            Some(ChatChannelThreadState::Active),
            None,
        )
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if !updated {
        return Err(StatusCode::NOT_FOUND);
    }
    Ok(Json(serde_json::json!({
        "channel_id": channel_id,
        "message_id": message_id,
        "surface": ChatChannelMessageSurface::Issue,
        "thread_state": ChatChannelThreadState::Active,
    })))
}

async fn archive_channel_thread(
    State((service, db, _, _, _)): State<ChatState>,
    Extension(user): Extension<UserContext>,
    Path((channel_id, message_id)): Path<(String, String)>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let channel_service = ChatChannelService::new(db);
    let _ = ensure_channel_access(service.as_ref(), &channel_service, &user, &channel_id).await?;
    let root = channel_service
        .get_message(&message_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    if root.channel_id != channel_id {
        return Err(StatusCode::FORBIDDEN);
    }
    if root.thread_root_id.is_some() {
        return Err(StatusCode::BAD_REQUEST);
    }
    if matches!(root.surface, ChatChannelMessageSurface::Activity) {
        return Err(StatusCode::BAD_REQUEST);
    }
    let updated = channel_service
        .update_thread_classification(
            &channel_id,
            &message_id,
            None,
            Some(ChatChannelThreadState::Archived),
            Some(Some(ChatChannelDisplayStatus::Rejected)),
        )
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if !updated {
        return Err(StatusCode::NOT_FOUND);
    }
    Ok(Json(serde_json::json!({
        "channel_id": channel_id,
        "message_id": message_id,
        "surface": root.surface,
        "thread_state": ChatChannelThreadState::Archived,
    })))
}

async fn sync_channel_collaboration_result(
    State((service, db, _, _, _)): State<ChatState>,
    Extension(user): Extension<UserContext>,
    Path((channel_id, message_id)): Path<(String, String)>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let channel_service = ChatChannelService::new(db.clone());
    let _ = ensure_channel_access(service.as_ref(), &channel_service, &user, &channel_id).await?;
    let root = channel_service
        .get_message(&message_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    if root.channel_id != channel_id || root.thread_root_id.is_some() {
        return Err(StatusCode::BAD_REQUEST);
    }
    if matches!(root.surface, ChatChannelMessageSurface::Activity) {
        return Err(StatusCode::BAD_REQUEST);
    }
    let metadata_obj = root.metadata.as_object().cloned().unwrap_or_default();
    let summary = metadata_obj
        .get("summary_text")
        .and_then(|value| value.as_str())
        .filter(|value| !value.trim().is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| clamp_summary(&root.content_text, 160));
    let status_label = match root.collaboration_status {
        Some(ChatChannelDisplayStatus::Adopted) => "已采用",
        Some(ChatChannelDisplayStatus::Rejected) => "未采用",
        Some(ChatChannelDisplayStatus::AwaitingConfirmation) => "等你判断",
        Some(ChatChannelDisplayStatus::Proposed) => "建议",
        _ => "推进中",
    };
    let latest_actor = root.author_name.clone();
    let text = format!(
        "协作结果同步\n{}\n当前状态：{}\n最近推进者：{}",
        summary, status_label, latest_actor
    );
    let channel = channel_service
        .get_channel(&channel_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    let agent_name = metadata_obj
        .get("selected_agent_name")
        .and_then(|value| value.as_str())
        .filter(|value| !value.trim().is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| channel.default_agent_id.clone());
    let result_message = channel_service
        .create_message(
            &channel,
            None,
            Some(message_id.clone()),
            ChatChannelMessageSurface::Activity,
            ChatChannelThreadState::Active,
            root.collaboration_status,
            ChatChannelAuthorType::Agent,
            None,
            None,
            agent_name,
            root.agent_id.clone(),
            text.clone(),
            serde_json::json!([{ "type": "text", "text": text }]),
            serde_json::json!({
                "discussion_card_kind": "result",
                "card_purpose": "collaboration_result_sync",
                "linked_collaboration_id": message_id,
                "summary_text": summary,
                "status_label": status_label,
                "latest_actor": latest_actor,
            }),
        )
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let _ = channel_service
        .record_orchestrator_heartbeat(
            &channel,
            "result_sync",
            Some(format!("result:{}", message_id)),
        )
        .await;
    Ok(Json(serde_json::json!({
        "channel_id": channel_id,
        "message_id": result_message.message_id,
        "linked_collaboration_id": message_id,
        "display_kind": ChatChannelDisplayKind::Result,
    })))
}

#[derive(Debug, Clone, serde::Deserialize)]
struct UpdateCollaborationStatusRequest {
    status: ChatChannelDisplayStatus,
}

async fn update_channel_collaboration_status(
    State((service, db, _, _, _)): State<ChatState>,
    Extension(user): Extension<UserContext>,
    Path((channel_id, message_id)): Path<(String, String)>,
    Json(request): Json<UpdateCollaborationStatusRequest>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let channel_service = ChatChannelService::new(db);
    let (channel, _, _) =
        ensure_channel_access(service.as_ref(), &channel_service, &user, &channel_id).await?;
    let root = channel_service
        .get_message(&message_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    if root.channel_id != channel_id || root.thread_root_id.is_some() {
        return Err(StatusCode::BAD_REQUEST);
    }
    if matches!(root.surface, ChatChannelMessageSurface::Activity)
        && !matches!(
            request.status,
            ChatChannelDisplayStatus::Proposed | ChatChannelDisplayStatus::Rejected
        )
    {
        return Err(StatusCode::BAD_REQUEST);
    }
    let rejected_suggestion_fingerprint =
        if matches!(root.surface, ChatChannelMessageSurface::Activity)
            && matches!(request.status, ChatChannelDisplayStatus::Rejected)
        {
            derive_suggestion_fingerprint(&channel_service, &root)
                .await
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        } else {
            None
        };
    let updated = channel_service
        .update_thread_classification(
            &channel_id,
            &message_id,
            None,
            Some(
                if matches!(request.status, ChatChannelDisplayStatus::Rejected) {
                    ChatChannelThreadState::Archived
                } else {
                    ChatChannelThreadState::Active
                },
            ),
            Some(Some(request.status)),
        )
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if !updated {
        return Err(StatusCode::NOT_FOUND);
    }
    if let Some(fingerprint) = rejected_suggestion_fingerprint {
        let _ = channel_service
            .ignore_orchestrator_suggestion_fingerprint(&channel, &fingerprint)
            .await;
    }
    Ok(Json(serde_json::json!({
        "channel_id": channel_id,
        "message_id": message_id,
        "display_status": request.status,
    })))
}

async fn stream_channel(
    State((service, db, _, channel_manager, _)): State<ChatState>,
    Extension(user): Extension<UserContext>,
    headers: HeaderMap,
    Path(channel_id): Path<String>,
    Query(q): Query<StreamQuery>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, StatusCode> {
    let channel_service = ChatChannelService::new(db);
    let _ = ensure_channel_access(service.as_ref(), &channel_service, &user, &channel_id).await?;
    let last_event_id = q.last_event_id.or_else(|| {
        headers
            .get("last-event-id")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.parse::<u64>().ok())
    });
    let (mut rx, history) = channel_manager
        .subscribe_with_history(&channel_id, last_event_id)
        .await
        .ok_or(StatusCode::NOT_FOUND)?;

    let stream = async_stream::stream! {
        yield Ok(Event::default()
            .event("status")
            .data(serde_json::json!({
                "type": "Status",
                "status": "running"
            }).to_string()));

        for event in history {
            let is_done = event.event.is_done();
            let json = serde_json::to_string(&event.event).unwrap_or_default();
            let mut sse = Event::default().event(event.event.event_type()).data(json);
            if event.id > 0 {
                sse = sse.id(event.id.to_string());
            }
            yield Ok(sse);
            if is_done {
                return;
            }
        }

        let deadline = tokio::time::Instant::now() + Duration::from_secs(2 * 60 * 60);
        loop {
            match tokio::time::timeout_at(deadline, rx.recv()).await {
                Ok(Ok(event)) => {
                    let is_done = event.event.is_done();
                    let json = serde_json::to_string(&event.event).unwrap_or_default();
                    let mut sse = Event::default().event(event.event.event_type()).data(json);
                    if event.id > 0 {
                        sse = sse.id(event.id.to_string());
                    }
                    yield Ok(sse);
                    if is_done {
                        break;
                    }
                }
                Ok(Err(tokio::sync::broadcast::error::RecvError::Closed)) => break,
                Ok(Err(tokio::sync::broadcast::error::RecvError::Lagged(_))) => continue,
                Err(_) => break,
            }
        }
    };

    Ok(Sse::new(stream).keep_alive(
        axum::response::sse::KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("ping"),
    ))
}

async fn list_channel_events(
    State((service, db, _, channel_manager, _)): State<ChatState>,
    Extension(user): Extension<UserContext>,
    Path(channel_id): Path<String>,
    Query(query): Query<ChannelEventListQuery>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let channel_service = ChatChannelService::new(db.clone());
    let _ = ensure_channel_access(service.as_ref(), &channel_service, &user, &channel_id).await?;
    let descending = query
        .order
        .as_deref()
        .map(str::trim)
        .map(|value| value.eq_ignore_ascii_case("desc"))
        .unwrap_or(false);
    let limit = query.limit.unwrap_or(500).clamp(1, 2000);
    let selected_run_id = match query
        .run_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        Some(value) => Some(value.to_string()),
        None => channel_manager.active_run_id(&channel_id).await,
    };
    let events = channel_service
        .list_channel_events(&channel_id, selected_run_id.as_deref(), limit, descending)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let mut value = serde_json::to_value(events).unwrap_or_default();
    fix_bson_dates(&mut value);
    Ok(Json(value))
}

async fn mark_channel_read(
    State((service, db, _, _, _)): State<ChatState>,
    Extension(user): Extension<UserContext>,
    Path(channel_id): Path<String>,
    Json(body): Json<MarkChatChannelReadRequest>,
) -> Result<Json<ChatChannelReadResponse>, StatusCode> {
    let channel_service = ChatChannelService::new(db);
    let (channel, _, _) =
        ensure_channel_access(service.as_ref(), &channel_service, &user, &channel_id).await?;
    let read = channel_service
        .mark_read(&channel, &user.user_id, body.last_read_message_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(ChatChannelReadResponse {
        channel_id: read.channel_id,
        user_id: read.user_id,
        last_read_message_id: read.last_read_message_id,
        last_read_at: read.last_read_at.to_chrono().to_rfc3339(),
    }))
}

/// GET /chat/sessions - List user's chat sessions
async fn list_sessions(
    State((service, _, _, _, _)): State<ChatState>,
    Extension(user): Extension<UserContext>,
    Query(mut query): Query<UserSessionListQuery>,
) -> Result<Json<Vec<SessionListItem>>, StatusCode> {
    let is_member = service
        .is_team_member(&user.user_id, &query.team_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    if !is_member {
        return Err(StatusCode::FORBIDDEN);
    }

    // C1 fix: Always inject authenticated user_id to prevent data leakage
    query.user_id = Some(user.user_id.clone());

    service
        .list_user_sessions(query)
        .await
        .map(Json)
        .map_err(|e| {
            tracing::error!("Failed to list sessions: {:?}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })
}

/// POST /chat/sessions - Create a new chat session
async fn create_session(
    State((service, db, _, _, _)): State<ChatState>,
    Extension(user): Extension<UserContext>,
    Json(req): Json<CreateChatSessionRequest>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    // Look up agent to get team_id
    let team_id = service
        .get_agent_team_id(&req.agent_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    let is_member = service
        .is_team_member(&user.user_id, &team_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    if !is_member {
        return Err(StatusCode::FORBIDDEN);
    }

    // Enforce agent group-based access control
    let user_group_ids =
        agime_team::services::mongo::user_group_service_mongo::UserGroupService::new((*db).clone())
            .get_user_group_ids(&team_id, &user.user_id)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let has_agent_access = service
        .check_agent_access(&req.agent_id, &user.user_id, &user_group_ids)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if !has_agent_access {
        return Err(StatusCode::FORBIDDEN);
    }
    let extra_instructions = if req.portal_restricted {
        req.extra_instructions
    } else {
        let team_settings = TeamService::new((*db).clone())
            .get_settings(&team_id)
            .await
            .ok();
        let memory = ChatMemoryService::new(db.clone())
            .get_memory(&team_id, &user.user_id)
            .await
            .ok()
            .flatten();
        merge_chat_extra_instructions(
            std::iter::once(build_assistant_identity_overlay())
                .chain(std::iter::once(build_assistant_persona_overlay(&user)))
                .chain(
                    team_settings
                        .as_ref()
                        .and_then(build_team_context_overlay)
                        .into_iter(),
                )
                .chain(std::iter::once(render_user_relationship_overlay(
                    &user.display_name,
                    memory.as_ref(),
                )))
                .chain(std::iter::once(build_personal_document_scope_overlay(
                    req.attached_document_ids.len(),
                )))
                .collect(),
            req.extra_instructions,
        )
    };

    let effective_document_access_mode = req
        .document_access_mode
        .clone()
        .or_else(|| Some("attached_only".to_string()));

    let session = service
        .create_chat_session(
            &team_id,
            &req.agent_id,
            &user.user_id,
            req.attached_document_ids,
            extra_instructions,
            req.allowed_extensions,
            req.allowed_skill_ids,
            req.retry_config,
            req.max_turns,
            req.tool_timeout_seconds,
            req.max_portal_retry_rounds,
            req.require_final_report,
            req.portal_restricted,
            effective_document_access_mode,
            req.delegation_policy_override,
            None,
            None,
        )
        .await
        .map_err(|e| {
            tracing::error!("Failed to create chat session: {:?}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    Ok(Json(serde_json::json!({
        "session_id": session.session_id,
        "agent_id": session.agent_id,
        "status": session.status,
    })))
}

/// GET /chat/agents/{agent_id}/composer-capabilities - Resolved skills/extensions for a new chat
async fn get_agent_composer_capabilities(
    State((service, db, _, _, _)): State<ChatState>,
    Extension(user): Extension<UserContext>,
    headers: HeaderMap,
    Path(agent_id): Path<String>,
) -> Result<Json<ComposerCapabilitiesCatalog>, StatusCode> {
    let team_id = service
        .get_agent_team_id(&agent_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    let is_member = service
        .is_team_member(&user.user_id, &team_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if !is_member {
        return Err(StatusCode::FORBIDDEN);
    }

    let user_group_ids =
        agime_team::services::mongo::user_group_service_mongo::UserGroupService::new((*db).clone())
            .get_user_group_ids(&team_id, &user.user_id)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let has_agent_access = service
        .check_agent_access(&agent_id, &user.user_id, &user_group_ids)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if !has_agent_access {
        return Err(StatusCode::FORBIDDEN);
    }

    let agent = service
        .get_agent(&agent_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    let preferred_lang = preferred_lang_from_headers(&headers);

    Ok(Json(
        build_composer_capability_catalog(&db, &team_id, &agent, None, preferred_lang.as_deref())
            .await,
    ))
}

/// GET /chat/sessions/{id}/composer-capabilities - Resolved skills/extensions for an existing chat
async fn get_session_composer_capabilities(
    State((service, db, _, _, _)): State<ChatState>,
    Extension(user): Extension<UserContext>,
    headers: HeaderMap,
    Path(session_id): Path<String>,
) -> Result<Json<ComposerCapabilitiesCatalog>, StatusCode> {
    let session = service
        .get_session(&session_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    if session.user_id != user.user_id {
        return Err(StatusCode::FORBIDDEN);
    }

    let agent = service
        .get_agent(&session.agent_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    let preferred_lang = preferred_lang_from_headers(&headers);

    Ok(Json(
        build_composer_capability_catalog(
            &db,
            &agent.team_id,
            &agent,
            Some(&session),
            preferred_lang.as_deref(),
        )
        .await,
    ))
}

#[derive(serde::Deserialize)]
struct CreatePortalCodingSessionRequest {
    team_id: String,
    portal_id: String,
    #[serde(default)]
    retry_config: Option<RetryConfig>,
    #[serde(default)]
    max_turns: Option<i32>,
    #[serde(default)]
    tool_timeout_seconds: Option<u64>,
    #[serde(default)]
    max_portal_retry_rounds: Option<u32>,
    #[serde(default)]
    require_final_report: Option<bool>,
}

#[derive(serde::Deserialize)]
struct CreatePortalManagerSessionRequest {
    team_id: String,
    #[serde(default)]
    manager_agent_id: Option<String>,
    #[serde(default)]
    portal_id: Option<String>,
    #[serde(default)]
    retry_config: Option<RetryConfig>,
    #[serde(default)]
    max_turns: Option<i32>,
    #[serde(default)]
    tool_timeout_seconds: Option<u64>,
    #[serde(default)]
    max_portal_retry_rounds: Option<u32>,
    #[serde(default)]
    require_final_report: Option<bool>,
}

fn has_manager_tooling(agent: &TeamAgent) -> bool {
    let runtime_snapshot = AgentRuntimePolicyResolver::resolve(agent, None, None);
    runtime_snapshot
        .extensions
        .effective_allowed_extension_names
        .iter()
        .any(|name| {
            matches!(
                name.as_str(),
                "developer" | "portal_tools" | "extension_manager"
            )
        })
}

async fn resolve_manager_agent_id(
    service: &AgentService,
    team_id: &str,
    manager_agent_id: Option<&str>,
) -> Result<String, StatusCode> {
    let requested = manager_agent_id
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);

    if let Some(agent_id) = requested {
        let agent = service
            .get_agent(&agent_id)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
            .ok_or(StatusCode::NOT_FOUND)?;
        if agent.team_id != team_id {
            return Err(StatusCode::FORBIDDEN);
        }
        return Ok(agent_id);
    }

    let agents = service
        .list_agents(ListAgentsQuery {
            team_id: team_id.to_string(),
            page: 1,
            limit: 100,
        })
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if agents.items.is_empty() {
        return Err(StatusCode::BAD_REQUEST);
    }

    if let Some(agent) = agents.items.iter().find(|agent| has_manager_tooling(agent)) {
        return Ok(agent.id.clone());
    }

    Ok(agents.items[0].id.clone())
}

/// POST /chat/sessions/portal-coding - Create a portal lab coding session with strict policy.
async fn create_portal_coding_session(
    State((service, db, _, _, _)): State<ChatState>,
    Extension(user): Extension<UserContext>,
    Json(req): Json<CreatePortalCodingSessionRequest>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let is_member = service
        .is_team_member(&user.user_id, &req.team_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if !is_member {
        return Err(StatusCode::FORBIDDEN);
    }
    let is_admin = service
        .is_team_admin(&user.user_id, &req.team_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if !is_admin {
        return Err(StatusCode::FORBIDDEN);
    }

    let portal_svc = PortalService::new((*db).clone());
    let portal = portal_svc
        .get(&req.team_id, &req.portal_id)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;
    let portal_id = portal
        .id
        .map(|id| id.to_hex())
        .ok_or(StatusCode::INTERNAL_SERVER_ERROR)?;

    let agent_id = portal
        .coding_agent_id
        .clone()
        .or_else(|| portal.agent_id.clone())
        .or_else(|| portal.service_agent_id.clone())
        .ok_or(StatusCode::BAD_REQUEST)?;

    // Enforce agent group-based access control
    let user_group_ids =
        agime_team::services::mongo::user_group_service_mongo::UserGroupService::new((*db).clone())
            .get_user_group_ids(&req.team_id, &user.user_id)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let has_agent_access = service
        .check_agent_access(&agent_id, &user.user_id, &user_group_ids)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if !has_agent_access {
        return Err(StatusCode::FORBIDDEN);
    }

    let raw_project_path = portal.project_path.clone().ok_or(StatusCode::BAD_REQUEST)?;
    let project_path = normalize_workspace_path(&raw_project_path);
    let portal_slug = portal.slug.clone();

    if project_path != raw_project_path {
        if let Err(e) = portal_svc
            .set_project_path(&req.team_id, &portal_id, &project_path)
            .await
        {
            tracing::warn!(
                "Failed to normalize project_path for portal {}: {}",
                portal_id,
                e
            );
        }
    }

    // Ensure project directory exists; auto-create if missing
    if !std::path::Path::new(&project_path).exists() {
        tracing::warn!("Portal project_path missing, recreating: {}", project_path);
        if let Err(e) = std::fs::create_dir_all(&project_path) {
            tracing::error!("Failed to create project dir {}: {}", project_path, e);
            return Err(StatusCode::INTERNAL_SERVER_ERROR);
        }
    }

    // Ensure selected coding agent can actually run developer tools.
    let agent = service
        .get_agent(&agent_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    let runtime_snapshot = AgentRuntimePolicyResolver::resolve(&agent, None, None);
    let has_developer = runtime_snapshot
        .extensions
        .effective_allowed_extension_names
        .iter()
        .any(|name| name == "developer");
    if !has_developer {
        return Err(StatusCode::BAD_REQUEST);
    }

    // Inject project directory context so the agent knows the current state
    let project_ctx = super::runtime::scan_project_context(&project_path, 8000);
    let portal_policy_overlay = portal.agent_system_prompt.clone();
    let extra = build_portal_coding_overlay(PortalCodingProfileInput {
        portal_slug: &portal_slug,
        project_path: &project_path,
        portal_policy_overlay: portal_policy_overlay.as_deref(),
        project_context: if project_ctx.trim().is_empty() {
            None
        } else {
            Some(project_ctx.as_str())
        },
    });

    let effective_retry_config = req
        .retry_config
        .clone()
        .unwrap_or_else(default_portal_retry_config);

    let session = service
        .create_chat_session(
            &req.team_id,
            &agent_id,
            &user.user_id,
            portal.bound_document_ids.clone(),
            Some(extra),
            None,
            None,
            Some(effective_retry_config),
            req.max_turns,
            req.tool_timeout_seconds,
            req.max_portal_retry_rounds,
            req.require_final_report.unwrap_or(false),
            false,
            Some("full".to_string()),
            None,
            Some("portal_coding".to_string()),
            Some(true),
        )
        .await
        .map_err(|e| {
            tracing::error!("Failed to create portal coding session: {:?}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    service
        .set_session_workspace(&session.session_id, &project_path)
        .await
        .map_err(|e| {
            tracing::error!(
                "Failed to set workspace for portal coding session {}: {:?}",
                session.session_id,
                e
            );
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    service
        .set_session_portal_context(
            &session.session_id,
            &portal_id,
            &portal_slug,
            None,
            Some("full"),
            false,
        )
        .await
        .map_err(|e| {
            tracing::error!(
                "Failed to set portal context for portal coding session {}: {:?}",
                session.session_id,
                e
            );
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    Ok(Json(serde_json::json!({
        "session_id": session.session_id,
        "agent_id": session.agent_id,
        "status": session.status,
        "portal_restricted": false,
        "workspace_path": project_path,
        "allowed_extensions": serde_json::Value::Null,
        "retry_config": session.retry_config,
        "max_turns": session.max_turns,
        "tool_timeout_seconds": session.tool_timeout_seconds,
        "max_portal_retry_rounds": session.max_portal_retry_rounds,
        "require_final_report": session.require_final_report,
    })))
}

/// POST /chat/sessions/portal-manager - Create team-level portal manager session.
/// This session is used to create/configure digital avatars before any portal exists.
async fn create_portal_manager_session(
    State((service, db, _, _, _)): State<ChatState>,
    Extension(user): Extension<UserContext>,
    Json(req): Json<CreatePortalManagerSessionRequest>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let is_member = service
        .is_team_member(&user.user_id, &req.team_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if !is_member {
        return Err(StatusCode::FORBIDDEN);
    }
    let is_admin = service
        .is_team_admin(&user.user_id, &req.team_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if !is_admin {
        return Err(StatusCode::FORBIDDEN);
    }

    let portal_context = if let Some(portal_id) = req.portal_id.as_deref() {
        let portal = PortalService::new((*db).clone())
            .get(&req.team_id, portal_id)
            .await
            .map_err(|_| StatusCode::NOT_FOUND)?;
        Some(PortalDetail::from(portal))
    } else {
        None
    };

    let requested_manager_agent_id = if let Some(portal) = portal_context.as_ref() {
        let bound_manager_id: Option<&str> = portal
            .coding_agent_id
            .as_deref()
            .or(portal.agent_id.as_deref())
            .map(str::trim)
            .filter(|value: &&str| !value.is_empty());
        match (req.manager_agent_id.as_deref(), bound_manager_id) {
            (Some(requested), Some(bound)) if requested != bound => {
                return Err(StatusCode::BAD_REQUEST);
            }
            (Some(requested), _) => Some(requested),
            (None, Some(bound)) => Some(bound),
            (None, None) => None,
        }
    } else {
        req.manager_agent_id.as_deref()
    };

    let manager_agent_id =
        resolve_manager_agent_id(&service, &req.team_id, requested_manager_agent_id).await?;

    // Enforce agent group-based access control
    let user_group_ids =
        agime_team::services::mongo::user_group_service_mongo::UserGroupService::new((*db).clone())
            .get_user_group_ids(&req.team_id, &user.user_id)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let has_agent_access = service
        .check_agent_access(&manager_agent_id, &user.user_id, &user_group_ids)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if !has_agent_access {
        return Err(StatusCode::FORBIDDEN);
    }

    let mut extra = build_portal_manager_overlay();
    if let Some(portal) = portal_context.as_ref() {
        let service_agent_id = portal
            .service_agent_id
            .as_deref()
            .or(portal.agent_id.as_deref())
            .map(str::trim)
            .filter(|value: &&str| !value.is_empty())
            .unwrap_or("未配置");
        let bound_documents = portal.bound_document_ids.len();
        let document_access_mode = format!("{:?}", portal.document_access_mode).to_lowercase();
        let allowed_extensions = portal
            .allowed_extensions
            .as_ref()
            .map(|items: &Vec<String>| items.join(", "))
            .filter(|value: &String| !value.trim().is_empty())
            .unwrap_or_else(|| "继承服务 Agent".to_string());
        let allowed_skills = portal
            .allowed_skill_ids
            .as_ref()
            .map(|items: &Vec<String>| items.join(", "))
            .filter(|value: &String| !value.trim().is_empty())
            .unwrap_or_else(|| "继承服务 Agent".to_string());
        extra.push_str(
            &format!(
                "\n\n【Current Avatar Context】\n当前默认工作目标数字分身：{name}\nportal_id: {portal_id}\nslug: {slug}\nservice_agent_id: {service_agent_id}\ndocument_access_mode: {document_access_mode}\nbound_document_count: {bound_documents}\nallowed_extensions: {allowed_extensions}\nallowed_skill_ids: {allowed_skills}\n说明：除非用户明确切换其他分身，本会话里的创建、配置、治理、审批与发布默认都针对这个数字分身。",
                name = portal.name,
                portal_id = portal.id,
                slug = portal.slug,
                service_agent_id = service_agent_id,
                document_access_mode = document_access_mode,
                bound_documents = bound_documents,
                allowed_extensions = allowed_extensions,
                allowed_skills = allowed_skills,
            ),
        );
    }

    let effective_retry_config = req
        .retry_config
        .clone()
        .unwrap_or_else(default_portal_retry_config);

    let session = service
        .create_chat_session(
            &req.team_id,
            &manager_agent_id,
            &user.user_id,
            Vec::new(),
            Some(extra),
            None,
            None,
            Some(effective_retry_config),
            req.max_turns,
            req.tool_timeout_seconds,
            req.max_portal_retry_rounds,
            req.require_final_report.unwrap_or(false),
            false,
            Some("full".to_string()),
            None,
            Some("portal_manager".to_string()),
            Some(true),
        )
        .await
        .map_err(|e| {
            tracing::error!("Failed to create portal manager session: {:?}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    if let Some(portal) = portal_context.as_ref() {
        if let Err(err) = service
            .set_session_portal_context(
                &session.session_id,
                &portal.id,
                &portal.slug,
                None,
                Some("full"),
                false,
            )
            .await
        {
            tracing::error!(
                session_id = %session.session_id,
                portal_id = %portal.id,
                "Failed to bind portal manager session to avatar context: {:?}",
                err
            );
            let _ = service.delete_session(&session.session_id).await;
            return Err(StatusCode::INTERNAL_SERVER_ERROR);
        }
    }

    Ok(Json(serde_json::json!({
        "session_id": session.session_id,
        "agent_id": session.agent_id,
        "status": session.status,
        "portal_restricted": false,
        "portal_id": portal_context.as_ref().map(|portal| portal.id.clone()),
        "portal_slug": portal_context.as_ref().map(|portal| portal.slug.clone()),
        "allowed_extensions": serde_json::Value::Null,
        "retry_config": session.retry_config,
        "max_turns": session.max_turns,
        "tool_timeout_seconds": session.tool_timeout_seconds,
        "max_portal_retry_rounds": session.max_portal_retry_rounds,
        "require_final_report": session.require_final_report,
    })))
}

/// GET /chat/sessions/{id} - Get session details with messages
async fn get_session(
    State((service, db, _, _, _)): State<ChatState>,
    Extension(user): Extension<UserContext>,
    Path(session_id): Path<String>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let session = service
        .get_session(&session_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    // Verify ownership
    if session.user_id != user.user_id {
        return Err(StatusCode::FORBIDDEN);
    }

    // H4 fix: Convert bson::DateTime to ISO 8601 strings for frontend
    let mut json = serde_json::json!({
        "session_id": session.session_id,
        "team_id": session.team_id,
        "agent_id": session.agent_id,
        "user_id": session.user_id,
        "name": session.name,
        "status": session.status,
        "messages_json": session.messages_json,
        "message_count": session.message_count,
        "total_tokens": session.total_tokens,
        "input_tokens": session.input_tokens,
        "output_tokens": session.output_tokens,
        "compaction_count": session.compaction_count,
        "disabled_extensions": session.disabled_extensions,
        "enabled_extensions": session.enabled_extensions,
        "created_at": session.created_at.to_chrono().to_rfc3339(),
        "updated_at": session.updated_at.to_chrono().to_rfc3339(),
        "title": session.title,
        "pinned": session.pinned,
        "last_message_preview": session.last_message_preview,
        "is_processing": session.is_processing,
        "last_execution_status": session.last_execution_status,
        "last_execution_error": session.last_execution_error,
        "workspace_path": session.workspace_path,
        "extra_instructions": session.extra_instructions,
        "allowed_extensions": session.allowed_extensions,
        "allowed_skill_ids": session.allowed_skill_ids,
        "retry_config": session.retry_config,
        "max_turns": session.max_turns,
        "tool_timeout_seconds": session.tool_timeout_seconds,
        "max_portal_retry_rounds": session.max_portal_retry_rounds,
        "require_final_report": session.require_final_report,
        "portal_restricted": session.portal_restricted,
        "document_access_mode": session.document_access_mode,
        "document_scope_mode": session.document_scope_mode,
        "document_write_mode": session.document_write_mode,
        "portal_id": session.portal_id,
        "portal_slug": session.portal_slug,
        "visitor_id": session.visitor_id,
        "session_source": session.session_source,
        "hidden_from_chat_list": session.hidden_from_chat_list,
    });

    if let Some(lma) = session.last_message_at {
        json["last_message_at"] = serde_json::Value::String(lma.to_chrono().to_rfc3339());
    }
    if let Some(finished_at) = session.last_execution_finished_at {
        json["last_execution_finished_at"] =
            serde_json::Value::String(finished_at.to_chrono().to_rfc3339());
    }
    let agent = service
        .get_agent(&session.agent_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let runtime_snapshot = resolve_runtime_snapshot_for_session(&service, &db, &session).await;
    let tasks_enabled = runtime_snapshot
        .as_ref()
        .map(|snapshot| {
            snapshot
                .extensions
                .effective_allowed_extension_names
                .iter()
                .any(|name| name == "tasks")
        })
        .unwrap_or(false);
    json["tasks_enabled"] = serde_json::Value::Bool(tasks_enabled);
    if let Some(snapshot) = runtime_snapshot.as_ref() {
        if let Some(agent) = agent.as_ref() {
            let prompt_introspection = build_prompt_introspection_snapshot(
                snapshot,
                agent.model.as_deref().unwrap_or_default(),
                session.require_final_report || snapshot.delegation_policy.require_final_report,
            );
            json["prompt_snapshot_version"] =
                serde_json::Value::String(prompt_introspection.prompt_snapshot_version.clone());
            json["capability_snapshot"] =
                serde_json::to_value(&prompt_introspection.capability_snapshot)
                    .unwrap_or(serde_json::Value::Null);
            json["delegation_snapshot"] =
                serde_json::to_value(&prompt_introspection.delegation_snapshot)
                    .unwrap_or(serde_json::Value::Null);
            json["harness_capabilities"] =
                serde_json::to_value(&prompt_introspection.harness_capabilities)
                    .unwrap_or(serde_json::Value::Null);
            json["subagent_enabled"] =
                serde_json::Value::Bool(prompt_introspection.subagent_enabled);
            json["swarm_enabled"] = serde_json::Value::Bool(prompt_introspection.swarm_enabled);
            json["worker_peer_messaging_enabled"] =
                serde_json::Value::Bool(prompt_introspection.worker_peer_messaging_enabled);
            json["validation_worker_enabled"] =
                serde_json::Value::Bool(prompt_introspection.validation_worker_enabled);
        }
    }

    if tasks_enabled {
        if let Some(tasks_state) = load_runtime_tasks_snapshot(
            &session.session_id,
            session.last_runtime_session_id.as_deref(),
        )
        .await
        {
            json["task_board_id"] = serde_json::Value::String(tasks_state.board_id.clone());
            json["current_tasks"] = serde_json::to_value(&tasks_state.items)
                .unwrap_or(serde_json::Value::Array(vec![]));
            json["task_summary"] = build_task_summary(&tasks_state);
        } else {
            json["task_board_id"] = serde_json::Value::String(session.session_id.clone());
            json["current_tasks"] = serde_json::Value::Array(vec![]);
            json["task_summary"] = serde_json::json!({
                "board_id": session.session_id,
                "scope": "standalone",
                "task_count": 0,
                "pending_count": 0,
                "in_progress_count": 0,
                "completed_count": 0,
            });
        }
    }

    Ok(Json(json))
}

/// PUT /chat/sessions/{id} - Update session (rename/pin)
#[derive(serde::Deserialize)]
struct UpdateSessionBody {
    title: Option<String>,
    pinned: Option<bool>,
}

async fn update_session(
    State((service, _, _, _, _)): State<ChatState>,
    Extension(user): Extension<UserContext>,
    Path(session_id): Path<String>,
    Json(body): Json<UpdateSessionBody>,
) -> Result<StatusCode, StatusCode> {
    let session = service
        .get_session(&session_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    if session.user_id != user.user_id {
        return Err(StatusCode::FORBIDDEN);
    }

    if let Some(title) = &body.title {
        service
            .rename_session(&session_id, title)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    }

    if let Some(pinned) = body.pinned {
        service
            .pin_session(&session_id, pinned)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    }

    Ok(StatusCode::OK)
}

/// POST /chat/sessions/{id}/messages - Send a message (triggers execution)
async fn send_message(
    State((service, db, chat_manager, _, ref workspace_root)): State<ChatState>,
    Extension(user): Extension<UserContext>,
    Path(session_id): Path<String>,
    Json(req): Json<SendChatMessageRequest>,
) -> Result<Json<SendMessageResponse>, StatusCode> {
    // M7: Validate content is not empty or too long
    let content = req.content.trim().to_string();
    if content.is_empty() || content.len() > 100_000 {
        return Err(StatusCode::BAD_REQUEST);
    }

    // Verify session exists and user owns it
    let session = service
        .get_session(&session_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    if session.user_id != user.user_id {
        return Err(StatusCode::FORBIDDEN);
    }

    if session
        .session_source
        .eq_ignore_ascii_case("portal_manager")
    {
        if let Some(turn_notice) = build_portal_manager_turn_notice(
            db.as_ref(),
            &session.team_id,
            session.portal_id.as_deref(),
            &content,
        )
        .await
        {
            if let Err(err) = service
                .append_hidden_session_notice(&session_id, &turn_notice)
                .await
            {
                tracing::warn!(
                    session_id = %session_id,
                    "Failed to append portal_manager turn notice: {}",
                    err
                );
            }
        }
    } else if let Some(turn_notice) = build_general_turn_notice(
        service.as_ref(),
        &session.team_id,
        &session.agent_id,
        &content,
    )
    .await
    {
        if let Err(err) = service
            .append_hidden_session_notice(&session_id, &turn_notice)
            .await
        {
            tracing::warn!(
                session_id = %session_id,
                "Failed to append general MCP turn notice: {}",
                err
            );
        }
    }

    // Register in ChatManager first (authoritative in-memory gate)
    let (cancel_token, _stream_tx) = match chat_manager.register(&session_id).await {
        Some(pair) => pair,
        None => return Err(StatusCode::CONFLICT),
    };

    // Then set MongoDB is_processing flag (secondary persistence)
    let claimed = service
        .try_start_processing(&session_id, &user.user_id)
        .await
        .map_err(|e| {
            tracing::error!("try_start_processing DB error for {}: {}", session_id, e);
            StatusCode::INTERNAL_SERVER_ERROR
        });
    match claimed {
        Ok(true) => {}
        _ => {
            // Rollback ChatManager registration
            chat_manager.unregister(&session_id).await;
            return Err(claimed.err().unwrap_or(StatusCode::CONFLICT));
        }
    }

    // Spawn background execution
    let executor = ChatExecutor::new(db.clone(), chat_manager.clone(), workspace_root.clone());
    let sid = session_id.clone();
    let agent_id = session.agent_id.clone();

    tokio::spawn(async move {
        if let Err(e) = executor
            .execute_chat(&sid, &agent_id, &content, cancel_token)
            .await
        {
            tracing::error!("Chat execution failed for session {}: {}", sid, e);
        }
    });

    Ok(Json(SendMessageResponse {
        session_id,
        streaming: true,
    }))
}

/// GET /chat/sessions/{id}/stream - SSE stream for chat events
async fn stream_chat(
    State((service, _, chat_manager, _, _)): State<ChatState>,
    Extension(user): Extension<UserContext>,
    headers: HeaderMap,
    Path(session_id): Path<String>,
    Query(q): Query<StreamQuery>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, StatusCode> {
    // Verify ownership
    let session = service
        .get_session(&session_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    if session.user_id != user.user_id {
        return Err(StatusCode::FORBIDDEN);
    }

    let last_event_id = q.last_event_id.or_else(|| {
        headers
            .get("last-event-id")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.parse::<u64>().ok())
    });

    // Subscribe to chat events with buffered history for reconnect/late join.
    let (mut rx, history) = chat_manager
        .subscribe_with_history(&session_id, last_event_id)
        .await
        .ok_or(StatusCode::NOT_FOUND)?;

    let stream = async_stream::stream! {
        yield Ok(Event::default()
            .event("status")
            .data(serde_json::json!({
                "type": "Status",
                "status": "running"
            }).to_string()));

        for event in history {
            let is_done = event.event.is_done();
            let json = serde_json::to_string(&event.event).unwrap_or_default();
            let mut sse = Event::default().event(event.event.event_type()).data(json);
            if event.id > 0 {
                sse = sse.id(event.id.to_string());
            }
            yield Ok(sse);
            if is_done {
                return;
            }
        }

        let sse_lifetime_secs = std::env::var("TEAM_SSE_MAX_LIFETIME_SECS")
            .ok().and_then(|v| v.parse::<u64>().ok()).unwrap_or(2 * 60 * 60);
        let deadline = tokio::time::Instant::now() + Duration::from_secs(sse_lifetime_secs);

        loop {
            match tokio::time::timeout_at(deadline, rx.recv()).await {
                Ok(Ok(event)) => {
                    let is_done = event.event.is_done();
                    let json = serde_json::to_string(&event.event).unwrap_or_default();
                    let mut sse = Event::default().event(event.event.event_type()).data(json);
                    if event.id > 0 {
                        sse = sse.id(event.id.to_string());
                    }
                    yield Ok(sse);
                    if is_done {
                        break;
                    }
                }
                Ok(Err(tokio::sync::broadcast::error::RecvError::Closed)) => break,
                Ok(Err(tokio::sync::broadcast::error::RecvError::Lagged(_))) => continue,
                Err(_) => {
                    tracing::info!("SSE stream deadline reached, closing for client reconnect");
                    break;
                }
            }
        }
    };

    Ok(Sse::new(stream).keep_alive(
        axum::response::sse::KeepAlive::new()
            .interval(std::time::Duration::from_secs(15))
            .text("ping"),
    ))
}

fn fix_bson_dates(val: &mut serde_json::Value) {
    match val {
        serde_json::Value::Object(map) => {
            if map.len() == 1 && map.contains_key("$date") {
                if let Some(date_val) = map.get("$date") {
                    if let Some(date_obj) = date_val.as_object() {
                        if let Some(ms) = date_obj.get("$numberLong").and_then(|v| v.as_str()) {
                            if let Ok(ts) = ms.parse::<i64>() {
                                if let Some(dt) =
                                    chrono::DateTime::<chrono::Utc>::from_timestamp_millis(ts)
                                {
                                    *val = serde_json::Value::String(dt.to_rfc3339());
                                    return;
                                }
                            }
                        }
                    }
                    if let Some(s) = date_val.as_str() {
                        *val = serde_json::Value::String(s.to_string());
                        return;
                    }
                }
            }
            for v in map.values_mut() {
                fix_bson_dates(v);
            }
        }
        serde_json::Value::Array(arr) => {
            for v in arr.iter_mut() {
                fix_bson_dates(v);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::{
        manager_message_mentions_extension_inventory, manager_message_mentions_registry,
        manager_message_mentions_skill_inventory, message_mentions_mcp_install,
    };

    #[test]
    fn manager_skill_inventory_detection_matches_cn_and_en_queries() {
        assert!(manager_message_mentions_skill_inventory(
            "你找一下你目前能用的skills"
        ));
        assert!(manager_message_mentions_skill_inventory(
            "what skills are currently available"
        ));
        assert!(manager_message_mentions_skill_inventory("有哪些技能"));
        assert!(!manager_message_mentions_skill_inventory("帮我打开访客页"));
    }

    #[test]
    fn manager_registry_detection_matches_registry_queries() {
        assert!(manager_message_mentions_registry(
            "帮我看看 skills.sh 热门技能"
        ));
        assert!(manager_message_mentions_registry(
            "search registry trending skills"
        ));
        assert!(manager_message_mentions_registry("热门技能有哪些"));
        assert!(!manager_message_mentions_registry("查看当前分身权限"));
    }

    #[test]
    fn manager_extension_inventory_detection_matches_extension_queries() {
        assert!(manager_message_mentions_extension_inventory(
            "列出当前可用的扩展和MCP"
        ));
        assert!(manager_message_mentions_extension_inventory(
            "what extensions are currently enabled"
        ));
        assert!(manager_message_mentions_extension_inventory("有哪些扩展"));
        assert!(!manager_message_mentions_extension_inventory(
            "帮我列出文档"
        ));
    }

    #[test]
    fn mcp_install_detection_matches_cn_and_en_queries() {
        assert!(message_mentions_mcp_install("安装一个新的 MCP"));
        assert!(message_mentions_mcp_install("install mcp server"));
        assert!(message_mentions_mcp_install("给这个分身加一个自定义扩展"));
        assert!(message_mentions_mcp_install("卸载这个 MCP"));
        assert!(message_mentions_mcp_install(
            "remove extension from this agent"
        ));
        assert!(message_mentions_mcp_install("删掉这个扩展"));
        assert!(!message_mentions_mcp_install("帮我列出文档"));
    }
}

/// GET /chat/sessions/{id}/events - List persisted runtime events.
async fn list_session_events(
    State((service, _, chat_manager, _, _)): State<ChatState>,
    Extension(user): Extension<UserContext>,
    Path(session_id): Path<String>,
    Query(q): Query<EventListQuery>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let session = service
        .get_session(&session_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    if session.user_id != user.user_id {
        let is_admin = service
            .is_team_admin(&user.user_id, &session.team_id)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        if !is_admin {
            return Err(StatusCode::FORBIDDEN);
        }
    }

    let limit = q.limit.unwrap_or(500).clamp(1, 2000);
    let descending = q
        .order
        .as_deref()
        .map(str::trim)
        .map(|v| v.eq_ignore_ascii_case("desc"))
        .unwrap_or(false);
    let explicit_run_id = q.run_id.as_deref().map(str::trim).filter(|s| !s.is_empty());
    let selected_run_id: Option<String> = match explicit_run_id {
        Some(rid)
            if rid.eq_ignore_ascii_case("__all__")
                || rid.eq_ignore_ascii_case("all")
                || rid == "*" =>
        {
            None
        }
        Some(rid) => Some(rid.to_string()),
        None => chat_manager.active_run_id(&session_id).await,
    };

    let events = service
        .list_chat_events(
            &session_id,
            selected_run_id.as_deref(),
            q.after_event_id,
            q.before_event_id,
            limit,
            descending,
        )
        .await
        .map_err(|e| {
            tracing::error!("Failed to list chat events for {}: {:?}", session_id, e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    let mut value = serde_json::to_value(events).unwrap_or_default();
    fix_bson_dates(&mut value);
    Ok(Json(value))
}

/// POST /chat/sessions/{id}/cancel - Cancel active chat
async fn cancel_chat(
    State((service, _, chat_manager, _, _)): State<ChatState>,
    Extension(user): Extension<UserContext>,
    Path(session_id): Path<String>,
) -> Result<StatusCode, StatusCode> {
    let session = service
        .get_session(&session_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    if session.user_id != user.user_id {
        return Err(StatusCode::FORBIDDEN);
    }

    let cancelled = chat_manager.cancel(&session_id).await;
    if cancelled {
        let _ = service.set_session_processing(&session_id, false).await;
        Ok(StatusCode::OK)
    } else {
        Err(StatusCode::NOT_FOUND)
    }
}

/// POST /chat/sessions/{id}/archive - Archive session
async fn archive_session(
    State((service, _, _, _, _)): State<ChatState>,
    Extension(user): Extension<UserContext>,
    Path(session_id): Path<String>,
) -> Result<StatusCode, StatusCode> {
    let session = service
        .get_session(&session_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    if session.user_id != user.user_id {
        return Err(StatusCode::FORBIDDEN);
    }

    // M8: Atomic archive — only succeeds if session is not processing
    let archived = service
        .archive_session_if_idle(&session_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    if archived {
        Ok(StatusCode::OK)
    } else {
        Err(StatusCode::CONFLICT)
    }
}

/// DELETE /chat/sessions/{id} - Permanently delete session
async fn delete_session(
    State((service, _, _, _, _)): State<ChatState>,
    Extension(user): Extension<UserContext>,
    Path(session_id): Path<String>,
) -> Result<StatusCode, StatusCode> {
    let session = service
        .get_session(&session_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    if session.user_id != user.user_id {
        return Err(StatusCode::FORBIDDEN);
    }

    // M8: Atomic delete — only succeeds if session is not processing
    let deleted = service
        .delete_session_if_idle(&session_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    if deleted {
        // P2: Best-effort workspace cleanup (after DB delete to avoid orphaned records)
        if let Err(e) = super::runtime::cleanup_workspace_dir(session.workspace_path.as_deref()) {
            tracing::warn!(
                "Failed to cleanup workspace for session {}: {}",
                session_id,
                e
            );
        }
        Ok(StatusCode::NO_CONTENT)
    } else {
        // Session was verified above but disappeared before delete — concurrent deletion
        Err(StatusCode::CONFLICT)
    }
}

// ── Phase 2: Document attachment routes ──

#[derive(serde::Deserialize)]
struct DocumentIdsBody {
    document_ids: Vec<String>,
}

/// POST /chat/sessions/{id}/documents - Attach documents
async fn attach_documents(
    State((service, _, _, _, _)): State<ChatState>,
    Extension(user): Extension<UserContext>,
    Path(session_id): Path<String>,
    Json(body): Json<DocumentIdsBody>,
) -> Result<StatusCode, StatusCode> {
    let session = service
        .get_session(&session_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    if session.user_id != user.user_id {
        return Err(StatusCode::FORBIDDEN);
    }

    service
        .attach_documents_to_session(&session_id, &body.document_ids)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(StatusCode::OK)
}

/// DELETE /chat/sessions/{id}/documents - Detach documents
async fn detach_documents(
    State((service, _, _, _, _)): State<ChatState>,
    Extension(user): Extension<UserContext>,
    Path(session_id): Path<String>,
    Json(body): Json<DocumentIdsBody>,
) -> Result<StatusCode, StatusCode> {
    let session = service
        .get_session(&session_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    if session.user_id != user.user_id {
        return Err(StatusCode::FORBIDDEN);
    }

    service
        .detach_documents_from_session(&session_id, &body.document_ids)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(StatusCode::NO_CONTENT)
}

/// GET /chat/sessions/{id}/documents - List attached documents
async fn list_attached_documents(
    State((service, _, _, _, _)): State<ChatState>,
    Extension(user): Extension<UserContext>,
    Path(session_id): Path<String>,
) -> Result<Json<Vec<String>>, StatusCode> {
    let session = service
        .get_session(&session_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    if session.user_id != user.user_id {
        return Err(StatusCode::FORBIDDEN);
    }

    Ok(Json(session.attached_document_ids))
}
