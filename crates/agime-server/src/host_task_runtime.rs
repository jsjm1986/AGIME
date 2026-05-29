//! Desktop wrapper around the core [`agime::agents::harness::TaskRuntime`].
//!
//! The core crate already ships an in-memory [`TaskRuntime`] backed by
//! `DashMap` + `tokio::sync::broadcast`, which is exactly what the
//! desktop server needs for sub-agent / swarm-worker / validation-worker
//! orchestration during a single chat turn. This module exposes a thin
//! lazy-initialised wrapper on top of it so that:
//!
//! - The same runtime is shared across every [`run_harness_host`] call
//!   for the lifetime of the desktop process (matching team-server
//!   semantics).
//! - Future SQLite-backed persistence (long-running task durability,
//!   `GET /tasks/:id/stream` attach, cross-process resume) plugs in here
//!   without changing the call sites in `routes/reply.rs`.
//!
//! [`run_harness_host`]: agime::agents::harness::run_harness_host

use std::sync::Arc;

use agime::agents::harness::TaskRuntime;

/// Process-wide handle to the desktop's [`TaskRuntime`].
#[derive(Clone)]
pub struct DesktopTaskRuntime {
    inner: Arc<TaskRuntime>,
}

impl DesktopTaskRuntime {
    pub fn new() -> Self {
        Self {
            inner: TaskRuntime::shared(),
        }
    }

    /// Return the shared core runtime so callers can pass it as
    /// [`agime::agents::harness::HarnessHostRequest::task_runtime`].
    pub fn inner(&self) -> Arc<TaskRuntime> {
        self.inner.clone()
    }

    /// Returns whether any sub-agent task is still active for the given
    /// parent (chat) session. Useful as an early signal for the desktop
    /// to disable the "send" button while a turn is mid-flight.
    pub fn has_active_tasks(&self, parent_session_id: &str) -> bool {
        self.inner
            .has_active_tasks_for_parent_session(parent_session_id)
    }
}

impl Default for DesktopTaskRuntime {
    fn default() -> Self {
        Self::new()
    }
}
