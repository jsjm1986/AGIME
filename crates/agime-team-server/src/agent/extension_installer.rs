//! Team extension installer/runtime resolver for server-side auto-loading.
//!
//! In auto resource mode, team-shared stdio extensions may reference commands
//! that are not yet available on the server. This module resolves runtime
//! command configuration and optionally performs backend-side installation.

use agime_team::models::mongo::Extension;
use agime_team::models::CustomExtensionConfig;
use agime_team::MongoDb;
use anyhow::{anyhow, Context, Result};
use bson::doc;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::process::Command;
use tokio::time::{sleep, Duration};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutoInstallPolicy {
    Enabled,
    Disabled,
}

#[derive(Clone)]
pub struct ExtensionInstaller {
    db: Arc<MongoDb>,
    cache_root: PathBuf,
    policy: AutoInstallPolicy,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
struct InstallSpec {
    /// npm | uvx | pipx | binary_url
    kind: String,
    package: Option<String>,
    version: Option<String>,
    url: Option<String>,
    bin: Option<String>,
    checksum: Option<String>,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    envs: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CachedInstall {
    command: String,
    #[serde(default)]
    args_prefix: Vec<String>,
    #[serde(default)]
    envs: HashMap<String, String>,
}

#[derive(Debug, Clone)]
struct InstallResolution {
    command: String,
    args_prefix: Vec<String>,
    envs: HashMap<String, String>,
}

struct InstallLock {
    path: PathBuf,
}

impl Drop for InstallLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

impl ExtensionInstaller {
    pub fn new(
        db: Arc<MongoDb>,
        cache_root: impl Into<PathBuf>,
        policy: AutoInstallPolicy,
    ) -> Self {
        Self {
            db,
            cache_root: cache_root.into(),
            policy,
        }
    }

    /// Convert a team shared Extension document into a runnable CustomExtensionConfig.
    /// For stdio extensions with missing commands, attempts auto-install if enabled.
    pub async fn resolve_team_extension(
        &self,
        team_id: &str,
        ext: &Extension,
    ) -> Result<CustomExtensionConfig> {
        let mut cfg = self.base_config_from_extension(ext)?;
        if !cfg.ext_type.eq_ignore_ascii_case("stdio") {
            return Ok(cfg);
        }

        if command_exists(&cfg.uri_or_cmd) {
            return Ok(cfg);
        }

        if self.policy == AutoInstallPolicy::Disabled {
            return Err(anyhow!(
                "Extension '{}' command '{}' not found and auto-install is disabled",
                ext.name,
                cfg.uri_or_cmd
            ));
        }

        let ext_id = ext.id.map(|id| id.to_hex()).unwrap_or_default();
        let result: Result<CustomExtensionConfig> = async {
            let spec = parse_install_spec(&ext.config)?
                .ok_or_else(|| anyhow!("Missing install spec for extension '{}'", ext.name))?;

            let install_dir = self.install_dir(team_id, &ext_id, &ext.version);
            fs::create_dir_all(&install_dir).with_context(|| {
                format!(
                    "Failed to create install dir for extension '{}': {}",
                    ext.name,
                    install_dir.display()
                )
            })?;

            // Single-writer install lock per extension version.
            let lock_path = install_dir.join(".install.lock");
            let _lock = acquire_lock(&lock_path, Duration::from_secs(60)).await?;

            // Re-check if another worker has installed while waiting for lock.
            if let Some(cached) = read_cached_install(&install_dir)? {
                if command_exists(&cached.command) {
                    apply_resolution(&mut cfg, cached.command, cached.args_prefix, cached.envs);
                    let _ = self
                        .record_status(
                            team_id,
                            &ext_id,
                            &ext.name,
                            &ext.version,
                            "cached",
                            Some(&cfg.uri_or_cmd),
                            None,
                        )
                        .await;
                    return Ok(cfg);
                }
            }

            self.record_status(
                team_id,
                &ext_id,
                &ext.name,
                &ext.version,
                "installing",
                None,
                None,
            )
            .await
            .ok();

            let resolved = match spec.kind.to_lowercase().as_str() {
                "npm" => self.install_with_npm(&install_dir, &spec).await?,
                "binary_url" => {
                    self.install_binary_url(&install_dir, &ext.name, &spec)
                        .await?
                }
                "uvx" => self.resolve_uvx(&spec)?,
                "pipx" => self.resolve_pipx(&spec)?,
                other => {
                    return Err(anyhow!(
                        "Unsupported install kind '{}' for extension '{}'",
                        other,
                        ext.name
                    ))
                }
            };

            write_cached_install(
                &install_dir,
                &CachedInstall {
                    command: resolved.command.clone(),
                    args_prefix: resolved.args_prefix.clone(),
                    envs: resolved.envs.clone(),
                },
            )?;
            apply_resolution(
                &mut cfg,
                resolved.command,
                resolved.args_prefix,
                resolved.envs,
            );

            self.record_status(
                team_id,
                &ext_id,
                &ext.name,
                &ext.version,
                "ready",
                Some(&cfg.uri_or_cmd),
                None,
            )
            .await
            .ok();

            Ok(cfg)
        }
        .await;

        if let Err(err) = &result {
            let _ = self
                .record_status(
                    team_id,
                    &ext_id,
                    &ext.name,
                    &ext.version,
                    "failed",
                    None,
                    Some(&err.to_string()),
                )
                .await;
        }

        result
    }

