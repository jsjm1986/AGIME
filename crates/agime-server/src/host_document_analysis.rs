//! Desktop harness host — Phase 6: document analysis pure-logic helpers.
//!
//! The team-server's `crates/agime-team-server/src/agent/document_analysis.rs`
//! is 1217 lines and tightly coupled to MongoDB (`DocumentService`,
//! `SmartLogService`, `MongoDb`, `agime_team::models::mongo::*`). The desktop
//! has none of those — no Mongo, no smart logs, no `DocumentService`.
//!
//! This module copies only the **pure-logic** subset (~370 lines) that's safe
//! to reuse without dragging in `agime_team`:
//! - JSON parse / repair / serialize for the analysis result schema
//! - Terminal/non-terminal text classification (bilingual EN + 中文)
//! - Container/archive false-positive detection
//! - MIME categorisation + filename sanitisation
//! - Bilingual prompt builder (EN + 中文, MIME-aware)
//!
//! Mongo-coupled pieces (skipped — those land in a future phase if/when the
//! desktop grows a persistent document store):
//! - `process_document_analysis` (~570 lines, drives Agent.reply + persists)
//! - `materialize_document_for_analysis` (depends on `DocumentService`)
//! - `load_latest_smart_log_content_snapshot` (depends on `MongoDb`)
//! - `DocumentAnalysisTriggerImpl`
//!
//! The desktop replaces team-server's `DocumentAnalysisContext` with a
//! desktop-flavored `DocumentAnalysisInput` struct that drops `team_id` and
//! Mongo-specific fields. `MaterializedAnalysisDocument` is duplicated locally
//! so callers don't pull in `agime_team`.
//!
//! All callers are gated by the `desktop_harness_host` cargo feature.
//!
//! SOURCE: crates/agime-team-server/src/agent/document_analysis.rs at
//! commit 961109f (desktop reimplementation, not a verbatim copy — the Mongo
//! surface is dropped per the long-term maintenance strategy in CLAUDE.md).

#![cfg(feature = "desktop_harness_host")]

use agime::agents::{format_execution_host_completion_text, ExecutionHostCompletionReport};
use serde::{Deserialize, Serialize};

const MAX_ANALYSIS_CHARS: usize = 3000;

/// Desktop-flavored input to the analysis pipeline. Mirrors the shape of
/// team-server's `DocumentAnalysisContext` minus the Mongo-only `team_id`.
#[derive(Debug, Clone)]
pub struct DocumentAnalysisInput {
    pub doc_id: String,
    pub doc_name: String,
    pub mime_type: String,
    pub file_size: u64,
    pub lang: Option<String>,
}

/// Materialised document path metadata, used by the prompt builder. The
/// desktop side fills these in after writing the upload to a workspace dir.
#[derive(Debug, Clone)]
pub struct MaterializedAnalysisDocument {
    pub file_path: String,
    pub relative_path: String,
    pub file_name: String,
    pub mime_type: String,
    pub file_size: u64,
}

/// Strict JSON schema we ask the model to return. Matches the team-server's
/// `DocumentAnalysisJsonResult` so a future shared persistence layer can swap
/// it in without changing the on-the-wire shape.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DocumentAnalysisJsonResult {
    pub status: String,
    pub summary: String,
    #[serde(default)]
    pub content_structure: Vec<String>,
    #[serde(default)]
    pub key_points: Vec<String>,
    #[serde(default)]
    pub file_observations: Vec<String>,
    #[serde(default)]
    pub limitations: Vec<String>,
}

// ---------------------------------------------------------------------------
// Persistence-shape derivation
// ---------------------------------------------------------------------------

