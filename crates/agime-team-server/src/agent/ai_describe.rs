//! AI Describe service for generating human-friendly descriptions
//! of extensions and skills using LLM APIs.
//!
//! Uses the same Provider abstraction as document analysis and chat,
//! so all API format differences (Volcengine, Anthropic, OpenAI) are
//! handled uniformly by `provider_factory::create_provider_for_agent`.

use agime::conversation::message::Message;
use agime::providers::base::Provider;
use agime_team::models::{ApiFormat, TeamAgent};
use agime_team::MongoDb;
use chrono::{DateTime, Utc};
use mongodb::bson::{doc, oid::ObjectId};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fmt;
use std::sync::{Arc, Mutex};

use super::provider_factory;
use super::service_mongo::AgentService;
use crate::config::Config;

/// Typed errors for AI Describe operations
#[derive(Debug)]
pub enum AiDescribeError {
    NotConfigured,
    NotFound(&'static str),
    InvalidInput(String),
    LlmError(String),
    Internal(String),
}

impl fmt::Display for AiDescribeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotConfigured => write!(f, "AI service not configured"),
            Self::NotFound(what) => write!(f, "{} not found", what),
            Self::InvalidInput(msg) => write!(f, "Invalid input: {}", msg),
            Self::LlmError(msg) => write!(f, "LLM API error: {}", msg),
            Self::Internal(msg) => write!(f, "Internal error: {}", msg),
        }
    }
}

impl std::error::Error for AiDescribeError {}

impl From<mongodb::error::Error> for AiDescribeError {
    fn from(e: mongodb::error::Error) -> Self {
        Self::Internal(e.to_string())
    }
}

/// A single AI insight item for the insights summary endpoint
#[derive(Debug, Serialize)]
pub struct InsightItem {
    pub id: String,
    #[serde(rename = "type")]
    pub item_type: String,
    pub name: String,
    pub ai_description: String,
    pub ai_description_lang: String,
    pub ai_described_at: DateTime<Utc>,
}

/// Response for the describe endpoints
#[derive(Debug, Serialize)]
pub struct DescribeResponse {
    pub description: String,
    pub lang: String,
    pub generated_at: DateTime<Utc>,
}

/// Response for the insights endpoint
#[derive(Debug, Serialize)]
pub struct InsightsResponse {
    pub insights: Vec<InsightItem>,
    pub total: usize,
}

/// Request body for describe endpoints
#[derive(Debug, Deserialize)]
pub struct DescribeRequest {
    #[serde(default = "default_lang")]
    pub lang: String,
}

fn default_lang() -> String {
    "zh".to_string()
}

/// Request body for builtin extension describe endpoint
#[derive(Debug, Deserialize)]
pub struct BuiltinDescribeRequest {
    pub id: String,
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub is_platform: bool,
    #[serde(default = "default_lang")]
    pub lang: String,
}