    fn base_config_from_extension(&self, ext: &Extension) -> Result<CustomExtensionConfig> {
        let uri_or_cmd = ext
            .config
            .get_str("uri_or_cmd")
            .or_else(|_| ext.config.get_str("uriOrCmd"))
            .unwrap_or_default()
            .to_string();
        if uri_or_cmd.is_empty() {
            return Err(anyhow!(
                "Extension '{}' missing uri_or_cmd in config",
                ext.name
            ));
        }

        let empty_args = Vec::new();
        let args: Vec<String> = ext
            .config
            .get_array("args")
            .unwrap_or(&empty_args)
            .iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect();

        let envs: HashMap<String, String> = ext
            .config
            .get_document("envs")
            .map(|doc| {
                doc.iter()
                    .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                    .collect()
            })
            .unwrap_or_default();

        Ok(CustomExtensionConfig {
            name: ext.name.clone(),
            ext_type: ext.extension_type.clone(),
            uri_or_cmd,
            args,
            envs,
            enabled: true,
            source: Some("team".to_string()),
            source_extension_id: Some(ext.id.map(|id| id.to_hex()).unwrap_or_default()),
        })
    }

    fn install_dir(&self, team_id: &str, ext_id: &str, version: &str) -> PathBuf {
        self.cache_root.join(team_id).join(ext_id).join(version)
    }

    async fn install_with_npm(
        &self,
        install_dir: &Path,
        spec: &InstallSpec,
    ) -> Result<InstallResolution> {
        let package = spec
            .package
            .as_deref()
            .ok_or_else(|| anyhow!("npm install requires package"))?;
        let package_spec = if let Some(ver) = &spec.version {
            // Keep package untouched if already includes explicit version.
            if package.contains('@') && !package.starts_with('@') {
                package.to_string()
            } else {
                format!("{}@{}", package, ver)
            }
        } else {
            package.to_string()
        };

        let output = Command::new("npm")
            .arg("install")
            .arg("--no-audit")
            .arg("--no-fund")
            .arg("--prefix")
            .arg(install_dir)
            .arg(&package_spec)
            .output()
            .await
            .context("Failed to spawn npm for extension install")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(anyhow!("npm install failed: {}", stderr.trim()));
        }

        let bin_name = spec
            .bin
            .clone()
            .unwrap_or_else(|| derive_binary_name_from_package(package));
        let bin_path = install_dir
            .join("node_modules")
            .join(".bin")
            .join(if cfg!(windows) {
                format!("{}.cmd", bin_name)
            } else {
                bin_name
            });
        if !bin_path.exists() {
            return Err(anyhow!(
                "Installed npm package but binary not found: {}",
                bin_path.display()
            ));
        }

        Ok(InstallResolution {
            command: bin_path.to_string_lossy().to_string(),
            args_prefix: spec.args.clone(),
            envs: spec.envs.clone(),
        })
    }

