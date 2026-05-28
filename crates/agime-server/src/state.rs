use crate::host_prompt::{PromptCapabilitySnapshot, PromptHarnessOverlay, PromptSurfaceContract};
use crate::host_provider::HostProviderConfig;
use crate::host_workspace::WorkspaceExecutionContext;
use agime::execution::manager::AgentManager;
use axum::http::StatusCode;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::AtomicUsize;
use std::sync::Arc;
use tokio::sync::Mutex;

/// Per-session override carrier.
///
/// Step 5 hookpoint: the desktop server now has a place to stash session-scoped
/// LLM / prompt / workspace overrides without changing the core `SessionConfig`
/// shape (which is shared with the CLI).
///
/// - `provider_config` — populated from session metadata or a future
///   `/agent/session_override` route; consumed by
///   [`crate::host_provider::create_provider_for_config`].
/// - `prompt_profile_overlay` / `extra_instructions` /
///   `runtime_overlay_text` / `harness_delegation_overlay_text` /
///   `surface_contract_overlay_text` — pre-rendered overlay slots fed to
///   [`crate::host_prompt::compose_session_overlays_only`]. These are
///   *session-level* — they survive across turns and are re-applied each
///   reply.
/// - `capability_snapshot` / `harness_overlay` / `surface_contract` —
///   typed alternatives to the three pre-rendered text slots above. When a
///   typed slot is present and the matching `*_overlay_text` is `None`,
///   reply.rs renders the text via
///   [`crate::host_prompt::build_runtime_capability_snapshot_overlay`] /
///   [`crate::host_prompt::build_harness_delegation_overlay_text`] /
///   [`crate::host_prompt::build_surface_contract_overlay_text`]. Pre-rendered
///   text wins when both are supplied.
/// - `turn_system_instruction` — single-turn injection. It is *consumed* on
///   the next call to [`AppState::consume_session_override_for_turn`] so the
///   next reply does not see it.
/// - `workspace_context` — fed to
///   [`crate::host_workspace::merge_workspace_execution_instruction`].
#[derive(Debug, Clone, Default)]
pub struct SessionOverride {
    pub provider_config: Option<HostProviderConfig>,
    pub prompt_profile_overlay: Option<String>,
    pub extra_instructions: Option<String>,
    pub turn_system_instruction: Option<String>,
    pub runtime_overlay_text: Option<String>,
    pub harness_delegation_overlay_text: Option<String>,
    pub surface_contract_overlay_text: Option<String>,
    pub capability_snapshot: Option<PromptCapabilitySnapshot>,
    pub harness_overlay: Option<PromptHarnessOverlay>,
    pub surface_contract: Option<PromptSurfaceContract>,
    pub workspace_context: Option<WorkspaceExecutionContext>,
}

