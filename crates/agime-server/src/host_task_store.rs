//! File-based durability for the desktop user-level task manager.
//!
//! The `desktop_harness_host` cargo feature does **not** pull in `sqlx`
//! (that dependency is gated behind the separate `team` feature), so the
//! desktop honors its dual-track independence by persisting tasks as plain
//! JSON files instead of a SQLite table. One file per task keeps writes
//! atomic-per-task and avoids a global rewrite on every event.
//!
//! Layout: `Paths::data_dir()/desktop_tasks/<task_id>.json`
//!
//! Each file holds a [`PersistedTask`]: the latest [`UserTaskSnapshot`] plus
//! the original user [`Message`] so an interrupted task can be replayed on the
//! next server start (see [`DesktopTaskStore::load_all`] and the manager's
//! `resume_queued_tasks`). Live event streams are **not** persisted — only the
//! durable snapshot is. A client re-attaching after a restart gets the
//! snapshot (carrying the terminal status) rather than a replayed event log.

use std::path::{Path, PathBuf};

use agime::config::paths::Paths;
use agime::conversation::message::Message;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::host_task_manager::UserTaskSnapshot;

/// The on-disk record for a single task: its latest snapshot plus the original
/// prompt message needed to re-drive an interrupted run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistedTask {
    pub snapshot: UserTaskSnapshot,
    pub user_message: Message,
}

/// File-backed task store. Cheap to clone (just holds the root path).
#[derive(Clone)]
pub struct DesktopTaskStore {
    root: PathBuf,
}

impl DesktopTaskStore {
    /// Construct a store rooted at `Paths::data_dir()/desktop_tasks`,
    /// creating the directory if needed. A failure to create the directory is
    /// returned to the caller, which downgrades to a no-op (in-memory only)
    /// store so task submission never hard-fails on a read-only data dir.
    pub fn new() -> Result<Self> {
        let root = Paths::data_dir().join("desktop_tasks");
        std::fs::create_dir_all(&root)
            .with_context(|| format!("create desktop task store dir {}", root.display()))?;
        Ok(Self { root })
    }

    /// Construct a store rooted at an explicit path (no implicit `data_dir`
    /// lookup). Primarily for tests that want an isolated temp directory; the
    /// directory is expected to already exist.
    #[cfg(test)]
    pub fn new_at(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    fn path_for(&self, task_id: &str) -> PathBuf {
        // Task ids are server-generated `task-<uuid>` strings, so they are
        // already filesystem-safe; sanitize defensively anyway in case a
        // future id scheme allows separators.
        let safe: String = task_id
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        self.root.join(format!("{}.json", safe))
    }

    /// Persist (create or overwrite) a task record. Written via a temp file +
    /// rename so a crash mid-write never leaves a half-written JSON file that
    /// would poison `load_all` on the next start.
    pub fn save(&self, record: &PersistedTask) -> Result<()> {
        let path = self.path_for(&record.snapshot.task_id);
        let tmp = path.with_extension("json.tmp");
        let json = serde_json::to_vec_pretty(record).context("serialize persisted task")?;
        std::fs::write(&tmp, &json)
            .with_context(|| format!("write temp task file {}", tmp.display()))?;
        std::fs::rename(&tmp, &path)
            .with_context(|| format!("commit task file {}", path.display()))?;
        Ok(())
    }

    /// Remove a task's persisted file. Missing files are not an error.
    #[allow(dead_code)]
    pub fn remove(&self, task_id: &str) -> Result<()> {
        let path = self.path_for(task_id);
        match std::fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e).with_context(|| format!("remove task file {}", path.display())),
        }
    }

    /// Load every persisted task. Files that fail to parse are skipped with a
    /// warning rather than aborting the whole load — one corrupt record should
    /// not block resuming the rest.
    pub fn load_all(&self) -> Result<Vec<PersistedTask>> {
        load_all_from(&self.root)
    }
}

fn load_all_from(root: &Path) -> Result<Vec<PersistedTask>> {
    let mut out = Vec::new();
    let entries = match std::fs::read_dir(root) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(out),
        Err(e) => return Err(e).with_context(|| format!("read task dir {}", root.display())),
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        match std::fs::read(&path) {
            Ok(bytes) => match serde_json::from_slice::<PersistedTask>(&bytes) {
                Ok(record) => out.push(record),
                Err(e) => {
                    tracing::warn!("skipping unparsable task file {}: {}", path.display(), e);
                }
            },
            Err(e) => {
                tracing::warn!("skipping unreadable task file {}: {}", path.display(), e);
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host_task_manager::UserTaskStatus;

    fn snapshot(task_id: &str, status: UserTaskStatus) -> UserTaskSnapshot {
        UserTaskSnapshot {
            task_id: task_id.to_string(),
            session_id: "s1".to_string(),
            status,
            prompt_preview: "hello".to_string(),
            last_event_id: 0,
            error: None,
            created_at: 100,
            updated_at: 100,
            finished_at: None,
        }
    }

    #[test]
    fn save_load_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let store = DesktopTaskStore {
            root: dir.path().to_path_buf(),
        };
        let record = PersistedTask {
            snapshot: snapshot("task-1", UserTaskStatus::Pending),
            user_message: Message::user().with_text("hello"),
        };
        store.save(&record).unwrap();

        let loaded = store.load_all().unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].snapshot.task_id, "task-1");
        assert_eq!(loaded[0].snapshot.status, UserTaskStatus::Pending);
    }

    #[test]
    fn overwrite_updates_record() {
        let dir = tempfile::tempdir().unwrap();
        let store = DesktopTaskStore {
            root: dir.path().to_path_buf(),
        };
        let mut record = PersistedTask {
            snapshot: snapshot("task-2", UserTaskStatus::Pending),
            user_message: Message::user().with_text("hello"),
        };
        store.save(&record).unwrap();
        record.snapshot.status = UserTaskStatus::Completed;
        store.save(&record).unwrap();

        let loaded = store.load_all().unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].snapshot.status, UserTaskStatus::Completed);
    }

    #[test]
    fn remove_deletes_file() {
        let dir = tempfile::tempdir().unwrap();
        let store = DesktopTaskStore {
            root: dir.path().to_path_buf(),
        };
        let record = PersistedTask {
            snapshot: snapshot("task-3", UserTaskStatus::Pending),
            user_message: Message::user().with_text("hi"),
        };
        store.save(&record).unwrap();
        store.remove("task-3").unwrap();
        assert!(store.load_all().unwrap().is_empty());
        // Removing a missing file is a no-op success.
        store.remove("task-3").unwrap();
    }

    #[test]
    fn corrupt_file_is_skipped() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("garbage.json"), b"not json").unwrap();
        let good = PersistedTask {
            snapshot: snapshot("task-4", UserTaskStatus::Pending),
            user_message: Message::user().with_text("ok"),
        };
        let store = DesktopTaskStore {
            root: dir.path().to_path_buf(),
        };
        store.save(&good).unwrap();
        let loaded = store.load_all().unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].snapshot.task_id, "task-4");
    }
}
