use std::fs::{self, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{anyhow, Context, Result};
use chrono::Utc;
use fs2::FileExt;
use once_cell::sync::Lazy;
use rmcp::model::JsonObject;
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};
use tokio::time::sleep;

use crate::permission::permission_confirmation::PrincipalType;
use crate::permission::{Permission, PermissionConfirmation};

const AGIME_PERMISSION_BRIDGE_TIMEOUT_SECS_ENV: &str = "AGIME_PERMISSION_BRIDGE_TIMEOUT_SECS";

fn root_dir(working_dir: &Path, run_id: &str) -> PathBuf {
    working_dir
        .join(".agime")
        .join("swarm")
        .join(run_id)
        .join("permissions")
}

pub fn pending_dir(working_dir: &Path, run_id: &str) -> PathBuf {
    root_dir(working_dir, run_id).join("pending")
}

pub fn resolved_dir(working_dir: &Path, run_id: &str) -> PathBuf {
    root_dir(working_dir, run_id).join("resolved")
}

pub fn permission_bridge_timeout() -> Duration {
    std::env::var(AGIME_PERMISSION_BRIDGE_TIMEOUT_SECS_ENV)
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| *value > 0)
        .map(Duration::from_secs)
        .unwrap_or_else(|| Duration::from_secs(60))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermissionBridgeRequest {
    pub request_id: String,
    pub run_id: String,
    pub worker_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub logical_worker_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attempt_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attempt_index: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_task_id: Option<String>,
    pub tool_name: String,
    pub tool_use_id: String,
    pub description: String,
    pub arguments: JsonObject,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_input_snapshot: Option<JsonObject>,
    pub write_scope: Vec<String>,
    #[serde(default)]
    pub validation_mode: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission_policy: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionDecisionSource {
    Config,
    RuntimePolicy,
    UserTemporary,
    UserPersistent,
    UserReject,
    Cached,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermissionBridgeResolution {
    pub request_id: String,
    pub permission: Permission,
    pub feedback: Option<String>,
    #[serde(default = "default_permission_decision_source")]
    pub source: PermissionDecisionSource,
    pub resolved_at: String,
}

fn default_permission_decision_source() -> PermissionDecisionSource {
    PermissionDecisionSource::Config
}

pub fn permission_decision_source_label(source: &PermissionDecisionSource) -> &'static str {
    match source {
        PermissionDecisionSource::Config => "config",
        PermissionDecisionSource::RuntimePolicy => "runtime_policy",
        PermissionDecisionSource::UserTemporary => "user_temporary",
        PermissionDecisionSource::UserPersistent => "user_persistent",
        PermissionDecisionSource::UserReject => "user_reject",
        PermissionDecisionSource::Cached => "cached",
    }
}

pub fn permission_resolution_feedback(resolution: &PermissionBridgeResolution) -> Option<String> {
    let source = permission_decision_source_label(&resolution.source);
    match resolution.feedback.as_deref().map(str::trim) {
        Some(feedback) if !feedback.is_empty() => Some(format!("{} [source:{}]", feedback, source)),
        _ => Some(format!("decision source: {}", source)),
    }
}

pub type PermissionBridgeResolver =
    Arc<dyn Fn(&PermissionBridgeRequest) -> PermissionBridgeResolution + Send + Sync>;

static PERMISSION_BRIDGE_RESOLVER: Lazy<Mutex<Option<PermissionBridgeResolver>>> =
    Lazy::new(|| Mutex::new(None));

fn ensure_dirs(working_dir: &Path, run_id: &str) -> Result<()> {
    fs::create_dir_all(pending_dir(working_dir, run_id))?;
    fs::create_dir_all(resolved_dir(working_dir, run_id))?;
    Ok(())
}

fn request_path(working_dir: &Path, run_id: &str, request_id: &str) -> PathBuf {
    pending_dir(working_dir, run_id).join(format!("{}.json", request_id))
}

fn resolution_path(working_dir: &Path, run_id: &str, request_id: &str) -> PathBuf {
    resolved_dir(working_dir, run_id).join(format!("{}.json", request_id))
}

fn write_json_locked<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut file = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .read(true)
        .open(path)
        .with_context(|| format!("failed to open {}", path.display()))?;
    file.lock_exclusive()?;
    let result = (|| {
        file.set_len(0)?;
        file.seek(SeekFrom::Start(0))?;
        file.write_all(serde_json::to_string_pretty(value)?.as_bytes())?;
        file.flush()?;
        Ok(())
    })();
    let unlock_result = file.unlock();
    match (result, unlock_result) {
        (Ok(()), Ok(())) => Ok(()),
        (Ok(()), Err(error)) => Err(error).with_context(|| "failed to unlock permission file"),
        (Err(error), _) => Err(error),
    }
}

