use crate::{
    agents::{
        subagent_task_config::TaskConfig, task_board::TaskBoardStore, task_board::TaskBoardUpdate,
        AgentEvent, SessionConfig,
    },
    conversation::{
        message::{ActionRequiredData, Message, MessageContent},
        Conversation,
    },
    execution::manager::AgentManager,
    permission::PermissionConfirmation,
    prompt_template::render_global_file,
    recipe::Recipe,
    session::{ExtensionState, SessionManager, SessionType, TaskStatus},
    utils::normalize_delegation_summary_text,
};
use anyhow::{anyhow, Result};
use futures::StreamExt;
use serde::Serialize;
use std::future::Future;
use std::pin::Pin;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info};

use crate::agents::harness::coordinator::CoordinatorRole;
use crate::agents::harness::permission_bridge::{
    await_permission_resolution, permission_bridge_timeout, permission_decision_source_label,
    timeout_resolution, to_permission_confirmation, write_permission_request,
    PermissionBridgeRequest,
};
use crate::agents::harness::{
    annotate_task_ledger_child_evidence_view, annotate_task_ledger_child_session,
    annotate_task_ledger_child_transcript_resume, build_child_transcript_excerpt,
    build_child_transcript_resume_lines, save_worker_runtime_state, HarnessWorkerRuntimeState,
    TaskRuntimeHost, WorkerAttemptIdentity,
};

#[derive(Serialize)]
struct SubagentPromptContext {
    max_turns: usize,
    subagent_id: String,
    task_instructions: String,
    tool_count: usize,
    available_tools: String,
}

#[derive(Serialize)]
struct WorkerPromptContext {
    worker_name: String,
    target_artifact: String,
    result_contract: String,
    write_scope: String,
    scratchpad_root: String,
    mailbox_root: String,
    runtime_tool_surface: String,
    hidden_coordinator_tools: String,
    permission_policy: String,
    peer_messaging_policy: String,
    available_peer_workers: String,
}

type AgentMessagesFuture =
    Pin<Box<dyn Future<Output = Result<(Conversation, Option<String>)>> + Send>>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompletedSubagentTaskResult {
    pub summary: String,
    pub recovered_terminal_summary: bool,
}

fn extract_message_output_text(message: &Message) -> Option<String> {
    let assistant_text = if message.role == rmcp::model::Role::Assistant {
        message.content.iter().find_map(|content| match content {
            crate::conversation::message::MessageContent::Text(text_content) => {
                Some(text_content.text.clone())
            }
            _ => None,
        })
    } else {
        None
    };

    assistant_text.or_else(|| {
        message.content.iter().find_map(|content| match content {
            crate::conversation::message::MessageContent::ToolResponse(tool_response) => {
                tool_response.tool_result.as_ref().ok().and_then(|result| {
                    let texts = result
                        .content
                        .iter()
                        .filter_map(|content| {
                            if let rmcp::model::RawContent::Text(raw_text_content) = &content.raw {
                                Some(raw_text_content.text.clone())
                            } else {
                                None
                            }
                        })
                        .collect::<Vec<_>>();
                    (!texts.is_empty()).then(|| texts.join("\n"))
                })
            }
            _ => None,
        })
    })
}

