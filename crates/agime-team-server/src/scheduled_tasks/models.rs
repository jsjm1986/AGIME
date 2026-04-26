use chrono::{DateTime, Utc};
use mongodb::bson::oid::ObjectId;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ScheduledTaskKind {
    OneShot,
    Cron,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ScheduledTaskStatus {
    Draft,
    Active,
    Paused,
    Completed,
    Deleted,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ScheduledTaskRunStatus {
    Running,
    Completed,
    Failed,
    Cancelled,
    WaitingInput,
    WaitingApproval,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ScheduledTaskDeliveryTier {
    Durable,
    SessionScoped,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ScheduledTaskListView {
    Mine,
    AllVisible,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ScheduledTaskScheduleMode {
    EveryMinutes,
    EveryHours,
    DailyAt,
    WeekdaysAt,
    WeeklyOn,
    Custom,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ScheduledTaskProfile {
    DocumentTask,
    WorkspaceTask,
    HybridTask,
    RetrievalTask,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ScheduledTaskOutputMode {
    SummaryOnly,
    SummaryAndArtifact,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ScheduledTaskPublishBehavior {
    None,
    PublishWorkspaceArtifact,
    CreateDocumentFromFile,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ScheduledTaskSourceScope {
    TeamDocuments,
    WorkspaceOnly,
    Mixed,
    ExternalRetrieval,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ScheduledTaskSourcePolicy {
    OfficialFirst,
    DomesticPreferred,
    GlobalPreferred,
    Mixed,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ScheduledTaskRunOutcomeReason {
    Completed,
    CompletedWithWarnings,
    FailedNoFinalAnswer,
    FailedContractViolation,
    BlockedCapabilityPolicy,
    Cancelled,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ScheduledTaskSelfEvaluationGrade {
    Excellent,
    Good,
    Acceptable,
    Weak,
    Failed,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ScheduledTaskScheduleSpecKind {
    OneShot,
    Every,
    Calendar,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ScheduledTaskPayloadKind {
    SystemSummary,
    ArtifactTask,
    DocumentPipeline,
    RetrievalPipeline,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ScheduledTaskSessionBinding {
    IsolatedTask,
    BoundSession,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ScheduledTaskDeliveryPlanKind {
    ChannelOnly,
    ChannelAndArtifact,
    ChannelAndPublish,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ScheduledTaskScheduleConfig {
    pub mode: ScheduledTaskScheduleMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub every_minutes: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub every_hours: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub daily_time: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub weekly_days: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cron_expression: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ScheduledTaskExecutionContract {
    pub output_mode: ScheduledTaskOutputMode,
    #[serde(default = "default_true")]
    pub must_return_final_text: bool,
    #[serde(default = "default_true")]
    pub allow_partial_result: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact_path: Option<String>,
    #[serde(default = "default_publish_behavior")]
    pub publish_behavior: ScheduledTaskPublishBehavior,
    pub source_scope: ScheduledTaskSourceScope,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_policy: Option<ScheduledTaskSourcePolicy>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minimum_source_attempts: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minimum_successful_sources: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prefer_structured_sources: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow_query_retry: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fallback_to_secondary_sources: Option<bool>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub required_sections: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ScheduledTaskScheduleSpec {
    pub kind: ScheduledTaskScheduleSpecKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub one_shot_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schedule_config: Option<ScheduledTaskScheduleConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cron_expression: Option<String>,
    pub timezone: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScheduledTaskDoc {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub task_id: String,
    pub team_id: String,
    pub channel_id: String,
    pub owner_user_id: String,
    pub agent_id: String,
    pub title: String,
    pub prompt: String,
    pub task_kind: ScheduledTaskKind,
    #[serde(default = "default_task_profile")]
    pub task_profile: ScheduledTaskProfile,
    #[serde(default = "default_payload_kind")]
    pub payload_kind: ScheduledTaskPayloadKind,
    #[serde(default = "default_session_binding")]
    pub session_binding: ScheduledTaskSessionBinding,
    #[serde(default = "default_delivery_plan_kind")]
    pub delivery_plan: ScheduledTaskDeliveryPlanKind,
    #[serde(default = "default_execution_contract")]
    pub execution_contract: ScheduledTaskExecutionContract,
    #[serde(default = "default_delivery_tier")]
    pub delivery_tier: ScheduledTaskDeliveryTier,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub one_shot_at: Option<bson::DateTime>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cron_expression: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schedule_config: Option<ScheduledTaskScheduleConfig>,
    pub timezone: String,
    pub status: ScheduledTaskStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_fire_at: Option<bson::DateTime>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_fire_at: Option<bson::DateTime>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_run_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_expected_fire_at: Option<bson::DateTime>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_missed_at: Option<bson::DateTime>,
    #[serde(default)]
    pub missed_fire_count: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lease_owner: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lease_expires_at: Option<bson::DateTime>,
    pub created_at: bson::DateTime,
    pub updated_at: bson::DateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScheduledTaskRunDoc {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub run_id: String,
    pub task_id: String,
    pub team_id: String,
    pub channel_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fire_message_id: Option<String>,
    pub status: ScheduledTaskRunStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome_reason: Option<ScheduledTaskRunOutcomeReason>,
    #[serde(default)]
    pub warning_count: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub self_evaluation: Option<ScheduledTaskSelfEvaluation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub initial_self_evaluation: Option<ScheduledTaskSelfEvaluation>,
    #[serde(default)]
    pub improvement_loop_applied: bool,
    #[serde(default)]
    pub improvement_loop_count: i32,
    pub trigger_source: String,
    pub started_at: bson::DateTime,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<bson::DateTime>,
    pub created_at: bson::DateTime,
    pub updated_at: bson::DateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScheduledTaskSelfEvaluation {
    pub score: i32,
    pub grade: ScheduledTaskSelfEvaluationGrade,
    pub goal_completion: i32,
    pub result_quality: i32,
    pub evidence_quality: i32,
    pub execution_stability: i32,
    pub contract_compliance: i32,
    pub summary: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub completed_steps: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub failed_steps: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub risks: Vec<String>,
    pub confidence: f32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CreateScheduledTaskRequest {
    pub agent_id: String,
    pub title: String,
    pub prompt: String,
    pub task_kind: ScheduledTaskKind,
    #[serde(default, with = "bson_datetime_option")]
    pub one_shot_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub cron_expression: Option<String>,
    #[serde(default)]
    pub timezone: Option<String>,
    #[serde(default)]
    pub delivery_tier: Option<ScheduledTaskDeliveryTier>,
    #[serde(default)]
    pub owner_session_id: Option<String>,
    #[serde(default)]
    pub schedule_config: Option<ScheduledTaskScheduleConfig>,
    #[serde(default)]
    pub task_profile: Option<ScheduledTaskProfile>,
    #[serde(default)]
    pub payload_kind: Option<ScheduledTaskPayloadKind>,
    #[serde(default)]
    pub session_binding: Option<ScheduledTaskSessionBinding>,
    #[serde(default)]
    pub delivery_plan: Option<ScheduledTaskDeliveryPlanKind>,
    #[serde(default)]
    pub execution_contract: Option<ScheduledTaskExecutionContract>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct UpdateScheduledTaskRequest {
    #[serde(default)]
    pub agent_id: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub prompt: Option<String>,
    #[serde(default)]
    pub task_kind: Option<ScheduledTaskKind>,
    #[serde(default, with = "bson_datetime_option")]
    pub one_shot_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub cron_expression: Option<Option<String>>,
    #[serde(default)]
    pub timezone: Option<String>,
    #[serde(default)]
    pub delivery_tier: Option<ScheduledTaskDeliveryTier>,
    #[serde(default)]
    pub owner_session_id: Option<Option<String>>,
    #[serde(default)]
    pub schedule_config: Option<ScheduledTaskScheduleConfig>,
    #[serde(default)]
    pub task_profile: Option<ScheduledTaskProfile>,
    #[serde(default)]
    pub payload_kind: Option<ScheduledTaskPayloadKind>,
    #[serde(default)]
    pub session_binding: Option<ScheduledTaskSessionBinding>,
    #[serde(default)]
    pub delivery_plan: Option<ScheduledTaskDeliveryPlanKind>,
    #[serde(default)]
    pub execution_contract: Option<ScheduledTaskExecutionContract>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ParseScheduledTaskRequest {
    pub text: String,
    #[serde(default)]
    pub timezone: Option<String>,
    #[serde(default)]
    pub agent_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScheduledTaskParseResult {
    pub title: String,
    pub prompt: String,
    pub task_kind: ScheduledTaskKind,
    pub task_profile: ScheduledTaskProfile,
    pub payload_kind: ScheduledTaskPayloadKind,
    pub session_binding: ScheduledTaskSessionBinding,
    pub delivery_plan: ScheduledTaskDeliveryPlanKind,
    pub schedule_spec: ScheduledTaskScheduleSpec,
    pub execution_contract: ScheduledTaskExecutionContract,
    pub human_schedule: String,
    #[serde(default)]
    pub warnings: Vec<String>,
    #[serde(default)]
    pub advanced_mode: bool,
    pub confidence: f32,
    pub ready_to_create: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ScheduledTaskParseResponse {
    pub preview: ScheduledTaskParseResult,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct ScheduledTaskParseOverrides {
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub prompt: Option<String>,
    #[serde(default)]
    pub agent_id: Option<String>,
    #[serde(default)]
    pub timezone: Option<String>,
    #[serde(default)]
    pub delivery_tier: Option<ScheduledTaskDeliveryTier>,
    #[serde(default, with = "bson_datetime_option")]
    pub one_shot_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub schedule_config: Option<ScheduledTaskScheduleConfig>,
    #[serde(default)]
    pub artifact_path: Option<String>,
    #[serde(default)]
    pub publish_behavior: Option<ScheduledTaskPublishBehavior>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CreateScheduledTaskFromParseRequest {
    pub preview: ScheduledTaskParseResult,
    #[serde(default)]
    pub overrides: ScheduledTaskParseOverrides,
}

#[derive(Debug, Clone, Serialize)]
pub struct ScheduledTaskSummaryResponse {
    pub task_id: String,
    pub team_id: String,
    pub channel_id: String,
    pub owner_user_id: String,
    pub agent_id: String,
    pub title: String,
    pub task_kind: ScheduledTaskKind,
    pub task_profile: ScheduledTaskProfile,
    pub payload_kind: ScheduledTaskPayloadKind,
    pub session_binding: ScheduledTaskSessionBinding,
    pub delivery_plan: ScheduledTaskDeliveryPlanKind,
    pub execution_contract: ScheduledTaskExecutionContract,
    pub delivery_tier: ScheduledTaskDeliveryTier,
    pub timezone: String,
    pub status: ScheduledTaskStatus,
    pub human_schedule: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schedule_config: Option<ScheduledTaskScheduleConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub one_shot_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cron_expression: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_fire_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_fire_preview: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_fire_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_run_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_expected_fire_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_missed_at: Option<String>,
    #[serde(default)]
    pub missed_fire_count: i64,
    #[serde(default)]
    pub can_resume: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resume_hint: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ScheduledTaskDetailResponse {
    #[serde(flatten)]
    pub summary: ScheduledTaskSummaryResponse,
    pub prompt: String,
    #[serde(default)]
    pub runs: Vec<ScheduledTaskRunResponse>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ScheduledTaskRunResponse {
    pub run_id: String,
    pub task_id: String,
    pub channel_id: String,
    pub status: ScheduledTaskRunStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome_reason: Option<ScheduledTaskRunOutcomeReason>,
    #[serde(default)]
    pub warning_count: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fire_message_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub self_evaluation: Option<ScheduledTaskSelfEvaluation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub initial_self_evaluation: Option<ScheduledTaskSelfEvaluation>,
    #[serde(default)]
    pub improvement_loop_applied: bool,
    #[serde(default)]
    pub improvement_loop_count: i32,
    pub trigger_source: String,
    pub started_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ScheduledTaskRepairReason {
    MissingChannel,
}

#[derive(Debug, Clone, Serialize)]
pub struct ScheduledTaskRepairEventResponse {
    pub task_id: String,
    pub channel_id: String,
    pub title: String,
    pub owner_user_id: String,
    pub delivery_tier: ScheduledTaskDeliveryTier,
    pub reason: ScheduledTaskRepairReason,
    pub user_message: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ScheduledTaskListResponse {
    pub tasks: Vec<ScheduledTaskSummaryResponse>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub repair_events: Vec<ScheduledTaskRepairEventResponse>,
}

fn default_delivery_tier() -> ScheduledTaskDeliveryTier {
    ScheduledTaskDeliveryTier::Durable
}

fn default_true() -> bool {
    true
}

fn default_publish_behavior() -> ScheduledTaskPublishBehavior {
    ScheduledTaskPublishBehavior::None
}

fn default_task_profile() -> ScheduledTaskProfile {
    ScheduledTaskProfile::WorkspaceTask
}

fn default_payload_kind() -> ScheduledTaskPayloadKind {
    ScheduledTaskPayloadKind::SystemSummary
}

fn default_session_binding() -> ScheduledTaskSessionBinding {
    ScheduledTaskSessionBinding::IsolatedTask
}

fn default_delivery_plan_kind() -> ScheduledTaskDeliveryPlanKind {
    ScheduledTaskDeliveryPlanKind::ChannelOnly
}

fn default_execution_contract() -> ScheduledTaskExecutionContract {
    ScheduledTaskExecutionContract {
        output_mode: ScheduledTaskOutputMode::SummaryOnly,
        must_return_final_text: true,
        allow_partial_result: true,
        artifact_path: None,
        publish_behavior: ScheduledTaskPublishBehavior::None,
        source_scope: ScheduledTaskSourceScope::WorkspaceOnly,
        source_policy: None,
        minimum_source_attempts: None,
        minimum_successful_sources: None,
        prefer_structured_sources: None,
        allow_query_retry: None,
        fallback_to_secondary_sources: None,
        required_sections: Vec::new(),
    }
}

fn default_schedule_mode() -> ScheduledTaskScheduleMode {
    ScheduledTaskScheduleMode::Custom
}

fn pad_time_part(value: u32) -> String {
    format!("{value:02}")
}

fn normalize_weekly_days(days: &[String]) -> Vec<String> {
    let allowed = ["1", "2", "3", "4", "5", "6", "0"];
    let mut result = Vec::new();
    for key in allowed {
        if days.iter().any(|item| item == key) {
            result.push(key.to_string());
        }
    }
    if result.is_empty() {
        vec!["1".to_string()]
    } else {
        result
    }
}

fn contains_any(text: &str, patterns: &[&str]) -> bool {
    patterns.iter().any(|pattern| text.contains(pattern))
}

fn looks_like_document_task(prompt: &str) -> bool {
    contains_any(
        prompt,
        &[
            "document",
            "documents",
            "doc_id",
            "doc_ref",
            "knowledge base",
            "knowledge-base",
            "team document",
            "team documents",
            "document change",
            "document changes",
            "document update",
            "document updates",
            "文档",
            "团队文档",
            "文档变化",
            "文档变更",
            "文档更新",
            "知识库",
            "AI工作台",
            "工作台文档",
        ],
    )
}

fn looks_like_workspace_artifact_task(prompt: &str) -> bool {
    contains_any(
        prompt,
        &[
            "workspace",
            "workdir",
            "markdown",
            " md ",
            "md 报告",
            "md总结",
            "md 总结",
            "md 文件",
            ".md",
            "save to workspace",
            "write to workspace",
            "report",
            "digest",
            "summary file",
            "保存到工作区",
            "写入工作区",
            "生成报告",
            "汇总做一个md",
            "保存成",
        ],
    )
}

fn looks_like_publish_back_task(prompt: &str) -> bool {
    contains_any(
        prompt,
        &[
            "publish workspace artifact",
            "create document from file",
            "publish back",
            "正式文档区",
            "发布到文档",
            "回写文档",
            "归档为文档",
        ],
    )
}

fn looks_like_retrieval_task(prompt: &str) -> bool {
    contains_any(
        prompt,
        &[
            "internet",
            "latest ai",
            "latest news",
            "web search",
            "search online",
            "search the internet",
            "release note",
            "release notes",
            "official announcement",
            "official blog",
            "openai",
            "anthropic",
            "google deepmind",
            "互联网",
            "最新",
            "资讯",
            "新闻",
            "搜索互联网",
            "联网搜索",
            "官方公告",
            "官方博客",
            "官方发布",
            "发布说明",
            "更新日志",
        ],
    )
}

fn infer_source_policy_from_prompt(
    prompt: &str,
    profile: ScheduledTaskProfile,
) -> Option<ScheduledTaskSourcePolicy> {
    if !matches!(profile, ScheduledTaskProfile::RetrievalTask) {
        return None;
    }

    let normalized = prompt.trim().to_ascii_lowercase();
    let domestic_markers = [
        "国内",
        "中国",
        "本土",
        "a股",
        "港股",
        "国务院",
        "工信部",
        "国家",
        "网信办",
        "新华社",
        "人民日报",
        "腾讯",
        "阿里",
        "百度",
        "字节",
    ];
    if contains_any(prompt, &domestic_markers) || contains_any(&normalized, &domestic_markers) {
        return Some(ScheduledTaskSourcePolicy::DomesticPreferred);
    }

    let global_markers = [
        "全球",
        "国际",
        "海外",
        "world",
        "global",
        "international",
        "silicon valley",
        "openai",
        "anthropic",
        "google deepmind",
    ];
    if contains_any(prompt, &global_markers) || contains_any(&normalized, &global_markers) {
        return Some(ScheduledTaskSourcePolicy::GlobalPreferred);
    }

    let official_markers = [
        "官方",
        "公告",
        "博客",
        "文档",
        "release note",
        "release notes",
        "changelog",
        "developer blog",
    ];
    if contains_any(prompt, &official_markers) || contains_any(&normalized, &official_markers) {
        return Some(ScheduledTaskSourcePolicy::OfficialFirst);
    }

    Some(ScheduledTaskSourcePolicy::Mixed)
}

fn default_artifact_path(
    task_id: &str,
    output_mode: ScheduledTaskOutputMode,
    prompt: &str,
) -> Option<String> {
    if !matches!(output_mode, ScheduledTaskOutputMode::SummaryAndArtifact) {
        return None;
    }
    if prompt.contains(".md")
        || prompt.contains("markdown")
        || prompt.contains("Markdown")
        || prompt.contains("md保存")
        || prompt.contains("md 报告")
        || prompt.contains("md总结")
        || prompt.contains("md 总结")
        || prompt.contains("md 文件")
        || prompt.contains(" md ")
    {
        Some(format!("artifacts/scheduled-task-{task_id}.md"))
    } else {
        Some(format!("artifacts/scheduled-task-{task_id}.txt"))
    }
}

pub fn infer_task_profile_from_prompt(prompt: &str) -> ScheduledTaskProfile {
    let normalized = prompt.trim().to_ascii_lowercase();
    let has_document = looks_like_document_task(&normalized) || looks_like_document_task(prompt);
    let has_workspace = looks_like_workspace_artifact_task(&normalized)
        || looks_like_workspace_artifact_task(prompt);
    let has_publish =
        looks_like_publish_back_task(&normalized) || looks_like_publish_back_task(prompt);
    let has_retrieval = looks_like_retrieval_task(&normalized) || looks_like_retrieval_task(prompt);

    if has_document && (has_workspace || has_publish) {
        ScheduledTaskProfile::HybridTask
    } else if has_document {
        ScheduledTaskProfile::DocumentTask
    } else if has_retrieval {
        ScheduledTaskProfile::RetrievalTask
    } else {
        ScheduledTaskProfile::WorkspaceTask
    }
}

pub fn infer_execution_contract(
    task_id: &str,
    prompt: &str,
    profile: ScheduledTaskProfile,
) -> ScheduledTaskExecutionContract {
    let output_mode = if looks_like_workspace_artifact_task(&prompt.to_ascii_lowercase())
        || looks_like_workspace_artifact_task(prompt)
    {
        ScheduledTaskOutputMode::SummaryAndArtifact
    } else {
        ScheduledTaskOutputMode::SummaryOnly
    };
    let publish_behavior = if looks_like_publish_back_task(&prompt.to_ascii_lowercase())
        || looks_like_publish_back_task(prompt)
    {
        if prompt
            .to_ascii_lowercase()
            .contains("create document from file")
            || prompt.contains("创建文档")
        {
            ScheduledTaskPublishBehavior::CreateDocumentFromFile
        } else {
            ScheduledTaskPublishBehavior::PublishWorkspaceArtifact
        }
    } else {
        ScheduledTaskPublishBehavior::None
    };
    let source_scope = match profile {
        ScheduledTaskProfile::DocumentTask => ScheduledTaskSourceScope::TeamDocuments,
        ScheduledTaskProfile::WorkspaceTask => ScheduledTaskSourceScope::WorkspaceOnly,
        ScheduledTaskProfile::HybridTask => ScheduledTaskSourceScope::Mixed,
        ScheduledTaskProfile::RetrievalTask => ScheduledTaskSourceScope::ExternalRetrieval,
    };

    ScheduledTaskExecutionContract {
        output_mode,
        must_return_final_text: true,
        allow_partial_result: true,
        artifact_path: default_artifact_path(task_id, output_mode, prompt),
        publish_behavior,
        source_scope,
        source_policy: infer_source_policy_from_prompt(prompt, profile),
        minimum_source_attempts: matches!(profile, ScheduledTaskProfile::RetrievalTask)
            .then_some(3),
        minimum_successful_sources: matches!(profile, ScheduledTaskProfile::RetrievalTask)
            .then_some(2),
        prefer_structured_sources: matches!(profile, ScheduledTaskProfile::RetrievalTask)
            .then_some(true),
        allow_query_retry: matches!(profile, ScheduledTaskProfile::RetrievalTask).then_some(true),
        fallback_to_secondary_sources: matches!(profile, ScheduledTaskProfile::RetrievalTask)
            .then_some(true),
        required_sections: Vec::new(),
    }
}

pub fn infer_payload_kind(
    profile: ScheduledTaskProfile,
    contract: &ScheduledTaskExecutionContract,
) -> ScheduledTaskPayloadKind {
    match profile {
        ScheduledTaskProfile::DocumentTask | ScheduledTaskProfile::HybridTask => {
            ScheduledTaskPayloadKind::DocumentPipeline
        }
        ScheduledTaskProfile::RetrievalTask => ScheduledTaskPayloadKind::RetrievalPipeline,
        ScheduledTaskProfile::WorkspaceTask => {
            if matches!(
                contract.output_mode,
                ScheduledTaskOutputMode::SummaryAndArtifact
            ) {
                ScheduledTaskPayloadKind::ArtifactTask
            } else {
                ScheduledTaskPayloadKind::SystemSummary
            }
        }
    }
}

pub fn infer_session_binding(
    prompt: &str,
    delivery_tier: ScheduledTaskDeliveryTier,
) -> ScheduledTaskSessionBinding {
    if matches!(delivery_tier, ScheduledTaskDeliveryTier::SessionScoped)
        || contains_any(
            prompt,
            &[
                "当前对话",
                "当前会话",
                "本次会话",
                "当前聊天",
                "this chat",
                "current session",
            ],
        )
    {
        ScheduledTaskSessionBinding::BoundSession
    } else {
        ScheduledTaskSessionBinding::IsolatedTask
    }
}

pub fn infer_delivery_plan(
    contract: &ScheduledTaskExecutionContract,
) -> ScheduledTaskDeliveryPlanKind {
    match contract.publish_behavior {
        ScheduledTaskPublishBehavior::PublishWorkspaceArtifact
        | ScheduledTaskPublishBehavior::CreateDocumentFromFile => {
            ScheduledTaskDeliveryPlanKind::ChannelAndPublish
        }
        ScheduledTaskPublishBehavior::None => {
            if matches!(
                contract.output_mode,
                ScheduledTaskOutputMode::SummaryAndArtifact
            ) {
                ScheduledTaskDeliveryPlanKind::ChannelAndArtifact
            } else {
                ScheduledTaskDeliveryPlanKind::ChannelOnly
            }
        }
    }
}

pub fn normalize_execution_contract(
    task_id: &str,
    prompt: &str,
    profile: ScheduledTaskProfile,
    contract: Option<ScheduledTaskExecutionContract>,
) -> ScheduledTaskExecutionContract {
    let inferred = infer_execution_contract(task_id, prompt, profile);
    let mut contract = contract.unwrap_or_else(|| inferred.clone());
    if contract
        .artifact_path
        .as_deref()
        .is_none_or(|value| value.trim().is_empty())
    {
        contract.artifact_path = inferred.artifact_path;
    }
    if matches!(
        contract.source_scope,
        ScheduledTaskSourceScope::WorkspaceOnly
    ) && !matches!(
        inferred.source_scope,
        ScheduledTaskSourceScope::WorkspaceOnly
    ) {
        contract.source_scope = inferred.source_scope;
    }
    if matches!(
        contract.publish_behavior,
        ScheduledTaskPublishBehavior::None
    ) && !matches!(
        inferred.publish_behavior,
        ScheduledTaskPublishBehavior::None
    ) {
        contract.publish_behavior = inferred.publish_behavior;
    }
    if contract.source_policy.is_none() && inferred.source_policy.is_some() {
        contract.source_policy = inferred.source_policy;
    }
    if contract.minimum_source_attempts.is_none() {
        contract.minimum_source_attempts = inferred.minimum_source_attempts;
    }
    if contract.minimum_successful_sources.is_none() {
        contract.minimum_successful_sources = inferred.minimum_successful_sources;
    }
    if contract.prefer_structured_sources.is_none() {
        contract.prefer_structured_sources = inferred.prefer_structured_sources;
    }
    if contract.allow_query_retry.is_none() {
        contract.allow_query_retry = inferred.allow_query_retry;
    }
    if contract.fallback_to_secondary_sources.is_none() {
        contract.fallback_to_secondary_sources = inferred.fallback_to_secondary_sources;
    }
    if matches!(contract.output_mode, ScheduledTaskOutputMode::SummaryOnly)
        && matches!(
            inferred.output_mode,
            ScheduledTaskOutputMode::SummaryAndArtifact
        )
    {
        contract.output_mode = inferred.output_mode;
    }
    contract
}

pub fn reconcile_task_contract(doc: &mut ScheduledTaskDoc) {
    let inferred_profile = infer_task_profile_from_prompt(&doc.prompt);
    if matches!(doc.task_profile, ScheduledTaskProfile::WorkspaceTask)
        && looks_like_document_task(&doc.prompt)
    {
        doc.task_profile = inferred_profile;
    }
    doc.execution_contract = normalize_execution_contract(
        &doc.task_id,
        &doc.prompt,
        doc.task_profile,
        Some(doc.execution_contract.clone()),
    );
    doc.payload_kind = infer_payload_kind(doc.task_profile, &doc.execution_contract);
    doc.session_binding = infer_session_binding(&doc.prompt, doc.delivery_tier);
    doc.delivery_plan = infer_delivery_plan(&doc.execution_contract);
}

fn weekday_labels(days: &[String]) -> String {
    let ordered = normalize_weekly_days(days);
    ordered
        .into_iter()
        .map(|value| match value.as_str() {
            "1" => "周一".to_string(),
            "2" => "周二".to_string(),
            "3" => "周三".to_string(),
            "4" => "周四".to_string(),
            "5" => "周五".to_string(),
            "6" => "周六".to_string(),
            "0" => "周日".to_string(),
            _ => value,
        })
        .collect::<Vec<_>>()
        .join("、")
}

fn parse_day_of_week_field(value: &str) -> Option<(ScheduledTaskScheduleMode, Vec<String>)> {
    let normalized = value.trim();
    if normalized == "1-5" {
        return Some((
            ScheduledTaskScheduleMode::WeekdaysAt,
            vec![
                "1".to_string(),
                "2".to_string(),
                "3".to_string(),
                "4".to_string(),
                "5".to_string(),
            ],
        ));
    }
    if normalized
        .split(',')
        .all(|item| matches!(item, "0" | "1" | "2" | "3" | "4" | "5" | "6"))
    {
        return Some((
            ScheduledTaskScheduleMode::WeeklyOn,
            normalize_weekly_days(
                &normalized
                    .split(',')
                    .map(|item| item.to_string())
                    .collect::<Vec<_>>(),
            ),
        ));
    }
    None
}

pub fn schedule_config_for_task(doc: &ScheduledTaskDoc) -> Option<ScheduledTaskScheduleConfig> {
    if let Some(schedule_config) = doc.schedule_config.as_ref() {
        return Some(schedule_config.clone());
    }
    if !matches!(doc.task_kind, ScheduledTaskKind::Cron) {
        return None;
    }
    let cron_expression = doc.cron_expression.as_deref()?.trim();
    let parts = cron_expression.split_whitespace().collect::<Vec<_>>();
    if parts.len() == 5 {
        let minute = parts[0];
        let hour = parts[1];
        let day_of_month = parts[2];
        let month = parts[3];
        let day_of_week = parts[4];

        if hour == "*" && day_of_month == "*" && month == "*" && day_of_week == "*" {
            if let Some(raw) = minute.strip_prefix("*/") {
                if let Ok(interval) = raw.parse::<u32>() {
                    return Some(ScheduledTaskScheduleConfig {
                        mode: ScheduledTaskScheduleMode::EveryMinutes,
                        every_minutes: Some(interval.max(1)),
                        every_hours: None,
                        daily_time: None,
                        weekly_days: None,
                        cron_expression: Some(cron_expression.to_string()),
                    });
                }
            }
        }

        if minute == "0" && day_of_month == "*" && month == "*" && day_of_week == "*" {
            if hour == "*" {
                return Some(ScheduledTaskScheduleConfig {
                    mode: ScheduledTaskScheduleMode::EveryHours,
                    every_minutes: None,
                    every_hours: Some(1),
                    daily_time: None,
                    weekly_days: None,
                    cron_expression: Some(cron_expression.to_string()),
                });
            }
            if let Some(raw) = hour.strip_prefix("*/") {
                if let Ok(interval) = raw.parse::<u32>() {
                    return Some(ScheduledTaskScheduleConfig {
                        mode: ScheduledTaskScheduleMode::EveryHours,
                        every_minutes: None,
                        every_hours: Some(interval.max(1)),
                        daily_time: None,
                        weekly_days: None,
                        cron_expression: Some(cron_expression.to_string()),
                    });
                }
            }
        }

        if day_of_month == "*"
            && month == "*"
            && minute.chars().all(|item| item.is_ascii_digit())
            && hour.chars().all(|item| item.is_ascii_digit())
        {
            let parsed_hour = hour.parse::<u32>().ok();
            let parsed_minute = minute.parse::<u32>().ok();
            if let (Some(parsed_hour), Some(parsed_minute)) = (parsed_hour, parsed_minute) {
                let daily_time = format!(
                    "{}:{}",
                    pad_time_part(parsed_hour.min(23)),
                    pad_time_part(parsed_minute.min(59))
                );
                if day_of_week == "*" {
                    return Some(ScheduledTaskScheduleConfig {
                        mode: ScheduledTaskScheduleMode::DailyAt,
                        every_minutes: None,
                        every_hours: None,
                        daily_time: Some(daily_time),
                        weekly_days: None,
                        cron_expression: Some(cron_expression.to_string()),
                    });
                }
                if let Some((mode, weekly_days)) = parse_day_of_week_field(day_of_week) {
                    return Some(ScheduledTaskScheduleConfig {
                        mode,
                        every_minutes: None,
                        every_hours: None,
                        daily_time: Some(daily_time),
                        weekly_days: Some(weekly_days),
                        cron_expression: Some(cron_expression.to_string()),
                    });
                }
            }
        }
    }

    Some(ScheduledTaskScheduleConfig {
        mode: default_schedule_mode(),
        every_minutes: None,
        every_hours: None,
        daily_time: None,
        weekly_days: None,
        cron_expression: Some(cron_expression.to_string()),
    })
}

pub fn human_schedule_for_task(doc: &ScheduledTaskDoc) -> String {
    match doc.task_kind {
        ScheduledTaskKind::OneShot => {
            let when = doc
                .one_shot_at
                .map(to_rfc3339)
                .unwrap_or_else(|| "未设置".to_string());
            format!("一次性执行：{} ({})", when, doc.timezone)
        }
        ScheduledTaskKind::Cron => match schedule_config_for_task(doc) {
            Some(ScheduledTaskScheduleConfig {
                mode: ScheduledTaskScheduleMode::EveryMinutes,
                every_minutes: Some(value),
                ..
            }) => format!("每 {value} 分钟执行一次 ({})", doc.timezone),
            Some(ScheduledTaskScheduleConfig {
                mode: ScheduledTaskScheduleMode::EveryHours,
                every_hours: Some(value),
                ..
            }) => format!("每 {value} 小时整点执行一次 ({})", doc.timezone),
            Some(ScheduledTaskScheduleConfig {
                mode: ScheduledTaskScheduleMode::DailyAt,
                daily_time: Some(value),
                ..
            }) => format!("每天 {value} 执行 ({})", doc.timezone),
            Some(ScheduledTaskScheduleConfig {
                mode: ScheduledTaskScheduleMode::WeekdaysAt,
                daily_time: Some(value),
                ..
            }) => format!("每个工作日 {value} 执行 ({})", doc.timezone),
            Some(ScheduledTaskScheduleConfig {
                mode: ScheduledTaskScheduleMode::WeeklyOn,
                daily_time: Some(value),
                weekly_days: Some(days),
                ..
            }) => format!(
                "每周 {} {value} 执行 ({})",
                weekday_labels(&days),
                doc.timezone
            ),
            Some(ScheduledTaskScheduleConfig {
                cron_expression: Some(expression),
                ..
            }) => format!("自定义周期任务：{} ({})", expression, doc.timezone),
            _ => format!("周期任务（{}）", doc.timezone),
        },
    }
}

pub fn next_fire_preview_for_task(doc: &ScheduledTaskDoc) -> Option<String> {
    doc.next_fire_at
        .map(|value| format!("{} ({})", to_rfc3339(value), doc.timezone))
}

impl ScheduledTaskSummaryResponse {
    pub fn from_doc(doc: &ScheduledTaskDoc) -> Self {
        let mut doc = doc.clone();
        reconcile_task_contract(&mut doc);
        let now = Utc::now();
        let can_resume = match doc.status {
            ScheduledTaskStatus::Paused => match doc.task_kind {
                ScheduledTaskKind::Cron => true,
                ScheduledTaskKind::OneShot => doc
                    .one_shot_at
                    .map(|value| value.to_chrono() > now)
                    .unwrap_or(false),
            },
            _ => false,
        };
        let resume_hint = if matches!(doc.task_kind, ScheduledTaskKind::OneShot)
            && matches!(
                doc.status,
                ScheduledTaskStatus::Paused | ScheduledTaskStatus::Completed
            )
            && doc
                .one_shot_at
                .map(|value| value.to_chrono() <= now)
                .unwrap_or(false)
        {
            Some(
                "这条一次性任务的原始执行时间已经过去。请修改时间后重新启用，或直接立即运行。"
                    .to_string(),
            )
        } else {
            None
        };
        Self {
            task_id: doc.task_id.clone(),
            team_id: doc.team_id.clone(),
            channel_id: doc.channel_id.clone(),
            owner_user_id: doc.owner_user_id.clone(),
            agent_id: doc.agent_id.clone(),
            title: doc.title.clone(),
            task_kind: doc.task_kind.clone(),
            task_profile: doc.task_profile,
            payload_kind: doc.payload_kind,
            session_binding: doc.session_binding,
            delivery_plan: doc.delivery_plan,
            execution_contract: doc.execution_contract.clone(),
            delivery_tier: doc.delivery_tier,
            timezone: doc.timezone.clone(),
            status: doc.status.clone(),
            human_schedule: human_schedule_for_task(&doc),
            schedule_config: schedule_config_for_task(&doc),
            one_shot_at: doc.one_shot_at.map(to_rfc3339),
            cron_expression: doc.cron_expression.clone(),
            next_fire_at: doc.next_fire_at.map(to_rfc3339),
            next_fire_preview: next_fire_preview_for_task(&doc),
            last_fire_at: doc.last_fire_at.map(to_rfc3339),
            last_run_id: doc.last_run_id.clone(),
            owner_session_id: doc.owner_session_id.clone(),
            last_expected_fire_at: doc.last_expected_fire_at.map(to_rfc3339),
            last_missed_at: doc.last_missed_at.map(to_rfc3339),
            missed_fire_count: doc.missed_fire_count,
            can_resume,
            resume_hint,
            created_at: to_rfc3339(doc.created_at),
            updated_at: to_rfc3339(doc.updated_at),
        }
    }
}

impl ScheduledTaskRunResponse {
    pub fn from_doc(doc: &ScheduledTaskRunDoc) -> Self {
        Self {
            run_id: doc.run_id.clone(),
            task_id: doc.task_id.clone(),
            channel_id: doc.channel_id.clone(),
            status: doc.status.clone(),
            outcome_reason: doc.outcome_reason,
            warning_count: doc.warning_count,
            runtime_session_id: doc.runtime_session_id.clone(),
            fire_message_id: doc.fire_message_id.clone(),
            summary: doc.summary.clone(),
            error: doc.error.clone(),
            self_evaluation: doc.self_evaluation.clone(),
            initial_self_evaluation: doc.initial_self_evaluation.clone(),
            improvement_loop_applied: doc.improvement_loop_applied,
            improvement_loop_count: doc.improvement_loop_count,
            trigger_source: doc.trigger_source.clone(),
            started_at: to_rfc3339(doc.started_at),
            finished_at: doc.finished_at.map(to_rfc3339),
        }
    }
}

impl ScheduledTaskRepairEventResponse {
    pub fn missing_channel(doc: &ScheduledTaskDoc) -> Self {
        Self {
            task_id: doc.task_id.clone(),
            channel_id: doc.channel_id.clone(),
            title: doc.title.clone(),
            owner_user_id: doc.owner_user_id.clone(),
            delivery_tier: doc.delivery_tier,
            reason: ScheduledTaskRepairReason::MissingChannel,
            user_message: format!(
                "任务“{}”引用的频道已不存在，系统已自动将它清理为已删除。",
                doc.title
            ),
        }
    }
}

pub fn to_rfc3339(value: bson::DateTime) -> String {
    value.to_chrono().to_rfc3339()
}

mod bson_datetime_option {
    use chrono::{DateTime, Utc};
    use serde::{Deserialize, Deserializer};

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Option<DateTime<Utc>>, D::Error>
    where
        D: Deserializer<'de>,
    {
        Option::<DateTime<Utc>>::deserialize(deserializer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retrieval_contract_prefers_domestic_sources_for_domestic_prompt() {
        let contract = infer_execution_contract(
            "task-1",
            "每天早上8点检索国内 AI 政策和中国大模型公司最新资讯",
            ScheduledTaskProfile::RetrievalTask,
        );
        assert_eq!(
            contract.source_policy,
            Some(ScheduledTaskSourcePolicy::DomesticPreferred)
        );
    }

    #[test]
    fn retrieval_contract_defaults_to_mixed_sources_for_general_ai_news() {
        let contract = infer_execution_contract(
            "task-1",
            "每天早上8点搜索互联网最新AI资讯并输出中文总结",
            ScheduledTaskProfile::RetrievalTask,
        );
        assert_eq!(
            contract.source_policy,
            Some(ScheduledTaskSourcePolicy::Mixed)
        );
    }

    #[test]
    fn retrieval_prompt_detection_includes_official_release_sources() {
        assert_eq!(
            infer_task_profile_from_prompt(
                "每天早上8点跟踪 OpenAI 官方公告和 release notes 并输出中文总结"
            ),
            ScheduledTaskProfile::RetrievalTask
        );
    }
}
