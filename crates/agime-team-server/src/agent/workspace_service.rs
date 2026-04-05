use agime_team::models::mongo::DocumentAnalysisContext;
use anyhow::Result;
use chrono::Utc;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use uuid::Uuid;

use super::local_fs_workspace_store::LocalFsWorkspaceStore;
use super::runtime;
use super::session_mongo::AgentSessionDoc;
use super::workspace_physical_store::WorkspacePhysicalStore;
pub use super::workspace_types::{
    WorkspaceArtifactArea, WorkspaceBinding, WorkspaceKind, WorkspaceLifecycleState,
    WorkspaceManifest, WorkspacePublicationTarget, WorkspaceRef, WorkspaceRunRef,
};

struct WorkspaceCreateSpec {
    workspace_kind: WorkspaceKind,
    team_id: String,
    conversation_id: String,
    root_path: Option<String>,
    segments: Vec<(String, &'static str)>,
    source_session_id: Option<String>,
    source_channel_id: Option<String>,
    source_thread_root_id: Option<String>,
    source_document_id: Option<String>,
    publication_targets: Vec<WorkspacePublicationTarget>,
}

#[derive(Clone)]
pub struct WorkspaceService {
    workspace_root: String,
    store: Arc<dyn WorkspacePhysicalStore>,
}

impl std::fmt::Debug for WorkspaceService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WorkspaceService")
            .field("workspace_root", &self.workspace_root)
            .finish_non_exhaustive()
    }
}

impl WorkspaceService {
    pub fn new(workspace_root: impl Into<String>) -> Self {
        Self::with_store(workspace_root, Arc::new(LocalFsWorkspaceStore))
    }

    pub fn with_store(
        workspace_root: impl Into<String>,
        store: Arc<dyn WorkspacePhysicalStore>,
    ) -> Self {
        Self {
            workspace_root: workspace_root.into(),
            store,
        }
    }

    pub fn ensure_chat_workspace(&self, session: &AgentSessionDoc) -> Result<WorkspaceBinding> {
        let root_path = session.workspace_path.clone();
        let spec = WorkspaceCreateSpec {
            workspace_kind: WorkspaceKind::Conversation,
            team_id: session.team_id.clone(),
            conversation_id: session.session_id.clone(),
            root_path,
            segments: vec![
                (session.team_id.clone(), "team_id"),
                ("conversations".to_string(), "category"),
                ("chat".to_string(), "conversation_kind"),
                (session.session_id.clone(), "session_id"),
            ],
            source_session_id: Some(session.session_id.clone()),
            source_channel_id: session.source_channel_id.clone(),
            source_thread_root_id: session.source_thread_root_id.clone(),
            source_document_id: None,
            publication_targets: Vec::new(),
        };
        self.ensure_workspace(spec)
    }

    pub fn ensure_document_analysis_workspace(
        &self,
        session: &AgentSessionDoc,
        ctx: &DocumentAnalysisContext,
    ) -> Result<WorkspaceBinding> {
        let root_path = session.workspace_path.clone();
        let spec = WorkspaceCreateSpec {
            workspace_kind: WorkspaceKind::DocumentAnalysis,
            team_id: session.team_id.clone(),
            conversation_id: format!("{}-{}", ctx.doc_id, session.session_id),
            root_path,
            segments: vec![
                (session.team_id.clone(), "team_id"),
                ("document-analysis".to_string(), "category"),
                (ctx.doc_id.clone(), "doc_id"),
                (session.session_id.clone(), "analysis_run_id"),
            ],
            source_session_id: Some(session.session_id.clone()),
            source_channel_id: None,
            source_thread_root_id: None,
            source_document_id: Some(ctx.doc_id.clone()),
            publication_targets: vec![WorkspacePublicationTarget {
                target_type: "document".to_string(),
                target_id: ctx.doc_id.clone(),
                label: Some(ctx.doc_name.clone()),
                folder_path: None,
            }],
        };
        self.ensure_workspace(spec)
    }

