use std::sync::Arc;

use anyhow::Result;
use chrono::{DateTime, Utc};
use futures::TryStreamExt;
use mongodb::{
    bson::{doc, Document},
    options::{FindOptions, IndexOptions},
    IndexModel,
};
use serde_json::{json, Value};
use std::collections::HashSet;
use uuid::Uuid;

use agime_team::MongoDb;

use super::contract::{assess_draft_publish_readiness, summarize_verification_quality};
use super::models::{
    ArtifactKind, AutomationArtifactDoc, AutomationIntegrationDoc, AutomationModuleDoc,
    AutomationProjectDoc, AutomationRunDoc, AutomationScheduleDoc, AutomationTaskDraftDoc,
    ConnectionStatus, CreateIntegrationRequest, CreateProjectRequest, CreateScheduleRequest,
    CreateTaskDraftRequest, DraftStatus, IntegrationAuthType, RunMode, RunStatus,
    SaveModuleRequest, ScheduleStatus, UpdateScheduleRequest, UpdateTaskDraftRequest,
};

const PROJECTS: &str = "automation_projects";
const INTEGRATIONS: &str = "automation_integrations";
const DRAFTS: &str = "automation_task_drafts";
const MODULES: &str = "automation_modules";
const RUNS: &str = "automation_runs";
const ARTIFACTS: &str = "automation_artifacts";
const SCHEDULES: &str = "automation_schedules";

#[derive(Clone)]
pub struct AutomationService {
    db: Arc<MongoDb>,
}

impl AutomationService {
    pub fn new(db: Arc<MongoDb>) -> Self {
        Self { db }
    }

    pub async fn ensure_indexes(&self) -> Result<()> {
        self.db
            .collection::<AutomationProjectDoc>(PROJECTS)
            .create_indexes(
                vec![
                    IndexModel::builder()
                        .keys(doc! { "team_id": 1, "project_id": 1 })
                        .options(IndexOptions::builder().unique(true).build())
                        .build(),
                    IndexModel::builder()
                        .keys(doc! { "team_id": 1, "archived": 1, "updated_at": -1 })
                        .build(),
                ],
                None,
            )
            .await?;
        self.db
            .collection::<AutomationIntegrationDoc>(INTEGRATIONS)
            .create_indexes(
                vec![
                    IndexModel::builder()
                        .keys(doc! { "team_id": 1, "integration_id": 1 })
                        .options(IndexOptions::builder().unique(true).build())
                        .build(),
                    IndexModel::builder()
                        .keys(doc! { "project_id": 1, "archived": 1, "updated_at": -1 })
                        .build(),
                ],
                None,
            )
            .await?;
        self.db
            .collection::<AutomationTaskDraftDoc>(DRAFTS)
            .create_indexes(
                vec![
                    IndexModel::builder()
                        .keys(doc! { "team_id": 1, "draft_id": 1 })
                        .options(IndexOptions::builder().unique(true).build())
                        .build(),
                    IndexModel::builder()
                        .keys(doc! { "project_id": 1, "archived": 1, "updated_at": -1 })
                        .build(),
                ],
                None,
            )
            .await?;
        self.db
            .collection::<AutomationModuleDoc>(MODULES)
            .create_indexes(
                vec![
                    IndexModel::builder()
                        .keys(doc! { "team_id": 1, "module_id": 1 })
                        .options(IndexOptions::builder().unique(true).build())
                        .build(),
                    IndexModel::builder()
                        .keys(doc! { "project_id": 1, "archived": 1, "updated_at": -1 })
                        .build(),
                ],
                None,
            )
            .await?;
        self.db
            .collection::<AutomationRunDoc>(RUNS)
            .create_indexes(
                vec![
                    IndexModel::builder()
                        .keys(doc! { "team_id": 1, "run_id": 1 })
                        .options(IndexOptions::builder().unique(true).build())
                        .build(),
                    IndexModel::builder()
                        .keys(doc! { "project_id": 1, "created_at": -1 })
                        .build(),
                ],
                None,
            )
            .await?;
        self.db
            .collection::<AutomationArtifactDoc>(ARTIFACTS)
            .create_indexes(
                vec![IndexModel::builder()
                    .keys(doc! { "run_id": 1, "created_at": 1 })
                    .build()],
                None,
            )
            .await?;
        self.db
            .collection::<AutomationScheduleDoc>(SCHEDULES)
            .create_indexes(
                vec![
                    IndexModel::builder()
                        .keys(doc! { "team_id": 1, "schedule_id": 1 })
                        .options(IndexOptions::builder().unique(true).build())
                        .build(),
                    IndexModel::builder()
                        .keys(doc! { "status": 1, "next_run_at": 1 })
                        .build(),
                ],
                None,
            )
            .await?;
        Ok(())
    }

    fn projects(&self) -> mongodb::Collection<AutomationProjectDoc> {
        self.db.collection(PROJECTS)
    }

    fn integrations(&self) -> mongodb::Collection<AutomationIntegrationDoc> {
        self.db.collection(INTEGRATIONS)
    }

    fn drafts(&self) -> mongodb::Collection<AutomationTaskDraftDoc> {
        self.db.collection(DRAFTS)
    }

    fn modules(&self) -> mongodb::Collection<AutomationModuleDoc> {
        self.db.collection(MODULES)
    }

    fn runs(&self) -> mongodb::Collection<AutomationRunDoc> {
        self.db.collection(RUNS)
    }

    fn artifacts(&self) -> mongodb::Collection<AutomationArtifactDoc> {
        self.db.collection(ARTIFACTS)
    }

    fn schedules(&self) -> mongodb::Collection<AutomationScheduleDoc> {
        self.db.collection(SCHEDULES)
    }

    fn schedules_raw(&self) -> mongodb::Collection<Document> {
        self.db.collection(SCHEDULES)
    }

