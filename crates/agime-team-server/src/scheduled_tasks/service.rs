use std::sync::Arc;

use anyhow::Result;
use chrono::{DateTime, Utc};
use mongodb::{
    bson::{self, doc, Bson, Document},
    options::{FindOneAndUpdateOptions, FindOptions, IndexOptions, ReturnDocument},
    IndexModel,
};
use uuid::Uuid;

use agime_team::MongoDb;

use super::models::{
    normalize_execution_contract, reconcile_task_contract, ScheduledTaskDeliveryTier,
    ScheduledTaskDetailResponse, ScheduledTaskDoc, ScheduledTaskKind, ScheduledTaskListResponse,
    ScheduledTaskRepairEventResponse, ScheduledTaskRunDoc, ScheduledTaskRunOutcomeReason,
    ScheduledTaskRunResponse, ScheduledTaskRunStatus, ScheduledTaskSelfEvaluation,
    ScheduledTaskStatus, ScheduledTaskSummaryResponse,
};

const TASKS: &str = "scheduled_tasks";
const RUNS: &str = "scheduled_task_runs";

#[derive(Clone)]
pub struct ScheduledTaskService {
    db: Arc<MongoDb>,
}

impl ScheduledTaskService {
    pub fn new(db: Arc<MongoDb>) -> Self {
        Self { db }
    }

    fn tasks(&self) -> mongodb::Collection<ScheduledTaskDoc> {
        self.db.collection(TASKS)
    }

    fn runs(&self) -> mongodb::Collection<ScheduledTaskRunDoc> {
        self.db.collection(RUNS)
    }

    fn auth_sessions(&self) -> mongodb::Collection<Document> {
        self.db.collection("sessions")
    }

    fn channels(&self) -> mongodb::Collection<Document> {
        self.db.collection("chat_channels")
    }

    async fn channel_exists(&self, channel_id: &str) -> Result<bool> {
        Ok(self
            .channels()
            .count_documents(doc! { "channel_id": channel_id }, None)
            .await?
            > 0)
    }

    async fn mark_task_deleted(&self, task: &ScheduledTaskDoc) -> Result<()> {
        self.tasks()
            .update_one(
                doc! {
                    "task_id": &task.task_id,
                    "team_id": &task.team_id,
                },
                doc! {
                    "$set": {
                        "status": bson::to_bson(&ScheduledTaskStatus::Deleted)?,
                        "next_fire_at": Bson::Null,
                        "lease_owner": Bson::Null,
                        "lease_expires_at": Bson::Null,
                        "updated_at": bson::DateTime::now(),
                    }
                },
                None,
            )
            .await?;
        Ok(())
    }

    async fn prune_orphaned_task(
        &self,
        task: ScheduledTaskDoc,
    ) -> Result<(
        Option<ScheduledTaskDoc>,
        Option<ScheduledTaskRepairEventResponse>,
    )> {
        if self.channel_exists(&task.channel_id).await? {
            return Ok((Some(task), None));
        }
        let repair_event = ScheduledTaskRepairEventResponse::missing_channel(&task);
        self.mark_task_deleted(&task).await?;
        Ok((None, Some(repair_event)))
    }

    async fn ensure_task_contract(&self, mut task: ScheduledTaskDoc) -> Result<ScheduledTaskDoc> {
        let original_profile = task.task_profile;
        let original_contract = task.execution_contract.clone();
        let original_payload_kind = task.payload_kind;
        let original_session_binding = task.session_binding;
        let original_delivery_plan = task.delivery_plan;
        reconcile_task_contract(&mut task);
        task.execution_contract = normalize_execution_contract(
            &task.task_id,
            &task.prompt,
            task.task_profile,
            Some(task.execution_contract.clone()),
        );
        if task.task_profile != original_profile
            || task.execution_contract != original_contract
            || task.payload_kind != original_payload_kind
            || task.session_binding != original_session_binding
            || task.delivery_plan != original_delivery_plan
        {
            self.tasks()
                .update_one(
                    doc! {
                        "task_id": &task.task_id,
                        "team_id": &task.team_id,
                    },
                    doc! {
                        "$set": {
                            "task_profile": bson::to_bson(&task.task_profile)?,
                            "payload_kind": bson::to_bson(&task.payload_kind)?,
                            "session_binding": bson::to_bson(&task.session_binding)?,
                            "delivery_plan": bson::to_bson(&task.delivery_plan)?,
                            "execution_contract": bson::to_bson(&task.execution_contract)?,
                            "updated_at": bson::DateTime::now(),
                        }
                    },
                    None,
                )
                .await?;
        }
        Ok(task)
    }

