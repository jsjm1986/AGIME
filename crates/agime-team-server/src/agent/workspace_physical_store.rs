use anyhow::Result;
use std::path::{Path, PathBuf};

use super::workspace_types::{
    WorkspaceArtifactArea, WorkspaceArtifactRecord, WorkspaceManifest, WorkspaceRunRef,
};

pub trait WorkspacePhysicalStore: Send + Sync {
    fn ensure_workspace_root(&self, root: &Path) -> Result<()>;
    fn ensure_standard_layout(&self, root: &Path) -> Result<()>;
    fn ensure_area_dir(&self, root: &Path, area: WorkspaceArtifactArea) -> Result<PathBuf>;
    fn ensure_run_dir(&self, root: &Path, run_id: &str) -> Result<WorkspaceRunRef>;
    fn load_manifest(&self, manifest_path: &Path) -> Result<Option<WorkspaceManifest>>;
    fn write_manifest(&self, manifest_path: &Path, manifest: &WorkspaceManifest) -> Result<()>;
    fn append_artifact_record(
        &self,
        manifest_path: &Path,
        record: WorkspaceArtifactRecord,
    ) -> Result<WorkspaceManifest>;
}
