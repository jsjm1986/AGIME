//! Chat API routes (Phase 1 - Chat Track)
//!
//! These routes handle direct chat sessions that bypass the Task system.
//! Mounted at `/api/team/agent/chat`.

use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{
        sse::{Event, Sse},
        Json,
    },
    routing::{delete, get, post, put},
    Extension, Router,
};
use futures::{stream::Stream, StreamExt};
use mongodb::bson::{doc, oid::ObjectId, Bson, Document};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;

use crate::auth::middleware::UserContext;
use agime::agents::types::{RetryConfig, SuccessCheck};
use agime_team::models::mongo::PortalDetail;
use agime_team::models::{
    AgentSkillConfig, BuiltinExtension, CustomExtensionConfig, ListAgentsQuery, TeamAgent,
};
use agime_team::MongoDb;

use super::chat_executor::ChatExecutor;
use super::chat_manager::ChatManager;
use super::normalize_workspace_path;
use super::prompt_profiles::{
    build_portal_coding_overlay, build_portal_manager_overlay, PortalCodingProfileInput,
};
use super::service_mongo::AgentService;
use super::session_mongo::{
    CreateChatSessionRequest, SendChatMessageRequest, SendMessageResponse, SessionListItem,
    UserSessionListQuery,
};
use agime_team::services::mongo::PortalService;

type ChatState = (Arc<AgentService>, Arc<MongoDb>, Arc<ChatManager>, String);

#[derive(serde::Deserialize)]
struct StreamQuery {
    last_event_id: Option<u64>,
}

#[derive(serde::Deserialize)]
struct EventListQuery {
    #[serde(default)]
    run_id: Option<String>,
    #[serde(default)]
    after_event_id: Option<u64>,
    #[serde(default)]
    before_event_id: Option<u64>,
    #[serde(default)]
    order: Option<String>,
    #[serde(default)]
    limit: Option<u32>,
}

#[derive(Debug, Clone, serde::Serialize)]
struct ComposerCapabilitySkill {
    id: String,
    name: String,
    version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    summary_text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    detail_text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    detail_lang: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    detail_source: Option<String>,
    skill_ref: String,
    display_line_zh: String,
    plain_line_zh: String,
}

#[derive(Debug, Clone, serde::Serialize)]
struct ComposerCapabilityExtension {
    runtime_name: String,
    display_name: String,
    class: String,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    ext_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    summary_text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    detail_text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    detail_lang: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    detail_source: Option<String>,
    ext_ref: String,
    display_line_zh: String,
    plain_line_zh: String,
}

#[derive(Debug, Clone, serde::Serialize)]
struct ComposerHiddenCapabilityExtension {
    runtime_name: String,
    display_name: String,
    reason: String,
}

#[derive(Debug, Clone, serde::Serialize)]
struct ComposerCapabilitiesCatalog {
    skills: Vec<ComposerCapabilitySkill>,
    extensions: Vec<ComposerCapabilityExtension>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    hidden_extensions: Vec<ComposerHiddenCapabilityExtension>,
}

#[derive(Debug, Clone)]
struct ComposerExtensionEntry {
    runtime_name: String,
    display_name: String,
    class: String,
    ext_type: Option<String>,
    description: Option<String>,
    ext_ref: String,
    display_line_zh: String,
    plain_line_zh: String,
    source_extension_id: Option<String>,
    builtin_lookup_id: Option<String>,
}

#[derive(Debug, Clone, Default)]
struct ComposerDescriptionText {
    summary_text: Option<String>,
    detail_text: Option<String>,
    detail_lang: Option<String>,
    detail_source: Option<String>,
}

fn normalize_name(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

fn normalize_skill_id(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

fn build_skill_ref(skill_id: &str, name: &str, skill_class: &str, meta: &str) -> String {
    format!("[[skill:{}|{}|{}|{}]]", skill_id, name, skill_class, meta)
}

fn build_extension_ref(
    extension_id: &str,
    name: &str,
    extension_class: &str,
    meta: &str,
) -> String {
    format!(
        "[[ext:{}|{}|{}|{}]]",
        extension_id, name, extension_class, meta
    )
}

fn skill_display_line_zh(skill_ref: &str, version: &str) -> String {
    format!("{}（团队技能，v{}）", skill_ref, version)
}

fn skill_plain_line_zh(name: &str, version: &str) -> String {
    format!("{}（团队技能，v{}）", name, version)
}

fn builtin_extension_display_name(extension: BuiltinExtension) -> &'static str {
    match extension {
        BuiltinExtension::Skills => "Skills",
        BuiltinExtension::SkillRegistry => "Skill Registry",
        BuiltinExtension::Todo => "Todo",
        BuiltinExtension::ExtensionManager => "Extension Manager",
        BuiltinExtension::Team => "Team",
        BuiltinExtension::ChatRecall => "Chat Recall",
        BuiltinExtension::DocumentTools => "Document Tools",
        BuiltinExtension::Developer => "Developer",
        BuiltinExtension::Memory => "Memory",
        BuiltinExtension::ComputerController => "Computer Controller",
        BuiltinExtension::AutoVisualiser => "Auto Visualiser",
        BuiltinExtension::Tutorial => "Tutorial",
    }
}

fn extension_display_line_zh(extension_ref: &str, extension_class: &str) -> String {
    let label = match extension_class {
        "builtin" => "内置扩展",
        "team" => "团队扩展",
        _ => "自定义扩展",
    };
    format!("{}（{}）", extension_ref, label)
}

fn extension_plain_line_zh(display_name: &str, extension_class: &str) -> String {
    let label = match extension_class {
        "builtin" => "内置扩展",
        "team" => "团队扩展",
        _ => "自定义扩展",
    };
    format!("{}（{}）", display_name, label)
}

fn is_runtime_visible_builtin(extension: BuiltinExtension) -> bool {
    matches!(
        extension,
        BuiltinExtension::SkillRegistry
            | BuiltinExtension::Todo
            | BuiltinExtension::DocumentTools
            | BuiltinExtension::Developer
            | BuiltinExtension::Memory
            | BuiltinExtension::ComputerController
            | BuiltinExtension::AutoVisualiser
            | BuiltinExtension::Tutorial
    )
}

fn builtin_runtime_names(extension: BuiltinExtension) -> Vec<String> {
    let mut names = vec![normalize_name(extension.name())];
    if let Some(mcp_name) = extension.mcp_name() {
        let normalized = normalize_name(mcp_name);
        if !names.contains(&normalized) {
            names.push(normalized);
        }
    }
    names
}

fn extension_class_from_config(extension: &CustomExtensionConfig) -> &'static str {
    if extension.source.as_deref() == Some("team") || extension.source_extension_id.is_some() {
        "team"
    } else {
        "custom"
    }
}

fn build_skill_catalog_item(skill: &AgentSkillConfig) -> ComposerCapabilitySkill {
    let version = if skill.version.trim().is_empty() {
        "1.0.0".to_string()
    } else {
        skill.version.trim().to_string()
    };
    let skill_ref = build_skill_ref(&format!("team:{}", skill.skill_id), &skill.name, "team", &version);
    ComposerCapabilitySkill {
        id: skill.skill_id.clone(),
        name: skill.name.clone(),
        version: version.clone(),
        description: skill.description.clone(),
        summary_text: None,
        detail_text: None,
        detail_lang: None,
        detail_source: None,
        skill_ref: skill_ref.clone(),
        display_line_zh: skill_display_line_zh(&skill_ref, &version),
        plain_line_zh: skill_plain_line_zh(&skill.name, &version),
    }
}

fn build_builtin_extension_entry(extension: BuiltinExtension) -> ComposerExtensionEntry {
    let runtime_name = extension
        .mcp_name()
        .map(normalize_name)
        .unwrap_or_else(|| normalize_name(extension.name()));
    let display_name = builtin_extension_display_name(extension).to_string();
    let ext_ref = build_extension_ref(
        &format!("builtin:{}", runtime_name),
        &display_name,
        "builtin",
        &runtime_name,
    );
    ComposerExtensionEntry {
        runtime_name,
        display_name: display_name.clone(),
        class: "builtin".to_string(),
        ext_type: if extension.mcp_name().is_some() {
            Some("stdio".to_string())
        } else {
            None
        },
        description: Some(extension.description().to_string()),
        ext_ref: ext_ref.clone(),
        display_line_zh: extension_display_line_zh(&ext_ref, "builtin"),
        plain_line_zh: extension_plain_line_zh(&display_name, "builtin"),
        source_extension_id: None,
        builtin_lookup_id: Some(extension.name().to_string()),
    }
}

fn build_custom_extension_entry(extension: &CustomExtensionConfig) -> ComposerExtensionEntry {
    let runtime_name = normalize_name(&extension.name);
    let class = extension_class_from_config(extension).to_string();
    let ext_ref = build_extension_ref(
        &match class.as_str() {
            "team" => format!(
                "team:{}",
                extension
                    .source_extension_id
                    .clone()
                    .unwrap_or_else(|| runtime_name.clone())
            ),
            _ => format!("custom:{}", runtime_name),
        },
        &extension.name,
        &class,
        extension.ext_type.as_str(),
    );
    ComposerExtensionEntry {
        runtime_name,
        display_name: extension.name.clone(),
        class: class.clone(),
        ext_type: Some(extension.ext_type.clone()),
        description: None,
        ext_ref: ext_ref.clone(),
        display_line_zh: extension_display_line_zh(&ext_ref, &class),
        plain_line_zh: extension_plain_line_zh(&extension.name, &class),
        source_extension_id: extension.source_extension_id.clone(),
        builtin_lookup_id: None,
    }
}