    pub async fn ensure_indexes(&self) -> Result<()> {
        self.tasks()
            .create_indexes(
                vec![
                    IndexModel::builder()
                        .keys(doc! { "team_id": 1, "task_id": 1 })
                        .options(IndexOptions::builder().unique(true).build())
                        .build(),
                    IndexModel::builder()
                        .keys(doc! { "team_id": 1, "status": 1, "updated_at": -1 })
                        .build(),
                    IndexModel::builder()
                        .keys(doc! { "team_id": 1, "owner_user_id": 1, "status": 1, "updated_at": -1 })
                        .build(),
                    IndexModel::builder()
                        .keys(doc! { "delivery_tier": 1, "owner_session_id": 1, "status": 1 })
                        .build(),
                    IndexModel::builder()
                        .keys(doc! { "status": 1, "next_fire_at": 1, "lease_expires_at": 1 })
                        .build(),
                    IndexModel::builder()
                        .keys(doc! { "channel_id": 1 })
                        .options(IndexOptions::builder().unique(true).build())
                        .build(),
                ],
                None,
            )
            .await?;
        self.runs()
            .create_indexes(
                vec![
                    IndexModel::builder()
                        .keys(doc! { "task_id": 1, "run_id": 1 })
                        .options(IndexOptions::builder().unique(true).build())
                        .build(),
                    IndexModel::builder()
                        .keys(doc! { "task_id": 1, "created_at": -1 })
                        .build(),
                ],
                None,
            )
            .await?;
        Ok(())
    }

    pub async fn create_task(&self, mut doc: ScheduledTaskDoc) -> Result<ScheduledTaskDoc> {
        doc.id = None;
        self.tasks().insert_one(&doc, None).await?;
        Ok(doc)
    }

    pub async fn get_task(&self, team_id: &str, task_id: &str) -> Result<Option<ScheduledTaskDoc>> {
        let task = self
            .tasks()
            .find_one(
                doc! {
                    "team_id": team_id,
                    "task_id": task_id,
                    "status": { "$ne": "deleted" },
                },
                None,
            )
            .await?;
        let Some(task) = task else {
            return Ok(None);
        };
        let (task, _) = self.prune_orphaned_task(task).await?;
        match task {
            Some(task) => Ok(Some(self.ensure_task_contract(task).await?)),
            None => Ok(None),
        }
    }

    pub async fn list_tasks(&self, team_id: &str) -> Result<ScheduledTaskListResponse> {
        let mut cursor = self
            .tasks()
            .find(
                doc! { "team_id": team_id, "status": { "$ne": "deleted" } },
                FindOptions::builder()
                    .sort(doc! { "updated_at": -1 })
                    .build(),
            )
            .await?;
        let mut items = Vec::new();
        let mut repair_events = Vec::new();
        while cursor.advance().await? {
            let doc: ScheduledTaskDoc = cursor.deserialize_current()?;
            let (task, repair_event) = self.prune_orphaned_task(doc).await?;
            if let Some(repair_event) = repair_event {
                repair_events.push(repair_event);
            }
            if let Some(doc) = task {
                let doc = self.ensure_task_contract(doc).await?;
                items.push(ScheduledTaskSummaryResponse::from_doc(&doc));
            }
        }
        Ok(ScheduledTaskListResponse {
            tasks: items,
            repair_events,
        })
    }

    pub async fn count_non_deleted_tasks(&self, team_id: &str) -> Result<u64> {
        let mut cursor = self
            .tasks()
            .find(
                doc! { "team_id": team_id, "status": { "$ne": "deleted" } },
                None,
            )
            .await?;
        let mut count = 0_u64;
        while cursor.advance().await? {
            let doc: ScheduledTaskDoc = cursor.deserialize_current()?;
            if let Some(doc) = self.prune_orphaned_task(doc).await?.0 {
                let _ = self.ensure_task_contract(doc).await?;
                count += 1;
            }
        }
        Ok(count)
    }

