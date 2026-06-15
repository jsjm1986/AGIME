//! JSON-file persistence for scheduled tasks.
//!
//! Layout:
//! - `data_dir()/scheduled_tasks/<task_id>.json`
//! - `data_dir()/scheduled_task_runs/<run_id>.json`
//!
//! Uses atomic writes (temp file + rename) following the `host_task_store.rs` pattern.

use crate::scheduled_tasks::models::{
    ScheduledTaskDoc, ScheduledTaskRunDoc, ScheduledTaskRunOutcomeReason, ScheduledTaskRunStatus,
    ScheduledTaskStatus,
};
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use std::path::{Path, PathBuf};
use uuid::Uuid;

/// Maximum wall-clock age a run may stay in `Running` before it is presumed
/// dead. Must exceed the longest realistic task execution; a crashed or
/// never-finalized run older than this is reclaimed to `Failed` so its task
/// isn't blocked forever. Set well above the scheduler lease (600s).
const MAX_RUN_AGE_SECS: i64 = 3600;

/// Service for managing scheduled tasks and runs via JSON files.
#[derive(Clone)]
pub struct ScheduledTaskService {
    tasks_dir: PathBuf,
    runs_dir: PathBuf,
}

/// Validate an id used to build a JSON filename, rejecting path-traversal
/// and separator characters. Task/run ids are UUIDs in normal operation;
/// anything containing a path separator, `..`, or `%` is rejected so a
/// crafted id can't escape the tasks/runs directories.
fn validate_id(id: &str) -> Result<()> {
    if id.is_empty() {
        anyhow::bail!("id must not be empty");
    }
    if id.contains('/')
        || id.contains('\\')
        || id.contains("..")
        || id.contains('%')
        || id.contains('\0')
    {
        anyhow::bail!("invalid id: {}", id);
    }
    Ok(())
}

impl ScheduledTaskService {
    /// Construct a service rooted at the given directories.
    /// Directories are created at construction; a failure to create them
    /// results in an empty/inoperative store so startup never hard-fails.
    pub fn new(tasks_dir: PathBuf, runs_dir: PathBuf) -> Self {
        let _ = std::fs::create_dir_all(&tasks_dir);
        let _ = std::fs::create_dir_all(&runs_dir);
        Self {
            tasks_dir,
            runs_dir,
        }
    }

    /// List tasks with optional ownership filter.
    pub fn list_tasks(&self) -> Result<Vec<ScheduledTaskDoc>> {
        let mut tasks = Vec::new();
        if !self.tasks_dir.exists() {
            std::fs::create_dir_all(&self.tasks_dir)?;
            return Ok(tasks);
        }
        for entry in std::fs::read_dir(&self.tasks_dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("json") {
                continue;
            }
            match self.read_task(&path) {
                Ok(task) => tasks.push(task),
                Err(e) => {
                    eprintln!("WARN: failed to read scheduled task {:?}: {}", path, e);
                }
            }
        }
        tasks.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        Ok(tasks)
    }

    /// Get a single task by ID.
    pub fn get_task(&self, task_id: &str) -> Result<Option<ScheduledTaskDoc>> {
        validate_id(task_id)?;
        let path = self.tasks_dir.join(format!("{task_id}.json"));
        if !path.exists() {
            return Ok(None);
        }
        self.read_task(&path).map(Some)
    }

    /// Create a new task and write it to disk.
    pub fn create_task(&self, mut doc: ScheduledTaskDoc) -> Result<ScheduledTaskDoc> {
        if doc.task_id.is_empty() {
            doc.task_id = Uuid::new_v4().to_string();
        }
        doc.created_at = Utc::now();
        doc.updated_at = Utc::now();
        self.write_task(&doc.task_id, &doc)?;
        Ok(doc)
    }

    /// Update an existing task.
    pub fn update_task(&self, task_id: &str, doc: &ScheduledTaskDoc) -> Result<()> {
        validate_id(task_id)?;
        let path = self.tasks_dir.join(format!("{task_id}.json"));
        if !path.exists() {
            anyhow::bail!("task not found: {}", task_id);
        }
        self.write_task(task_id, doc)?;
        Ok(())
    }