fn build_hidden_builtin_extension(
    extension: BuiltinExtension,
) -> Option<ComposerHiddenCapabilityExtension> {
    let runtime_name = extension
        .mcp_name()
        .map(normalize_name)
        .unwrap_or_else(|| normalize_name(extension.name()));
    let display_name = builtin_extension_display_name(extension).to_string();
    let reason = match extension {
        BuiltinExtension::Skills => "skill_runtime",
        BuiltinExtension::ExtensionManager | BuiltinExtension::ChatRecall => "system_assist",
        BuiltinExtension::Team => "legacy_hidden",
        _ => return None,
    };
    Some(ComposerHiddenCapabilityExtension {
        runtime_name,
        display_name,
        reason: reason.to_string(),
    })
}

fn find_extension_entry_from_agent(agent: &TeamAgent, runtime_name: &str) -> Option<ComposerExtensionEntry> {
    let normalized = normalize_name(runtime_name);
    if let Some(custom) = agent
        .custom_extensions
        .iter()
        .find(|ext| normalize_name(&ext.name) == normalized)
    {
        let mut cfg = custom.clone();
        cfg.enabled = true;
        return Some(build_custom_extension_entry(&cfg));
    }

    for ext in &agent.enabled_extensions {
        let runtime_names = builtin_runtime_names(ext.extension);
        if runtime_names.iter().any(|name| name == &normalized)
            && is_runtime_visible_builtin(ext.extension)
        {
            return Some(build_builtin_extension_entry(ext.extension));
        }
    }

    None
}

fn resolve_composer_extensions(
    agent: &TeamAgent,
    session: Option<&super::session_mongo::AgentSessionDoc>,
) -> Vec<ComposerExtensionEntry> {
    let mut entries = BTreeMap::<String, ComposerExtensionEntry>::new();

    for ext in agent.enabled_extensions.iter().filter(|ext| ext.enabled) {
        if is_runtime_visible_builtin(ext.extension) {
            let entry = build_builtin_extension_entry(ext.extension);
            entries.insert(entry.runtime_name.clone(), entry);
        }
    }

    for ext in agent.custom_extensions.iter().filter(|ext| ext.enabled) {
        let entry = build_custom_extension_entry(ext);
        entries.insert(entry.runtime_name.clone(), entry);
    }

    if let Some(session) = session {
        let disabled: HashSet<String> = session
            .disabled_extensions
            .iter()
            .map(|value| normalize_name(value))
            .collect();
        for runtime_name in disabled {
            entries.remove(&runtime_name);
        }

        for enabled_name in &session.enabled_extensions {
            let normalized = normalize_name(enabled_name);
            if entries.contains_key(&normalized) {
                continue;
            }
            if let Some(entry) = find_extension_entry_from_agent(agent, &normalized) {
                entries.insert(normalized, entry);
            }
        }

        if let Some(allowed_extensions) = &session.allowed_extensions {
            let allowed: HashSet<String> = allowed_extensions
                .iter()
                .map(|value| normalize_name(value))
                .collect();
            entries.retain(|runtime_name, _| allowed.contains(runtime_name));
        }
    }

    entries.into_values().collect()
}

fn resolve_hidden_composer_extensions(agent: &TeamAgent) -> Vec<ComposerHiddenCapabilityExtension> {
    let mut hidden = BTreeMap::<String, ComposerHiddenCapabilityExtension>::new();
    for ext in agent.enabled_extensions.iter().filter(|ext| ext.enabled) {
        if is_runtime_visible_builtin(ext.extension) {
            continue;
        }
        if let Some(entry) = build_hidden_builtin_extension(ext.extension) {
            hidden.insert(entry.runtime_name.clone(), entry);
        }
    }
    hidden.into_values().collect()
}

fn resolve_composer_skills(
    agent: &TeamAgent,
    session: Option<&super::session_mongo::AgentSessionDoc>,
) -> Vec<ComposerCapabilitySkill> {
    let allowed_skill_ids: Option<HashSet<String>> = session.and_then(|session| {
        session.allowed_skill_ids.as_ref().map(|ids| {
            ids.iter()
                .map(|value| normalize_skill_id(value))
                .collect::<HashSet<_>>()
        })
    });

    agent.assigned_skills
        .iter()
        .filter(|skill| skill.enabled)
        .filter(|skill| {
            allowed_skill_ids
                .as_ref()
                .map(|ids| ids.contains(&normalize_skill_id(&skill.skill_id)))
                .unwrap_or(true)
        })
        .map(build_skill_catalog_item)
        .collect()
}