#[derive(Clone)]
pub struct AppState {
    pub(crate) agent_manager: Arc<AgentManager>,
    pub recipe_file_hash_map: Arc<Mutex<HashMap<String, PathBuf>>>,
    pub session_counter: Arc<AtomicUsize>,
    /// Tracks sessions that have already emitted recipe telemetry to prevent double counting.
    recipe_session_tracker: Arc<Mutex<HashSet<String>>>,
    /// Per-session overrides for provider / prompt overlays / workspace rules.
    /// Populated lazily by routes that opt in (Step 5 work); read by
    /// [`AppState::session_override`] from the chat / update_from_session paths.
    session_overrides: Arc<Mutex<HashMap<String, SessionOverride>>>,
    /// Records the last `HostProviderConfig` actually applied via
    /// [`agime::agents::Agent::update_provider`] for a given session, so the
    /// `/reply` hook can skip the call when nothing changed. Wiped via
    /// [`AppState::clear_applied_provider_fingerprint`] when an override is
    /// cleared so a subsequent identical override re-applies.
    applied_provider_fingerprints: Arc<Mutex<HashMap<String, HostProviderConfig>>>,
    /// Per-session execution-slot provider. Mirrors team-server's admission
    /// control, but uses an in-process `HashMap<agent_id, count>` because the
    /// desktop is single-process. Default `max_slots = 1` matches today's
    /// single-turn-per-agent behavior.
    #[cfg(feature = "desktop_harness_host")]
    pub(crate) execution_slots: Arc<crate::host_admission::DesktopExecutionSlotProvider>,
    /// Per-session capability snapshot built from
    /// [`crate::host_capability::HostSessionPolicyContext`] when the session
    /// override carries an approval-mode / allow-list. Consumed by the reply
    /// path (and future tool dispatch interception) to evaluate
    /// `requires_approval` and `extension_allowed`. `None` means "no policy"
    /// — fully permissive, matching today's desktop default.
    #[cfg(feature = "desktop_harness_host")]
    pub(crate) capability_snapshots:
        Arc<Mutex<HashMap<String, crate::host_capability::RuntimeCapabilitySnapshot>>>,
    /// Process-wide [`crate::host_workspace_service::WorkspaceService`].
    /// Lazy: created on the first call to [`AppState::workspace_service`] and
    /// rooted under `Paths::data_dir().join("workspaces")` so each desktop
    /// session shares the same on-disk root.
    #[cfg(feature = "desktop_harness_host")]
    pub(crate) workspace_service:
        Arc<Mutex<Option<Arc<crate::host_workspace_service::WorkspaceService>>>>,
    /// Per-session active workspace binding (returned by
    /// `WorkspaceService::ensure_chat_workspace`). Cached so each session
    /// only materialises its workspace folder once per server lifetime.
    #[cfg(feature = "desktop_harness_host")]
    pub(crate) workspace_bindings:
        Arc<Mutex<HashMap<String, crate::host_workspace_types::WorkspaceBinding>>>,
    /// Per-session latest `WorkspaceExecutionContext` (carrying `run_id`).
    /// Updated on each call to
    /// [`AppState::workspace_execution_context_for_session`] so the mirror /
    /// completion-persist path can reuse the same context (and run_id) the
    /// agent saw in its prompt for this turn.
    #[cfg(feature = "desktop_harness_host")]
    pub(crate) workspace_execution_contexts:
        Arc<Mutex<HashMap<String, crate::host_workspace::WorkspaceExecutionContext>>>,
    /// Process-wide desktop [`TaskRuntime`] handle. Lazy-init on first
    /// access via [`AppState::task_runtime`]; passed into every
    /// [`run_harness_host`] call so sub-agent / swarm-worker /
    /// validation-worker events flow through a single shared runtime.
    ///
    /// [`run_harness_host`]: agime::agents::harness::run_harness_host
    /// [`TaskRuntime`]: agime::agents::harness::TaskRuntime
    #[cfg(feature = "desktop_harness_host")]
    pub(crate) task_runtime: Arc<Mutex<Option<Arc<crate::host_task_runtime::DesktopTaskRuntime>>>>,
}

impl AppState {
    pub async fn new() -> anyhow::Result<Arc<AppState>> {
        let agent_manager = AgentManager::instance().await?;

        Ok(Arc::new(Self {
            agent_manager,
            recipe_file_hash_map: Arc::new(Mutex::new(HashMap::new())),
            session_counter: Arc::new(AtomicUsize::new(0)),
            recipe_session_tracker: Arc::new(Mutex::new(HashSet::new())),
            session_overrides: Arc::new(Mutex::new(HashMap::new())),
            applied_provider_fingerprints: Arc::new(Mutex::new(HashMap::new())),
            #[cfg(feature = "desktop_harness_host")]
            execution_slots: Arc::new(
                crate::host_admission::DesktopExecutionSlotProvider::default(),
            ),
            #[cfg(feature = "desktop_harness_host")]
            capability_snapshots: Arc::new(Mutex::new(HashMap::new())),
            #[cfg(feature = "desktop_harness_host")]
            workspace_service: Arc::new(Mutex::new(None)),
            #[cfg(feature = "desktop_harness_host")]
            workspace_bindings: Arc::new(Mutex::new(HashMap::new())),
            #[cfg(feature = "desktop_harness_host")]
            workspace_execution_contexts: Arc::new(Mutex::new(HashMap::new())),
            #[cfg(feature = "desktop_harness_host")]
            task_runtime: Arc::new(Mutex::new(None)),
        }))
    }
    pub async fn set_recipe_file_hash_map(&self, hash_map: HashMap<String, PathBuf>) {
        let mut map = self.recipe_file_hash_map.lock().await;
        *map = hash_map;
    }

    pub async fn mark_recipe_run_if_absent(&self, session_id: &str) -> bool {
        let mut sessions = self.recipe_session_tracker.lock().await;
        if sessions.contains(session_id) {
            false
        } else {
            sessions.insert(session_id.to_string());
            true
        }
    }

    pub async fn get_agent(&self, session_id: String) -> anyhow::Result<Arc<agime::agents::Agent>> {
        self.agent_manager.get_or_create_agent(session_id).await
    }

