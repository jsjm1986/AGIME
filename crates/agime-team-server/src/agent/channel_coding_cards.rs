use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use std::process::Command;

use agime::agents::harness::{render_child_evidence_line, PersistedChildEvidenceItem};
use serde_json::{Map, Value};

use super::channel_project_workspace::ChannelProjectWorkspaceService;
use super::chat_channels::{
    ChatChannelDoc, ChatChannelMessageResponse, ChatChannelThreadResponse, ChatChannelType,
};
use super::service_mongo::AgentService;

#[derive(Debug, Clone, Default)]
struct RuntimeDiagnosticsSnapshot {
    status: Option<String>,
    summary: Option<String>,
    validation_status: Option<String>,
    blocking_reason: Option<String>,
    next_steps: Vec<String>,
    child_evidence: Vec<String>,
    child_transcript_resume: Vec<String>,
}

pub fn is_coding_channel(channel: &ChatChannelDoc) -> bool {
    matches!(channel.channel_type, ChatChannelType::Coding)
        && channel
            .workspace_path
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty())
        && channel
            .repo_path
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty())
}

pub fn attach_card_domain_metadata(
    channel: &ChatChannelDoc,
    metadata: Value,
    coding_payload: Option<Value>,
) -> Value {
    let mut object = metadata.as_object().cloned().unwrap_or_default();
    object.insert(
        "card_domain".to_string(),
        Value::String(if is_coding_channel(channel) {
            "coding".to_string()
        } else {
            "general".to_string()
        }),
    );
    if let Some(payload) = coding_payload {
        object.insert("coding_payload".to_string(), payload);
    }
    Value::Object(object)
}

pub fn build_channel_coding_payload(
    channel: &ChatChannelDoc,
    next_action: Option<&str>,
    blockers: Vec<String>,
) -> Option<Value> {
    let mut payload = base_payload(channel)?;
    payload.insert("changed_files".to_string(), Value::Array(Vec::new()));
    payload.insert("commands_run".to_string(), Value::Array(Vec::new()));
    payload.insert("artifacts".to_string(), Value::Array(Vec::new()));
    payload.insert(
        "blockers".to_string(),
        Value::Array(blockers.into_iter().map(Value::String).collect()),
    );
    if let Some(next_action) = next_action.map(str::trim).filter(|value| !value.is_empty()) {
        payload.insert(
            "next_action".to_string(),
            Value::String(next_action.to_string()),
        );
    }
    Some(Value::Object(payload))
}

pub async fn build_thread_coding_payload(
    agent_service: &AgentService,
    channel: &ChatChannelDoc,
    thread_root_id: &str,
    thread: Option<&ChatChannelThreadResponse>,
) -> Option<Value> {
    let mut payload = base_payload(channel)?;
    let session = agent_service
        .find_latest_channel_session(&channel.channel_id, thread_root_id)
        .await
        .ok()
        .flatten();
    let repo_path = channel.repo_path.as_deref()?.trim().to_string();
    let thread_worktree_path = session
        .as_ref()
        .and_then(|item| item.workspace_path.clone())
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| {
            Path::new(&repo_path)
                .join("worktrees")
                .join(thread_root_id)
                .to_string_lossy()
                .to_string()
        });
    let thread_branch = session
        .as_ref()
        .and_then(|item| item.thread_branch.clone())
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| {
            ChannelProjectWorkspaceService::thread_branch_name(&channel.channel_id, thread_root_id)
        });
    let thread_repo_ref = session
        .as_ref()
        .and_then(|item| item.thread_repo_ref.clone())
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| {
            Path::new(&repo_path)
                .join("bare.git")
                .to_string_lossy()
                .to_string()
        });
    payload.insert(
        "thread_worktree_path".to_string(),
        Value::String(thread_worktree_path.clone()),
    );
    payload.insert("thread_branch".to_string(), Value::String(thread_branch));
    payload.insert(
        "thread_repo_ref".to_string(),
        Value::String(thread_repo_ref),
    );
    payload.insert(
        "changed_files".to_string(),
        Value::Array(
            collect_changed_files(&thread_worktree_path)
                .into_iter()
                .map(Value::String)
                .collect(),
        ),
    );
    payload.insert("commands_run".to_string(), Value::Array(Vec::new()));
    payload.insert(
        "artifacts".to_string(),
        Value::Array(
            collect_artifacts(&thread_worktree_path)
                .into_iter()
                .map(Value::String)
                .collect(),
        ),
    );

    let diagnostics = thread.and_then(extract_latest_runtime_diagnostics);
    let blockers = diagnostics
        .as_ref()
        .and_then(|item| item.blocking_reason.clone())
        .map(|value| vec![value])
        .unwrap_or_default();
    payload.insert(
        "blockers".to_string(),
        Value::Array(blockers.into_iter().map(Value::String).collect()),
    );
    if let Some(next_action) = diagnostics
        .as_ref()
        .and_then(|item| item.next_steps.first().cloned())
    {
        payload.insert("next_action".to_string(), Value::String(next_action));
    }
    if let Some(child_evidence) = diagnostics
        .as_ref()
        .map(|item| item.child_evidence.clone())
        .filter(|items| !items.is_empty())
    {
        payload.insert(
            "child_evidence".to_string(),
            Value::Array(child_evidence.into_iter().map(Value::String).collect()),
        );
    }
    if let Some(child_transcript_resume) = diagnostics
        .as_ref()
        .map(|item| item.child_transcript_resume.clone())
        .filter(|items| !items.is_empty())
    {
        payload.insert(
            "child_transcript_resume".to_string(),
            Value::Array(
                child_transcript_resume
                    .into_iter()
                    .map(Value::String)
                    .collect(),
            ),
        );
    }
    if let Some(tests) = diagnostics_to_tests_object(diagnostics.as_ref()) {
        payload.insert("tests".to_string(), tests);
    }
    Some(Value::Object(payload))
}