    async fn install_binary_url(
        &self,
        install_dir: &Path,
        extension_name: &str,
        spec: &InstallSpec,
    ) -> Result<InstallResolution> {
        let url = spec
            .url
            .as_deref()
            .ok_or_else(|| anyhow!("binary_url install requires url"))?;
        let response = reqwest::get(url)
            .await
            .with_context(|| format!("Failed to download binary from {}", url))?;
        if !response.status().is_success() {
            return Err(anyhow!(
                "Binary download failed from {} (status {})",
                url,
                response.status()
            ));
        }
        let bytes = response
            .bytes()
            .await
            .context("Failed to read downloaded binary bytes")?;

        if let Some(checksum) = spec.checksum.as_deref() {
            let actual = format!("{:x}", Sha256::digest(&bytes));
            let expected = checksum.trim().to_lowercase().replace("sha256:", "");
            if actual != expected {
                return Err(anyhow!(
                    "Binary checksum mismatch: expected {}, got {}",
                    expected,
                    actual
                ));
            }
        }

        let bin_dir = install_dir.join("bin");
        fs::create_dir_all(&bin_dir)
            .with_context(|| format!("Failed to create {}", bin_dir.display()))?;

        let bin_name = spec
            .bin
            .clone()
            .unwrap_or_else(|| sanitize_binary_name(extension_name));
        let bin_path = bin_dir.join(bin_name);
        fs::write(&bin_path, &bytes)
            .with_context(|| format!("Failed to write {}", bin_path.display()))?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(&bin_path)?.permissions();
            perms.set_mode(0o755);
            fs::set_permissions(&bin_path, perms)?;
        }

        Ok(InstallResolution {
            command: bin_path.to_string_lossy().to_string(),
            args_prefix: spec.args.clone(),
            envs: spec.envs.clone(),
        })
    }

    fn resolve_uvx(&self, spec: &InstallSpec) -> Result<InstallResolution> {
        if !command_exists("uvx") {
            return Err(anyhow!("uvx not found in PATH"));
        }
        let package = spec
            .package
            .as_deref()
            .ok_or_else(|| anyhow!("uvx install requires package"))?;
        let package_spec = if let Some(ver) = &spec.version {
            format!("{}@{}", package, ver)
        } else {
            package.to_string()
        };

        let mut args_prefix = vec![package_spec];
        args_prefix.extend(spec.args.clone());

        Ok(InstallResolution {
            command: "uvx".to_string(),
            args_prefix,
            envs: spec.envs.clone(),
        })
    }

    fn resolve_pipx(&self, spec: &InstallSpec) -> Result<InstallResolution> {
        if !command_exists("pipx") {
            return Err(anyhow!("pipx not found in PATH"));
        }
        let package = spec
            .package
            .as_deref()
            .ok_or_else(|| anyhow!("pipx install requires package"))?;
        let package_spec = if let Some(ver) = &spec.version {
            format!("{}=={}", package, ver)
        } else {
            package.to_string()
        };

        let mut args_prefix = vec!["run".to_string(), package_spec];
        args_prefix.extend(spec.args.clone());

        Ok(InstallResolution {
            command: "pipx".to_string(),
            args_prefix,
            envs: spec.envs.clone(),
        })
    }

    async fn record_status(
        &self,
        team_id: &str,
        extension_id: &str,
        extension_name: &str,
        version: &str,
        status: &str,
        command_path: Option<&str>,
        message: Option<&str>,
    ) -> Result<()> {
        let coll: mongodb::Collection<bson::Document> =
            self.db.collection("agent_extension_installs");
        let now = bson::DateTime::from_chrono(Utc::now());
        coll.update_one(
            doc! {
                "team_id": team_id,
                "extension_id": extension_id,
                "version": version,
            },
            doc! {
                "$set": {
                    "team_id": team_id,
                    "extension_id": extension_id,
                    "extension_name": extension_name,
                    "version": version,
                    "status": status,
                    "command_path": command_path.unwrap_or(""),
                    "message": message.unwrap_or(""),
                    "updated_at": &now,
                },
                "$setOnInsert": {
                    "created_at": &now,
                }
            },
            mongodb::options::UpdateOptions::builder()
                .upsert(true)
                .build(),
        )
        .await?;
        Ok(())
    }
}

fn parse_install_spec(config: &bson::Document) -> Result<Option<InstallSpec>> {
    if let Ok(install_doc) = config.get_document("install") {
        let parsed: InstallSpec = bson::from_document(install_doc.clone())
            .context("Failed to parse extension install spec")?;
        return Ok(Some(parsed));
    }
    Ok(None)
}

fn apply_resolution(
    cfg: &mut CustomExtensionConfig,
    command: String,
    args_prefix: Vec<String>,
    envs: HashMap<String, String>,
) {
    cfg.uri_or_cmd = command;
    if !args_prefix.is_empty() {
        let mut args = args_prefix;
        args.extend(cfg.args.clone());
        cfg.args = args;
    }
    for (k, v) in envs {
        cfg.envs.entry(k).or_insert(v);
    }
}

