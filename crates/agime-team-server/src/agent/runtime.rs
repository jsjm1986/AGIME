//! Shared runtime utilities for executor bridge pattern
//!
//! Both ChatExecutor and MissionExecutor use the same "bridge pattern"
//! to reuse TaskExecutor: create temp task → approve → register in
//! internal TaskManager → bridge events → execute → cleanup.
//!
//! This module extracts the shared logic to eliminate duplication.

use agime_team::models::{BuiltinExtension, TeamAgent};
use agime_team::MongoDb;
use anyhow::{anyhow, Result};
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;

use super::executor_mongo::TaskExecutor;
use super::service_mongo::AgentService;
use super::task_manager::{StreamEvent, TaskManager};

/// Trait for broadcasting stream events to subscribers.
///
/// Both ChatManager and MissionManager implement this trait,
/// allowing shared runtime functions to work with either.
pub trait EventBroadcaster: Send + Sync + 'static {
    fn broadcast(
        &self,
        context_id: &str,
        event: StreamEvent,
    ) -> impl std::future::Future<Output = ()> + Send;
}

/// Create a temporary task record for TaskExecutor to consume.
pub async fn create_temp_task(
    agent_service: &AgentService,
    team_id: &str,
    agent_id: &str,
    user_id: &str,
    content: serde_json::Value,
) -> Result<agime_team::models::AgentTask> {
    use agime_team::models::{SubmitTaskRequest, TaskType};

    let req = SubmitTaskRequest {
        team_id: team_id.to_string(),
        agent_id: agent_id.to_string(),
        task_type: TaskType::Chat,
        content,
        priority: 0,
    };

    agent_service
        .submit_task(user_id, req)
        .await
        .map_err(|e| anyhow!("Failed to create temp task: {}", e))
}

/// Clean up a temporary task record from MongoDB.
pub async fn cleanup_temp_task(db: &MongoDb, task_id: &str) -> Result<()> {
    use mongodb::bson::doc;
    let collection: mongodb::Collection<mongodb::bson::Document> = db.collection("agent_tasks");
    collection
        .delete_one(doc! { "task_id": task_id }, None)
        .await
        .map_err(|e| anyhow!("Failed to cleanup temp task: {}", e))?;
    Ok(())
}

/// Bridge events from internal TaskManager to an EventBroadcaster.
///
/// Forwards all events except Done (which the outer executor sends itself).
pub async fn bridge_events<B: EventBroadcaster>(
    task_mgr: &TaskManager,
    task_id: &str,
    broadcaster: &B,
    context_id: &str,
) {
    let mut rx = match task_mgr.subscribe(task_id).await {
        Some(rx) => rx,
        None => return,
    };

    loop {
        match rx.recv().await {
            Ok(event) => {
                let is_done = matches!(&event, StreamEvent::Done { .. });
                if is_done {
                    break;
                }
                broadcaster.broadcast(context_id, event).await;
            }
            Err(broadcast::error::RecvError::Closed) => break,
            Err(broadcast::error::RecvError::Lagged(n)) => {
                tracing::warn!("Bridge lagged {} events", n);
            }
        }
    }
}

/// Execute a user message via the bridge pattern.
///
/// This is the core shared flow:
/// 1. Load session to get team_id/user_id
/// 2. Create temp task
/// 3. Approve temp task
/// 4. Register in internal TaskManager
/// 5. Bridge events to broadcaster
/// 6. Link cancellation tokens
/// 7. Execute via TaskExecutor
/// 8. Wait for bridge, cleanup
pub async fn execute_via_bridge<B: EventBroadcaster>(
    db: &Arc<MongoDb>,
    agent_service: &AgentService,
    internal_task_manager: &Arc<TaskManager>,
    broadcaster: &Arc<B>,
    context_id: &str,
    agent_id: &str,
    session_id: &str,
    user_message: &str,
    cancel_token: CancellationToken,
    workspace_path: Option<&str>,
) -> Result<()> {
    // Load session to get team_id and user_id
    let session = agent_service
        .get_session(session_id)
        .await
        .map_err(|e| anyhow!("DB error: {}", e))?
        .ok_or_else(|| anyhow!("Session not found"))?;

    // Build task content
    let mut content = serde_json::json!({
        "messages": [{"role": "user", "content": user_message}],
        "session_id": session_id,
        "compaction_strategy": "cfpm_memory_v1",
    });
    if let Some(wp) = workspace_path {
        content["workspace_path"] = serde_json::Value::String(wp.to_string());
    }

    // Create temp task
    let task = create_temp_task(
        agent_service,
        &session.team_id,
        agent_id,
        &session.user_id,
        content,
    )
    .await?;
    let task_id = task.id.clone();

    // Approve temp task
    agent_service
        .approve_task(&task_id, &session.user_id)
        .await
        .map_err(|e| anyhow!("Failed to approve: {}", e))?;

    // Register in internal task manager
    let (internal_cancel, _) = internal_task_manager.register(&task_id).await;

    // Bridge events
    let bridge_context_id = context_id.to_string();
    let bridge_broadcaster = broadcaster.clone();
    let bridge_task_mgr = internal_task_manager.clone();
    let bridge_task_id = task_id.clone();

    let bridge_handle = tokio::spawn(async move {
        bridge_events(
            &bridge_task_mgr,
            &bridge_task_id,
            bridge_broadcaster.as_ref(),
            &bridge_context_id,
        )
        .await;
    });

    // Link cancellation tokens
    let linked = internal_cancel.clone();
    let external = cancel_token.clone();
    tokio::spawn(async move {
        external.cancelled().await;
        linked.cancel();
    });

    // Execute via TaskExecutor
    let executor = TaskExecutor::new(db.clone(), internal_task_manager.clone());
    let exec_result = executor.execute_task(&task_id, internal_cancel).await;

    // Wait for bridge to finish
    let _ = bridge_handle.await;

    // Cleanup temp task
    let _ = cleanup_temp_task(db, &task_id).await;

    exec_result
}