    pub async fn list_runs(
        &self,
        task_id: &str,
        limit: i64,
    ) -> Result<Vec<ScheduledTaskRunResponse>> {
        let mut cursor = self
            .runs()
            .find(
                doc! { "task_id": task_id },
                FindOptions::builder()
                    .sort(doc! { "created_at": -1 })
                    .limit(limit)
                    .build(),
            )
            .await?;
        let mut items = Vec::new();
        while cursor.advance().await? {
            let doc: ScheduledTaskRunDoc = cursor.deserialize_current()?;
            items.push(ScheduledTaskRunResponse::from_doc(&doc));
        }
        Ok(items)
    }

    pub async fn get_task_detail(
        &self,
        team_id: &str,
        task_id: &str,
    ) -> Result<Option<ScheduledTaskDetailResponse>> {
        let Some(task) = self.get_task(team_id, task_id).await? else {
            return Ok(None);
        };
        let runs = self.list_runs(task_id, 20).await?;
        Ok(Some(ScheduledTaskDetailResponse {
            summary: ScheduledTaskSummaryResponse::from_doc(&task),
            prompt: task.prompt,
            runs,
        }))
    }

    pub async fn update_task_doc(
        &self,
        team_id: &str,
        task_id: &str,
        set_doc: mongodb::bson::Document,
    ) -> Result<Option<ScheduledTaskDoc>> {
        let updated = self
            .tasks()
            .find_one_and_update(
                doc! {
                    "team_id": team_id,
                    "task_id": task_id,
                    "status": { "$ne": "deleted" },
                },
                doc! { "$set": set_doc },
                FindOneAndUpdateOptions::builder()
                    .return_document(ReturnDocument::After)
                    .build(),
            )
            .await?;
        Ok(updated)
    }

    pub async fn create_run(
        &self,
        task: &ScheduledTaskDoc,
        trigger_source: &str,
        fire_message_id: &str,
        runtime_session_id: &str,
        expected_fire_at: Option<DateTime<Utc>>,
    ) -> Result<ScheduledTaskRunDoc> {
        let now = bson::DateTime::now();
        let run = ScheduledTaskRunDoc {
            id: None,
            run_id: Uuid::new_v4().to_string(),
            task_id: task.task_id.clone(),
            team_id: task.team_id.clone(),
            channel_id: task.channel_id.clone(),
            runtime_session_id: Some(runtime_session_id.to_string()),
            fire_message_id: Some(fire_message_id.to_string()),
            status: ScheduledTaskRunStatus::Running,
            outcome_reason: None,
            warning_count: 0,
            summary: None,
            error: None,
            self_evaluation: None,
            initial_self_evaluation: None,
            improvement_loop_applied: false,
            improvement_loop_count: 0,
            trigger_source: trigger_source.to_string(),
            started_at: now,
            finished_at: None,
            created_at: now,
            updated_at: now,
        };
        self.runs().insert_one(&run, None).await?;
        let mut set_doc = doc! {
            "last_run_id": &run.run_id,
            "last_fire_at": now,
            "updated_at": now,
        };
        if let Some(expected_fire_at) = expected_fire_at {
            set_doc.insert(
                "last_expected_fire_at",
                bson::DateTime::from_chrono(expected_fire_at),
            );
        }
        if trigger_source == "missed_recovery" {
            set_doc.insert("last_missed_at", now);
        }
        let mut update_doc = doc! { "$set": set_doc };
        if trigger_source == "missed_recovery" {
            update_doc.insert("$inc", doc! { "missed_fire_count": 1_i64 });
        }
        self.tasks()
            .update_one(
                doc! { "task_id": &task.task_id, "team_id": &task.team_id },
                update_doc,
                None,
            )
            .await?;
        Ok(run)
    }

