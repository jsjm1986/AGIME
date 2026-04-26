use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{anyhow, Context, Result};

use super::chat_channels::{ChatChannelDoc, ChatChannelRepoStorageMode};
use super::workspace_service::{WorkspaceBinding, WorkspaceService};

const DEFAULT_BRANCH: &str = "main";
const BOT_USER_NAME: &str = "AGIME Collaboration Bot";
const BOT_USER_EMAIL: &str = "agime-bot@local";

#[derive(Debug, Clone)]
pub struct ChannelProjectWorkspaceBinding {
    pub workspace: WorkspaceBinding,
    pub workspace_display_name: String,
    pub repo_path: String,
    pub main_checkout_path: String,
    pub bare_repo_path: String,
    pub repo_default_branch: String,
    pub repo_storage_mode: ChatChannelRepoStorageMode,
}

#[derive(Debug, Clone)]
pub struct ThreadWorktreeBinding {
    pub worktree_path: String,
    pub branch: String,
    pub repo_ref: String,
}

#[derive(Debug, Clone)]
pub struct ChannelProjectWorkspaceService {
    workspace_service: WorkspaceService,
}

#[derive(Debug, Clone)]
struct RepositoryLayout {
    repo_path: String,
    bare_repo_path: String,
    main_checkout_path: String,
}

impl ChannelProjectWorkspaceService {
    pub fn new(workspace_root: impl Into<String>) -> Self {
        Self {
            workspace_service: WorkspaceService::new(workspace_root),
        }
    }

    pub fn ensure_channel_project_workspace(
        &self,
        team_id: &str,
        channel_id: &str,
        channel_name: &str,
        workspace_display_name: Option<&str>,
        repo_default_branch: Option<&str>,
    ) -> Result<ChannelProjectWorkspaceBinding> {
        let display_name = workspace_display_name
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or(channel_name)
            .to_string();
        let branch = sanitize_branch_component(repo_default_branch.unwrap_or(DEFAULT_BRANCH));
        let workspace = self.workspace_service.ensure_channel_project_workspace(
            team_id,
            channel_id,
            &display_name,
        )?;
        let repo_layout = Self::ensure_repository_layout(&workspace.root_path, &branch)?;
        Ok(ChannelProjectWorkspaceBinding {
            workspace,
            workspace_display_name: display_name,
            repo_path: repo_layout.repo_path,
            main_checkout_path: repo_layout.main_checkout_path,
            bare_repo_path: repo_layout.bare_repo_path,
            repo_default_branch: branch,
            repo_storage_mode: ChatChannelRepoStorageMode::BareRepoWorktree,
        })
    }

