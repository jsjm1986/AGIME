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
///   `turn_system_instruction` / `runtime_overlay_text` /
///   `harness_delegation_overlay_text` / `surface_contract_overlay_text` —
///   pre-rendered overlay slots fed to
///   [`crate::host_prompt::compose_top_level_prompt`].
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
    /// if one existed.
    pub async fn set_session_override(
        &self,
        session_id: impl Into<String>,
        override_value: SessionOverride,
    ) -> Option<SessionOverride> {
        let mut map = self.session_overrides.lock().await;
        map.insert(session_id.into(), override_value)
    }

    /// Snapshot the current override entry for `session_id`, if any. Returns a
    /// clone so callers can hold the value without keeping the lock.
    pub async fn session_override(&self, session_id: &str) -> Option<SessionOverride> {
        let map = self.session_overrides.lock().await;
        map.get(session_id).cloned()
    }

    /// Drop the override entry for `session_id`. Returns the removed entry if
    /// one existed.
    pub async fn clear_session_override(&self, session_id: &str) -> Option<SessionOverride> {
        let mut map = self.session_overrides.lock().await;
        map.remove(session_id)
    }
}