    pub async fn create_project(
        &self,
        req: CreateProjectRequest,
        actor_user_id: &str,
    ) -> Result<AutomationProjectDoc> {
        let now = bson::DateTime::now();
        let project = AutomationProjectDoc {
            id: None,
            project_id: Uuid::new_v4().to_string(),
            team_id: req.team_id,
            name: req.name.trim().to_string(),
            description: req.description.and_then(normalize_optional_string),
            created_by: actor_user_id.to_string(),
            created_at: now,
            updated_at: now,
            archived: false,
        };
        self.projects().insert_one(&project, None).await?;
        Ok(project)
    }

    pub async fn list_projects(&self, team_id: &str) -> Result<Vec<Value>> {
        let mut cursor = self
            .projects()
            .find(
                doc! { "team_id": team_id, "archived": false },
                FindOptions::builder()
                    .sort(doc! { "updated_at": -1 })
                    .build(),
            )
            .await?;
        let mut items = Vec::new();
        while let Some(project) = cursor.try_next().await? {
            items.push(self.project_to_value(&project).await?);
        }
        Ok(items)
    }

    pub async fn get_project(
        &self,
        team_id: &str,
        project_id: &str,
    ) -> Result<Option<AutomationProjectDoc>> {
        self.projects()
            .find_one(
                doc! { "team_id": team_id, "project_id": project_id, "archived": false },
                None,
            )
            .await
            .map_err(Into::into)
    }

    pub async fn delete_project(&self, team_id: &str, project_id: &str) -> Result<bool> {
        let result = self
            .projects()
            .delete_one(
                doc! { "team_id": team_id, "project_id": project_id, "archived": false },
                None,
            )
            .await?;
        if result.deleted_count == 0 {
            return Ok(false);
        }
        let project_filter = doc! { "team_id": team_id, "project_id": project_id };
        self.integrations()
            .delete_many(project_filter.clone(), None)
            .await?;
        self.drafts()
            .delete_many(project_filter.clone(), None)
            .await?;
        self.modules()
            .delete_many(project_filter.clone(), None)
            .await?;
        self.runs()
            .delete_many(project_filter.clone(), None)
            .await?;
        self.artifacts()
            .delete_many(project_filter.clone(), None)
            .await?;
        self.schedules().delete_many(project_filter, None).await?;
        Ok(true)
    }

    async fn project_to_value(&self, project: &AutomationProjectDoc) -> Result<Value> {
        let integrations_count = self
            .integrations()
            .count_documents(
                doc! { "project_id": &project.project_id, "archived": false },
                None,
            )
            .await?;
        let draft_count = self
            .drafts()
            .count_documents(
                doc! { "project_id": &project.project_id, "archived": false },
                None,
            )
            .await?;
        let module_count = self
            .modules()
            .count_documents(
                doc! { "project_id": &project.project_id, "archived": false },
                None,
            )
            .await?;
        let run_count = self
            .runs()
            .count_documents(doc! { "project_id": &project.project_id }, None)
            .await?;
        let schedule_count = self
            .schedules()
            .count_documents(
                doc! { "project_id": &project.project_id, "status": { "$ne": "deleted" } },
                None,
            )
            .await?;
        Ok(json!({
            "project_id": project.project_id,
            "team_id": project.team_id,
            "name": project.name,
            "description": project.description,
            "created_by": project.created_by,
            "created_at": project.created_at.to_chrono().to_rfc3339(),
            "updated_at": project.updated_at.to_chrono().to_rfc3339(),
            "stats": {
                "integrations": integrations_count,
                "drafts": draft_count,
                "modules": module_count,
                "runs": run_count,
                "schedules": schedule_count,
            }
        }))
    }

    pub async fn create_integration(
        &self,
        req: CreateIntegrationRequest,
        actor_user_id: &str,
    ) -> Result<AutomationIntegrationDoc> {
        let now = bson::DateTime::now();
        let auth_type = req.auth_type.unwrap_or(IntegrationAuthType::None);
        let normalized_base_url = req.base_url.and_then(normalize_optional_string);
        let context_summary = build_integration_context_summary(
            &req.spec_content,
            normalized_base_url.as_deref(),
            auth_type.clone(),
        );
        let integration = AutomationIntegrationDoc {
            id: None,
            integration_id: Uuid::new_v4().to_string(),
            team_id: req.team_id,
            project_id: req.project_id,
            name: req.name.trim().to_string(),
            spec_kind: req.spec_kind,
            spec_content: req.spec_content,
            source_file_name: req.source_file_name.and_then(normalize_optional_string),
            base_url: normalized_base_url,
            auth_type,
            auth_config: req.auth_config.unwrap_or_else(|| json!({})),
            context_summary: Some(context_summary),
            connection_status: ConnectionStatus::Unknown,
            last_tested_at: None,
            last_test_message: None,
            created_by: actor_user_id.to_string(),
            created_at: now,
            updated_at: now,
            archived: false,
        };
        self.integrations().insert_one(&integration, None).await?;
        Ok(integration)
    }

    pub async fn list_integrations(&self, team_id: &str, project_id: &str) -> Result<Vec<Value>> {
        let mut cursor = self
            .integrations()
            .find(
                doc! { "team_id": team_id, "project_id": project_id, "archived": false },
                FindOptions::builder()
                    .sort(doc! { "updated_at": -1 })
                    .build(),
            )
            .await?;
        let mut items = Vec::new();
        while let Some(integration) = cursor.try_next().await? {
            items.push(integration_to_value(&integration));
        }
        Ok(items)
    }

    pub async fn get_integration(
        &self,
        team_id: &str,
        integration_id: &str,
    ) -> Result<Option<AutomationIntegrationDoc>> {
        self.integrations()
            .find_one(
                doc! { "team_id": team_id, "integration_id": integration_id, "archived": false },
                None,
            )
            .await
            .map_err(Into::into)
    }

    pub async fn get_integrations_by_ids(
        &self,
        team_id: &str,
        integration_ids: &[String],
    ) -> Result<Vec<AutomationIntegrationDoc>> {
        let ids = integration_ids.to_vec();
        let mut cursor = self
            .integrations()
            .find(
                doc! { "team_id": team_id, "integration_id": { "$in": ids }, "archived": false },
                None,
            )
            .await?;
        let mut items = Vec::new();
        while let Some(item) = cursor.try_next().await? {
            items.push(item);
        }
        Ok(items)
    }