    pub fn ensure_thread_worktree(
        &self,
        channel: &ChatChannelDoc,
        thread_root_id: &str,
    ) -> Result<ThreadWorktreeBinding> {
        let project_root = channel
            .workspace_path
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| anyhow!("channel is missing workspace_path"))?;
        let main_checkout_path = channel
            .main_checkout_path
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .or_else(|| {
                channel
                    .repo_root
                    .as_deref()
                    .filter(|value| !value.trim().is_empty())
            })
            .map(str::to_string)
            .or_else(|| {
                channel.repo_path.as_deref().map(|repo_path| {
                    Path::new(repo_path)
                        .join("main")
                        .to_string_lossy()
                        .to_string()
                })
            });
        let main_checkout_path = main_checkout_path
            .as_deref()
            .ok_or_else(|| anyhow!("channel is missing main_checkout_path"))?;
        let default_branch = channel
            .repo_default_branch
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or(DEFAULT_BRANCH);
        let bare_repo = Self::bare_repo_path(project_root);
        let worktree_path = Self::thread_worktree_path(project_root, thread_root_id);
        let branch = Self::thread_branch_name(&channel.channel_id, thread_root_id);
        let worktree_dir = Path::new(&worktree_path);
        if !worktree_dir.join(".git").exists() {
            fs::create_dir_all(worktree_dir.parent().unwrap_or(worktree_dir))
                .with_context(|| format!("failed to create worktree parent for {worktree_path}"))?;
            if Self::local_branch_exists(main_checkout_path, &branch)? {
                Self::run_git(
                    main_checkout_path,
                    &["worktree", "add", &worktree_path, &branch],
                )?;
            } else {
                Self::run_git(
                    main_checkout_path,
                    &[
                        "worktree",
                        "add",
                        "-b",
                        &branch,
                        &worktree_path,
                        default_branch,
                    ],
                )?;
            }
        }
        Self::ensure_managed_paths_are_ignored(&worktree_path)?;
        Ok(ThreadWorktreeBinding {
            worktree_path,
            branch,
            repo_ref: bare_repo,
        })
    }

    pub fn thread_branch_name(channel_id: &str, thread_root_id: &str) -> String {
        format!(
            "channel/{}/thread/{}",
            shorten_id(channel_id, 8),
            sanitize_branch_component(thread_root_id)
        )
    }

    fn ensure_repository_layout(
        project_root: &str,
        default_branch: &str,
    ) -> Result<RepositoryLayout> {
        let repo_container = Path::new(project_root).join("repo");
        let bare_repo = repo_container.join("bare.git");
        let main_checkout = repo_container.join("main");
        let worktrees_root = repo_container.join("worktrees");
        fs::create_dir_all(&repo_container)?;
        fs::create_dir_all(&worktrees_root)?;

        if !bare_repo.join("HEAD").exists() {
            Self::run_git_inherit(&["init", "--bare", bare_repo.to_string_lossy().as_ref()])?;
        }

        if !main_checkout.join(".git").exists() {
            Self::run_git_inherit(&[
                "clone",
                bare_repo.to_string_lossy().as_ref(),
                main_checkout.to_string_lossy().as_ref(),
            ])?;
        }

        Self::run_git(
            main_checkout.to_string_lossy().as_ref(),
            &["config", "user.name", BOT_USER_NAME],
        )?;
        Self::run_git(
            main_checkout.to_string_lossy().as_ref(),
            &["config", "user.email", BOT_USER_EMAIL],
        )?;

        if !Self::head_exists(main_checkout.to_string_lossy().as_ref())? {
            Self::run_git(
                main_checkout.to_string_lossy().as_ref(),
                &["checkout", "--orphan", default_branch],
            )?;
            Self::run_git(
                main_checkout.to_string_lossy().as_ref(),
                &[
                    "commit",
                    "--allow-empty",
                    "-m",
                    "Initialize channel workspace",
                ],
            )?;
            let _ = Self::run_git(
                main_checkout.to_string_lossy().as_ref(),
                &["push", "origin", default_branch],
            );
        } else if Self::local_branch_exists(
            main_checkout.to_string_lossy().as_ref(),
            default_branch,
        )? {
            Self::run_git(
                main_checkout.to_string_lossy().as_ref(),
                &["checkout", default_branch],
            )?;
        } else {
            Self::run_git(
                main_checkout.to_string_lossy().as_ref(),
                &["checkout", "-b", default_branch],
            )?;
        }

        Ok(RepositoryLayout {
            repo_path: repo_container.to_string_lossy().to_string(),
            bare_repo_path: bare_repo.to_string_lossy().to_string(),
            main_checkout_path: main_checkout.to_string_lossy().to_string(),
        })
    }

    fn head_exists(repo_root: &str) -> Result<bool> {
        Ok(Command::new("git")
            .args(["-C", repo_root, "rev-parse", "--verify", "HEAD"])
            .output()
            .with_context(|| format!("failed to run git in {repo_root}"))?
            .status
            .success())
    }

    fn local_branch_exists(repo_root: &str, branch: &str) -> Result<bool> {
        Ok(Command::new("git")
            .args([
                "-C",
                repo_root,
                "show-ref",
                "--verify",
                "--quiet",
                &format!("refs/heads/{branch}"),
            ])
            .output()
            .with_context(|| format!("failed to inspect local branch {branch} in {repo_root}"))?
            .status
            .success())
    }

    fn ensure_managed_paths_are_ignored(worktree_path: &str) -> Result<()> {
        let git_dir = Self::run_git(worktree_path, &["rev-parse", "--git-dir"])?;
        let exclude_path = PathBuf::from(git_dir.trim()).join("info").join("exclude");
        let existing = fs::read_to_string(&exclude_path).unwrap_or_default();
        let mut lines = existing
            .lines()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToString::to_string)
            .collect::<Vec<_>>();
        for pattern in [
            "workspace.json",
            "attachments/",
            "artifacts/",
            "notes/",
            "runs/",
        ] {
            if !lines.iter().any(|value| value == pattern) {
                lines.push(pattern.to_string());
            }
        }
        if let Some(parent) = exclude_path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(exclude_path, lines.join("\n") + "\n")?;
        Ok(())
    }

    fn thread_worktree_path(project_root: &str, thread_root_id: &str) -> String {
        Path::new(project_root)
            .join("repo")
            .join("worktrees")
            .join(sanitize_path_segment(thread_root_id))
            .to_string_lossy()
            .to_string()
    }

    fn bare_repo_path(project_root: &str) -> String {
        Path::new(project_root)
            .join("repo")
            .join("bare.git")
            .to_string_lossy()
            .to_string()
    }

    fn run_git(repo_root: &str, args: &[&str]) -> Result<String> {
        let output = Command::new("git")
            .args(["-C", repo_root])
            .args(args)
            .output()
            .with_context(|| format!("failed to execute git in {repo_root}"))?;
        if !output.status.success() {
            return Err(anyhow!(
                "git failed in {} with args {:?}: {}",
                repo_root,
                args,
                String::from_utf8_lossy(&output.stderr)
            ));
        }
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }

    fn run_git_inherit(args: &[&str]) -> Result<String> {
        let output = Command::new("git")
            .args(args)
            .output()
            .with_context(|| format!("failed to execute git {:?}", args))?;
        if !output.status.success() {
            return Err(anyhow!(
                "git failed with args {:?}: {}",
                args,
                String::from_utf8_lossy(&output.stderr)
            ));
        }
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }
}

fn sanitize_path_segment(value: &str) -> String {
    value
        .chars()
        .map(|ch| match ch {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' | ' ' => '-',
            _ => ch,
        })
        .collect::<String>()
        .trim_matches('-')
        .to_string()
}

fn sanitize_branch_component(value: &str) -> String {
    let normalized = sanitize_path_segment(value);
    if normalized.is_empty() {
        "main".to_string()
    } else {
        normalized
    }
}

fn shorten_id(value: &str, max_len: usize) -> String {
    sanitize_branch_component(value)
        .chars()
        .take(max_len)
        .collect::<String>()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thread_branch_name_is_stable_and_sanitized() {
        let branch = ChannelProjectWorkspaceService::thread_branch_name(
            "channel-1234-abcd",
            "thread/with weird:chars",
        );
        assert_eq!(branch, "channel/channel-/thread/thread-with-weird-chars");
    }

    #[test]
    fn sanitize_path_segment_rewrites_reserved_chars() {
        assert_eq!(
            sanitize_path_segment("feat: implement/worktree?"),
            "feat--implement-worktree"
        );
    }
}
