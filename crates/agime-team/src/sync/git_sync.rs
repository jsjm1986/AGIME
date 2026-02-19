//! Git synchronization module
//!
//! This module handles synchronization of team resources with Git repositories.
//! It exports/imports skills, recipes, and extensions to/from a Git repository.

use crate::error::{TeamError, TeamResult};
use crate::models::{SyncState, SyncStatus};
use crate::security::validate_resource_name;
use git2::{Cred, FetchOptions, PushOptions, RemoteCallbacks, Repository, Signature};
use std::path::{Path, PathBuf};
use tracing::{debug, info, warn};

/// Default branch name
const DEFAULT_BRANCH: &str = "main";

/// Git sync configuration
#[derive(Debug, Clone)]
pub struct GitSyncConfig {
    /// Base directory for storing repositories
    pub base_dir: PathBuf,
    /// Git author name
    pub author_name: String,
    /// Git author email
    pub author_email: String,
    /// SSH key path (optional)
    pub ssh_key_path: Option<PathBuf>,
    /// Whether to use SSH agent
    pub use_ssh_agent: bool,
}

impl Default for GitSyncConfig {
    fn default() -> Self {
        let base_dir = dirs::data_local_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("agime")
            .join("team-repos");

        Self {
            base_dir,
            author_name: "AGIME".to_string(),
            author_email: "agime@local".to_string(),
            ssh_key_path: None,
            use_ssh_agent: true,
        }
    }
}

/// Git sync manager
pub struct GitSync {
    config: GitSyncConfig,
}

impl GitSync {
    /// Create a new git sync manager
    pub fn new() -> Self {
        Self {
            config: GitSyncConfig::default(),
        }
    }

    /// Create with custom config
    pub fn with_config(config: GitSyncConfig) -> Self {
        Self { config }
    }

    /// Get the repository path for a team
    fn repo_path(&self, team_id: &str) -> PathBuf {
        self.config.base_dir.join(team_id)
    }

    /// Create git credentials callback
    fn create_callbacks(&self) -> RemoteCallbacks<'_> {
        let mut callbacks = RemoteCallbacks::new();

        callbacks.credentials(|_url, username_from_url, allowed_types| {
            if allowed_types.is_ssh_key() {
                if self.config.use_ssh_agent {
                    if let Some(username) = username_from_url {
                        return Cred::ssh_key_from_agent(username);
                    }
                }

                if let Some(ref key_path) = self.config.ssh_key_path {
                    if let Some(username) = username_from_url {
                        return Cred::ssh_key(username, None, key_path, None);
                    }
                }
            }

            Cred::default()
        });

