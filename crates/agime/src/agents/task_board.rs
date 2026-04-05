use std::collections::HashMap;
use std::path::PathBuf;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::agents::harness::runtime_state::HarnessWorkerRuntimeState;
use crate::session::{
    ExtensionState, ProjectedTaskRef, SessionManager, SessionType, TaskItem, TaskProjectionEvent,
    TaskScope, TaskStatus, TasksStateV2, TodoState,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskBoardId(pub String);

impl TaskBoardId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<String> for TaskBoardId {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl From<&str> for TaskBoardId {
    fn from(value: &str) -> Self {
        Self(value.to_string())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TaskBoardScope {
    Leader,
    Worker,
    Standalone,
}

impl From<TaskBoardScope> for TaskScope {
    fn from(value: TaskBoardScope) -> Self {
        match value {
            TaskBoardScope::Leader => TaskScope::Leader,
            TaskBoardScope::Worker => TaskScope::Worker,
            TaskBoardScope::Standalone => TaskScope::Standalone,
        }
    }
}

impl From<TaskScope> for TaskBoardScope {
    fn from(value: TaskScope) -> Self {
        match value {
            TaskScope::Leader => TaskBoardScope::Leader,
            TaskScope::Worker => TaskBoardScope::Worker,
            TaskScope::Standalone => TaskBoardScope::Standalone,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskBoardContext {
    pub board_id: TaskBoardId,
    pub scope: TaskBoardScope,
    pub storage_session_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub projection_target: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub logical_worker_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attempt_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attempt_index: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_task_id: Option<String>,
}

impl TaskBoardContext {
    pub fn standalone(session_id: impl Into<String>) -> Self {
        let session_id = session_id.into();
        Self {
            board_id: TaskBoardId::new(session_id.clone()),
            scope: TaskBoardScope::Standalone,
            storage_session_id: session_id,
            projection_target: None,
            logical_worker_id: None,
            attempt_id: None,
            attempt_index: None,
            previous_task_id: None,
        }
    }

    pub fn from_runtime(
        session_id: impl Into<String>,
        runtime: Option<&HarnessWorkerRuntimeState>,
    ) -> Self {
        let session_id = session_id.into();
        match runtime {
            Some(runtime)
                if runtime.coordinator_role == crate::agents::harness::CoordinatorRole::Worker =>
            {
                let parent_board_id = runtime
                    .task_board_session_id
                    .clone()
                    .unwrap_or_else(|| session_id.clone());
                let logical_worker_id = runtime
                    .logical_worker_id
                    .clone()
                    .or_else(|| runtime.worker_name.clone())
                    .unwrap_or_else(|| session_id.clone());
                let board_id = format!("{}:{}", parent_board_id, logical_worker_id);
                Self {
                    board_id: TaskBoardId::new(board_id.clone()),
                    scope: TaskBoardScope::Worker,
                    storage_session_id: board_id,
                    projection_target: Some(parent_board_id),
                    logical_worker_id: Some(logical_worker_id),
                    attempt_id: runtime.attempt_id.clone(),
                    attempt_index: runtime.attempt_index,
                    previous_task_id: runtime.previous_task_id.clone(),
                }
            }
            Some(runtime)
                if runtime
                    .task_board_session_id
                    .as_deref()
                    .is_some_and(|value| value == session_id) =>
            {
                Self {
                    board_id: TaskBoardId::new(session_id.clone()),
                    scope: TaskBoardScope::Leader,
                    storage_session_id: session_id,
                    projection_target: None,
                    logical_worker_id: None,
                    attempt_id: None,
                    attempt_index: None,
                    previous_task_id: None,
                }
            }
            _ => Self::standalone(session_id),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskBoardSnapshot {
    pub board_id: TaskBoardId,
    pub scope: TaskBoardScope,
    pub high_water_mark: u64,
    pub items: Vec<TaskItem>,
    pub updated_at: String,
}

#[derive(Debug, Clone, Default)]
pub struct TaskBoardUpdate {
    pub task_id: String,
    pub subject: Option<String>,
    pub description: Option<String>,
    pub active_form: Option<String>,
    pub owner: Option<String>,
    pub status: Option<TaskStatus>,
    pub add_blocks: Option<Vec<String>>,
    pub add_blocked_by: Option<Vec<String>>,
    pub metadata: Option<HashMap<String, Value>>,
}

#[derive(Debug, Clone)]
pub struct TaskBoardMutation {
    pub snapshot: TaskBoardSnapshot,
    pub task: TaskItem,
    pub projection: Option<TaskProjectionEvent>,
}

pub struct TaskBoardStore;

impl TaskBoardStore {
    async fn ensure_storage_session(
        storage_session_id: &str,
    ) -> Result<crate::session::Session, String> {
        match SessionManager::get_session(storage_session_id, false).await {
            Ok(session) => Ok(session),
            Err(_) => SessionManager::create_session_with_id(
                storage_session_id.to_string(),
                std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
                format!("task-board-{}", storage_session_id),
                SessionType::Hidden,
            )
            .await
            .map_err(|_| "Failed to initialize task board storage session".to_string()),
        }
    }

    async fn load_state(
        context: &TaskBoardContext,
    ) -> Result<(crate::session::Session, TasksStateV2), String> {
        let mut session = Self::ensure_storage_session(&context.storage_session_id).await?;
        let mut extension_data = session.extension_data.clone();
        let mut state = if let Some(existing) = TasksStateV2::from_extension_data(&extension_data) {
            existing
        } else if let Some(legacy) = TodoState::from_extension_data(&extension_data) {
            let migrated = TasksStateV2::migrate_from_legacy_todo(
                &legacy,
                context.scope.into(),
                context.board_id.0.clone(),
            );
            migrated
                .to_extension_data(&mut extension_data)
                .map_err(|e| e.to_string())?;
            SessionManager::set_extension_state(&context.storage_session_id, &migrated)
                .await
                .map_err(|_| "Failed to persist migrated Tasks V2 state".to_string())?;
            session.extension_data = extension_data;
            migrated
        } else {
            TasksStateV2::new(context.scope.into(), context.board_id.0.clone())
        };

        state.scope = context.scope.into();
        if state.board_id.trim().is_empty() {
            state.board_id = context.board_id.0.clone();
        }
        Ok((session, state))
    }

    async fn save_state(
        context: &TaskBoardContext,
        mut session: crate::session::Session,
        state: &TasksStateV2,
    ) -> Result<(), String> {
        state
            .to_extension_data(&mut session.extension_data)
            .map_err(|e| e.to_string())?;
        SessionManager::set_extension_state(&context.storage_session_id, state)
            .await
            .map_err(|_| "Failed to persist task board state".to_string())
    }

    fn snapshot_from_state(state: &TasksStateV2) -> TaskBoardSnapshot {
        TaskBoardSnapshot {
            board_id: TaskBoardId::new(state.board_id.clone()),
            scope: state.scope.into(),
            high_water_mark: state.high_water_mark,
            items: state.items.clone(),
            updated_at: state.updated_at.clone(),
        }
    }

    fn merge_metadata(
        target: &mut HashMap<String, Value>,
        incoming: Option<HashMap<String, Value>>,
    ) {
        if let Some(incoming) = incoming {
            for (key, value) in incoming {
                target.insert(key, value);
            }
        }
    }

    fn demote_other_in_progress_tasks(items: &mut [TaskItem], keep_id: &str) {
        for item in items {
            if item.id != keep_id && item.status == TaskStatus::InProgress {
                item.status = TaskStatus::Pending;
            }
        }
    }

    async fn sync_projection(
        context: &TaskBoardContext,
        local_state: &TasksStateV2,
    ) -> Result<Option<TaskProjectionEvent>, String> {
        let Some(projection_target) = context.projection_target.as_deref() else {
            return Ok(None);
        };
        let logical_worker_id = context
            .logical_worker_id
            .clone()
            .unwrap_or_else(|| context.board_id.0.clone());

        let leader_context = TaskBoardContext {
            board_id: TaskBoardId::new(projection_target.to_string()),
            scope: TaskBoardScope::Leader,
            storage_session_id: projection_target.to_string(),
            projection_target: None,
            logical_worker_id: None,
            attempt_id: None,
            attempt_index: None,
            previous_task_id: None,
        };
        let (leader_session, mut leader_state) = Self::load_state(&leader_context).await?;

        leader_state.items.retain(|item| {
            item.metadata
                .get("projection_source_session_id")
                .and_then(Value::as_str)
                != Some(context.storage_session_id.as_str())
        });

        let mut projected_tasks = Vec::new();
        for item in &local_state.items {
            let projection_ref = ProjectedTaskRef {
                leader_task_id: format!("{}:{}", logical_worker_id, item.id),
                source_task_id: item.id.clone(),
                source_session_id: context.storage_session_id.clone(),
                logical_worker_id: logical_worker_id.clone(),
                attempt_id: context.attempt_id.clone(),
                attempt_index: context.attempt_index,
                previous_task_id: context.previous_task_id.clone(),
            };
            let mut metadata = item.metadata.clone();
            metadata.insert(
                "projection_source_session_id".to_string(),
                Value::String(context.storage_session_id.clone()),
            );
            metadata.insert(
                "projection_source_task_id".to_string(),
                Value::String(item.id.clone()),
            );
            metadata.insert(
                "logical_worker_id".to_string(),
                Value::String(logical_worker_id.clone()),
            );
            if let Some(attempt_id) = context.attempt_id.clone() {
                metadata.insert("attempt_id".to_string(), Value::String(attempt_id));
            }
            if let Some(attempt_index) = context.attempt_index {
                metadata.insert(
                    "attempt_index".to_string(),
                    Value::Number(attempt_index.into()),
                );
            }
            if let Some(previous_task_id) = context.previous_task_id.clone() {
                metadata.insert(
                    "previous_task_id".to_string(),
                    Value::String(previous_task_id),
                );
            }
            metadata.insert(
                "projected_task_ref".to_string(),
                serde_json::to_value(&projection_ref).map_err(|e| e.to_string())?,
            );
            leader_state.items.push(TaskItem {
                id: projection_ref.leader_task_id.clone(),
                subject: item.subject.clone(),
                description: item.description.clone(),
                active_form: item.active_form.clone(),
                owner: Some(logical_worker_id.clone()),
                status: item.status,
                blocks: item
                    .blocks
                    .iter()
                    .map(|value| format!("{}:{}", logical_worker_id, value))
                    .collect(),
                blocked_by: item
                    .blocked_by
                    .iter()
                    .map(|value| format!("{}:{}", logical_worker_id, value))
                    .collect(),
                metadata,
            });
            projected_tasks.push(projection_ref);
        }
        leader_state.touch();
        Self::save_state(&leader_context, leader_session, &leader_state).await?;
        Ok(Some(TaskProjectionEvent {
            board_id: projection_target.to_string(),
            source_scope: context.scope.into(),
            source_session_id: context.storage_session_id.clone(),
            projected_tasks,
            updated_at: leader_state.updated_at.clone(),
        }))
    }

    pub async fn list(context: &TaskBoardContext) -> Result<TaskBoardSnapshot, String> {
        let (_session, state) = Self::load_state(context).await?;
        Ok(Self::snapshot_from_state(&state))
    }

    pub async fn get(
        context: &TaskBoardContext,
        task_id: &str,
    ) -> Result<(TaskBoardSnapshot, Option<TaskItem>), String> {
        let (_session, state) = Self::load_state(context).await?;
        Ok((
            Self::snapshot_from_state(&state),
            state.get_task(task_id).cloned(),
        ))
    }

    pub async fn create_task(
        context: &TaskBoardContext,
        subject: String,
        description: String,
        active_form: String,
        owner: Option<String>,
        metadata: HashMap<String, Value>,
    ) -> Result<TaskBoardMutation, String> {
        let (session, mut state) = Self::load_state(context).await?;
        let task = TaskItem {
            id: state.next_task_id(),
            subject,
            description,
            active_form,
            owner,
            status: TaskStatus::Pending,
            blocks: Vec::new(),
            blocked_by: Vec::new(),
            metadata,
        };
        state.items.push(task.clone());
        Self::save_state(context, session, &state).await?;
        let projection = Self::sync_projection(context, &state).await?;
        Ok(TaskBoardMutation {
            snapshot: Self::snapshot_from_state(&state),
            task,
            projection,
        })
    }

    pub async fn update_task(
        context: &TaskBoardContext,
        update: TaskBoardUpdate,
    ) -> Result<TaskBoardMutation, String> {
        let (session, mut state) = Self::load_state(context).await?;
        let index = state
            .items
            .iter()
            .position(|item| item.id == update.task_id)
            .ok_or_else(|| format!("Task {} not found", update.task_id))?;

        if let Some(subject) = update.subject.as_deref() {
            if !subject.trim().is_empty() {
                state.items[index].subject = subject.trim().to_string();
            }
        }
        if let Some(description) = update.description.as_deref() {
            if !description.trim().is_empty() {
                state.items[index].description = description.trim().to_string();
            }
        }
        if let Some(active_form) = update.active_form.as_deref() {
            if !active_form.trim().is_empty() {
                state.items[index].active_form = active_form.trim().to_string();
            }
        }
        if let Some(owner) = update.owner.as_deref() {
            if !owner.trim().is_empty() {
                state.items[index].owner = Some(owner.trim().to_string());
            }
        }
        if let Some(status) = update.status {
            if status == TaskStatus::InProgress {
                Self::demote_other_in_progress_tasks(&mut state.items, &update.task_id);
            }
            state.items[index].status = status;
        }
        if let Some(add_blocks) = update.add_blocks.as_ref() {
            for task in add_blocks {
                let candidate = task.trim().to_string();
                if !candidate.is_empty()
                    && !state.items[index]
                        .blocks
                        .iter()
                        .any(|item| item == &candidate)
                {
                    state.items[index].blocks.push(candidate);
                }
            }
        }
        if let Some(add_blocked_by) = update.add_blocked_by.as_ref() {
            for task in add_blocked_by {
                let candidate = task.trim().to_string();
                if !candidate.is_empty()
                    && !state.items[index]
                        .blocked_by
                        .iter()
                        .any(|item| item == &candidate)
                {
                    state.items[index].blocked_by.push(candidate);
                }
            }
        }
        Self::merge_metadata(&mut state.items[index].metadata, update.metadata);
        state.touch();
        let task = state.items[index].clone();
        Self::save_state(context, session, &state).await?;
        let projection = Self::sync_projection(context, &state).await?;
        Ok(TaskBoardMutation {
            snapshot: Self::snapshot_from_state(&state),
            task,
            projection,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn task_board_store_persists_standalone_board() {
        let board_id = format!("task-board-test-{}", uuid::Uuid::new_v4());
        let context = TaskBoardContext::standalone(board_id.clone());

        let mutation = TaskBoardStore::create_task(
            &context,
            "Ship Tasks V2".to_string(),
            "Replace legacy todo flow".to_string(),
            "Shipping Tasks V2".to_string(),
            None,
            HashMap::new(),
        )
        .await
        .expect("create task");

        assert_eq!(mutation.snapshot.board_id.as_str(), board_id);
        assert_eq!(mutation.snapshot.items.len(), 1);

        let snapshot = TaskBoardStore::list(&context).await.expect("list tasks");
        assert_eq!(snapshot.items.len(), 1);
        assert_eq!(snapshot.items[0].subject, "Ship Tasks V2");
    }

    #[tokio::test]
    async fn task_board_store_projects_worker_tasks_to_leader_board() {
        let leader_id = format!("leader-board-{}", uuid::Uuid::new_v4());
        let worker_context = TaskBoardContext {
            board_id: TaskBoardId::new(format!("{}:worker-a", leader_id)),
            scope: TaskBoardScope::Worker,
            storage_session_id: format!("{}:worker-a", leader_id),
            projection_target: Some(leader_id.clone()),
            logical_worker_id: Some("worker-a".to_string()),
            attempt_id: Some("attempt-1".to_string()),
            attempt_index: Some(1),
            previous_task_id: None,
        };
        let leader_context = TaskBoardContext {
            board_id: TaskBoardId::new(leader_id.clone()),
            scope: TaskBoardScope::Leader,
            storage_session_id: leader_id.clone(),
            projection_target: None,
            logical_worker_id: None,
            attempt_id: None,
            attempt_index: None,
            previous_task_id: None,
        };

        let mutation = TaskBoardStore::create_task(
            &worker_context,
            "Draft worker output".to_string(),
            "Produce worker-local deliverable".to_string(),
            "Drafting worker output".to_string(),
            Some("worker-a".to_string()),
            HashMap::new(),
        )
        .await
        .expect("create worker task");

        assert!(mutation.projection.is_some());
        let leader_snapshot = TaskBoardStore::list(&leader_context)
            .await
            .expect("list leader board");
        assert_eq!(leader_snapshot.items.len(), 1);
        assert_eq!(leader_snapshot.items[0].id, "worker-a:1");
        assert_eq!(leader_snapshot.items[0].owner.as_deref(), Some("worker-a"));
    }
}