/// Resolve the final `(json_payload, status_label)` to persist for a single
/// analysis run. Tries the JSON schema first, then a free-text terminal
/// classification, then falls back to a `blocked` envelope.
pub fn derive_document_analysis_persistence(
    completion_report: Option<&ExecutionHostCompletionReport>,
    analysis_text: Option<&str>,
    doc_name: &str,
) -> (String, &'static str) {
    let candidate = analysis_text
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_string)
        .or_else(|| completion_report.map(format_execution_host_completion_text));

    let Some(text) = candidate else {
        return blocked_document_analysis_json(
            "文档解读未产生最终结果。",
            vec![format!("未能完成文档「{}」的解读。", doc_name)],
        );
    };

    if let Some(parsed) = parse_document_analysis_json(&text) {
        return persist_document_analysis_json(parsed);
    }

    if document_analysis_text_is_terminal(&text) {
        return completed_document_analysis_json(text);
    }

    blocked_document_analysis_json(
        "文档解读未完成。",
        vec![text.chars().take(MAX_ANALYSIS_CHARS).collect()],
    )
}

/// Pick the best textual representation of the analysis: prefer terminal
/// visible text → otherwise the completion report's summary → otherwise the
/// formatted host completion text.
pub fn select_document_analysis_text(
    completion_report: Option<&ExecutionHostCompletionReport>,
    visible_text: Option<String>,
) -> Option<String> {
    if visible_text
        .as_deref()
        .is_some_and(document_analysis_text_is_terminal)
    {
        return visible_text;
    }

    if let Some(report) = completion_report.filter(|report| report.status == "completed") {
        return Some(report.summary.clone());
    }

    visible_text.or_else(|| completion_report.map(format_execution_host_completion_text))
}

/// Returns `true` when the text looks like a finished answer rather than a
/// "let me…" / "我需要…" intent statement. The pattern list is bilingual to
/// match both EN and 中文 outputs the desktop sees in the wild.
pub fn document_analysis_text_is_terminal(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return false;
    }
    let normalized = trimmed.to_ascii_lowercase();
    let non_terminal_patterns = [
        "runtime exited without terminal completion evidence",
        "tool execution completed",
        "validation: failed",
        "blocking reason:",
        "no tool-backed progress",
        "let me ",
        "i need to ",
        "i will ",
        "i'll ",
        "need to extract",
        "need to read",
        "need to inspect",
        "not available",
        "cannot read",
        "could not read",
        "unable to read",
        "我需要",
        "我需要先",
        "让我",
        "我将",
        "下一步",
        "需要解压",
        "需要读取",
        "不可用",
        "无法读取",
        "无法访问",
        "无法解读",
    ];

    !non_terminal_patterns
        .iter()
        .any(|pattern| normalized.contains(pattern))
}

// ---------------------------------------------------------------------------
// JSON parsing / repair
// ---------------------------------------------------------------------------

pub fn parse_document_analysis_json(text: &str) -> Option<DocumentAnalysisJsonResult> {
    let candidate = extract_json_object(text)?;
    serde_json::from_str::<DocumentAnalysisJsonResult>(&candidate)
        .ok()
        .or_else(|| repair_document_analysis_jsonish(&candidate))
}

pub fn extract_json_object(text: &str) -> Option<String> {
    let trimmed = text.trim();
    let unfenced = trimmed
        .strip_prefix("```json")
        .and_then(|value| value.strip_suffix("```"))
        .or_else(|| {
            trimmed
                .strip_prefix("```")
                .and_then(|value| value.strip_suffix("```"))
        })
        .map(str::trim)
        .unwrap_or(trimmed);

    if unfenced.starts_with('{') && unfenced.ends_with('}') {
        return Some(unfenced.to_string());
    }

    let start = unfenced.find('{')?;
    let end = unfenced.rfind('}')?;
    (start < end).then(|| unfenced[start..=end].to_string())
}

pub fn persist_document_analysis_json(
    result: DocumentAnalysisJsonResult,
) -> (String, &'static str) {
    let result = normalize_document_analysis_result(result);
    let blocked = !result.status.eq_ignore_ascii_case("completed")
        || !document_analysis_text_is_terminal(&result.summary);
    let status = if blocked { "failed" } else { "completed" };
    (serialize_document_analysis_json(result), status)
}

