use anyhow::{anyhow, Result};
use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::time::UNIX_EPOCH;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArtifactType {
    Code,
    Document,
    Config,
    Image,
    Data,
    Other,
}

fn normalize_scope_path(path: &str) -> Option<String> {
    let normalized = normalize_relative_workspace_path(path)?;
    if normalized.is_empty() {
        None
    } else {
        Some(normalized)
    }
}

pub fn constrain_subagent_write_scope(
    parent_scope: &[String],
    requested_scope: &[String],
) -> Vec<String> {
    let parent = parent_scope
        .iter()
        .filter_map(|item| normalize_scope_path(item))
        .collect::<Vec<_>>();
    if parent.is_empty() {
        return requested_scope
            .iter()
            .filter_map(|item| normalize_scope_path(item))
            .collect();
    }
    if requested_scope.is_empty() {
        return parent;
    }
    let mut allowed = requested_scope
        .iter()
        .filter_map(|item| normalize_scope_path(item))
        .filter(|candidate| {
            parent.iter().any(|root| {
                candidate == root
                    || candidate
                        .strip_prefix(root)
                        .is_some_and(|suffix| suffix.starts_with('/'))
            })
        })
        .collect::<Vec<_>>();
    if allowed.is_empty() {
        allowed = parent;
    }
    allowed.sort();
    allowed.dedup();
    allowed
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceFileFingerprint {
    pub size: u64,
    pub modified_ms: u128,
}

pub type WorkspaceSnapshot = HashMap<String, WorkspaceFileFingerprint>;

#[derive(Debug, Clone)]
pub struct ScannedWorkspaceArtifact {
    pub name: String,
    pub relative_path: String,
    pub artifact_type: ArtifactType,
    pub content: Option<String>,
    pub mime_type: Option<String>,
    pub size: i64,
}

const DEFAULT_SCAN_MAX_DEPTH: usize = 6;
const DEFAULT_INLINE_TEXT_LIMIT: u64 = 50 * 1024;

fn validate_path_segment(segment: &str, label: &str) -> Result<()> {
    if segment.is_empty() {
        return Err(anyhow!("{} must not be empty", label));
    }
    if segment == "."
        || segment.contains("..")
        || segment.contains('/')
        || segment.contains('\\')
        || segment.contains('\0')
    {
        return Err(anyhow!(
            "{} contains invalid characters: {:?}",
            label,
            segment
        ));
    }
    Ok(())
}

pub fn create_workspace_dir(root: &str, segments: &[(&str, &str)]) -> Result<String> {
    let mut path = PathBuf::from(root);
    for (segment, label) in segments {
        validate_path_segment(segment, label)?;
        path.push(segment);
    }
    std::fs::create_dir_all(&path)
        .map_err(|e| anyhow!("Failed to create workspace dir {:?}: {}", path, e))?;
    Ok(path.to_string_lossy().to_string())
}

pub fn cleanup_workspace_dir(workspace_path: Option<&str>) -> Result<bool> {
    let path = match workspace_path {
        Some(p) if !p.is_empty() => PathBuf::from(p),
        _ => return Ok(false),
    };
    if !path.exists() {
        return Ok(false);
    }
    std::fs::remove_dir_all(&path)
        .map_err(|e| anyhow!("Failed to remove workspace dir {:?}: {}", path, e))?;
    Ok(true)
}

fn is_hidden_or_temp_file(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    name.starts_with('.')
        || name.starts_with("~$")
        || name.ends_with('~')
        || lower.ends_with(".tmp")
        || lower.ends_with(".temp")
        || lower.ends_with(".swp")
        || lower.ends_with(".swo")
        || lower == ".ds_store"
}

fn is_workspace_noise_dir(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "node_modules"
            | ".pnpm-store"
            | ".yarn"
            | ".npm"
            | ".next"
            | ".nuxt"
            | ".svelte-kit"
            | ".turbo"
            | ".cache"
            | "__pycache__"
            | ".pytest_cache"
            | ".mypy_cache"
            | ".ruff_cache"
            | ".venv"
            | "venv"
            | "env"
            | "dist"
            | "build"
            | "target"
            | "coverage"
            | ".idea"
            | ".vscode"
            | "recovered"
            | "recovery"
            | ".tmp"
            | "_tmp"
            | "tmp"
            | "temp"
    )
}