fn read_cached_install(install_dir: &Path) -> Result<Option<CachedInstall>> {
    let marker = install_dir.join("resolved_install.json");
    if !marker.exists() {
        return Ok(None);
    }
    let raw = fs::read_to_string(&marker)
        .with_context(|| format!("Failed to read {}", marker.display()))?;
    let cached: CachedInstall =
        serde_json::from_str(&raw).context("Failed to parse cached install marker")?;
    Ok(Some(cached))
}

fn write_cached_install(install_dir: &Path, cached: &CachedInstall) -> Result<()> {
    let marker = install_dir.join("resolved_install.json");
    let raw = serde_json::to_string_pretty(cached).context("Serialize cached install marker")?;
    fs::write(&marker, raw).with_context(|| format!("Failed to write {}", marker.display()))
}

async fn acquire_lock(lock_path: &Path, timeout: Duration) -> Result<InstallLock> {
    let start = std::time::Instant::now();
    loop {
        match OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(lock_path)
        {
            Ok(_) => {
                return Ok(InstallLock {
                    path: lock_path.to_path_buf(),
                });
            }
            Err(e) if e.kind() == ErrorKind::AlreadyExists => {
                if start.elapsed() >= timeout {
                    return Err(anyhow!(
                        "Timed out waiting install lock: {}",
                        lock_path.display()
                    ));
                }
                sleep(Duration::from_millis(200)).await;
            }
            Err(e) => {
                return Err(anyhow!(
                    "Failed to acquire install lock {}: {}",
                    lock_path.display(),
                    e
                ))
            }
        }
    }
}

fn derive_binary_name_from_package(package: &str) -> String {
    let last = package.rsplit('/').next().unwrap_or(package);
    last.trim_start_matches('@')
        .split('@')
        .next()
        .unwrap_or(last)
        .to_string()
}

fn sanitize_binary_name(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect()
}

fn command_exists(cmd: &str) -> bool {
    let cmd_path = Path::new(cmd);
    if cmd_path.components().count() > 1 {
        return cmd_path.exists();
    }

    let path_var = match std::env::var_os("PATH") {
        Some(v) => v,
        None => return false,
    };

    #[cfg(windows)]
    let exts: Vec<String> = std::env::var("PATHEXT")
        .unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".to_string())
        .split(';')
        .map(|s| s.to_string())
        .collect();

    for dir in std::env::split_paths(&path_var) {
        let base = dir.join(cmd);
        if base.exists() {
            return true;
        }
        #[cfg(windows)]
        {
            for ext in &exts {
                let candidate = dir.join(format!("{}{}", cmd, ext));
                if candidate.exists() {
                    return true;
                }
            }
        }
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_install_spec_from_document() {
        let config = bson::doc! {
            "install": {
                "kind": "npm",
                "package": "@scope/my-ext",
                "version": "1.2.3",
                "bin": "my-ext"
            }
        };
        let spec = parse_install_spec(&config)
            .expect("parse should succeed")
            .expect("install spec should exist");
        assert_eq!(spec.kind, "npm");
        assert_eq!(spec.package.as_deref(), Some("@scope/my-ext"));
        assert_eq!(spec.version.as_deref(), Some("1.2.3"));
        assert_eq!(spec.bin.as_deref(), Some("my-ext"));
    }

    #[test]
    fn derive_binary_name_handles_scope_and_version() {
        assert_eq!(
            derive_binary_name_from_package("@scope/toolkit"),
            "toolkit".to_string()
        );
        assert_eq!(
            derive_binary_name_from_package("toolkit@2.0.0"),
            "toolkit".to_string()
        );
        assert_eq!(
            derive_binary_name_from_package("org/toolkit"),
            "toolkit".to_string()
        );
    }

    #[test]
    fn apply_resolution_prepends_args() {
        let mut cfg = CustomExtensionConfig {
            name: "demo".to_string(),
            ext_type: "stdio".to_string(),
            uri_or_cmd: "old".to_string(),
            args: vec!["--foo".to_string()],
            envs: HashMap::new(),
            enabled: true,
            source: None,
            source_extension_id: None,
        };
        let mut envs = HashMap::new();
        envs.insert("A".to_string(), "1".to_string());
        apply_resolution(
            &mut cfg,
            "new-cmd".to_string(),
            vec!["run".to_string()],
            envs,
        );
        assert_eq!(cfg.uri_or_cmd, "new-cmd");
        assert_eq!(cfg.args, vec!["run".to_string(), "--foo".to_string()]);
        assert_eq!(cfg.envs.get("A").map(String::as_str), Some("1"));
    }
}