    pub async fn get_agent_for_route(
        &self,
        session_id: String,
    ) -> Result<Arc<agime::agents::Agent>, StatusCode> {
        self.get_agent(session_id).await.map_err(|e| {
            tracing::error!("Failed to get agent: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })
    }

    /// Replace the override entry for `session_id`. Returns the previous entry
    /// if one existed. Also wipes the cached "already applied" provider
    /// fingerprint so the next chat turn is guaranteed to re-apply the
    /// (possibly new) provider config exactly once.
    pub async fn set_session_override(
        &self,
        session_id: impl Into<String>,
        override_value: SessionOverride,
    ) -> Option<SessionOverride> {
        let key = session_id.into();
        {
            let mut applied = self.applied_provider_fingerprints.lock().await;
            applied.remove(&key);
        }
        let mut map = self.session_overrides.lock().await;
        map.insert(key, override_value)
    }

    /// Snapshot the current override entry for `session_id`, if any. Returns a
    /// clone so callers can hold the value without keeping the lock.
    ///
    /// **Note**: this does not consume `turn_system_instruction`. Use
    /// [`Self::consume_session_override_for_turn`] from the chat reply path
    /// when the caller wants the single-turn semantics.
    pub async fn session_override(&self, session_id: &str) -> Option<SessionOverride> {
        let map = self.session_overrides.lock().await;
        map.get(session_id).cloned()
    }

    /// Take a snapshot of the current override entry for use by a single
    /// chat turn. The returned snapshot includes any pending
    /// `turn_system_instruction`; the field is then atomically cleared from
    /// the stored entry so it is *not* observed by the next turn.
    pub async fn consume_session_override_for_turn(
        &self,
        session_id: &str,
    ) -> Option<SessionOverride> {
        let mut map = self.session_overrides.lock().await;
        let entry = map.get_mut(session_id)?;
        let snapshot = entry.clone();
        if entry.turn_system_instruction.is_some() {
            entry.turn_system_instruction = None;
        }
        Some(snapshot)
    }

    /// Drop the override entry for `session_id`. Returns the removed entry if
    /// one existed. Also wipes the cached "already applied" provider
    /// fingerprint.
    pub async fn clear_session_override(&self, session_id: &str) -> Option<SessionOverride> {
        {
            let mut applied = self.applied_provider_fingerprints.lock().await;
            applied.remove(session_id);
        }
        let mut map = self.session_overrides.lock().await;
        map.remove(session_id)
    }

    /// Returns `true` and records `cfg` as applied if the session has never
    /// applied this exact provider config before; returns `false` if the
    /// previously-applied fingerprint matches (caller should skip the
    /// expensive `Agent::update_provider` call).
    pub async fn record_applied_provider(
        &self,
        session_id: &str,
        cfg: &HostProviderConfig,
    ) -> bool {
        let mut applied = self.applied_provider_fingerprints.lock().await;
        match applied.get(session_id) {
            Some(prev) if prev == cfg => false,
            _ => {
                applied.insert(session_id.to_string(), cfg.clone());
                true
            }
        }
    }

    /// Forget the recorded applied-provider fingerprint, if any. Useful when
    /// the agent is recycled or the session ends.
    pub async fn clear_applied_provider_fingerprint(&self, session_id: &str) {
        let mut applied = self.applied_provider_fingerprints.lock().await;
        applied.remove(session_id);
    }

    /// Resolve and stash a capability snapshot for `session_id`. Returns the
    /// freshly-computed snapshot so callers can also propagate it inline.
    /// Calling with an empty / default policy clears any existing snapshot.
    #[cfg(feature = "desktop_harness_host")]
    pub async fn set_capability_context(
        &self,
        session_id: impl Into<String>,
        ctx: crate::host_capability::HostSessionPolicyContext,
    ) -> crate::host_capability::RuntimeCapabilitySnapshot {
        let snapshot = crate::host_capability::resolve_capabilities(&ctx);
        let mut map = self.capability_snapshots.lock().await;
        map.insert(session_id.into(), snapshot.clone());
        snapshot
    }

    /// Snapshot the current capability snapshot for `session_id`, if one was
    /// installed via [`Self::set_capability_context`]. Returns a clone so
    /// callers can hold the value without keeping the lock.
    #[cfg(feature = "desktop_harness_host")]
    pub async fn capability_snapshot(
        &self,
        session_id: &str,
    ) -> Option<crate::host_capability::RuntimeCapabilitySnapshot> {
        let map = self.capability_snapshots.lock().await;
        map.get(session_id).cloned()
    }

    /// Drop the capability snapshot for `session_id`. Returns the removed
    /// snapshot if one existed.
    #[cfg(feature = "desktop_harness_host")]
    pub async fn clear_capability_context(
        &self,
        session_id: &str,
    ) -> Option<crate::host_capability::RuntimeCapabilitySnapshot> {
        let mut map = self.capability_snapshots.lock().await;
        map.remove(session_id)
    }

    /// Lazy-init the process-wide [`crate::host_workspace_service::WorkspaceService`].
    /// All sessions share the same on-disk root (`Paths::data_dir()/workspaces`)
    /// so the desktop's manifest layout matches what `WorkspaceService` already
    /// expects.
    #[cfg(feature = "desktop_harness_host")]
    pub async fn workspace_service(&self) -> Arc<crate::host_workspace_service::WorkspaceService> {
        let mut guard = self.workspace_service.lock().await;
        if let Some(svc) = guard.as_ref() {
            return svc.clone();
        }
        let root = agime::config::paths::Paths::data_dir().join("workspaces");
        let svc = Arc::new(crate::host_workspace_service::WorkspaceService::new(
            root.to_string_lossy().to_string(),
        ));
        *guard = Some(svc.clone());
        svc
    }

    /// Get the cached chat-workspace binding for `session_id`, or run
    /// `ensure_chat_workspace` once and cache the result.
    ///
    /// Holds the lock across the `ensure_chat_workspace` call to eliminate
    /// the TOCTOU window where two concurrent reply turns for the same
    /// session could each call `ensure_chat_workspace` (which is filesystem
    /// idempotent but not free). The lock is fine-grained — it only
    /// protects this map and is not held across the agent reply itself.
    #[cfg(feature = "desktop_harness_host")]
    pub async fn ensure_chat_workspace_binding(
        &self,
        session_id: &str,
    ) -> anyhow::Result<crate::host_workspace_types::WorkspaceBinding> {
        let mut map = self.workspace_bindings.lock().await;
        if let Some(b) = map.get(session_id) {
            return Ok(b.clone());
        }
        let svc = self.workspace_service().await;
        let session = crate::host_workspace_service::HostWorkspaceSession {
            session_id: session_id.to_string(),
            team_id: String::new(),
            workspace_path: None,
            source_channel_id: None,
            source_channel_name: None,
            source_thread_root_id: None,
        };
        let binding = svc.ensure_chat_workspace(&session)?;
        map.insert(session_id.to_string(), binding.clone());
        Ok(binding)
    }

    /// Compose a desktop [`crate::host_workspace::WorkspaceExecutionContext`]
    /// for `session_id` by ensuring the workspace, allocating a run, and
    /// converting the service's context into the rule-side struct used by
    /// `merge_workspace_execution_instruction` and `workspace_write_allowed`.
    ///
    /// The freshly-built context is cached on the session so the
    /// completion-mirror persist path can retrieve it via
    /// [`Self::cached_workspace_execution_context`] and reuse the same
    /// `run_id` the agent's prompt advertised for this turn.
    #[cfg(feature = "desktop_harness_host")]
    pub async fn workspace_execution_context_for_session(
        &self,
        session_id: &str,
        run_id: &str,
    ) -> anyhow::Result<crate::host_workspace::WorkspaceExecutionContext> {
        let binding = self.ensure_chat_workspace_binding(session_id).await?;
        let svc = self.workspace_service().await;
        let ctx = svc.execution_context(&binding, run_id)?;
        let rule_ctx = crate::host_workspace::WorkspaceExecutionContext {
            workspace_root: ctx.workspace_root,
            run_id: ctx.run_id,
            run_dir: ctx.run_dir,
            allowed_read_roots: ctx.allowed_read_roots,
            allowed_write_roots: ctx.allowed_write_roots,
            artifact_dirs: ctx.artifact_dirs,
            workspace_kind: crate::host_workspace::WorkspaceKind::Conversation,
        };
        {
            let mut cache = self.workspace_execution_contexts.lock().await;
            cache.insert(session_id.to_string(), rule_ctx.clone());
        }
        Ok(rule_ctx)
    }

    /// Snapshot the most recent `WorkspaceExecutionContext` cached for
    /// `session_id`, if any. Used by the completion-mirror persist path to
    /// reuse the same `run_id` the prompt advertised for this turn.
    #[cfg(feature = "desktop_harness_host")]
    pub async fn cached_workspace_execution_context(
        &self,
        session_id: &str,
    ) -> Option<crate::host_workspace::WorkspaceExecutionContext> {
        let cache = self.workspace_execution_contexts.lock().await;
        cache.get(session_id).cloned()
    }

    /// Lazy-init the process-wide
    /// [`crate::host_task_runtime::DesktopTaskRuntime`]. The runtime wraps a
    /// shared core [`agime::agents::harness::TaskRuntime`] and is reused
    /// across every `run_harness_host` invocation so sub-agent /
    /// swarm-worker / validation-worker events flow through a single
    /// in-process broker.
    #[cfg(feature = "desktop_harness_host")]
    pub async fn task_runtime(&self) -> Arc<crate::host_task_runtime::DesktopTaskRuntime> {
        let mut guard = self.task_runtime.lock().await;
        if let Some(rt) = guard.as_ref() {
            return rt.clone();
        }
        let rt = Arc::new(crate::host_task_runtime::DesktopTaskRuntime::new());
        *guard = Some(rt.clone());
        rt
    }
}
