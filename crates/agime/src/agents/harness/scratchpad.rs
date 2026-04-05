use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

fn root_dir(working_dir: &Path, run_id: &str) -> PathBuf {
    working_dir.join(".agime").join("swarm").join(run_id)
}

#[derive(Debug, Clone)]
pub struct SwarmScratchpad {
    pub root: PathBuf,
    pub shared_dir: PathBuf,
    pub workers_dir: PathBuf,
}

impl SwarmScratchpad {
    pub fn create(working_dir: &Path, run_id: &str) -> Result<Self> {
        let root = root_dir(working_dir, run_id).join("scratchpad");
        let shared_dir = root.join("shared");
        let workers_dir = root.join("workers");
        fs::create_dir_all(&shared_dir)
            .with_context(|| format!("failed to create scratchpad {}", shared_dir.display()))?;
        fs::create_dir_all(&workers_dir).with_context(|| {
            format!(
                "failed to create worker scratchpad {}",
                workers_dir.display()
            )
        })?;
        Ok(Self {
            root,
            shared_dir,
            workers_dir,
        })
    }

    pub fn shared_file(&self, name: &str) -> PathBuf {
        self.shared_dir.join(name)
    }

    pub fn worker_dir(&self, worker_name: &str) -> PathBuf {
        self.workers_dir.join(worker_name)
    }

    pub fn ensure_worker_dir(&self, worker_name: &str) -> Result<PathBuf> {
        let path = self.worker_dir(worker_name);
        fs::create_dir_all(&path)
            .with_context(|| format!("failed to create worker dir {}", path.display()))?;
        Ok(path)
    }

    pub fn write_shared_note(&self, name: &str, content: &str) -> Result<PathBuf> {
        let path = self.shared_file(name);
        fs::write(&path, content)
            .with_context(|| format!("failed to write shared scratchpad {}", path.display()))?;
        Ok(path)
    }

    pub fn write_worker_note(
        &self,
        worker_name: &str,
        name: &str,
        content: &str,
    ) -> Result<PathBuf> {
        let dir = self.ensure_worker_dir(worker_name)?;
        let path = dir.join(name);
        fs::write(&path, content)
            .with_context(|| format!("failed to write worker scratchpad {}", path.display()))?;
        Ok(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scratchpad_creates_shared_and_worker_paths() {
        let temp = tempfile::tempdir().expect("tempdir");
        let scratchpad = SwarmScratchpad::create(temp.path(), "run-1").expect("scratchpad");
        let shared = scratchpad
            .write_shared_note("README.md", "hello")
            .expect("shared");
        let worker = scratchpad
            .write_worker_note("worker-1", "summary.md", "done")
            .expect("worker");
        assert!(shared.exists());
        assert!(worker.exists());
    }
}