    pub async fn update_integration_test_result(
        &self,
        team_id: &str,
        integration_id: &str,
        status: ConnectionStatus,
        message: Option<String>,
    ) -> Result<Option<AutomationIntegrationDoc>> {
        self.integrations()
            .update_one(
                doc! { "team_id": team_id, "integration_id": integration_id, "archived": false },
                doc! {
                    "$set": {
                        "connection_status": bson::to_bson(&status)?,
                        "last_tested_at": bson::DateTime::from_chrono(Utc::now()),
                        "last_test_message": message,
                        "updated_at": bson::DateTime::now(),
                    }
                },
                None,
            )
            .await?;
        self.get_integration(team_id, integration_id).await
    }

    pub async fn create_task_draft(
        &self,
        req: CreateTaskDraftRequest,
        actor_user_id: &str,
    ) -> Result<AutomationTaskDraftDoc> {
        let now = bson::DateTime::now();
        let draft = AutomationTaskDraftDoc {
            id: None,
            draft_id: Uuid::new_v4().to_string(),
            team_id: req.team_id,
            project_id: req.project_id,
            name: req.name.trim().to_string(),
            driver_agent_id: req.driver_agent_id,
            integration_ids: req.integration_ids,
            goal: req.goal,
            constraints: req.constraints,
            success_criteria: req.success_criteria,
            risk_preference: req
                .risk_preference
                .unwrap_or_else(|| "balanced".to_string()),
            status: DraftStatus::Draft,
            builder_session_id: None,
            candidate_plan: json!({}),
            probe_report: json!({}),
            candidate_endpoints: json!([]),
            latest_probe_prompt: None,
            linked_module_id: None,
            created_by: actor_user_id.to_string(),
            created_at: now,
            updated_at: now,
            archived: false,
        };
        self.drafts().insert_one(&draft, None).await?;
        Ok(draft)
    }

    pub async fn list_task_drafts(&self, team_id: &str, project_id: &str) -> Result<Vec<Value>> {
        let mut cursor = self
            .drafts()
            .find(
                doc! { "team_id": team_id, "project_id": project_id, "archived": false },
                FindOptions::builder()
                    .sort(doc! { "updated_at": -1 })
                    .build(),
            )
            .await?;
        let mut items = Vec::new();
        while let Some(draft) = cursor.try_next().await? {
            items.push(task_draft_to_compact_value(&draft));
        }
        Ok(items)
    }

    pub async fn get_task_draft(
        &self,
        team_id: &str,
        draft_id: &str,
    ) -> Result<Option<AutomationTaskDraftDoc>> {
        self.drafts()
            .find_one(
                doc! { "team_id": team_id, "draft_id": draft_id, "archived": false },
                None,
            )
            .await
            .map_err(Into::into)
    }

    pub async fn get_task_draft_by_builder_session(
        &self,
        session_id: &str,
    ) -> Result<Option<AutomationTaskDraftDoc>> {
        self.drafts()
            .find_one(
                doc! { "builder_session_id": session_id, "archived": false },
                None,
            )
            .await
            .map_err(Into::into)
    }

    pub async fn update_task_draft(
        &self,
        team_id: &str,
        draft_id: &str,
        req: UpdateTaskDraftRequest,
    ) -> Result<Option<AutomationTaskDraftDoc>> {
        let mut set_doc = doc! { "updated_at": bson::DateTime::now() };
        if let Some(value) = req.name.and_then(normalize_optional_string) {
            set_doc.insert("name", value);
        }
        if let Some(value) = req.driver_agent_id.and_then(normalize_optional_string) {
            set_doc.insert("driver_agent_id", value);
        }
        if let Some(value) = req.integration_ids {
            set_doc.insert("integration_ids", bson::to_bson(&value)?);
        }
        if let Some(value) = req.goal.and_then(normalize_optional_string) {
            set_doc.insert("goal", value);
        }
        if let Some(value) = req.constraints {
            set_doc.insert("constraints", bson::to_bson(&value)?);
        }
        if let Some(value) = req.success_criteria {
            set_doc.insert("success_criteria", bson::to_bson(&value)?);
        }
        if let Some(value) = req.risk_preference.and_then(normalize_optional_string) {
            set_doc.insert("risk_preference", value);
        }
        self.drafts()
            .update_one(
                doc! { "team_id": team_id, "draft_id": draft_id, "archived": false },
                doc! { "$set": set_doc },
                None,
            )
            .await?;
        self.get_task_draft(team_id, draft_id).await
    }

    pub async fn update_task_draft_probe_state(
        &self,
        team_id: &str,
        draft_id: &str,
        status: DraftStatus,
        builder_session_id: Option<String>,
        latest_probe_prompt: Option<String>,
    ) -> Result<Option<AutomationTaskDraftDoc>> {
        let mut set_doc = doc! {
            "status": bson::to_bson(&status)?,
            "updated_at": bson::DateTime::now(),
        };
        if let Some(session_id) = builder_session_id {
            set_doc.insert("builder_session_id", session_id);
        }
        if let Some(prompt) = latest_probe_prompt {
            set_doc.insert("latest_probe_prompt", prompt);
        }
        self.drafts()
            .update_one(
                doc! { "team_id": team_id, "draft_id": draft_id, "archived": false },
                doc! { "$set": set_doc },
                None,
            )
            .await?;
        self.get_task_draft(team_id, draft_id).await
    }

    pub async fn complete_task_draft_probe(
        &self,
        team_id: &str,
        draft_id: &str,
        status: DraftStatus,
        probe_report: Value,
        candidate_plan: Value,
    ) -> Result<()> {
        self.drafts()
            .update_one(
                doc! { "team_id": team_id, "draft_id": draft_id, "archived": false },
                doc! {
                    "$set": {
                        "status": bson::to_bson(&status)?,
                        "probe_report": bson::to_bson(&probe_report)?,
                        "candidate_plan": bson::to_bson(&candidate_plan)?,
                        "updated_at": bson::DateTime::now(),
                    }
                },
                None,
            )
            .await?;
        Ok(())
    }

