use super::base::Config;
use super::paths::Paths;
use crate::agents::extension::PLATFORM_EXTENSIONS;
use crate::agents::ExtensionConfig;
use indexmap::IndexMap;
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use serde_yaml::Mapping;
use std::path::PathBuf;
use tracing::warn;
use utoipa::ToSchema;

pub const DEFAULT_EXTENSION: &str = "developer";
pub const DEFAULT_EXTENSION_TIMEOUT: u64 = 300;
pub const DEFAULT_EXTENSION_DESCRIPTION: &str = "";
pub const DEFAULT_DISPLAY_NAME: &str = "Developer";
const EXTENSIONS_CONFIG_KEY: &str = "extensions";

/// Get the embedded Playwright runtime paths if available
/// Returns (node_path, run_js_path) if embedded runtime is found
/// The run.js launcher handles environment isolation to prevent conflicts with user's Node.js
fn get_embedded_playwright_paths() -> Option<(PathBuf, PathBuf)> {
    // Get the executable directory
    let exe_path = std::env::current_exe().ok()?;
    let exe_dir = exe_path.parent()?;

    // Determine the playwright base directory
    // In development: look relative to exe
    // In production (Electron): look in resources directory
    let mut playwright_dirs = vec![
        // Electron production path (Windows/Linux)
        Some(exe_dir.join("resources").join("playwright")),
        // Development path
        Some(exe_dir.join("playwright")),
    ];

    // Add paths that require parent directory (macOS .app bundle and alt dev path)
    if let Some(parent_dir) = exe_dir.parent() {
        // Electron production path (macOS .app bundle)
        playwright_dirs.push(Some(parent_dir.join("Resources").join("playwright")));
        // Alternative development path
        playwright_dirs.push(Some(parent_dir.join("src").join("playwright")));
    }

    // Find the first existing playwright directory
    let playwright_dir = playwright_dirs.into_iter().flatten().find(|p| p.exists())?;

    // Determine platform-specific node binary name
    #[cfg(target_os = "windows")]
    let (platform_dir, node_exe) = ("win-x64", "node.exe");

    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    let (platform_dir, node_exe) = ("darwin-x64", "node");

    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    let (platform_dir, node_exe) = ("darwin-arm64", "node");

    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    let (platform_dir, node_exe) = ("linux-x64", "node");

    #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
    let (platform_dir, node_exe) = ("linux-arm64", "node");

    #[cfg(not(any(
        target_os = "windows",
        all(target_os = "macos", target_arch = "x86_64"),
        all(target_os = "macos", target_arch = "aarch64"),
        all(target_os = "linux", target_arch = "x86_64"),
        all(target_os = "linux", target_arch = "aarch64")
    )))]
    {
        tracing::warn!(
            "Unsupported platform for embedded Playwright: os={}, arch={}",
            std::env::consts::OS,
            std::env::consts::ARCH
        );
        return None;
    }

    // Build paths
    let node_path = playwright_dir
        .join("node")
        .join(platform_dir)
        .join(node_exe);
    // Use run.js launcher which handles environment isolation
    let run_js_path = playwright_dir.join("run.js");
    // Also verify MCP CLI exists (run.js will use it)
    let mcp_cli_path = playwright_dir
        .join("mcp")
        .join("node_modules")
        .join("@playwright")
        .join("mcp")
        .join("cli.js");

    // Verify all required files exist
    if node_path.exists() && run_js_path.exists() && mcp_cli_path.exists() {
        tracing::info!(
            "Found embedded Playwright runtime: node={}, launcher={}",
            node_path.display(),
            run_js_path.display()
        );
        Some((node_path, run_js_path))
    } else {
        tracing::debug!(
            "Embedded Playwright runtime not found: node={} (exists={}), run.js={} (exists={}), mcp={} (exists={})",
            node_path.display(),
            node_path.exists(),
            run_js_path.display(),
            run_js_path.exists(),
            mcp_cli_path.display(),
            mcp_cli_path.exists()
        );
        None
    }
}