        callbacks
    }

    /// Initialize or clone a git repository for the team
    pub fn init_repo(&self, team_id: &str, repo_url: Option<&str>) -> TeamResult<PathBuf> {
        let repo_path = self.repo_path(team_id);

        // Create directory if it doesn't exist
        if !repo_path.exists() {
            std::fs::create_dir_all(&repo_path).map_err(|e| TeamError::SyncFailed {
                reason: format!("Failed to create repository directory: {}", e),
            })?;
        }

        // Check if already a git repo
        if repo_path.join(".git").exists() {
            info!("Repository already exists at {:?}", repo_path);
            return Ok(repo_path);
        }

        match repo_url {
            Some(url) => {
                // Clone from remote
                info!("Cloning repository from {} to {:?}", url, repo_path);

                let mut callbacks = self.create_callbacks();
                callbacks.transfer_progress(|progress| {
                    debug!(
                        "Git clone progress: {}/{}",
                        progress.received_objects(),
                        progress.total_objects()
                    );
                    true
                });

                let mut fetch_options = FetchOptions::new();
                fetch_options.remote_callbacks(callbacks);

                let mut builder = git2::build::RepoBuilder::new();
                builder.fetch_options(fetch_options);

                builder
                    .clone(url, &repo_path)
                    .map_err(|e| TeamError::SyncFailed {
                        reason: format!("Failed to clone repository: {}", e),
                    })?;

                info!("Repository cloned successfully");
            }
            None => {
                // Initialize new repo
                info!("Initializing new repository at {:?}", repo_path);

                Repository::init(&repo_path).map_err(|e| TeamError::SyncFailed {
                    reason: format!("Failed to initialize repository: {}", e),
                })?;

                // Create initial structure
                self.create_repo_structure(&repo_path)?;

                info!("Repository initialized successfully");
            }
        }

        Ok(repo_path)
    }

    /// Create the standard repository structure
    fn create_repo_structure(&self, repo_path: &Path) -> TeamResult<()> {
        let dirs = ["skills", "recipes", "extensions"];

        for dir in &dirs {
            let dir_path = repo_path.join(dir);
            std::fs::create_dir_all(&dir_path).map_err(|e| TeamError::SyncFailed {
                reason: format!("Failed to create {} directory: {}", dir, e),
            })?;

            // Create .gitkeep to ensure directory is tracked
            let gitkeep = dir_path.join(".gitkeep");
            std::fs::write(&gitkeep, "").map_err(|e| TeamError::SyncFailed {
                reason: format!("Failed to create .gitkeep: {}", e),
            })?;
        }

        // Create README
        let readme_content = r#"# Team Resources

This repository contains shared team resources for AGIME.

## Structure

- `skills/` - Skill definitions (markdown files)
- `recipes/` - Recipe configurations (YAML files)
- `extensions/` - Extension configurations (JSON files)

## Usage

Resources in this repository are automatically synced with your AGIME instance.
"#;
        std::fs::write(repo_path.join("README.md"), readme_content).map_err(|e| {
            TeamError::SyncFailed {
                reason: format!("Failed to create README: {}", e),
            }
        })?;

        // Initial commit
        let repo = Repository::open(repo_path).map_err(|e| TeamError::SyncFailed {
            reason: format!("Failed to open repository: {}", e),
        })?;

        let sig =
            Signature::now(&self.config.author_name, &self.config.author_email).map_err(|e| {
                TeamError::SyncFailed {
                    reason: format!("Failed to create signature: {}", e),
                }
            })?;

        let mut index = repo.index().map_err(|e| TeamError::SyncFailed {
            reason: format!("Failed to get index: {}", e),
        })?;

        index
            .add_all(["*"].iter(), git2::IndexAddOption::DEFAULT, None)
            .map_err(|e| TeamError::SyncFailed {
                reason: format!("Failed to add files: {}", e),
            })?;

        index.write().map_err(|e| TeamError::SyncFailed {
            reason: format!("Failed to write index: {}", e),
        })?;

        let tree_id = index.write_tree().map_err(|e| TeamError::SyncFailed {
            reason: format!("Failed to write tree: {}", e),
        })?;

        let tree = repo.find_tree(tree_id).map_err(|e| TeamError::SyncFailed {
            reason: format!("Failed to find tree: {}", e),
        })?;

        repo.commit(Some("HEAD"), &sig, &sig, "Initial commit", &tree, &[])
            .map_err(|e| TeamError::SyncFailed {
                reason: format!("Failed to create initial commit: {}", e),
            })?;

        Ok(())
    }

    /// Pull changes from remote
    pub fn pull(&self, team_id: &str) -> TeamResult<SyncStatus> {
        let repo_path = self.repo_path(team_id);

        if !repo_path.exists() {
            return Err(TeamError::SyncFailed {
                reason: "Repository not initialized".to_string(),
            });
        }

        let repo = Repository::open(&repo_path).map_err(|e| TeamError::SyncFailed {
            reason: format!("Failed to open repository: {}", e),
        })?;

        // Check if remote "origin" exists
        let remote_result = repo.find_remote("origin");
        if remote_result.is_err() {
            // No remote configured - return success status (local-only repo)
            info!(
                "No remote 'origin' configured for team {}, skipping pull",
                team_id
            );
            let head = repo.head().ok().and_then(|h| h.target());
            return Ok(SyncStatus {
                team_id: team_id.to_string(),
                state: SyncState::Idle,
                last_sync_at: Some(chrono::Utc::now()),
                last_commit_hash: head.map(|h| h.to_string()),
                error_message: None,
            });
        }

        // Get the remote
        let mut remote = remote_result.map_err(|e| TeamError::SyncFailed {
            reason: format!("Failed to find remote 'origin': {}", e),
        })?;

        // Fetch
        let callbacks = self.create_callbacks();
        let mut fetch_options = FetchOptions::new();
        fetch_options.remote_callbacks(callbacks);

        info!("Fetching from remote...");
        remote
            .fetch(&[DEFAULT_BRANCH], Some(&mut fetch_options), None)
            .map_err(|e| TeamError::SyncFailed {
                reason: format!("Failed to fetch: {}", e),
            })?;

        // Get the fetch head
        let fetch_head = repo
            .find_reference("FETCH_HEAD")
            .map_err(|e| TeamError::SyncFailed {
                reason: format!("Failed to find FETCH_HEAD: {}", e),
            })?;

        let fetch_commit = repo
            .reference_to_annotated_commit(&fetch_head)
            .map_err(|e| TeamError::SyncFailed {
                reason: format!("Failed to get commit: {}", e),
            })?;

        // Perform merge (fast-forward if possible)
        let analysis =
            repo.merge_analysis(&[&fetch_commit])
                .map_err(|e| TeamError::SyncFailed {
                    reason: format!("Failed to analyze merge: {}", e),
                })?;

        let last_commit_hash = if analysis.0.is_fast_forward() {
            info!("Fast-forward merge");
            let refname = format!("refs/heads/{}", DEFAULT_BRANCH);
            let mut reference =
                repo.find_reference(&refname)
                    .map_err(|e| TeamError::SyncFailed {
                        reason: format!("Failed to find reference: {}", e),
                    })?;

            reference
                .set_target(fetch_commit.id(), "Fast-forward")
                .map_err(|e| TeamError::SyncFailed {
                    reason: format!("Failed to update reference: {}", e),
                })?;

            repo.set_head(&refname).map_err(|e| TeamError::SyncFailed {
                reason: format!("Failed to set HEAD: {}", e),
            })?;

            repo.checkout_head(Some(git2::build::CheckoutBuilder::default().force()))
                .map_err(|e| TeamError::SyncFailed {
                    reason: format!("Failed to checkout: {}", e),
                })?;

            Some(fetch_commit.id().to_string())
        } else if analysis.0.is_up_to_date() {
            info!("Already up to date");
            let head = repo.head().ok().and_then(|h| h.target());
            head.map(|h| h.to_string())
        } else if analysis.0.is_normal() {
            // Normal merge required - perform automatic merge
            info!("Performing automatic merge");

            // Perform the merge
            repo.merge(&[&fetch_commit], None, None)
                .map_err(|e| TeamError::SyncFailed {
                    reason: format!("Failed to merge: {}", e),
                })?;

            // Check for conflicts
            let index = repo.index().map_err(|e| TeamError::SyncFailed {
                reason: format!("Failed to get index: {}", e),
            })?;

            if index.has_conflicts() {
                // Auto-resolve conflicts using "theirs" strategy for resource files
                // This means remote changes take precedence
                warn!("Merge conflicts detected, auto-resolving with 'theirs' strategy");

                let mut resolved_index = repo.index().map_err(|e| TeamError::SyncFailed {
                    reason: format!("Failed to get index: {}", e),
                })?;

                // Collect conflicts first to avoid borrow issues
                let conflicts: Vec<_> = resolved_index
                    .conflicts()
                    .map_err(|e| TeamError::SyncFailed {
                        reason: format!("Failed to get conflicts: {}", e),
                    })?
                    .filter_map(|c| c.ok())
                    .collect();

                for conflict in conflicts {
                    if let Some(their) = conflict.their {
                        // Use their version (remote)
                        let their_path = String::from_utf8_lossy(&their.path).to_string();
                        info!("Resolving conflict in '{}' with remote version", their_path);

                        // Get the blob and write it
                        let blob = repo
                            .find_blob(their.id)
                            .map_err(|e| TeamError::SyncFailed {
                                reason: format!("Failed to find blob: {}", e),
                            })?;

                        let file_path = repo_path.join(&their_path);
                        if let Some(parent) = file_path.parent() {
                            std::fs::create_dir_all(parent).ok();
                        }
                        std::fs::write(&file_path, blob.content()).map_err(|e| {
                            TeamError::SyncFailed {
                                reason: format!("Failed to write resolved file: {}", e),
                            }
                        })?;

                        // Stage the resolved file
                        resolved_index
                            .add_path(Path::new(&their_path))
                            .map_err(|e| TeamError::SyncFailed {
                                reason: format!("Failed to stage resolved file: {}", e),
                            })?;
                    } else if let Some(our) = conflict.our {
                        // If no their version, keep ours (local addition)
                        let our_path = String::from_utf8_lossy(&our.path).to_string();
                        info!("Keeping local version for '{}'", our_path);
                        resolved_index.add_path(Path::new(&our_path)).map_err(|e| {
                            TeamError::SyncFailed {
                                reason: format!("Failed to stage local file: {}", e),
                            }
                        })?;
                    }
                }

                resolved_index.write().map_err(|e| TeamError::SyncFailed {
                    reason: format!("Failed to write resolved index: {}", e),
                })?;
            }

            // Create merge commit
            let sig = Signature::now(&self.config.author_name, &self.config.author_email).map_err(
                |e| TeamError::SyncFailed {
                    reason: format!("Failed to create signature: {}", e),
                },
            )?;

            let mut index = repo.index().map_err(|e| TeamError::SyncFailed {
                reason: format!("Failed to get index: {}", e),
            })?;

            let tree_id = index.write_tree().map_err(|e| TeamError::SyncFailed {
                reason: format!("Failed to write tree: {}", e),
            })?;

            let tree = repo.find_tree(tree_id).map_err(|e| TeamError::SyncFailed {
                reason: format!("Failed to find tree: {}", e),
            })?;

            let head = repo.head().map_err(|e| TeamError::SyncFailed {
                reason: format!("Failed to get HEAD: {}", e),
            })?;

            let local_commit = head.peel_to_commit().map_err(|e| TeamError::SyncFailed {
                reason: format!("Failed to get local commit: {}", e),
            })?;

            let remote_commit =
                repo.find_commit(fetch_commit.id())
                    .map_err(|e| TeamError::SyncFailed {
                        reason: format!("Failed to find remote commit: {}", e),
                    })?;

            let merge_commit = repo
                .commit(
                    Some("HEAD"),
                    &sig,
                    &sig,
                    "Merge remote changes",
                    &tree,
                    &[&local_commit, &remote_commit],
                )
                .map_err(|e| TeamError::SyncFailed {
                    reason: format!("Failed to create merge commit: {}", e),
                })?;

            // Clean up merge state
            repo.cleanup_state().map_err(|e| TeamError::SyncFailed {
                reason: format!("Failed to cleanup merge state: {}", e),
            })?;

            info!("Merge completed successfully");
            Some(merge_commit.to_string())
        } else {
            warn!("Unknown merge analysis result");
            return Err(TeamError::SyncFailed {
                reason: "Unknown merge state".to_string(),
            });
        };

        Ok(SyncStatus {
            team_id: team_id.to_string(),
            state: SyncState::Idle,
            last_sync_at: Some(chrono::Utc::now()),
            last_commit_hash,
            error_message: None,
        })
    }

    /// Push changes to remote
    pub fn push(&self, team_id: &str, message: &str) -> TeamResult<()> {
        let repo_path = self.repo_path(team_id);

        if !repo_path.exists() {
            return Err(TeamError::SyncFailed {
                reason: "Repository not initialized".to_string(),
            });
        }

        let repo = Repository::open(&repo_path).map_err(|e| TeamError::SyncFailed {
            reason: format!("Failed to open repository: {}", e),
        })?;

        // Check if there are changes to commit
        let statuses = repo.statuses(None).map_err(|e| TeamError::SyncFailed {
            reason: format!("Failed to get status: {}", e),
        })?;

        if statuses.is_empty() {
            info!("No changes to push");
            return Ok(());
        }

        // Add all changes
        let mut index = repo.index().map_err(|e| TeamError::SyncFailed {
            reason: format!("Failed to get index: {}", e),
        })?;

        index
            .add_all(["*"].iter(), git2::IndexAddOption::DEFAULT, None)
            .map_err(|e| TeamError::SyncFailed {
                reason: format!("Failed to add files: {}", e),
            })?;

        index.write().map_err(|e| TeamError::SyncFailed {
            reason: format!("Failed to write index: {}", e),
        })?;

        // Create commit
        let sig =
            Signature::now(&self.config.author_name, &self.config.author_email).map_err(|e| {
                TeamError::SyncFailed {
                    reason: format!("Failed to create signature: {}", e),
                }
            })?;

        let tree_id = index.write_tree().map_err(|e| TeamError::SyncFailed {
            reason: format!("Failed to write tree: {}", e),
        })?;

        let tree = repo.find_tree(tree_id).map_err(|e| TeamError::SyncFailed {
            reason: format!("Failed to find tree: {}", e),
        })?;

        let head = repo.head().map_err(|e| TeamError::SyncFailed {
            reason: format!("Failed to get HEAD: {}", e),
        })?;

        let parent_commit = head.peel_to_commit().map_err(|e| TeamError::SyncFailed {
            reason: format!("Failed to get parent commit: {}", e),
        })?;

        repo.commit(Some("HEAD"), &sig, &sig, message, &tree, &[&parent_commit])
            .map_err(|e| TeamError::SyncFailed {
                reason: format!("Failed to create commit: {}", e),
            })?;

        info!("Created commit: {}", message);

        // Check if remote "origin" exists before pushing
        let remote_result = repo.find_remote("origin");
        if remote_result.is_err() {
            // No remote configured - commit was made locally, skip push
            info!(
                "No remote 'origin' configured for team {}, commit saved locally only",
                team_id
            );
            return Ok(());
        }

        // Push to remote
        let mut remote = remote_result.map_err(|e| TeamError::SyncFailed {
            reason: format!("Failed to find remote: {}", e),
        })?;

        let callbacks = self.create_callbacks();
        let mut push_options = PushOptions::new();
        push_options.remote_callbacks(callbacks);

        let refspec = format!(
            "refs/heads/{}:refs/heads/{}",
            DEFAULT_BRANCH, DEFAULT_BRANCH
        );

        remote
            .push(&[&refspec], Some(&mut push_options))
            .map_err(|e| TeamError::SyncFailed {
                reason: format!("Failed to push: {}", e),
            })?;

        info!("Pushed to remote successfully");
        Ok(())
    }

    /// Get current sync status
    pub fn get_status(&self, team_id: &str) -> TeamResult<SyncStatus> {
        let repo_path = self.repo_path(team_id);

        if !repo_path.exists() {
            return Ok(SyncStatus {
                team_id: team_id.to_string(),
                state: SyncState::Idle,
                last_sync_at: None,
                last_commit_hash: None,
                error_message: Some("Repository not initialized".to_string()),
            });
        }

        let repo = Repository::open(&repo_path).map_err(|e| TeamError::SyncFailed {
            reason: format!("Failed to open repository: {}", e),
        })?;

        let last_commit_hash = repo
            .head()
            .ok()
            .and_then(|h| h.target())
            .map(|t| t.to_string());

        Ok(SyncStatus {
            team_id: team_id.to_string(),
            state: SyncState::Idle,
            last_sync_at: None, // Would need to track this separately
            last_commit_hash,
            error_message: None,
        })
    }

    /// Export a resource file to the repository
    fn export_resource(
        &self,
        team_id: &str,
        subdir: &str,
        file_name: &str,
        resource_name: &str,
        content: &str,
    ) -> TeamResult<PathBuf> {
        validate_resource_name(resource_name)?;

        let file_path = self.repo_path(team_id).join(subdir).join(file_name);

        std::fs::write(&file_path, content).map_err(|e| TeamError::SyncFailed {
            reason: format!("Failed to write {}: {}", subdir.trim_end_matches('s'), e),
        })?;

        Ok(file_path)
    }

    /// Export a skill to the repository
    pub fn export_skill(
        &self,
        team_id: &str,
        skill_name: &str,
        content: &str,
    ) -> TeamResult<PathBuf> {
        self.export_resource(team_id, "skills", &format!("{}.md", skill_name), skill_name, content)
    }

    /// Export a recipe to the repository
    pub fn export_recipe(
        &self,
        team_id: &str,
        recipe_name: &str,
        content: &str,
    ) -> TeamResult<PathBuf> {
        self.export_resource(team_id, "recipes", &format!("{}.yaml", recipe_name), recipe_name, content)
    }

    /// Export an extension to the repository
    pub fn export_extension(
        &self,
        team_id: &str,
        ext_name: &str,
        config: &str,
    ) -> TeamResult<PathBuf> {
        self.export_resource(team_id, "extensions", &format!("{}.json", ext_name), ext_name, config)
    }

    /// List all skills in the repository
    pub fn list_skills(&self, team_id: &str) -> TeamResult<Vec<String>> {
        let repo_path = self.repo_path(team_id);
        let skills_dir = repo_path.join("skills");

        if !skills_dir.exists() {
            return Ok(vec![]);
        }

        let mut skills = vec![];
        for entry in std::fs::read_dir(&skills_dir).map_err(|e| TeamError::SyncFailed {
            reason: format!("Failed to read skills directory: {}", e),
        })? {
            let entry = entry.map_err(|e| TeamError::SyncFailed {
                reason: format!("Failed to read entry: {}", e),
            })?;

            let path = entry.path();
            if path.extension().map(|e| e == "md").unwrap_or(false) {
                if let Some(name) = path.file_stem().and_then(|n| n.to_str()) {
                    if name != ".gitkeep" {
                        skills.push(name.to_string());
                    }
                }
            }
        }

        Ok(skills)
    }

    /// Add remote to repository
    pub fn set_remote(&self, team_id: &str, remote_url: &str) -> TeamResult<()> {
        let repo_path = self.repo_path(team_id);
        let repo = Repository::open(&repo_path).map_err(|e| TeamError::SyncFailed {
            reason: format!("Failed to open repository: {}", e),
        })?;

        // Remove existing origin if present
        if repo.find_remote("origin").is_ok() {
            repo.remote_delete("origin")
                .map_err(|e| TeamError::SyncFailed {
                    reason: format!("Failed to delete existing remote: {}", e),
                })?;
        }

        // Add new remote
        repo.remote("origin", remote_url)
            .map_err(|e| TeamError::SyncFailed {
                reason: format!("Failed to add remote: {}", e),
            })?;

        info!("Set remote origin to: {}", remote_url);
        Ok(())
    }

    // ========================================
    // Async wrappers using spawn_blocking
    // ========================================

    /// Run a blocking git operation on a spawn_blocking thread with optional timeout
    async fn run_blocking<T, F>(&self, timeout_secs: Option<u64>, f: F) -> TeamResult<T>
    where
        T: Send + 'static,
        F: FnOnce(GitSync) -> TeamResult<T> + Send + 'static,
    {
        let config = self.config.clone();
        let future = tokio::task::spawn_blocking(move || f(GitSync::with_config(config)));

        let result = if let Some(secs) = timeout_secs {
            tokio::time::timeout(std::time::Duration::from_secs(secs), future)
                .await
                .map_err(|_| TeamError::SyncFailed {
                    reason: format!("Git operation timed out after {} seconds", secs),
                })?
        } else {
            future.await
        };

        result.map_err(|e| TeamError::SyncFailed {
            reason: format!("Spawn blocking error: {}", e),
        })?
    }

    pub async fn init_repo_async(
        &self,
        team_id: &str,
        repo_url: Option<&str>,
    ) -> TeamResult<PathBuf> {
        let team_id = team_id.to_string();
        let repo_url = repo_url.map(|s| s.to_string());
        self.run_blocking(Some(300), move |gs| gs.init_repo(&team_id, repo_url.as_deref()))
            .await
    }

    pub async fn pull_async(&self, team_id: &str) -> TeamResult<SyncStatus> {
        let team_id = team_id.to_string();
        self.run_blocking(Some(120), move |gs| gs.pull(&team_id)).await
    }

    pub async fn push_async(&self, team_id: &str, message: &str) -> TeamResult<()> {
        let team_id = team_id.to_string();
        let message = message.to_string();
        self.run_blocking(Some(120), move |gs| gs.push(&team_id, &message))
            .await
    }

    pub async fn get_status_async(&self, team_id: &str) -> TeamResult<SyncStatus> {
        let team_id = team_id.to_string();
        self.run_blocking(None, move |gs| gs.get_status(&team_id)).await
    }

    pub async fn export_skill_async(
        &self,
        team_id: &str,
        skill_name: &str,
        content: &str,
    ) -> TeamResult<PathBuf> {
        let (team_id, skill_name, content) =
            (team_id.to_string(), skill_name.to_string(), content.to_string());
        self.run_blocking(None, move |gs| gs.export_skill(&team_id, &skill_name, &content))
            .await
    }

    pub async fn export_recipe_async(
        &self,
        team_id: &str,
        recipe_name: &str,
        content: &str,
    ) -> TeamResult<PathBuf> {
        let (team_id, recipe_name, content) =
            (team_id.to_string(), recipe_name.to_string(), content.to_string());
        self.run_blocking(None, move |gs| gs.export_recipe(&team_id, &recipe_name, &content))
            .await
    }

    pub async fn export_extension_async(
        &self,
        team_id: &str,
        ext_name: &str,
        config_str: &str,
    ) -> TeamResult<PathBuf> {
        let (team_id, ext_name, config_str) =
            (team_id.to_string(), ext_name.to_string(), config_str.to_string());
        self.run_blocking(None, move |gs| gs.export_extension(&team_id, &ext_name, &config_str))
            .await
    }

    pub async fn set_remote_async(&self, team_id: &str, remote_url: &str) -> TeamResult<()> {
        let (team_id, remote_url) = (team_id.to_string(), remote_url.to_string());
        self.run_blocking(None, move |gs| gs.set_remote(&team_id, &remote_url))
            .await
    }

    pub async fn list_skills_async(&self, team_id: &str) -> TeamResult<Vec<String>> {
        let team_id = team_id.to_string();
        self.run_blocking(None, move |gs| gs.list_skills(&team_id)).await
    }
}