    /// Delete a task and all of its run records. Orphaned run files would
    /// otherwise accumulate on disk forever, since runs are keyed by run_id and
    /// nothing else references them once the task is gone.
    pub fn delete_task(&self, task_id: &str) -> Result<()> {
        validate_id(task_id)?;
        let path = self.tasks_dir.join(format!("{task_id}.json"));
        if path.exists() {
            std::fs::remove_file(&path).context(format!("delete scheduled task {task_id}"))?;
        }
        for run in self.list_runs(task_id)? {
            let run_path = self.runs_dir.join(format!("{}.json", run.run_id));
            if run_path.exists() {
                if let Err(e) = std::fs::remove_file(&run_path) {
                    eprintln!(
                        "WARN: failed to delete run {} for task {}: {}",
                        run.run_id, task_id, e
                    );
                }
            }
        }
        Ok(())
    }

    /// Claim tasks that are due for execution (lease-based concurrency).
    pub fn claim_due_tasks(
        &self,
        lease_owner: &str,
        lease_secs: i64,
    ) -> Result<Vec<ScheduledTaskDoc>> {
        let all = self.list_tasks()?;
        let now = Utc::now();
        let deadline = now + chrono::Duration::seconds(lease_secs);
        let mut claimed = Vec::new();

        for mut task in all {
            if task.status != ScheduledTaskStatus::Active {
                continue;
            }
            if task.next_fire_at.map(|t| t > now).unwrap_or(true) {
                continue;
            }
            let expired = task.lease_expires_at.map(|e| e <= now).unwrap_or(true);
            if !expired {
                continue;
            }
            // Even with an expired lease, skip a task that still has an
            // unfinished run: a long execution (longer than the lease) is still
            // in flight, and re-claiming it here would spawn a duplicate run.
            if self.has_running_run(&task.task_id).unwrap_or(false) {
                continue;
            }
            task.lease_owner = Some(lease_owner.to_string());
            task.lease_expires_at = Some(deadline);
            if let Err(e) = self.write_task(&task.task_id, &task) {
                eprintln!("WARN: failed to claim task {}: {}", task.task_id, e);
                continue;
            }
            claimed.push(task);
        }
        Ok(claimed)
    }

    /// Release a task's lease.
    #[allow(dead_code)]
    pub fn release_task(&self, task_id: &str, _lease_owner: &str) -> Result<()> {
        let path = self.tasks_dir.join(format!("{task_id}.json"));
        if !path.exists() {
            return Ok(());
        }
        let mut doc = self.read_task(&path)?;
        doc.lease_owner = None;
        doc.lease_expires_at = None;
        doc.updated_at = Utc::now();
        self.write_task(task_id, &doc)?;
        Ok(())
    }

    /// Create a new run record.
    pub fn create_run(&self, mut run: ScheduledTaskRunDoc) -> Result<ScheduledTaskRunDoc> {
        if run.run_id.is_empty() {
            run.run_id = Uuid::new_v4().to_string();
        }
        run.created_at = Utc::now();
        run.updated_at = Utc::now();
        self.write_run(&run.run_id, &run)?;
        Ok(run)
    }

    /// Get a single run by ID.
    #[allow(dead_code)]
    pub fn get_run(&self, run_id: &str) -> Result<Option<ScheduledTaskRunDoc>> {
        let path = self.runs_dir.join(format!("{run_id}.json"));
        if !path.exists() {
            return Ok(None);
        }
        self.read_run(&path).map(Some)
    }

    /// List all runs for a task.
    pub fn list_runs(&self, task_id: &str) -> Result<Vec<ScheduledTaskRunDoc>> {
        let mut runs = Vec::new();
        if !self.runs_dir.exists() {
            return Ok(runs);
        }
        for entry in std::fs::read_dir(&self.runs_dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("json") {
                continue;
            }
            match self.read_run(&path) {
                Ok(run) if run.task_id == task_id => runs.push(run),
                Ok(_) => {}
                Err(e) => {
                    eprintln!("WARN: failed to read run {:?}: {}", path, e);
                }
            }
        }
        runs.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        Ok(runs)
    }