/// Create Playwright extension config, preferring embedded runtime if available
fn create_playwright_config() -> ExtensionConfig {
    // Get persistent user data directory for browser state (cookies, login, localStorage)
    let user_data_dir = Paths::data_dir().join("playwright-data");

    // Try to use embedded runtime first
    if let Some((node_path, run_js_path)) = get_embedded_playwright_paths() {
        tracing::info!("Using embedded Playwright runtime with isolated environment");
        ExtensionConfig::Stdio {
            name: "Playwright".to_string(),
            description: "Playwright MCP browser automation - launches isolated browser with persistent cookies and login state (mutually exclusive with Extension Mode)".to_string(),
            cmd: node_path.to_string_lossy().to_string(),
            args: vec![
                run_js_path.to_string_lossy().to_string(),
                "--user-data-dir".to_string(),
                user_data_dir.to_string_lossy().to_string(),
            ],
            envs: Default::default(),
            env_keys: vec![],
            timeout: Some(600), // 10 minutes for browser operations
            bundled: Some(true),
            available_tools: vec![],
        }
    } else {
        // Fallback to npx (requires Node.js to be installed)
        tracing::info!("Using npx for Playwright (embedded runtime not found)");
        ExtensionConfig::Stdio {
            name: "Playwright".to_string(),
            description: "Playwright MCP browser automation - launches isolated browser with persistent cookies and login state (mutually exclusive with Extension Mode)".to_string(),
            cmd: "npx".to_string(),
            args: vec![
                "-y".to_string(),
                "@playwright/mcp@latest".to_string(),
                "--user-data-dir".to_string(),
                user_data_dir.to_string_lossy().to_string(),
            ],
            envs: Default::default(),
            env_keys: vec![],
            timeout: Some(600), // 10 minutes for browser operations
            bundled: Some(true),
            available_tools: vec![],
        }
    }
}

/// Create Playwright Extension Mode config - connects to user's existing browser
/// Requires the "Playwright MCP Bridge" browser extension to be installed
fn create_playwright_extension_mode_config() -> ExtensionConfig {
    // Try to use embedded runtime first
    if let Some((node_path, run_js_path)) = get_embedded_playwright_paths() {
        tracing::info!("Using embedded Playwright runtime (extension mode)");
        ExtensionConfig::Stdio {
            name: "Playwright (Extension Mode)".to_string(),
            description: "Connect to your already logged-in Chrome/Edge browser - requires Playwright MCP Bridge extension (mutually exclusive with Playwright)".to_string(),
            cmd: node_path.to_string_lossy().to_string(),
            args: vec![
                run_js_path.to_string_lossy().to_string(),
                "--extension".to_string(),
            ],
            envs: Default::default(),
            env_keys: vec![],
            timeout: Some(600), // 10 minutes for browser operations
            bundled: Some(true),
            available_tools: vec![],
        }
    } else {
        // Fallback to npx (requires Node.js to be installed)
        tracing::info!("Using npx for Playwright extension mode (embedded runtime not found)");
        ExtensionConfig::Stdio {
            name: "Playwright (Extension Mode)".to_string(),
            description: "Connect to your already logged-in Chrome/Edge browser - requires Playwright MCP Bridge extension (mutually exclusive with Playwright)".to_string(),
            cmd: "npx".to_string(),
            args: vec![
                "-y".to_string(),
                "@playwright/mcp@latest".to_string(),
                "--extension".to_string(),
            ],
            envs: Default::default(),
            env_keys: vec![],
            timeout: Some(600), // 10 minutes for browser operations
            bundled: Some(true),
            available_tools: vec![],
        }
    }
}

/// Bundled stdio extensions that are pre-configured but disabled by default
pub static BUNDLED_STDIO_EXTENSIONS: Lazy<Vec<(&'static str, ExtensionEntry)>> = Lazy::new(|| {
    vec![
        (
            "playwright",
            ExtensionEntry {
                enabled: false, // Disabled by default, user needs to enable
                config: create_playwright_config(),
            },
        ),
        (
            "playwright(extensionmode)", // Must match name_to_key("Playwright (Extension Mode)")
            ExtensionEntry {
                enabled: false, // Disabled by default, requires browser extension
                config: create_playwright_extension_mode_config(),
            },
        ),
    ]
});

#[derive(Debug, Deserialize, Serialize, Clone, ToSchema)]
pub struct ExtensionEntry {
    pub enabled: bool,
    #[serde(flatten)]
    pub config: ExtensionConfig,
}

pub fn name_to_key(name: &str) -> String {
    name.chars()
        .filter(|c| !c.is_whitespace())
        .collect::<String>()
        .to_lowercase()
}