fn collapse_whitespace(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn trim_optional_text(value: Option<String>) -> Option<String> {
    value.and_then(|text| {
        let normalized = collapse_whitespace(&text);
        if normalized.is_empty() {
            None
        } else {
            Some(normalized)
        }
    })
}

fn first_summary_chunk(value: &str) -> String {
    let normalized = collapse_whitespace(value);
    if normalized.is_empty() {
        return String::new();
    }

    for separator in ["\n\n", "。", ".", "；", ";", "！", "!", "？", "?"] {
        if let Some((head, _)) = normalized.split_once(separator) {
            let trimmed = head.trim();
            if trimmed.len() >= 18 {
                return trimmed.to_string();
            }
        }
    }

    normalized
}

fn clamp_summary(value: &str, max_chars: usize) -> String {
    let chunk = first_summary_chunk(value);
    if chunk.chars().count() <= max_chars {
        return chunk;
    }

    let mut end = 0usize;
    for (idx, _) in chunk.char_indices() {
        if chunk[..idx].chars().count() >= max_chars {
            break;
        }
        end = idx;
    }

    let candidate = if end == 0 {
        chunk.chars().take(max_chars).collect::<String>()
    } else {
        chunk[..end].trim_end().to_string()
    };
    format!("{}…", candidate.trim_end_matches(['。', '.', ';', '；', '!', '！', '?', '？']))
}

fn build_description_text(
    ai_description: Option<String>,
    ai_lang: Option<String>,
    raw_description: Option<String>,
    ai_source: &str,
) -> ComposerDescriptionText {
    let ai_description = trim_optional_text(ai_description);
    let raw_description = trim_optional_text(raw_description);

    if let Some(detail_text) = ai_description {
        return ComposerDescriptionText {
            summary_text: Some(clamp_summary(&detail_text, 96)),
            detail_text: Some(detail_text),
            detail_lang: trim_optional_text(ai_lang),
            detail_source: Some(ai_source.to_string()),
        };
    }

    if let Some(detail_text) = raw_description {
        return ComposerDescriptionText {
            summary_text: Some(clamp_summary(&detail_text, 96)),
            detail_text: Some(detail_text),
            detail_lang: None,
            detail_source: Some("raw_description".to_string()),
        };
    }

    ComposerDescriptionText::default()
}

async fn enrich_composer_skills(
    db: &MongoDb,
    team_id: &str,
    skills: &mut [ComposerCapabilitySkill],
) {
    let Ok(team_oid) = ObjectId::parse_str(team_id) else {
        return;
    };

    let skill_ids: Vec<ObjectId> = skills
        .iter()
        .filter_map(|skill| ObjectId::parse_str(&skill.id).ok())
        .collect();
    if skill_ids.is_empty() {
        return;
    }

    let coll = db.collection::<Document>("skills");
    let filter = doc! {
        "team_id": team_oid,
        "_id": { "$in": Bson::Array(skill_ids.into_iter().map(Bson::ObjectId).collect()) },
        "is_deleted": false,
    };
    let Ok(mut cursor) = coll.find(filter, None).await else {
        return;
    };

    let mut docs_by_id = BTreeMap::<String, Document>::new();
    while let Some(Ok(doc)) = cursor.next().await {
        if let Ok(id) = doc.get_object_id("_id") {
            docs_by_id.insert(id.to_hex(), doc);
        }
    }

    for skill in skills.iter_mut() {
        let source_doc = docs_by_id.get(&skill.id);
        let raw_description = source_doc
            .and_then(|doc| doc.get_str("description").ok().map(str::to_string))
            .or_else(|| skill.description.clone());
        let ai_description = source_doc
            .and_then(|doc| doc.get_str("ai_description").ok().map(str::to_string));
        let ai_lang = source_doc
            .and_then(|doc| doc.get_str("ai_description_lang").ok().map(str::to_string));
        let detail = build_description_text(ai_description, ai_lang, raw_description.clone(), "ai_description");
        skill.description = trim_optional_text(raw_description);
        skill.summary_text = detail.summary_text;
        skill.detail_text = detail.detail_text;
        skill.detail_lang = detail.detail_lang;
        skill.detail_source = detail.detail_source;
    }
}

fn composer_skill_dedupe_key(skill: &ComposerCapabilitySkill) -> String {
    format!(
        "{}::{}",
        normalize_name(&skill.name),
        skill.version.trim().to_ascii_lowercase()
    )
}

fn composer_detail_source_rank(source: Option<&str>) -> u8 {
    match source.unwrap_or_default() {
        "ai_description" => 3,
        "builtin_cache" => 2,
        "raw_description" => 1,
        _ => 0,
    }
}

fn composer_detail_lang_rank(lang: Option<&str>) -> u8 {
    let normalized = lang.unwrap_or_default().trim().to_ascii_lowercase();
    if normalized.starts_with("zh") {
        2
    } else if normalized.is_empty() {
        0
    } else {
        1
    }
}

fn normalize_lang_tag(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

fn preferred_lang_from_headers(headers: &HeaderMap) -> Option<String> {
    let raw = headers.get("accept-language")?.to_str().ok()?.trim();
    if raw.is_empty() {
        return None;
    }

    raw.split(',')
        .map(|part| part.split(';').next().unwrap_or_default().trim())
        .find(|part| !part.is_empty())
        .map(normalize_lang_tag)
}

fn composer_doc_lang_preference_rank(
    doc_lang: Option<&str>,
    preferred_lang: Option<&str>,
) -> u8 {
    let normalized = normalize_lang_tag(doc_lang.unwrap_or_default());
    let preferred = preferred_lang
        .map(normalize_lang_tag)
        .unwrap_or_else(|| "zh".to_string());

    if normalized.is_empty() {
        return 0;
    }
    if normalized == preferred || normalized.starts_with(&format!("{}-", preferred)) {
        return 4;
    }
    if preferred.starts_with("zh") && normalized.starts_with("zh") {
        return 3;
    }
    if normalized.starts_with("zh") {
        return 2;
    }
    if normalized.starts_with("en") {
        return 1;
    }
    1
}

fn composer_skill_rank(skill: &ComposerCapabilitySkill) -> (u8, u8, usize, usize) {
    (
        composer_detail_source_rank(skill.detail_source.as_deref()),
        composer_detail_lang_rank(skill.detail_lang.as_deref()),
        skill.detail_text.as_deref().map(str::len).unwrap_or(0),
        skill.description.as_deref().map(str::len).unwrap_or(0),
    )
}

fn dedupe_composer_skills(skills: Vec<ComposerCapabilitySkill>) -> Vec<ComposerCapabilitySkill> {
    let mut order = Vec::<String>::new();
    let mut deduped = HashMap::<String, ComposerCapabilitySkill>::new();

    for skill in skills {
        let key = composer_skill_dedupe_key(&skill);
        match deduped.get(&key) {
            Some(existing) if composer_skill_rank(&skill) <= composer_skill_rank(existing) => {}
            Some(_) => {
                deduped.insert(key, skill);
            }
            None => {
                order.push(key.clone());
                deduped.insert(key, skill);
            }
        }
    }

    order
        .into_iter()
        .filter_map(|key| deduped.remove(&key))
        .collect()
}

async fn enrich_composer_extensions(
    db: &MongoDb,
    team_id: &str,
    entries: Vec<ComposerExtensionEntry>,
    preferred_lang: Option<&str>,
) -> Vec<ComposerCapabilityExtension> {
    let Ok(team_oid) = ObjectId::parse_str(team_id) else {
        return entries
            .into_iter()
            .map(|entry| ComposerCapabilityExtension {
                runtime_name: entry.runtime_name,
                display_name: entry.display_name,
                class: entry.class,
                ext_type: entry.ext_type,
                description: entry.description,
                summary_text: None,
                detail_text: None,
                detail_lang: None,
                detail_source: None,
                ext_ref: entry.ext_ref,
                display_line_zh: entry.display_line_zh,
                plain_line_zh: entry.plain_line_zh,
            })
            .collect();
    };

    let source_extension_ids: Vec<ObjectId> = entries
        .iter()
        .filter_map(|entry| entry.source_extension_id.as_deref())
        .filter_map(|value| ObjectId::parse_str(value).ok())
        .collect();
    let builtin_ids: Vec<String> = entries
        .iter()
        .filter_map(|entry| entry.builtin_lookup_id.clone())
        .collect();

    let mut extension_docs = BTreeMap::<String, Document>::new();
    if !source_extension_ids.is_empty() {
        let coll = db.collection::<Document>("extensions");
        let filter = doc! {
            "team_id": team_oid,
            "_id": { "$in": Bson::Array(source_extension_ids.into_iter().map(Bson::ObjectId).collect()) },
            "is_deleted": false,
        };
        if let Ok(mut cursor) = coll.find(filter, None).await {
            while let Some(Ok(doc)) = cursor.next().await {
                if let Ok(id) = doc.get_object_id("_id") {
                    extension_docs.insert(id.to_hex(), doc);
                }
            }
        }
    }

    let mut builtin_docs = HashMap::<String, Document>::new();
    if !builtin_ids.is_empty() {
        let coll = db.collection::<Document>("builtin_extension_descriptions");
        let filter = doc! {
            "team_id": team_oid,
            "extension_id": { "$in": Bson::Array(builtin_ids.into_iter().map(Bson::String).collect()) },
        };
        if let Ok(mut cursor) = coll.find(filter, None).await {
            while let Some(Ok(doc)) = cursor.next().await {
                if let Ok(id) = doc.get_str("extension_id") {
                    let doc_lang = doc.get_str("ai_description_lang").ok();
                    let should_replace = match builtin_docs.get(id) {
                        Some(existing) => {
                            let existing_lang = existing.get_str("ai_description_lang").ok();
                            composer_doc_lang_preference_rank(doc_lang, preferred_lang)
                                > composer_doc_lang_preference_rank(existing_lang, preferred_lang)
                        }
                        None => true,
                    };
                    if should_replace {
                        builtin_docs.insert(id.to_string(), doc);
                    }
                }
            }
        }
    }

    entries
        .into_iter()
        .map(|entry| {
            let source_doc = entry
                .source_extension_id
                .as_ref()
                .and_then(|id| extension_docs.get(id));
            let builtin_doc = entry
                .builtin_lookup_id
                .as_ref()
                .and_then(|id| builtin_docs.get(id));

            let raw_description = source_doc
                .and_then(|doc| doc.get_str("description").ok().map(str::to_string))
                .or_else(|| entry.description.clone());

            let (ai_description, ai_lang, ai_source) = if let Some(doc) = builtin_doc {
                (
                    doc.get_str("ai_description").ok().map(str::to_string),
                    doc.get_str("ai_description_lang").ok().map(str::to_string),
                    "builtin_cache",
                )
            } else if let Some(doc) = source_doc {
                (
                    doc.get_str("ai_description").ok().map(str::to_string),
                    doc.get_str("ai_description_lang").ok().map(str::to_string),
                    "ai_description",
                )
            } else {
                (None, None, "raw_description")
            };

            let detail = build_description_text(ai_description, ai_lang, raw_description.clone(), ai_source);
            ComposerCapabilityExtension {
                runtime_name: entry.runtime_name,
                display_name: entry.display_name,
                class: entry.class,
                ext_type: entry.ext_type,
                description: trim_optional_text(raw_description),
                summary_text: detail.summary_text,
                detail_text: detail.detail_text,
                detail_lang: detail.detail_lang,
                detail_source: detail.detail_source,
                ext_ref: entry.ext_ref,
                display_line_zh: entry.display_line_zh,
                plain_line_zh: entry.plain_line_zh,
            }
        })
        .collect()
}

async fn build_composer_capability_catalog(
    db: &MongoDb,
    team_id: &str,
    agent: &TeamAgent,
    session: Option<&super::session_mongo::AgentSessionDoc>,
    preferred_lang: Option<&str>,
) -> ComposerCapabilitiesCatalog {
    let mut skills = resolve_composer_skills(agent, session);
    enrich_composer_skills(db, team_id, &mut skills).await;
    let skills = dedupe_composer_skills(skills);

    ComposerCapabilitiesCatalog {
        skills,
        extensions: enrich_composer_extensions(
            db,
            team_id,
            resolve_composer_extensions(agent, session),
            preferred_lang,
        )
        .await,
        hidden_extensions: resolve_hidden_composer_extensions(agent),
    }
}

fn default_portal_retry_config() -> RetryConfig {
    let check_command = if cfg!(windows) {
        "if exist index.html (exit /b 0) else (echo index.html not found & exit /b 1)".to_string()
    } else {
        "[ -f index.html ]".to_string()
    };
    RetryConfig {
        max_retries: 6,
        checks: vec![SuccessCheck::Shell {
            command: check_command,
        }],
        on_failure: None,
        timeout_seconds: Some(180),
        on_failure_timeout_seconds: Some(300),
    }
}

fn manager_message_mentions_skill_inventory(user_content: &str) -> bool {
    let lowered = user_content.to_ascii_lowercase();
    lowered.contains("available skill")
        || lowered.contains("available skills")
        || lowered.contains("installed skill")
        || lowered.contains("installed skills")
        || lowered.contains("current skill")
        || lowered.contains("current skills")
        || lowered.contains("my skills")
        || lowered.contains("usable skill")
        || lowered.contains("usable skills")
        || lowered.contains("what skills")
        || user_content.contains("有哪些技能")
        || user_content.contains("什么技能")
        || user_content.contains("可用技能")
        || user_content.contains("当前技能")
        || user_content.contains("已安装技能")
        || user_content.contains("已安装并启用")
        || user_content.contains("安装并启用")
        || user_content.contains("启用的skills")
        || user_content.contains("启用的 skills")
        || user_content.contains("我已安装并启用的 skills")
        || user_content.contains("我已安装并启用的skills")
        || user_content.contains("目前能用")
        || user_content.contains("找一下你目前能用的skills")
        || user_content.contains("找一下你目前能用的 skills")
}

fn manager_message_mentions_registry(user_content: &str) -> bool {
    let lowered = user_content.to_ascii_lowercase();
    lowered.contains("skills.sh")
        || lowered.contains("skill_registry")
        || lowered.contains("registry")
        || lowered.contains("trending")
        || lowered.contains("all_time")
        || lowered.contains("all time")
        || lowered.contains("hot")
        || user_content.contains("热门")
        || user_content.contains("技能")
}

fn manager_message_mentions_extension_inventory(user_content: &str) -> bool {
    let lowered = user_content.to_ascii_lowercase();
    lowered.contains("available extension")
        || lowered.contains("available extensions")
        || lowered.contains("current extension")
        || lowered.contains("current extensions")
        || lowered.contains("enabled extension")
        || lowered.contains("enabled extensions")
        || lowered.contains("builtin extension")
        || lowered.contains("builtin extensions")
        || lowered.contains("mcp")
        || user_content.contains("可用扩展")
        || user_content.contains("当前扩展")
        || user_content.contains("已启用扩展")
        || user_content.contains("启用的扩展")
        || user_content.contains("有哪些扩展")
        || user_content.contains("什么扩展")
        || user_content.contains("哪些扩展")
        || user_content.contains("MCP")
        || user_content.contains("mcp")
}

fn message_mentions_mcp_install(user_content: &str) -> bool {
    let lowered = user_content.to_ascii_lowercase();
    lowered.contains("install mcp")
        || lowered.contains("install extension")
        || lowered.contains("custom extension")
        || lowered.contains("mcp server")
        || lowered.contains("stdio mcp")
        || lowered.contains("sse mcp")
        || lowered.contains("streamable http")
        || lowered.contains("streamable_http")
        || lowered.contains("playwright-mcp")
        || user_content.contains("安装mcp")
        || user_content.contains("安装 MCP")
        || user_content.contains("安装扩展")
        || user_content.contains("安装一个新的 MCP")
        || user_content.contains("自定义扩展")
        || user_content.contains("stdio")
        || user_content.contains("SSE")
        || user_content.contains("streamable")
}

async fn build_portal_manager_turn_notice(
    db: &MongoDb,
    team_id: &str,
    portal_id: Option<&str>,
    user_content: &str,
) -> Option<String> {
    let mentions_registry = manager_message_mentions_registry(user_content);
    let mentions_extension_inventory = manager_message_mentions_extension_inventory(user_content);

    let mentions_mcp_install = message_mentions_mcp_install(user_content);

    if !mentions_registry
        && !mentions_extension_inventory
        && !mentions_mcp_install
    {
        return None;
    }

    let mut skill_registry_allowed = false;
    if let Some(portal_id) = portal_id.filter(|value| !value.trim().is_empty()) {
        let portal_svc = PortalService::new(db.clone());
        if let Ok(portal) = portal_svc.get(team_id, portal_id).await {
            if let Ok(effective) = portal_svc.resolve_effective_public_config(&portal).await {
                skill_registry_allowed = effective
                    .effective_allowed_extensions
                    .iter()
                    .any(|ext| ext == "skill_registry");
            }
        }
    }

    let mut parts = Vec::new();
    if mentions_registry {
        if skill_registry_allowed {
            parts.push(
                "特别提醒：当前用户正在询问 skills.sh / registry / 热门技能相关问题，并且当前数字分身上下文允许 `skill_registry`。本轮应优先调用 `skill_registry__list_popular_skills`、`skill_registry__search_skills` 或 `skill_registry__preview_skill` 等工具完成查询；只有在工具真实返回上游错误时，才能说明外部接口失败，不能再说“没有该能力”或要求再次申请。"
                    .to_string(),
            );
        } else {
            parts.push(
                "特别提醒：当前用户正在询问 skills.sh / registry / 热门技能相关问题。如果本轮实际不可用 `skill_registry` 工具，应如实说明当前会话未开放 registry 能力或仍需治理审批；禁止再说“401 Missing API key”或把问题归因到缺少 API key。"
                    .to_string(),
            );
        }
    }

    if mentions_extension_inventory {
        parts.push(
            "特别提醒：当前用户正在询问“当前可用的扩展/MCP/已启用扩展”。在 portal_manager 管理会话中，这类问题必须先调用 `portal_tools__get_portal_service_capability_profile`，并优先使用 `profile.serviceAgent.enabledBuiltinExtensionDetails`、`enabledCustomExtensionDetails`、`catalog.teamExtensions` 中的 `display_line_zh` 逐条列出当前真实可用的扩展/MCP。禁止自行概括成 `xxx ✓`、只列原始内部名，或跳过 profile 直接凭记忆回答。若 profile 没有返回可列举项，再明确说明当前没有可显示的扩展详情。".to_string(),
        );
    }

    if mentions_mcp_install {
        parts.push(
            "特别提醒：当前用户正在要求正式安装 MCP/自定义扩展。禁止把 `git clone`、`npm install`、把代码放进当前 workspace、或在 shell 里临时把 server 跑起来描述成“系统已经安装”。在 portal_manager 管理会话中，正式安装必须优先走 `team_mcp__install_team_mcp` 写入团队扩展库；若用户还要求立即给某个 Agent/分身可用，再调用 `team_mcp__attach_team_mcp` 挂载到目标 Agent。更新必须走 `team_mcp__update_team_mcp`，卸载必须走 `team_mcp__remove_team_mcp`。只有在拿到 `name`、`type`（stdio/sse/streamable_http）、`uri_or_cmd` 以及必要 `args/envs` 后才能执行安装；信息不全时先补齐安装计划。若涉及当前数字分身，还应在安装/挂载后调用 `portal_tools__get_portal_service_capability_profile` 回读，确认扩展已出现在 `catalog.teamExtensions`、`enabledCustomExtensionDetails` 或运行边界里。".to_string(),
        );
    }

    Some(parts.join(" "))
}

async fn build_general_turn_notice(
    service: &AgentService,
    team_id: &str,
    agent_id: &str,
    user_content: &str,
) -> Option<String> {
    let mentions_mcp_install = message_mentions_mcp_install(user_content);

    if !mentions_mcp_install {
        return None;
    }

    let mut notices = Vec::new();

    let agent = match service.get_agent(agent_id).await {
        Ok(Some(agent)) if agent.team_id == team_id => agent,
        _ => {
            notices.push(
                "特别提醒：当前用户正在要求安装 MCP/自定义扩展。禁止把 clone 仓库、npm install、把代码放进 workspace 或临时跑通 server 描述成“系统已经安装”。如果当前会话没有正式的扩展管理工具能力，应明确说明需要改用 MCP 工作区 `/teams/{teamId}/mcp/workspace` 或数字分身管理会话完成正式安装。"
                    .to_string(),
            );
            return Some(notices.join(" "));
        }
    };

    if has_manager_tooling(&agent) {
        notices.push(
            "特别提醒：当前用户正在要求正式安装 MCP/自定义扩展。禁止把 clone 仓库、npm install、把代码放进 workspace、或临时跑通 server 当成“系统已经安装”。正式安装必须优先走 `team_mcp__install_team_mcp` 写入团队扩展库；若需要立即给某个 Agent/分身可用，再调用 `team_mcp__attach_team_mcp`。更新走 `team_mcp__update_team_mcp`，卸载走 `team_mcp__remove_team_mcp`。如果当前会话明确绑定了数字分身/服务 Agent，则在挂载后再调用 `portal_tools__get_portal_service_capability_profile` 回读确认；若用户只是想通过 UI 完成安装，则引导使用 MCP 工作区 `/teams/{teamId}/mcp/workspace`。".to_string(),
        );
        return Some(notices.join(" "));
    }

    notices.push(
        "特别提醒：当前用户正在要求安装 MCP/自定义扩展，但本会话不具备正式的扩展管理工具能力。禁止把 workspace 里的临时安装描述成系统安装成功；应明确说明需要切换到具备管理能力的会话，或直接使用 MCP 工作区 `/teams/{teamId}/mcp/workspace` 完成正式安装。".to_string(),
    );
    Some(notices.join(" "))
}

/// Create chat router
pub fn chat_router(
    db: Arc<MongoDb>,
    chat_manager: Arc<ChatManager>,
    workspace_root: String,
) -> Router {
    let service = Arc::new(AgentService::new(db.clone()));

    Router::new()
        .route(
            "/agents/{agent_id}/composer-capabilities",
            get(get_agent_composer_capabilities),
        )
        .route("/sessions", get(list_sessions))
        .route("/sessions", post(create_session))
        .route(
            "/sessions/portal-coding",
            post(create_portal_coding_session),
        )
        .route(
            "/sessions/portal-manager",
            post(create_portal_manager_session),
        )
        .route("/sessions/{id}", get(get_session))
        .route(
            "/sessions/{id}/composer-capabilities",
            get(get_session_composer_capabilities),
        )
        .route("/sessions/{id}", put(update_session))
        .route("/sessions/{id}", delete(delete_session))
        .route("/sessions/{id}/messages", post(send_message))
        .route("/sessions/{id}/stream", get(stream_chat))
        .route("/sessions/{id}/events", get(list_session_events))
        .route("/sessions/{id}/cancel", post(cancel_chat))
        .route("/sessions/{id}/archive", post(archive_session))
        // Phase 2: Document attachment
        .route(
            "/sessions/{id}/documents",
            get(list_attached_documents)
                .post(attach_documents)
                .delete(detach_documents),
        )
        .with_state((service, db, chat_manager, workspace_root))
}

/// GET /chat/sessions - List user's chat sessions
async fn list_sessions(
    State((service, _, _, _)): State<ChatState>,
    Extension(user): Extension<UserContext>,
    Query(mut query): Query<UserSessionListQuery>,
) -> Result<Json<Vec<SessionListItem>>, StatusCode> {
    let is_member = service
        .is_team_member(&user.user_id, &query.team_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    if !is_member {
        return Err(StatusCode::FORBIDDEN);
    }

    // C1 fix: Always inject authenticated user_id to prevent data leakage
    query.user_id = Some(user.user_id.clone());

    service
        .list_user_sessions(query)
        .await
        .map(Json)
        .map_err(|e| {
            tracing::error!("Failed to list sessions: {:?}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })
}

/// POST /chat/sessions - Create a new chat session
async fn create_session(
    State((service, db, _, _)): State<ChatState>,
    Extension(user): Extension<UserContext>,
    Json(req): Json<CreateChatSessionRequest>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    // Look up agent to get team_id
    let team_id = service
        .get_agent_team_id(&req.agent_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    let is_member = service
        .is_team_member(&user.user_id, &team_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    if !is_member {
        return Err(StatusCode::FORBIDDEN);
    }

    // Enforce agent group-based access control
    let user_group_ids =
        agime_team::services::mongo::user_group_service_mongo::UserGroupService::new((*db).clone())
            .get_user_group_ids(&team_id, &user.user_id)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let has_agent_access = service
        .check_agent_access(&req.agent_id, &user.user_id, &user_group_ids)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if !has_agent_access {
        return Err(StatusCode::FORBIDDEN);
    }

    let session = service
        .create_chat_session(
            &team_id,
            &req.agent_id,
            &user.user_id,
            req.attached_document_ids,
            req.extra_instructions,
            req.allowed_extensions,
            req.allowed_skill_ids,
            req.retry_config,
            req.max_turns,
            req.tool_timeout_seconds,
            req.max_portal_retry_rounds,
            req.require_final_report,
            req.portal_restricted,
            req.document_access_mode,
            None,
            None,
            None,
        )
        .await
        .map_err(|e| {
            tracing::error!("Failed to create chat session: {:?}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    Ok(Json(serde_json::json!({
        "session_id": session.session_id,
        "agent_id": session.agent_id,
        "status": session.status,
    })))
}

/// GET /chat/agents/{agent_id}/composer-capabilities - Resolved skills/extensions for a new chat
async fn get_agent_composer_capabilities(
    State((service, db, _, _)): State<ChatState>,
    Extension(user): Extension<UserContext>,
    headers: HeaderMap,
    Path(agent_id): Path<String>,
) -> Result<Json<ComposerCapabilitiesCatalog>, StatusCode> {
    let team_id = service
        .get_agent_team_id(&agent_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    let is_member = service
        .is_team_member(&user.user_id, &team_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if !is_member {
        return Err(StatusCode::FORBIDDEN);
    }

    let user_group_ids =
        agime_team::services::mongo::user_group_service_mongo::UserGroupService::new((*db).clone())
            .get_user_group_ids(&team_id, &user.user_id)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let has_agent_access = service
        .check_agent_access(&agent_id, &user.user_id, &user_group_ids)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if !has_agent_access {
        return Err(StatusCode::FORBIDDEN);
    }

    let agent = service
        .get_agent(&agent_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    let preferred_lang = preferred_lang_from_headers(&headers);

    Ok(Json(
        build_composer_capability_catalog(
            &db,
            &team_id,
            &agent,
            None,
            preferred_lang.as_deref(),
        )
        .await,
    ))
}

/// GET /chat/sessions/{id}/composer-capabilities - Resolved skills/extensions for an existing chat
async fn get_session_composer_capabilities(
    State((service, db, _, _)): State<ChatState>,
    Extension(user): Extension<UserContext>,
    headers: HeaderMap,
    Path(session_id): Path<String>,
) -> Result<Json<ComposerCapabilitiesCatalog>, StatusCode> {
    let session = service
        .get_session(&session_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    if session.user_id != user.user_id {
        return Err(StatusCode::FORBIDDEN);
    }

    let agent = service
        .get_agent(&session.agent_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    let preferred_lang = preferred_lang_from_headers(&headers);

    Ok(Json(
        build_composer_capability_catalog(
            &db,
            &agent.team_id,
            &agent,
            Some(&session),
            preferred_lang.as_deref(),
        )
        .await,
    ))
}

#[derive(serde::Deserialize)]
struct CreatePortalCodingSessionRequest {
    team_id: String,
    portal_id: String,
    #[serde(default)]
    retry_config: Option<RetryConfig>,
    #[serde(default)]
    max_turns: Option<i32>,
    #[serde(default)]
    tool_timeout_seconds: Option<u64>,
    #[serde(default)]
    max_portal_retry_rounds: Option<u32>,
    #[serde(default)]
    require_final_report: Option<bool>,
}

#[derive(serde::Deserialize)]
struct CreatePortalManagerSessionRequest {
    team_id: String,
    #[serde(default)]
    manager_agent_id: Option<String>,
    #[serde(default)]
    portal_id: Option<String>,
    #[serde(default)]
    retry_config: Option<RetryConfig>,
    #[serde(default)]
    max_turns: Option<i32>,
    #[serde(default)]
    tool_timeout_seconds: Option<u64>,
    #[serde(default)]
    max_portal_retry_rounds: Option<u32>,
    #[serde(default)]
    require_final_report: Option<bool>,
}

fn has_manager_tooling(agent: &TeamAgent) -> bool {
    let builtin = agent.enabled_extensions.iter().any(|ext| {
        ext.enabled
            && matches!(
                ext.extension,
                BuiltinExtension::Developer
                    | BuiltinExtension::ExtensionManager
            )
    });
    let custom = agent.custom_extensions.iter().any(|ext| {
        ext.enabled
            && matches!(
                ext.name.trim().to_ascii_lowercase().as_str(),
                "developer" | "portal_tools" | "extension_manager"
            )
    });
    builtin || custom
}

async fn resolve_manager_agent_id(
    service: &AgentService,
    team_id: &str,
    manager_agent_id: Option<&str>,
) -> Result<String, StatusCode> {
    let requested = manager_agent_id
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);

    if let Some(agent_id) = requested {
        let agent = service
            .get_agent(&agent_id)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
            .ok_or(StatusCode::NOT_FOUND)?;
        if agent.team_id != team_id {
            return Err(StatusCode::FORBIDDEN);
        }
        return Ok(agent_id);
    }

    let agents = service
        .list_agents(ListAgentsQuery {
            team_id: team_id.to_string(),
            page: 1,
            limit: 100,
        })
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if agents.items.is_empty() {
        return Err(StatusCode::BAD_REQUEST);
    }

    if let Some(agent) = agents.items.iter().find(|agent| has_manager_tooling(agent)) {
        return Ok(agent.id.clone());
    }

    Ok(agents.items[0].id.clone())
}

/// POST /chat/sessions/portal-coding - Create a portal lab coding session with strict policy.
async fn create_portal_coding_session(
    State((service, db, _, _)): State<ChatState>,
    Extension(user): Extension<UserContext>,
    Json(req): Json<CreatePortalCodingSessionRequest>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let is_member = service
        .is_team_member(&user.user_id, &req.team_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if !is_member {
        return Err(StatusCode::FORBIDDEN);
    }
    let is_admin = service
        .is_team_admin(&user.user_id, &req.team_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if !is_admin {
        return Err(StatusCode::FORBIDDEN);
    }

    let portal_svc = PortalService::new((*db).clone());
    let portal = portal_svc
        .get(&req.team_id, &req.portal_id)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;
    let portal_id = portal
        .id
        .map(|id| id.to_hex())
        .ok_or(StatusCode::INTERNAL_SERVER_ERROR)?;

    let agent_id = portal
        .coding_agent_id
        .clone()
        .or_else(|| portal.agent_id.clone())
        .or_else(|| portal.service_agent_id.clone())
        .ok_or(StatusCode::BAD_REQUEST)?;

    // Enforce agent group-based access control
    let user_group_ids =
        agime_team::services::mongo::user_group_service_mongo::UserGroupService::new((*db).clone())
            .get_user_group_ids(&req.team_id, &user.user_id)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let has_agent_access = service
        .check_agent_access(&agent_id, &user.user_id, &user_group_ids)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if !has_agent_access {
        return Err(StatusCode::FORBIDDEN);
    }

    let raw_project_path = portal.project_path.clone().ok_or(StatusCode::BAD_REQUEST)?;
    let project_path = normalize_workspace_path(&raw_project_path);
    let portal_slug = portal.slug.clone();

    if project_path != raw_project_path {
        if let Err(e) = portal_svc
            .set_project_path(&req.team_id, &portal_id, &project_path)
            .await
        {
            tracing::warn!(
                "Failed to normalize project_path for portal {}: {}",
                portal_id,
                e
            );
        }
    }

    // Ensure project directory exists; auto-create if missing
    if !std::path::Path::new(&project_path).exists() {
        tracing::warn!("Portal project_path missing, recreating: {}", project_path);
        if let Err(e) = std::fs::create_dir_all(&project_path) {
            tracing::error!("Failed to create project dir {}: {}", project_path, e);
            return Err(StatusCode::INTERNAL_SERVER_ERROR);
        }
    }

    // Ensure selected coding agent can actually run developer tools.
    let agent = service
        .get_agent(&agent_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    let has_developer_builtin = agent
        .enabled_extensions
        .iter()
        .any(|ext| ext.enabled && ext.extension == BuiltinExtension::Developer);
    let has_developer_custom = agent
        .custom_extensions
        .iter()
        .any(|ext| ext.enabled && ext.name.trim().eq_ignore_ascii_case("developer"));
    if !has_developer_builtin && !has_developer_custom {
        return Err(StatusCode::BAD_REQUEST);
    }

    // Inject project directory context so the agent knows the current state
    let project_ctx = super::runtime::scan_project_context(&project_path, 8000);
    let portal_policy_overlay = portal.agent_system_prompt.clone();
    let extra = build_portal_coding_overlay(PortalCodingProfileInput {
        portal_slug: &portal_slug,
        project_path: &project_path,
        portal_policy_overlay: portal_policy_overlay.as_deref(),
        project_context: if project_ctx.trim().is_empty() {
            None
        } else {
            Some(project_ctx.as_str())
        },
    });

    let effective_retry_config = req
        .retry_config
        .clone()
        .unwrap_or_else(default_portal_retry_config);

    let session = service
        .create_chat_session(
            &req.team_id,
            &agent_id,
            &user.user_id,
            portal.bound_document_ids.clone(),
            Some(extra),
            None,
            None,
            Some(effective_retry_config),
            req.max_turns,
            req.tool_timeout_seconds,
            req.max_portal_retry_rounds,
            req.require_final_report.unwrap_or(false),
            false,
            Some("full".to_string()),
            Some("portal_coding".to_string()),
            None,
            Some(true),
        )
        .await
        .map_err(|e| {
            tracing::error!("Failed to create portal coding session: {:?}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    service
        .set_session_workspace(&session.session_id, &project_path)
        .await
        .map_err(|e| {
            tracing::error!(
                "Failed to set workspace for portal coding session {}: {:?}",
                session.session_id,
                e
            );
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    service
        .set_session_portal_context(
            &session.session_id,
            &portal_id,
            &portal_slug,
            None,
            Some("full"),
            false,
        )
        .await
        .map_err(|e| {
            tracing::error!(
                "Failed to set portal context for portal coding session {}: {:?}",
                session.session_id,
                e
            );
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    Ok(Json(serde_json::json!({
        "session_id": session.session_id,
        "agent_id": session.agent_id,
        "status": session.status,
        "portal_restricted": false,
        "workspace_path": project_path,
        "allowed_extensions": serde_json::Value::Null,
        "retry_config": session.retry_config,
        "max_turns": session.max_turns,
        "tool_timeout_seconds": session.tool_timeout_seconds,
        "max_portal_retry_rounds": session.max_portal_retry_rounds,
        "require_final_report": session.require_final_report,
    })))
}

/// POST /chat/sessions/portal-manager - Create team-level portal manager session.
/// This session is used to create/configure digital avatars before any portal exists.
async fn create_portal_manager_session(
    State((service, db, _, _)): State<ChatState>,
    Extension(user): Extension<UserContext>,
    Json(req): Json<CreatePortalManagerSessionRequest>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let is_member = service
        .is_team_member(&user.user_id, &req.team_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if !is_member {
        return Err(StatusCode::FORBIDDEN);
    }
    let is_admin = service
        .is_team_admin(&user.user_id, &req.team_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if !is_admin {
        return Err(StatusCode::FORBIDDEN);
    }

    let portal_context = if let Some(portal_id) = req.portal_id.as_deref() {
        let portal = PortalService::new((*db).clone())
            .get(&req.team_id, portal_id)
            .await
            .map_err(|_| StatusCode::NOT_FOUND)?;
        Some(PortalDetail::from(portal))
    } else {
        None
    };

    let requested_manager_agent_id = if let Some(portal) = portal_context.as_ref() {
        let bound_manager_id: Option<&str> = portal
            .coding_agent_id
            .as_deref()
            .or(portal.agent_id.as_deref())
            .map(str::trim)
            .filter(|value: &&str| !value.is_empty());
        match (req.manager_agent_id.as_deref(), bound_manager_id) {
            (Some(requested), Some(bound)) if requested != bound => {
                return Err(StatusCode::BAD_REQUEST);
            }
            (Some(requested), _) => Some(requested),
            (None, Some(bound)) => Some(bound),
            (None, None) => None,
        }
    } else {
        req.manager_agent_id.as_deref()
    };

    let manager_agent_id =
        resolve_manager_agent_id(&service, &req.team_id, requested_manager_agent_id).await?;

    // Enforce agent group-based access control
    let user_group_ids =
        agime_team::services::mongo::user_group_service_mongo::UserGroupService::new((*db).clone())
            .get_user_group_ids(&req.team_id, &user.user_id)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let has_agent_access = service
        .check_agent_access(&manager_agent_id, &user.user_id, &user_group_ids)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if !has_agent_access {
        return Err(StatusCode::FORBIDDEN);
    }

    let mut extra = build_portal_manager_overlay();
    if let Some(portal) = portal_context.as_ref() {
        let service_agent_id = portal
            .service_agent_id
            .as_deref()
            .or(portal.agent_id.as_deref())
            .map(str::trim)
            .filter(|value: &&str| !value.is_empty())
            .unwrap_or("未配置");
        let bound_documents = portal.bound_document_ids.len();
        let document_access_mode = format!("{:?}", portal.document_access_mode).to_lowercase();
        let allowed_extensions = portal
            .allowed_extensions
            .as_ref()
            .map(|items: &Vec<String>| items.join(", "))
            .filter(|value: &String| !value.trim().is_empty())
            .unwrap_or_else(|| "继承服务 Agent".to_string());
        let allowed_skills = portal
            .allowed_skill_ids
            .as_ref()
            .map(|items: &Vec<String>| items.join(", "))
            .filter(|value: &String| !value.trim().is_empty())
            .unwrap_or_else(|| "继承服务 Agent".to_string());
        extra.push_str(
            &format!(
                "\n\n【Current Avatar Context】\n当前默认工作目标数字分身：{name}\nportal_id: {portal_id}\nslug: {slug}\nservice_agent_id: {service_agent_id}\ndocument_access_mode: {document_access_mode}\nbound_document_count: {bound_documents}\nallowed_extensions: {allowed_extensions}\nallowed_skill_ids: {allowed_skills}\n说明：除非用户明确切换其他分身，本会话里的创建、配置、治理、审批与发布默认都针对这个数字分身。",
                name = portal.name,
                portal_id = portal.id,
                slug = portal.slug,
                service_agent_id = service_agent_id,
                document_access_mode = document_access_mode,
                bound_documents = bound_documents,
                allowed_extensions = allowed_extensions,
                allowed_skills = allowed_skills,
            ),
        );
    }

    let effective_retry_config = req
        .retry_config
        .clone()
        .unwrap_or_else(default_portal_retry_config);

    let session = service
        .create_chat_session(
            &req.team_id,
            &manager_agent_id,
            &user.user_id,
            Vec::new(),
            Some(extra),
            None,
            None,
            Some(effective_retry_config),
            req.max_turns,
            req.tool_timeout_seconds,
            req.max_portal_retry_rounds,
            req.require_final_report.unwrap_or(false),
            false,
            Some("full".to_string()),
            Some("portal_manager".to_string()),
            None,
            Some(true),
        )
        .await
        .map_err(|e| {
            tracing::error!("Failed to create portal manager session: {:?}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    if let Some(portal) = portal_context.as_ref() {
        if let Err(err) = service
            .set_session_portal_context(
                &session.session_id,
                &portal.id,
                &portal.slug,
                None,
                Some("full"),
                false,
            )
            .await
        {
            tracing::error!(
                session_id = %session.session_id,
                portal_id = %portal.id,
                "Failed to bind portal manager session to avatar context: {:?}",
                err
            );
            let _ = service.delete_session(&session.session_id).await;
            return Err(StatusCode::INTERNAL_SERVER_ERROR);
        }
    }

    Ok(Json(serde_json::json!({
        "session_id": session.session_id,
        "agent_id": session.agent_id,
        "status": session.status,
        "portal_restricted": false,
        "portal_id": portal_context.as_ref().map(|portal| portal.id.clone()),
        "portal_slug": portal_context.as_ref().map(|portal| portal.slug.clone()),
        "allowed_extensions": serde_json::Value::Null,
        "retry_config": session.retry_config,
        "max_turns": session.max_turns,
        "tool_timeout_seconds": session.tool_timeout_seconds,
        "max_portal_retry_rounds": session.max_portal_retry_rounds,
        "require_final_report": session.require_final_report,
    })))
}

/// GET /chat/sessions/{id} - Get session details with messages
async fn get_session(
    State((service, _, _, _)): State<ChatState>,
    Extension(user): Extension<UserContext>,
    Path(session_id): Path<String>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let session = service
        .get_session(&session_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    // Verify ownership
    if session.user_id != user.user_id {
        return Err(StatusCode::FORBIDDEN);
    }

    // H4 fix: Convert bson::DateTime to ISO 8601 strings for frontend
    let mut json = serde_json::json!({
        "session_id": session.session_id,
        "team_id": session.team_id,
        "agent_id": session.agent_id,
        "user_id": session.user_id,
        "name": session.name,
        "status": session.status,
        "messages_json": session.messages_json,
        "message_count": session.message_count,
        "total_tokens": session.total_tokens,
        "input_tokens": session.input_tokens,
        "output_tokens": session.output_tokens,
        "compaction_count": session.compaction_count,
        "disabled_extensions": session.disabled_extensions,
        "enabled_extensions": session.enabled_extensions,
        "created_at": session.created_at.to_chrono().to_rfc3339(),
        "updated_at": session.updated_at.to_chrono().to_rfc3339(),
        "title": session.title,
        "pinned": session.pinned,
        "last_message_preview": session.last_message_preview,
        "is_processing": session.is_processing,
        "last_execution_status": session.last_execution_status,
        "last_execution_error": session.last_execution_error,
        "workspace_path": session.workspace_path,
        "extra_instructions": session.extra_instructions,
        "allowed_extensions": session.allowed_extensions,
        "allowed_skill_ids": session.allowed_skill_ids,
        "retry_config": session.retry_config,
        "max_turns": session.max_turns,
        "tool_timeout_seconds": session.tool_timeout_seconds,
        "max_portal_retry_rounds": session.max_portal_retry_rounds,
        "require_final_report": session.require_final_report,
        "portal_restricted": session.portal_restricted,
        "document_access_mode": session.document_access_mode,
        "portal_id": session.portal_id,
        "portal_slug": session.portal_slug,
        "visitor_id": session.visitor_id,
        "session_source": session.session_source,
        "source_mission_id": session.source_mission_id,
        "hidden_from_chat_list": session.hidden_from_chat_list,
    });

    if let Some(lma) = session.last_message_at {
        json["last_message_at"] = serde_json::Value::String(lma.to_chrono().to_rfc3339());
    }
    if let Some(finished_at) = session.last_execution_finished_at {
        json["last_execution_finished_at"] =
            serde_json::Value::String(finished_at.to_chrono().to_rfc3339());
    }

    Ok(Json(json))
}

/// PUT /chat/sessions/{id} - Update session (rename/pin)
#[derive(serde::Deserialize)]
struct UpdateSessionBody {
    title: Option<String>,
    pinned: Option<bool>,
}

async fn update_session(
    State((service, _, _, _)): State<ChatState>,
    Extension(user): Extension<UserContext>,
    Path(session_id): Path<String>,
    Json(body): Json<UpdateSessionBody>,
) -> Result<StatusCode, StatusCode> {
    let session = service
        .get_session(&session_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    if session.user_id != user.user_id {
        return Err(StatusCode::FORBIDDEN);
    }

    if let Some(title) = &body.title {
        service
            .rename_session(&session_id, title)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    }

    if let Some(pinned) = body.pinned {
        service
            .pin_session(&session_id, pinned)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    }

    Ok(StatusCode::OK)
}

/// POST /chat/sessions/{id}/messages - Send a message (triggers execution)
async fn send_message(
    State((service, db, chat_manager, ref workspace_root)): State<ChatState>,
    Extension(user): Extension<UserContext>,
    Path(session_id): Path<String>,
    Json(req): Json<SendChatMessageRequest>,
) -> Result<Json<SendMessageResponse>, StatusCode> {
    // M7: Validate content is not empty or too long
    let content = req.content.trim().to_string();
    if content.is_empty() || content.len() > 100_000 {
        return Err(StatusCode::BAD_REQUEST);
    }

    // Verify session exists and user owns it
    let session = service
        .get_session(&session_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    if session.user_id != user.user_id {
        return Err(StatusCode::FORBIDDEN);
    }

    if session
        .session_source
        .eq_ignore_ascii_case("portal_manager")
    {
        if let Some(turn_notice) = build_portal_manager_turn_notice(
            db.as_ref(),
            &session.team_id,
            session.portal_id.as_deref(),
            &content,
        )
        .await
        {
            if let Err(err) = service
                .append_hidden_session_notice(&session_id, &turn_notice)
                .await
            {
                tracing::warn!(
                    session_id = %session_id,
                    "Failed to append portal_manager turn notice: {}",
                    err
                );
            }
        }
    } else if let Some(turn_notice) = build_general_turn_notice(
        service.as_ref(),
        &session.team_id,
        &session.agent_id,
        &content,
    )
    .await
    {
        if let Err(err) = service
            .append_hidden_session_notice(&session_id, &turn_notice)
            .await
        {
            tracing::warn!(
                session_id = %session_id,
                "Failed to append general MCP turn notice: {}",
                err
            );
        }
    }

    // Register in ChatManager first (authoritative in-memory gate)
    let (cancel_token, _stream_tx) = match chat_manager.register(&session_id).await {
        Some(pair) => pair,
        None => return Err(StatusCode::CONFLICT),
    };

    // Then set MongoDB is_processing flag (secondary persistence)
    let claimed = service
        .try_start_processing(&session_id, &user.user_id)
        .await
        .map_err(|e| {
            tracing::error!("try_start_processing DB error for {}: {}", session_id, e);
            StatusCode::INTERNAL_SERVER_ERROR
        });
    match claimed {
        Ok(true) => {}
        _ => {
            // Rollback ChatManager registration
            chat_manager.unregister(&session_id).await;
            return Err(claimed.err().unwrap_or(StatusCode::CONFLICT));
        }
    }

    // Spawn background execution
    let executor = ChatExecutor::new(db.clone(), chat_manager.clone(), workspace_root.clone());
    let sid = session_id.clone();
    let agent_id = session.agent_id.clone();

    tokio::spawn(async move {
        if let Err(e) = executor
            .execute_chat(&sid, &agent_id, &content, cancel_token)
            .await
        {
            tracing::error!("Chat execution failed for session {}: {}", sid, e);
        }
    });

    Ok(Json(SendMessageResponse {
        session_id,
        streaming: true,
    }))
}

/// GET /chat/sessions/{id}/stream - SSE stream for chat events
async fn stream_chat(
    State((service, _, chat_manager, _)): State<ChatState>,
    Extension(user): Extension<UserContext>,
    headers: HeaderMap,
    Path(session_id): Path<String>,
    Query(q): Query<StreamQuery>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, StatusCode> {
    // Verify ownership
    let session = service
        .get_session(&session_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    if session.user_id != user.user_id {
        return Err(StatusCode::FORBIDDEN);
    }

    let last_event_id = q.last_event_id.or_else(|| {
        headers
            .get("last-event-id")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.parse::<u64>().ok())
    });

    // Subscribe to chat events with buffered history for reconnect/late join.
    let (mut rx, history) = chat_manager
        .subscribe_with_history(&session_id, last_event_id)
        .await
        .ok_or(StatusCode::NOT_FOUND)?;

    let stream = async_stream::stream! {
        yield Ok(Event::default()
            .event("status")
            .data(serde_json::json!({
                "type": "Status",
                "status": "running"
            }).to_string()));

        for event in history {
            let is_done = event.event.is_done();
            let json = serde_json::to_string(&event.event).unwrap_or_default();
            let mut sse = Event::default().event(event.event.event_type()).data(json);
            if event.id > 0 {
                sse = sse.id(event.id.to_string());
            }
            yield Ok(sse);
            if is_done {
                return;
            }
        }

        let sse_lifetime_secs = std::env::var("TEAM_SSE_MAX_LIFETIME_SECS")
            .ok().and_then(|v| v.parse::<u64>().ok()).unwrap_or(2 * 60 * 60);
        let deadline = tokio::time::Instant::now() + Duration::from_secs(sse_lifetime_secs);

        loop {
            match tokio::time::timeout_at(deadline, rx.recv()).await {
                Ok(Ok(event)) => {
                    let is_done = event.event.is_done();
                    let json = serde_json::to_string(&event.event).unwrap_or_default();
                    let mut sse = Event::default().event(event.event.event_type()).data(json);
                    if event.id > 0 {
                        sse = sse.id(event.id.to_string());
                    }
                    yield Ok(sse);
                    if is_done {
                        break;
                    }
                }
                Ok(Err(tokio::sync::broadcast::error::RecvError::Closed)) => break,
                Ok(Err(tokio::sync::broadcast::error::RecvError::Lagged(_))) => continue,
                Err(_) => {
                    tracing::info!("SSE stream deadline reached, closing for client reconnect");
                    break;
                }
            }
        }
    };

    Ok(Sse::new(stream).keep_alive(
        axum::response::sse::KeepAlive::new()
            .interval(std::time::Duration::from_secs(15))
            .text("ping"),
    ))
}

fn fix_bson_dates(val: &mut serde_json::Value) {
    match val {
        serde_json::Value::Object(map) => {
            if map.len() == 1 && map.contains_key("$date") {
                if let Some(date_val) = map.get("$date") {
                    if let Some(date_obj) = date_val.as_object() {
                        if let Some(ms) = date_obj.get("$numberLong").and_then(|v| v.as_str()) {
                            if let Ok(ts) = ms.parse::<i64>() {
                                if let Some(dt) =
                                    chrono::DateTime::<chrono::Utc>::from_timestamp_millis(ts)
                                {
                                    *val = serde_json::Value::String(dt.to_rfc3339());
                                    return;
                                }
                            }
                        }
                    }
                    if let Some(s) = date_val.as_str() {
                        *val = serde_json::Value::String(s.to_string());
                        return;
                    }
                }
            }
            for v in map.values_mut() {
                fix_bson_dates(v);
            }
        }
        serde_json::Value::Array(arr) => {
            for v in arr.iter_mut() {
                fix_bson_dates(v);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::{
        manager_message_mentions_extension_inventory, manager_message_mentions_registry,
        manager_message_mentions_skill_inventory, message_mentions_mcp_install,
    };

    #[test]
    fn manager_skill_inventory_detection_matches_cn_and_en_queries() {
        assert!(manager_message_mentions_skill_inventory(
            "你找一下你目前能用的skills"
        ));
        assert!(manager_message_mentions_skill_inventory(
            "what skills are currently available"
        ));
        assert!(manager_message_mentions_skill_inventory("有哪些技能"));
        assert!(!manager_message_mentions_skill_inventory("帮我打开访客页"));
    }

    #[test]
    fn manager_registry_detection_matches_registry_queries() {
        assert!(manager_message_mentions_registry(
            "帮我看看 skills.sh 热门技能"
        ));
        assert!(manager_message_mentions_registry(
            "search registry trending skills"
        ));
        assert!(manager_message_mentions_registry("热门技能有哪些"));
        assert!(!manager_message_mentions_registry("查看当前分身权限"));
    }

    #[test]
    fn manager_extension_inventory_detection_matches_extension_queries() {
        assert!(manager_message_mentions_extension_inventory(
            "列出当前可用的扩展和MCP"
        ));
        assert!(manager_message_mentions_extension_inventory(
            "what extensions are currently enabled"
        ));
        assert!(manager_message_mentions_extension_inventory("有哪些扩展"));
        assert!(!manager_message_mentions_extension_inventory(
            "帮我列出文档"
        ));
    }

    #[test]
    fn mcp_install_detection_matches_cn_and_en_queries() {
        assert!(message_mentions_mcp_install("安装一个新的 MCP"));
        assert!(message_mentions_mcp_install("install mcp server"));
        assert!(message_mentions_mcp_install("给这个分身加一个自定义扩展"));
        assert!(!message_mentions_mcp_install("帮我列出文档"));
    }
}

/// GET /chat/sessions/{id}/events - List persisted runtime events.
async fn list_session_events(
    State((service, _, chat_manager, _)): State<ChatState>,
    Extension(user): Extension<UserContext>,
    Path(session_id): Path<String>,
    Query(q): Query<EventListQuery>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let session = service
        .get_session(&session_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    if session.user_id != user.user_id {
        let is_admin = service
            .is_team_admin(&user.user_id, &session.team_id)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        if !is_admin {
            return Err(StatusCode::FORBIDDEN);
        }
    }

    let limit = q.limit.unwrap_or(500).clamp(1, 2000);
    let descending = q
        .order
        .as_deref()
        .map(str::trim)
        .map(|v| v.eq_ignore_ascii_case("desc"))
        .unwrap_or(false);
    let explicit_run_id = q.run_id.as_deref().map(str::trim).filter(|s| !s.is_empty());
    let selected_run_id: Option<String> = match explicit_run_id {
        Some(rid)
            if rid.eq_ignore_ascii_case("__all__")
                || rid.eq_ignore_ascii_case("all")
                || rid == "*" =>
        {
            None
        }
        Some(rid) => Some(rid.to_string()),
        None => chat_manager.active_run_id(&session_id).await,
    };

    let events = service
        .list_chat_events(
            &session_id,
            selected_run_id.as_deref(),
            q.after_event_id,
            q.before_event_id,
            limit,
            descending,
        )
        .await
        .map_err(|e| {
            tracing::error!("Failed to list chat events for {}: {:?}", session_id, e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    let mut value = serde_json::to_value(events).unwrap_or_default();
    fix_bson_dates(&mut value);
    Ok(Json(value))
}

/// POST /chat/sessions/{id}/cancel - Cancel active chat
async fn cancel_chat(
    State((service, _, chat_manager, _)): State<ChatState>,
    Extension(user): Extension<UserContext>,
    Path(session_id): Path<String>,
) -> Result<StatusCode, StatusCode> {
    let session = service
        .get_session(&session_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    if session.user_id != user.user_id {
        return Err(StatusCode::FORBIDDEN);
    }

    let cancelled = chat_manager.cancel(&session_id).await;
    if cancelled {
        let _ = service.set_session_processing(&session_id, false).await;
        Ok(StatusCode::OK)
    } else {
        Err(StatusCode::NOT_FOUND)
    }
}

/// POST /chat/sessions/{id}/archive - Archive session
async fn archive_session(
    State((service, _, _, _)): State<ChatState>,
    Extension(user): Extension<UserContext>,
    Path(session_id): Path<String>,
) -> Result<StatusCode, StatusCode> {
    let session = service
        .get_session(&session_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    if session.user_id != user.user_id {
        return Err(StatusCode::FORBIDDEN);
    }

    // M8: Atomic archive — only succeeds if session is not processing
    let archived = service
        .archive_session_if_idle(&session_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    if archived {
        Ok(StatusCode::OK)
    } else {
        Err(StatusCode::CONFLICT)
    }
}

/// DELETE /chat/sessions/{id} - Permanently delete session
async fn delete_session(
    State((service, _, _, _)): State<ChatState>,
    Extension(user): Extension<UserContext>,
    Path(session_id): Path<String>,
) -> Result<StatusCode, StatusCode> {
    let session = service
        .get_session(&session_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    if session.user_id != user.user_id {
        return Err(StatusCode::FORBIDDEN);
    }

    // M8: Atomic delete — only succeeds if session is not processing
    let deleted = service
        .delete_session_if_idle(&session_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    if deleted {
        // P2: Best-effort workspace cleanup (after DB delete to avoid orphaned records)
        if let Err(e) = super::runtime::cleanup_workspace_dir(session.workspace_path.as_deref()) {
            tracing::warn!(
                "Failed to cleanup workspace for session {}: {}",
                session_id,
                e
            );
        }
        Ok(StatusCode::NO_CONTENT)
    } else {
        // Session was verified above but disappeared before delete — concurrent deletion
        Err(StatusCode::CONFLICT)
    }
}

// ── Phase 2: Document attachment routes ──

#[derive(serde::Deserialize)]
struct DocumentIdsBody {
    document_ids: Vec<String>,
}

/// POST /chat/sessions/{id}/documents - Attach documents
async fn attach_documents(
    State((service, _, _, _)): State<ChatState>,
    Extension(user): Extension<UserContext>,
    Path(session_id): Path<String>,
    Json(body): Json<DocumentIdsBody>,
) -> Result<StatusCode, StatusCode> {
    let session = service
        .get_session(&session_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    if session.user_id != user.user_id {
        return Err(StatusCode::FORBIDDEN);
    }

    service
        .attach_documents_to_session(&session_id, &body.document_ids)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(StatusCode::OK)
}

/// DELETE /chat/sessions/{id}/documents - Detach documents
async fn detach_documents(
    State((service, _, _, _)): State<ChatState>,
    Extension(user): Extension<UserContext>,
    Path(session_id): Path<String>,
    Json(body): Json<DocumentIdsBody>,
) -> Result<StatusCode, StatusCode> {
    let session = service
        .get_session(&session_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    if session.user_id != user.user_id {
        return Err(StatusCode::FORBIDDEN);
    }

    service
        .detach_documents_from_session(&session_id, &body.document_ids)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(StatusCode::NO_CONTENT)
}

/// GET /chat/sessions/{id}/documents - List attached documents
async fn list_attached_documents(
    State((service, _, _, _)): State<ChatState>,
    Extension(user): Extension<UserContext>,
    Path(session_id): Path<String>,
) -> Result<Json<Vec<String>>, StatusCode> {
    let session = service
        .get_session(&session_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    if session.user_id != user.user_id {
        return Err(StatusCode::FORBIDDEN);
    }

    Ok(Json(session.attached_document_ids))
}