    pub fn ensure_channel_workspace(&self, session: &AgentSessionDoc) -> Result<WorkspaceBinding> {
        let channel_id = session
            .source_channel_id
            .clone()
            .unwrap_or_else(|| session.session_id.clone());
        let thread_id = session
            .source_thread_root_id
            .clone()
            .unwrap_or_else(|| channel_id.clone());
        let root_path = session.workspace_path.clone();
        let spec = WorkspaceCreateSpec {
            workspace_kind: WorkspaceKind::ChannelThread,
            team_id: session.team_id.clone(),
            conversation_id: thread_id.clone(),
            root_path,
            segments: vec![
                (session.team_id.clone(), "team_id"),
                ("channels".to_string(), "category"),
                (channel_id.clone(), "channel_id"),
                ("threads".to_string(), "threads"),
                (thread_id, "thread_root_id"),
            ],
            source_session_id: Some(session.session_id.clone()),
            source_channel_id: session.source_channel_id.clone(),
            source_thread_root_id: session.source_thread_root_id.clone(),
            source_document_id: None,
            publication_targets: session
                .source_channel_id
                .as_ref()
                .map(|value| {
                    vec![WorkspacePublicationTarget {
                        target_type: "channel_folder".to_string(),
                        target_id: value.clone(),
                        label: session.source_channel_name.clone(),
                        folder_path: None,
                    }]
                })
                .unwrap_or_default(),
        };
        self.ensure_workspace(spec)
    }

    pub fn ensure_run(&self, workspace: &WorkspaceRef, run_id: &str) -> Result<WorkspaceRunRef> {
        self.store
            .ensure_run_dir(Path::new(&workspace.root_path), run_id)
    }

    pub fn load_workspace(&self, root_path: &str) -> Result<Option<WorkspaceRef>> {
        let normalized_root = super::normalize_workspace_path(root_path);
        let manifest_path = PathBuf::from(&normalized_root).join("workspace.json");
        let Some(manifest) = self.store.load_manifest(&manifest_path)? else {
            return Ok(None);
        };

        Ok(Some(WorkspaceRef {
            workspace_id: manifest.workspace_id,
            workspace_kind: manifest.workspace_kind,
            conversation_id: manifest.conversation_id,
            root_path: normalized_root,
            manifest_path: manifest.manifest_path,
        }))
    }

    pub fn record_workspace_attachment(
        &self,
        workspace: &WorkspaceRef,
        relative_path: &str,
        content_type: Option<String>,
    ) -> Result<WorkspaceManifest> {
        self.record_workspace_file(
            workspace,
            WorkspaceArtifactArea::Attachments,
            relative_path,
            content_type,
        )
    }

    pub fn record_workspace_artifact(
        &self,
        workspace: &WorkspaceRef,
        relative_path: &str,
        content_type: Option<String>,
    ) -> Result<WorkspaceManifest> {
        self.record_workspace_file(
            workspace,
            WorkspaceArtifactArea::Artifacts,
            relative_path,
            content_type,
        )
    }