    /// Whether the task currently has at least one *live* run in `Running`
    /// state. A run whose `started_at` is older than [`MAX_RUN_AGE_SECS`] is
    /// presumed dead (process crash, or a `finish_run` that failed to persist)
    /// and is ignored — otherwise a stale `Running` record would block the task
    /// from ever being claimed again, deadlocking it permanently across
    /// restarts. Stale records are transitioned to `Failed` by
    /// [`Self::reclaim_stale_runs`].
    pub fn has_running_run(&self, task_id: &str) -> Result<bool> {
        let cutoff = Utc::now() - chrono::Duration::seconds(MAX_RUN_AGE_SECS);
        Ok(self
            .list_runs(task_id)?
            .iter()
            .any(|run| run.status == ScheduledTaskRunStatus::Running && run.started_at > cutoff))
    }

    /// Reconcile orphaned runs: any run still `Running` past
    /// [`MAX_RUN_AGE_SECS`] is presumed dead and transitioned to `Failed`.
    /// Returns the number of runs reclaimed. Called once per scheduler tick so
    /// crashed/never-finished runs don't accumulate or block their task.
    pub fn reclaim_stale_runs(&self) -> Result<usize> {
        let cutoff = Utc::now() - chrono::Duration::seconds(MAX_RUN_AGE_SECS);
        let mut reclaimed = 0;
        if !self.runs_dir.exists() {
            return Ok(0);
        }
        for entry in std::fs::read_dir(&self.runs_dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("json") {
                continue;
            }
            let run = match self.read_run(&path) {
                Ok(run) => run,
                Err(_) => continue,
            };
            if run.status == ScheduledTaskRunStatus::Running && run.started_at <= cutoff {
                if let Err(e) = self.finish_run(
                    &run.run_id,
                    ScheduledTaskRunStatus::Failed,
                    Some(ScheduledTaskRunOutcomeReason::FailedNoFinalAnswer),
                    None,
                    Some(
                        "run exceeded max age; presumed dead (crash or persistence failure)"
                            .to_string(),
                    ),
                ) {
                    eprintln!("WARN: failed to reclaim stale run {}: {}", run.run_id, e);
                    continue;
                }
                reclaimed += 1;
            }
        }
        Ok(reclaimed)
    }

    /// Update a task's fire tracking after a run completes (cron roll-forward).
    /// Only mutates the task if `lease_owner` still holds the lease, so a run
    /// whose lease expired and was re-claimed by another scheduler instance
    /// can't clobber the new owner's state.
    pub fn record_task_run(
        &self,
        task_id: &str,
        run_id: String,
        next_fire_at: Option<DateTime<Utc>>,
        missed: bool,
        lease_owner: &str,
    ) -> Result<()> {
        validate_id(task_id)?;
        let path = self.tasks_dir.join(format!("{task_id}.json"));
        if !path.exists() {
            anyhow::bail!("task not found: {}", task_id);
        }
        let mut doc = self.read_task(&path)?;
        if !Self::lease_held_by(&doc, lease_owner) {
            return Ok(());
        }
        doc.last_run_id = Some(run_id);
        doc.last_fire_at = Some(Utc::now());
        doc.lease_owner = None;
        doc.lease_expires_at = None;
        if missed {
            doc.missed_fire_count += 1;
            doc.last_missed_at = Some(Utc::now());
        }
        if let Some(next) = next_fire_at {
            doc.next_fire_at = Some(next);
        }
        doc.updated_at = Utc::now();
        self.write_task(task_id, &doc)?;
        Ok(())
    }

    /// Terminal-state a one-shot task so it never re-fires: mark Completed,
    /// clear the lease and `next_fire_at`.
    ///
    /// Unlike the cron roll-forward, this is NOT gated on lease ownership: a
    /// one-shot task must reach its terminal `Completed` state regardless of
    /// which owner's run finished it. Gating on the lease would leave a task
    /// whose lease expired mid-run stuck `Active` with a past `next_fire_at`,
    /// causing it to be re-claimed and re-run on every tick. `Completed` is a
    /// terminal state, so forcing it can't clobber a meaningful newer state.
    pub fn complete_task(&self, task_id: &str, run_id: &str, _lease_owner: &str) -> Result<()> {
        validate_id(task_id)?;
        let path = self.tasks_dir.join(format!("{task_id}.json"));
        if !path.exists() {
            anyhow::bail!("task not found: {}", task_id);
        }
        let mut doc = self.read_task(&path)?;
        doc.status = ScheduledTaskStatus::Completed;
        doc.last_run_id = Some(run_id.to_string());
        doc.last_fire_at = Some(Utc::now());
        doc.next_fire_at = None;
        doc.lease_owner = None;
        doc.lease_expires_at = None;
        doc.updated_at = Utc::now();
        self.write_task(task_id, &doc)?;
        Ok(())
    }