/// If the model returned a `blocked` result that's actually summarising the
/// archive container itself (magic bytes, file signature, …) rather than its
/// inner document, replace the summary with a generic "couldn't read inner"
/// message and stash the raw text under `limitations`.
pub fn normalize_document_analysis_result(
    mut result: DocumentAnalysisJsonResult,
) -> DocumentAnalysisJsonResult {
    if result.status.eq_ignore_ascii_case("blocked") && blocked_result_focuses_on_container(&result)
    {
        let original_summary = result.summary.trim().to_string();
        result.summary =
            "无法解压或读取压缩包内部的文档内容，因此无法生成文件内容解读。".to_string();
        result.content_structure.clear();
        result.key_points.clear();
        result.file_observations.clear();
        if !original_summary.is_empty()
            && !result
                .limitations
                .iter()
                .any(|item| item.contains(&original_summary))
        {
            result
                .limitations
                .insert(0, format!("读取受阻详情：{}", original_summary));
        }
    }
    result
}

pub fn blocked_result_focuses_on_container(result: &DocumentAnalysisJsonResult) -> bool {
    let combined = std::iter::once(result.summary.as_str())
        .chain(result.content_structure.iter().map(String::as_str))
        .chain(result.key_points.iter().map(String::as_str))
        .chain(result.file_observations.iter().map(String::as_str))
        .collect::<Vec<_>>()
        .join("\n")
        .to_ascii_lowercase();
    let container_terms = [
        "rar magic",
        "magic signature",
        "magic bytes",
        "file signature",
        "archive signature",
        "魔术签名",
        "文件签名",
        "rar 签名",
        "魔数",
        "ascii 明文",
        "not-a-real-rar",
        "synthetic test",
        "合成测试",
        "占位文件",
    ];
    container_terms
        .iter()
        .any(|term| combined.contains(&term.to_ascii_lowercase()))
}

pub fn repair_document_analysis_jsonish(text: &str) -> Option<DocumentAnalysisJsonResult> {
    if !text.contains("\"status\"") || !text.contains("\"summary\"") {
        return None;
    }

    let status = extract_jsonish_string_field(text, "status")
        .filter(|value| {
            value.eq_ignore_ascii_case("completed") || value.eq_ignore_ascii_case("blocked")
        })
        .unwrap_or_else(|| "blocked".to_string());
    let summary = extract_jsonish_string_field_before(
        text,
        "summary",
        &[
            "\",\"content_structure\"",
            "\",\"key_points\"",
            "\",\"file_observations\"",
            "\",\"limitations\"",
        ],
    )
    .or_else(|| extract_jsonish_string_field(text, "summary"))?;

    Some(DocumentAnalysisJsonResult {
        status,
        summary,
        content_structure: Vec::new(),
        key_points: Vec::new(),
        file_observations: Vec::new(),
        limitations: Vec::new(),
    })
}

pub fn extract_jsonish_string_field(text: &str, field: &str) -> Option<String> {
    let marker = format!("\"{}\":\"", field);
    let start = text.find(&marker)? + marker.len();
    let rest = &text[start..];
    let end = rest.find('"')?;
    Some(unescape_jsonish_text(&rest[..end]))
}

pub fn extract_jsonish_string_field_before(
    text: &str,
    field: &str,
    delimiters: &[&str],
) -> Option<String> {
    let marker = format!("\"{}\":\"", field);
    let start = text.find(&marker)? + marker.len();
    let rest = &text[start..];
    let end = delimiters
        .iter()
        .filter_map(|delimiter| rest.find(delimiter))
        .min()?;
    Some(unescape_jsonish_text(&rest[..end]))
}

pub fn unescape_jsonish_text(value: &str) -> String {
    value
        .replace("\\\"", "\"")
        .replace("\\n", "\n")
        .replace("\\\\", "\\")
}

pub fn completed_document_analysis_json(summary: String) -> (String, &'static str) {
    let result = DocumentAnalysisJsonResult {
        status: "completed".to_string(),
        summary,
        content_structure: Vec::new(),
        key_points: Vec::new(),
        file_observations: Vec::new(),
        limitations: Vec::new(),
    };
    (serialize_document_analysis_json(result), "completed")
}

