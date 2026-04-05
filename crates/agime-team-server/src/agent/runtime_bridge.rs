use std::sync::Arc;

use agime::agents::harness::{
    TaskKind, TaskResultEnvelope, TaskRuntimeEvent, TaskSnapshot, TaskStatus,
};
use chrono::Utc;

use super::harness_core::{
    HarnessTurnMode, RunCheckpoint, RunCheckpointKind, RunJournal, RunStatus,
};
use super::service_mongo::AgentService;

pub fn task_status_to_run_status(status: TaskStatus) -> RunStatus {
    match status {
        TaskStatus::Pending => RunStatus::Pending,
        TaskStatus::Running => RunStatus::Executing,
        TaskStatus::Completed => RunStatus::Completed,
        TaskStatus::Failed => RunStatus::Failed,
        TaskStatus::Cancelled => RunStatus::Cancelled,
    }
}

pub fn task_result_to_journal(
    run_id: &str,
    task_node_id: &str,
    result: &TaskResultEnvelope,
) -> RunJournal {
    RunJournal {
        id: None,
        run_id: run_id.to_string(),
        task_node_id: task_node_id.to_string(),
        mode: HarnessTurnMode::Execute,
        tool_calls: 0,
        produced_file_delta: result.produced_delta,
        produced_evidence_delta: false,
        produced_blocker_delta: false,
        reason: Some(result.summary.clone()),
        next_node_id: None,
        created_at: Some(bson::DateTime::from_chrono(Utc::now())),
    }
}

pub fn task_event_to_checkpoint(
    run_id: &str,
    task_graph_id: Option<String>,
    current_node_id: Option<String>,
    event: &TaskRuntimeEvent,
) -> Option<RunCheckpoint> {
    let (status, checkpoint_kind) = match event {
        TaskRuntimeEvent::Started(_) => (RunStatus::Executing, RunCheckpointKind::NodeStart),
        TaskRuntimeEvent::Progress { .. } => return None,
        TaskRuntimeEvent::FollowupRequested { .. } => return None,
        TaskRuntimeEvent::Idle { .. } => return None,
        TaskRuntimeEvent::PermissionRequested { .. } => return None,
        TaskRuntimeEvent::PermissionResolved { .. } => return None,
        TaskRuntimeEvent::PermissionTimedOut { .. } => return None,
        TaskRuntimeEvent::Completed(_) => (RunStatus::Completed, RunCheckpointKind::SettleComplete),
        TaskRuntimeEvent::Failed(_) => (RunStatus::Failed, RunCheckpointKind::RepairStart),
        TaskRuntimeEvent::Cancelled { .. } => (RunStatus::Cancelled, RunCheckpointKind::Manual),
    };

    Some(RunCheckpoint {
        id: None,
        run_id: run_id.to_string(),
        task_graph_id,
        current_node_id,
        checkpoint_kind,
        status,
        lease: None,
        memory: None,
        last_turn_outcome: None,
        created_at: Some(bson::DateTime::from_chrono(Utc::now())),
    })
}

#[derive(Clone)]
pub struct ServerTaskRuntimeObserver {
    agent_service: Arc<AgentService>,
    parent_run_id: String,
    task_graph_id: Option<String>,
    current_node_id: Option<String>,
}

impl ServerTaskRuntimeObserver {
    pub fn new(
        agent_service: Arc<AgentService>,
        parent_run_id: String,
        task_graph_id: Option<String>,
        current_node_id: Option<String>,
    ) -> Self {
        Self {
            agent_service,
            parent_run_id,
            task_graph_id,
            current_node_id,
        }
    }

    async fn mark_started(&self, task_id: &str, kind: TaskKind) {
        match kind {
            TaskKind::Subagent => {
                let _ = self
                    .agent_service
                    .mark_run_subagent_started(&self.parent_run_id, task_id)
                    .await;
            }
            _ => {
                let _ = self
                    .agent_service
                    .mark_run_child_task_started(&self.parent_run_id, task_id)
                    .await;
            }
        }
    }

    async fn mark_finished(&self, task_id: &str, kind: TaskKind) {
        match kind {
            TaskKind::Subagent => {
                let _ = self
                    .agent_service
                    .mark_run_subagent_finished(&self.parent_run_id, task_id)
                    .await;
            }
            _ => {
                let _ = self
                    .agent_service
                    .mark_run_child_task_finished(&self.parent_run_id, task_id)
                    .await;
            }
        }
    }

    pub async fn on_started_snapshot(&self, snapshot: TaskSnapshot) {
        self.mark_started(&snapshot.task_id, snapshot.kind).await;
        if let Some(checkpoint) = task_event_to_checkpoint(
            &self.parent_run_id,
            self.task_graph_id.clone(),
            self.current_node_id.clone(),
            &TaskRuntimeEvent::Started(snapshot),
        ) {
            let _ = self.agent_service.save_run_checkpoint(&checkpoint).await;
        }
    }

    pub async fn on_completed(&self, envelope: TaskResultEnvelope) {
        self.mark_finished(&envelope.task_id, envelope.kind).await;
        let journal = task_result_to_journal(
            &self.parent_run_id,
            self.current_node_id
                .as_deref()
                .unwrap_or("child-task:completed"),
            &envelope,
        );
        let _ = self.agent_service.append_run_journal(&[journal]).await;
        if let Some(checkpoint) = task_event_to_checkpoint(
            &self.parent_run_id,
            self.task_graph_id.clone(),
            self.current_node_id.clone(),
            &TaskRuntimeEvent::Completed(envelope),
        ) {
            let _ = self.agent_service.save_run_checkpoint(&checkpoint).await;
        }
    }

    pub async fn on_failed(&self, envelope: TaskResultEnvelope) {
        self.mark_finished(&envelope.task_id, envelope.kind).await;
        let journal = task_result_to_journal(
            &self.parent_run_id,
            self.current_node_id
                .as_deref()
                .unwrap_or("child-task:failed"),
            &envelope,
        );
        let _ = self.agent_service.append_run_journal(&[journal]).await;
        if let Some(checkpoint) = task_event_to_checkpoint(
            &self.parent_run_id,
            self.task_graph_id.clone(),
            self.current_node_id.clone(),
            &TaskRuntimeEvent::Failed(envelope),
        ) {
            let _ = self.agent_service.save_run_checkpoint(&checkpoint).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agime::agents::harness::{TaskKind, TaskRuntimeEvent, TaskSnapshot};

    #[test]
    fn completed_task_maps_to_completed_run_status() {
        assert_eq!(
            task_status_to_run_status(TaskStatus::Completed),
            RunStatus::Completed
        );
    }

    #[test]
    fn started_event_builds_checkpoint() {
        let event = TaskRuntimeEvent::Started(TaskSnapshot {
            task_id: "task-1".to_string(),
            parent_session_id: "session-1".to_string(),
            depth: 1,
            kind: TaskKind::Subagent,
            status: TaskStatus::Running,
            description: None,
            write_scope: Vec::new(),
            target_artifacts: Vec::new(),
            result_contract: Vec::new(),
            summary: None,
            produced_delta: false,
            accepted_targets: Vec::new(),
            metadata: Default::default(),
            started_at: 0,
            updated_at: 0,
            finished_at: None,
        });
        let checkpoint =
            task_event_to_checkpoint("run-1", None, Some("node-1".to_string()), &event)
                .expect("checkpoint");
        assert_eq!(checkpoint.status, RunStatus::Executing);
    }
}