fn base_payload(channel: &ChatChannelDoc) -> Option<Map<String, Value>> {
    if !is_coding_channel(channel) {
        return None;
    }
    let workspace_path = channel.workspace_path.as_deref()?.trim().to_string();
    let repo_path = channel.repo_path.as_deref()?.trim().to_string();
    let main_checkout_path = channel
        .main_checkout_path
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| {
            channel
                .repo_root
                .as_deref()
                .filter(|value| !value.trim().is_empty())
        })?
        .to_string();
    let mut payload = Map::new();
    payload.insert(
        "workspace_display_name".to_string(),
        Value::String(
            channel
                .workspace_display_name
                .clone()
                .filter(|value| !value.trim().is_empty())
                .unwrap_or_else(|| channel.name.clone()),
        ),
    );
    payload.insert("workspace_path".to_string(), Value::String(workspace_path));
    payload.insert("repo_path".to_string(), Value::String(repo_path));
    payload.insert(
        "main_checkout_path".to_string(),
        Value::String(main_checkout_path),
    );
    payload.insert(
        "repo_default_branch".to_string(),
        Value::String(
            channel
                .repo_default_branch
                .clone()
                .filter(|value| !value.trim().is_empty())
                .unwrap_or_else(|| "main".to_string()),
        ),
    );
    Some(payload)
}

fn extract_latest_runtime_diagnostics(
    thread: &ChatChannelThreadResponse,
) -> Option<RuntimeDiagnosticsSnapshot> {
    thread
        .messages
        .iter()
        .rev()
        .map(extract_message_runtime_diagnostics)
        .find(|item| item.is_some())
        .flatten()
        .or_else(|| extract_message_runtime_diagnostics(&thread.root_message))
}