/// Metadata for a known built-in extension: (id, name, description, is_platform)
pub type BuiltinMeta = (&'static str, &'static str, &'static str, bool);

/// Known built-in extensions -- single source of truth for IDs and metadata.
pub const KNOWN_BUILTINS: &[BuiltinMeta] = &[
    ("skills", "Skills", "Load and use skills", true),
    ("todo", "Todo", "Task tracking", true),
    (
        "extension_manager",
        "Extension Manager",
        "Extension management",
        true,
    ),
    ("team", "Team", "Team collaboration", true),
    ("chat_recall", "Chat Recall", "Conversation memory", true),
    (
        "document_tools",
        "Document Tools",
        "Read, create, search and list team documents",
        true,
    ),
    (
        "developer",
        "Developer",
        "File editing and shell commands",
        false,
    ),
    ("memory", "Memory", "Knowledge base", false),
    (
        "computer_controller",
        "Computer Controller",
        "Computer control",
        false,
    ),
    (
        "auto_visualiser",
        "Auto Visualiser",
        "Auto visualization",
        false,
    ),
    ("tutorial", "Tutorial", "Tutorials", false),
];

/// Check whether an ID belongs to a known built-in extension.
pub fn is_known_builtin(id: &str) -> bool {
    KNOWN_BUILTINS.iter().any(|(bid, _, _, _)| *bid == id)
}

/// Metadata for a known built-in skill: (id, name, description)
pub type BuiltinSkillMeta = (&'static str, &'static str, &'static str);

/// Known built-in skills managed by the Skills MCP extension.
pub const KNOWN_BUILTIN_SKILLS: &[BuiltinSkillMeta] = &[
    (
        "team-onboarding",
        "Team Onboarding",
        "团队协作入职指南 - 如何使用团队功能搜索、安装和分享资源",
    ),
    (
        "extension-security-review",
        "Extension Security Review",
        "MCP Extension 安全审核指南 - 评估和审核团队共享的扩展",
    ),
];

/// Check whether an ID belongs to a known built-in skill.
pub fn is_known_builtin_skill(id: &str) -> bool {
    KNOWN_BUILTIN_SKILLS.iter().any(|(sid, _, _)| *sid == id)
}

/// Query params for insights endpoint
#[derive(Debug, Deserialize)]
pub struct InsightsQuery {
    #[serde(default = "default_lang")]
    pub lang: String,
}

pub struct AiDescribeService {
    db: Arc<MongoDb>,
    config: Arc<Config>,
    agent_service: Arc<AgentService>,
    /// Track in-flight LLM calls to prevent duplicate requests for the same resource
    in_flight: Mutex<HashSet<String>>,
}

impl AiDescribeService {
    pub fn new(db: Arc<MongoDb>, config: Arc<Config>, agent_service: Arc<AgentService>) -> Self {
        Self {
            db,
            config,
            agent_service,
            in_flight: Mutex::new(HashSet::new()),
        }
    }

    /// Try to acquire an in-flight slot for the given key.
    /// Returns false if a request for this key is already in progress.
    fn try_acquire(&self, key: &str) -> bool {
        self.in_flight.lock().unwrap().insert(key.to_string())
    }

    /// Release an in-flight slot.
    fn release(&self, key: &str) {
        self.in_flight.lock().unwrap().remove(key);
    }

    /// Parse `ai_described_at` from a BSON document, handling both RFC 3339 strings
    /// (new format) and BSON DateTime (legacy format). Falls back to `Utc::now()`.
    fn parse_described_at(doc: &mongodb::bson::Document) -> DateTime<Utc> {
        doc.get_str("ai_described_at")
            .ok()
            .and_then(|s| s.parse::<DateTime<Utc>>().ok())
            .or_else(|| {
                doc.get_datetime("ai_described_at")
                    .ok()
                    .map(|dt| dt.to_chrono())
            })
            .unwrap_or_else(Utc::now)
    }

    /// Validate lang parameter
    fn validate_lang(lang: &str) -> Result<&str, AiDescribeError> {
        match lang {
            "zh" | "en" => Ok(lang),
            _ => Err(AiDescribeError::InvalidInput(format!(
                "Unsupported language: '{}', expected 'zh' or 'en'",
                lang
            ))),
        }
    }

    /// Resolve a Provider instance: env vars first, then fall back to team agent.
    /// Uses the same provider_factory as document analysis and chat.
    async fn get_provider(&self, team_id: &str) -> Result<Arc<dyn Provider>, AiDescribeError> {
        let agent = if let Some(ref api_key) = self.config.ai_describe_api_key {
            // Path 1: dedicated env vars -> synthetic TeamAgent
            let model = self
                .config
                .ai_describe_model
                .clone()
                .unwrap_or_else(|| "gpt-4o-mini".to_string());
            let api_url = self
                .config
                .ai_describe_api_url
                .clone()
                .unwrap_or_else(|| "https://api.openai.com/v1".to_string());
            let api_format: ApiFormat = self
                .config
                .ai_describe_api_format
                .as_deref()
                .unwrap_or("openai")
                .parse()
                .unwrap_or_default();

            let mut agent = TeamAgent::new(team_id.to_string(), "ai-describe-env".to_string())
                .with_api(api_url, model, api_format);
            agent.api_key = Some(api_key.clone());
            agent
        } else {
            // Path 2: team agent config (same as document analysis)
            self.agent_service
                .get_first_agent_with_key(team_id)
                .await
                .map_err(|e| AiDescribeError::Internal(e.to_string()))?
                .ok_or(AiDescribeError::NotConfigured)?
        };

        provider_factory::create_provider_for_agent(&agent)
            .map_err(|e| AiDescribeError::Internal(format!("Provider creation failed: {}", e)))
    }

    /// Call LLM via Provider abstraction with system and user prompts
    async fn call_llm(
        &self,
        provider: &dyn Provider,
        system: &str,
        user: &str,
    ) -> Result<String, AiDescribeError> {
        let messages = vec![Message::user().with_text(user)];
        let (response, _usage) = provider
            .complete(system, &messages, &[])
            .await
            .map_err(|e| AiDescribeError::LlmError(format!("Provider error: {}", e)))?;

        // Extract text from response message
        response
            .content
            .iter()
            .find_map(|c| c.as_text().map(|s| s.to_string()))
            .ok_or_else(|| AiDescribeError::LlmError("Empty response from LLM".into()))
    }

    fn extension_system_prompt(lang: &str) -> &'static str {
        match lang {
            "zh" => {
                "\
你是技术文档专家。请用简洁通俗的中文描述这个 MCP 扩展的功能，让非技术人员也能理解。\n\
重点关注：\n\
- 这个扩展连接了什么外部服务或系统\n\
- 它提供了哪些工具/能力给 AI Agent\n\
- 配置项的含义和作用\n\n\
严格使用以下 Markdown 格式输出：\n\n\
一段简短摘要（1-2句话，说明扩展的核心用途）\n\n\
**提供的能力**\n\n\
- 能力1\n\
- 能力2\n\n\
**配置说明**\n\n\
- 关键配置项及其作用\n\n\
不要添加其他标题或多余内容。"
            }
            _ => {
                "\
You are a technical documentation expert. Describe this MCP extension in plain English.\n\
Focus on:\n\
- What external service or system it connects to\n\
- What tools/capabilities it provides to the AI Agent\n\
- What the configuration options mean\n\n\
Use this exact Markdown format:\n\n\
A brief summary (1-2 sentences on core purpose)\n\n\
**Capabilities Provided**\n\n\
- Capability 1\n\
- Capability 2\n\n\
**Configuration Notes**\n\n\
- Key config options and their purpose\n\n\
Do not add extra headings or filler."
            }
        }
    }

    fn skill_system_prompt(lang: &str) -> &'static str {
        match lang {
            "zh" => {
                "\
你是技术文档专家。请用简洁通俗的中文描述这个技能(Skill)的功能，让非技术人员也能理解。\n\
重点关注：\n\
- 这个技能让 AI Agent 能做什么\n\
- 使用场景和触发条件\n\
- 执行的关键步骤\n\n\
严格使用以下 Markdown 格式输出：\n\n\
一段简短摘要（1-2句话，说明技能的核心用途）\n\n\
**使用场景**\n\n\
- 场景1\n\
- 场景2\n\n\
**执行步骤**\n\n\
- 步骤1\n\
- 步骤2\n\n\
不要添加其他标题或多余内容。"
            }
            _ => {
                "\
You are a technical documentation expert. Describe this Skill in plain English.\n\
Focus on:\n\
- What the AI Agent can do with this skill\n\
- Usage scenarios and trigger conditions\n\
- Key execution steps\n\n\
Use this exact Markdown format:\n\n\
A brief summary (1-2 sentences on core purpose)\n\n\
**Usage Scenarios**\n\n\
- Scenario 1\n\
- Scenario 2\n\n\
**Execution Steps**\n\n\
- Step 1\n\
- Step 2\n\n\
Do not add extra headings or filler."
            }
        }
    }

    fn builtin_ext_system_prompt(lang: &str) -> &'static str {
        match lang {
            "zh" => "\
你是技术文档专家。请用简洁通俗的中文描述这个内置扩展在 AI Agent 架构中的角色，让非技术人员也能理解。\n\
重点关注：\n\
- 它在 Agent 工作流中承担什么职责\n\
- 它是平台级组件还是可选模块\n\
- 对用户的实际价值\n\n\
严格使用以下 Markdown 格式输出：\n\n\
一段简短摘要（1-2句话，说明在 Agent 中的角色）\n\n\
**核心职责**\n\n\
- 职责1\n\
- 职责2\n\n\
不要添加其他标题或多余内容。",
            _ => "\
You are a technical documentation expert. Describe this built-in extension's role in the AI Agent architecture.\n\
Focus on:\n\
- What responsibility it has in the Agent workflow\n\
- Whether it's a platform component or optional module\n\
- Practical value to users\n\n\
Use this exact Markdown format:\n\n\
A brief summary (1-2 sentences on its Agent role)\n\n\
**Core Responsibilities**\n\n\
- Responsibility 1\n\
- Responsibility 2\n\n\
Do not add extra headings or filler.",
        }
    }

    fn builtin_skill_system_prompt(lang: &str) -> &'static str {
        match lang {
            "zh" => "\
你是技术文档专家。请用简洁通俗的中文描述这个内置技能在 AI Agent 工作流中的作用，让非技术人员也能理解。\n\
重点关注：\n\
- 它帮助用户完成什么任务\n\
- 什么时候会用到这个技能\n\
- 它的输出结果是什么\n\n\
严格使用以下 Markdown 格式输出：\n\n\
一段简短摘要（1-2句话，说明技能的核心价值）\n\n\
**适用场景**\n\n\
- 场景1\n\
- 场景2\n\n\
**输出内容**\n\n\
- 输出1\n\
- 输出2\n\n\
不要添加其他标题或多余内容。",
            _ => "\
You are a technical documentation expert. Describe this built-in skill's role in the AI Agent workflow.\n\
Focus on:\n\
- What task it helps users accomplish\n\
- When this skill is used\n\
- What it outputs\n\n\
Use this exact Markdown format:\n\n\
A brief summary (1-2 sentences on core value)\n\n\
**Use Cases**\n\n\
- Case 1\n\
- Case 2\n\n\
**Output**\n\n\
- Output 1\n\
- Output 2\n\n\
Do not add extra headings or filler.",
        }
    }

    fn truncate_content(s: &str, max_chars: usize) -> String {
        // Fast path: byte length is always >= char count, so if bytes fit, chars fit too
        if s.len() <= max_chars {
            return s.to_string();
        }
        // Slow path: count actual chars for multi-byte content
        if s.chars().count() <= max_chars {
            return s.to_string();
        }
        let truncated: String = s.chars().take(max_chars).collect();
        format!("{}... [truncated]", truncated)
    }

    /// Generate or return cached AI description for an extension
    pub async fn describe_extension(
        &self,
        team_id: &str,
        ext_id: &str,
        lang: &str,
    ) -> Result<DescribeResponse, AiDescribeError> {
        Self::validate_lang(lang)?;
        let ext_oid = ObjectId::parse_str(ext_id)
            .map_err(|_| AiDescribeError::InvalidInput("Invalid extension id".into()))?;
        let team_oid = ObjectId::parse_str(team_id)
            .map_err(|_| AiDescribeError::InvalidInput("Invalid team_id".into()))?;

        let coll = self.db.collection::<mongodb::bson::Document>("extensions");
        let ext_doc = coll
            .find_one(
                doc! { "_id": ext_oid, "team_id": team_oid, "is_deleted": false },
                None,
            )
            .await?
            .ok_or(AiDescribeError::NotFound("Extension"))?;

        // Check cache
        if let (Ok(cached), Ok(cached_lang)) = (
            ext_doc.get_str("ai_description"),
            ext_doc.get_str("ai_description_lang"),
        ) {
            if cached_lang == lang {
                return Ok(DescribeResponse {
                    description: cached.to_string(),
                    lang: lang.to_string(),
                    generated_at: Self::parse_described_at(&ext_doc),
                });
            }
        }

        // Concurrency guard: prevent duplicate LLM calls for the same resource
        let flight_key = format!("ext:{}:{}", ext_id, lang);
        if !self.try_acquire(&flight_key) {
            return Err(AiDescribeError::LlmError(
                "A describe request for this extension is already in progress".into(),
            ));
        }

        let result = self
            .do_describe_ext(&ext_doc, team_id, ext_oid, lang, &coll)
            .await;
        self.release(&flight_key);
        result
    }

    /// Internal: perform LLM call and save for extension (called under concurrency guard)
    async fn do_describe_ext(
        &self,
        ext_doc: &mongodb::bson::Document,
        team_id: &str,
        ext_oid: ObjectId,
        lang: &str,
        coll: &mongodb::Collection<mongodb::bson::Document>,
    ) -> Result<DescribeResponse, AiDescribeError> {
        let name = ext_doc.get_str("name").unwrap_or("Unknown");
        let ext_type = ext_doc.get_str("extension_type").unwrap_or("unknown");
        let config_doc = ext_doc
            .get_document("config")
            .map(|d| serde_json::to_string_pretty(d).unwrap_or_default())
            .unwrap_or_default();
        let tags: Vec<&str> = ext_doc
            .get_array("tags")
            .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect())
            .unwrap_or_default();

        let desc = ext_doc.get_str("description").unwrap_or("");

        let user_prompt = format!(
            "Extension name: {}\nDescription: {}\nExtension type: {}\nConfig:\n{}\nTags: {}",
            name,
            desc,
            ext_type,
            Self::truncate_content(&config_doc, 4000),
            tags.join(", ")
        );

        let provider = self.get_provider(team_id).await?;
        let description = self
            .call_llm(
                provider.as_ref(),
                Self::extension_system_prompt(lang),
                &user_prompt,
            )
            .await?;

        let now = Utc::now();
        coll.update_one(
            doc! { "_id": ext_oid },
            doc! { "$set": {
                "ai_description": &description,
                "ai_description_lang": lang,
                "ai_described_at": now.to_rfc3339(),
            }},
            None,
        )
        .await?;

        Ok(DescribeResponse {
            description,
            lang: lang.to_string(),
            generated_at: now,
        })
    }

    /// Generate or return cached AI description for a skill
    pub async fn describe_skill(
        &self,
        team_id: &str,
        skill_id: &str,
        lang: &str,
    ) -> Result<DescribeResponse, AiDescribeError> {
        Self::validate_lang(lang)?;
        let skill_oid = ObjectId::parse_str(skill_id)
            .map_err(|_| AiDescribeError::InvalidInput("Invalid skill id".into()))?;
        let team_oid = ObjectId::parse_str(team_id)
            .map_err(|_| AiDescribeError::InvalidInput("Invalid team_id".into()))?;

        let coll = self.db.collection::<mongodb::bson::Document>("skills");
        let skill_doc = coll
            .find_one(
                doc! { "_id": skill_oid, "team_id": team_oid, "is_deleted": false },
                None,
            )
            .await?
            .ok_or(AiDescribeError::NotFound("Skill"))?;

        // Check cache
        if let (Ok(cached), Ok(cached_lang)) = (
            skill_doc.get_str("ai_description"),
            skill_doc.get_str("ai_description_lang"),
        ) {
            if cached_lang == lang {
                return Ok(DescribeResponse {
                    description: cached.to_string(),
                    lang: lang.to_string(),
                    generated_at: Self::parse_described_at(&skill_doc),
                });
            }
        }

        // Concurrency guard: prevent duplicate LLM calls for the same resource
        let flight_key = format!("skill:{}:{}", skill_id, lang);
        if !self.try_acquire(&flight_key) {
            return Err(AiDescribeError::LlmError(
                "A describe request for this skill is already in progress".into(),
            ));
        }

        let result = self
            .do_describe_skill(&skill_doc, team_id, skill_oid, lang, &coll)
            .await;
        self.release(&flight_key);
        result
    }

    /// Internal: perform LLM call and save for skill (called under concurrency guard)
    async fn do_describe_skill(
        &self,
        skill_doc: &mongodb::bson::Document,
        team_id: &str,
        skill_oid: ObjectId,
        lang: &str,
        coll: &mongodb::Collection<mongodb::bson::Document>,
    ) -> Result<DescribeResponse, AiDescribeError> {
        let name = skill_doc.get_str("name").unwrap_or("Unknown");
        let desc = skill_doc.get_str("description").unwrap_or("");
        let content = skill_doc.get_str("content").unwrap_or("");
        let skill_md = skill_doc.get_str("skill_md").unwrap_or("");
        let main_content = if !content.is_empty() {
            content
        } else {
            skill_md
        };
        let files: Vec<&str> = skill_doc
            .get_array("files")
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_document()?.get_str("path").ok())
                    .collect()
            })
            .unwrap_or_default();

        let user_prompt = format!(
            "Skill name: {}\nDescription: {}\nContent:\n{}\nFiles: {}",
            name,
            desc,
            Self::truncate_content(main_content, 4000),
            files.join(", ")
        );

        let provider = self.get_provider(team_id).await?;
        let description = self
            .call_llm(
                provider.as_ref(),
                Self::skill_system_prompt(lang),
                &user_prompt,
            )
            .await?;

        let now = Utc::now();
        coll.update_one(
            doc! { "_id": skill_oid },
            doc! { "$set": {
                "ai_description": &description,
                "ai_description_lang": lang,
                "ai_described_at": now.to_rfc3339(),
            }},
            None,
        )
        .await?;

        Ok(DescribeResponse {
            description,
            lang: lang.to_string(),
            generated_at: now,
        })
    }

    /// Generate or return cached AI description for a built-in extension
    pub async fn describe_builtin_extension(
        &self,
        team_id: &str,
        req: &BuiltinDescribeRequest,
    ) -> Result<DescribeResponse, AiDescribeError> {
        let lang = &req.lang;
        Self::validate_lang(lang)?;

        // Validate against whitelist
        if !is_known_builtin(&req.id) {
            return Err(AiDescribeError::InvalidInput(format!(
                "Unknown builtin extension id: '{}'",
                req.id
            )));
        }

        let team_oid = ObjectId::parse_str(team_id)
            .map_err(|_| AiDescribeError::InvalidInput("Invalid team_id".into()))?;

        let coll = self
            .db
            .collection::<mongodb::bson::Document>("builtin_extension_descriptions");

        // Check cache
        let filter = doc! {
            "team_id": team_oid,
            "extension_id": &req.id,
            "ai_description_lang": lang,
        };
        if let Some(cached_doc) = coll.find_one(filter, None).await? {
            if let Ok(cached) = cached_doc.get_str("ai_description") {
                return Ok(DescribeResponse {
                    description: cached.to_string(),
                    lang: lang.to_string(),
                    generated_at: Self::parse_described_at(&cached_doc),
                });
            }
        }

        // Concurrency guard
        let flight_key = format!("builtin:{}:{}:{}", team_id, req.id, lang);
        if !self.try_acquire(&flight_key) {
            return Err(AiDescribeError::LlmError(
                "A describe request for this builtin extension is already in progress".into(),
            ));
        }

        let result = self
            .do_describe_builtin(team_id, team_oid, req, &coll)
            .await;
        self.release(&flight_key);
        result
    }

    /// Internal: perform LLM call and upsert for builtin extension
    async fn do_describe_builtin(
        &self,
        team_id: &str,
        team_oid: ObjectId,
        req: &BuiltinDescribeRequest,
        coll: &mongodb::Collection<mongodb::bson::Document>,
    ) -> Result<DescribeResponse, AiDescribeError> {
        let lang = &req.lang;

        let ext_category = if req.is_platform {
            "Platform"
        } else {
            "MCP Server"
        };
        let user_prompt = format!(
            "Built-in extension name: {}\nCategory: {}\nShort description: {}",
            req.name, ext_category, req.description
        );

        let provider = self.get_provider(team_id).await?;
        let description = self
            .call_llm(
                provider.as_ref(),
                Self::builtin_ext_system_prompt(lang),
                &user_prompt,
            )
            .await?;

        let now = Utc::now();

        // Upsert into builtin_extension_descriptions
        let filter = doc! {
            "team_id": team_oid,
            "extension_id": &req.id,
            "ai_description_lang": lang,
        };
        let update = doc! { "$set": {
            "team_id": team_oid,
            "extension_id": &req.id,
            "extension_name": &req.name,
            "ai_description": &description,
            "ai_description_lang": lang,
            "ai_described_at": now.to_rfc3339(),
        }};
        let opts = mongodb::options::UpdateOptions::builder()
            .upsert(true)
            .build();
        coll.update_one(filter, update, Some(opts)).await?;

        Ok(DescribeResponse {
            description,
            lang: lang.to_string(),
            generated_at: now,
        })
    }

    /// Batch describe all built-in extensions that are missing descriptions for the given language.
    /// Returns the count of newly generated descriptions.
    pub async fn describe_all_builtin_extensions(
        &self,
        team_id: &str,
        lang: &str,
    ) -> Result<Vec<DescribeResponse>, AiDescribeError> {
        Self::validate_lang(lang)?;

        let mut results = Vec::new();
        for &(id, name, description, is_platform) in KNOWN_BUILTINS {
            let req = BuiltinDescribeRequest {
                id: id.to_string(),
                name: name.to_string(),
                description: description.to_string(),
                is_platform,
                lang: lang.to_string(),
            };
            match self.describe_builtin_extension(team_id, &req).await {
                Ok(resp) => results.push(resp),
                Err(e) => {
                    tracing::warn!("Batch describe builtin '{}' failed: {}", id, e);
                }
            }
        }

        Ok(results)
    }

    /// Generate or return cached AI description for a built-in skill
    pub async fn describe_builtin_skill(
        &self,
        team_id: &str,
        req: &BuiltinDescribeRequest,
    ) -> Result<DescribeResponse, AiDescribeError> {
        let lang = &req.lang;
        Self::validate_lang(lang)?;

        if !is_known_builtin_skill(&req.id) {
            return Err(AiDescribeError::InvalidInput(format!(
                "Unknown builtin skill id: '{}'",
                req.id
            )));
        }

        let team_oid = ObjectId::parse_str(team_id)
            .map_err(|_| AiDescribeError::InvalidInput("Invalid team_id".into()))?;

        let coll = self
            .db
            .collection::<mongodb::bson::Document>("builtin_skill_descriptions");

        // Check cache
        let filter = doc! {
            "team_id": team_oid,
            "skill_id": &req.id,
            "ai_description_lang": lang,
        };
        if let Some(cached_doc) = coll.find_one(filter, None).await? {
            if let Ok(cached) = cached_doc.get_str("ai_description") {
                return Ok(DescribeResponse {
                    description: cached.to_string(),
                    lang: lang.to_string(),
                    generated_at: Self::parse_described_at(&cached_doc),
                });
            }
        }

        // Concurrency guard
        let flight_key = format!("builtin_skill:{}:{}:{}", team_id, req.id, lang);
        if !self.try_acquire(&flight_key) {
            return Err(AiDescribeError::LlmError(
                "A describe request for this builtin skill is already in progress".into(),
            ));
        }

        let result = self
            .do_describe_builtin_skill(team_id, team_oid, req, &coll)
            .await;
        self.release(&flight_key);
        result
    }

    /// Internal: perform LLM call and upsert for builtin skill
    async fn do_describe_builtin_skill(
        &self,
        team_id: &str,
        team_oid: ObjectId,
        req: &BuiltinDescribeRequest,
        coll: &mongodb::Collection<mongodb::bson::Document>,
    ) -> Result<DescribeResponse, AiDescribeError> {
        let lang = &req.lang;

        let user_prompt = format!(
            "Built-in skill name: {}\nShort description: {}",
            req.name, req.description
        );

        let provider = self.get_provider(team_id).await?;
        let description = self
            .call_llm(
                provider.as_ref(),
                Self::builtin_skill_system_prompt(lang),
                &user_prompt,
            )
            .await?;

        let now = Utc::now();

        let filter = doc! {
            "team_id": team_oid,
            "skill_id": &req.id,
            "ai_description_lang": lang,
        };
        let update = doc! { "$set": {
            "team_id": team_oid,
            "skill_id": &req.id,
            "skill_name": &req.name,
            "ai_description": &description,
            "ai_description_lang": lang,
            "ai_described_at": now.to_rfc3339(),
        }};
        let opts = mongodb::options::UpdateOptions::builder()
            .upsert(true)
            .build();
        coll.update_one(filter, update, Some(opts)).await?;

        Ok(DescribeResponse {
            description,
            lang: lang.to_string(),
            generated_at: now,
        })
    }

    /// Batch describe all built-in skills that are missing descriptions for the given language.
    pub async fn describe_all_builtin_skills(
        &self,
        team_id: &str,
        lang: &str,
    ) -> Result<Vec<DescribeResponse>, AiDescribeError> {
        Self::validate_lang(lang)?;

        let mut results = Vec::new();
        for &(id, name, description) in KNOWN_BUILTIN_SKILLS {
            let req = BuiltinDescribeRequest {
                id: id.to_string(),
                name: name.to_string(),
                description: description.to_string(),
                is_platform: false,
                lang: lang.to_string(),
            };
            match self.describe_builtin_skill(team_id, &req).await {
                Ok(resp) => results.push(resp),
                Err(e) => {
                    tracing::warn!("Batch describe builtin skill '{}' failed: {}", id, e);
                }
            }
        }

        Ok(results)
    }

    /// Batch describe all team skills that are missing descriptions for the given language.
    pub async fn describe_all_skills(
        &self,
        team_id: &str,
        lang: &str,
    ) -> Result<Vec<DescribeResponse>, AiDescribeError> {
        Self::validate_lang(lang)?;
        let team_oid = ObjectId::parse_str(team_id)
            .map_err(|_| AiDescribeError::InvalidInput("Invalid team_id".into()))?;

        let coll = self.db.collection::<mongodb::bson::Document>("skills");

        // Find skills without ai_description for this lang
        let filter = doc! {
            "team_id": team_oid,
            "is_deleted": false,
            "$or": [
                { "ai_description": { "$exists": false } },
                { "ai_description": null },
                { "ai_description_lang": { "$ne": lang } },
            ],
        };
        let mut cursor = coll.find(filter, None).await?;
        let mut skill_ids = Vec::new();
        while let Some(doc) = futures::TryStreamExt::try_next(&mut cursor).await? {
            if let Ok(oid) = doc.get_object_id("_id") {
                skill_ids.push(oid.to_hex());
            }
        }

        let mut results = Vec::new();
        for skill_id in skill_ids {
            match self.describe_skill(team_id, &skill_id, lang).await {
                Ok(resp) => results.push(resp),
                Err(e) => {
                    tracing::warn!("Batch describe skill '{}' failed: {}", skill_id, e);
                }
            }
        }

        Ok(results)
    }

    /// Batch describe all team extensions that are missing descriptions for the given language.
    pub async fn describe_all_extensions(
        &self,
        team_id: &str,
        lang: &str,
    ) -> Result<Vec<DescribeResponse>, AiDescribeError> {
        Self::validate_lang(lang)?;
        let team_oid = ObjectId::parse_str(team_id)
            .map_err(|_| AiDescribeError::InvalidInput("Invalid team_id".into()))?;

        let coll = self.db.collection::<mongodb::bson::Document>("extensions");

        let filter = doc! {
            "team_id": team_oid,
            "is_deleted": false,
            "$or": [
                { "ai_description": { "$exists": false } },
                { "ai_description": null },
                { "ai_description_lang": { "$ne": lang } },
            ],
        };
        let mut cursor = coll.find(filter, None).await?;
        let mut ext_ids = Vec::new();
        while let Some(doc) = futures::TryStreamExt::try_next(&mut cursor).await? {
            if let Ok(oid) = doc.get_object_id("_id") {
                ext_ids.push(oid.to_hex());
            }
        }

        let mut results = Vec::new();
        for ext_id in ext_ids {
            match self.describe_extension(team_id, &ext_id, lang).await {
                Ok(resp) => results.push(resp),
                Err(e) => {
                    tracing::warn!("Batch describe extension '{}' failed: {}", ext_id, e);
                }
            }
        }

        Ok(results)
    }

    async fn collect_insights(
        &self,
        collection: &str,
        filter: mongodb::bson::Document,
        item_type: &str,
        id_field: &str,
        name_field: &str,
        insights: &mut Vec<InsightItem>,
    ) -> Result<(), AiDescribeError> {
        let coll = self.db.collection::<mongodb::bson::Document>(collection);
        let mut cursor = coll.find(filter, None).await?;
        while let Some(doc) = futures::TryStreamExt::try_next(&mut cursor).await? {
            if let (Ok(desc), Ok(desc_lang)) = (
                doc.get_str("ai_description"),
                doc.get_str("ai_description_lang"),
            ) {
                let id = doc
                    .get_object_id("_id")
                    .map(|oid| oid.to_hex())
                    .ok()
                    .or_else(|| doc.get_str(id_field).ok().map(|s| s.to_string()))
                    .unwrap_or_default();
                let name = doc.get_str(name_field).unwrap_or("").to_string();
                insights.push(InsightItem {
                    id,
                    item_type: item_type.to_string(),
                    name,
                    ai_description: desc.to_string(),
                    ai_description_lang: desc_lang.to_string(),
                    ai_described_at: Self::parse_described_at(&doc),
                });
            }
        }
        Ok(())
    }

    /// Get all existing AI descriptions for a team (no new LLM calls)
    pub async fn get_team_insights(
        &self,
        team_id: &str,
        lang: &str,
    ) -> Result<InsightsResponse, AiDescribeError> {
        Self::validate_lang(lang)?;
        let team_oid = ObjectId::parse_str(team_id)
            .map_err(|_| AiDescribeError::InvalidInput("Invalid team_id".into()))?;

        let mut insights = Vec::new();

        let team_filter = doc! {
            "team_id": team_oid,
            "is_deleted": false,
            "ai_description": { "$exists": true, "$ne": null },
            "ai_description_lang": lang,
        };
        let builtin_filter = doc! {
            "team_id": team_oid,
            "ai_description": { "$exists": true, "$ne": null },
            "ai_description_lang": lang,
        };

        self.collect_insights(
            "extensions",
            team_filter.clone(),
            "extension",
            "_id",
            "name",
            &mut insights,
        )
        .await?;
        self.collect_insights("skills", team_filter, "skill", "_id", "name", &mut insights)
            .await?;
        self.collect_insights(
            "builtin_extension_descriptions",
            builtin_filter.clone(),
            "builtin_extension",
            "extension_id",
            "extension_name",
            &mut insights,
        )
        .await?;
        self.collect_insights(
            "builtin_skill_descriptions",
            builtin_filter,
            "builtin_skill",
            "skill_id",
            "skill_name",
            &mut insights,
        )
        .await?;

        let total = insights.len();
        Ok(InsightsResponse { insights, total })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agime_team::models::BuiltinExtension;
    use std::collections::HashSet;

    #[test]
    fn known_builtins_matches_enum() {
        let enum_names: HashSet<&str> = BuiltinExtension::all().iter().map(|e| e.name()).collect();
        let known_names: HashSet<&str> = KNOWN_BUILTINS.iter().map(|(id, _, _, _)| *id).collect();
        assert_eq!(
            enum_names,
            known_names,
            "KNOWN_BUILTINS is out of sync with BuiltinExtension enum. \
             Missing from KNOWN_BUILTINS: {:?}, Extra in KNOWN_BUILTINS: {:?}",
            enum_names.difference(&known_names).collect::<Vec<_>>(),
            known_names.difference(&enum_names).collect::<Vec<_>>(),
        );
    }
}