    pub async fn complete_run(
        &self,
        task: &ScheduledTaskDoc,
        run_id: &str,
        status: ScheduledTaskRunStatus,
        outcome_reason: Option<ScheduledTaskRunOutcomeReason>,
        warning_count: i32,
        summary: Option<String>,
        error: Option<String>,
        self_evaluation: Option<ScheduledTaskSelfEvaluation>,
        next_fire_at: Option<DateTime<Utc>>,
        release_owner: &str,
    ) -> Result<()> {
        let now = bson::DateTime::now();
        self.runs()
            .update_one(
                doc! { "task_id": &task.task_id, "run_id": run_id },
                doc! {
                    "$set": {
                        "status": bson::to_bson(&status)?,
                        "outcome_reason": bson::to_bson(&outcome_reason)?,
                        "warning_count": warning_count,
                        "summary": bson::to_bson(&summary)?,
                        "error": bson::to_bson(&error)?,
                        "self_evaluation": bson::to_bson(&self_evaluation)?,
                        "finished_at": now,
                        "updated_at": now,
                    }
                },
                None,
            )
            .await?;

        let terminal_status = match task.status {
            ScheduledTaskStatus::Deleted => ScheduledTaskStatus::Deleted,
            ScheduledTaskStatus::Paused => ScheduledTaskStatus::Paused,
            ScheduledTaskStatus::Completed => ScheduledTaskStatus::Completed,
            _ => task.status,
        };
        let task_status = if matches!(task.status, ScheduledTaskStatus::Active)
            && matches!(task.task_kind, ScheduledTaskKind::OneShot)
        {
            ScheduledTaskStatus::Completed
        } else {
            terminal_status
        };

        self.tasks()
            .update_one(
                doc! {
                    "task_id": &task.task_id,
                    "team_id": &task.team_id,
                    "$or": [
                        { "lease_owner": release_owner },
                        { "lease_owner": Bson::Null },
                        { "lease_owner": { "$exists": false } }
                    ],
                },
                doc! {
                    "$set": {
                        "status": bson::to_bson(&task_status)?,
                        "next_fire_at": bson::to_bson(&next_fire_at.map(bson::DateTime::from_chrono))?,
                        "updated_at": now,
                        "lease_owner": Bson::Null,
                        "lease_expires_at": Bson::Null,
                    }
                },
                None,
            )
            .await?;
        Ok(())
    }

    pub async fn update_run_self_evaluation(
        &self,
        task_id: &str,
        run_id: &str,
        self_evaluation: &ScheduledTaskSelfEvaluation,
        initial_self_evaluation: Option<&ScheduledTaskSelfEvaluation>,
        improvement_loop_applied: bool,
        improvement_loop_count: i32,
    ) -> Result<()> {
        let mut set_doc = doc! {
            "self_evaluation": bson::to_bson(self_evaluation)?,
            "improvement_loop_applied": improvement_loop_applied,
            "improvement_loop_count": improvement_loop_count,
            "updated_at": bson::DateTime::now(),
        };
        set_doc.insert(
            "initial_self_evaluation",
            bson::to_bson(&initial_self_evaluation)?,
        );
        self.runs()
            .update_one(
                doc! { "task_id": task_id, "run_id": run_id },
                doc! { "$set": set_doc },
                None,
            )
            .await?;
        Ok(())
    }

    pub async fn update_run_result_projection(
        &self,
        task_id: &str,
        run_id: &str,
        status: ScheduledTaskRunStatus,
        outcome_reason: Option<ScheduledTaskRunOutcomeReason>,
        warning_count: i32,
        summary: Option<String>,
        error: Option<String>,
    ) -> Result<()> {
        self.runs()
            .update_one(
                doc! { "task_id": task_id, "run_id": run_id },
                doc! {
                    "$set": {
                        "status": bson::to_bson(&status)?,
                        "outcome_reason": bson::to_bson(&outcome_reason)?,
                        "warning_count": warning_count,
                        "summary": bson::to_bson(&summary)?,
                        "error": bson::to_bson(&error)?,
                        "updated_at": bson::DateTime::now(),
                    }
                },
                None,
            )
            .await?;
        Ok(())
    }

    pub async fn release_lease(
        &self,
        team_id: &str,
        task_id: &str,
        release_owner: &str,
    ) -> Result<()> {
        self.tasks()
            .update_one(
                doc! {
                    "team_id": team_id,
                    "task_id": task_id,
                    "lease_owner": release_owner,
                },
                doc! {
                    "$set": {
                        "lease_owner": Bson::Null,
                        "lease_expires_at": Bson::Null,
                        "updated_at": bson::DateTime::now(),
                    }
                },
                None,
            )
            .await?;
        Ok(())
    }

