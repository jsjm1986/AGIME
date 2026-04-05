use agime::agents::harness::{
    TaskKind, TaskResultEnvelope, TaskRuntime, TaskRuntimeEvent, TaskRuntimeHost, TaskSpec,
    TaskStatus,
};

#[tokio::test]
async fn task_runtime_tracks_lifecycle_and_progress() {
    let runtime = TaskRuntime::default();
    let spec = TaskSpec {
        task_id: "task-1".to_string(),
        parent_session_id: "session-1".to_string(),
        depth: 1,
        kind: TaskKind::Subagent,
        description: Some("test subagent".to_string()),
        write_scope: vec!["src/out.rs".to_string()],
        target_artifacts: vec!["src/out.rs".to_string()],
        result_contract: vec!["src/out.rs".to_string()],
        metadata: Default::default(),
    };

    let _handle = runtime.spawn_task(spec).await.expect("spawn should work");
    runtime
        .record_progress("task-1", "halfway", Some(50))
        .await
        .expect("progress should work");
    runtime
        .complete(TaskResultEnvelope {
            task_id: "task-1".to_string(),
            kind: TaskKind::Subagent,
            status: TaskStatus::Completed,
            summary: "done".to_string(),
            accepted_targets: vec!["src/out.rs".to_string()],
            produced_delta: true,
            metadata: Default::default(),
        })
        .await
        .expect("complete should work");

    let snapshot = runtime.snapshot("task-1").await.expect("snapshot exists");
    assert_eq!(snapshot.status, TaskStatus::Completed);
    assert!(snapshot.produced_delta);
    assert_eq!(snapshot.summary.as_deref(), Some("done"));
}

#[tokio::test]
async fn task_runtime_emits_cancel_event() {
    let runtime = TaskRuntime::default();
    let spec = TaskSpec {
        task_id: "task-2".to_string(),
        parent_session_id: "session-1".to_string(),
        depth: 1,
        kind: TaskKind::ValidationWorker,
        description: None,
        write_scope: Vec::new(),
        target_artifacts: vec!["README.md".to_string()],
        result_contract: vec!["README.md".to_string()],
        metadata: Default::default(),
    };

    runtime.spawn_task(spec).await.expect("spawn should work");
    let mut rx = runtime.subscribe("task-2").expect("receiver");
    runtime.cancel("task-2").await.expect("cancel should work");

    let mut saw_cancel = false;
    for _ in 0..3 {
        match rx.recv().await.expect("event") {
            TaskRuntimeEvent::Cancelled { task_id } if task_id == "task-2" => {
                saw_cancel = true;
                break;
            }
            _ => {}
        }
    }
    assert!(saw_cancel);
}

#[tokio::test]
async fn task_runtime_cancelled_task_ignores_late_completion() {
    let runtime = TaskRuntime::default();
    let spec = TaskSpec {
        task_id: "task-cancel-late".to_string(),
        parent_session_id: "session-1".to_string(),
        depth: 1,
        kind: TaskKind::Subagent,
        description: Some("cancel me".to_string()),
        write_scope: vec!["src/out.rs".to_string()],
        target_artifacts: vec!["src/out.rs".to_string()],
        result_contract: vec!["src/out.rs".to_string()],
        metadata: Default::default(),
    };

    runtime.spawn_task(spec).await.expect("spawn should work");
    runtime
        .cancel("task-cancel-late")
        .await
        .expect("cancel should work");
    runtime
        .complete(TaskResultEnvelope {
            task_id: "task-cancel-late".to_string(),
            kind: TaskKind::Subagent,
            status: TaskStatus::Completed,
            summary: "late completion".to_string(),
            accepted_targets: vec!["src/out.rs".to_string()],
            produced_delta: true,
            metadata: Default::default(),
        })
        .await
        .expect("late completion should be ignored");

    let snapshot = runtime
        .snapshot("task-cancel-late")
        .await
        .expect("snapshot exists");
    assert_eq!(snapshot.status, TaskStatus::Cancelled);
    assert_eq!(snapshot.summary, None);
    assert!(!snapshot.produced_delta);
    assert!(snapshot.accepted_targets.is_empty());
}