    pub async fn create_module_from_draft(
        &self,
        team_id: &str,
        draft: &AutomationTaskDraftDoc,
        req: SaveModuleRequest,
        actor_user_id: &str,
    ) -> Result<AutomationModuleDoc> {
        let module_name = req
            .name
            .and_then(normalize_optional_string)
            .unwrap_or_else(|| draft.name.clone());
        let existing_count = self
            .modules()
            .count_documents(
                doc! {
                    "team_id": team_id,
                    "project_id": &draft.project_id,
                    "name": &module_name,
                    "archived": false
                },
                None,
            )
            .await?;
        let version = existing_count as i32 + 1;
        let now = bson::DateTime::now();
        let verified_http_actions = draft
            .candidate_plan
            .get("verified_http_actions")
            .cloned()
            .or_else(|| draft.probe_report.get("verified_http_actions").cloned())
            .unwrap_or_else(|| json!([]));
        let native_execution_ready = verified_http_actions
            .as_array()
            .map(|items| !items.is_empty())
            .unwrap_or(false);
        let module = AutomationModuleDoc {
            id: None,
            module_id: Uuid::new_v4().to_string(),
            team_id: team_id.to_string(),
            project_id: draft.project_id.clone(),
            name: module_name.clone(),
            version,
            source_draft_id: draft.draft_id.clone(),
            driver_agent_id: draft.driver_agent_id.clone(),
            integration_ids: draft.integration_ids.clone(),
            intent_snapshot: json!({
                "goal": draft.goal,
                "constraints": draft.constraints,
                "success_criteria": draft.success_criteria,
                "risk_preference": draft.risk_preference,
            }),
            capability_pack_snapshot: draft.candidate_plan.clone(),
            execution_contract: json!({
                "probe_report": draft.probe_report,
                "builder_session_id": draft.builder_session_id,
                "verified_http_actions": verified_http_actions,
                "native_execution_ready": native_execution_ready,
                "execution_mode": if native_execution_ready { "native_http_preferred" } else { "agent_chat_fallback" },
            }),
            latest_builder_session_id: draft.builder_session_id.clone(),
            runtime_session_id: None,
            created_by: actor_user_id.to_string(),
            created_at: now,
            updated_at: now,
            archived: false,
        };
        self.modules().insert_one(&module, None).await?;
        self.drafts()
            .update_one(
                doc! { "team_id": team_id, "draft_id": &draft.draft_id },
                doc! { "$set": { "linked_module_id": &module.module_id, "updated_at": bson::DateTime::now() } },
                None,
            )
            .await?;
        Ok(module)
    }

    pub async fn list_modules(&self, team_id: &str, project_id: &str) -> Result<Vec<Value>> {
        let mut cursor = self
            .modules()
            .find(
                doc! { "team_id": team_id, "project_id": project_id, "archived": false },
                FindOptions::builder()
                    .sort(doc! { "updated_at": -1 })
                    .build(),
            )
            .await?;
        let mut items = Vec::new();
        while let Some(module) = cursor.try_next().await? {
            items.push(module_to_compact_value(&module));
        }
        Ok(items)
    }

    pub async fn get_module(
        &self,
        team_id: &str,
        module_id: &str,
    ) -> Result<Option<AutomationModuleDoc>> {
        self.modules()
            .find_one(
                doc! { "team_id": team_id, "module_id": module_id, "archived": false },
                None,
            )
            .await
            .map_err(Into::into)
    }

    pub async fn update_module_runtime_session(
        &self,
        team_id: &str,
        module_id: &str,
        runtime_session_id: &str,
    ) -> Result<Option<AutomationModuleDoc>> {
        self.modules()
            .update_one(
                doc! { "team_id": team_id, "module_id": module_id, "archived": false },
                doc! {
                    "$set": {
                        "runtime_session_id": runtime_session_id,
                        "updated_at": bson::DateTime::now(),
                    }
                },
                None,
            )
            .await?;
        self.get_module(team_id, module_id).await
    }

    pub async fn delete_module(&self, team_id: &str, module_id: &str) -> Result<bool> {
        let now = bson::DateTime::now();
        let result = self
            .modules()
            .delete_one(
                doc! { "team_id": team_id, "module_id": module_id, "archived": false },
                None,
            )
            .await?;
        if result.deleted_count == 0 {
            return Ok(false);
        }

        self.drafts()
            .update_many(
                doc! { "team_id": team_id, "linked_module_id": module_id },
                doc! {
                    "$set": {
                        "linked_module_id": bson::Bson::Null,
                        "updated_at": now,
                    }
                },
                None,
            )
            .await?;

        let run_ids: Vec<String> = self
            .list_runs_for_module(team_id, module_id)
            .await?
            .into_iter()
            .map(|run| run.run_id)
            .collect();

        if !run_ids.is_empty() {
            self.artifacts()
                .delete_many(
                    doc! { "team_id": team_id, "run_id": { "$in": &run_ids } },
                    None,
                )
                .await?;
        }

        self.runs()
            .delete_many(doc! { "team_id": team_id, "module_id": module_id }, None)
            .await?;

        self.schedules()
            .delete_many(doc! { "team_id": team_id, "module_id": module_id }, None)
            .await?;

        Ok(true)
    }

    pub async fn create_run(
        &self,
        team_id: &str,
        project_id: &str,
        module_id: &str,
        module_version: i32,
        mode: RunMode,
        created_by: &str,
        schedule_id: Option<String>,
        session_id: Option<String>,
    ) -> Result<AutomationRunDoc> {
        let now = bson::DateTime::now();
        let run = AutomationRunDoc {
            id: None,
            run_id: Uuid::new_v4().to_string(),
            team_id: team_id.to_string(),
            project_id: project_id.to_string(),
            module_id: module_id.to_string(),
            module_version,
            mode,
            status: RunStatus::Running,
            schedule_id,
            session_id,
            summary: None,
            error: None,
            started_at: Some(Utc::now()),
            finished_at: None,
            created_by: created_by.to_string(),
            created_at: now,
            updated_at: now,
        };
        self.runs().insert_one(&run, None).await?;
        Ok(run)
    }