    pub async fn claim_due_tasks(
        &self,
        now: DateTime<Utc>,
        lease_owner: &str,
        lease_secs: i64,
        limit: usize,
    ) -> Result<Vec<ScheduledTaskDoc>> {
        let mut claimed = Vec::new();
        for _ in 0..limit {
            let Some(task) = self
                .tasks()
                .find_one_and_update(
                    doc! {
                        "status": "active",
                        "next_fire_at": { "$lte": bson::DateTime::from_chrono(now) },
                        "$or": [
                            { "lease_owner": Bson::Null },
                            { "lease_owner": { "$exists": false } },
                            { "lease_expires_at": { "$lt": bson::DateTime::from_chrono(now) } },
                        ],
                    },
                    doc! {
                        "$set": {
                            "lease_owner": lease_owner,
                            "lease_expires_at": bson::DateTime::from_chrono(now + chrono::Duration::seconds(lease_secs)),
                            "updated_at": bson::DateTime::now(),
                        }
                    },
                    FindOneAndUpdateOptions::builder()
                        .sort(doc! { "next_fire_at": 1, "updated_at": 1 })
                        .return_document(ReturnDocument::After)
                        .build(),
            )
            .await?
        else {
            break;
        };
            claimed.push(self.ensure_task_contract(task).await?);
        }
        Ok(claimed)
    }

    pub async fn claim_task_for_manual_run(
        &self,
        team_id: &str,
        task_id: &str,
        lease_owner: &str,
        lease_secs: i64,
    ) -> Result<Option<ScheduledTaskDoc>> {
        let now = Utc::now();
        let task = self
            .tasks()
            .find_one_and_update(
                doc! {
                    "team_id": team_id,
                    "task_id": task_id,
                    "status": { "$in": ["draft", "active", "paused", "completed"] },
                    "$or": [
                        { "lease_owner": Bson::Null },
                        { "lease_owner": { "$exists": false } },
                        { "lease_expires_at": { "$lt": bson::DateTime::from_chrono(now) } },
                    ],
                },
                doc! {
                    "$set": {
                        "lease_owner": lease_owner,
                        "lease_expires_at": bson::DateTime::from_chrono(now + chrono::Duration::seconds(lease_secs)),
                        "updated_at": bson::DateTime::now(),
                    }
                },
                FindOneAndUpdateOptions::builder()
                    .return_document(ReturnDocument::After)
                    .build(),
            )
            .await?;
        match task {
            Some(task) => Ok(Some(self.ensure_task_contract(task).await?)),
            None => Ok(None),
        }
    }

    pub async fn expire_session_scoped_tasks(
        &self,
        now: DateTime<Utc>,
    ) -> Result<Vec<ScheduledTaskDoc>> {
        let mut cursor = self
            .tasks()
            .find(
                doc! {
                    "delivery_tier": bson::to_bson(&ScheduledTaskDeliveryTier::SessionScoped)?,
                    "status": { "$in": ["draft", "active", "paused", "completed"] },
                    "owner_session_id": { "$type": "string", "$ne": "" },
                },
                None,
            )
            .await?;
        let mut expired = Vec::new();
        while cursor.advance().await? {
            let task: ScheduledTaskDoc = cursor.deserialize_current()?;
            let Some(owner_session_id) = task.owner_session_id.as_deref() else {
                continue;
            };
            let active_session = self
                .auth_sessions()
                .find_one(
                    doc! {
                        "session_id": owner_session_id,
                        "expires_at": { "$gt": bson::DateTime::from_chrono(now) },
                    },
                    None,
                )
                .await?;
            if active_session.is_some() {
                continue;
            }
            self.tasks()
                .update_one(
                    doc! {
                        "task_id": &task.task_id,
                        "team_id": &task.team_id,
                    },
                    doc! {
                        "$set": {
                            "status": bson::to_bson(&ScheduledTaskStatus::Deleted)?,
                            "next_fire_at": Bson::Null,
                            "lease_owner": Bson::Null,
                            "lease_expires_at": Bson::Null,
                            "updated_at": bson::DateTime::now(),
                        }
                    },
                    None,
                )
                .await?;
            expired.push(task);
        }
        Ok(expired)
    }
}