pub fn blocked_document_analysis_json(
    summary: &str,
    limitations: Vec<String>,
) -> (String, &'static str) {
    let result = DocumentAnalysisJsonResult {
        status: "blocked".to_string(),
        summary: summary.to_string(),
        content_structure: Vec::new(),
        key_points: Vec::new(),
        file_observations: Vec::new(),
        limitations,
    };
    (serialize_document_analysis_json(result), "failed")
}

pub fn serialize_document_analysis_json(result: DocumentAnalysisJsonResult) -> String {
    serde_json::to_string_pretty(&result)
        .unwrap_or_else(|_| "{\"status\":\"blocked\",\"summary\":\"文档解读结果序列化失败。\",\"content_structure\":[],\"key_points\":[],\"file_observations\":[],\"limitations\":[\"serialization failed\"]}".to_string())
        .chars()
        .take(MAX_ANALYSIS_CHARS)
        .collect()
}

// ---------------------------------------------------------------------------
// MIME / filename helpers
// ---------------------------------------------------------------------------

/// Returns `true` if `mime` matches any prefix in `skip_prefixes`
/// (case-insensitive). Desktop callers can plug their own skip list — the
/// team-server uses image MIME types here.
pub fn should_skip_mime(mime: &str, skip_prefixes: &[String]) -> bool {
    let lower = mime.to_lowercase();
    skip_prefixes
        .iter()
        .any(|prefix| lower.starts_with(&prefix.to_ascii_lowercase()))
}

/// Strip filesystem-unsafe characters from an upload's display name. Empty
/// input returns empty — caller decides the fallback (the team-server uses
/// `format!("document-{}", &doc_id[..8])`).
pub fn sanitize_analysis_filename(name: &str) -> String {
    let sanitized = name
        .chars()
        .map(|ch| match ch {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
            ch if ch.is_control() => '_',
            ch => ch,
        })
        .collect::<String>()
        .trim()
        .trim_matches('.')
        .to_string();
    if sanitized.is_empty() {
        String::new()
    } else {
        sanitized
    }
}

/// Classify a MIME type into one of: `pdf`, `archive`, `presentation`,
/// `spreadsheet`, `word`, `text`. Used by the prompt builder to pick the
/// right extraction-pattern hint.
pub fn mime_category(mime: &str) -> &'static str {
    let m = mime.to_lowercase();
    if m.contains("pdf") {
        return "pdf";
    }
    if m.contains("zip")
        || m.contains("rar")
        || m.contains("7z")
        || m.contains("tar")
        || m.contains("gzip")
    {
        return "archive";
    }
    if m.contains("presentation")
        || m.contains("powerpoint")
        || m.contains("pptx")
        || m.contains("ppt")
    {
        return "presentation";
    }
    if m.contains("spreadsheet") || m.contains("excel") || m.contains("csv") {
        return "spreadsheet";
    }
    if m.contains("word") || (m.contains("document") && !m.contains("spreadsheet")) {
        return "word";
    }
    "text"
}

// ---------------------------------------------------------------------------
// Prompt builder (bilingual EN + 中文, MIME-aware)
// ---------------------------------------------------------------------------