fn read_json_locked<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T> {
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .with_context(|| format!("failed to open {}", path.display()))?;
    file.lock_shared()?;
    let result = (|| {
        let mut raw = String::new();
        file.read_to_string(&mut raw)?;
        serde_json::from_str(&raw).with_context(|| format!("failed to parse {}", path.display()))
    })();
    let unlock_result = file.unlock();
    match (result, unlock_result) {
        (Ok(value), Ok(())) => Ok(value),
        (Ok(_), Err(error)) => Err(error).with_context(|| "failed to unlock permission file"),
        (Err(error), _) => Err(error),
    }
}

pub fn write_permission_request(
    working_dir: &Path,
    request: &PermissionBridgeRequest,
) -> Result<PathBuf> {
    ensure_dirs(working_dir, &request.run_id)?;
    let path = request_path(working_dir, &request.run_id, &request.request_id);
    write_json_locked(&path, request)?;
    Ok(path)
}

pub fn list_pending_requests(
    working_dir: &Path,
    run_id: &str,
) -> Result<Vec<PermissionBridgeRequest>> {
    ensure_dirs(working_dir, run_id)?;
    let mut requests = Vec::new();
    for entry in fs::read_dir(pending_dir(working_dir, run_id))? {
        let path = entry?.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        requests.push(read_json_locked(&path)?);
    }
    Ok(requests)
}

pub fn resolve_permission_request(
    working_dir: &Path,
    run_id: &str,
    resolution: &PermissionBridgeResolution,
) -> Result<PathBuf> {
    ensure_dirs(working_dir, run_id)?;
    let path = resolution_path(working_dir, run_id, &resolution.request_id);
    write_json_locked(&path, resolution)?;
    let pending = request_path(working_dir, run_id, &resolution.request_id);
    if pending.exists() {
        let _ = fs::remove_file(pending);
    }
    Ok(path)
}

pub async fn await_permission_resolution(
    working_dir: &Path,
    run_id: &str,
    request_id: &str,
    timeout: Duration,
) -> Result<PermissionBridgeResolution> {
    let path = resolution_path(working_dir, run_id, request_id);
    let deadline = Instant::now() + timeout;
    loop {
        if path.exists() {
            return read_json_locked(&path);
        }
        if Instant::now() >= deadline {
            return Err(anyhow!(
                "timed out waiting for permission resolution {}",
                request_id
            ));
        }
        sleep(Duration::from_millis(200)).await;
    }
}

fn tool_name_is_read_only(tool_name: &str) -> bool {
    let read_only_markers = ["read", "search", "list", "inspect", "query", "fetch", "get"];
    read_only_markers
        .iter()
        .any(|marker| tool_name.to_ascii_lowercase().contains(marker))
}

fn tool_name_is_high_risk(tool_name: &str) -> bool {
    let mutating_markers = [
        "write",
        "edit",
        "create",
        "update",
        "delete",
        "apply",
        "run",
        "execute",
        "shell",
        "bash",
        "powershell",
        "python",
        "network",
        "http",
        "post",
        "put",
        "patch",
        "install",
    ];
    mutating_markers
        .iter()
        .any(|marker| tool_name.to_ascii_lowercase().contains(marker))
}

pub fn auto_resolve_request(request: &PermissionBridgeRequest) -> PermissionBridgeResolution {
    let allow = if request.validation_mode {
        tool_name_is_read_only(&request.tool_name)
    } else if tool_name_is_read_only(&request.tool_name) {
        true
    } else if tool_name_is_high_risk(&request.tool_name) {
        !request.write_scope.is_empty()
    } else {
        !request.write_scope.is_empty()
    };

    let feedback = if allow {
        None
    } else if request.validation_mode {
        Some(
            "Validation workers are read-only. This permission request was denied because the tool is not classified as read-only."
                .to_string(),
        )
    } else {
        Some(
            "Leader permission bridge denied this worker request because no bounded write scope was declared."
                .to_string(),
        )
    };

    PermissionBridgeResolution {
        request_id: request.request_id.clone(),
        permission: if allow {
            Permission::AllowOnce
        } else {
            Permission::DenyOnce
        },
        feedback,
        source: PermissionDecisionSource::Config,
        resolved_at: Utc::now().to_rfc3339(),
    }
}