fn collect_workspace_files(
    base: &Path,
    dir: &Path,
    depth: usize,
    max_depth: usize,
    out: &mut Vec<PathBuf>,
) -> Result<()> {
    let entries = std::fs::read_dir(dir)
        .map_err(|e| anyhow!("Failed to read workspace directory {:?}: {}", dir, e))?;

    for entry in entries {
        let entry = entry.map_err(|e| anyhow!("Failed to read directory entry: {}", e))?;
        let name = entry.file_name().to_string_lossy().to_string();
        if is_hidden_or_temp_file(&name) {
            continue;
        }

        let file_type = entry
            .file_type()
            .map_err(|e| anyhow!("Failed to read file type for {:?}: {}", entry.path(), e))?;

        if file_type.is_symlink() {
            continue;
        }
        if file_type.is_dir() {
            if is_workspace_noise_dir(&name) {
                continue;
            }
            if depth < max_depth {
                collect_workspace_files(base, &entry.path(), depth + 1, max_depth, out)?;
            }
            continue;
        }
        if !file_type.is_file() {
            continue;
        }

        let path = entry.path();
        if path.starts_with(base) {
            out.push(path);
        }
    }

    Ok(())
}

fn file_fingerprint(path: &Path) -> Result<WorkspaceFileFingerprint> {
    let metadata = std::fs::metadata(path)
        .map_err(|e| anyhow!("Failed to read file metadata {:?}: {}", path, e))?;
    let modified_ms = metadata
        .modified()
        .ok()
        .and_then(|m| m.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_millis())
        .unwrap_or_default();
    Ok(WorkspaceFileFingerprint {
        size: metadata.len(),
        modified_ms,
    })
}

pub fn snapshot_workspace_files(workspace_path: &str) -> Result<WorkspaceSnapshot> {
    let base = Path::new(workspace_path);
    if !base.exists() || !base.is_dir() {
        return Ok(HashMap::new());
    }

    let mut files = Vec::new();
    collect_workspace_files(base, base, 0, DEFAULT_SCAN_MAX_DEPTH, &mut files)?;

    let mut snap = HashMap::with_capacity(files.len());
    for path in files {
        let rel = match path.strip_prefix(base) {
            Ok(p) => p,
            Err(_) => continue,
        };
        let rel_str = rel.to_string_lossy().replace('\\', "/");
        let fp = file_fingerprint(&path)?;
        snap.insert(rel_str, fp);
    }
    Ok(snap)
}

fn infer_artifact_type(path: &Path) -> ArtifactType {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    match ext.as_str() {
        "rs" | "py" | "js" | "jsx" | "ts" | "tsx" | "go" | "java" | "kt" | "swift" | "c"
        | "cpp" | "cc" | "h" | "hpp" | "cs" | "php" | "rb" | "sh" | "bash" | "ps1" | "sql"
        | "vue" | "svelte" | "css" | "scss" | "less" | "html" => ArtifactType::Code,
        "md" | "txt" | "doc" | "docx" | "pdf" | "rtf" => ArtifactType::Document,
        "json" | "yaml" | "yml" | "toml" | "ini" | "conf" | "cfg" | "env" | "xml" => {
            ArtifactType::Config
        }
        "png" | "jpg" | "jpeg" | "gif" | "bmp" | "webp" | "svg" | "ico" => ArtifactType::Image,
        "csv" | "tsv" | "parquet" | "xlsx" | "xls" => ArtifactType::Data,
        _ => ArtifactType::Other,
    }
}

fn should_inline_text(path: &Path, size: u64) -> bool {
    if size > DEFAULT_INLINE_TEXT_LIMIT {
        return false;
    }
    matches!(
        infer_artifact_type(path),
        ArtifactType::Code | ArtifactType::Document | ArtifactType::Config | ArtifactType::Data
    )
}

pub fn scan_workspace_artifacts(
    workspace_path: &str,
    before: Option<&WorkspaceSnapshot>,
) -> Result<Vec<ScannedWorkspaceArtifact>> {
    let base = Path::new(workspace_path);
    if !base.exists() || !base.is_dir() {
        return Ok(Vec::new());
    }

    let mut files = Vec::new();
    collect_workspace_files(base, base, 0, DEFAULT_SCAN_MAX_DEPTH, &mut files)?;
    files.sort();

    let mut artifacts = Vec::new();
    for path in files {
        let rel = match path.strip_prefix(base) {
            Ok(p) => p,
            Err(_) => continue,
        };
        let rel_str = rel.to_string_lossy().replace('\\', "/");
        let fp = file_fingerprint(&path)?;

        let changed = before
            .and_then(|b| b.get(&rel_str))
            .map(|old| old != &fp)
            .unwrap_or(true);
        if !changed {
            continue;
        }

        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unnamed")
            .to_string();
        let mime_type = mime_guess::from_path(&path)
            .first_raw()
            .map(|s| s.to_string());
        let content = if should_inline_text(&path, fp.size) {
            match std::fs::read_to_string(&path) {
                Ok(s) => Some(s),
                Err(e) => {
                    tracing::debug!("Cannot read content of {:?}: {}", path, e);
                    None
                }
            }
        } else {
            None
        };

        artifacts.push(ScannedWorkspaceArtifact {
            name,
            relative_path: rel_str,
            artifact_type: infer_artifact_type(&path),
            content,
            mime_type,
            size: fp.size as i64,
        });
    }

    Ok(artifacts)
}

