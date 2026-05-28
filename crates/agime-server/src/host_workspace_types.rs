//! Workspace data types (desktop-flavored).
//!
//! SOURCE: crates/agime-team-server/src/agent/workspace_types.rs at commit
//! 961109f. Verbatim copy except for cosmetic touches; the type names mirror
//! the team-server originals so future manual reconciliation stays trivial.
//! See CLAUDE.md long-term maintenance strategy.
//!
//! NOTE: We intentionally do NOT reuse [`crate::host_workspace::WorkspaceKind`]
//! — that one is the lighter rule-side enum (Conversation/Document/Skill).
//! Workspace persistence needs the full 6-variant flavour (including
//! ChannelProject / ChannelThread / DocumentAnalysis / PortalProject /
//! McpAdmin) so manifests round-trip cleanly with team-server payloads.

#![cfg(feature = "desktop_harness_host")]
#![allow(dead_code)]

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceKind {
    Conversation,
    ChannelProject,
    ChannelThread,
    DocumentAnalysis,
    PortalProject,
    McpAdmin,
}

impl WorkspaceKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Conversation => "conversation",
            Self::ChannelProject => "channel_project",
            Self::ChannelThread => "channel_thread",
            Self::DocumentAnalysis => "document_analysis",
            Self::PortalProject => "portal_project",
            Self::McpAdmin => "mcp_admin",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceLifecycleState {
    Active,
    Archived,
    CleanedUp,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceArtifactArea {
    Attachments,
    Artifacts,
    Notes,
}

impl WorkspaceArtifactArea {
    pub fn as_rel_dir(self) -> &'static str {
        match self {
            Self::Attachments => "attachments",
            Self::Artifacts => "artifacts",
            Self::Notes => "notes",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WorkspacePublicationTarget {
    pub target_type: String,
    pub target_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub folder_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceArtifactRecord {
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceManifest {
    pub workspace_id: String,
    pub workspace_kind: WorkspaceKind,
    pub team_id: String,
    pub conversation_id: String,
    pub root_path: String,
    pub manifest_path: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_channel_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_thread_root_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_document_id: Option<String>,
    pub lifecycle_state: WorkspaceLifecycleState,
    #[serde(default)]
    pub publication_targets: Vec<WorkspacePublicationTarget>,
    #[serde(default)]
    pub artifact_index: Vec<WorkspaceArtifactRecord>,
}

#[derive(Debug, Clone)]
pub struct WorkspaceRef {
    pub workspace_id: String,
    pub workspace_kind: WorkspaceKind,
    pub conversation_id: String,
    pub root_path: String,
    pub manifest_path: String,
}

pub type WorkspaceBinding = WorkspaceRef;

#[derive(Debug, Clone)]
pub struct WorkspaceRunRef {
    pub run_id: String,
    pub run_path: String,
}
