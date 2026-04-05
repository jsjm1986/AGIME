use anyhow::{anyhow, Result};
use std::fs;
use std::path::{Path, PathBuf};

use super::workspace_physical_store::WorkspacePhysicalStore;
use super::workspace_types::{
    WorkspaceArtifactArea, WorkspaceArtifactRecord, WorkspaceManifest, WorkspaceRunRef,
};

#[derive(Debug, Clone, Default)]
pub struct LocalFsWorkspaceStore;

impl WorkspacePhysicalStore for LocalFsWorkspaceStore {
    fn ensure_workspace_root(&self, root: &Path) -> Result<()> {
        fs::create_dir_all(root)
            .map_err(|e| anyhow!("Failed to create workspace root {:?}: {}", root, e))
    }

    fn ensure_standard_layout(&self, root: &Path) -> Result<()> {
        for rel in [
            WorkspaceArtifactArea::Attachments.as_rel_dir(),
            WorkspaceArtifactArea::Artifacts.as_rel_dir(),
            WorkspaceArtifactArea::Notes.as_rel_dir(),
            ".agime/runtime",
            ".agime/mailboxes",
            ".agime/scratchpad",
            ".agime/permissions",
            "runs",
        ] {
            let path = root.join(rel);
            fs::create_dir_all(&path)
                .map_err(|e| anyhow!("Failed to create workspace subdir {:?}: {}", path, e))?;
        }
        Ok(())
    }

    fn ensure_area_dir(&self, root: &Path, area: WorkspaceArtifactArea) -> Result<PathBuf> {
        let path = root.join(area.as_rel_dir());
        fs::create_dir_all(&path)
            .map_err(|e| anyhow!("Failed to create workspace area {:?}: {}", path, e))?;
        Ok(path)
    }

    fn ensure_run_dir(&self, root: &Path, run_id: &str) -> Result<WorkspaceRunRef> {
        let path = root.join("runs").join(run_id);
        fs::create_dir_all(&path)
            .map_err(|e| anyhow!("Failed to create workspace run dir {:?}: {}", path, e))?;
        Ok(WorkspaceRunRef {
            run_id: run_id.to_string(),
            run_path: path.to_string_lossy().to_string(),
        })
    }

    fn load_manifest(&self, manifest_path: &Path) -> Result<Option<WorkspaceManifest>> {
        if !manifest_path.exists() {
            return Ok(None);
        }
        let bytes = fs::read(manifest_path).map_err(|e| {
            anyhow!(
                "Failed to read workspace manifest {:?}: {}",
                manifest_path,
                e
            )
        })?;
        let manifest = serde_json::from_slice::<WorkspaceManifest>(&bytes).map_err(|e| {
            anyhow!(
                "Failed to parse workspace manifest {:?}: {}",
                manifest_path,
                e
            )
        })?;
        Ok(Some(manifest))
    }

    fn write_manifest(&self, manifest_path: &Path, manifest: &WorkspaceManifest) -> Result<()> {
        fs::write(
            manifest_path,
            serde_json::to_vec_pretty(manifest)
                .map_err(|e| anyhow!("Failed to serialize workspace manifest: {}", e))?,
        )
        .map_err(|e| {
            anyhow!(
                "Failed to write workspace manifest {:?}: {}",
                manifest_path,
                e
            )
        })
    }

    fn append_artifact_record(
        &self,
        manifest_path: &Path,
        record: WorkspaceArtifactRecord,
    ) -> Result<WorkspaceManifest> {
        let mut manifest = self
            .load_manifest(manifest_path)?
            .ok_or_else(|| anyhow!("Workspace manifest {:?} does not exist", manifest_path))?;
        if !manifest
            .artifact_index
            .iter()
            .any(|item| item.path == record.path)
        {
            manifest.artifact_index.push(record);
        }
        manifest.updated_at = chrono::Utc::now();
        self.write_manifest(manifest_path, &manifest)?;
        Ok(manifest)
    }
}