    pub async fn list_runs(&self, team_id: &str, project_id: &str) -> Result<Vec<Value>> {
        let mut cursor = self
            .runs()
            .find(
                doc! { "team_id": team_id, "project_id": project_id },
                FindOptions::builder()
                    .sort(doc! { "created_at": -1 })
                    .build(),
            )
            .await?;
        let mut items = Vec::new();
        while let Some(run) = cursor.try_next().await? {
            items.push(run_to_value(&run));
        }
        Ok(items)
    }

    pub async fn list_runs_for_module(
        &self,
        team_id: &str,
        module_id: &str,
    ) -> Result<Vec<AutomationRunDoc>> {
        let mut cursor = self
            .runs()
            .find(
                doc! { "team_id": team_id, "module_id": module_id },
                FindOptions::builder()
                    .sort(doc! { "created_at": -1 })
                    .limit(12)
                    .build(),
            )
            .await?;
        let mut items = Vec::new();
        while let Some(run) = cursor.try_next().await? {
            items.push(run);
        }
        Ok(items)
    }

    pub async fn get_run(&self, team_id: &str, run_id: &str) -> Result<Option<AutomationRunDoc>> {
        self.runs()
            .find_one(doc! { "team_id": team_id, "run_id": run_id }, None)
            .await
            .map_err(Into::into)
    }

    pub async fn complete_run(
        &self,
        team_id: &str,
        run_id: &str,
        status: RunStatus,
        summary: Option<String>,
        error: Option<String>,
    ) -> Result<()> {
        self.runs()
            .update_one(
                doc! { "team_id": team_id, "run_id": run_id },
                doc! {
                    "$set": {
                        "status": bson::to_bson(&status)?,
                        "summary": summary,
                        "error": error,
                        "finished_at": bson::DateTime::from_chrono(Utc::now()),
                        "updated_at": bson::DateTime::now(),
                    }
                },
                None,
            )
            .await?;
        Ok(())
    }

    pub async fn create_artifact(
        &self,
        team_id: &str,
        project_id: &str,
        run_id: &str,
        kind: ArtifactKind,
        name: &str,
        content_type: Option<String>,
        text_content: Option<String>,
        metadata: Value,
    ) -> Result<AutomationArtifactDoc> {
        let artifact = AutomationArtifactDoc {
            id: None,
            artifact_id: Uuid::new_v4().to_string(),
            team_id: team_id.to_string(),
            project_id: project_id.to_string(),
            run_id: run_id.to_string(),
            kind,
            name: name.to_string(),
            content_type,
            text_content,
            metadata,
            created_at: bson::DateTime::now(),
        };
        self.artifacts().insert_one(&artifact, None).await?;
        Ok(artifact)
    }

    pub async fn list_artifacts(&self, team_id: &str, project_id: &str) -> Result<Vec<Value>> {
        let mut cursor = self
            .artifacts()
            .find(
                doc! { "team_id": team_id, "project_id": project_id },
                FindOptions::builder()
                    .sort(doc! { "created_at": -1 })
                    .build(),
            )
            .await?;
        let mut items = Vec::new();
        while let Some(item) = cursor.try_next().await? {
            items.push(artifact_to_compact_value(&item));
        }
        Ok(items)
    }

    pub async fn list_artifacts_for_module(
        &self,
        team_id: &str,
        module_id: &str,
    ) -> Result<Vec<AutomationArtifactDoc>> {
        let run_ids: Vec<String> = self
            .list_runs_for_module(team_id, module_id)
            .await?
            .into_iter()
            .map(|run| run.run_id)
            .collect();
        if run_ids.is_empty() {
            return Ok(Vec::new());
        }
        let mut cursor = self
            .artifacts()
            .find(
                doc! { "team_id": team_id, "run_id": { "$in": run_ids } },
                FindOptions::builder()
                    .sort(doc! { "created_at": -1 })
                    .limit(16)
                    .build(),
            )
            .await?;
        let mut items = Vec::new();
        while let Some(artifact) = cursor.try_next().await? {
            items.push(artifact);
        }
        Ok(items)
    }

    pub async fn create_schedule(
        &self,
        team_id: &str,
        project_id: &str,
        module_id: &str,
        module_version: i32,
        req: CreateScheduleRequest,
        created_by: &str,
        next_run_at: Option<DateTime<Utc>>,
    ) -> Result<AutomationScheduleDoc> {
        let now = bson::DateTime::now();
        let schedule = AutomationScheduleDoc {
            id: None,
            schedule_id: Uuid::new_v4().to_string(),
            team_id: team_id.to_string(),
            project_id: project_id.to_string(),
            module_id: module_id.to_string(),
            module_version,
            mode: req.mode,
            status: ScheduleStatus::Active,
            cron_expression: req.cron_expression.and_then(normalize_optional_string),
            poll_interval_seconds: req.poll_interval_seconds,
            monitor_instruction: req.monitor_instruction.and_then(normalize_optional_string),
            next_run_at,
            last_run_at: None,
            last_run_id: None,
            created_by: created_by.to_string(),
            created_at: now,
            updated_at: now,
        };
        self.schedules().insert_one(&schedule, None).await?;
        Ok(schedule)
    }

    pub async fn list_schedules(&self, team_id: &str, project_id: &str) -> Result<Vec<Value>> {
        let mut cursor = self
            .schedules()
            .find(
                doc! { "team_id": team_id, "project_id": project_id, "status": { "$ne": "deleted" } },
                FindOptions::builder().sort(doc! { "updated_at": -1 }).build(),
            )
            .await?;
        let mut items = Vec::new();
        while let Some(schedule) = cursor.try_next().await? {
            items.push(schedule_to_value(&schedule));
        }
        Ok(items)
    }

    pub async fn list_schedules_for_module(
        &self,
        team_id: &str,
        module_id: &str,
    ) -> Result<Vec<AutomationScheduleDoc>> {
        let mut cursor = self
            .schedules()
            .find(
                doc! { "team_id": team_id, "module_id": module_id, "status": { "$ne": "deleted" } },
                FindOptions::builder()
                    .sort(doc! { "updated_at": -1 })
                    .limit(12)
                    .build(),
            )
            .await?;
        let mut items = Vec::new();
        while let Some(schedule) = cursor.try_next().await? {
            items.push(schedule);
        }
        Ok(items)
    }

