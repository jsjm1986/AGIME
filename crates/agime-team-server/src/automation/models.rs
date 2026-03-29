use chrono::{DateTime, Utc};
use mongodb::bson::oid::ObjectId;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum IntegrationSpecKind {
    Openapi,
    Postman,
    Curl,
    Markdown,
    Json,
    Text,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum IntegrationAuthType {
    None,
    Bearer,
    Header,
    Basic,
    ApiKey,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionStatus {
    Unknown,
    Success,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DraftStatus {
    Draft,
    Probing,
    Ready,
    Failed,
    Archived,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RunMode {
    Chat,
    RunOnce,
    Schedule,
    Monitor,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Draft,
    Running,
    WaitingInput,
    WaitingApproval,
    Completed,
    Failed,
    Paused,
    Canceled,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ScheduleMode {
    Schedule,
    Monitor,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ScheduleStatus {
    Active,
    Paused,
    Deleted,
}

impl From<ScheduleMode> for RunMode {
    fn from(value: ScheduleMode) -> Self {
        match value {
            ScheduleMode::Schedule => RunMode::Schedule,
            ScheduleMode::Monitor => RunMode::Monitor,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactKind {
    Deliverable,
    Supporting,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutomationProjectDoc {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub project_id: String,
    pub team_id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub created_by: String,
    pub created_at: bson::DateTime,
    pub updated_at: bson::DateTime,
    #[serde(default)]
    pub archived: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutomationIntegrationDoc {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub integration_id: String,
    pub team_id: String,
    pub project_id: String,
    pub name: String,
    pub spec_kind: IntegrationSpecKind,
    pub spec_content: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_file_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    pub auth_type: IntegrationAuthType,
    #[serde(default)]
    pub auth_config: serde_json::Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_summary: Option<String>,
    pub connection_status: ConnectionStatus,
    #[serde(
        default,
        with = "bson_datetime_option",
        skip_serializing_if = "Option::is_none"
    )]
    pub last_tested_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_test_message: Option<String>,
    pub created_by: String,
    pub created_at: bson::DateTime,
    pub updated_at: bson::DateTime,
    #[serde(default)]
    pub archived: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutomationTaskDraftDoc {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub draft_id: String,
    pub team_id: String,
    pub project_id: String,
    pub name: String,
    pub driver_agent_id: String,
    #[serde(default)]
    pub integration_ids: Vec<String>,
    pub goal: String,
    #[serde(default)]
    pub constraints: Vec<String>,
    #[serde(default)]
    pub success_criteria: Vec<String>,
    #[serde(default)]
    pub risk_preference: String,
    pub status: DraftStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub builder_session_id: Option<String>,
    #[serde(default)]
    pub candidate_plan: serde_json::Value,
    #[serde(default)]
    pub probe_report: serde_json::Value,
    #[serde(default)]
    pub candidate_endpoints: serde_json::Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest_probe_prompt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub linked_module_id: Option<String>,
    pub created_by: String,
    pub created_at: bson::DateTime,
    pub updated_at: bson::DateTime,
    #[serde(default)]
    pub archived: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutomationModuleDoc {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub module_id: String,
    pub team_id: String,
    pub project_id: String,
    pub name: String,
    pub version: i32,
    pub source_draft_id: String,
    pub driver_agent_id: String,
    #[serde(default)]
    pub integration_ids: Vec<String>,
    #[serde(default)]
    pub intent_snapshot: serde_json::Value,
    #[serde(default)]
    pub capability_pack_snapshot: serde_json::Value,
    #[serde(default)]
    pub execution_contract: serde_json::Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest_builder_session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_session_id: Option<String>,
    pub created_by: String,
    pub created_at: bson::DateTime,
    pub updated_at: bson::DateTime,
    #[serde(default)]
    pub archived: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutomationRunDoc {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub run_id: String,
    pub team_id: String,
    pub project_id: String,
    pub module_id: String,
    pub module_version: i32,
    pub mode: RunMode,
    pub status: RunStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schedule_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(
        default,
        with = "bson_datetime_option",
        skip_serializing_if = "Option::is_none"
    )]
    pub started_at: Option<DateTime<Utc>>,
    #[serde(
        default,
        with = "bson_datetime_option",
        skip_serializing_if = "Option::is_none"
    )]
    pub finished_at: Option<DateTime<Utc>>,
    pub created_by: String,
    pub created_at: bson::DateTime,
    pub updated_at: bson::DateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutomationArtifactDoc {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub artifact_id: String,
    pub team_id: String,
    pub project_id: String,
    pub run_id: String,
    pub kind: ArtifactKind,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_content: Option<String>,
    #[serde(default)]
    pub metadata: serde_json::Value,
    pub created_at: bson::DateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutomationScheduleDoc {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub schedule_id: String,
    pub team_id: String,
    pub project_id: String,
    pub module_id: String,
    pub module_version: i32,
    pub mode: ScheduleMode,
    pub status: ScheduleStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cron_expression: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub poll_interval_seconds: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub monitor_instruction: Option<String>,
    #[serde(
        default,
        with = "bson_datetime_option",
        skip_serializing_if = "Option::is_none"
    )]
    pub next_run_at: Option<DateTime<Utc>>,
    #[serde(
        default,
        with = "bson_datetime_option",
        skip_serializing_if = "Option::is_none"
    )]
    pub last_run_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_run_id: Option<String>,
    pub created_by: String,
    pub created_at: bson::DateTime,
    pub updated_at: bson::DateTime,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CreateProjectRequest {
    pub team_id: String,
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CreateIntegrationRequest {
    pub team_id: String,
    pub project_id: String,
    pub name: String,
    pub spec_kind: IntegrationSpecKind,
    pub spec_content: String,
    #[serde(default)]
    pub source_file_name: Option<String>,
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default)]
    pub auth_type: Option<IntegrationAuthType>,
    #[serde(default)]
    pub auth_config: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TestIntegrationRequest {
    #[serde(default)]
    pub probe_path: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CreateTaskDraftRequest {
    pub team_id: String,
    pub project_id: String,
    pub name: String,
    pub driver_agent_id: String,
    #[serde(default)]
    pub integration_ids: Vec<String>,
    pub goal: String,
    #[serde(default)]
    pub constraints: Vec<String>,
    #[serde(default)]
    pub success_criteria: Vec<String>,
    #[serde(default)]
    pub risk_preference: Option<String>,
    #[serde(default)]
    pub create_builder_session: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UpdateTaskDraftRequest {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub driver_agent_id: Option<String>,
    #[serde(default)]
    pub integration_ids: Option<Vec<String>>,
    #[serde(default)]
    pub goal: Option<String>,
    #[serde(default)]
    pub constraints: Option<Vec<String>>,
    #[serde(default)]
    pub success_criteria: Option<Vec<String>>,
    #[serde(default)]
    pub risk_preference: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SaveModuleRequest {
    #[serde(default)]
    pub name: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct StartRunRequest {
    pub mode: RunMode,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CreateScheduleRequest {
    pub mode: ScheduleMode,
    #[serde(default)]
    pub cron_expression: Option<String>,
    #[serde(default)]
    pub poll_interval_seconds: Option<i64>,
    #[serde(default)]
    pub monitor_instruction: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UpdateScheduleRequest {
    #[serde(default)]
    pub status: Option<ScheduleStatus>,
    #[serde(default)]
    pub cron_expression: Option<String>,
    #[serde(default)]
    pub poll_interval_seconds: Option<i64>,
    #[serde(default)]
    pub monitor_instruction: Option<String>,
}

mod bson_datetime_option {
    use chrono::{DateTime, Utc};
    use serde::{self, Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S>(date: &Option<DateTime<Utc>>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match date {
            Some(dt) => {
                let bson_dt = bson::DateTime::from_chrono(*dt);
                Serialize::serialize(&bson_dt, serializer)
            }
            None => serializer.serialize_none(),
        }
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Option<DateTime<Utc>>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let opt: Option<bson::DateTime> = Option::deserialize(deserializer)?;
        Ok(opt.map(|dt| dt.to_chrono()))
    }
}