pub fn timeout_resolution(
    request: &PermissionBridgeRequest,
    timeout: Duration,
) -> PermissionBridgeResolution {
    PermissionBridgeResolution {
        request_id: request.request_id.clone(),
        permission: Permission::DenyOnce,
        feedback: Some(format!(
            "Leader permission bridge timed out after {}s while waiting to resolve `{}`.",
            timeout.as_secs(),
            request.tool_name
        )),
        source: PermissionDecisionSource::RuntimePolicy,
        resolved_at: Utc::now().to_rfc3339(),
    }
}

pub fn set_permission_bridge_resolver(resolver: PermissionBridgeResolver) {
    let mut slot = PERMISSION_BRIDGE_RESOLVER
        .lock()
        .expect("permission bridge resolver lock poisoned");
    *slot = Some(resolver);
}

pub fn clear_permission_bridge_resolver() {
    let mut slot = PERMISSION_BRIDGE_RESOLVER
        .lock()
        .expect("permission bridge resolver lock poisoned");
    *slot = None;
}

pub fn permission_bridge_resolver_configured() -> bool {
    PERMISSION_BRIDGE_RESOLVER
        .lock()
        .expect("permission bridge resolver lock poisoned")
        .is_some()
}

pub fn resolve_permission_request_via_runtime(
    request: &PermissionBridgeRequest,
) -> PermissionBridgeResolution {
    let resolver = PERMISSION_BRIDGE_RESOLVER
        .lock()
        .expect("permission bridge resolver lock poisoned")
        .clone();
    if let Some(resolver) = resolver {
        resolver(request)
    } else {
        auto_resolve_request(request)
    }
}

pub fn resolve_permission_request_via_policy(
    request: &PermissionBridgeRequest,
) -> PermissionBridgeResolution {
    resolve_permission_request_via_runtime(request)
}

