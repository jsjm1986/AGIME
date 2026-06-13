//! JSON-file persistence for scheduled tasks.
//!
//! Layout:
//! - `data_dir()/scheduled_tasks/<task_id>.json`
//! - `data_dir()/scheduled_task_runs/<run_id>.json`
//!
//! Uses atomic writes (temp file + rename) following the `host_task_store.rs` pattern.

use crate::scheduled_tasks::models::{
    ScheduledTaskDeliveryTier, ScheduledTaskDoc, ScheduledTaskExecutionContract, ScheduledTaskKind,
    ScheduledTaskListView, ScheduledTaskPayloadKind, ScheduledTaskProfile,
    ScheduledTaskPublishBehavior, ScheduledTaskRunDoc, ScheduledTaskRunOutcomeReason,
    ScheduledTaskRunStatus, ScheduledTaskScheduleConfig, ScheduledTaskScheduleSpec,
    ScheduledTaskScheduleSpecKind, ScheduledTaskSessionBinding, ScheduledTaskStatus,
};
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use std::path::{Path, PathBuf};
use uuid::Uuid;

/// Service for managing scheduled tasks and runs via JSON files.
#[derive(Clone)]
pub struct ScheduledTaskService {
    tasks_dir: PathBuf,
    runs_dir: PathBuf,
}

impl ScheduledTaskService {
    /// Construct a service rooted at the given directories.
    /// Directories are created lazily on first access; a failure to create them
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
        let path = self.tasks_dir.join(format!("{task_id}.json"));
        if !path.exists() {
            anyhow::bail!("task not found: {}", task_id);
        }
        self.write_task(task_id, doc)?;
        Ok(())
    }

    /// Delete a task.
    pub fn delete_task(&self, task_id: &str) -> Result<()> {
        let path = self.tasks_dir.join(format!("{task_id}.json"));
        if path.exists() {
            std::fs::remove_file(&path).context(format!("delete scheduled task {task_id}"))?;
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
    pub fn get_run(&self, run_id: &str) -> Result<Option<ScheduledTaskRunDoc>> {
        let path = self.runs_dir.join(format!("{run_id}.json"));
        if !path.exists() {
            return Ok(None);
        }
        self.read_run(&path).map(Some)
    }

    /// Update run status.
    pub fn complete_run(
        &self,
        run_id: &str,
        status: ScheduledTaskRunStatus,
        outcome_reason: Option<ScheduledTaskRunOutcomeReason>,
    ) -> Result<()> {
        let path = self.runs_dir.join(format!("{run_id}.json"));
        if !path.exists() {
            anyhow::bail!("run not found: {}", run_id);
        }
        let mut doc = self.read_run(&path)?;
        doc.status = status;
        doc.outcome_reason = outcome_reason;
        doc.finished_at = Some(Utc::now());
        doc.updated_at = Utc::now();
        self.write_run(run_id, &doc)?;
        Ok(())
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

    /// Update a task's fire tracking after a run completes.
    pub fn record_task_run(
        &self,
        task_id: &str,
        run_id: String,
        next_fire_at: Option<DateTime<Utc>>,
        missed: bool,
    ) -> Result<()> {
        let path = self.tasks_dir.join(format!("{task_id}.json"));
        if !path.exists() {
            anyhow::bail!("task not found: {}", task_id);
        }
        let mut doc = self.read_task(&path)?;
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

    // ------------------------------------------------------------------------
    // File I/O helpers
    // ------------------------------------------------------------------------

    fn read_task(&self, path: &Path) -> Result<ScheduledTaskDoc> {
        let json =
            std::fs::read_to_string(path).context(format!("read task file {}", path.display()))?;
        serde_json::from_str(&json).context(format!("parse task file {}", path.display()))
    }

    fn write_task(&self, task_id: &str, doc: &ScheduledTaskDoc) -> Result<()> {
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

/// Task result from LLM execution.
pub struct TaskRunResult {
    pub summary: Option<String>,
    pub error: Option<String>,
}