fn extract_message_runtime_trace_text(message: &Message) -> Option<String> {
    for content in &message.content {
        if let Some(text) = content.as_text() {
            let trimmed = text.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
        if let Some(text) = content.as_tool_response_text() {
            let trimmed = text.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
        match content {
            MessageContent::SystemNotification(notification) => {
                let trimmed = notification.msg.trim();
                if !trimmed.is_empty() {
                    return Some(trimmed.to_string());
                }
            }
            MessageContent::ActionRequired(action) => {
                let description = match &action.data {
                    ActionRequiredData::ToolConfirmation {
                        tool_name, prompt, ..
                    } => prompt
                        .as_deref()
                        .filter(|value| !value.trim().is_empty())
                        .map(ToString::to_string)
                        .unwrap_or_else(|| format!("permission requested for {}", tool_name)),
                    ActionRequiredData::Elicitation { message, .. } => message.clone(),
                    ActionRequiredData::ElicitationResponse { id, .. } => {
                        format!("elicitation response recorded for {}", id)
                    }
                };
                let trimmed = description.trim();
                if !trimmed.is_empty() {
                    return Some(trimmed.to_string());
                }
            }
            MessageContent::ToolRequest(request) => {
                if let Ok(tool_call) = &request.tool_call {
                    return Some(format!("called tool {}", tool_call.name));
                }
            }
            _ => {}
        }
    }
    None
}

fn synthesize_subagent_fallback_summary(messages: &Conversation) -> Option<String> {
    let assistant_trace = messages
        .messages()
        .iter()
        .rev()
        .filter(|message| message.role == rmcp::model::Role::Assistant)
        .find_map(extract_message_runtime_trace_text);
    if let Some(trace) = assistant_trace {
        return Some(format!(
            "Scope: recovered bounded worker trace\nResult: {}\nBlocker: worker finished without an explicit terminal summary",
            normalize_delegation_summary_text(&trace)
        ));
    }

    let excerpt = build_child_transcript_excerpt(messages);
    if !excerpt.is_empty() {
        return Some(format!(
            "Scope: recovered bounded worker transcript\nResult: {}\nBlocker: worker finished without an explicit terminal summary",
            normalize_delegation_summary_text(&excerpt.join(" | "))
        ));
    }

    (!messages.messages().is_empty()).then(|| {
        format!(
            "Scope: bounded worker session\nResult: worker session produced {} message(s)\nBlocker: worker finished without an explicit terminal summary",
            messages.messages().len()
        )
    })
}

async fn persist_child_replay_evidence(
    parent_session_id: &str,
    task_id: &str,
    conversation: Option<&Conversation>,
    preview_fallback: Option<&str>,
    child_updated_at: Option<i64>,
) {
    let excerpt = conversation
        .map(build_child_transcript_excerpt)
        .unwrap_or_default();
    let resume_lines = conversation
        .map(build_child_transcript_resume_lines)
        .unwrap_or_default();
    let _ = annotate_task_ledger_child_evidence_view(
        parent_session_id,
        task_id,
        preview_fallback.filter(|value| !value.trim().is_empty()),
        &excerpt,
    )
    .await;
    let _ = annotate_task_ledger_child_transcript_resume(
        parent_session_id,
        task_id,
        &resume_lines,
        resume_lines.len(),
        child_updated_at,
    )
    .await;
}

async fn persist_child_replay_evidence_from_session(
    parent_session_id: &str,
    task_id: &str,
    child_session_id: &str,
    preview_fallback: Option<&str>,
) {
    let session = SessionManager::get_session(child_session_id, true)
        .await
        .ok();
    let child_updated_at = session
        .as_ref()
        .map(|session| session.updated_at.timestamp());
    let conversation = session.and_then(|session| session.conversation);
    persist_child_replay_evidence(
        parent_session_id,
        task_id,
        conversation.as_ref(),
        preview_fallback,
        child_updated_at,
    )
    .await;
}

async fn maybe_handle_worker_permission_bridge(
    agent: &crate::agents::Agent,
    task_config: &TaskConfig,
    content: &MessageContent,
) -> Result<bool> {
    if !task_config.enable_permission_bridge
        || task_config.coordinator_role != CoordinatorRole::Worker
    {
        return Ok(false);
    }

    let Some(run_id) = task_config.swarm_run_id.as_deref() else {
        return Ok(false);
    };
    let Some(worker_name) = task_config.worker_name.as_deref() else {
        return Ok(false);
    };
    let MessageContent::ActionRequired(action_required) = content else {
        return Ok(false);
    };
    let ActionRequiredData::ToolConfirmation {
        id,
        tool_name,
        arguments,
        prompt,
    } = &action_required.data
    else {
        return Ok(false);
    };

    let request = PermissionBridgeRequest {
        request_id: id.clone(),
        run_id: run_id.to_string(),
        worker_name: worker_name.to_string(),
        logical_worker_id: None,
        attempt_id: None,
        attempt_index: None,
        previous_task_id: None,
        tool_name: tool_name.clone(),
        tool_use_id: id.clone(),
        description: prompt.clone().unwrap_or_else(|| tool_name.clone()),
        arguments: arguments.clone(),
        tool_input_snapshot: Some(arguments.clone()),
        write_scope: task_config.write_scope.clone(),
        validation_mode: task_config.validation_mode,
        permission_policy: task_config
            .worker_capability_context()
            .map(|context| context.permission_policy),
        created_at: chrono::Utc::now().to_rfc3339(),
    };
    let mut request = request;
    if let (Some(runtime), Some(task_id)) =
        (&task_config.task_runtime, &task_config.current_task_id)
    {
        if let Some(snapshot) = runtime.snapshot(task_id).await {
            if let Some(identity) = WorkerAttemptIdentity::from_metadata(&snapshot.metadata) {
                request.logical_worker_id = Some(identity.logical_worker_id);
                request.attempt_id = Some(identity.attempt_id);
                request.attempt_index = Some(identity.attempt_index);
                request.previous_task_id = identity.previous_task_id;
            }
        }
        let _ = runtime
            .record_idle(
                task_id,
                format!("waiting for leader permission on {}", tool_name),
            )
            .await;
        let _ = runtime
            .record_permission_requested(task_id, Some(worker_name.to_string()), tool_name.clone())
            .await;
    }
    write_permission_request(&task_config.parent_working_dir, &request)?;
    let timeout = permission_bridge_timeout();
    let resolution =
        match await_permission_resolution(&task_config.parent_working_dir, run_id, id, timeout)
            .await
        {
            Ok(resolution) => resolution,
            Err(_) => {
                if let (Some(runtime), Some(task_id)) =
                    (&task_config.task_runtime, &task_config.current_task_id)
                {
                    let _ = runtime
                        .record_permission_timed_out(
                            task_id,
                            Some(worker_name.to_string()),
                            tool_name.clone(),
                            timeout.as_millis().min(u64::MAX as u128) as u64,
                        )
                        .await;
                }
                timeout_resolution(&request, timeout)
            }
        };
    if let (Some(runtime), Some(task_id)) =
        (&task_config.task_runtime, &task_config.current_task_id)
    {
        let _ = runtime
            .record_permission_resolved(
                task_id,
                Some(worker_name.to_string()),
                tool_name.clone(),
                format!("{:?}", resolution.permission),
                Some(permission_decision_source_label(&resolution.source).to_string()),
            )
            .await;
    }
    let confirmation: PermissionConfirmation = to_permission_confirmation(&resolution);
    agent.handle_confirmation(id.clone(), confirmation).await;
    Ok(true)
}

fn render_worker_prompt_addendum(task_config: &TaskConfig) -> Option<String> {
    if task_config.coordinator_role != CoordinatorRole::Worker {
        return None;
    }

    let capability_context = task_config.worker_capability_context().unwrap_or_default();
    let worker_name = task_config
        .worker_name
        .clone()
        .unwrap_or_else(|| "worker".to_string());
    let target_artifact = task_config
        .target_artifacts
        .first()
        .cloned()
        .or_else(|| task_config.result_contract.first().cloned())
        .unwrap_or_else(|| "bounded target".to_string());
    let prompt_context = WorkerPromptContext {
        worker_name,
        target_artifact,
        result_contract: if task_config.result_contract.is_empty() {
            "none".to_string()
        } else {
            task_config.result_contract.join(", ")
        },
        write_scope: if task_config.write_scope.is_empty() {
            "none".to_string()
        } else {
            task_config.write_scope.join(", ")
        },
        scratchpad_root: task_config
            .scratchpad_dir
            .as_ref()
            .map(|value| value.display().to_string())
            .unwrap_or_else(|| "disabled".to_string()),
        mailbox_root: task_config
            .mailbox_dir
            .as_ref()
            .map(|value| value.display().to_string())
            .unwrap_or_else(|| "disabled".to_string()),
        runtime_tool_surface: capability_context.runtime_tool_surface_text(),
        hidden_coordinator_tools: capability_context.hidden_tools_text(),
        permission_policy: capability_context.permission_policy,
        peer_messaging_policy: if capability_context.allow_worker_messaging {
            "Peer worker messaging is enabled for this swarm run. Use `send_message` only for bounded coordination with the listed recipients or the leader.".to_string()
        } else {
            "Peer worker messaging is disabled for this runtime. Coordinate only through your final summary or leader follow-up.".to_string()
        },
        available_peer_workers: if capability_context.peer_worker_catalog.is_empty() {
            "none".to_string()
        } else {
            capability_context.peer_worker_catalog.join(", ")
        },
    };
    let mut prompt = render_global_file(
        if task_config.validation_mode {
            "validation_worker_system.md"
        } else {
            "worker_system.md"
        },
        &prompt_context,
    )
    .ok()?;

    if task_config.validation_mode {
        if let Ok(verification_hint) = render_global_file("verification_worker.md", &prompt_context)
        {
            prompt.push_str("\n\n");
            prompt.push_str(&verification_hint);
        }
    }

    if task_config.enable_permission_bridge {
        if let Ok(permission_hint) =
            render_global_file("permission_bridge_explainer.md", &serde_json::json!({}))
        {
            return Some(format!("{}\n\n{}", prompt, permission_hint));
        }
    }

    Some(prompt)
}

async fn settle_worker_task_board_after_success(
    task_config: &TaskConfig,
    session_id: &str,
) -> Result<()> {
    if task_config.coordinator_role != CoordinatorRole::Worker {
        return Ok(());
    }

    let session = SessionManager::get_session(session_id, false).await?;
    let runtime = HarnessWorkerRuntimeState::from_extension_data(&session.extension_data);
    let context = crate::agents::task_board::TaskBoardContext::from_runtime(
        session_id.to_string(),
        runtime.as_ref(),
    );
    if context.scope != crate::agents::task_board::TaskBoardScope::Worker {
        return Ok(());
    }

    let snapshot = TaskBoardStore::list(&context)
        .await
        .map_err(|error| anyhow!("Failed to load worker task board: {}", error))?;

    let open_task_ids = snapshot
        .items
        .iter()
        .filter(|item| item.status != TaskStatus::Completed)
        .map(|item| item.id.clone())
        .collect::<Vec<_>>();

    for task_id in open_task_ids {
        let mut metadata = std::collections::HashMap::new();
        metadata.insert(
            "runtime_auto_completed".to_string(),
            serde_json::Value::Bool(true),
        );
        metadata.insert(
            "runtime_settled_by".to_string(),
            serde_json::Value::String("bounded_worker_completion".to_string()),
        );
        TaskBoardStore::update_task(
            &context,
            TaskBoardUpdate {
                task_id,
                status: Some(TaskStatus::Completed),
                metadata: Some(metadata),
                ..TaskBoardUpdate::default()
            },
        )
        .await
        .map_err(|error| anyhow!("Failed to settle worker task board: {}", error))?;
    }

    Ok(())
}

/// Standalone function to run a complete subagent task with output options
pub async fn run_complete_subagent_task(
    recipe: Recipe,
    task_config: TaskConfig,
    return_last_only: bool,
    session_id: String,
    cancellation_token: Option<CancellationToken>,
) -> Result<CompletedSubagentTaskResult, anyhow::Error> {
    let settlement_task_config = task_config.clone();
    let settlement_session_id = session_id.clone();
    let (messages, final_output) =
        match get_agent_messages(recipe, task_config, session_id, cancellation_token).await {
            Ok(value) => value,
            Err(e) => {
                if let Some(task_id) = settlement_task_config.current_task_id.as_deref() {
                    persist_child_replay_evidence_from_session(
                        &settlement_task_config.parent_session_id,
                        task_id,
                        &settlement_session_id,
                        Some(&format!("Failed to execute task: {}", e)),
                    )
                    .await;
                }
                return Err(anyhow!("Failed to execute task: {}", e));
            }
        };

    if let Some(output) = final_output {
        if let Some(task_id) = settlement_task_config.current_task_id.as_deref() {
            persist_child_replay_evidence(
                &settlement_task_config.parent_session_id,
                task_id,
                Some(&messages),
                Some(&output),
                None,
            )
            .await;
        }
        settle_worker_task_board_after_success(&settlement_task_config, &settlement_session_id)
            .await?;
        return Ok(CompletedSubagentTaskResult {
            summary: output,
            recovered_terminal_summary: false,
        });
    }

    let (response_text, recovered_terminal_summary) = if return_last_only {
        let latest_message_text = messages
            .messages()
            .last()
            .and_then(extract_message_output_text);

        if let Some(text) = latest_message_text.or_else(|| {
            messages
                .messages()
                .iter()
                .rev()
                .find_map(|message| extract_message_output_text(message))
        }) {
            (text, false)
        } else if let Some(text) = synthesize_subagent_fallback_summary(&messages) {
            (text, true)
        } else {
            return Err(anyhow!(
                "Subagent completed without recoverable terminal output"
            ));
        }
    } else {
        let all_text_content: Vec<String> = messages
            .iter()
            .flat_map(|message| {
                message.content.iter().filter_map(|content| {
                    match content {
                        crate::conversation::message::MessageContent::Text(text_content) => {
                            Some(text_content.text.clone())
                        }
                        crate::conversation::message::MessageContent::ToolResponse(
                            tool_response,
                        ) => {
                            // Extract text from tool response
                            if let Ok(result) = &tool_response.tool_result {
                                let texts: Vec<String> = result
                                    .content
                                    .iter()
                                    .filter_map(|content| {
                                        if let rmcp::model::RawContent::Text(raw_text_content) =
                                            &content.raw
                                        {
                                            Some(raw_text_content.text.clone())
                                        } else {
                                            None
                                        }
                                    })
                                    .collect();
                                if !texts.is_empty() {
                                    Some(format!("Tool result: {}", texts.join("\n")))
                                } else {
                                    None
                                }
                            } else {
                                None
                            }
                        }
                        _ => None,
                    }
                })
            })
            .collect();

        if all_text_content.is_empty() {
            let text = synthesize_subagent_fallback_summary(&messages).ok_or_else(|| {
                anyhow!(
                    "Subagent completed without any recoverable assistant or tool-result content"
                )
            })?;
            (text, true)
        } else {
            (all_text_content.join("\n"), false)
        }
    };

    if let Some(task_id) = settlement_task_config.current_task_id.as_deref() {
        persist_child_replay_evidence(
            &settlement_task_config.parent_session_id,
            task_id,
            Some(&messages),
            Some(&response_text),
            None,
        )
        .await;
    }
    settle_worker_task_board_after_success(&settlement_task_config, &settlement_session_id).await?;
    Ok(CompletedSubagentTaskResult {
        summary: response_text,
        recovered_terminal_summary,
    })
}

fn get_agent_messages(
    recipe: Recipe,
    task_config: TaskConfig,
    session_id: String,
    cancellation_token: Option<CancellationToken>,
) -> AgentMessagesFuture {
    Box::pin(async move {
        let text_instruction = recipe
            .instructions
            .clone()
            .or(recipe.prompt.clone())
            .ok_or_else(|| anyhow!("Recipe has no instructions or prompt"))?;

        if SessionManager::get_session(&session_id, false)
            .await
            .is_err()
        {
            if let Err(create_error) = SessionManager::create_session_with_id(
                session_id.clone(),
                task_config.parent_working_dir.clone(),
                "Bounded worker".to_string(),
                SessionType::SubAgent,
            )
            .await
            {
                SessionManager::get_session(&session_id, false)
                    .await
                    .map_err(|_| {
                        anyhow!(
                            "Failed to ensure bounded worker session {}: {}",
                            session_id,
                            create_error
                        )
                    })?;
            }
        }
        let worker_attempt_identity = if let (Some(runtime), Some(task_id)) =
            (&task_config.task_runtime, &task_config.current_task_id)
        {
            runtime
                .snapshot(task_id)
                .await
                .and_then(|snapshot| WorkerAttemptIdentity::from_metadata(&snapshot.metadata))
        } else {
            None
        };
        let _ = save_worker_runtime_state(
            &session_id,
            &HarnessWorkerRuntimeState {
                task_board_session_id: task_config
                    .task_board_session_id
                    .clone()
                    .or_else(|| Some(task_config.parent_session_id.clone())),
                swarm_run_id: task_config.swarm_run_id.clone(),
                worker_name: task_config.worker_name.clone(),
                leader_name: task_config.leader_name.clone(),
                logical_worker_id: worker_attempt_identity
                    .as_ref()
                    .map(|identity| identity.logical_worker_id.clone()),
                attempt_id: worker_attempt_identity
                    .as_ref()
                    .map(|identity| identity.attempt_id.clone()),
                attempt_index: worker_attempt_identity
                    .as_ref()
                    .map(|identity| identity.attempt_index),
                previous_task_id: worker_attempt_identity
                    .as_ref()
                    .and_then(|identity| identity.previous_task_id.clone()),
                coordinator_role: task_config.coordinator_role,
                mailbox_dir: task_config.mailbox_dir.clone(),
                permission_dir: task_config.permission_dir.clone(),
                scratchpad_dir: task_config.scratchpad_dir.clone(),
                enable_permission_bridge: task_config.enable_permission_bridge,
                allow_worker_messaging: task_config.allow_worker_messaging,
                peer_worker_addresses: task_config.peer_worker_addresses.clone(),
                peer_worker_catalog: task_config.peer_worker_catalog.clone(),
                validation_mode: task_config.validation_mode,
            },
        )
        .await;
        if let Some(task_id) = task_config.current_task_id.as_deref() {
            let _ = annotate_task_ledger_child_session(
                &task_config.parent_session_id,
                task_id,
                &session_id,
                "sub_agent",
            )
            .await;
        }

        let agent_manager = AgentManager::instance()
            .await
            .map_err(|e| anyhow!("Failed to create AgentManager: {}", e))?;

        let agent = agent_manager
            .get_or_create_agent(session_id.clone())
            .await
            .map_err(|e| anyhow!("Failed to get sub agent session file path: {}", e))?;
        agent
            .set_worker_capability_context(task_config.worker_capability_context())
            .await;

        agent
            .update_provider(task_config.provider.clone(), &session_id)
            .await
            .map_err(|e| anyhow!("Failed to set provider on sub agent: {}", e))?;

        for extension in task_config.extensions.clone() {
            if let Err(e) = agent.add_extension(extension.clone()).await {
                debug!(
                    "Failed to add extension '{}' to subagent: {}",
                    extension.name(),
                    e
                );
            }
        }

        let has_response_schema = recipe.response.is_some();
        agent
            .apply_recipe_components(recipe.sub_recipes.clone(), recipe.response.clone(), true)
            .await;

        let tools = agent.list_tools(None).await;
        let subagent_prompt = render_global_file(
            "subagent_system.md",
            &SubagentPromptContext {
                max_turns: task_config
                    .max_turns
                    .expect("TaskConfig always sets max_turns"),
                subagent_id: session_id.clone(),
                task_instructions: text_instruction.clone(),
                tool_count: tools.len(),
                available_tools: tools
                    .iter()
                    .map(|t| t.name.to_string())
                    .collect::<Vec<_>>()
                    .join(", "),
            },
        )
        .map_err(|e| anyhow!("Failed to render subagent system prompt: {}", e))?;
        let subagent_prompt =
            if let Some(worker_addendum) = render_worker_prompt_addendum(&task_config) {
                format!("{}\n\n{}", subagent_prompt, worker_addendum)
            } else {
                subagent_prompt
            };
        agent.override_system_prompt(subagent_prompt).await;

        let user_message = Message::user().with_text(text_instruction);
        let mut conversation = Conversation::new_unvalidated(vec![user_message.clone()]);

        if let Some(activities) = recipe.activities {
            for activity in activities {
                info!("Recipe activity: {}", activity);
            }
        }
        let session_config = SessionConfig {
            id: session_id.clone(),
            schedule_id: None,
            max_turns: task_config.max_turns.map(|v| v as u32),
            retry_config: recipe.retry,
        };

        let mut stream = crate::session_context::with_session_id(Some(session_id.clone()), async {
            agent
                .reply(user_message, session_config, cancellation_token)
                .await
        })
        .await
        .map_err(|e| anyhow!("Failed to get reply from agent: {}", e))?;
        while let Some(message_result) = stream.next().await {
            match message_result {
                Ok(AgentEvent::Message(msg)) => {
                    let mut swallowed = false;
                    for content in &msg.content {
                        if maybe_handle_worker_permission_bridge(&agent, &task_config, content)
                            .await?
                        {
                            swallowed = true;
                        }
                    }
                    if !swallowed {
                        conversation.push(msg);
                    }
                }
                Ok(AgentEvent::ToolTransportRequest(event)) => {
                    tracing::warn!(
                        "Unexpected transport-level tool request in subagent stream: {} ({:?}, {:?})",
                        event.request.id,
                        event.transport,
                        event.surface,
                    );
                }
                Ok(AgentEvent::McpNotification(_)) | Ok(AgentEvent::ModelChange { .. }) => {}
                Ok(AgentEvent::HistoryReplaced(updated_conversation)) => {
                    conversation = updated_conversation;
                }
                Err(e) => {
                    tracing::error!("Error receiving message from subagent: {}", e);
                    return Err(anyhow!("Error receiving message from subagent: {}", e));
                }
            }
        }

        let final_output = if has_response_schema {
            agent
                .final_output_tool
                .lock()
                .await
                .as_ref()
                .and_then(|tool| tool.final_output.clone())
        } else {
            None
        };

        Ok((conversation, final_output))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::harness::load_task_ledger_state;
    use crate::conversation::message::Message;
    use crate::model::ModelConfig;
    use crate::providers::base::{Provider, ProviderMetadata, ProviderUsage};
    use crate::providers::errors::ProviderError;
    use crate::session::{TaskItem, TaskScope, TasksStateV2};
    use rmcp::model::CallToolResult;
    use rmcp::model::Tool;
    use std::collections::HashMap;
    use std::path::Path;
    use std::sync::Arc;
    use uuid::Uuid;

    #[derive(Debug)]
    struct DummyProvider {
        model_config: ModelConfig,
    }

    #[async_trait::async_trait]
    impl Provider for DummyProvider {
        fn metadata() -> ProviderMetadata
        where
            Self: Sized,
        {
            ProviderMetadata::empty()
        }

        fn get_name(&self) -> &str {
            "dummy"
        }

        async fn complete_with_model(
            &self,
            _model_config: &ModelConfig,
            _system: &str,
            _messages: &[Message],
            _tools: &[Tool],
        ) -> Result<(Message, ProviderUsage), ProviderError> {
            panic!("DummyProvider should not be invoked in subagent handler tests")
        }

        fn get_model_config(&self) -> ModelConfig {
            self.model_config.clone()
        }
    }

    #[test]
    fn extract_message_output_text_ignores_plain_user_prompt_text() {
        let message = Message::user().with_text("user prompt");
        assert!(extract_message_output_text(&message).is_none());
    }

    #[test]
    fn extract_message_output_text_accepts_assistant_text() {
        let message = Message::assistant().with_text("assistant summary");
        assert_eq!(
            extract_message_output_text(&message).as_deref(),
            Some("assistant summary")
        );
    }

    #[test]
    fn extract_message_output_text_accepts_tool_response_text() {
        let message = Message::user().with_tool_response(
            "tool-1",
            Ok(CallToolResult {
                content: vec![rmcp::model::Content::text("tool output")],
                structured_content: None,
                is_error: Some(false),
                meta: None,
            }),
        );
        assert_eq!(
            extract_message_output_text(&message).as_deref(),
            Some("tool output")
        );
    }

    #[test]
    fn synthesize_subagent_fallback_summary_uses_runtime_trace_when_terminal_output_is_missing() {
        let conversation = Conversation::new_unvalidated(vec![
            Message::user().with_text("inspect the repo"),
            Message::assistant().with_system_notification(
                crate::conversation::message::SystemNotificationType::InlineMessage,
                "worker requested leader approval",
            ),
        ]);

        assert_eq!(
            synthesize_subagent_fallback_summary(&conversation).as_deref(),
            Some(
                "Scope: recovered bounded worker trace\nResult: worker requested leader approval\nBlocker: worker finished without an explicit terminal summary"
            )
        );
    }

    #[test]
    fn synthesize_subagent_fallback_summary_normalizes_noise_separator() {
        let conversation = Conversation::new_unvalidated(vec![
            Message::user().with_text("inspect the repo"),
            Message::assistant().with_system_notification(
                crate::conversation::message::SystemNotificationType::InlineMessage,
                "worker result бк file not found",
            ),
        ]);

        assert_eq!(
            synthesize_subagent_fallback_summary(&conversation).as_deref(),
            Some(
                "Scope: recovered bounded worker trace\nResult: worker result - file not found\nBlocker: worker finished without an explicit terminal summary"
            )
        );
    }

    #[tokio::test]
    async fn settle_worker_task_board_after_success_completes_open_tasks() {
        let worker_session_id = format!("worker-session-{}", Uuid::new_v4());
        let board_session_id = format!("leader-board:worker-a:{}", Uuid::new_v4());
        SessionManager::create_session_with_id(
            worker_session_id.clone(),
            std::env::current_dir().unwrap_or_else(|_| ".".into()),
            "worker-runtime".to_string(),
            SessionType::SubAgent,
        )
        .await
        .expect("create worker session");
        SessionManager::create_session_with_id(
            board_session_id.clone(),
            std::env::current_dir().unwrap_or_else(|_| ".".into()),
            "worker-board".to_string(),
            SessionType::Hidden,
        )
        .await
        .expect("create task board session");

        save_worker_runtime_state(
            &worker_session_id,
            &HarnessWorkerRuntimeState {
                task_board_session_id: Some("leader-board".to_string()),
                swarm_run_id: None,
                worker_name: Some("worker-a".to_string()),
                leader_name: Some("leader".to_string()),
                logical_worker_id: Some("worker-a".to_string()),
                attempt_id: Some("attempt-1".to_string()),
                attempt_index: Some(0),
                previous_task_id: None,
                coordinator_role: CoordinatorRole::Worker,
                mailbox_dir: None,
                permission_dir: None,
                scratchpad_dir: None,
                enable_permission_bridge: false,
                allow_worker_messaging: false,
                peer_worker_addresses: Vec::new(),
                peer_worker_catalog: Vec::new(),
                validation_mode: false,
            },
        )
        .await
        .expect("save runtime state");

        let mut state = TasksStateV2::new(TaskScope::Worker, board_session_id.clone());
        state.items.push(TaskItem {
            id: "1".to_string(),
            subject: "worker task".to_string(),
            description: "finish the bounded worker task".to_string(),
            active_form: "finishing worker task".to_string(),
            owner: Some("worker-a".to_string()),
            status: TaskStatus::InProgress,
            blocks: Vec::new(),
            blocked_by: Vec::new(),
            metadata: HashMap::new(),
        });
        SessionManager::set_extension_state(&board_session_id, &state)
            .await
            .expect("persist tasks");

        let task_config = TaskConfig::new(
            Arc::new(DummyProvider {
                model_config: ModelConfig::new_or_fail("gpt-4o"),
            }),
            "leader-board",
            Path::new("."),
            Vec::new(),
        )
        .with_worker_runtime(
            None,
            Some("worker-a".to_string()),
            Some("leader".to_string()),
            CoordinatorRole::Worker,
            None,
            None,
            None,
            false,
            false,
            Vec::new(),
            Vec::new(),
            false,
        )
        .with_task_board_session_id(Some("leader-board".to_string()));

        settle_worker_task_board_after_success(&task_config, &worker_session_id)
            .await
            .expect("settle tasks");

        let session = SessionManager::get_session(&board_session_id, false)
            .await
            .expect("load board");
        let restored =
            TasksStateV2::from_extension_data(&session.extension_data).expect("tasks should exist");
        assert_eq!(restored.items.len(), 1);
        assert_eq!(restored.items[0].status, TaskStatus::Completed);
        assert_eq!(
            restored.items[0]
                .metadata
                .get("runtime_auto_completed")
                .and_then(serde_json::Value::as_bool),
            Some(true)
        );
    }

    #[tokio::test]
    async fn persist_child_replay_evidence_from_session_uses_child_transcript_on_failure() {
        let parent_session_id = format!("parent-ledger-{}", Uuid::new_v4());
        let child_session_id = format!("child-ledger-{}", Uuid::new_v4());
        SessionManager::create_session_with_id(
            parent_session_id.clone(),
            std::env::current_dir().unwrap_or_else(|_| ".".into()),
            "parent".to_string(),
            SessionType::Hidden,
        )
        .await
        .expect("create parent");
        SessionManager::create_session_with_id(
            child_session_id.clone(),
            std::env::current_dir().unwrap_or_else(|_| ".".into()),
            "child".to_string(),
            SessionType::SubAgent,
        )
        .await
        .expect("create child");
        SessionManager::add_message(
            &child_session_id,
            &Message::assistant().with_text("child failure context"),
        )
        .await
        .expect("append child message");

        let snapshot = crate::agents::harness::TaskSnapshot {
            task_id: "task-1".to_string(),
            parent_session_id: parent_session_id.clone(),
            depth: 1,
            kind: crate::agents::harness::TaskKind::Subagent,
            status: crate::agents::harness::TaskStatus::Failed,
            description: Some("subagent".to_string()),
            write_scope: vec!["docs".to_string()],
            target_artifacts: vec!["docs/a.md".to_string()],
            result_contract: vec!["docs/a.md".to_string()],
            summary: Some("failed".to_string()),
            produced_delta: false,
            accepted_targets: Vec::new(),
            metadata: HashMap::new(),
            started_at: 1,
            updated_at: 1,
            finished_at: Some(2),
        };
        crate::agents::harness::upsert_task_ledger_snapshot(&parent_session_id, &snapshot)
            .await
            .expect("persist task snapshot");

        persist_child_replay_evidence_from_session(
            &parent_session_id,
            "task-1",
            &child_session_id,
            Some("Failed to execute task: boom"),
        )
        .await;

        let ledger = load_task_ledger_state(&parent_session_id)
            .await
            .expect("load ledger");
        let task = &ledger.tasks[0];
        assert_eq!(
            task.metadata
                .get("child_session_preview")
                .map(String::as_str),
            Some("Failed to execute task: boom")
        );
        let excerpt = task
            .metadata
            .get("child_session_excerpt")
            .cloned()
            .expect("excerpt persisted");
        assert!(excerpt.contains("child failure context"));
    }

    #[tokio::test]
    async fn persist_child_replay_evidence_from_session_falls_back_to_error_preview_without_transcript(
    ) {
        let parent_session_id = format!("parent-ledger-preview-{}", Uuid::new_v4());
        SessionManager::create_session_with_id(
            parent_session_id.clone(),
            std::env::current_dir().unwrap_or_else(|_| ".".into()),
            "parent".to_string(),
            SessionType::Hidden,
        )
        .await
        .expect("create parent");

        let snapshot = crate::agents::harness::TaskSnapshot {
            task_id: "task-1".to_string(),
            parent_session_id: parent_session_id.clone(),
            depth: 1,
            kind: crate::agents::harness::TaskKind::Subagent,
            status: crate::agents::harness::TaskStatus::Failed,
            description: Some("subagent".to_string()),
            write_scope: vec!["docs".to_string()],
            target_artifacts: vec!["docs/a.md".to_string()],
            result_contract: vec!["docs/a.md".to_string()],
            summary: Some("failed".to_string()),
            produced_delta: false,
            accepted_targets: Vec::new(),
            metadata: HashMap::new(),
            started_at: 1,
            updated_at: 1,
            finished_at: Some(2),
        };
        crate::agents::harness::upsert_task_ledger_snapshot(&parent_session_id, &snapshot)
            .await
            .expect("persist task snapshot");

        persist_child_replay_evidence_from_session(
            &parent_session_id,
            "task-1",
            "missing-child-session",
            Some("Failed to execute task: boom"),
        )
        .await;

        let ledger = load_task_ledger_state(&parent_session_id)
            .await
            .expect("load ledger");
        let task = &ledger.tasks[0];
        assert_eq!(
            task.metadata
                .get("child_session_preview")
                .map(String::as_str),
            Some("Failed to execute task: boom")
        );
    }
}
