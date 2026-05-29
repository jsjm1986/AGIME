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
//! - Token usage and the `ContextRuntimeState` summary are appended to a
//!   per-session JSONL journal under `Paths::data_dir()/desktop_harness_journal/`
//!   so the desktop keeps a durable, greppable record of each turn's cost and
//!   compaction activity. This is the file-based stand-in for the team-server's
//!   Mongo `harness_journal` collection (the `desktop_harness_host` feature does
//!   not pull in sqlx), mirroring the shape of
//!   `crates/agime-team-server/src/agent/harness_persistence.rs` at commit
//!   961109f with the Mongo write replaced by a JSONL append.

use std::path::PathBuf;
use std::sync::Arc;

use agime::agents::harness::{HarnessMode, HarnessPersistenceAdapter};
use agime::config::paths::Paths;
use agime::context_runtime::{summarize_context_runtime_state, ContextRuntimeState};
use agime::conversation::Conversation;
use agime::session::SessionManager;
use anyhow::Result;
use async_trait::async_trait;
use chrono::Utc;
use serde::Serialize;

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
        context_runtime_state: Option<&ContextRuntimeState>,
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

        // Append a durable journal entry for this turn (token cost + context
        // runtime summary). Best-effort: a write failure logs at warn level
        // and never aborts the harness loop.
        write_journal_entry(
            logical_session_id,
            runtime_session_id,
            mode,
            context_runtime_state,
            total_tokens,
            input_tokens,
            output_tokens,
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

/// One line of the per-session harness journal. Serialized as a single JSON
/// object per line (JSONL) so entries can be appended without rewriting the
/// file and tailed/grepped without a parser.
#[derive(Debug, Serialize)]
struct JournalEntry {
    /// Unix epoch seconds when the turn finished.
    finished_at: i64,
    runtime_session_id: String,
    /// `HarnessMode` rendered via `Debug` (the enum is not `Serialize`).
    mode: String,
    total_tokens: Option<i32>,
    input_tokens: Option<i32>,
    output_tokens: Option<i32>,
    /// Compaction / context-runtime summary for this turn, when available.
    #[serde(skip_serializing_if = "Option::is_none")]
    context_runtime: Option<agime::context_runtime::ContextRuntimeStateSummary>,
}

fn journal_dir() -> PathBuf {
    Paths::data_dir().join("desktop_harness_journal")
}

/// Append a [`JournalEntry`] for a finished turn. Best-effort: every failure
/// (directory creation, serialization, append) is logged and swallowed so the
/// journal never affects the harness loop. The logical session id is sanitized
/// into the filename so an unusual id can't escape the journal directory.
fn write_journal_entry(
    logical_session_id: &str,
    runtime_session_id: &str,
    mode: HarnessMode,
    context_runtime_state: Option<&ContextRuntimeState>,
    total_tokens: Option<i32>,
    input_tokens: Option<i32>,
    output_tokens: Option<i32>,
) {
    use std::io::Write;

    let dir = journal_dir();
    if let Err(e) = std::fs::create_dir_all(&dir) {
        tracing::warn!(
            "DesktopHarnessPersistence: create journal dir failed: {}",
            e
        );
        return;
    }

    let safe: String = logical_session_id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let path = dir.join(format!("{}.jsonl", safe));

    let entry = JournalEntry {
        finished_at: Utc::now().timestamp(),
        runtime_session_id: runtime_session_id.to_string(),
        mode: format!("{:?}", mode),
        total_tokens,
        input_tokens,
        output_tokens,
        context_runtime: context_runtime_state.map(summarize_context_runtime_state),
    };

    let mut line = match serde_json::to_string(&entry) {
        Ok(line) => line,
        Err(e) => {
            tracing::warn!(
                "DesktopHarnessPersistence: serialize journal entry failed: {}",
                e
            );
            return;
        }
    };
    line.push('\n');

    match std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        Ok(mut file) => {
            if let Err(e) = file.write_all(line.as_bytes()) {
                tracing::warn!(
                    "DesktopHarnessPersistence: append journal {} failed: {}",
                    path.display(),
                    e
                );
            }
        }
        Err(e) => {
            tracing::warn!(
                "DesktopHarnessPersistence: open journal {} failed: {}",
                path.display(),
                e
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn journal_entry_serializes_to_one_line() {
        let entry = JournalEntry {
            finished_at: 1700000000,
            runtime_session_id: "rt-1".to_string(),
            mode: "Conversation".to_string(),
            total_tokens: Some(120),
            input_tokens: Some(100),
            output_tokens: Some(20),
            context_runtime: None,
        };
        let line = serde_json::to_string(&entry).unwrap();
        assert!(!line.contains('\n'));
        assert!(line.contains("\"runtime_session_id\":\"rt-1\""));
        assert!(line.contains("\"total_tokens\":120"));
        // `context_runtime` is skipped when None.
        assert!(!line.contains("context_runtime"));
    }
}