// ─── Shared Helpers ─────────────────────────────────────

/// Extract JSON from ```json ... ``` code block, or return the whole string.
pub fn extract_json_block(text: &str) -> String {
    if let Some(start) = text.find("```json") {
        let after = &text[start + 7..];
        if let Some(end) = after.find("```") {
            return after[..end].trim().to_string();
        }
    }
    if let Some(start) = text.find("```") {
        let after = &text[start + 3..];
        if let Some(end) = after.find("```") {
            return after[..end].trim().to_string();
        }
    }
    text.trim().to_string()
}

/// Extract text content from the last assistant message in a messages JSON array.
pub fn extract_last_assistant_text(messages_json: &str) -> Option<String> {
    let msgs: Vec<serde_json::Value> = serde_json::from_str(messages_json).ok()?;
    for msg in msgs.iter().rev() {
        if msg.get("role").and_then(|r| r.as_str()) != Some("assistant") {
            continue;
        }
        let content = msg.get("content")?;
        if let Some(s) = content.as_str() {
            if !s.is_empty() {
                return Some(s.to_string());
            }
        }
        if let Some(arr) = content.as_array() {
            for item in arr {
                if let Some(t) = item.get("text").and_then(|t| t.as_str()) {
                    if !t.is_empty() {
                        return Some(t.to_string());
                    }
                }
            }
        }
    }
    None
}

// ─── Workspace Directory Helpers ────────────────────────

/// Validate that a path segment is safe (no traversal, separators, or special chars).
fn validate_path_segment(segment: &str, label: &str) -> Result<()> {
    if segment.is_empty() {
        return Err(anyhow!("{} must not be empty", label));
    }
    if segment == "."
        || segment.contains("..")
        || segment.contains('/')
        || segment.contains('\\')
        || segment.contains('\0')
    {
        return Err(anyhow!(
            "{} contains invalid characters: {:?}",
            label,
            segment
        ));
    }
    Ok(())
}

/// Create a workspace directory with path validation.
///
/// `segments` are joined under `root` to form the final path.
/// Each segment is validated to prevent path traversal attacks.
pub fn create_workspace_dir(root: &str, segments: &[(&str, &str)]) -> Result<String> {
    let mut path = PathBuf::from(root);
    for (segment, label) in segments {
        validate_path_segment(segment, label)?;
        path.push(segment);
    }
    std::fs::create_dir_all(&path)
        .map_err(|e| anyhow!("Failed to create workspace dir {:?}: {}", path, e))?;
    Ok(path.to_string_lossy().to_string())
}

/// Remove a workspace directory.
///
/// Returns `Ok(true)` if removed, `Ok(false)` if path was empty/missing,
/// `Err` if `remove_dir_all` fails (callers should treat as non-fatal).
pub fn cleanup_workspace_dir(workspace_path: Option<&str>) -> Result<bool> {
    let path = match workspace_path {
        Some(p) if !p.is_empty() => PathBuf::from(p),
        _ => return Ok(false),
    };
    if !path.exists() {
        return Ok(false);
    }
    std::fs::remove_dir_all(&path)
        .map_err(|e| anyhow!("Failed to remove workspace dir {:?}: {}", path, e))?;
    Ok(true)
}

