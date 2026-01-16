use std::{
    env::{self},
    ffi::{OsStr, OsString},
    path::PathBuf,
    process::Command,
};

use anyhow::{Context, Result};
use tracing::debug;

use crate::config::Config;

pub struct SearchPaths {
    paths: Vec<PathBuf>,
}

impl SearchPaths {
    pub fn builder() -> Self {
        let mut paths = Config::global()
            .get_agime_search_paths()
            .unwrap_or_default();

        paths.push("~/.local/bin".into());

        #[cfg(unix)]
        {
            paths.push("/usr/local/bin".into());
        }

        if cfg!(target_os = "macos") {
            paths.push("/opt/homebrew/bin".into());
            paths.push("/opt/local/bin".into());
        }

        Self {
            paths: paths
                .into_iter()
                .map(|s| PathBuf::from(shellexpand::tilde(&s).as_ref()))
                .collect(),
        }
    }

    pub fn with_npm(mut self) -> Self {
        // Add npm global packages directory
        if cfg!(windows) {
            if let Some(appdata) = dirs::data_dir() {
                self.paths.push(appdata.join("npm"));
            }
        } else if let Some(home) = dirs::home_dir() {
            self.paths.push(home.join(".npm-global/bin"));
        }

        // Hybrid strategy: prefer system Node.js, fallback to embedded
        // 1. Try to detect system Node.js installation
        if let Some(nodejs_dir) = Self::detect_nodejs_dir() {
            debug!("Using system Node.js directory: {:?}", nodejs_dir);
            if !self.paths.contains(&nodejs_dir) {
                self.paths.push(nodejs_dir);
            }
        }
        // 2. Always add embedded Node.js as fallback (if available)
        else if let Some(embedded_dir) = Self::get_embedded_nodejs_dir() {
            debug!("Using embedded Node.js directory: {:?}", embedded_dir);
            if !self.paths.contains(&embedded_dir) {
                self.paths.push(embedded_dir);
            }
        }

        self
    }

    /// Detect system Node.js installation directory by finding the `node` executable
    fn detect_nodejs_dir() -> Option<PathBuf> {
        #[cfg(windows)]
        let output = Command::new("where").arg("node").output().ok()?;

        #[cfg(not(windows))]
        let output = Command::new("which").arg("node").output().ok()?;

        if !output.status.success() {
            return None;
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        // Take the first line (in case of multiple matches)
        let node_path = stdout.lines().next()?.trim();

        if node_path.is_empty() {
            return None;
        }

        // Get the parent directory of the node executable
        PathBuf::from(node_path).parent().map(|p| p.to_path_buf())
    }

    /// Get the embedded Node.js directory if available
    /// This provides a fallback when system Node.js is not installed
    pub fn get_embedded_nodejs_dir() -> Option<PathBuf> {
        // Get the executable directory
        let exe_path = std::env::current_exe().ok()?;
        let exe_dir = exe_path.parent()?;

        // Determine platform-specific subdirectory
        #[cfg(target_os = "windows")]
        let platform_dir = "win-x64";

        #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
        let platform_dir = "darwin-x64";

        #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
        let platform_dir = "darwin-arm64";

        #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
        let platform_dir = "linux-x64";

        #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
        let platform_dir = "linux-arm64";

        #[cfg(not(any(
            target_os = "windows",
            all(target_os = "macos", target_arch = "x86_64"),
            all(target_os = "macos", target_arch = "aarch64"),
            all(target_os = "linux", target_arch = "x86_64"),
            all(target_os = "linux", target_arch = "aarch64")
        )))]
        {
            debug!(
                "Unsupported platform for embedded Node.js: os={}, arch={}",
                std::env::consts::OS,
                std::env::consts::ARCH
            );
            return None;
        }

        // Possible locations for embedded Node.js
        let mut nodejs_dirs: Vec<PathBuf> = vec![
            // Electron production path (Windows/Linux)
            exe_dir.join("resources").join("nodejs").join(platform_dir),
            // Development path
            exe_dir.join("nodejs").join(platform_dir),
        ];

        // Add paths that require parent directory
        if let Some(parent_dir) = exe_dir.parent() {
            // macOS .app bundle
            nodejs_dirs.push(
                parent_dir
                    .join("Resources")
                    .join("nodejs")
                    .join(platform_dir),
            );
            // Alternative development path
            nodejs_dirs.push(parent_dir.join("src").join("nodejs").join(platform_dir));
        }

        // Find the first existing directory with npx
        for dir in nodejs_dirs {
            #[cfg(windows)]
            let npx_path = dir.join("npx.cmd");
            #[cfg(not(windows))]
            let npx_path = dir.join("npx");

            if npx_path.exists() {
                debug!("Found embedded Node.js with npx at: {:?}", dir);
                return Some(dir);
            }
        }

        debug!("No embedded Node.js found");
        None
    }

    pub fn path(self) -> Result<OsString> {
        env::join_paths(
            self.paths.into_iter().chain(
                env::var_os("PATH")
                    .as_ref()
                    .map(env::split_paths)
                    .into_iter()
                    .flatten(),
            ),
        )
        .map_err(Into::into)
    }

    pub fn resolve<N>(self, name: N) -> Result<PathBuf>
    where
        N: AsRef<OsStr>,
    {
        which::which_in_global(name.as_ref(), Some(self.path()?))?
            .next()
            .with_context(|| {
                format!(
                    "could not resolve command '{}': file does not exist",
                    name.as_ref().to_string_lossy()
                )
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_path_preserves_existing_path() {
        let search_paths = SearchPaths::builder();
        let combined_path = search_paths.path().unwrap();

        if let Some(existing_path) = env::var_os("PATH") {
            let combined_str = combined_path.to_string_lossy();
            let existing_str = existing_path.to_string_lossy();

            assert!(combined_str.contains(&existing_str.to_string()));
        }
    }

    #[test]
    fn test_resolve_nonexistent_executable() {
        let search_paths = SearchPaths::builder();

        let result = search_paths.resolve("nonexistent_executable_12345_abcdef");

        assert!(
            result.is_err(),
            "Resolving nonexistent executable should return an error"
        );
    }

    #[test]
    fn test_resolve_common_executable() {
        let search_paths = SearchPaths::builder();

        #[cfg(unix)]
        let test_executable = "sh";

        #[cfg(windows)]
        let test_executable = "cmd";

        search_paths
            .resolve(test_executable)
            .expect("should resolve sh (or cmd on Windows)");
    }

    #[test]
    fn test_detect_nodejs_dir() {
        // This test will pass if Node.js is installed, skip otherwise
        if let Some(nodejs_dir) = SearchPaths::detect_nodejs_dir() {
            // Verify that the detected directory contains node executable
            #[cfg(windows)]
            let node_exe = nodejs_dir.join("node.exe");
            #[cfg(not(windows))]
            let node_exe = nodejs_dir.join("node");

            assert!(
                node_exe.exists(),
                "Detected Node.js directory should contain node executable"
            );
        }
        // If Node.js is not installed, the test passes silently
    }

    #[test]
    fn test_with_npm_includes_nodejs_dir() {
        let search_paths = SearchPaths::builder().with_npm();
        let combined_path = search_paths.path().unwrap();

        // If Node.js is installed, the path should include its directory
        if let Some(nodejs_dir) = SearchPaths::detect_nodejs_dir() {
            let combined_str = combined_path.to_string_lossy();
            let nodejs_str = nodejs_dir.to_string_lossy();
            assert!(
                combined_str.contains(&*nodejs_str),
                "Combined PATH should include Node.js directory"
            );
        }
    }
}
