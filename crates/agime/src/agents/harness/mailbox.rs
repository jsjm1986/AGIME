use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::Utc;
use fs2::FileExt;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub struct MailboxLineage {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub logical_worker_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attempt_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attempt_index: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_task_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MailboxMessageKind {
    Directive,
    Progress,
    Summary,
    PeerMessage,
    PeerReply,
    PermissionRequest,
    PermissionResolution,
    FollowupRequest,
    IdleNotification,
    ShutdownNotification,
    PlanControl,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SwarmProtocolKind {
    Directive,
    Progress,
    Summary,
    PeerMessage,
    PeerReply,
    PermissionRequest,
    PermissionResolution,
    FollowupRequest,
    IdleNotification,
    ShutdownNotification,
    PlanControl,
}

impl Default for SwarmProtocolKind {
    fn default() -> Self {
        Self::PeerMessage
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SwarmProtocolMessage {
    pub kind: SwarmProtocolKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub directive_kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason_code: Option<String>,
    #[serde(default)]
    pub target_artifacts: Vec<String>,
    #[serde(default)]
    pub referenced_task_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lineage: Option<MailboxLineage>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MailboxMessage {
    pub id: String,
    pub from: String,
    pub to: String,
    pub kind: MailboxMessageKind,
    pub text: String,
    pub summary: Option<String>,
    pub timestamp: String,
    pub read: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub protocol: Option<SwarmProtocolMessage>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub metadata: HashMap<String, String>,
}

fn root_dir(working_dir: &Path, run_id: &str) -> PathBuf {
    working_dir.join(".agime").join("swarm").join(run_id)
}

fn mailbox_path_from_root(mailbox_root: &Path, worker_name: &str) -> PathBuf {
    mailbox_root.join(format!("{}.json", worker_name))
}

pub fn mailbox_dir(working_dir: &Path, run_id: &str) -> PathBuf {
    root_dir(working_dir, run_id).join("mailboxes")
}

pub fn mailbox_path(working_dir: &Path, run_id: &str, worker_name: &str) -> PathBuf {
    mailbox_dir(working_dir, run_id).join(format!("{}.json", worker_name))
}

pub fn read_unread_messages_from_root(
    mailbox_root: &Path,
    worker_name: &str,
) -> Result<Vec<MailboxMessage>> {
    let path = mailbox_path_from_root(mailbox_root, worker_name);
    with_locked_messages(&path, |messages| {
        Ok(messages
            .iter()
            .filter(|message| !message.read)
            .cloned()
            .collect())
    })
}

pub fn drain_unread_messages_from_root(
    mailbox_root: &Path,
    worker_name: &str,
) -> Result<Vec<MailboxMessage>> {
    let path = mailbox_path_from_root(mailbox_root, worker_name);
    with_locked_messages(&path, |messages| {
        let unread = messages
            .iter()
            .filter(|message| !message.read)
            .cloned()
            .collect::<Vec<_>>();
        for message in messages.iter_mut() {
            if !message.read {
                message.read = true;
            }
        }
        Ok(unread)
    })
}

fn ensure_mailbox(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create mailbox dir {}", parent.display()))?;
    }
    if !path.exists() {
        fs::write(path, "[]")
            .with_context(|| format!("failed to initialize mailbox {}", path.display()))?;
    }
    Ok(())
}

fn with_locked_messages<T>(
    path: &Path,
    mut f: impl FnMut(&mut Vec<MailboxMessage>) -> Result<T>,
) -> Result<T> {
    ensure_mailbox(path)?;
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .with_context(|| format!("failed to open mailbox {}", path.display()))?;
    file.lock_exclusive()
        .with_context(|| format!("failed to lock mailbox {}", path.display()))?;

    let result = (|| {
        let mut raw = String::new();
        file.read_to_string(&mut raw)?;
        let mut messages: Vec<MailboxMessage> = if raw.trim().is_empty() {
            Vec::new()
        } else {
            serde_json::from_str(&raw)
                .with_context(|| format!("failed to parse mailbox {}", path.display()))?
        };
        let output = f(&mut messages)?;
        file.set_len(0)?;
        file.seek(SeekFrom::Start(0))?;
        file.write_all(serde_json::to_string_pretty(&messages)?.as_bytes())?;
        file.flush()?;
        Ok(output)
    })();

    let unlock_result = file.unlock();
    match (result, unlock_result) {
        (Ok(output), Ok(())) => Ok(output),
        (Ok(_), Err(error)) => Err(error).with_context(|| "failed to unlock mailbox"),
        (Err(error), _) => Err(error),
    }
}

pub fn read_mailbox(
    working_dir: &Path,
    run_id: &str,
    worker_name: &str,
) -> Result<Vec<MailboxMessage>> {
    let path = mailbox_path(working_dir, run_id, worker_name);
    with_locked_messages(&path, |messages| Ok(messages.clone()))
}

pub fn read_unread_messages(
    working_dir: &Path,
    run_id: &str,
    worker_name: &str,
) -> Result<Vec<MailboxMessage>> {
    let path = mailbox_path(working_dir, run_id, worker_name);
    with_locked_messages(&path, |messages| {
        Ok(messages
            .iter()
            .filter(|message| !message.read)
            .cloned()
            .collect())
    })
}

pub fn write_to_mailbox(
    working_dir: &Path,
    run_id: &str,
    worker_name: &str,
    mut message: MailboxMessage,
) -> Result<()> {
    let path = mailbox_path(working_dir, run_id, worker_name);
    with_locked_messages(&path, |messages| {
        if message.timestamp.trim().is_empty() {
            message.timestamp = Utc::now().to_rfc3339();
        }
        messages.push(message.clone());
        Ok(())
    })
}

pub fn write_to_mailbox_from_root(
    mailbox_root: &Path,
    worker_name: &str,
    mut message: MailboxMessage,
) -> Result<()> {
    let path = mailbox_path_from_root(mailbox_root, worker_name);
    with_locked_messages(&path, |messages| {
        if message.timestamp.trim().is_empty() {
            message.timestamp = Utc::now().to_rfc3339();
        }
        messages.push(message.clone());
        Ok(())
    })
}

pub fn mark_all_read(working_dir: &Path, run_id: &str, worker_name: &str) -> Result<()> {
    let path = mailbox_path(working_dir, run_id, worker_name);
    with_locked_messages(&path, |messages| {
        for message in messages.iter_mut() {
            message.read = true;
        }
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mailbox_round_trip_preserves_unread_messages() {
        let root = tempfile::tempdir().expect("tempdir");
        let working_dir = root.path().to_path_buf();
        write_to_mailbox(
            &working_dir,
            "run-1",
            "leader",
            MailboxMessage {
                id: "msg-1".to_string(),
                from: "worker-1".to_string(),
                to: "leader".to_string(),
                kind: MailboxMessageKind::Summary,
                text: "done".to_string(),
                summary: Some("worker complete".to_string()),
                timestamp: String::new(),
                read: false,
                protocol: None,
                metadata: HashMap::new(),
            },
        )
        .expect("write");

        let unread = read_unread_messages(&working_dir, "run-1", "leader").expect("read");
        assert_eq!(unread.len(), 1);
        assert_eq!(unread[0].text, "done");

        mark_all_read(&working_dir, "run-1", "leader").expect("mark read");
        let unread_after = read_unread_messages(&working_dir, "run-1", "leader").expect("read");
        assert!(unread_after.is_empty());
    }
}
