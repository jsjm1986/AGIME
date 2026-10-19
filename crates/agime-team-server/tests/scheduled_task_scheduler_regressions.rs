//! Mongo-backed scheduler lifecycle and fencing regression tests.
//!
//! These tests intentionally require a reachable disposable MongoDB. They do
//! not skip when MongoDB is unavailable, so CI cannot silently lose coverage.

use std::sync::Arc;

use agime_team::MongoDb;
use agime_team_server::scheduled_tasks::models::{
    ScheduledTaskRunOutcomeReason, ScheduledTaskRunStatus,
};
use agime_team_server::scheduled_tasks::service::ScheduledTaskService;
use chrono::Utc;
use mongodb::bson::{doc, Document};
use serde_json::json;
use uuid::Uuid;

fn mongo_uri() -> String {
    std::env::var("AGIME_TEST_MONGO_URI")
        .unwrap_or_else(|_| "mongodb://127.0.0.1:27017".to_string())
}

async fn test_database() -> (Arc<MongoDb>, String) {
    let uri = mongo_uri();
    let db_name = format!("agime_scheduler_regression_{}", Uuid::new_v4().simple());
    let db = MongoDb::connect(&uri, &db_name)
        .await
        .unwrap_or_else(|error| panic!("MongoDB regression tests require a reachable disposable MongoDB at {uri}: {error}"));
    (Arc::new(db), db_name)
}

fn task_document(task_id: &str, next_fire_at: mongodb::bson::DateTime) -> Document {
    mongodb::bson::to_document(&json!({
        "task_id": task_id,
        "team_id": "regression-team",
        "channel_id": "regression-channel",
        "owner_user_id": "regression-user",
        "agent_id": "regression-agent",
        "title": "lease regression",
        "prompt": "deterministic scheduler regression",
        "task_kind": "one_shot",
        "task_profile": "workspace_task",
        "payload_kind": "system_summary",
        "session_binding": "isolated_task",
        "delivery_plan": "channel_only",
        "execution_contract": {
            "output_mode": "summary_only",
            "must_return_final_text": true,
            "allow_partial_result": true,
            "publish_behavior": "none",
            "source_scope": "workspace_only",
            "required_sections": []
        },
        "delivery_tier": "durable",
        "timezone": "UTC",
        "status": "active",
        "next_fire_at": next_fire_at,
        "missed_fire_count": 0,
        "lease_owner": null,
        "lease_expires_at": null,
        "created_at": next_fire_at,
        "updated_at": next_fire_at
    }))
    .expect("task fixture must serialize")
}

#[tokio::test]
async fn scheduler_claim_is_atomic_and_stale_release_is_fenced() {
    let (db, db_name) = test_database().await;
    let service = ScheduledTaskService::new(db.clone());
    let now = mongodb::bson::DateTime::from_chrono(Utc::now() - chrono::Duration::seconds(1));
    let task_id = format!("task-{}", Uuid::new_v4());
    db.collection::<Document>("chat_channels")
        .insert_one(doc! { "channel_id": "regression-channel" }, None)
        .await
        .expect("channel fixture insert must succeed");
    db.collection::<Document>("scheduled_tasks")
        .insert_one(task_document(&task_id, now), None)
        .await
        .expect("task fixture insert must succeed");

    let first = service
        .claim_due_tasks(Utc::now(), "owner-a", 120, 1)
        .await
        .expect("first claim must succeed");
    assert_eq!(first.len(), 1, "exactly one scheduler should claim the due task");

    let second = service
        .claim_due_tasks(Utc::now(), "owner-b", 120, 1)
        .await
        .expect("second claim must succeed");
    assert!(second.is_empty(), "a live lease must fence competing schedulers");

    service
        .release_lease("regression-team", &task_id, "owner-b")
        .await
        .expect("stale release request must not error");
    let after_stale_release = db
        .collection::<Document>("scheduled_tasks")
        .find_one(doc! { "task_id": &task_id }, None)
        .await
        .expect("task read must succeed")
        .expect("task must exist");
    assert_eq!(after_stale_release.get_str("lease_owner").unwrap(), "owner-a");

    service
        .release_lease("regression-team", &task_id, "owner-a")
        .await
        .expect("owner release must succeed");
    let after_owner_release = db
        .collection::<Document>("scheduled_tasks")
        .find_one(doc! { "task_id": &task_id }, None)
        .await
        .expect("task read must succeed")
        .expect("task must exist");
    assert!(after_owner_release.get("lease_owner").is_none() || after_owner_release.get("lease_owner").unwrap().is_null());

    db.db().drop(None).await.expect("disposable test database cleanup must succeed");
    let _ = db_name;
}

#[tokio::test]
async fn scheduler_completion_cannot_clear_a_newer_owner_lease() {
    let (db, _db_name) = test_database().await;
    let service = ScheduledTaskService::new(db.clone());
    let now = mongodb::bson::DateTime::from_chrono(Utc::now() - chrono::Duration::seconds(1));
    let task_id = format!("task-{}", Uuid::new_v4());
    db.collection::<Document>("chat_channels")
        .insert_one(doc! { "channel_id": "regression-channel" }, None)
        .await
        .expect("channel fixture insert must succeed");
    db.collection::<Document>("scheduled_tasks")
        .insert_one(task_document(&task_id, now), None)
        .await
        .expect("task fixture insert must succeed");

    let mut claimed = service
        .claim_due_tasks(Utc::now(), "owner-a", 120, 1)
        .await
        .expect("claim must succeed");
    let task = claimed.pop().expect("task must be claimed");
    let run = service
        .create_run(&task, "scheduler", "message-1", "session-1", None)
        .await
        .expect("run creation must succeed");

    service
        .complete_run(
            &task,
            &run.run_id,
            ScheduledTaskRunStatus::Completed,
            Some(ScheduledTaskRunOutcomeReason::Completed),
            0,
            Some("done".to_string()),
            None,
            None,
            None,
            "owner-b",
        )
        .await
        .expect("stale completion must not error");

    let task_after = db
        .collection::<Document>("scheduled_tasks")
        .find_one(doc! { "task_id": &task_id }, None)
        .await
        .expect("task read must succeed")
        .expect("task must exist");
    assert_eq!(task_after.get_str("lease_owner").unwrap(), "owner-a");
    assert_eq!(task_after.get_str("status").unwrap(), "active");

    db.db().drop(None).await.expect("disposable test database cleanup must succeed");
}
