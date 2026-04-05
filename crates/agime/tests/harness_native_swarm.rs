use std::time::Duration;

use agime::agents::harness::{
    auto_resolve_request, await_permission_resolution, create_swarm_tool, mailbox_dir,
    mark_all_read, native_swarm_tool_enabled, read_unread_messages, resolve_permission_request,
    write_permission_request, write_to_mailbox, MailboxMessage, MailboxMessageKind,
    PermissionBridgeRequest, SwarmScratchpad, SWARM_TOOL_NAME,
};
use agime::prompt_template::render_global_file;
use rmcp::model::JsonObject;
use serde::Serialize;

#[test]
fn swarm_tool_has_expected_name() {
    let tool = create_swarm_tool();
    assert_eq!(tool.name.as_ref(), SWARM_TOOL_NAME);
}

#[test]
fn swarm_mailbox_round_trip_works() {
    let temp = tempfile::tempdir().expect("tempdir");
    write_to_mailbox(
        temp.path(),
        "run-1",
        "leader",
        MailboxMessage {
            id: "msg-1".to_string(),
            from: "worker-1".to_string(),
            to: "leader".to_string(),
            kind: MailboxMessageKind::Summary,
            text: "worker completed".to_string(),
            summary: Some("summary".to_string()),
            timestamp: String::new(),
            read: false,
            protocol: None,
            metadata: std::collections::HashMap::new(),
        },
    )
    .expect("write mailbox");

    let unread = read_unread_messages(temp.path(), "run-1", "leader").expect("unread");
    assert_eq!(unread.len(), 1);
    assert!(mailbox_dir(temp.path(), "run-1").exists());
    mark_all_read(temp.path(), "run-1", "leader").expect("mark read");
    assert!(read_unread_messages(temp.path(), "run-1", "leader")
        .expect("after")
        .is_empty());
}

#[tokio::test]
async fn swarm_permission_bridge_round_trip_works() {
    let temp = tempfile::tempdir().expect("tempdir");
    let request = PermissionBridgeRequest {
        request_id: "perm-1".to_string(),
        run_id: "run-1".to_string(),
        worker_name: "worker-1".to_string(),
        logical_worker_id: None,
        attempt_id: None,
        attempt_index: None,
        previous_task_id: None,
        tool_name: "write_file".to_string(),
        tool_use_id: "tool-1".to_string(),
        description: "write result".to_string(),
        arguments: JsonObject::default(),
        tool_input_snapshot: None,
        write_scope: vec!["out.md".to_string()],
        validation_mode: false,
        permission_policy: None,
        created_at: chrono::Utc::now().to_rfc3339(),
    };
    write_permission_request(temp.path(), &request).expect("request");
    let resolution = auto_resolve_request(&request);
    resolve_permission_request(temp.path(), "run-1", &resolution).expect("resolve");
    let resolved =
        await_permission_resolution(temp.path(), "run-1", "perm-1", Duration::from_secs(1))
            .await
            .expect("resolved");
    assert_eq!(
        resolved.permission,
        agime::permission::Permission::AllowOnce
    );
}

#[test]
fn swarm_scratchpad_creates_worker_area() {
    let temp = tempfile::tempdir().expect("tempdir");
    let scratchpad = SwarmScratchpad::create(temp.path(), "run-1").expect("scratchpad");
    let worker_note = scratchpad
        .write_worker_note("worker-1", "summary.md", "done")
        .expect("worker note");
    assert!(worker_note.exists());
}

#[test]
fn native_swarm_tool_flag_defaults_off() {
    std::env::remove_var("AGIME_ENABLE_NATIVE_SWARM_TOOL");
    assert!(!native_swarm_tool_enabled());
}

#[derive(Serialize)]
struct LeaderPromptContext {
    delegation_mode: String,
    current_depth: u32,
    max_depth: u32,
    targets: String,
    result_contract: String,
    write_scope: String,
}

#[derive(Serialize)]
struct WorkerPromptContext {
    worker_name: String,
    target_artifact: String,
    result_contract: String,
    write_scope: String,
    scratchpad_root: String,
    mailbox_root: String,
}

#[derive(Serialize)]
struct WorkerFollowupPromptContext {
    worker_name: String,
    target_artifact: String,
    result_contract: String,
    write_scope: String,
    previous_summary: String,
    correction_reason: String,
}

#[test]
fn leader_prompt_renders_target_context() {
    let rendered = render_global_file(
        "leader_system.md",
        &LeaderPromptContext {
            delegation_mode: "Swarm".to_string(),
            current_depth: 0,
            max_depth: 1,
            targets: "docs/a.md, docs/b.md".to_string(),
            result_contract: "docs/a.md, docs/b.md".to_string(),
            write_scope: "docs".to_string(),
        },
    )
    .expect("render");
    assert!(rendered.contains("docs/a.md"));
    assert!(rendered.contains("Swarm"));
}

#[test]
fn worker_prompt_renders_worker_identity() {
    let rendered = render_global_file(
        "worker_system.md",
        &WorkerPromptContext {
            worker_name: "worker_1_docs".to_string(),
            target_artifact: "docs/out.md".to_string(),
            result_contract: "docs/out.md".to_string(),
            write_scope: "docs".to_string(),
            scratchpad_root: "/tmp/scratchpad".to_string(),
            mailbox_root: "/tmp/mailbox".to_string(),
        },
    )
    .expect("render");
    assert!(rendered.contains("worker_1_docs"));
    assert!(rendered.contains("docs/out.md"));
}

#[test]
fn worker_continue_prompt_renders_previous_summary() {
    let rendered = render_global_file(
        "worker_continue.md",
        &WorkerFollowupPromptContext {
            worker_name: "worker_1_docs".to_string(),
            target_artifact: "docs/out.md".to_string(),
            result_contract: "docs/out.md".to_string(),
            write_scope: "docs".to_string(),
            previous_summary: "partial update completed".to_string(),
            correction_reason: "needs more progress".to_string(),
        },
    )
    .expect("render");
    assert!(rendered.contains("partial update completed"));
    assert!(rendered.contains("docs/out.md"));
}

#[test]
fn worker_correction_prompt_renders_reason() {
    let rendered = render_global_file(
        "worker_correction.md",
        &WorkerFollowupPromptContext {
            worker_name: "worker_1_docs".to_string(),
            target_artifact: "docs/out.md".to_string(),
            result_contract: "docs/out.md".to_string(),
            write_scope: "docs".to_string(),
            previous_summary: "FAIL: missing section".to_string(),
            correction_reason: "validation failed".to_string(),
        },
    )
    .expect("render");
    assert!(rendered.contains("validation failed"));
    assert!(rendered.contains("FAIL: missing section"));
}

#[test]
fn verification_worker_prompt_renders_general_execution_guidance() {
    let rendered = render_global_file(
        "verification_worker.md",
        &WorkerPromptContext {
            worker_name: "validator_1_docs".to_string(),
            target_artifact: "docs/out.md".to_string(),
            result_contract: "docs/out.md".to_string(),
            write_scope: "none".to_string(),
            scratchpad_root: "/tmp/scratchpad".to_string(),
            mailbox_root: "/tmp/mailbox".to_string(),
        },
    )
    .expect("render");
    assert!(rendered.contains("PASS:"));
    assert!(rendered.contains("general execution tasks"));
}