/// Scan a project directory and return a context string with file tree and key file contents.
/// Used to give the agent immediate awareness of the project structure at session start.
pub fn scan_project_context(project_path: &str, max_total_bytes: usize) -> String {
    let base = std::path::Path::new(project_path);
    if !base.is_dir() {
        return String::new();
    }

    let mut out = String::new();
    out.push_str("<project_context>\n");

    // 1. File tree (max 3 levels)
    out.push_str("## 文件结构\n```\n");
    collect_tree(base, base, 0, 3, &mut out);
    out.push_str("```\n\n");

    // 2. Read key files
    const KEY_FILES: &[&str] = &[
        "README.md",
        "readme.md",
        "README.txt",
        "index.html",
        "package.json",
        "Cargo.toml",
        "pyproject.toml",
        "requirements.txt",
        ".env.example",
        "portal-sdk.js",
        "portal-agent-client.js",
    ];

    let mut remaining = max_total_bytes.saturating_sub(out.len());
    for name in KEY_FILES {
        if remaining < 100 {
            break;
        }
        let file_path = base.join(name);
        if let Ok(content) = std::fs::read_to_string(&file_path) {
            if content.trim().is_empty() {
                continue;
            }
            let truncated: String = content.chars().take(remaining.min(4000)).collect();
            out.push_str(&format!("## {}\n```\n{}\n```\n\n", name, truncated));
            remaining = remaining.saturating_sub(truncated.len() + name.len() + 20);
        }
    }

    out.push_str("</project_context>");
    out
}

fn collect_tree(
    base: &std::path::Path,
    dir: &std::path::Path,
    depth: usize,
    max_depth: usize,
    out: &mut String,
) {
    if depth >= max_depth {
        return;
    }
    let mut entries: Vec<_> = match std::fs::read_dir(dir) {
        Ok(rd) => rd.filter_map(|e| e.ok()).collect(),
        Err(_) => return,
    };
    entries.sort_by_key(|e| e.file_name());

    let skip = ["node_modules", ".git", "target", "__pycache__", ".next", "dist"];
    let indent = "  ".repeat(depth);
    let mut count = 0;
    for entry in &entries {
        let name = entry.file_name().to_string_lossy().to_string();
        if skip.iter().any(|s| *s == name) {
            continue;
        }
        let is_dir = entry.file_type().map_or(false, |ft| ft.is_dir());
        if is_dir {
            out.push_str(&format!("{}{}/\n", indent, name));
            collect_tree(base, &entry.path(), depth + 1, max_depth, out);
        } else {
            out.push_str(&format!("{}{}\n", indent, name));
        }
        count += 1;
        if count >= 80 {
            out.push_str(&format!("{}... (truncated)\n", indent));
            break;
        }
    }
}

/// Classify whether an error is transient and worth retrying.
pub fn is_retryable_error(e: &anyhow::Error) -> bool {
    let msg = e.to_string().to_lowercase();
    let retryable_patterns = [
        "timeout",
        "timed out",
        "rate limit",
        "too many requests",
        "429",
        "503",
        "502",
        "connection reset",
        "connection refused",
        "broken pipe",
        "temporarily unavailable",
        "overloaded",
    ];
    retryable_patterns.iter().any(|p| msg.contains(p))
}

// ─── Extension Override Helpers ────────────────────────

/// Result of computing extension overrides between agent config and runtime state.
pub struct ExtensionOverrides {
    pub disabled: Vec<String>,
    pub enabled: Vec<String>,
}

/// Compute extension overrides by diffing agent default config against active runtime names.
///
/// Shared by `executor_mongo` (session save on shutdown) and
/// `extension_manager_client` (session sync after manage operations).
///
/// Rules:
/// - ExtensionManager, ChatRecall, and DocumentTools are excluded
///   (ExtensionManager/ChatRecall are never loaded as regular extensions;
///    DocumentTools is always force-loaded as fallback by PlatformExtensionRunner)
/// - MCP subprocess extensions use `mcp_name()` as their runtime name
/// - Skills→team_skills replacement is handled when team_skills is active
pub fn compute_extension_overrides(
    agent: &TeamAgent,
    active: &HashSet<String>,
) -> ExtensionOverrides {
    let default_names: HashSet<String> = agent
        .enabled_extensions
        .iter()
        .filter(|e| e.enabled)
        .filter(|e| {
            !matches!(
                e.extension,
                BuiltinExtension::ExtensionManager
                    | BuiltinExtension::ChatRecall
                    | BuiltinExtension::DocumentTools
            )
        })
        .map(|e| {
            if let Some(mcp) = e.extension.mcp_name() {
                return mcp.to_string();
            }
            // Skills may be replaced by team_skills at runtime
            if e.extension == BuiltinExtension::Skills && active.contains("team_skills") {
                return "team_skills".to_string();
            }
            e.extension.name().to_string()
        })
        .chain(
            agent
                .custom_extensions
                .iter()
                .filter(|e| e.enabled)
                .map(|e| e.name.clone()),
        )
        .collect();

    let disabled = default_names.difference(active).cloned().collect();
    let enabled = active.difference(&default_names).cloned().collect();

    ExtensionOverrides { disabled, enabled }
}