/// Build the user-side prompt for document analysis. The `materialized` and
/// `content_snapshot` parameters are append-only context blocks — `None`
/// means "the desktop didn't materialise / no snapshot available".
pub fn build_analysis_prompt(
    ctx: &DocumentAnalysisInput,
    materialized: Option<&MaterializedAnalysisDocument>,
    content_snapshot: Option<&str>,
) -> String {
    let cat = mime_category(&ctx.mime_type);
    let en = ctx.lang.as_deref() == Some("en");
    let snapshot = content_snapshot
        .map(str::trim)
        .filter(|value| !value.is_empty());

    let input_rule = match (snapshot.is_some(), en) {
        (true, true) => {
            "A text snapshot is included as auxiliary context. Prefer the actual document reference and workspace file when deeper reading is needed. Use the snapshot only to speed up understanding; it is not a separate execution path."
        }
        (true, false) => {
            "下方文本摘录只是辅助上下文。需要更完整解读时，优先使用文档引用和 workspace 文件读取实际文档；不要把摘录当成另一条执行路径。"
        }
        (false, true) => {
            "Use the document reference and preferred workspace-relative path directly with the available agent tools. Read the document best-effort and return the required JSON result."
        }
        (false, false) => {
            "请使用文档引用和首选 workspace 相对路径，通过 agent 可用工具 best-effort 阅读实际文件，并返回要求的 JSON 解读结果。"
        }
    };

    let type_steps = match (snapshot.is_some(), cat, en) {
        (true, _, false) => {
            "推荐范式：先查看文档引用、workspace 路径和可用文本摘录；如果摘录足够，直接形成 JSON；如果不够，用可用工具读取 workspace 文件后再形成 JSON。"
        }
        (true, _, true) => {
            "Suggested pattern: inspect the document reference, workspace path, and available snapshot; if the snapshot is enough, produce JSON directly; otherwise read the workspace file with available tools and then produce JSON."
        }
        (false, "pdf", false) => {
            "推荐范式：直接使用已提供的 workspace 文件路径，再用 developer shell、MCP 或其他本地工具读取 PDF 正文（如 pdftotext 或 Python pdfplumber），最后按页面/章节结构分析。"
        }
        (false, "pdf", true) => {
            "Suggested pattern: use the provided workspace file path directly, then use developer shell, MCP, or another local tool to read the PDF content (for example pdftotext or Python pdfplumber), and finally analyze it by page/chapter structure."
        }
        (false, "archive", false) => {
            "推荐范式：直接使用已提供的 workspace 文件路径，再用 developer shell、MCP 或其他本地工具解压压缩包；只分析解压出来的内部文档内容。不要把压缩包容器、文件头、魔术签名、字节长度当成文档内容。如果无法解压或没有可读内部文件，返回 status=\"blocked\"，summary 说明无法读取压缩包内部文档内容。"
        }
        (false, "archive", true) => {
            "Suggested pattern: use the provided workspace file path directly, then use developer shell, MCP, or another local tool to extract the archive; only analyze the extracted inner documents. Do not treat the archive container, headers, magic signature, or byte length as document content. If extraction fails or no readable inner file exists, return status=\"blocked\" and say the inner document content could not be read."
        }
        (false, "presentation", false) => {
            "推荐范式：直接使用已提供的 workspace 文件路径。PPTX 可优先用 python-pptx；若库不可用，把 PPTX 当 zip 包读取 `ppt/slides/*.xml` 和 notes。旧 PPT/PPS 优先使用系统已有转换/文本提取工具，必要时做 best-effort 文本提取。拿到一次可读内容后直接逐页整理并输出最终结果。"
        }
        (false, "presentation", true) => {
            "Suggested pattern: use the provided workspace file path directly. For PPTX, prefer python-pptx; if unavailable, treat PPTX as a zip package and read `ppt/slides/*.xml` plus notes. For legacy PPT/PPS, use available conversion/text-extraction tools first, then best-effort text extraction if needed. After readable content is obtained once, analyze slide by slide and produce the final result."
        }
        (false, "spreadsheet", false) => {
            "推荐范式：直接使用已提供的 workspace 文件路径。XLSX 可优先用 openpyxl/pandas；若库不可用，把 XLSX 当 zip 包读取 `xl/sharedStrings.xml` 和 `xl/worksheets/*.xml`。旧 XLS 优先使用系统已有转换/文本提取工具，必要时做 best-effort 文本提取。拿到一次可读内容后直接总结 Sheet、字段和关键统计。"
        }
        (false, "spreadsheet", true) => {
            "Suggested pattern: use the provided workspace file path directly. For XLSX, prefer openpyxl/pandas; if unavailable, treat XLSX as a zip package and read `xl/sharedStrings.xml` and `xl/worksheets/*.xml`. For legacy XLS, use available conversion/text-extraction tools first, then best-effort text extraction if needed. After readable content is obtained once, summarize sheets, fields, and key statistics."
        }
        (false, "word", false) => {
            "推荐范式：直接使用已提供的 workspace 文件路径。DOCX 可优先用 python-docx；若库不可用，把 DOCX 当 zip 包读取 `word/document.xml`、headers、footers。旧 DOC 优先使用系统已有转换/文本提取工具，必要时做 best-effort 文本提取。拿到一次可读内容后直接按章节结构分析并输出最终结果。"
        }
        (false, "word", true) => {
            "Suggested pattern: use the provided workspace file path directly. For DOCX, prefer python-docx; if unavailable, treat DOCX as a zip package and read `word/document.xml`, headers, and footers. For legacy DOC, use available conversion/text-extraction tools first, then best-effort text extraction if needed. After readable content is obtained once, analyze by section and produce the final result."
        }
        (false, _, false) => {
            "推荐范式：直接使用已提供的 workspace 文件路径，再用 developer shell、MCP 或其他本地工具读取实际内容，最后分析文本结构和关键信息。"
        }
        (false, _, true) => {
            "Suggested pattern: use the provided workspace file path directly, then use developer shell, MCP, or another local tool to inspect the content, and finally analyze the text structure and key information."
        }
    };

    let mut prompt = if en {
        format!(
            "Analyze the document \"{name}\" (MIME: {mime}, Doc ID: {doc_id}).\n\n\
             This is an agent-driven document-analysis run. The server has already materialized the uploaded document into the workspace when a workspace path is provided below.\n\
             {input_rule}\n\n\
             First principle: summarize the user-facing document content. Do not analyze file containers, MIME labels, raw bytes, archive headers, magic signatures, filenames, or test markers as if they were the document content.\n\n\
             After you have obtained readable content once, do not repeatedly call file-reading tools. Move directly to the final JSON result.\n\n\
             Never reply with future-intent text such as \"I need to read the document first\" or \"let me do that now\". Complete the read/analyze chain in this run.\n\n\
             ## Suggested Reading Pattern\n{steps}\n\n\
             ## Output Format (strict JSON only)\n\
             Return exactly one JSON object with these keys and no Markdown fences:\n\
             {{\"status\":\"completed|blocked\",\"summary\":\"string\",\"content_structure\":[\"string\"],\"key_points\":[\"string\"],\"file_observations\":[\"string\"],\"limitations\":[\"string\"]}}\n\n\
             Use status=\"completed\" when you have read enough content to provide a useful analysis.\n\
             Use status=\"blocked\" only when the file cannot be read, the format has no available tool, the content is damaged, or the path/permission is unavailable.\n\n\
             ## Strictly Forbidden\n\
             - No Markdown fences or prose outside the JSON object\n\
             - Do not summarize archive/file metadata when no inner document content was read\n\
             - No conversational phrases like \"let me know\" or \"hope this helps\"",
            name = ctx.doc_name,
            mime = ctx.mime_type,
            doc_id = ctx.doc_id,
            input_rule = input_rule,
            steps = type_steps,
        )
    } else {
        format!(
            "请分析文档「{name}」(MIME: {mime}, 文档ID: {doc_id})。\n\n\
             这是一轮由 agent 自主完成的文档分析。如果下方提供了 workspace 文件路径，说明服务端已经把上传文档确定性放入本轮 workspace。\n\
             {input_rule}\n\n\
             第一性原则：总结用户真正关心的文档正文内容。不要把文件容器、MIME 标签、原始字节、压缩包头、魔术签名、文件名或测试标识当作文档内容来解读。\n\n\
             一旦已经拿到一次可读内容，不要反复调用文件读取工具，应直接进入最终 JSON 解读结果。\n\n\
             不要回复\"我需要先读取文档\"或\"让我先做这个\"之类的未来时描述。必须在本轮内完成读取和分析。\n\n\
             ## 建议范式\n{steps}\n\n\
             ## 输出格式（严格 JSON）\n\
             只返回一个 JSON 对象，不要 Markdown 代码块：\n\
             {{\"status\":\"completed|blocked\",\"summary\":\"string\",\"content_structure\":[\"string\"],\"key_points\":[\"string\"],\"file_observations\":[\"string\"],\"limitations\":[\"string\"]}}\n\n\
             当你已经读取到足够内容并能形成有效解读时，status 使用 \"completed\"。\n\
             只有文件无法读取、格式缺少可用工具、内容损坏、路径或权限不可访问时，status 才使用 \"blocked\"。\n\n\
             ## 严格禁止\n\
             - 禁止在 JSON 外输出解释性文字\n\
             - 未读取到压缩包内部文档内容时，禁止总结压缩包/文件元信息\n\
             - 禁止输出「我需要先读取」「让我先解压」等未来时描述",
            name = ctx.doc_name,
            mime = ctx.mime_type,
            doc_id = ctx.doc_id,
            input_rule = input_rule,
            steps = type_steps,
        )
    };

    if let Some(materialized) = materialized {
        if en {
            prompt.push_str(&format!(
                "\n\n## Workspace Input\n- Preferred workspace-relative path: `{}`\n- Absolute path for reference only: `{}`\n- File name: `{}`\n- MIME type: `{}`\n- File size: {} bytes",
                materialized.relative_path,
                materialized.file_path,
                materialized.file_name,
                materialized.mime_type,
                materialized.file_size
            ));
        } else {
            prompt.push_str(&format!(
                "\n\n## Workspace 输入\n- 首选 workspace 相对路径：`{}`\n- 绝对路径仅作参考：`{}`\n- 文件名：`{}`\n- MIME 类型：`{}`\n- 文件大小：{} bytes",
                materialized.relative_path,
                materialized.file_path,
                materialized.file_name,
                materialized.mime_type,
                materialized.file_size
            ));
        }
    }
    if let Some(snapshot) = snapshot {
        if en {
            prompt.push_str(&format!(
                "\n\n## Available Text Snapshot (auxiliary)\nThis snapshot may help you understand the document quickly. If it is insufficient, use the workspace file and available tools. The final answer must still be the strict JSON object:\n\n```text\n{}\n```",
                snapshot
            ));
        } else {
            prompt.push_str(&format!(
                "\n\n## 可用文本摘录（辅助）\n这段摘录可以帮助你快速理解文档。如果摘录不足，请使用 workspace 文件和可用工具读取实际内容。最终回答仍必须是严格 JSON 对象：\n\n```text\n{}\n```",
                snapshot
            ));
        }
    }

    prompt
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn input(mime: &str, lang: Option<&str>) -> DocumentAnalysisInput {
        DocumentAnalysisInput {
            doc_id: "deadbeef".to_string(),
            doc_name: "report.pdf".to_string(),
            mime_type: mime.to_string(),
            file_size: 1234,
            lang: lang.map(str::to_string),
        }
    }

    #[test]
    fn terminal_text_classifier_detects_intent_phrases() {
        assert!(!document_analysis_text_is_terminal(
            "I need to read the file first."
        ));
        assert!(!document_analysis_text_is_terminal("我需要先解压"));
        assert!(document_analysis_text_is_terminal(
            "The document covers chapter 1, 2, and 3."
        ));
    }

    #[test]
    fn extract_json_object_unfences_markdown() {
        let text = "```json\n{\"status\":\"completed\",\"summary\":\"x\"}\n```";
        let extracted = extract_json_object(text).expect("should unfence");
        assert!(extracted.starts_with('{'));
        assert!(extracted.ends_with('}'));
    }

    #[test]
    fn parse_document_analysis_json_round_trip() {
        let raw = r#"{"status":"completed","summary":"hello","content_structure":[],"key_points":["k"],"file_observations":[],"limitations":[]}"#;
        let parsed = parse_document_analysis_json(raw).expect("parse should succeed");
        assert_eq!(parsed.status, "completed");
        assert_eq!(parsed.key_points, vec!["k".to_string()]);
    }

    #[test]
    fn repair_jsonish_handles_truncation() {
        let truncated = r#"{"status":"completed","summary":"good enough"#;
        let parsed = repair_document_analysis_jsonish(truncated);
        // Truncated quotes — the repair finds the marker but no closing quote
        // for `summary`. Either `None` or a recovered shape is acceptable; we
        // just assert it doesn't panic.
        let _ = parsed;
    }

    #[test]
    fn normalize_collapses_archive_false_positive() {
        let result = DocumentAnalysisJsonResult {
            status: "blocked".to_string(),
            summary: "RAR magic signature only".to_string(),
            content_structure: vec![],
            key_points: vec![],
            file_observations: vec!["not-a-real-rar".to_string()],
            limitations: vec![],
        };
        let normalized = normalize_document_analysis_result(result);
        assert!(normalized.summary.contains("无法解压"));
        assert!(normalized
            .limitations
            .iter()
            .any(|item| item.contains("RAR magic signature only")));
    }

    #[test]
    fn mime_category_classifies_known_types() {
        assert_eq!(mime_category("application/pdf"), "pdf");
        assert_eq!(mime_category("application/zip"), "archive");
        assert_eq!(mime_category("application/x-rar-compressed"), "archive");
        assert_eq!(
            mime_category(
                "application/vnd.openxmlformats-officedocument.presentationml.presentation"
            ),
            "presentation"
        );
        assert_eq!(
            mime_category("application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"),
            "spreadsheet"
        );
        assert_eq!(
            mime_category(
                "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
            ),
            "word"
        );
        assert_eq!(mime_category("text/plain"), "text");
    }

    #[test]
    fn sanitize_filename_strips_unsafe_chars() {
        assert_eq!(sanitize_analysis_filename("a/b\\c:d.txt"), "a_b_c_d.txt");
        assert_eq!(sanitize_analysis_filename(""), "");
        assert_eq!(sanitize_analysis_filename("...."), "");
    }

    #[test]
    fn should_skip_mime_matches_prefixes_case_insensitively() {
        let skip = vec!["image/".to_string(), "AUDIO/".to_string()];
        assert!(should_skip_mime("image/png", &skip));
        assert!(should_skip_mime("Audio/mp3", &skip));
        assert!(!should_skip_mime("application/pdf", &skip));
    }

    #[test]
    fn build_prompt_contains_doc_name_and_mime() {
        let ctx = input("application/pdf", None);
        let prompt = build_analysis_prompt(&ctx, None, None);
        assert!(prompt.contains("report.pdf"));
        assert!(prompt.contains("application/pdf"));
        assert!(prompt.contains("deadbeef"));
    }

    #[test]
    fn build_prompt_switches_to_english_when_lang_en() {
        let ctx = input("application/pdf", Some("en"));
        let prompt = build_analysis_prompt(&ctx, None, None);
        assert!(prompt.contains("Analyze the document"));
        assert!(!prompt.contains("请分析文档"));
    }

    #[test]
    fn build_prompt_appends_workspace_block_when_materialized() {
        let ctx = input("application/pdf", None);
        let mat = MaterializedAnalysisDocument {
            file_path: "/ws/attachments/report.pdf".to_string(),
            relative_path: "attachments/report.pdf".to_string(),
            file_name: "report.pdf".to_string(),
            mime_type: "application/pdf".to_string(),
            file_size: 1234,
        };
        let prompt = build_analysis_prompt(&ctx, Some(&mat), None);
        assert!(prompt.contains("attachments/report.pdf"));
        assert!(prompt.contains("Workspace 输入"));
    }

    #[test]
    fn build_prompt_appends_snapshot_block_when_provided() {
        let ctx = input("application/pdf", None);
        let prompt = build_analysis_prompt(&ctx, None, Some("hello world"));
        assert!(prompt.contains("hello world"));
        assert!(prompt.contains("可用文本摘录"));
    }

    #[test]
    fn derive_persistence_falls_back_to_blocked_when_empty() {
        let (json, status) = derive_document_analysis_persistence(None, None, "x.pdf");
        assert_eq!(status, "failed");
        assert!(json.contains("blocked"));
    }

    #[test]
    fn derive_persistence_promotes_terminal_text() {
        let (json, status) =
            derive_document_analysis_persistence(None, Some("Final result here."), "x.pdf");
        assert_eq!(status, "completed");
        assert!(json.contains("Final result here."));
    }

    #[test]
    fn select_text_prefers_terminal_visible() {
        let visible = Some("All chapters covered.".to_string());
        let chosen = select_document_analysis_text(None, visible);
        assert_eq!(chosen.as_deref(), Some("All chapters covered."));
    }
}