fn extract_message_runtime_diagnostics(
    message: &ChatChannelMessageResponse,
) -> Option<RuntimeDiagnosticsSnapshot> {
    let metadata = message.metadata.as_object()?;
    let raw = metadata.get("runtime_diagnostics")?.as_object()?;
    let next_steps = raw
        .get("next_steps")
        .and_then(|value| value.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToString::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let child_evidence = extract_persisted_child_evidence_lines(raw);
    let child_transcript_resume = extract_persisted_child_transcript_resume_lines(raw);
    Some(RuntimeDiagnosticsSnapshot {
        status: raw
            .get("status")
            .and_then(|value| value.as_str())
            .map(ToString::to_string),
        summary: raw
            .get("summary")
            .and_then(|value| value.as_str())
            .map(ToString::to_string),
        validation_status: raw
            .get("validation_status")
            .and_then(|value| value.as_str())
            .map(ToString::to_string),
        blocking_reason: raw
            .get("blocking_reason")
            .and_then(|value| value.as_str())
            .map(ToString::to_string),
        next_steps,
        child_evidence,
        child_transcript_resume,
    })
}

fn extract_persisted_child_evidence_lines(raw: &Map<String, Value>) -> Vec<String> {
    raw.get("persisted_child_evidence")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_object)
                .map(|item| {
                    let evidence = PersistedChildEvidenceItem {
                        task_id: item
                            .get("task_id")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string(),
                        status: item
                            .get("status")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string(),
                        logical_worker_id: item
                            .get("logical_worker_id")
                            .and_then(Value::as_str)
                            .map(ToString::to_string),
                        attempt_id: item
                            .get("attempt_id")
                            .and_then(Value::as_str)
                            .map(ToString::to_string),
                        recovered_terminal_summary: item
                            .get("recovered_terminal_summary")
                            .and_then(Value::as_bool)
                            .unwrap_or(false),
                        child_session_id: item
                            .get("child_session_id")
                            .and_then(Value::as_str)
                            .map(ToString::to_string),
                        session_preview: item
                            .get("session_preview")
                            .and_then(Value::as_str)
                            .map(ToString::to_string),
                        transcript_excerpt: item
                            .get("transcript_excerpt")
                            .and_then(Value::as_array)
                            .map(|lines| {
                                lines
                                    .iter()
                                    .filter_map(Value::as_str)
                                    .map(ToString::to_string)
                                    .collect::<Vec<_>>()
                            })
                            .unwrap_or_default(),
                        runtime_state: item
                            .get("runtime_state")
                            .and_then(Value::as_str)
                            .map(ToString::to_string),
                        runtime_detail: item
                            .get("runtime_detail")
                            .and_then(Value::as_str)
                            .map(ToString::to_string),
                        summary: item
                            .get("summary")
                            .and_then(Value::as_str)
                            .map(ToString::to_string),
                    };
                    render_child_evidence_line(
                        &evidence,
                        evidence.status != "running" && evidence.status != "pending",
                    )
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

fn extract_persisted_child_transcript_resume_lines(raw: &Map<String, Value>) -> Vec<String> {
    raw.get("persisted_child_transcript_resume")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_object)
                .flat_map(|item| {
                    let session = item
                        .get("child_session_id")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    let source = item
                        .get("transcript_source")
                        .and_then(Value::as_str)
                        .unwrap_or("unknown");
                    item.get("transcript_lines")
                        .and_then(Value::as_array)
                        .map(|lines| {
                            lines
                                .iter()
                                .filter_map(Value::as_str)
                                .map(|line| {
                                    if session.is_empty() {
                                        format!("[{}] {}", source, line)
                                    } else {
                                        format!("[{}][{}] {}", source, session, line)
                                    }
                                })
                                .collect::<Vec<_>>()
                        })
                        .unwrap_or_default()
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

fn diagnostics_to_tests_object(diagnostics: Option<&RuntimeDiagnosticsSnapshot>) -> Option<Value> {
    let diagnostics = diagnostics?;
    let mut object = Map::new();
    if let Some(status) = diagnostics
        .validation_status
        .clone()
        .or_else(|| diagnostics.status.clone())
    {
        object.insert("status".to_string(), Value::String(status));
    }
    if let Some(summary) = diagnostics
        .summary
        .clone()
        .filter(|value| !value.trim().is_empty())
    {
        object.insert("summary".to_string(), Value::String(summary));
    }
    if object.is_empty() {
        None
    } else {
        Some(Value::Object(object))
    }
}

fn collect_changed_files(worktree_path: &str) -> Vec<String> {
    let output = match Command::new("git")
        .args([
            "-C",
            worktree_path,
            "status",
            "--short",
            "--untracked-files=all",
        ])
        .output()
    {
        Ok(output) if output.status.success() => output,
        _ => return Vec::new(),
    };
    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut paths = BTreeSet::new();
    for line in stdout
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
    {
        let raw = line.get(3..).unwrap_or("").trim();
        if raw.is_empty() {
            continue;
        }
        let path = raw
            .rsplit_once(" -> ")
            .map(|(_, value)| value)
            .unwrap_or(raw)
            .trim();
        if !path.is_empty() {
            paths.insert(path.to_string());
        }
        if paths.len() >= 12 {
            break;
        }
    }
    paths.into_iter().collect()
}

fn collect_artifacts(worktree_path: &str) -> Vec<String> {
    let artifact_dir = Path::new(worktree_path).join("artifacts");
    let Ok(entries) = fs::read_dir(artifact_dir) else {
        return Vec::new();
    };
    entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.trim().is_empty() {
                None
            } else {
                Some(name)
            }
        })
        .take(8)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_persisted_child_evidence_lines_prefers_runtime_detail_then_preview() {
        let raw = serde_json::json!({
            "persisted_child_evidence": [
                {
                    "task_id": "task-1",
                    "child_session_id": "subagent-1",
                    "runtime_state": "permission_requested",
                    "runtime_detail": "waiting on developer__shell"
                },
                {
                    "task_id": "task-2",
                    "child_session_id": "subagent-2",
                    "runtime_state": "running",
                    "session_preview": "drafted docs/b.md"
                }
            ]
        });
        let lines = extract_persisted_child_evidence_lines(raw.as_object().expect("object"));
        assert_eq!(lines.len(), 2);
        assert!(lines[0].contains("waiting on developer__shell"));
        assert!(lines[1].contains("drafted docs/b.md"));
    }

    #[test]
    fn extract_persisted_child_evidence_lines_falls_back_to_transcript_excerpt() {
        let raw = serde_json::json!({
            "persisted_child_evidence": [
                {
                    "task_id": "task-1",
                    "child_session_id": "subagent-1",
                    "runtime_state": "running",
                    "transcript_excerpt": [
                        "user: inspect docs/a.md",
                        "assistant: checking the file"
                    ]
                }
            ]
        });
        let lines = extract_persisted_child_evidence_lines(raw.as_object().expect("object"));
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("assistant: checking the file"));
    }

    #[test]
    fn extract_persisted_child_transcript_resume_lines_keeps_selection_context() {
        let raw = serde_json::json!({
            "persisted_child_transcript_resume": [
                {
                    "task_id": "task-1",
                    "child_session_id": "subagent-1",
                    "transcript_source": "active:live_child_session",
                    "transcript_lines": [
                        "user: inspect docs/a.md",
                        "assistant: found the issue"
                    ]
                }
            ]
        });
        let lines =
            extract_persisted_child_transcript_resume_lines(raw.as_object().expect("object"));
        assert_eq!(lines.len(), 2);
        assert!(lines[0].contains("[active:live_child_session][subagent-1]"));
        assert!(lines[1].contains("assistant: found the issue"));
    }
}