fn get_extensions_map() -> IndexMap<String, ExtensionEntry> {
    let config = Config::global();
    tracing::debug!("Config file path: {}", config.path());

    let raw: Mapping = config
        .get_param(EXTENSIONS_CONFIG_KEY)
        .unwrap_or_else(|err| {
            warn!(
                "Failed to load {}: {err}. Falling back to empty object.",
                EXTENSIONS_CONFIG_KEY
            );
            Default::default()
        });

    tracing::debug!(
        "Loading extensions from config, raw entries count: {}",
        raw.len()
    );

    let mut extensions_map = IndexMap::with_capacity(raw.len());
    for (k, v) in raw {
        match (
            k.clone(),
            serde_yaml::from_value::<ExtensionEntry>(v.clone()),
        ) {
            (serde_yaml::Value::String(s), Ok(entry)) => {
                tracing::debug!("Successfully loaded extension: {}", s);
                extensions_map.insert(s, entry);
            }
            (k, Err(e)) => {
                warn!(
                    key = ?k,
                    error = %e,
                    "Skipping malformed extension config entry - PARSE ERROR"
                );
            }
            (k, v) => {
                warn!(
                    key = ?k,
                    value = ?v,
                    "Skipping malformed extension config entry"
                );
            }
        }
    }

    // Migration: rename old keys to new keys for consistency
    // Old key "playwright-extension-mode" -> new key "playwright(extensionmode)"
    let migrations: &[(&str, &str)] = &[("playwright-extension-mode", "playwright(extensionmode)")];

    let mut needs_save = false;
    for (old_key, new_key) in migrations {
        if extensions_map.contains_key(*old_key) && !extensions_map.contains_key(*new_key) {
            if let Some(entry) = extensions_map.shift_remove(*old_key) {
                tracing::info!("Migrating extension key: {} -> {}", old_key, new_key);
                extensions_map.insert(new_key.to_string(), entry);
                needs_save = true;
            }
        } else if extensions_map.contains_key(*old_key) && extensions_map.contains_key(*new_key) {
            // Both old and new keys exist, remove the old one
            tracing::info!("Removing duplicate old extension key: {}", old_key);
            extensions_map.shift_remove(*old_key);
            needs_save = true;
        }
    }

    // Migration: normalize keys and remove duplicates
    // This fixes issues where the same extension might be stored under different keys
    let mut normalized_map: IndexMap<String, ExtensionEntry> = IndexMap::new();
    let mut seen_normalized_keys: std::collections::HashSet<String> =
        std::collections::HashSet::new();

    for (key, entry) in extensions_map.into_iter() {
        let normalized_key = entry.config.key(); // Uses name_to_key internally

        if seen_normalized_keys.contains(&normalized_key) {
            // Duplicate found - skip this entry
            tracing::warn!(
                "Removing duplicate extension: key='{}', normalized='{}', name='{}'",
                key,
                normalized_key,
                entry.config.name()
            );
            needs_save = true;
            continue;
        }

        seen_normalized_keys.insert(normalized_key.clone());

        // If the stored key differs from the normalized key, migrate it
        if key != normalized_key {
            tracing::info!(
                "Normalizing extension key: '{}' -> '{}'",
                key,
                normalized_key
            );
            needs_save = true;
        }

        normalized_map.insert(normalized_key, entry);
    }

    let mut extensions_map = normalized_map;

    // Save the migrated config if needed
    if needs_save {
        let config = Config::global();
        if let Err(e) = config.set_param(EXTENSIONS_CONFIG_KEY, &extensions_map) {
            tracing::warn!("Failed to save migrated extensions config: {}", e);
        }
    }

    // Always add platform extensions (todo, chatrecall, extensionmanager, skills)
    // regardless of whether user has configured any extensions
    for (name, def) in PLATFORM_EXTENSIONS.iter() {
        if !extensions_map.contains_key(*name) {
            extensions_map.insert(
                name.to_string(),
                ExtensionEntry {
                    config: ExtensionConfig::Platform {
                        name: def.name.to_string(),
                        description: def.description.to_string(),
                        bundled: Some(true),
                        available_tools: Vec::new(),
                    },
                    enabled: true,
                },
            );
        }
    }

    // Add bundled stdio extensions (like Playwright) if not already configured
    for (name, entry) in BUNDLED_STDIO_EXTENSIONS.iter() {
        if !extensions_map.contains_key(*name) {
            extensions_map.insert(name.to_string(), entry.clone());
        }
    }

    tracing::debug!(
        "Total extensions loaded: {} (after adding platform extensions)",
        extensions_map.len()
    );
    extensions_map
}

fn save_extensions_map(extensions: IndexMap<String, ExtensionEntry>) {
    let config = Config::global();
    if let Err(e) = config.set_param(EXTENSIONS_CONFIG_KEY, &extensions) {
        tracing::warn!(
            "Failed to save extensions config: {}. User changes may not persist.",
            e
        );
    }
}

pub fn get_extension_by_name(name: &str) -> Option<ExtensionConfig> {
    let extensions = get_extensions_map();
    extensions
        .values()
        .find(|entry| entry.config.name() == name)
        .map(|entry| entry.config.clone())
}

