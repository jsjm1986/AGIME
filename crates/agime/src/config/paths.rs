use etcetera::{choose_app_strategy, AppStrategy, AppStrategyArgs};
use std::fs;
use std::path::PathBuf;

use crate::config::env_compat::get_env_compat;

pub struct Paths;

impl Paths {
    fn get_dir(dir_type: DirType) -> PathBuf {
        if let Some(test_root) = get_env_compat("PATH_ROOT") {
            let base = PathBuf::from(test_root);
            match dir_type {
                DirType::Config => base.join("config"),
                DirType::Data => base.join("data"),
                DirType::State => base.join("state"),
            }
        } else {
            let strategy = choose_app_strategy(AppStrategyArgs {
                top_level_domain: "AGIME".to_string(),
                author: "AGIME".to_string(),
                app_name: "agime".to_string(),
            })
            .expect("agime requires a home dir");

            match dir_type {
                DirType::Config => strategy.config_dir(),
                DirType::Data => strategy.data_dir(),
                DirType::State => strategy.state_dir().unwrap_or(strategy.data_dir()),
            }
        }
    }

    pub fn config_dir() -> PathBuf {
        Self::get_dir(DirType::Config)
    }

    pub fn data_dir() -> PathBuf {
        Self::get_dir(DirType::Data)
    }

    pub fn state_dir() -> PathBuf {
        Self::get_dir(DirType::State)
    }

    pub fn in_state_dir(subpath: &str) -> PathBuf {
        Self::state_dir().join(subpath)
    }

    pub fn in_config_dir(subpath: &str) -> PathBuf {
        Self::config_dir().join(subpath)
    }

    pub fn in_data_dir(subpath: &str) -> PathBuf {
        Self::data_dir().join(subpath)
    }

    /// Migrate configuration from old Block/agime or Block/goose directories to new AGIME/agime directory
    pub fn migrate_config_directory() -> Result<(), std::io::Error> {
        // Skip migration if PATH_ROOT is set (test environment)
        if get_env_compat("PATH_ROOT").is_some() {
            return Ok(());
        }

        let new_config_dir = Self::config_dir();

        // If new directory already exists, no migration needed
        if new_config_dir.exists() {
            return Ok(());
        }

        // Define old possible paths
        let old_paths = vec![
            // Block/agime path
            choose_app_strategy(AppStrategyArgs {
                top_level_domain: "Block".to_string(),
                author: "Block".to_string(),
                app_name: "agime".to_string(),
            })
            .expect("agime requires a home dir")
            .config_dir(),
            // Block/goose path (legacy)
            choose_app_strategy(AppStrategyArgs {
                top_level_domain: "Block".to_string(),
                author: "Block".to_string(),
                app_name: "goose".to_string(),
            })
            .expect("agime requires a home dir")
            .config_dir(),
        ];

        // Try to find an existing old config directory
        for old_path in old_paths {
            if old_path.exists() {
                // Check for nested "config" subdirectory structure (legacy desktop app layout)
                // Some older versions stored config at Block/goose/config/config.yaml instead of
                // Block/goose/config.yaml. We need to migrate from the nested config directory
                // if it exists, to flatten the structure.
                let nested_config_dir = old_path.join("config");
                let source_path = if nested_config_dir.join("config.yaml").exists() {
                    tracing::info!(
                        "Detected nested config structure at {}, will flatten during migration",
                        nested_config_dir.display()
                    );
                    nested_config_dir
                } else {
                    old_path.clone()
                };

                tracing::info!(
                    "Migrating configuration from {} to {}",
                    source_path.display(),
                    new_config_dir.display()
                );

                // Create parent directory if needed
                if let Some(parent) = new_config_dir.parent() {
                    fs::create_dir_all(parent)?;
                }

                // Copy the directory recursively
                copy_dir_all(&source_path, &new_config_dir)?;

                tracing::info!("Configuration migration completed successfully");
                return Ok(());
            }
        }

        // No old configuration found, nothing to migrate
        Ok(())
    }
}

/// Recursively copy a directory and all its contents
fn copy_dir_all(src: &PathBuf, dst: &PathBuf) -> Result<(), std::io::Error> {
    fs::create_dir_all(dst)?;

    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());

        if ty.is_dir() {
            copy_dir_all(&src_path, &dst_path)?;
        } else {
            fs::copy(&src_path, &dst_path)?;
        }
    }

    Ok(())
}

enum DirType {
    Config,
    Data,
    State,
}