    pub async fn get_schedule(
        &self,
        team_id: &str,
        schedule_id: &str,
    ) -> Result<Option<AutomationScheduleDoc>> {
        self.schedules()
            .find_one(
                doc! { "team_id": team_id, "schedule_id": schedule_id, "status": { "$ne": "deleted" } },
                None,
            )
            .await
            .map_err(Into::into)
    }

    pub async fn update_schedule(
        &self,
        team_id: &str,
        schedule_id: &str,
        req: UpdateScheduleRequest,
        next_run_at: Option<DateTime<Utc>>,
    ) -> Result<Option<AutomationScheduleDoc>> {
        let mut set_doc = doc! { "updated_at": bson::DateTime::now() };
        if let Some(status) = req.status {
            set_doc.insert("status", bson::to_bson(&status)?);
        }
        if req.cron_expression.is_some() {
            set_doc.insert(
                "cron_expression",
                bson::to_bson(&req.cron_expression.and_then(normalize_optional_string))?,
            );
        }
        if req.poll_interval_seconds.is_some() {
            set_doc.insert(
                "poll_interval_seconds",
                bson::to_bson(&req.poll_interval_seconds)?,
            );
        }
        if req.monitor_instruction.is_some() {
            set_doc.insert(
                "monitor_instruction",
                bson::to_bson(&req.monitor_instruction.and_then(normalize_optional_string))?,
            );
        }
        if let Some(next) = next_run_at {
            set_doc.insert("next_run_at", bson::DateTime::from_chrono(next));
        }
        self.schedules()
            .update_one(
                doc! { "team_id": team_id, "schedule_id": schedule_id, "status": { "$ne": "deleted" } },
                doc! { "$set": set_doc },
                None,
            )
            .await?;
        self.get_schedule(team_id, schedule_id).await
    }

    pub async fn delete_schedule(&self, team_id: &str, schedule_id: &str) -> Result<bool> {
        let result = self
            .schedules()
            .update_one(
                doc! { "team_id": team_id, "schedule_id": schedule_id, "status": { "$ne": "deleted" } },
                doc! {
                    "$set": {
                        "status": bson::to_bson(&ScheduleStatus::Deleted)?,
                        "updated_at": bson::DateTime::now(),
                    }
                },
                None,
            )
            .await?;
        Ok(result.modified_count > 0)
    }

    pub async fn claim_due_schedules(
        &self,
        now: DateTime<Utc>,
    ) -> Result<Vec<AutomationScheduleDoc>> {
        let raw_due_count = self
            .schedules()
            .count_documents(
                doc! {
                    "status": "active",
                    "next_run_at": { "$lte": bson::DateTime::from_chrono(now) }
                },
                None,
            )
            .await?;
        tracing::info!(
            "automation scheduler: raw due query at {} matched {} schedule(s)",
            now.to_rfc3339(),
            raw_due_count
        );
        let mut cursor = self
            .schedules_raw()
            .aggregate(
                vec![
                    doc! {
                        "$match": {
                            "status": "active",
                            "next_run_at": { "$lte": bson::DateTime::from_chrono(now) }
                        }
                    },
                    doc! { "$sort": { "next_run_at": 1 } },
                    doc! {
                        "$project": {
                            "_id": 0,
                            "team_id": 1,
                            "schedule_id": 1,
                        }
                    },
                ],
                None,
            )
            .await?;
        let mut claimed = Vec::new();
        let mut seen_schedule_ids = HashSet::new();
        while let Some(schedule_doc) = cursor.try_next().await? {
            let Some(team_id) = schedule_doc.get_str("team_id").ok().map(str::to_string) else {
                tracing::warn!("automation scheduler: aggregate due document missing team_id");
                continue;
            };
            let Some(schedule_id) = schedule_doc.get_str("schedule_id").ok().map(str::to_string)
            else {
                tracing::warn!("automation scheduler: aggregate due document missing schedule_id");
                continue;
            };
            if seen_schedule_ids.contains(&schedule_id) {
                continue;
            }
            let Some(schedule) = self.get_schedule(&team_id, &schedule_id).await? else {
                tracing::warn!(
                    "automation scheduler: aggregate due schedule {} could not be reloaded",
                    schedule_id
                );
                continue;
            };
            tracing::info!(
                "automation scheduler: due cursor yielded schedule {} (team={}, project={})",
                schedule.schedule_id,
                schedule.team_id,
                schedule.project_id
            );
            let next_run = super::scheduler::compute_next_run_at(&schedule, now)?;
            let mut updated = schedule.clone();
            updated.last_run_at = Some(now);
            updated.next_run_at = next_run;
            claimed.push(updated);
            seen_schedule_ids.insert(schedule.schedule_id.clone());
        }
        Ok(claimed)
    }

    pub async fn mark_schedule_run(
        &self,
        team_id: &str,
        schedule_id: &str,
        run_id: &str,
        last_run_at: Option<DateTime<Utc>>,
        next_run_at: Option<DateTime<Utc>>,
    ) -> Result<()> {
        let mut set_doc = doc! {
            "last_run_id": run_id,
            "updated_at": bson::DateTime::now(),
        };
        if let Some(last_run_at) = last_run_at {
            set_doc.insert("last_run_at", bson::DateTime::from_chrono(last_run_at));
        }
        set_doc.insert(
            "next_run_at",
            bson::to_bson(&next_run_at.map(bson::DateTime::from_chrono))?,
        );
        self.schedules()
            .update_one(
                doc! { "team_id": team_id, "schedule_id": schedule_id, "status": { "$ne": "deleted" } },
                doc! { "$set": set_doc },
                None,
            )
            .await?;
        Ok(())
    }
}

fn normalize_optional_string(value: String) -> Option<String> {
    let trimmed = value.trim().to_string();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}

fn truncate_chars(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        value.to_string()
    } else {
        let truncated: String = value.chars().take(max_chars).collect();
        format!("{}…", truncated)
    }
}