pub fn is_low_signal_artifact_path(relative_path: &str) -> bool {
    let lower = relative_path.trim().replace('\\', "/").to_ascii_lowercase();
    if lower.is_empty() {
        return true;
    }
    if lower.starts_with("recovered/")
        || lower.starts_with("recovery/")
        || lower.starts_with(".tmp/")
        || lower.starts_with("_tmp/")
        || lower.starts_with("tmp/")
        || lower.starts_with("temp/")
        || lower.contains("/recovered/")
        || lower.contains("/recovery/")
        || lower.contains("/.tmp/")
        || lower.contains("/_tmp/")
        || lower.contains("/tmp/")
        || lower.contains("/temp/")
    {
        return true;
    }
    let ext = Path::new(&lower)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default();
    let file_name = Path::new(&lower)
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or_default();
    matches!(
        ext,
        "bat"
            | "cmd"
            | "ps1"
            | "sh"
            | "bash"
            | "tmp"
            | "temp"
            | "pyc"
            | "pyo"
            | "class"
            | "o"
            | "obj"
    ) || file_name.starts_with("recovered-")
        || file_name.starts_with("recovery-")
        || file_name.ends_with(".recover")
}

pub fn normalize_relative_workspace_path(path: &str) -> Option<String> {
    let normalized = path.trim().replace('\\', "/");
    if normalized.is_empty() {
        return None;
    }
    let parsed = Path::new(&normalized);
    if parsed.is_absolute() {
        return None;
    }
    if !parsed
        .components()
        .all(|c| matches!(c, Component::Normal(_)))
    {
        return None;
    }
    Some(normalized)
}

pub fn scan_project_context(project_path: &str, max_total_bytes: usize) -> String {
    let base = Path::new(project_path);
    if !base.is_dir() {
        return String::new();
    }

    let mut out = String::new();
    out.push_str("<project_context>\n");
    out.push_str("## 文件结构\n```\n");
    collect_tree(base, 0, 3, &mut out);
    out.push_str("```\n\n");

    const KEY_FILES: &[&str] = &[
        "README.md",
        "readme.md",
        "README.txt",
        "index.html",
        "package.json",
        "Cargo.toml",
        "pyproject.toml",
        "requirements.txt",
        ".env.example",
        "portal-sdk.js",
        "portal-agent-client.js",
    ];

    let mut remaining = max_total_bytes.saturating_sub(out.len());
    for name in KEY_FILES {
        if remaining < 100 {
            break;
        }
        let file_path = base.join(name);
        if let Ok(content) = std::fs::read_to_string(&file_path) {
            if content.trim().is_empty() {
                continue;
            }
            let truncated: String = content.chars().take(remaining.min(4000)).collect();
            out.push_str(&format!("## {}\n```\n{}\n```\n\n", name, truncated));
            remaining = remaining.saturating_sub(truncated.len() + name.len() + 20);
        }
    }

    out.push_str("</project_context>");
    out
}

fn collect_tree(dir: &Path, depth: usize, max_depth: usize, out: &mut String) {
    if depth >= max_depth {
        return;
    }
    let mut entries: Vec<_> = match std::fs::read_dir(dir) {
        Ok(rd) => rd.filter_map(|e| e.ok()).collect(),
        Err(_) => return,
    };
    entries.sort_by_key(|e| e.file_name());

    let skip = [
        "node_modules",
        ".git",
        "target",
        "__pycache__",
        ".next",
        "dist",
    ];
    let indent = "  ".repeat(depth);
    let mut count = 0;
    for entry in &entries {
        let name = entry.file_name().to_string_lossy().to_string();
        if skip.iter().any(|s| *s == name) {
            continue;
        }
        let is_dir = entry.file_type().is_ok_and(|ft| ft.is_dir());
        if is_dir {
            out.push_str(&format!("{}{}/\n", indent, name));
            collect_tree(&entry.path(), depth + 1, max_depth, out);
        } else {
            out.push_str(&format!("{}{}\n", indent, name));
        }
        count += 1;
        if count >= 80 {
            out.push_str(&format!("{}... (truncated)\n", indent));
            break;
        }
    }
}