pub fn to_permission_confirmation(
    resolution: &PermissionBridgeResolution,
) -> PermissionConfirmation {
    PermissionConfirmation {
        principal_type: PrincipalType::Tool,
        permission: resolution.permission.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn permission_bridge_round_trip_resolves_request() {
        let temp = tempfile::tempdir().expect("tempdir");
        let request = PermissionBridgeRequest {
            request_id: "req-1".to_string(),
            run_id: "run-1".to_string(),
            worker_name: "worker-1".to_string(),
            logical_worker_id: None,
            attempt_id: None,
            attempt_index: None,
            previous_task_id: None,
            tool_name: "write_file".to_string(),
            tool_use_id: "tool-1".to_string(),
            description: "write".to_string(),
            arguments: JsonObject::default(),
            tool_input_snapshot: None,
            write_scope: vec!["src/out.md".to_string()],
            validation_mode: false,
            permission_policy: None,
            created_at: Utc::now().to_rfc3339(),
        };
        write_permission_request(temp.path(), &request).expect("request");
        let pending = list_pending_requests(temp.path(), "run-1").expect("pending");
        assert_eq!(pending.len(), 1);

        let resolution = auto_resolve_request(&request);
        resolve_permission_request(temp.path(), "run-1", &resolution).expect("resolve");
        let resolved =
            await_permission_resolution(temp.path(), "run-1", "req-1", Duration::from_secs(1))
                .await
                .expect("await");
        assert_eq!(resolved.permission, Permission::AllowOnce);
    }

    #[test]
    fn custom_resolver_overrides_default_policy() {
        clear_permission_bridge_resolver();
        set_permission_bridge_resolver(Arc::new(|request| PermissionBridgeResolution {
            request_id: request.request_id.clone(),
            permission: Permission::DenyOnce,
            feedback: Some("custom policy".to_string()),
            source: PermissionDecisionSource::RuntimePolicy,
            resolved_at: Utc::now().to_rfc3339(),
        }));

        let request = PermissionBridgeRequest {
            request_id: "req-2".to_string(),
            run_id: "run-1".to_string(),
            worker_name: "worker-1".to_string(),
            logical_worker_id: None,
            attempt_id: None,
            attempt_index: None,
            previous_task_id: None,
            tool_name: "read_file".to_string(),
            tool_use_id: "tool-2".to_string(),
            description: "read".to_string(),
            arguments: JsonObject::default(),
            tool_input_snapshot: None,
            write_scope: vec!["src/out.md".to_string()],
            validation_mode: false,
            permission_policy: None,
            created_at: Utc::now().to_rfc3339(),
        };
        let resolved = resolve_permission_request_via_policy(&request);
        assert_eq!(resolved.permission, Permission::DenyOnce);
        clear_permission_bridge_resolver();
    }

    #[test]
    fn auto_resolve_request_uses_config_source() {
        let request = PermissionBridgeRequest {
            request_id: "req-5".to_string(),
            run_id: "run-1".to_string(),
            worker_name: "worker-1".to_string(),
            logical_worker_id: None,
            attempt_id: None,
            attempt_index: None,
            previous_task_id: None,
            tool_name: "read_file".to_string(),
            tool_use_id: "tool-5".to_string(),
            description: "read".to_string(),
            arguments: JsonObject::default(),
            tool_input_snapshot: None,
            write_scope: vec!["src/out.md".to_string()],
            validation_mode: false,
            permission_policy: None,
            created_at: Utc::now().to_rfc3339(),
        };
        let resolved = auto_resolve_request(&request);
        assert_eq!(permission_decision_source_label(&resolved.source), "config");
        assert_eq!(
            permission_resolution_feedback(&resolved).as_deref(),
            Some("decision source: config")
        );
    }

    #[test]
    fn timeout_resolution_uses_runtime_policy_source() {
        let request = PermissionBridgeRequest {
            request_id: "req-6".to_string(),
            run_id: "run-1".to_string(),
            worker_name: "worker-1".to_string(),
            logical_worker_id: None,
            attempt_id: None,
            attempt_index: None,
            previous_task_id: None,
            tool_name: "write_file".to_string(),
            tool_use_id: "tool-6".to_string(),
            description: "write".to_string(),
            arguments: JsonObject::default(),
            tool_input_snapshot: None,
            write_scope: vec!["src/out.md".to_string()],
            validation_mode: false,
            permission_policy: None,
            created_at: Utc::now().to_rfc3339(),
        };
        let resolved = timeout_resolution(&request, Duration::from_secs(5));
        assert_eq!(
            permission_decision_source_label(&resolved.source),
            "runtime_policy"
        );
        assert!(permission_resolution_feedback(&resolved)
            .as_deref()
            .is_some_and(|value| value.contains("[source:runtime_policy]")));
    }

    #[test]
    fn validation_workers_cannot_auto_approve_mutating_tools() {
        let request = PermissionBridgeRequest {
            request_id: "req-3".to_string(),
            run_id: "run-1".to_string(),
            worker_name: "validator-1".to_string(),
            logical_worker_id: None,
            attempt_id: None,
            attempt_index: None,
            previous_task_id: None,
            tool_name: "write_file".to_string(),
            tool_use_id: "tool-3".to_string(),
            description: "write".to_string(),
            arguments: JsonObject::default(),
            tool_input_snapshot: None,
            write_scope: vec!["src/out.md".to_string()],
            validation_mode: true,
            permission_policy: None,
            created_at: Utc::now().to_rfc3339(),
        };
        let resolved = auto_resolve_request(&request);
        assert_eq!(resolved.permission, Permission::DenyOnce);
    }

    #[test]
    fn resolve_permission_request_removes_pending_file() {
        let temp = tempfile::tempdir().expect("tempdir");
        let request = PermissionBridgeRequest {
            request_id: "req-4".to_string(),
            run_id: "run-1".to_string(),
            worker_name: "worker-1".to_string(),
            logical_worker_id: None,
            attempt_id: None,
            attempt_index: None,
            previous_task_id: None,
            tool_name: "write_file".to_string(),
            tool_use_id: "tool-4".to_string(),
            description: "write".to_string(),
            arguments: JsonObject::default(),
            tool_input_snapshot: None,
            write_scope: vec!["src/out.md".to_string()],
            validation_mode: false,
            permission_policy: None,
            created_at: Utc::now().to_rfc3339(),
        };
        write_permission_request(temp.path(), &request).expect("request");
        let pending_before = list_pending_requests(temp.path(), "run-1").expect("pending");
        assert_eq!(pending_before.len(), 1);

        let resolution = auto_resolve_request(&request);
        resolve_permission_request(temp.path(), "run-1", &resolution).expect("resolve");

        let pending_after = list_pending_requests(temp.path(), "run-1").expect("pending");
        assert!(pending_after.is_empty());
    }
}