impl Default for GitSync {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_init_local_repo() {
        let temp_dir = TempDir::new().unwrap();
        let config = GitSyncConfig {
            base_dir: temp_dir.path().to_path_buf(),
            ..Default::default()
        };

        let git_sync = GitSync::with_config(config);
        let result = git_sync.init_repo("test-team", None);

        assert!(result.is_ok());
        let repo_path = result.unwrap();
        assert!(repo_path.join(".git").exists());
        assert!(repo_path.join("skills").exists());
        assert!(repo_path.join("recipes").exists());
        assert!(repo_path.join("extensions").exists());
        assert!(repo_path.join("README.md").exists());
    }

    #[test]
    fn test_export_skill() {
        let temp_dir = TempDir::new().unwrap();
        let config = GitSyncConfig {
            base_dir: temp_dir.path().to_path_buf(),
            ..Default::default()
        };

        let git_sync = GitSync::with_config(config);
        git_sync.init_repo("test-team", None).unwrap();

        let skill_content = "# Test Skill\n\nThis is a test skill.";
        let result = git_sync.export_skill("test-team", "test-skill", skill_content);

        assert!(result.is_ok());
        let skill_path = result.unwrap();
        assert!(skill_path.exists());

        let content = std::fs::read_to_string(&skill_path).unwrap();
        assert_eq!(content, skill_content);
    }

    #[test]
    fn test_list_skills() {
        let temp_dir = TempDir::new().unwrap();
        let config = GitSyncConfig {
            base_dir: temp_dir.path().to_path_buf(),
            ..Default::default()
        };

        let git_sync = GitSync::with_config(config);
        git_sync.init_repo("test-team", None).unwrap();

        git_sync
            .export_skill("test-team", "skill1", "# Skill 1")
            .unwrap();
        git_sync
            .export_skill("test-team", "skill2", "# Skill 2")
            .unwrap();

        let skills = git_sync.list_skills("test-team").unwrap();
        assert_eq!(skills.len(), 2);
        assert!(skills.contains(&"skill1".to_string()));
        assert!(skills.contains(&"skill2".to_string()));
    }
}