    pub fn record_workspace_output(
        &self,
        workspace: &WorkspaceRef,
        path: &str,
        content_type: Option<String>,
    ) -> Result<WorkspaceManifest> {
        let normalized = path.trim().replace('\\', "/");
        if normalized.is_empty() {
            return self
                .store
                .load_manifest(Path::new(&workspace.manifest_path))?
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "Workspace manifest {:?} does not exist",
                        workspace.manifest_path
                    )
                });
        }

        let manifest_path = PathBuf::from(&workspace.manifest_path);
        let record_path = if normalized.eq_ignore_ascii_case("attachments")
            || normalized.eq_ignore_ascii_case("artifacts")
            || normalized.eq_ignore_ascii_case("notes")
            || normalized.starts_with("attachments/")
            || normalized.starts_with("artifacts/")
            || normalized.starts_with("notes/")
            || normalized.starts_with("runs/")
        {
            normalized
        } else {
            format!("artifacts/{}", normalized)
        };
        self.store.append_artifact_record(
            &manifest_path,
            super::workspace_types::WorkspaceArtifactRecord {
                path: record_path,
                content_type,
            },
        )
    }

    pub fn record_completion_artifacts(
        &self,
        workspace: &WorkspaceRef,
        produced_artifacts: &[String],
        accepted_artifacts: &[String],
    ) -> Result<WorkspaceManifest> {
        let mut last_manifest = self
            .store
            .load_manifest(Path::new(&workspace.manifest_path))?
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "Workspace manifest {:?} does not exist",
                    workspace.manifest_path
                )
            })?;

        for path in produced_artifacts.iter().chain(accepted_artifacts.iter()) {
            last_manifest = self.record_workspace_output(workspace, path, None)?;
        }

        Ok(last_manifest)
    }

    fn record_workspace_file(
        &self,
        workspace: &WorkspaceRef,
        area: WorkspaceArtifactArea,
        relative_path: &str,
        content_type: Option<String>,
    ) -> Result<WorkspaceManifest> {
        let relative_path = relative_path.trim().replace('\\', "/");
        let manifest_path = PathBuf::from(&workspace.manifest_path);
        self.store.append_artifact_record(
            &manifest_path,
            super::workspace_types::WorkspaceArtifactRecord {
                path: format!("{}/{}", area.as_rel_dir(), relative_path),
                content_type,
            },
        )
    }

    fn ensure_workspace(&self, spec: WorkspaceCreateSpec) -> Result<WorkspaceBinding> {
        let root_path = match spec.root_path {
            Some(path) if !path.trim().is_empty() => path,
            _ => {
                let segments = spec
                    .segments
                    .iter()
                    .map(|(segment, label)| (segment.as_str(), *label))
                    .collect::<Vec<_>>();
                runtime::create_workspace_dir(&self.workspace_root, &segments)?
            }
        };

        let normalized_root = super::normalize_workspace_path(&root_path);
        let root = PathBuf::from(&normalized_root);
        self.store.ensure_workspace_root(&root)?;
        self.store.ensure_standard_layout(&root)?;

        let manifest_path = root.join("workspace.json");
        if let Some(existing) = self.store.load_manifest(&manifest_path)? {
            return Ok(WorkspaceRef {
                workspace_id: existing.workspace_id,
                workspace_kind: existing.workspace_kind,
                conversation_id: existing.conversation_id,
                root_path: normalized_root,
                manifest_path: existing.manifest_path,
            });
        }

        let workspace_id = Uuid::new_v4().to_string();
        let now = Utc::now();
        let manifest = WorkspaceManifest {
            workspace_id: workspace_id.clone(),
            workspace_kind: spec.workspace_kind,
            team_id: spec.team_id,
            conversation_id: spec.conversation_id.clone(),
            root_path: normalized_root.clone(),
            manifest_path: manifest_path.to_string_lossy().to_string(),
            created_at: now,
            updated_at: now,
            source_session_id: spec.source_session_id,
            source_channel_id: spec.source_channel_id,
            source_thread_root_id: spec.source_thread_root_id,
            source_document_id: spec.source_document_id,
            lifecycle_state: WorkspaceLifecycleState::Active,
            publication_targets: spec.publication_targets,
            artifact_index: Vec::new(),
        };
        self.store.write_manifest(&manifest_path, &manifest)?;

        Ok(WorkspaceRef {
            workspace_id,
            workspace_kind: manifest.workspace_kind,
            conversation_id: spec.conversation_id,
            root_path: normalized_root,
            manifest_path: manifest.manifest_path,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::session_mongo::AgentSessionDoc;
    use std::fs;

    fn sample_session() -> AgentSessionDoc {
        let now = bson::DateTime::now();
        AgentSessionDoc {
            id: None,
            session_id: "session-1".to_string(),
            team_id: "team-1".to_string(),
            agent_id: "agent-1".to_string(),
            user_id: "user-1".to_string(),
            name: None,
            status: "active".to_string(),
            messages_json: "[]".to_string(),
            message_count: 0,
            total_tokens: None,
            input_tokens: None,
            output_tokens: None,
            compaction_count: 0,
            disabled_extensions: Vec::new(),
            enabled_extensions: Vec::new(),
            created_at: now,
            updated_at: now,
            title: None,
            pinned: false,
            last_message_preview: None,
            last_message_at: None,
            is_processing: false,
            last_execution_status: None,
            last_execution_error: None,
            last_execution_finished_at: None,
            last_runtime_session_id: None,
            attached_document_ids: Vec::new(),
            workspace_path: None,
            workspace_id: None,
            workspace_kind: None,
            workspace_manifest_path: None,
            extra_instructions: None,
            allowed_extensions: None,
            allowed_skill_ids: None,
            retry_config: None,
            max_turns: None,
            tool_timeout_seconds: None,
            max_portal_retry_rounds: None,
            require_final_report: false,
            portal_restricted: false,
            document_access_mode: None,
            document_scope_mode: None,
            document_write_mode: None,
            delegation_policy_override: None,
            portal_id: None,
            portal_slug: None,
            visitor_id: None,
            session_source: "chat".to_string(),
            source_channel_id: None,
            source_channel_name: None,
            source_thread_root_id: None,
            hidden_from_chat_list: false,
        }
    }

    #[test]
    fn creates_chat_workspace_manifest_and_layout() {
        let root = std::env::temp_dir().join(format!("workspace-service-{}", Uuid::new_v4()));
        let service = WorkspaceService::new(root.to_string_lossy().to_string());
        let session = sample_session();

        let binding = service
            .ensure_chat_workspace(&session)
            .expect("workspace should be created");

        assert_eq!(binding.workspace_kind, WorkspaceKind::Conversation);
        assert!(Path::new(&binding.root_path)
            .join("workspace.json")
            .exists());
        assert!(Path::new(&binding.root_path).join("attachments").exists());
        assert!(Path::new(&binding.root_path).join("artifacts").exists());
        assert!(Path::new(&binding.root_path)
            .join(".agime/runtime")
            .exists());
        let manifest: WorkspaceManifest = serde_json::from_slice(
            &fs::read(Path::new(&binding.root_path).join("workspace.json"))
                .expect("manifest should exist"),
        )
        .expect("manifest should deserialize");
        assert_eq!(manifest.workspace_kind, WorkspaceKind::Conversation);
        assert_eq!(manifest.conversation_id, "session-1");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn preserves_existing_workspace_path_when_binding_manifest() {
        let root = std::env::temp_dir()
            .join(format!("workspace-existing-{}", Uuid::new_v4()))
            .join("existing");
        fs::create_dir_all(&root).expect("existing root should be created");
        let service = WorkspaceService::new(
            root.parent()
                .expect("temp root")
                .to_string_lossy()
                .to_string(),
        );
        let mut session = sample_session();
        session.workspace_path = Some(root.to_string_lossy().to_string());

        let binding = service
            .ensure_chat_workspace(&session)
            .expect("workspace binding should succeed");
        assert_eq!(
            binding.root_path,
            super::super::normalize_workspace_path(&root.to_string_lossy())
        );
        assert!(root.join("workspace.json").exists());
        let _ = fs::remove_dir_all(root.parent().expect("temp root parent"));
    }

    #[test]
    fn reuses_existing_workspace_manifest_identity() {
        let root = std::env::temp_dir().join(format!("workspace-reuse-{}", Uuid::new_v4()));
        let service = WorkspaceService::new(root.to_string_lossy().to_string());
        let session = sample_session();

        let first = service
            .ensure_chat_workspace(&session)
            .expect("first workspace create should succeed");
        let second = service
            .ensure_chat_workspace(&session)
            .expect("second workspace create should reuse manifest");

        assert_eq!(first.workspace_id, second.workspace_id);
        assert_eq!(first.manifest_path, second.manifest_path);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn creates_channel_workspace_with_publication_target() {
        let root = std::env::temp_dir().join(format!("workspace-channel-{}", Uuid::new_v4()));
        let service = WorkspaceService::new(root.to_string_lossy().to_string());
        let mut session = sample_session();
        session.session_source = "channel_runtime".to_string();
        session.source_channel_id = Some("channel-1".to_string());
        session.source_channel_name = Some("Ops".to_string());
        session.source_thread_root_id = Some("thread-1".to_string());

        let binding = service
            .ensure_channel_workspace(&session)
            .expect("channel workspace should be created");
        let manifest: WorkspaceManifest = serde_json::from_slice(
            &fs::read(Path::new(&binding.root_path).join("workspace.json"))
                .expect("manifest should exist"),
        )
        .expect("manifest should deserialize");

        assert_eq!(manifest.workspace_kind, WorkspaceKind::ChannelThread);
        assert_eq!(manifest.conversation_id, "thread-1");
        assert_eq!(manifest.publication_targets.len(), 1);
        assert_eq!(
            manifest.publication_targets[0].target_type,
            "channel_folder"
        );
        assert_eq!(manifest.publication_targets[0].target_id, "channel-1");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn ensures_run_dir_under_workspace() {
        let root = std::env::temp_dir().join(format!("workspace-run-{}", Uuid::new_v4()));
        let service = WorkspaceService::new(root.to_string_lossy().to_string());
        let session = sample_session();
        let workspace = service
            .ensure_chat_workspace(&session)
            .expect("workspace should be created");
        let run = service
            .ensure_run(&workspace, "run-1")
            .expect("run dir should be created");
        assert!(Path::new(&run.run_path).exists());
        assert!(run.run_path.replace('\\', "/").ends_with("/runs/run-1"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn records_workspace_artifact_in_manifest() {
        let root = std::env::temp_dir().join(format!("workspace-artifact-{}", Uuid::new_v4()));
        let service = WorkspaceService::new(root.to_string_lossy().to_string());
        let session = sample_session();
        let workspace = service
            .ensure_chat_workspace(&session)
            .expect("workspace should be created");
        let manifest = service
            .record_workspace_artifact(&workspace, "report.md", Some("text/markdown".to_string()))
            .expect("artifact should be recorded");
        assert_eq!(manifest.artifact_index.len(), 1);
        assert_eq!(manifest.artifact_index[0].path, "artifacts/report.md");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn records_completion_artifacts_in_manifest() {
        let root = std::env::temp_dir().join(format!("workspace-complete-{}", Uuid::new_v4()));
        let service = WorkspaceService::new(root.to_string_lossy().to_string());
        let session = sample_session();
        let workspace = service
            .ensure_chat_workspace(&session)
            .expect("workspace should be created");
        let manifest = service
            .record_completion_artifacts(
                &workspace,
                &["reports/summary.md".to_string()],
                &[
                    "artifacts/final.md".to_string(),
                    "runs/run-1/tmp.json".to_string(),
                ],
            )
            .expect("completion artifacts should be recorded");
        let paths = manifest
            .artifact_index
            .iter()
            .map(|item| item.path.clone())
            .collect::<Vec<_>>();
        assert!(paths.contains(&"artifacts/reports/summary.md".to_string()));
        assert!(paths.contains(&"artifacts/final.md".to_string()));
        assert!(paths.contains(&"runs/run-1/tmp.json".to_string()));
        let _ = fs::remove_dir_all(root);
    }
}
