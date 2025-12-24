use etcetera::{choose_app_strategy, AppStrategy, AppStrategyArgs};
use std::fs;
use std::path::PathBuf;

use crate::config::env_compat::get_env_compat;

pub struct Paths;

impl Paths {
    fn get_dir(dir_type: DirType) -> PathBuf {
        if let Some(path_root) = get_env_compat("PATH_ROOT") {
            // When PATH_ROOT is set, use it directly as the base directory
            let base = PathBuf::from(&path_root);
            match dir_type {
                DirType::Config => base.clone(),
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
                // IMPORTANT: etcetera's config_dir() returns path with "/config" suffix on Windows
                // We want config files in the app root directory, so we use data_dir's parent
                // Example: data_dir = .../AGIME/agime/data, parent = .../AGIME/agime
                DirType::Config => {
                    let data_dir = strategy.data_dir();
                    data_dir.parent()
                        .map(|p| p.to_path_buf())
                        .unwrap_or(data_dir)
                }
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

    /// Clean up the nested config directory issue.
    ///
    /// Historical bug: PATH_ROOT used to add a "config" subdirectory, creating:
    ///   PATH_ROOT/config/config.yaml instead of PATH_ROOT/config.yaml
    ///
    /// This function:
    /// 1. Merges any config from the nested directory to the main directory
    /// 2. Removes the nested "config" directory if it becomes empty
    pub fn cleanup_nested_config_directory() -> Result<(), std::io::Error> {
        let config_dir = Self::config_dir();
        let nested_config_dir = config_dir.join("config");

        // If no nested config directory exists, nothing to clean up
        if !nested_config_dir.exists() || !nested_config_dir.is_dir() {
            return Ok(());
        }

        let main_config = config_dir.join("config.yaml");
        let nested_config = nested_config_dir.join("config.yaml");

        // If nested config.yaml exists, we need to handle the merge
        if nested_config.exists() {
            let main_size = fs::metadata(&main_config).map(|m| m.len()).unwrap_or(0);
            let nested_size = fs::metadata(&nested_config).map(|m| m.len()).unwrap_or(0);

            if !main_config.exists() || nested_size > main_size {
                // Nested config has more data, copy it to main location
                tracing::info!(
                    "Migrating config from nested directory {} to main {}",
                    nested_config.display(),
                    main_config.display()
                );
                fs::copy(&nested_config, &main_config)?;
            }

            // Remove the nested config.yaml
            fs::remove_file(&nested_config)?;
            tracing::info!("Removed nested config file: {}", nested_config.display());
        }

        // Move any other files from nested config dir to main config dir
        if nested_config_dir.exists() {
            for entry in fs::read_dir(&nested_config_dir)? {
                let entry = entry?;
                let src_path = entry.path();
                let file_name = entry.file_name();
                let dst_path = config_dir.join(&file_name);

                // Skip if destination already exists
                if dst_path.exists() {
                    tracing::debug!(
                        "Skipping {} - already exists at {}",
                        src_path.display(),
                        dst_path.display()
                    );
                    continue;
                }

                if src_path.is_file() {
                    fs::copy(&src_path, &dst_path)?;
                    fs::remove_file(&src_path)?;
                    tracing::info!(
                        "Moved {} to {}",
                        src_path.display(),
                        dst_path.display()
                    );
                } else if src_path.is_dir() {
                    copy_dir_all(&src_path, &dst_path)?;
                    fs::remove_dir_all(&src_path)?;
                    tracing::info!(
                        "Moved directory {} to {}",
                        src_path.display(),
                        dst_path.display()
                    );
                }
            }

            // Try to remove the now-empty nested config directory
            match fs::remove_dir(&nested_config_dir) {
                Ok(_) => {
                    tracing::info!(
                        "Removed empty nested config directory: {}",
                        nested_config_dir.display()
                    );
                }
                Err(e) => {
                    // Directory might not be empty if there were files we couldn't move
                    tracing::debug!(
                        "Could not remove nested config directory (may not be empty): {}",
                        e
                    );
                }
            }
        }

        Ok(())
    }

    /// Migrate configuration from old Block/agime or Block/goose directories to new AGIME/agime directory
    pub fn migrate_config_directory() -> Result<(), std::io::Error> {
        // Skip migration if PATH_ROOT is set (test/custom environment)
        if get_env_compat("PATH_ROOT").is_some() {
            return Ok(());
        }

        let new_config_dir = Self::config_dir();

        // Always try to clean up nested config directory issue (even if config dir exists)
        if new_config_dir.exists() {
            Self::cleanup_nested_config_directory()?;
        }

        // If new directory already exists, migration from old locations not needed
        if new_config_dir.exists() {
            return Ok(());
        }

        // Define old possible paths to migrate from
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

#[derive(Debug)]
enum DirType {
    Config,
    Data,
    State,
}
