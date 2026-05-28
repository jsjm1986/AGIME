//! Desktop implementation of [`agime::agents::harness::HarnessPersistenceAdapter`].
//!
//! `run_harness_host` creates a fresh runtime session per turn and writes the
//! conversation into that *runtime* session file. The desktop chat surface,
//! however, expects the **logical** session file (the long-lived
//! `session_id` users see in the UI) to keep accumulating messages.
//!
//! The desktop adapter bridges the two:
//!
//! - `on_started` is informational only.
//! - `on_finished` mirrors the harness's `final_conversation` back into the
//!   logical session via [`SessionManager::replace_conversation`] so the
//!   desktop UI continues to load the full transcript on reload.
//! - It also reconciles the workspace manifest so any artifacts produced
//!   during this turn are tracked even if the SSE-side
//!   `Completion.StructuredPublished` interceptor missed them.
//!
//! Token usage and `ContextRuntimeState` are logged but not yet routed to a
//! dedicated SQLite store. When the desktop adds a `desktop_harness_journal`
//! SQLite table (Milestone C, follow-up), the inserts will land here. The
//! shape will mirror `crates/agime-team-server/src/agent/harness_persistence.rs`
//! at commit 961109f, with Mongo writes replaced by sqlx upserts.

use std::sync::Arc;

use agime::agents::harness::{HarnessMode, HarnessPersistenceAdapter};
use agime::context_runtime::ContextRuntimeState;
use agime::conversation::Conversation;
use agime::session::SessionManager;
use anyhow::Result;
use async_trait::async_trait;

use crate::state::AppState;

#[derive(Clone)]
pub struct DesktopHarnessPersistence {
    state: Arc<AppState>,
}

impl DesktopHarnessPersistence {
    pub fn new(state: Arc<AppState>) -> Self {
        Self { state }
    }
}

#[async_trait]
impl HarnessPersistenceAdapter for DesktopHarnessPersistence {
    async fn on_started(
        &self,
        logical_session_id: &str,
        runtime_session_id: &str,
        _initial_conversation: &Conversation,
        mode: HarnessMode,
    ) -> Result<()> {
        tracing::info!(
            logical_session_id,
            runtime_session_id,
            ?mode,
            "DesktopHarnessPersistence: harness session started"
        );
        Ok(())
    }

    async fn on_finished(
        &self,
        logical_session_id: &str,
        runtime_session_id: &str,
        final_conversation: &Conversation,
        mode: HarnessMode,
        _context_runtime_state: Option<&ContextRuntimeState>,
        total_tokens: Option<i32>,
        input_tokens: Option<i32>,
        output_tokens: Option<i32>,
    ) -> Result<()> {
        tracing::info!(
            logical_session_id,
            runtime_session_id,
            ?mode,
            total_tokens,
            input_tokens,
            output_tokens,
            "DesktopHarnessPersistence: harness session finished"
        );

        // Mirror the harness's final conversation back to the logical
        // (desktop-visible) session file. Best-effort — a write failure here
        // logs at warn level but does not abort the harness loop.
        if let Err(err) =
            SessionManager::replace_conversation(logical_session_id, final_conversation).await
        {
            tracing::warn!(
                "DesktopHarnessPersistence: failed to mirror conversation to logical session {}: {}",
                logical_session_id,
                err
            );
        }

        // Reconcile the workspace manifest so any artifacts produced
        // during this turn are picked up even if the SSE mirror missed a
        // Completion.StructuredPublished envelope. Best-effort: any
        // failure is logged at debug level and never propagated.
        if let Ok(binding) = self
            .state
            .ensure_chat_workspace_binding(logical_session_id)
            .await
        {
            let svc = self.state.workspace_service().await;
            if let Err(err) = svc.reconcile_manifest_artifacts(&binding) {
                tracing::debug!(
                    "DesktopHarnessPersistence: reconcile_manifest_artifacts failed for {}: {}",
                    logical_session_id,
                    err
                );
            }
        }
        Ok(())
    }
}