fn compact_verified_http_actions(value: &mut Value) {
    let Some(actions) = value
        .get_mut("verified_http_actions")
        .and_then(Value::as_array_mut)
    else {
        return;
    };
    if actions.len() > 2 {
        actions.truncate(2);
    }
    for action in actions {
        if let Some(response) = action.get_mut("response").and_then(Value::as_object_mut) {
            if let Some(text) = response.get("body_text").and_then(Value::as_str) {
                response.insert("body_text".to_string(), json!(truncate_chars(text, 280)));
            }
        }
    }
}

fn compact_candidate_plan(value: &Value) -> Value {
    let mut compact = value.clone();
    if let Some(obj) = compact.as_object_mut() {
        if let Some(text) = obj.get("recommended_path").and_then(Value::as_str) {
            obj.insert(
                "recommended_path".to_string(),
                json!(truncate_chars(text, 1200)),
            );
        }
    }
    compact_verified_http_actions(&mut compact);
    compact
}

fn compact_probe_report(value: &Value) -> Value {
    let mut compact = value.clone();
    if let Some(obj) = compact.as_object_mut() {
        if let Some(text) = obj.get("summary").and_then(Value::as_str) {
            obj.insert("summary".to_string(), json!(truncate_chars(text, 280)));
        }
        if let Some(text) = obj.get("assistant_excerpt").and_then(Value::as_str) {
            obj.insert(
                "assistant_excerpt".to_string(),
                json!(truncate_chars(text, 900)),
            );
        }
    }
    compact_verified_http_actions(&mut compact);
    compact
}

fn compact_execution_contract(value: &Value) -> Value {
    let mut compact = value.clone();
    if let Some(obj) = compact.as_object_mut() {
        if let Some(probe_report) = obj.get("probe_report").cloned() {
            obj.insert(
                "probe_report".to_string(),
                compact_probe_report(&probe_report),
            );
        }
    }
    compact_verified_http_actions(&mut compact);
    compact
}

fn compact_text_content(value: Option<&str>, max_chars: usize) -> Option<String> {
    value.map(|text| truncate_chars(text, max_chars))
}

pub fn integration_to_value(integration: &AutomationIntegrationDoc) -> Value {
    json!({
        "integration_id": integration.integration_id,
        "team_id": integration.team_id,
        "project_id": integration.project_id,
        "name": integration.name,
        "spec_kind": integration.spec_kind,
        "source_file_name": integration.source_file_name,
        "base_url": integration.base_url,
        "auth_type": integration.auth_type,
        "auth_config": integration.auth_config,
        "connection_status": integration.connection_status,
        "last_tested_at": integration.last_tested_at.map(|dt| dt.to_rfc3339()),
        "last_test_message": integration.last_test_message,
        "created_by": integration.created_by,
        "created_at": integration.created_at.to_chrono().to_rfc3339(),
        "updated_at": integration.updated_at.to_chrono().to_rfc3339(),
        "spec_excerpt": integration.spec_content.chars().take(400).collect::<String>(),
        "context_summary": integration.context_summary,
    })
}

fn build_integration_context_summary(
    spec_content: &str,
    base_url: Option<&str>,
    auth_type: IntegrationAuthType,
) -> String {
    let mut hints = Vec::new();
    for raw_line in spec_content.lines() {
        let line = raw_line.trim();
        if line.is_empty() {
            continue;
        }
        let lower = line.to_ascii_lowercase();
        if lower.starts_with("operationid:")
            || lower.starts_with("- url:")
            || line.starts_with('/')
            || lower.contains(" get:")
            || lower.contains(" post:")
            || lower.contains(" put:")
            || lower.contains(" patch:")
            || lower.contains(" delete:")
            || lower.starts_with("get /")
            || lower.starts_with("post /")
            || lower.starts_with("put /")
            || lower.starts_with("patch /")
            || lower.starts_with("delete /")
        {
            hints.push(format!("- {}", line));
        }
        if hints.len() >= 8 {
            break;
        }
    }

    let summary = if hints.is_empty() {
        spec_content.chars().take(800).collect::<String>()
    } else {
        hints.join("\n")
    };

    format!(
        "base_url: {}\nauth_type: {:?}\n{}",
        base_url.unwrap_or("(未设置)"),
        auth_type,
        summary
    )
}

pub fn task_draft_to_value(draft: &AutomationTaskDraftDoc) -> Value {
    task_draft_to_value_with_mode(draft, false)
}

pub fn task_draft_to_compact_value(draft: &AutomationTaskDraftDoc) -> Value {
    task_draft_to_value_with_mode(draft, true)
}

fn task_draft_to_value_with_mode(draft: &AutomationTaskDraftDoc, compact: bool) -> Value {
    let publish_readiness = assess_draft_publish_readiness(draft);
    let candidate_plan = if compact {
        compact_candidate_plan(&draft.candidate_plan)
    } else {
        draft.candidate_plan.clone()
    };
    let probe_report = if compact {
        compact_probe_report(&draft.probe_report)
    } else {
        draft.probe_report.clone()
    };
    json!({
        "draft_id": draft.draft_id,
        "app_draft_id": draft.draft_id,
        "team_id": draft.team_id,
        "project_id": draft.project_id,
        "name": draft.name,
        "driver_agent_id": draft.driver_agent_id,
        "integration_ids": draft.integration_ids,
        "goal": draft.goal,
        "constraints": draft.constraints,
        "success_criteria": draft.success_criteria,
        "risk_preference": draft.risk_preference,
        "status": draft.status,
        "builder_session_id": draft.builder_session_id,
        "candidate_plan": candidate_plan,
        "probe_report": probe_report,
        "candidate_endpoints": draft.candidate_endpoints,
        "latest_probe_prompt": draft.latest_probe_prompt,
        "linked_module_id": draft.linked_module_id,
        "publish_readiness": publish_readiness,
        "agent_app": {
            "kind": "draft",
            "builder_session_id": draft.builder_session_id,
        },
        "summary": {
            "integration_count": draft.integration_ids.len(),
            "has_builder_session": draft.builder_session_id.is_some(),
            "has_candidate_plan": draft.candidate_plan != json!({}),
            "has_probe_report": draft.probe_report != json!({}),
            "verified_http_actions": publish_readiness.verification.valid_actions,
            "structured_verified_http_actions": publish_readiness.verification.structured_actions,
            "shell_fallback_verified_http_actions": publish_readiness.verification.shell_fallback_actions,
        },
        "created_by": draft.created_by,
        "created_at": draft.created_at.to_chrono().to_rfc3339(),
        "updated_at": draft.updated_at.to_chrono().to_rfc3339(),
    })
}