pub fn set_extension(entry: ExtensionEntry) {
    let mut extensions = get_extensions_map();
    let key = entry.config.key();

    // If enabling this extension, disable mutually exclusive ones
    if entry.enabled {
        let exclusive_keys = get_mutually_exclusive_keys(&key);
        for exclusive_key in exclusive_keys {
            if let Some(exclusive_entry) = extensions.get_mut(exclusive_key) {
                if exclusive_entry.enabled {
                    tracing::info!(
                        "Disabling {} because {} is being enabled (mutually exclusive)",
                        exclusive_key,
                        key
                    );
                    exclusive_entry.enabled = false;
                }
            }
        }
    }

    extensions.insert(key, entry);
    save_extensions_map(extensions);
}

pub fn remove_extension(key: &str) {
    let mut extensions = get_extensions_map();

    // First try to remove by exact key
    if extensions.shift_remove(key).is_some() {
        tracing::info!("Removed extension by key: {}", key);
        save_extensions_map(extensions);
        return;
    }

    // If not found by key, try to find by name (case-insensitive, ignoring whitespace)
    // This handles cases where the stored key doesn't match the normalized key
    let normalized_key = name_to_key(key);
    let keys_to_remove: Vec<String> = extensions
        .iter()
        .filter(|(k, entry)| {
            // Match by normalized key or by extension name
            name_to_key(k) == normalized_key || name_to_key(&entry.config.name()) == normalized_key
        })
        .map(|(k, _)| k.clone())
        .collect();

    if keys_to_remove.is_empty() {
        tracing::warn!("No extension found to remove: {}", key);
        return;
    }

    for k in &keys_to_remove {
        tracing::info!("Removing extension with key: {} (searched for: {})", k, key);
        extensions.shift_remove(k);
    }

    save_extensions_map(extensions);
}

// Mutually exclusive extension groups - when one is enabled, others in the same group are disabled
// Format: each inner slice contains keys that are mutually exclusive with each other
const MUTUALLY_EXCLUSIVE_GROUPS: &[&[&str]] = &[
    // Playwright modes are mutually exclusive - can only use one at a time
    &["playwright", "playwright(extensionmode)"],
];

/// Get the mutually exclusive counterparts for a given extension key
fn get_mutually_exclusive_keys(key: &str) -> Vec<&'static str> {
    for group in MUTUALLY_EXCLUSIVE_GROUPS {
        if group.contains(&key) {
            return group.iter().filter(|&&k| k != key).copied().collect();
        }
    }
    Vec::new()
}

pub fn set_extension_enabled(key: &str, enabled: bool) {
    let mut extensions = get_extensions_map();
    if let Some(entry) = extensions.get_mut(key) {
        entry.enabled = enabled;

        // If enabling this extension, disable mutually exclusive ones
        if enabled {
            let exclusive_keys = get_mutually_exclusive_keys(key);
            for exclusive_key in exclusive_keys {
                if let Some(exclusive_entry) = extensions.get_mut(exclusive_key) {
                    if exclusive_entry.enabled {
                        tracing::info!(
                            "Disabling {} because {} is being enabled (mutually exclusive)",
                            exclusive_key,
                            key
                        );
                        exclusive_entry.enabled = false;
                    }
                }
            }
        }

        save_extensions_map(extensions);
    }
}

pub fn get_all_extensions() -> Vec<ExtensionEntry> {
    let extensions = get_extensions_map();
    let values: Vec<ExtensionEntry> = extensions.into_values().collect();

    // Deduplicate by extension name (keep the first occurrence)
    // This prevents duplicate entries in UI when config has entries with different keys but same name
    let mut seen_names = std::collections::HashSet::new();
    values
        .into_iter()
        .filter(|entry| {
            let name = entry.config.name();
            if seen_names.contains(&name) {
                tracing::warn!(
                    "Duplicate extension name detected: {}. Keeping first occurrence.",
                    name
                );
                false
            } else {
                seen_names.insert(name);
                true
            }
        })
        .collect()
}

pub fn get_all_extension_names() -> Vec<String> {
    let extensions = get_extensions_map();
    extensions.keys().cloned().collect()
}

pub fn is_extension_enabled(key: &str) -> bool {
    let extensions = get_extensions_map();
    extensions.get(key).map(|e| e.enabled).unwrap_or(false)
}

pub fn get_enabled_extensions() -> Vec<ExtensionConfig> {
    get_all_extensions()
        .into_iter()
        .filter(|ext| ext.enabled)
        .map(|ext| ext.config)
        .collect()
}