    /// Pause a task whose next fire time can't be computed (e.g. an
    /// unparseable cron), clearing the lease and `next_fire_at` so it stops
    /// being re-claimed every tick. Guarded by lease ownership.
    pub fn pause_stalled_task(&self, task_id: &str, run_id: &str, lease_owner: &str) -> Result<()> {
        validate_id(task_id)?;
        let path = self.tasks_dir.join(format!("{task_id}.json"));
        if !path.exists() {
            anyhow::bail!("task not found: {}", task_id);
        }
        let mut doc = self.read_task(&path)?;
        if !Self::lease_held_by(&doc, lease_owner) {
            return Ok(());
        }
        doc.status = ScheduledTaskStatus::Paused;
        doc.last_run_id = Some(run_id.to_string());
        doc.last_fire_at = Some(Utc::now());
        doc.next_fire_at = None;
        doc.lease_owner = None;
        doc.lease_expires_at = None;
        doc.updated_at = Utc::now();
        self.write_task(task_id, &doc)?;
        Ok(())
    }

    /// Persist a run's terminal state (status, outcome, summary, error).
    pub fn finish_run(
        &self,
        run_id: &str,
        status: ScheduledTaskRunStatus,
        outcome_reason: Option<ScheduledTaskRunOutcomeReason>,
        summary: Option<String>,
        error: Option<String>,
    ) -> Result<()> {
        validate_id(run_id)?;
        let path = self.runs_dir.join(format!("{run_id}.json"));
        if !path.exists() {
            anyhow::bail!("run not found: {}", run_id);
        }
        let mut doc = self.read_run(&path)?;
        doc.status = status;
        doc.outcome_reason = outcome_reason;
        doc.summary = summary;
        doc.error = error;
        doc.finished_at = Some(Utc::now());
        doc.updated_at = Utc::now();
        self.write_run(run_id, &doc)?;
        Ok(())
    }

    /// Whether `lease_owner` currently holds the task's lease. A task with no
    /// recorded owner is treated as held (legacy/manual paths that don't set
    /// a lease still advance), but a mismatched owner is rejected.
    fn lease_held_by(doc: &ScheduledTaskDoc, lease_owner: &str) -> bool {
        match doc.lease_owner.as_deref() {
            Some(owner) => owner == lease_owner,
            None => true,
        }
    }

    // ------------------------------------------------------------------------
    // File I/O helpers
    // ------------------------------------------------------------------------

    fn read_task(&self, path: &Path) -> Result<ScheduledTaskDoc> {
        let json =
            std::fs::read_to_string(path).context(format!("read task file {}", path.display()))?;
        serde_json::from_str(&json).context(format!("parse task file {}", path.display()))
    }

    fn write_task(&self, task_id: &str, doc: &ScheduledTaskDoc) -> Result<()> {
        validate_id(task_id)?;
        let path = self.tasks_dir.join(format!("{task_id}.json"));
        let tmp = path.with_extension("json.tmp");
        let json =
            serde_json::to_string_pretty(doc).context(format!("serialize task {}", task_id))?;
        std::fs::write(&tmp, &json).context(format!("write temp task file {}", tmp.display()))?;
        std::fs::rename(&tmp, &path).context(format!("commit task file {}", path.display()))?;
        Ok(())
    }

    fn read_run(&self, path: &Path) -> Result<ScheduledTaskRunDoc> {
        let json =
            std::fs::read_to_string(path).context(format!("read run file {}", path.display()))?;
        serde_json::from_str(&json).context(format!("parse run file {}", path.display()))
    }

    fn write_run(&self, run_id: &str, run: &ScheduledTaskRunDoc) -> Result<()> {
        validate_id(run_id)?;
        std::fs::create_dir_all(&self.runs_dir)?;
        let path = self.runs_dir.join(format!("{run_id}.json"));
        let tmp = path.with_extension("json.tmp");
        let json =
            serde_json::to_string_pretty(run).context(format!("serialize run {}", run_id))?;
        std::fs::write(&tmp, &json).context(format!("write temp run file {}", tmp.display()))?;
        std::fs::rename(&tmp, &path).context(format!("commit run file {}", path.display()))?;
        Ok(())
    }
}