#[tokio::test]
async fn task_runtime_global_subscription_receives_worker_events() {
    let runtime = TaskRuntime::default();
    let spec = TaskSpec {
        task_id: "task-3".to_string(),
        parent_session_id: "session-2".to_string(),
        depth: 1,
        kind: TaskKind::SwarmWorker,
        description: Some("swarm worker".to_string()),
        write_scope: vec!["out.md".to_string()],
        target_artifacts: vec!["out.md".to_string()],
        result_contract: vec!["out.md".to_string()],
        metadata: Default::default(),
    };

    let mut rx = runtime.subscribe_all();
    runtime.spawn_task(spec).await.expect("spawn should work");
    runtime
        .record_progress("task-3", "running", Some(25))
        .await
        .expect("progress should work");

    let mut saw_start = false;
    let mut saw_progress = false;
    for _ in 0..4 {
        match rx.recv().await.expect("event") {
            TaskRuntimeEvent::Started(snapshot) if snapshot.task_id == "task-3" => {
                saw_start = true;
            }
            TaskRuntimeEvent::Progress { task_id, .. } if task_id == "task-3" => {
                saw_progress = true;
            }
            _ => {}
        }
        if saw_start && saw_progress {
            break;
        }
    }

    assert!(saw_start);
    assert!(saw_progress);
}

#[tokio::test]
async fn task_runtime_emits_permission_events() {
    let runtime = TaskRuntime::default();
    let spec = TaskSpec {
        task_id: "task-4".to_string(),
        parent_session_id: "session-2".to_string(),
        depth: 1,
        kind: TaskKind::SwarmWorker,
        description: Some("swarm worker".to_string()),
        write_scope: vec!["out.md".to_string()],
        target_artifacts: vec!["out.md".to_string()],
        result_contract: vec!["out.md".to_string()],
        metadata: Default::default(),
    };

    let mut rx = runtime.subscribe_all();
    runtime.spawn_task(spec).await.expect("spawn should work");
    runtime
        .record_permission_requested("task-4", Some("worker-1".to_string()), "write_file")
        .await
        .expect("request");
    runtime
        .record_permission_resolved(
            "task-4",
            Some("worker-1".to_string()),
            "write_file",
            "AllowOnce",
        )
        .await
        .expect("resolve");

    let mut saw_request = false;
    let mut saw_resolution = false;
    for _ in 0..6 {
        match rx.recv().await.expect("event") {
            TaskRuntimeEvent::PermissionRequested {
                task_id, tool_name, ..
            } if task_id == "task-4" && tool_name == "write_file" => {
                saw_request = true;
            }
            TaskRuntimeEvent::PermissionResolved {
                task_id,
                tool_name,
                decision,
                ..
            } if task_id == "task-4" && tool_name == "write_file" && decision == "AllowOnce" => {
                saw_resolution = true;
            }
            _ => {}
        }
        if saw_request && saw_resolution {
            break;
        }
    }

    assert!(saw_request);
    assert!(saw_resolution);
}

#[tokio::test]
async fn task_runtime_emits_followup_idle_and_permission_timeout_events() {
    let runtime = TaskRuntime::default();
    let spec = TaskSpec {
        task_id: "task-5".to_string(),
        parent_session_id: "session-3".to_string(),
        depth: 1,
        kind: TaskKind::SwarmWorker,
        description: Some("swarm worker".to_string()),
        write_scope: vec!["out.md".to_string()],
        target_artifacts: vec!["out.md".to_string()],
        result_contract: vec!["out.md".to_string()],
        metadata: Default::default(),
    };

    let mut rx = runtime.subscribe_all();
    runtime.spawn_task(spec).await.expect("spawn should work");
    runtime
        .record_followup_requested("task-5", "correction", "validation failed")
        .await
        .expect("followup");
    runtime
        .record_idle("task-5", "waiting for leader permission")
        .await
        .expect("idle");
    runtime
        .record_permission_timed_out("task-5", Some("worker-5".to_string()), "write_file", 60000)
        .await
        .expect("timed out");

    let mut saw_followup = false;
    let mut saw_idle = false;
    let mut saw_timeout = false;
    for _ in 0..8 {
        match rx.recv().await.expect("event") {
            TaskRuntimeEvent::FollowupRequested { task_id, kind, .. }
                if task_id == "task-5" && kind == "correction" =>
            {
                saw_followup = true;
            }
            TaskRuntimeEvent::Idle { task_id, .. } if task_id == "task-5" => {
                saw_idle = true;
            }
            TaskRuntimeEvent::PermissionTimedOut {
                task_id,
                tool_name,
                timeout_ms,
                ..
            } if task_id == "task-5" && tool_name == "write_file" && timeout_ms == 60000 => {
                saw_timeout = true;
            }
            _ => {}
        }
        if saw_followup && saw_idle && saw_timeout {
            break;
        }
    }

    assert!(saw_followup);
    assert!(saw_idle);
    assert!(saw_timeout);
}