pub fn module_to_value(module: &AutomationModuleDoc) -> Value {
    module_to_value_with_mode(module, false)
}

pub fn module_to_compact_value(module: &AutomationModuleDoc) -> Value {
    module_to_value_with_mode(module, true)
}

fn module_to_value_with_mode(module: &AutomationModuleDoc, compact: bool) -> Value {
    let native_execution_ready = module
        .execution_contract
        .get("native_execution_ready")
        .and_then(|value| value.as_bool())
        .unwrap_or(false);
    let verified_http_actions: Vec<super::contract::VerifiedHttpAction> = module
        .execution_contract
        .get("verified_http_actions")
        .cloned()
        .and_then(|value| {
            serde_json::from_value::<Vec<super::contract::VerifiedHttpAction>>(value).ok()
        })
        .unwrap_or_default();
    let verification = summarize_verification_quality(&verified_http_actions);
    let intent_snapshot = if compact {
        json!({
            "goal": module.intent_snapshot.get("goal"),
            "constraints": module.intent_snapshot.get("constraints"),
            "success_criteria": module.intent_snapshot.get("success_criteria"),
            "risk_preference": module.intent_snapshot.get("risk_preference"),
        })
    } else {
        module.intent_snapshot.clone()
    };
    let capability_pack_snapshot = if compact {
        compact_candidate_plan(&module.capability_pack_snapshot)
    } else {
        module.capability_pack_snapshot.clone()
    };
    let execution_contract = if compact {
        compact_execution_contract(&module.execution_contract)
    } else {
        module.execution_contract.clone()
    };
    json!({
        "module_id": module.module_id,
        "app_id": module.module_id,
        "team_id": module.team_id,
        "project_id": module.project_id,
        "name": module.name,
        "version": module.version,
        "source_draft_id": module.source_draft_id,
        "driver_agent_id": module.driver_agent_id,
        "integration_ids": module.integration_ids,
        "intent_snapshot": intent_snapshot,
        "capability_pack_snapshot": capability_pack_snapshot,
        "execution_contract": execution_contract,
        "latest_builder_session_id": module.latest_builder_session_id,
        "runtime_session_id": module.runtime_session_id,
        "agent_app": {
            "kind": "published",
            "runtime_session_id": module.runtime_session_id,
        },
        "summary": {
            "integration_count": module.integration_ids.len(),
            "has_execution_contract": module.execution_contract != json!({}),
            "native_execution_ready": native_execution_ready,
            "verified_http_actions": verification.valid_actions,
            "structured_verified_http_actions": verification.structured_actions,
            "shell_fallback_verified_http_actions": verification.shell_fallback_actions,
        },
        "created_by": module.created_by,
        "created_at": module.created_at.to_chrono().to_rfc3339(),
        "updated_at": module.updated_at.to_chrono().to_rfc3339(),
    })
}

pub fn run_to_value(run: &AutomationRunDoc) -> Value {
    json!({
        "run_id": run.run_id,
        "team_id": run.team_id,
        "project_id": run.project_id,
        "module_id": run.module_id,
        "app_id": run.module_id,
        "module_version": run.module_version,
        "mode": run.mode,
        "status": run.status,
        "schedule_id": run.schedule_id,
        "session_id": run.session_id,
        "summary": compact_text_content(run.summary.as_deref(), 320),
        "error": run.error,
        "started_at": run.started_at.map(|dt| dt.to_rfc3339()),
        "finished_at": run.finished_at.map(|dt| dt.to_rfc3339()),
        "created_by": run.created_by,
        "created_at": run.created_at.to_chrono().to_rfc3339(),
        "updated_at": run.updated_at.to_chrono().to_rfc3339(),
    })
}

pub fn artifact_to_value(artifact: &AutomationArtifactDoc) -> Value {
    artifact_to_value_with_mode(artifact, false)
}

pub fn artifact_to_compact_value(artifact: &AutomationArtifactDoc) -> Value {
    artifact_to_value_with_mode(artifact, true)
}

fn artifact_to_value_with_mode(artifact: &AutomationArtifactDoc, compact: bool) -> Value {
    let text_content = if compact {
        compact_text_content(artifact.text_content.as_deref(), 420)
    } else {
        artifact.text_content.clone()
    };
    json!({
        "artifact_id": artifact.artifact_id,
        "team_id": artifact.team_id,
        "project_id": artifact.project_id,
        "run_id": artifact.run_id,
        "kind": artifact.kind,
        "name": artifact.name,
        "content_type": artifact.content_type,
        "text_content": text_content,
        "metadata": artifact.metadata,
        "created_at": artifact.created_at.to_chrono().to_rfc3339(),
    })
}

pub fn schedule_to_value(schedule: &AutomationScheduleDoc) -> Value {
    json!({
        "schedule_id": schedule.schedule_id,
        "team_id": schedule.team_id,
        "project_id": schedule.project_id,
        "module_id": schedule.module_id,
        "app_id": schedule.module_id,
        "module_version": schedule.module_version,
        "mode": schedule.mode,
        "status": schedule.status,
        "cron_expression": schedule.cron_expression,
        "poll_interval_seconds": schedule.poll_interval_seconds,
        "monitor_instruction": schedule.monitor_instruction,
        "next_run_at": schedule.next_run_at.map(|dt| dt.to_rfc3339()),
        "last_run_at": schedule.last_run_at.map(|dt| dt.to_rfc3339()),
        "last_run_id": schedule.last_run_id,
        "created_by": schedule.created_by,
        "created_at": schedule.created_at.to_chrono().to_rfc3339(),
        "updated_at": schedule.updated_at.to_chrono().to_rfc3339(),
    })
}
