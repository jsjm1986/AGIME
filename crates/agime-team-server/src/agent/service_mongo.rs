//! Agent service layer for business logic (MongoDB version)

use super::mission_mongo::{
    resolve_execution_profile, AttemptRecord, CreateMissionRequest, GoalNode, GoalStatus,
    ListMissionsQuery, MissionArtifactDoc, MissionDoc, MissionEventDoc, MissionListItem,
    MissionStatus, MissionStep, ProgressSignal, RuntimeContract, RuntimeContractVerification,
    StepStatus, StepSupervisorState,
};
use super::normalize_workspace_path;
use super::session_mongo::{
    AgentSessionDoc, ChatEventDoc, CreateSessionRequest, SessionListItem, SessionListQuery,
    UserSessionListQuery,
};
use super::task_manager::StreamEvent;
use agime::agents::types::RetryConfig;
use agime_team::models::mongo::Document as TeamDocument;
use agime_team::models::{
    AgentExtensionConfig, AgentSkillConfig, AgentStatus, AgentTask, ApiFormat, BuiltinExtension,
    CreateAgentRequest, CustomExtensionConfig, ListAgentsQuery, ListTasksQuery, PaginatedResponse,
    SubmitTaskRequest, TaskResult, TaskResultType, TaskStatus, TaskType, TeamAgent,
    UpdateAgentRequest,
};
use agime_team::MongoDb;
use chrono::{DateTime, Utc};
use futures::TryStreamExt;
use mongodb::bson::{doc, oid::ObjectId, Bson, Document};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use uuid::Uuid;

/// Custom serde module for Option<DateTime<Utc>> with BSON datetime
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

/// Validation error for agent operations
#[derive(Debug, thiserror::Error)]
pub enum ValidationError {
    #[error("Name is required and must be 1-100 characters")]
    Name,
    #[error("Invalid API URL format")]
    ApiUrl,
    #[error("Model name must be 1-100 characters")]
    Model,
    #[error("Priority must be between 0 and 100")]
    Priority,
    #[error("Invalid extension config: missing uri_or_cmd (or legacy uriOrCmd/command)")]
    ExtensionConfig,
}

/// Service error that includes both database and validation errors
#[derive(Debug, thiserror::Error)]
pub enum ServiceError {
    #[error("Database error: {0}")]
    Database(#[from] mongodb::error::Error),
    #[error("Validation error: {0}")]
    Validation(#[from] ValidationError),
    #[error("Internal error: {0}")]
    Internal(String),
}

/// Validate agent name
fn validate_name(name: &str) -> Result<(), ValidationError> {
    let trimmed = name.trim();
    if trimmed.is_empty() || trimmed.len() > 100 {
        return Err(ValidationError::Name);
    }
    Ok(())
}

/// Validate API URL format
fn validate_api_url(url: &Option<String>) -> Result<(), ValidationError> {
    if let Some(ref u) = url {
        let trimmed = u.trim();
        if !trimmed.is_empty()
            && !trimmed.starts_with("http://")
            && !trimmed.starts_with("https://")
        {
            return Err(ValidationError::ApiUrl);
        }
    }
    Ok(())
}

/// Validate model name
fn validate_model(model: &Option<String>) -> Result<(), ValidationError> {
    if let Some(ref m) = model {
        let trimmed = m.trim();
        if trimmed.len() > 100 {
            return Err(ValidationError::Model);
        }
    }
    Ok(())
}

/// Validate priority
fn validate_priority(priority: i32) -> Result<(), ValidationError> {
    if !(0..=100).contains(&priority) {
        return Err(ValidationError::Priority);
    }
    Ok(())
}

/// Serialize custom extension configs for MongoDB persistence.
/// We build BSON explicitly so secret envs are stored in DB even though
/// API responses keep envs redacted via `skip_serializing`.
fn custom_extension_to_bson_document(ext: &CustomExtensionConfig) -> Document {
    let args = ext
        .args
        .iter()
        .map(|arg| Bson::String(arg.clone()))
        .collect::<Vec<_>>();

    let envs = ext
        .envs
        .iter()
        .map(|(k, v)| (k.clone(), Bson::String(v.clone())))
        .collect::<Document>();

    let mut doc = doc! {
        "name": ext.name.clone(),
        "type": ext.ext_type.clone(),
        "uri_or_cmd": ext.uri_or_cmd.clone(),
        "args": args,
        "envs": envs,
        "enabled": ext.enabled,
    };

    if let Some(source) = &ext.source {
        doc.insert("source", source.clone());
    }
    if let Some(source_extension_id) = &ext.source_extension_id {
        doc.insert("source_extension_id", source_extension_id.clone());
    }

    doc
}

fn custom_extensions_to_bson(exts: &[CustomExtensionConfig]) -> Bson {
    Bson::Array(
        exts.iter()
            .map(|ext| Bson::Document(custom_extension_to_bson_document(ext)))
            .collect::<Vec<_>>(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::mission_mongo::{ApprovalPolicy, ExecutionMode, ExecutionProfile};
    use mongodb::bson::doc;
    use serde_json::json;
    use std::collections::HashMap;

    fn sample_mission_doc() -> MissionDoc {
        MissionDoc {
            id: None,
            mission_id: "mission-sample".to_string(),
            team_id: "team-1".to_string(),
            agent_id: "agent-1".to_string(),
            creator_id: "user-1".to_string(),
            goal: "Sample goal".to_string(),
            context: None,
            approval_policy: ApprovalPolicy::Auto,
            status: MissionStatus::Draft,
            steps: vec![MissionStep {
                index: 0,
                title: "Step 1".to_string(),
                description: "desc".to_string(),
                status: StepStatus::Pending,
                is_checkpoint: false,
                approved_by: None,
                started_at: None,
                completed_at: None,
                error_message: None,
                supervisor_state: None,
                last_activity_at: None,
                last_progress_at: None,
                progress_score: None,
                current_blocker: None,
                last_supervisor_hint: None,
                stall_count: 0,
                recent_progress_events: Vec::new(),
                evidence_bundle: None,
                tokens_used: 0,
                output_summary: None,
                retry_count: 0,
                max_retries: 2,
                timeout_seconds: None,
                required_artifacts: Vec::new(),
                completion_checks: Vec::new(),
                runtime_contract: None,
                contract_verification: None,
                use_subagent: false,
                tool_calls: Vec::new(),
            }],
            current_step: Some(0),
            session_id: Some("session-1".to_string()),
            source_chat_session_id: None,
            token_budget: 0,
            total_tokens_used: 0,
            priority: 0,
            step_timeout_seconds: None,
            step_max_retries: None,
            plan_version: 1,
            execution_mode: ExecutionMode::Sequential,
            execution_profile: ExecutionProfile::Auto,
            goal_tree: None,
            current_goal_id: None,
            total_pivots: 0,
            total_abandoned: 0,
            error_message: None,
            final_summary: None,
            created_at: bson::DateTime::now(),
            updated_at: bson::DateTime::now(),
            started_at: None,
            completed_at: None,
            attached_document_ids: Vec::new(),
            workspace_path: Some("/tmp/workspace".to_string()),
            current_run_id: Some("run-1".to_string()),
        }
    }

    #[test]
    fn custom_extension_bson_keeps_envs_and_type_field() {
        let mut envs = HashMap::new();
        envs.insert("API_TOKEN".to_string(), "secret-token".to_string());

        let ext = CustomExtensionConfig {
            name: "demo_mcp".to_string(),
            ext_type: "stdio".to_string(),
            uri_or_cmd: "demo-cmd".to_string(),
            args: vec!["--foo".to_string()],
            envs,
            enabled: true,
            source: Some("team".to_string()),
            source_extension_id: Some("ext-id".to_string()),
        };

        let doc = custom_extension_to_bson_document(&ext);
        let env_doc = doc.get_document("envs").expect("envs must exist");
        assert_eq!(doc.get_str("type").unwrap_or_default(), "stdio");
        assert_eq!(
            env_doc.get_str("API_TOKEN").unwrap_or_default(),
            "secret-token"
        );
    }

    #[test]
    fn governance_config_prefers_top_level_config_over_nested_config() {
        let doc = doc! {
            "settings": {
                "digitalAvatarGovernanceConfig": {
                    "autoProposalTriggerCount": 5,
                    "managerApprovalMode": "human_gate",
                },
                "digitalAvatarGovernance": {
                    "config": {
                        "autoProposalTriggerCount": 9,
                        "managerApprovalMode": "manager_decides",
                    }
                }
            }
        };

        let config = AgentService::governance_config_json_from_portal_doc(&doc);
        assert_eq!(config.get("autoProposalTriggerCount"), Some(&json!(5)));
        assert_eq!(
            config.get("managerApprovalMode"),
            Some(&json!("human_gate"))
        );
    }

    #[test]
    fn governance_config_falls_back_to_nested_config_when_top_level_missing() {
        let doc = doc! {
            "settings": {
                "digitalAvatarGovernance": {
                    "config": {
                        "autoProposalTriggerCount": 7,
                        "optimizationMode": "manager_only",
                    }
                }
            }
        };

        let config = AgentService::governance_config_json_from_portal_doc(&doc);
        assert_eq!(config.get("autoProposalTriggerCount"), Some(&json!(7)));
        assert_eq!(config.get("optimizationMode"), Some(&json!("manager_only")));
    }

    #[test]
    fn governance_config_uses_defaults_when_no_config_exists() {
        let doc = doc! {
            "settings": {
                "domain": "avatar"
            }
        };

        let config = AgentService::governance_config_json_from_portal_doc(&doc);
        assert_eq!(config, AgentService::default_governance_config_json());
    }

    #[test]
    fn governance_state_uses_defaults_when_state_missing() {
        let doc = doc! {
            "settings": {
                "domain": "avatar"
            }
        };

        let state = AgentService::governance_state_json_from_portal_doc(&doc);
        assert_eq!(state, AgentService::default_governance_state_json());
    }

    #[test]
    fn governance_state_strips_nested_config_from_legacy_governance_object() {
        let doc = doc! {
            "settings": {
                "digitalAvatarGovernance": {
                    "config": {
                        "autoProposalTriggerCount": 7
                    },
                    "capabilityRequests": [
                        { "id": "cap-1", "status": "pending" }
                    ]
                }
            }
        };

        let state = AgentService::governance_state_json_from_portal_doc(&doc);
        assert_eq!(
            state,
            json!({
                "capabilityRequests": [
                    { "id": "cap-1", "status": "pending" }
                ]
            })
        );
        assert_eq!(
            AgentService::raw_portal_governance_state_json(&doc),
            Some(json!({
                "capabilityRequests": [
                    { "id": "cap-1", "status": "pending" }
                ]
            }))
        );
    }

    #[test]
    fn governance_counts_only_include_pending_statuses_used_by_current_ui() {
        let state = json!({
            "capabilityRequests": [
                { "id": "cap-1", "status": "pending" },
                { "id": "cap-2", "status": "approved" }
            ],
            "gapProposals": [
                { "id": "proposal-1", "status": "pending_approval" },
                { "id": "proposal-2", "status": "approved" }
            ],
            "optimizationTickets": [
                { "id": "ticket-1", "status": "pending" },
                { "id": "ticket-2", "status": "deployed" }
            ],
            "runtimeLogs": [
                { "id": "runtime-1", "status": "pending" },
                { "id": "runtime-2", "status": "dismissed" }
            ]
        });

        let counts = AgentService::governance_counts_from_state_json(&state);
        assert_eq!(counts.pending_capability_requests, 1);
        assert_eq!(counts.pending_gap_proposals, 1);
        assert_eq!(counts.pending_optimization_tickets, 1);
        assert_eq!(counts.pending_runtime_logs, 1);
    }

    #[test]
    fn governance_event_payload_from_doc_uses_synthetic_id_when_object_id_missing() {
        let created_at = Utc::now();
        let payload = AvatarGovernanceEventPayload::from(AvatarGovernanceEventDoc {
            id: None,
            portal_id: "portal-1".to_string(),
            team_id: "team-1".to_string(),
            event_type: "created".to_string(),
            entity_type: "capability".to_string(),
            entity_id: Some("cap-1".to_string()),
            title: "新增能力请求".to_string(),
            status: Some("pending".to_string()),
            detail: None,
            actor_id: None,
            actor_name: Some("system".to_string()),
            meta: json!({ "risk": "medium" }),
            created_at,
        });

        assert!(payload
            .event_id
            .starts_with("synthetic:team-1:portal-1:created:capability:cap-1:"));
    }

    #[test]
    fn governance_event_payloads_from_snapshot_derive_created_and_config_events() {
        let created_at = Utc::now();
        let state = json!({
            "capabilityRequests": [
                {
                    "id": "cap-1",
                    "title": "补充 CRM 查询能力",
                    "status": "pending",
                    "detail": "需要访问外部 CRM",
                    "risk": "medium"
                }
            ]
        });
        let config = json!({
            "autoProposalTriggerCount": 3
        });

        let payloads = AgentService::governance_event_payloads_from_snapshot(
            "team-1", "portal-1", &state, &config, created_at,
        );

        assert_eq!(payloads.len(), 2);
        assert!(payloads
            .iter()
            .any(|item| item.event_type == "created" && item.entity_type == "capability"));
        assert!(payloads
            .iter()
            .any(|item| item.event_type == "config_updated" && item.entity_type == "config"));
        assert!(payloads.iter().all(|item| item.created_at == created_at));
        assert!(payloads
            .iter()
            .all(|item| item.event_id.starts_with("synthetic:")));
    }

    #[test]
    fn merge_avatar_workbench_reports_keeps_latest_and_preserves_persisted_actions() {
        let now = Utc::now();
        let derived = vec![
            AvatarWorkbenchReportItemPayload {
                id: "shared".to_string(),
                ts: now,
                kind: "governance".to_string(),
                title: "派生治理汇总".to_string(),
                summary: "derived".to_string(),
                status: "pending".to_string(),
                source: "system".to_string(),
                recommendation: None,
                action_kind: Some("open_governance".to_string()),
                action_target_id: Some("portal-1".to_string()),
                work_objects: Vec::new(),
                outputs: Vec::new(),
                needs_decision: true,
            },
            AvatarWorkbenchReportItemPayload {
                id: "derived-only".to_string(),
                ts: now - chrono::Duration::minutes(1),
                kind: "runtime".to_string(),
                title: "派生运行汇总".to_string(),
                summary: "runtime".to_string(),
                status: "logged".to_string(),
                source: "system".to_string(),
                recommendation: None,
                action_kind: Some("open_logs".to_string()),
                action_target_id: Some("portal-1".to_string()),
                work_objects: Vec::new(),
                outputs: Vec::new(),
                needs_decision: false,
            },
        ];
        let persisted = vec![
            AvatarWorkbenchReportItemPayload {
                id: "manual".to_string(),
                ts: now + chrono::Duration::minutes(2),
                kind: "note".to_string(),
                title: "人工备注".to_string(),
                summary: "manual".to_string(),
                status: "active".to_string(),
                source: "manager".to_string(),
                recommendation: Some("follow".to_string()),
                action_kind: Some("open_governance".to_string()),
                action_target_id: Some("portal-1".to_string()),
                work_objects: Vec::new(),
                outputs: Vec::new(),
                needs_decision: false,
            },
            AvatarWorkbenchReportItemPayload {
                id: "shared".to_string(),
                ts: now - chrono::Duration::minutes(5),
                kind: "note".to_string(),
                title: "旧派生副本".to_string(),
                summary: "old".to_string(),
                status: "archived".to_string(),
                source: "legacy".to_string(),
                recommendation: None,
                action_kind: None,
                action_target_id: None,
                work_objects: Vec::new(),
                outputs: Vec::new(),
                needs_decision: false,
            },
        ];

        let merged = AgentService::merge_avatar_workbench_reports(&derived, &persisted, 4);

        assert_eq!(merged.len(), 3);
        assert_eq!(merged[0].id, "manual");
        assert!(merged.iter().any(|item| item.id == "derived-only"));
        assert_eq!(
            merged
                .iter()
                .find(|item| item.id == "shared")
                .map(|item| item.summary.as_str()),
            Some("derived")
        );
    }

    #[test]
    fn governance_divergence_classifies_default_only_when_both_sources_absent() {
        let kind = AgentService::classify_governance_divergence(false, false, false, false);
        assert!(matches!(kind, AvatarGovernanceDivergenceKind::DefaultOnly));
    }

    #[test]
    fn governance_divergence_classifies_differing_when_config_mismatches() {
        let kind = AgentService::classify_governance_divergence(true, true, true, false);
        assert!(matches!(kind, AvatarGovernanceDivergenceKind::Differing));
    }

    #[test]
    fn normalize_optional_scope_id_treats_blank_as_none() {
        assert_eq!(AgentService::normalize_optional_scope_id(None), None);
        assert_eq!(AgentService::normalize_optional_scope_id(Some("   ")), None);
        assert_eq!(
            AgentService::normalize_optional_scope_id(Some("  team-1  ")),
            Some("team-1".to_string())
        );
    }

    #[test]
    fn index_governance_state_docs_reports_duplicates_and_keeps_latest() {
        let older = Utc::now();
        let newer = older + chrono::Duration::minutes(5);
        let docs = vec![
            AvatarGovernanceStateDoc {
                id: None,
                portal_id: "portal-1".to_string(),
                team_id: "team-1".to_string(),
                state: json!({ "version": 1 }),
                config: json!({ "mode": "older" }),
                updated_at: older,
            },
            AvatarGovernanceStateDoc {
                id: None,
                portal_id: "portal-1".to_string(),
                team_id: "team-1".to_string(),
                state: json!({ "version": 2 }),
                config: json!({ "mode": "newer" }),
                updated_at: newer,
            },
            AvatarGovernanceStateDoc {
                id: None,
                portal_id: "portal-2".to_string(),
                team_id: "team-1".to_string(),
                state: json!({ "version": 3 }),
                config: json!({ "mode": "single" }),
                updated_at: older,
            },
        ];

        let (indexed, counts, duplicate_rows, duplicate_state_docs) =
            AgentService::index_governance_state_docs(docs);

        assert_eq!(indexed.len(), 2);
        assert_eq!(duplicate_state_docs, 1);
        assert_eq!(
            counts.get(&("team-1".to_string(), "portal-1".to_string())),
            Some(&2)
        );
        assert_eq!(duplicate_rows.len(), 1);
        assert_eq!(duplicate_rows[0].team_id, "team-1");
        assert_eq!(duplicate_rows[0].portal_id, "portal-1");
        assert_eq!(duplicate_rows[0].state_doc_count, 2);
        assert_eq!(
            indexed
                .get(&("team-1".to_string(), "portal-1".to_string()))
                .and_then(|doc| doc.config.get("mode"))
                .and_then(serde_json::Value::as_str),
            Some("newer")
        );
    }

    #[test]
    fn binding_issue_message_describes_owner_manager_mismatch() {
        let service_agent = TeamAgentDoc {
            id: None,
            agent_id: "service-1".to_string(),
            team_id: "team-1".to_string(),
            name: "Service 1".to_string(),
            description: None,
            avatar: None,
            system_prompt: None,
            api_url: None,
            model: None,
            api_key: None,
            api_format: "openai".to_string(),
            status: "idle".to_string(),
            last_error: None,
            enabled_extensions: Vec::new(),
            custom_extensions: Vec::new(),
            agent_domain: Some("digital_avatar".to_string()),
            agent_role: Some("service".to_string()),
            owner_manager_agent_id: Some("manager-legacy".to_string()),
            template_source_agent_id: None,
            allowed_groups: Vec::new(),
            max_concurrent_tasks: 1,
            temperature: None,
            max_tokens: None,
            context_limit: None,
            thinking_enabled: true,
            assigned_skills: Vec::new(),
            auto_approve_chat: false,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        let message = AgentService::binding_issue_message(
            &AvatarBindingIssueKind::OwnerManagerMismatch,
            Some("manager-1"),
            Some("service-1"),
            Some("manager-1"),
            Some("service-1"),
            None,
            Some(&service_agent),
        );
        assert!(message.contains("owner_manager_agent_id"));
        assert!(message.contains("manager-legacy"));
        assert!(message.contains("manager-1"));
    }

    #[test]
    fn binding_issue_messages_include_human_readable_role_mismatch() {
        let manager_agent = TeamAgentDoc {
            id: None,
            agent_id: "manager-1".to_string(),
            team_id: "team-1".to_string(),
            name: "Manager 1".to_string(),
            description: None,
            avatar: None,
            system_prompt: None,
            api_url: None,
            model: None,
            api_key: None,
            api_format: "openai".to_string(),
            status: "idle".to_string(),
            last_error: None,
            enabled_extensions: Vec::new(),
            custom_extensions: Vec::new(),
            agent_domain: Some("general".to_string()),
            agent_role: Some("default".to_string()),
            owner_manager_agent_id: None,
            template_source_agent_id: None,
            allowed_groups: Vec::new(),
            max_concurrent_tasks: 1,
            temperature: None,
            max_tokens: None,
            context_limit: None,
            thinking_enabled: true,
            assigned_skills: Vec::new(),
            auto_approve_chat: false,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        let messages = AgentService::binding_issue_messages(
            &[AvatarBindingIssueKind::ManagerRoleMismatch],
            Some("manager-1"),
            None,
            Some("manager-1"),
            None,
            Some(&manager_agent),
            None,
        );
        assert_eq!(messages.len(), 1);
        assert!(messages[0].contains("digital_avatar:manager"));
        assert!(messages[0].contains("general:default"));
    }

    #[test]
    fn orphaned_recovery_filter_excludes_current_instance_missions() {
        let filter = orphaned_mission_recovery_filter("instance-current");
        let clauses = filter
            .get_array("$or")
            .expect("recovery filter should include ownership clauses");

        assert_eq!(
            filter
                .get_document("status")
                .unwrap()
                .get_array("$in")
                .unwrap()
                .len(),
            2
        );
        assert!(
            clauses.iter().any(|item| {
                item.as_document()
                    .and_then(|doc| doc.get_document("server_instance_id").ok())
                    .and_then(|doc| doc.get_str("$ne").ok())
                    == Some("instance-current")
            }),
            "current instance exclusion must be present"
        );
    }

    #[test]
    fn orphaned_recovery_filter_keeps_legacy_missing_instance_docs_recoverable() {
        let filter = orphaned_mission_recovery_filter("instance-current");
        let clauses = filter
            .get_array("$or")
            .expect("recovery filter should include ownership clauses");

        assert!(
            clauses.iter().any(|item| {
                item.as_document()
                    .and_then(|doc| doc.get("server_instance_id"))
                    == Some(&Bson::Null)
            }),
            "null-instance legacy missions should still be recoverable"
        );
        assert!(
            clauses.iter().any(|item| {
                item.as_document()
                    .and_then(|doc| doc.get_document("server_instance_id").ok())
                    .and_then(|doc| doc.get_bool("$exists").ok())
                    == Some(false)
            }),
            "missing-instance legacy missions should still be recoverable"
        );
    }

    #[test]
    fn inactive_running_recovery_filter_excludes_active_missions() {
        let filter = active_running_mission_scan_filter(&[
            "mission-active".to_string(),
            "mission-busy".to_string(),
        ]);

        let mission_clause = filter
            .get_document("mission_id")
            .expect("filter should include mission_id exclusion");
        let excluded = mission_clause
            .get_array("$nin")
            .expect("mission exclusion should be an array");

        assert_eq!(excluded.len(), 2);
        assert!(excluded
            .iter()
            .any(|item: &Bson| item.as_str() == Some("mission-active")));
        assert!(excluded
            .iter()
            .any(|item: &Bson| item.as_str() == Some("mission-busy")));
    }

    #[test]
    fn inactive_running_recovery_filter_targets_active_statuses_before_cutoff() {
        let filter = active_running_mission_scan_filter(&[]);

        assert_eq!(
            filter
                .get_document("status")
                .unwrap()
                .get_array("$in")
                .unwrap()
                .len(),
            2
        );
        assert!(
            filter.get("mission_id").is_none(),
            "empty active mission list should not inject a mission_id filter"
        );
    }

    #[test]
    fn mission_inactive_for_resume_uses_step_activity_before_mission_updated_at() {
        let cutoff = Utc::now() - chrono::Duration::minutes(10);
        let mut mission = sample_mission_doc();
        mission.status = MissionStatus::Running;
        mission.current_step = Some(0);
        mission.updated_at =
            bson::DateTime::from_chrono(Utc::now() - chrono::Duration::minutes(30));
        mission.steps[0].last_activity_at = Some(bson::DateTime::from_chrono(
            Utc::now() - chrono::Duration::minutes(1),
        ));

        assert!(
            !mission_inactive_for_resume(&mission, cutoff),
            "fresh step activity should keep the mission recoverable only later"
        );
    }

    #[test]
    fn mission_inactive_for_resume_claims_stale_running_step() {
        let cutoff = Utc::now() - chrono::Duration::minutes(10);
        let mut mission = sample_mission_doc();
        mission.status = MissionStatus::Running;
        mission.current_step = Some(0);
        mission.updated_at =
            bson::DateTime::from_chrono(Utc::now() - chrono::Duration::minutes(30));
        mission.steps[0].last_activity_at = Some(bson::DateTime::from_chrono(
            Utc::now() - chrono::Duration::minutes(20),
        ));
        mission.steps[0].last_progress_at = Some(bson::DateTime::from_chrono(
            Utc::now() - chrono::Duration::minutes(15),
        ));

        assert!(
            mission_inactive_for_resume(&mission, cutoff),
            "stale step activity/progress should make the mission claimable for auto-resume"
        );
    }

    #[test]
    fn mission_inactive_for_resume_uses_current_goal_activity_when_steps_are_empty() {
        let cutoff = Utc::now() - chrono::Duration::minutes(10);
        let mut mission = sample_mission_doc();
        mission.execution_mode = ExecutionMode::Adaptive;
        mission.status = MissionStatus::Running;
        mission.current_step = None;
        mission.steps.clear();
        mission.current_goal_id = Some("g-1".to_string());
        mission.updated_at =
            bson::DateTime::from_chrono(Utc::now() - chrono::Duration::minutes(30));
        mission.goal_tree = Some(vec![GoalNode {
            goal_id: "g-1".to_string(),
            parent_id: None,
            title: "Goal 1".to_string(),
            description: "desc".to_string(),
            success_criteria: "done".to_string(),
            status: GoalStatus::Running,
            depth: 0,
            order: 0,
            exploration_budget: 3,
            attempts: Vec::new(),
            output_summary: None,
            runtime_contract: None,
            contract_verification: None,
            pivot_reason: None,
            is_checkpoint: false,
            created_at: Some(bson::DateTime::from_chrono(
                Utc::now() - chrono::Duration::minutes(40),
            )),
            started_at: Some(bson::DateTime::from_chrono(
                Utc::now() - chrono::Duration::minutes(20),
            )),
            last_activity_at: Some(bson::DateTime::from_chrono(
                Utc::now() - chrono::Duration::minutes(1),
            )),
            last_progress_at: None,
            completed_at: None,
        }]);

        assert!(
            !mission_inactive_for_resume(&mission, cutoff),
            "fresh adaptive goal activity should keep the mission from being claimed"
        );
    }

    #[test]
    fn mission_inactive_for_resume_claims_stale_running_goal() {
        let cutoff = Utc::now() - chrono::Duration::minutes(10);
        let mut mission = sample_mission_doc();
        mission.execution_mode = ExecutionMode::Adaptive;
        mission.status = MissionStatus::Running;
        mission.current_step = None;
        mission.steps.clear();
        mission.current_goal_id = Some("g-1".to_string());
        mission.updated_at =
            bson::DateTime::from_chrono(Utc::now() - chrono::Duration::minutes(30));
        mission.goal_tree = Some(vec![GoalNode {
            goal_id: "g-1".to_string(),
            parent_id: None,
            title: "Goal 1".to_string(),
            description: "desc".to_string(),
            success_criteria: "done".to_string(),
            status: GoalStatus::Running,
            depth: 0,
            order: 0,
            exploration_budget: 3,
            attempts: Vec::new(),
            output_summary: None,
            runtime_contract: None,
            contract_verification: None,
            pivot_reason: None,
            is_checkpoint: false,
            created_at: Some(bson::DateTime::from_chrono(
                Utc::now() - chrono::Duration::minutes(40),
            )),
            started_at: Some(bson::DateTime::from_chrono(
                Utc::now() - chrono::Duration::minutes(20),
            )),
            last_activity_at: Some(bson::DateTime::from_chrono(
                Utc::now() - chrono::Duration::minutes(20),
            )),
            last_progress_at: Some(bson::DateTime::from_chrono(
                Utc::now() - chrono::Duration::minutes(15),
            )),
            completed_at: None,
        }]);

        assert!(
            mission_inactive_for_resume(&mission, cutoff),
            "stale adaptive goal activity should make the mission claimable for auto-resume"
        );
    }
}

// MongoDB document types
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamAgentDoc {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub agent_id: String,
    pub team_id: String,
    pub name: String,
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub avatar: Option<String>,
    pub system_prompt: Option<String>,
    pub api_url: Option<String>,
    pub model: Option<String>,
    pub api_key: Option<String>,
    pub api_format: String,
    pub status: String,
    pub last_error: Option<String>,
    pub enabled_extensions: Vec<AgentExtensionConfig>,
    pub custom_extensions: Vec<CustomExtensionConfig>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub agent_domain: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub agent_role: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub owner_manager_agent_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub template_source_agent_id: Option<String>,
    #[serde(default)]
    pub allowed_groups: Vec<String>,
    #[serde(default = "default_max_concurrent")]
    pub max_concurrent_tasks: u32,
    /// LLM temperature (0.0 - 1.0)
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub temperature: Option<f32>,
    /// Maximum output tokens per LLM call
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub max_tokens: Option<i32>,
    /// Context window limit override
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub context_limit: Option<usize>,
    /// Whether think/reasoning mode should be enabled for this agent.
    #[serde(default = "default_thinking_enabled")]
    pub thinking_enabled: bool,
    /// Skills assigned from team shared skills
    #[serde(default)]
    pub assigned_skills: Vec<AgentSkillConfig>,
    /// Auto-approve chat tasks (skip manual approval for chat messages)
    #[serde(default = "default_auto_approve_chat")]
    pub auto_approve_chat: bool,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AvatarGovernanceCounts {
    #[serde(default)]
    pub pending_capability_requests: u32,
    #[serde(default)]
    pub pending_gap_proposals: u32,
    #[serde(default)]
    pub pending_optimization_tickets: u32,
    #[serde(default)]
    pub pending_runtime_logs: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AvatarInstanceDoc {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub portal_id: String,
    pub team_id: String,
    pub slug: String,
    pub name: String,
    pub status: String,
    pub avatar_type: String,
    pub manager_agent_id: Option<String>,
    pub service_agent_id: Option<String>,
    pub document_access_mode: String,
    pub governance_counts: AvatarGovernanceCounts,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub portal_updated_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub projected_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AvatarInstanceSummary {
    pub portal_id: String,
    pub team_id: String,
    pub slug: String,
    pub name: String,
    pub status: String,
    pub avatar_type: String,
    pub manager_agent_id: Option<String>,
    pub service_agent_id: Option<String>,
    pub document_access_mode: String,
    pub governance_counts: AvatarGovernanceCounts,
    pub portal_updated_at: DateTime<Utc>,
    pub projected_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AvatarGovernanceStateDoc {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub portal_id: String,
    pub team_id: String,
    pub state: serde_json::Value,
    pub config: serde_json::Value,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AvatarGovernanceStatePayload {
    pub portal_id: String,
    pub team_id: String,
    pub state: serde_json::Value,
    pub config: serde_json::Value,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AvatarGovernanceEventDoc {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub portal_id: String,
    pub team_id: String,
    pub event_type: String,
    pub entity_type: String,
    pub entity_id: Option<String>,
    pub title: String,
    pub status: Option<String>,
    pub detail: Option<String>,
    pub actor_id: Option<String>,
    pub actor_name: Option<String>,
    pub meta: serde_json::Value,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AvatarGovernanceEventPayload {
    pub event_id: String,
    pub portal_id: String,
    pub team_id: String,
    pub event_type: String,
    pub entity_type: String,
    pub entity_id: Option<String>,
    pub title: String,
    pub status: Option<String>,
    pub detail: Option<String>,
    pub actor_id: Option<String>,
    pub actor_name: Option<String>,
    pub meta: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AvatarGovernanceQueueItemPayload {
    pub id: String,
    pub kind: String,
    pub title: String,
    pub detail: String,
    pub status: String,
    pub ts: String,
    pub meta: Vec<String>,
    pub source_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AvatarWorkbenchSummaryPayload {
    pub portal_id: String,
    pub team_id: String,
    pub avatar_name: String,
    pub avatar_type: String,
    pub avatar_status: String,
    pub manager_agent_id: Option<String>,
    pub service_agent_id: Option<String>,
    pub document_access_mode: String,
    pub work_object_count: u32,
    pub pending_decision_count: u32,
    pub running_mission_count: u32,
    pub needs_attention_count: u32,
    pub last_activity_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AvatarWorkbenchReportItemPayload {
    pub id: String,
    pub ts: DateTime<Utc>,
    pub kind: String,
    pub title: String,
    pub summary: String,
    pub status: String,
    pub source: String,
    pub recommendation: Option<String>,
    pub action_kind: Option<String>,
    pub action_target_id: Option<String>,
    #[serde(default)]
    pub work_objects: Vec<String>,
    #[serde(default)]
    pub outputs: Vec<String>,
    #[serde(default)]
    pub needs_decision: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AvatarWorkbenchDecisionItemPayload {
    pub id: String,
    pub ts: DateTime<Utc>,
    #[serde(default)]
    pub kind: String,
    pub title: String,
    pub detail: String,
    pub status: String,
    pub risk: String,
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub recommendation: Option<String>,
    #[serde(default)]
    pub work_objects: Vec<String>,
    pub action_kind: Option<String>,
    pub action_target_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AvatarWorkbenchSnapshotPayload {
    pub portal_id: String,
    pub team_id: String,
    pub summary: AvatarWorkbenchSummaryPayload,
    pub reports: Vec<AvatarWorkbenchReportItemPayload>,
    pub decisions: Vec<AvatarWorkbenchDecisionItemPayload>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AvatarManagerReportDoc {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub report_id: String,
    pub portal_id: String,
    pub team_id: String,
    #[serde(default = "default_avatar_manager_report_source")]
    pub report_source: String,
    pub kind: String,
    pub title: String,
    pub summary: String,
    pub status: String,
    pub source: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub recommendation: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub action_kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub action_target_id: Option<String>,
    #[serde(default)]
    pub work_objects: Vec<String>,
    #[serde(default)]
    pub outputs: Vec<String>,
    #[serde(default)]
    pub needs_decision: bool,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub synced_at: DateTime<Utc>,
}

fn default_avatar_manager_report_source() -> String {
    "derived".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AvatarGovernanceBackfillReport {
    pub portals_scanned: u64,
    pub states_created: u64,
    pub events_seeded: u64,
    pub projections_synced_teams: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AvatarGovernanceDivergenceKind {
    DefaultOnly,
    InSync,
    StateOnly,
    SettingsOnly,
    Differing,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AvatarGovernanceDivergenceRow {
    pub team_id: String,
    pub portal_id: String,
    pub portal_name: String,
    pub slug: String,
    pub classification: AvatarGovernanceDivergenceKind,
    pub state_doc_exists: bool,
    pub portal_state_exists: bool,
    pub portal_config_exists: bool,
    pub state_matches_portal_state: bool,
    pub config_matches_portal_config: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AvatarGovernanceOrphanStateRow {
    pub team_id: String,
    pub portal_id: String,
    pub state_doc_count: u64,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AvatarGovernanceDuplicateStateRow {
    pub team_id: String,
    pub portal_id: String,
    pub state_doc_count: u64,
    pub retained_updated_at: String,
    pub duplicate_updated_at: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AvatarGovernanceDivergenceAuditReport {
    pub total_avatar_portals: u64,
    pub total_state_docs: u64,
    pub default_only: u64,
    pub in_sync: u64,
    pub state_only: u64,
    pub settings_only: u64,
    pub differing: u64,
    pub duplicate_state_docs: u64,
    pub orphan_state_docs: u64,
    pub rows: Vec<AvatarGovernanceDivergenceRow>,
    pub duplicate_rows: Vec<AvatarGovernanceDuplicateStateRow>,
    pub orphan_rows: Vec<AvatarGovernanceOrphanStateRow>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AvatarBindingIssueKind {
    MissingExplicitManagerBinding,
    MissingExplicitServiceBinding,
    MissingManagerBinding,
    MissingServiceBinding,
    ManagerAgentNotFound,
    ServiceAgentNotFound,
    ManagerRoleMismatch,
    ServiceRoleMismatch,
    OwnerManagerMismatch,
    SameAgentReused,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AvatarBindingAuditRow {
    pub team_id: String,
    pub portal_id: String,
    pub portal_name: String,
    pub slug: String,
    pub explicit_coding_agent_id: Option<String>,
    pub explicit_service_agent_id: Option<String>,
    pub effective_manager_agent_id: Option<String>,
    pub effective_service_agent_id: Option<String>,
    pub manager_agent_domain: Option<String>,
    pub manager_agent_role: Option<String>,
    pub service_agent_domain: Option<String>,
    pub service_agent_role: Option<String>,
    pub service_owner_manager_agent_id: Option<String>,
    pub issues: Vec<AvatarBindingIssueKind>,
    pub issue_messages: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AvatarBindingAuditReport {
    pub total_avatar_portals: u64,
    pub valid: u64,
    pub missing_explicit_manager_binding: u64,
    pub missing_explicit_service_binding: u64,
    pub missing_manager_binding: u64,
    pub missing_service_binding: u64,
    pub manager_agent_not_found: u64,
    pub service_agent_not_found: u64,
    pub manager_role_mismatch: u64,
    pub service_role_mismatch: u64,
    pub owner_manager_mismatch: u64,
    pub same_agent_reused: u64,
    pub shadow_invariant_rows: u64,
    pub rows: Vec<AvatarBindingAuditRow>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AvatarReadSideEffectAuditItem {
    pub operation: String,
    pub file: String,
    pub side_effects: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AvatarDeepWaterAuditReport {
    pub generated_at: String,
    pub requested_team_id: Option<String>,
    pub governance: AvatarGovernanceDivergenceAuditReport,
    pub bindings: AvatarBindingAuditReport,
    pub read_side_effects: Vec<AvatarReadSideEffectAuditItem>,
}

#[derive(Debug, Clone)]
struct GovernanceEntitySnapshot {
    entity_type: &'static str,
    id: String,
    title: String,
    status: String,
    detail: String,
    meta: serde_json::Value,
}

fn mission_status_label(status: &MissionStatus) -> &'static str {
    match status {
        MissionStatus::Draft => "draft",
        MissionStatus::Planning => "planning",
        MissionStatus::Planned => "planned",
        MissionStatus::Running => "running",
        MissionStatus::Paused => "paused",
        MissionStatus::Completed => "completed",
        MissionStatus::Failed => "failed",
        MissionStatus::Cancelled => "cancelled",
    }
}

fn mission_needs_attention(status: &MissionStatus) -> bool {
    matches!(status, MissionStatus::Failed | MissionStatus::Paused)
}

fn queue_item_risk(meta: &[String]) -> &'static str {
    if meta.iter().any(|item| item.contains("高风险")) {
        "high"
    } else if meta.iter().any(|item| item.contains("中风险")) {
        "medium"
    } else {
        "low"
    }
}

fn queue_item_source(kind: &str, meta: &[String]) -> String {
    meta.iter()
        .find(|item| {
            !item.contains("风险")
                && !item.contains("high")
                && !item.contains("medium")
                && !item.contains("low")
        })
        .cloned()
        .unwrap_or_else(|| match kind {
            "capability" => "能力边界".to_string(),
            "proposal" => "岗位提案".to_string(),
            "ticket" => "运行优化".to_string(),
            _ => "管理台".to_string(),
        })
}

fn queue_item_recommendation(kind: &str, risk: &str) -> String {
    match kind {
        "capability" => {
            if risk == "high" {
                "先确认这次能力放开是否触及高风险边界，再决定是交人工审批还是保持收敛。".to_string()
            } else {
                "先判断是否需要放开当前能力边界，再决定批准、试运行或继续收敛。".to_string()
            }
        }
        "proposal" => "先判断这项岗位提案是否值得试运行，再决定批准、试点或拒绝。".to_string(),
        "ticket" => {
            if risk == "high" {
                "先查看这次运行问题与风险证据，再决定回滚、人工确认或进入试运行。".to_string()
            } else {
                "先确认优化证据和预期收益，再决定试运行、批准或继续观察。".to_string()
            }
        }
        _ => "先阅读当前决策背景，再决定继续执行、调整方案或转人工确认。".to_string(),
    }
}

fn parse_rfc3339_utc(raw: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(raw)
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
}

fn default_max_concurrent() -> u32 {
    1
}

fn default_auto_approve_chat() -> bool {
    true
}

fn default_thinking_enabled() -> bool {
    true
}

impl From<TeamAgentDoc> for TeamAgent {
    fn from(doc: TeamAgentDoc) -> Self {
        Self {
            id: doc.agent_id,
            team_id: doc.team_id,
            name: doc.name,
            description: doc.description,
            avatar: doc.avatar,
            system_prompt: doc.system_prompt,
            api_url: doc.api_url,
            model: doc.model,
            api_key: None, // Don't expose API key
            api_format: doc.api_format.parse().unwrap_or(ApiFormat::OpenAI),
            enabled_extensions: doc.enabled_extensions,
            custom_extensions: doc.custom_extensions,
            agent_domain: doc.agent_domain,
            agent_role: doc.agent_role,
            owner_manager_agent_id: doc.owner_manager_agent_id,
            template_source_agent_id: doc.template_source_agent_id,
            status: doc.status.parse().unwrap_or(AgentStatus::Idle),
            last_error: doc.last_error,
            allowed_groups: doc.allowed_groups,
            max_concurrent_tasks: doc.max_concurrent_tasks,
            temperature: doc.temperature,
            max_tokens: doc.max_tokens,
            context_limit: doc.context_limit,
            thinking_enabled: doc.thinking_enabled,
            assigned_skills: doc.assigned_skills,
            auto_approve_chat: doc.auto_approve_chat,
            created_at: doc.created_at,
            updated_at: doc.updated_at,
        }
    }
}

impl From<AvatarManagerReportDoc> for AvatarWorkbenchReportItemPayload {
    fn from(doc: AvatarManagerReportDoc) -> Self {
        Self {
            id: doc.report_id,
            ts: doc.created_at,
            kind: doc.kind,
            title: doc.title,
            summary: doc.summary,
            status: doc.status,
            source: doc.source,
            recommendation: doc.recommendation,
            action_kind: doc.action_kind,
            action_target_id: doc.action_target_id,
            work_objects: doc.work_objects,
            outputs: doc.outputs,
            needs_decision: doc.needs_decision,
        }
    }
}

impl From<AvatarInstanceDoc> for AvatarInstanceSummary {
    fn from(doc: AvatarInstanceDoc) -> Self {
        Self {
            portal_id: doc.portal_id,
            team_id: doc.team_id,
            slug: doc.slug,
            name: doc.name,
            status: doc.status,
            avatar_type: doc.avatar_type,
            manager_agent_id: doc.manager_agent_id,
            service_agent_id: doc.service_agent_id,
            document_access_mode: doc.document_access_mode,
            governance_counts: doc.governance_counts,
            portal_updated_at: doc.portal_updated_at,
            projected_at: doc.projected_at,
        }
    }
}

impl From<AvatarGovernanceStateDoc> for AvatarGovernanceStatePayload {
    fn from(doc: AvatarGovernanceStateDoc) -> Self {
        Self {
            portal_id: doc.portal_id,
            team_id: doc.team_id,
            state: doc.state,
            config: doc.config,
            updated_at: doc.updated_at,
        }
    }
}

impl From<AvatarGovernanceEventDoc> for AvatarGovernanceEventPayload {
    fn from(doc: AvatarGovernanceEventDoc) -> Self {
        let event_id = doc.id.map(|value| value.to_hex()).unwrap_or_else(|| {
            format!(
                "synthetic:{}:{}:{}:{}:{}:{}",
                doc.team_id,
                doc.portal_id,
                doc.event_type,
                doc.entity_type,
                doc.entity_id.as_deref().unwrap_or("none"),
                doc.created_at.timestamp_millis()
            )
        });
        Self {
            event_id,
            portal_id: doc.portal_id,
            team_id: doc.team_id,
            event_type: doc.event_type,
            entity_type: doc.entity_type,
            entity_id: doc.entity_id,
            title: doc.title,
            status: doc.status,
            detail: doc.detail,
            actor_id: doc.actor_id,
            actor_name: doc.actor_name,
            meta: doc.meta,
            created_at: doc.created_at,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentTaskDoc {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub task_id: String,
    pub team_id: String,
    pub agent_id: String,
    pub submitter_id: String,
    pub approver_id: Option<String>,
    pub task_type: String,
    pub content: serde_json::Value,
    pub status: String,
    pub priority: i32,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub submitted_at: DateTime<Utc>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "bson_datetime_option"
    )]
    pub approved_at: Option<DateTime<Utc>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "bson_datetime_option"
    )]
    pub started_at: Option<DateTime<Utc>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "bson_datetime_option"
    )]
    pub completed_at: Option<DateTime<Utc>>,
    pub error_message: Option<String>,
}

impl From<AgentTaskDoc> for AgentTask {
    fn from(doc: AgentTaskDoc) -> Self {
        Self {
            id: doc.task_id,
            team_id: doc.team_id,
            agent_id: doc.agent_id,
            submitter_id: doc.submitter_id,
            approver_id: doc.approver_id,
            task_type: doc.task_type.parse().unwrap_or(TaskType::Chat),
            content: doc.content,
            status: doc.status.parse().unwrap_or(TaskStatus::Pending),
            priority: doc.priority,
            submitted_at: doc.submitted_at,
            approved_at: doc.approved_at,
            started_at: doc.started_at,
            completed_at: doc.completed_at,
            error_message: doc.error_message,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskResultDoc {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub id: Option<ObjectId>,
    pub result_id: String,
    pub task_id: String,
    pub result_type: String,
    pub content: serde_json::Value,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
}

impl From<TaskResultDoc> for TaskResult {
    fn from(doc: TaskResultDoc) -> Self {
        let result_type = match doc.result_type.as_str() {
            "message" => TaskResultType::Message,
            "tool_call" => TaskResultType::ToolCall,
            _ => TaskResultType::Error,
        };
        Self {
            id: doc.result_id,
            task_id: doc.task_id,
            result_type,
            content: doc.content,
            created_at: doc.created_at,
        }
    }
}

/// Convert a TeamAgentDoc to TeamAgent while preserving the api_key field
/// (which is normally stripped by the From impl for API safety).
fn agent_doc_with_key(doc: TeamAgentDoc) -> TeamAgent {
    let api_key = doc.api_key.clone();
    let mut agent: TeamAgent = doc.into();
    agent.api_key = api_key;
    agent
}

/// Agent service for managing team agents and tasks (MongoDB)
pub struct AgentService {
    db: Arc<MongoDb>,
}

impl AgentService {
    pub fn new(db: Arc<MongoDb>) -> Self {
        Self { db }
    }

    fn normalize_session_source(raw: Option<String>, portal_restricted: bool) -> String {
        let v = raw
            .unwrap_or_else(|| {
                if portal_restricted {
                    "portal".to_string()
                } else {
                    "chat".to_string()
                }
            })
            .trim()
            .to_ascii_lowercase()
            .replace('-', "_");
        match v.as_str() {
            "mission" | "portal" | "portal_coding" | "portal_manager" | "system" | "chat" => v,
            _ => {
                if portal_restricted {
                    "portal".to_string()
                } else {
                    "chat".to_string()
                }
            }
        }
    }

    fn normalize_skill_id_list(items: Vec<String>) -> Vec<String> {
        let mut seen = HashSet::new();
        items
            .into_iter()
            .map(|item| item.trim().to_string())
            .filter(|item| !item.is_empty())
            .filter(|item| seen.insert(item.clone()))
            .collect()
    }

    async fn resolve_session_allowed_skill_ids(
        &self,
        agent_id: &str,
        requested_allowed_skill_ids: Option<Vec<String>>,
    ) -> Result<Option<Vec<String>>, mongodb::error::Error> {
        let requested_allowed_skill_ids =
            requested_allowed_skill_ids.map(Self::normalize_skill_id_list);

        let Some(agent) = self.get_agent(agent_id).await? else {
            return Ok(requested_allowed_skill_ids);
        };

        let assigned_skill_ids = Self::normalize_skill_id_list(
            agent
                .assigned_skills
                .iter()
                .filter(|skill| skill.enabled)
                .map(|skill| skill.skill_id.clone())
                .collect(),
        );
        let assigned_lookup = assigned_skill_ids.iter().cloned().collect::<HashSet<_>>();

        let effective = match requested_allowed_skill_ids {
            Some(requested) => requested
                .into_iter()
                .filter(|skill_id| assigned_lookup.contains(skill_id))
                .collect::<Vec<_>>(),
            None => assigned_skill_ids,
        };

        Ok(Some(effective))
    }

    /// M12: Ensure MongoDB indexes for agent_sessions collection (chat track)
    pub async fn ensure_chat_indexes(&self) {
        use mongodb::options::IndexOptions;
        use mongodb::IndexModel;

        let session_indexes = vec![
            // User session list query (sorted by last message time)
            IndexModel::builder()
                .keys(doc! { "team_id": 1, "user_id": 1, "status": 1, "last_message_at": -1 })
                .build(),
            // User session list query with visibility filter
            IndexModel::builder()
                .keys(doc! { "team_id": 1, "user_id": 1, "status": 1, "hidden_from_chat_list": 1, "last_message_at": -1 })
                .build(),
            // Filter by agent
            IndexModel::builder()
                .keys(doc! { "team_id": 1, "agent_id": 1, "status": 1 })
                .build(),
            // Pinned + time sort
            IndexModel::builder()
                .keys(doc! { "team_id": 1, "user_id": 1, "pinned": -1, "last_message_at": -1 })
                .build(),
            // Session lookup by session_id (unique)
            IndexModel::builder()
                .keys(doc! { "session_id": 1 })
                .options(IndexOptions::builder().unique(true).build())
                .build(),
        ];

        if let Err(e) = self.sessions().create_indexes(session_indexes, None).await {
            tracing::warn!("Failed to create chat session indexes: {}", e);
        } else {
            tracing::info!("Chat session indexes ensured");
        }

        let event_indexes = vec![
            IndexModel::builder()
                .keys(doc! { "session_id": 1, "event_id": 1 })
                .build(),
            IndexModel::builder()
                .keys(doc! { "session_id": 1, "run_id": 1, "event_id": 1 })
                .options(
                    IndexOptions::builder()
                        .unique(true)
                        .partial_filter_expression(doc! { "run_id": { "$exists": true } })
                        .build(),
                )
                .build(),
            IndexModel::builder()
                .keys(doc! { "session_id": 1, "created_at": -1 })
                .build(),
            IndexModel::builder()
                .keys(doc! { "session_id": 1, "run_id": 1, "created_at": 1 })
                .build(),
        ];

        if let Err(e) = self.chat_events().create_indexes(event_indexes, None).await {
            tracing::warn!("Failed to create chat event indexes: {}", e);
        } else {
            tracing::info!("Chat event indexes ensured");
        }
    }

    fn agents(&self) -> mongodb::Collection<TeamAgentDoc> {
        self.db.collection("team_agents")
    }

    fn tasks(&self) -> mongodb::Collection<AgentTaskDoc> {
        self.db.collection("agent_tasks")
    }

    fn results(&self) -> mongodb::Collection<TaskResultDoc> {
        self.db.collection("agent_task_results")
    }

    fn sessions(&self) -> mongodb::Collection<AgentSessionDoc> {
        self.db.collection("agent_sessions")
    }

    fn chat_events(&self) -> mongodb::Collection<ChatEventDoc> {
        self.db.collection("agent_chat_events")
    }

    fn teams(&self) -> mongodb::Collection<Document> {
        self.db.collection("teams")
    }

    fn portals(&self) -> mongodb::Collection<Document> {
        self.db.collection(agime_team::db::collections::PORTALS)
    }

    fn documents_store(&self) -> mongodb::Collection<TeamDocument> {
        self.db.collection(agime_team::db::collections::DOCUMENTS)
    }

    fn avatar_instances(&self) -> mongodb::Collection<AvatarInstanceDoc> {
        self.db
            .collection(agime_team::db::collections::AVATAR_INSTANCES)
    }

    fn avatar_governance_states(&self) -> mongodb::Collection<AvatarGovernanceStateDoc> {
        self.db
            .collection(agime_team::db::collections::AVATAR_GOVERNANCE_STATES)
    }

    fn avatar_governance_events(&self) -> mongodb::Collection<AvatarGovernanceEventDoc> {
        self.db
            .collection(agime_team::db::collections::AVATAR_GOVERNANCE_EVENTS)
    }

    fn avatar_manager_reports(&self) -> mongodb::Collection<AvatarManagerReportDoc> {
        self.db
            .collection(agime_team::db::collections::AVATAR_MANAGER_REPORTS)
    }

    fn is_dedicated_avatar_marker(description: Option<&str>) -> bool {
        let desc = description.unwrap_or("").to_ascii_lowercase();
        desc.contains("[digital-avatar-manager]") || desc.contains("[digital-avatar-service]")
    }

    fn explicit_avatar_marker_role(description: Option<&str>) -> Option<&'static str> {
        let desc = description.unwrap_or("").to_ascii_lowercase();
        if desc.contains("[digital-avatar-manager]") {
            return Some("manager");
        }
        if desc.contains("[digital-avatar-service]") {
            return Some("service");
        }
        None
    }

    fn normalize_compact_text(value: Option<&str>) -> String {
        value
            .unwrap_or("")
            .trim()
            .to_ascii_lowercase()
            .chars()
            .filter(|ch| !ch.is_whitespace())
            .collect()
    }

    fn is_legacy_avatar_manager_candidate(agent: &TeamAgent) -> bool {
        if agent.agent_domain.as_deref() == Some("digital_avatar")
            && agent.agent_role.as_deref() == Some("manager")
        {
            return true;
        }
        if Self::is_dedicated_avatar_marker(agent.description.as_deref()) {
            return true;
        }
        let compact_name = Self::normalize_compact_text(Some(agent.name.as_str()));
        compact_name.contains("管理agent") || compact_name.contains("manageragent")
    }

    fn is_legacy_avatar_service_candidate(agent: &TeamAgent) -> bool {
        if agent.agent_domain.as_deref() == Some("digital_avatar")
            && agent.agent_role.as_deref() == Some("service")
        {
            return true;
        }
        if Self::is_dedicated_avatar_marker(agent.description.as_deref()) {
            return true;
        }
        let compact_name = Self::normalize_compact_text(Some(agent.name.as_str()));
        compact_name.ends_with("分身agent") || compact_name.contains("avataragent")
    }

    fn doc_string(doc: &Document, key: &str) -> Option<String> {
        doc.get(key).and_then(Bson::as_str).map(str::to_string)
    }

    fn nested_doc_string(doc: &Document, path: &[&str]) -> Option<String> {
        let mut current = doc;
        for key in &path[..path.len().saturating_sub(1)] {
            current = current.get_document(key).ok()?;
        }
        let leaf = *path.last()?;
        current.get(leaf).and_then(Bson::as_str).map(str::to_string)
    }

    fn doc_has_string(doc: &Document, key: &str, expected: &str) -> bool {
        Self::doc_string(doc, key)
            .as_deref()
            .map(str::trim)
            .is_some_and(|value| value == expected)
    }

    fn nested_doc_has_string(doc: &Document, path: &[&str], expected: &str) -> bool {
        Self::nested_doc_string(doc, path)
            .as_deref()
            .map(str::trim)
            .is_some_and(|value| value == expected)
    }

    fn doc_has_manager_tag(doc: &Document, expected: &str) -> bool {
        doc.get_array("tags")
            .ok()
            .map(|tags| {
                tags.iter()
                    .filter_map(Bson::as_str)
                    .map(str::trim)
                    .any(|tag| tag == format!("manager:{expected}"))
            })
            .unwrap_or(false)
    }

    fn extract_portal_manager_id(doc: &Document) -> Option<String> {
        Self::doc_string(doc, "coding_agent_id")
            .filter(|v| !v.trim().is_empty())
            .or_else(|| {
                Self::nested_doc_string(doc, &["settings", "managerAgentId"])
                    .filter(|v| !v.trim().is_empty())
            })
            .or_else(|| {
                Self::nested_doc_string(doc, &["settings", "managerGroupId"])
                    .filter(|v| !v.trim().is_empty())
            })
            .or_else(|| {
                doc.get_array("tags").ok().and_then(|tags| {
                    tags.iter()
                        .filter_map(Bson::as_str)
                        .map(str::trim)
                        .find_map(|tag| tag.strip_prefix("manager:").map(str::to_string))
                })
            })
            .or_else(|| Self::doc_string(doc, "agent_id").filter(|v| !v.trim().is_empty()))
    }

    fn count_pending_with_status(
        items: Option<&Vec<serde_json::Value>>,
        pending_statuses: &[&str],
    ) -> u32 {
        items.map_or(0, |entries| {
            entries
                .iter()
                .filter(|entry| {
                    entry
                        .get("status")
                        .and_then(serde_json::Value::as_str)
                        .is_some_and(|status| {
                            pending_statuses.iter().any(|expected| status == *expected)
                        })
                })
                .count() as u32
        })
    }

    fn avatar_type_from_doc(doc: &Document) -> String {
        Self::nested_doc_string(doc, &["settings", "avatarType"])
            .filter(|value| !value.trim().is_empty())
            .or_else(|| {
                doc.get_array("tags").ok().and_then(|tags| {
                    if tags
                        .iter()
                        .filter_map(Bson::as_str)
                        .any(|tag| tag.eq_ignore_ascii_case("avatar:internal"))
                    {
                        Some("internal_worker".to_string())
                    } else if tags
                        .iter()
                        .filter_map(Bson::as_str)
                        .any(|tag| tag.eq_ignore_ascii_case("avatar:external"))
                    {
                        Some("external_service".to_string())
                    } else {
                        None
                    }
                })
            })
            .unwrap_or_else(|| "external_service".to_string())
    }

    fn default_governance_state_json() -> serde_json::Value {
        serde_json::json!({
            "capabilityRequests": [],
            "gapProposals": [],
            "optimizationTickets": [],
            "runtimeLogs": []
        })
    }

    fn default_governance_config_json() -> serde_json::Value {
        serde_json::json!({
            "autoProposalTriggerCount": 3,
            "managerApprovalMode": "manager_decides",
            "optimizationMode": "dual_loop",
            "lowRiskAction": "auto_execute",
            "mediumRiskAction": "manager_review",
            "highRiskAction": "human_review",
            "autoCreateCapabilityRequests": true,
            "autoCreateOptimizationTickets": true,
            "requireHumanForPublish": true
        })
    }

    fn normalize_governance_state_candidate(value: serde_json::Value) -> Option<serde_json::Value> {
        match value {
            serde_json::Value::Object(mut map) => {
                map.remove("config");
                if map.is_empty() {
                    None
                } else {
                    Some(serde_json::Value::Object(map))
                }
            }
            serde_json::Value::Null => None,
            other => Some(other),
        }
    }

    fn raw_portal_governance_json(doc: &Document) -> Option<serde_json::Value> {
        doc.get_document("settings")
            .ok()
            .and_then(|settings| settings.get("digitalAvatarGovernance"))
            .and_then(|value| bson::from_bson::<serde_json::Value>(value.clone()).ok())
    }

    fn raw_portal_governance_config_top_level_json(doc: &Document) -> Option<serde_json::Value> {
        doc.get_document("settings")
            .ok()
            .and_then(|settings| settings.get("digitalAvatarGovernanceConfig"))
            .and_then(|value| bson::from_bson::<serde_json::Value>(value.clone()).ok())
    }

    fn governance_state_json_from_portal_doc(doc: &Document) -> serde_json::Value {
        Self::raw_portal_governance_json(doc)
            .and_then(Self::normalize_governance_state_candidate)
            .unwrap_or_else(Self::default_governance_state_json)
    }

    fn raw_portal_governance_state_json(doc: &Document) -> Option<serde_json::Value> {
        Self::raw_portal_governance_json(doc).and_then(Self::normalize_governance_state_candidate)
    }

    fn governance_config_json_from_portal_doc(doc: &Document) -> serde_json::Value {
        let from_top = Self::raw_portal_governance_config_top_level_json(doc);
        if let Some(value) = from_top {
            return value;
        }
        Self::raw_portal_governance_json(doc)
            .and_then(|governance| governance.get("config").cloned())
            .unwrap_or_else(Self::default_governance_config_json)
    }

    fn raw_portal_governance_config_json(doc: &Document) -> Option<serde_json::Value> {
        let from_top = Self::raw_portal_governance_config_top_level_json(doc);
        if from_top.is_some() {
            return from_top;
        }
        Self::raw_portal_governance_json(doc)
            .and_then(|governance| governance.get("config").cloned())
    }

    fn governance_counts_from_state_json(state: &serde_json::Value) -> AvatarGovernanceCounts {
        let capability_requests = state
            .get("capabilityRequests")
            .and_then(serde_json::Value::as_array)
            .cloned();
        let gap_proposals = state
            .get("gapProposals")
            .and_then(serde_json::Value::as_array)
            .cloned();
        let optimization_tickets = state
            .get("optimizationTickets")
            .and_then(serde_json::Value::as_array)
            .cloned();
        let runtime_logs = state
            .get("runtimeLogs")
            .and_then(serde_json::Value::as_array)
            .cloned();

        AvatarGovernanceCounts {
            pending_capability_requests: Self::count_pending_with_status(
                capability_requests.as_ref(),
                &["pending"],
            ),
            pending_gap_proposals: Self::count_pending_with_status(
                gap_proposals.as_ref(),
                &["pending_approval"],
            ),
            pending_optimization_tickets: Self::count_pending_with_status(
                optimization_tickets.as_ref(),
                &["pending"],
            ),
            pending_runtime_logs: Self::count_pending_with_status(
                runtime_logs.as_ref(),
                &["pending"],
            ),
        }
    }

    fn governance_counts_from_doc(doc: &Document) -> AvatarGovernanceCounts {
        Self::governance_counts_from_state_json(&Self::governance_state_json_from_portal_doc(doc))
    }

    fn default_avatar_governance_payload(
        team_id: &str,
        portal_id: &str,
    ) -> AvatarGovernanceStatePayload {
        AvatarGovernanceStatePayload {
            portal_id: portal_id.to_string(),
            team_id: team_id.to_string(),
            state: Self::default_governance_state_json(),
            config: Self::default_governance_config_json(),
            updated_at: Utc::now(),
        }
    }

    fn governance_payload_from_portal_doc(
        team_id: &str,
        portal_id: &str,
        portal_doc: &Document,
        updated_at: DateTime<Utc>,
    ) -> AvatarGovernanceStatePayload {
        AvatarGovernanceStatePayload {
            portal_id: portal_id.to_string(),
            team_id: team_id.to_string(),
            state: Self::governance_state_json_from_portal_doc(portal_doc),
            config: Self::governance_config_json_from_portal_doc(portal_doc),
            updated_at,
        }
    }

    async fn upsert_avatar_governance_state_payload(
        &self,
        payload: &AvatarGovernanceStatePayload,
    ) -> Result<(), mongodb::error::Error> {
        self.avatar_governance_states()
            .replace_one(
                doc! { "team_id": &payload.team_id, "portal_id": &payload.portal_id },
                AvatarGovernanceStateDoc {
                    id: None,
                    portal_id: payload.portal_id.clone(),
                    team_id: payload.team_id.clone(),
                    state: payload.state.clone(),
                    config: payload.config.clone(),
                    updated_at: payload.updated_at,
                },
                mongodb::options::ReplaceOptions::builder()
                    .upsert(true)
                    .build(),
            )
            .await?;
        Ok(())
    }

    async fn sync_avatar_governance_payload_to_portal(
        &self,
        payload: &AvatarGovernanceStatePayload,
    ) -> Result<Option<u64>, mongodb::error::Error> {
        let Ok(team_oid) = ObjectId::parse_str(&payload.team_id) else {
            return Ok(None);
        };
        let Ok(portal_oid) = ObjectId::parse_str(&payload.portal_id) else {
            return Ok(None);
        };

        let update_result = self
            .portals()
            .update_one(
                doc! {
                    "_id": portal_oid,
                    "team_id": team_oid,
                    "is_deleted": { "$ne": true }
                },
                doc! {
                    "$set": {
                        "settings.digitalAvatarGovernance": bson::to_bson(&payload.state).unwrap_or(Bson::Null),
                        "settings.digitalAvatarGovernanceConfig": bson::to_bson(&payload.config).unwrap_or(Bson::Null),
                        "updated_at": bson::DateTime::from_chrono(payload.updated_at),
                    }
                },
                None,
            )
            .await?;

        Ok(Some(update_result.matched_count))
    }

    async fn sync_avatar_governance_payload_to_portal_if_current(
        &self,
        payload: &AvatarGovernanceStatePayload,
        expected_updated_at: DateTime<Utc>,
    ) -> Result<Option<u64>, mongodb::error::Error> {
        let Ok(team_oid) = ObjectId::parse_str(&payload.team_id) else {
            return Ok(None);
        };
        let Ok(portal_oid) = ObjectId::parse_str(&payload.portal_id) else {
            return Ok(None);
        };

        let update_result = self
            .portals()
            .update_one(
                doc! {
                    "_id": portal_oid,
                    "team_id": team_oid,
                    "is_deleted": { "$ne": true },
                    "updated_at": bson::DateTime::from_chrono(expected_updated_at),
                },
                doc! {
                    "$set": {
                        "settings.digitalAvatarGovernance": bson::to_bson(&payload.state).unwrap_or(Bson::Null),
                        "settings.digitalAvatarGovernanceConfig": bson::to_bson(&payload.config).unwrap_or(Bson::Null),
                        "updated_at": bson::DateTime::from_chrono(payload.updated_at),
                    }
                },
                None,
            )
            .await?;

        Ok(Some(update_result.matched_count))
    }

    fn extract_portal_service_agent_id(doc: &Document) -> Option<String> {
        Self::doc_string(doc, "service_agent_id")
            .filter(|value| !value.trim().is_empty())
            .or_else(|| {
                Self::nested_doc_string(doc, &["settings", "serviceRuntimeAgentId"])
                    .filter(|value| !value.trim().is_empty())
            })
            .or_else(|| Self::doc_string(doc, "agent_id").filter(|value| !value.trim().is_empty()))
    }

    fn normalize_optional_scope_id(value: Option<&str>) -> Option<String> {
        value
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    }

    fn index_governance_state_docs(
        docs: Vec<AvatarGovernanceStateDoc>,
    ) -> (
        HashMap<(String, String), AvatarGovernanceStateDoc>,
        HashMap<(String, String), u64>,
        Vec<AvatarGovernanceDuplicateStateRow>,
        u64,
    ) {
        let mut grouped: HashMap<(String, String), Vec<AvatarGovernanceStateDoc>> = HashMap::new();
        for doc in docs {
            grouped
                .entry((doc.team_id.clone(), doc.portal_id.clone()))
                .or_default()
                .push(doc);
        }

        let mut indexed = HashMap::new();
        let mut counts = HashMap::new();
        let mut duplicate_rows = Vec::new();
        let mut duplicate_state_docs = 0u64;

        for ((team_id, portal_id), mut entries) in grouped {
            entries.sort_by(|left, right| right.updated_at.cmp(&left.updated_at));
            let state_doc_count = entries.len() as u64;
            counts.insert((team_id.clone(), portal_id.clone()), state_doc_count);

            if state_doc_count > 1 {
                duplicate_state_docs += state_doc_count - 1;
                duplicate_rows.push(AvatarGovernanceDuplicateStateRow {
                    team_id: team_id.clone(),
                    portal_id: portal_id.clone(),
                    state_doc_count,
                    retained_updated_at: entries[0].updated_at.to_rfc3339(),
                    duplicate_updated_at: entries
                        .iter()
                        .skip(1)
                        .map(|doc| doc.updated_at.to_rfc3339())
                        .collect(),
                });
            }

            let retained = entries.remove(0);
            indexed.insert((team_id, portal_id), retained);
        }

        duplicate_rows.sort_by(|left, right| {
            left.team_id
                .cmp(&right.team_id)
                .then_with(|| left.portal_id.cmp(&right.portal_id))
        });

        (indexed, counts, duplicate_rows, duplicate_state_docs)
    }

    fn classify_governance_divergence(
        has_state_doc: bool,
        has_portal_governance: bool,
        state_matches_portal_state: bool,
        config_matches_portal_config: bool,
    ) -> AvatarGovernanceDivergenceKind {
        match (has_state_doc, has_portal_governance) {
            (false, false) => AvatarGovernanceDivergenceKind::DefaultOnly,
            (true, false) => AvatarGovernanceDivergenceKind::StateOnly,
            (false, true) => AvatarGovernanceDivergenceKind::SettingsOnly,
            (true, true) if state_matches_portal_state && config_matches_portal_config => {
                AvatarGovernanceDivergenceKind::InSync
            }
            _ => AvatarGovernanceDivergenceKind::Differing,
        }
    }

    async fn refresh_avatar_governance_state_read_model(
        &self,
        payload: &AvatarGovernanceStatePayload,
    ) -> Result<(), mongodb::error::Error> {
        self.upsert_avatar_governance_state_payload(payload).await
    }

    async fn refresh_avatar_governance_event_read_model(
        &self,
        team_id: &str,
        portal_id: &str,
        state: &serde_json::Value,
        config: &serde_json::Value,
    ) -> Result<u64, mongodb::error::Error> {
        self.seed_avatar_governance_events_if_missing(team_id, portal_id, state, config)
            .await
    }

    async fn persist_avatar_governance_events(
        &self,
        events: Vec<AvatarGovernanceEventDoc>,
    ) -> Result<u64, mongodb::error::Error> {
        if events.is_empty() {
            return Ok(0);
        }
        let inserted = events.len() as u64;
        let _ = self
            .avatar_governance_events()
            .insert_many(events, None)
            .await?;
        Ok(inserted)
    }

    async fn refresh_avatar_governance_projection_after_write(
        &self,
        payload: &AvatarGovernanceStatePayload,
    ) -> Result<Option<u64>, mongodb::error::Error> {
        let matched_portals = self
            .sync_avatar_governance_payload_to_portal(payload)
            .await?;
        let _ = self
            .sync_avatar_instance_projections(&payload.team_id)
            .await?;
        Ok(matched_portals)
    }

    fn derive_avatar_governance_payload_from_portal_doc(
        team_id: &str,
        portal_id: &str,
        portal_doc: &Document,
    ) -> AvatarGovernanceStatePayload {
        let updated_at = portal_doc
            .get_datetime("updated_at")
            .ok()
            .map(|value| value.to_chrono())
            .unwrap_or_else(Utc::now);
        Self::governance_payload_from_portal_doc(team_id, portal_id, portal_doc, updated_at)
    }

    fn sort_and_limit_avatar_governance_events(
        mut events: Vec<AvatarGovernanceEventPayload>,
        limit: usize,
    ) -> Vec<AvatarGovernanceEventPayload> {
        events.sort_by(|left, right| {
            right
                .created_at
                .cmp(&left.created_at)
                .then_with(|| right.event_id.cmp(&left.event_id))
        });
        events.truncate(limit);
        events
    }

    async fn derive_avatar_governance_event_payloads(
        &self,
        team_id: &str,
        portal_id: &str,
    ) -> Result<Vec<AvatarGovernanceEventPayload>, mongodb::error::Error> {
        let payload = self.get_avatar_governance_state(team_id, portal_id).await?;
        Ok(Self::governance_event_payloads_from_snapshot(
            team_id,
            portal_id,
            &payload.state,
            &payload.config,
            payload.updated_at,
        ))
    }

    async fn derive_team_avatar_governance_event_payloads(
        &self,
        team_id: &str,
        portal_id: Option<&str>,
        excluded_portal_ids: &HashSet<String>,
    ) -> Result<Vec<AvatarGovernanceEventPayload>, mongodb::error::Error> {
        let scoped_portal_id = portal_id
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        let mut resolved_portal_ids = excluded_portal_ids.clone();
        let mut derived = Vec::new();

        let mut state_filter = doc! { "team_id": team_id };
        if let Some(portal_id) = scoped_portal_id.as_deref() {
            state_filter.insert("portal_id", portal_id);
        }
        let state_docs: Vec<AvatarGovernanceStateDoc> = self
            .avatar_governance_states()
            .find(state_filter, None)
            .await?
            .try_collect()
            .await?;
        for state_doc in state_docs {
            if !resolved_portal_ids.insert(state_doc.portal_id.clone()) {
                continue;
            }
            derived.extend(Self::governance_event_payloads_from_snapshot(
                &state_doc.team_id,
                &state_doc.portal_id,
                &state_doc.state,
                &state_doc.config,
                state_doc.updated_at,
            ));
        }

        let team_oid = match ObjectId::parse_str(team_id) {
            Ok(value) => value,
            Err(_) => return Ok(derived),
        };
        let mut portal_filter = Self::avatar_portal_filter(Some(team_oid));
        if let Some(portal_id) = scoped_portal_id.as_deref() {
            let portal_oid = match ObjectId::parse_str(portal_id) {
                Ok(value) => value,
                Err(_) => return Ok(derived),
            };
            portal_filter.insert("_id", portal_oid);
        }
        let portal_docs: Vec<Document> = self
            .portals()
            .find(portal_filter, None)
            .await?
            .try_collect()
            .await?;
        for portal_doc in portal_docs {
            let Some(portal_oid) = portal_doc.get_object_id("_id").ok() else {
                continue;
            };
            let portal_id = portal_oid.to_hex();
            if !resolved_portal_ids.insert(portal_id.clone()) {
                continue;
            }
            let payload = Self::derive_avatar_governance_payload_from_portal_doc(
                team_id,
                &portal_id,
                &portal_doc,
            );
            derived.extend(Self::governance_event_payloads_from_snapshot(
                team_id,
                &portal_id,
                &payload.state,
                &payload.config,
                payload.updated_at,
            ));
        }

        Ok(derived)
    }

    fn binding_issue_message(
        issue: &AvatarBindingIssueKind,
        explicit_coding_agent_id: Option<&str>,
        explicit_service_agent_id: Option<&str>,
        effective_manager_agent_id: Option<&str>,
        effective_service_agent_id: Option<&str>,
        manager_agent: Option<&TeamAgentDoc>,
        service_agent: Option<&TeamAgentDoc>,
    ) -> String {
        match issue {
            AvatarBindingIssueKind::MissingExplicitManagerBinding => {
                "portal 缺少显式 coding_agent_id 绑定".to_string()
            }
            AvatarBindingIssueKind::MissingExplicitServiceBinding => {
                "portal 缺少显式 service_agent_id 绑定".to_string()
            }
            AvatarBindingIssueKind::MissingManagerBinding => "无法解析有效 manager agent".to_string(),
            AvatarBindingIssueKind::MissingServiceBinding => "无法解析有效 service agent".to_string(),
            AvatarBindingIssueKind::ManagerAgentNotFound => format!(
                "manager agent '{}' 在团队中不存在",
                effective_manager_agent_id.unwrap_or(explicit_coding_agent_id.unwrap_or(""))
            ),
            AvatarBindingIssueKind::ServiceAgentNotFound => format!(
                "service agent '{}' 在团队中不存在",
                effective_service_agent_id.unwrap_or(explicit_service_agent_id.unwrap_or(""))
            ),
            AvatarBindingIssueKind::ManagerRoleMismatch => format!(
                "manager agent '{}' 是 {}:{}，不是 digital_avatar:manager",
                effective_manager_agent_id.unwrap_or(explicit_coding_agent_id.unwrap_or("")),
                manager_agent
                    .and_then(|agent| agent.agent_domain.as_deref())
                    .unwrap_or("general"),
                manager_agent
                    .and_then(|agent| agent.agent_role.as_deref())
                    .unwrap_or("default")
            ),
            AvatarBindingIssueKind::ServiceRoleMismatch => format!(
                "service agent '{}' 是 {}:{}，不是 digital_avatar:service",
                effective_service_agent_id.unwrap_or(explicit_service_agent_id.unwrap_or("")),
                service_agent
                    .and_then(|agent| agent.agent_domain.as_deref())
                    .unwrap_or("general"),
                service_agent
                    .and_then(|agent| agent.agent_role.as_deref())
                    .unwrap_or("default")
            ),
            AvatarBindingIssueKind::OwnerManagerMismatch => format!(
                "service agent '{}' 的 owner_manager_agent_id 是 '{}'，但当前 manager agent 是 '{}'",
                effective_service_agent_id.unwrap_or(explicit_service_agent_id.unwrap_or("")),
                service_agent
                    .and_then(|agent| agent.owner_manager_agent_id.as_deref())
                    .unwrap_or(""),
                effective_manager_agent_id.unwrap_or(explicit_coding_agent_id.unwrap_or(""))
            ),
            AvatarBindingIssueKind::SameAgentReused => format!(
                "manager agent '{}' 和 service agent '{}' 指向了同一个 agent",
                effective_manager_agent_id.unwrap_or(explicit_coding_agent_id.unwrap_or("")),
                effective_service_agent_id.unwrap_or(explicit_service_agent_id.unwrap_or(""))
            ),
        }
    }

    fn binding_issue_messages(
        issues: &[AvatarBindingIssueKind],
        explicit_coding_agent_id: Option<&str>,
        explicit_service_agent_id: Option<&str>,
        effective_manager_agent_id: Option<&str>,
        effective_service_agent_id: Option<&str>,
        manager_agent: Option<&TeamAgentDoc>,
        service_agent: Option<&TeamAgentDoc>,
    ) -> Vec<String> {
        issues
            .iter()
            .map(|issue| {
                Self::binding_issue_message(
                    issue,
                    explicit_coding_agent_id,
                    explicit_service_agent_id,
                    effective_manager_agent_id,
                    effective_service_agent_id,
                    manager_agent,
                    service_agent,
                )
            })
            .collect()
    }

    fn governance_event_payloads_from_snapshot(
        team_id: &str,
        portal_id: &str,
        state: &serde_json::Value,
        config: &serde_json::Value,
        created_at: DateTime<Utc>,
    ) -> Vec<AvatarGovernanceEventPayload> {
        Self::diff_governance_events_at(
            portal_id,
            team_id,
            &Self::default_governance_state_json(),
            &Self::default_governance_config_json(),
            state,
            config,
            None,
            Some("system"),
            created_at,
        )
        .into_iter()
        .map(Into::into)
        .collect()
    }

    fn avatar_read_side_effect_audit_items() -> Vec<AvatarReadSideEffectAuditItem> {
        vec![
            AvatarReadSideEffectAuditItem {
                operation: "get_avatar_governance_state".to_string(),
                file: "crates/agime-team-server/src/agent/service_mongo.rs".to_string(),
                side_effects: vec![
                    "no database writes; derives payload from portal doc when state doc is missing"
                        .to_string(),
                ],
            },
            AvatarReadSideEffectAuditItem {
                operation: "list_avatar_governance_queue".to_string(),
                file: "crates/agime-team-server/src/agent/service_mongo.rs".to_string(),
                side_effects: vec![
                    "no database writes; reuses derived governance payload fallback".to_string(),
                ],
            },
            AvatarReadSideEffectAuditItem {
                operation: "list_avatar_instance_projections".to_string(),
                file: "crates/agime-team-server/src/agent/service_mongo.rs".to_string(),
                side_effects: vec![
                    "no database writes; derives projection rows from portals and governance state"
                        .to_string(),
                ],
            },
            AvatarReadSideEffectAuditItem {
                operation: "get_avatar_workbench_snapshot".to_string(),
                file: "crates/agime-team-server/src/agent/service_mongo.rs".to_string(),
                side_effects: vec![
                    "no database writes for derived reports; merges persisted non-derived reports in memory"
                        .to_string(),
                    "reads governance queues through derived state fallback without writing"
                        .to_string(),
                ],
            },
        ]
    }

    fn governance_entity_snapshots_from_state(
        state: &serde_json::Value,
    ) -> HashMap<String, GovernanceEntitySnapshot> {
        fn first_text(item: &serde_json::Map<String, serde_json::Value>, keys: &[&str]) -> String {
            keys.iter()
                .find_map(|key| item.get(*key).and_then(serde_json::Value::as_str))
                .unwrap_or("")
                .trim()
                .to_string()
        }

        let mut entries = HashMap::new();
        let Some(root) = state.as_object() else {
            return entries;
        };

        let specs: [(&str, &str, &[&str]); 4] = [
            (
                "capabilityRequests",
                "capability",
                &["detail", "decisionReason", "requestedScope"],
            ),
            (
                "gapProposals",
                "proposal",
                &["description", "decisionReason", "expectedGain"],
            ),
            (
                "optimizationTickets",
                "ticket",
                &["proposal", "decisionReason", "evidence", "expectedGain"],
            ),
            (
                "runtimeLogs",
                "runtime",
                &["proposal", "evidence", "expectedGain"],
            ),
        ];

        for (array_key, entity_type, detail_keys) in specs {
            let Some(items) = root.get(array_key).and_then(serde_json::Value::as_array) else {
                continue;
            };
            for item in items {
                let Some(map) = item.as_object() else {
                    continue;
                };
                let id = map
                    .get("id")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("")
                    .trim()
                    .to_string();
                let title = map
                    .get("title")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("")
                    .trim()
                    .to_string();
                if id.is_empty() || title.is_empty() {
                    continue;
                }

                let status = map
                    .get("status")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("")
                    .trim()
                    .to_string();
                let detail = first_text(map, detail_keys);
                let key = format!("{entity_type}:{id}");
                entries.insert(
                    key,
                    GovernanceEntitySnapshot {
                        entity_type,
                        id,
                        title,
                        status,
                        detail,
                        meta: serde_json::Value::Object(map.clone()),
                    },
                );
            }
        }

        entries
    }

    fn diff_governance_events_at(
        portal_id: &str,
        team_id: &str,
        current_state: &serde_json::Value,
        current_config: &serde_json::Value,
        next_state: &serde_json::Value,
        next_config: &serde_json::Value,
        actor_id: Option<&str>,
        actor_name: Option<&str>,
        created_at: DateTime<Utc>,
    ) -> Vec<AvatarGovernanceEventDoc> {
        let current_entries = Self::governance_entity_snapshots_from_state(current_state);
        let next_entries = Self::governance_entity_snapshots_from_state(next_state);
        let mut events = Vec::new();

        for (key, next_entry) in &next_entries {
            match current_entries.get(key) {
                None => events.push(AvatarGovernanceEventDoc {
                    id: None,
                    portal_id: portal_id.to_string(),
                    team_id: team_id.to_string(),
                    event_type: "created".to_string(),
                    entity_type: next_entry.entity_type.to_string(),
                    entity_id: Some(next_entry.id.clone()),
                    title: next_entry.title.clone(),
                    status: Some(next_entry.status.clone()),
                    detail: (!next_entry.detail.is_empty()).then(|| next_entry.detail.clone()),
                    actor_id: actor_id.map(str::to_string),
                    actor_name: actor_name.map(str::to_string),
                    meta: next_entry.meta.clone(),
                    created_at,
                }),
                Some(current_entry)
                    if current_entry.status != next_entry.status
                        || current_entry.title != next_entry.title
                        || current_entry.detail != next_entry.detail
                        || current_entry.meta != next_entry.meta =>
                {
                    events.push(AvatarGovernanceEventDoc {
                        id: None,
                        portal_id: portal_id.to_string(),
                        team_id: team_id.to_string(),
                        event_type: "updated".to_string(),
                        entity_type: next_entry.entity_type.to_string(),
                        entity_id: Some(next_entry.id.clone()),
                        title: next_entry.title.clone(),
                        status: Some(next_entry.status.clone()),
                        detail: (!next_entry.detail.is_empty()).then(|| next_entry.detail.clone()),
                        actor_id: actor_id.map(str::to_string),
                        actor_name: actor_name.map(str::to_string),
                        meta: serde_json::json!({
                            "before": current_entry.meta,
                            "after": next_entry.meta,
                        }),
                        created_at,
                    });
                }
                Some(_) => {}
            }
        }

        for (key, current_entry) in &current_entries {
            if next_entries.contains_key(key) {
                continue;
            }
            events.push(AvatarGovernanceEventDoc {
                id: None,
                portal_id: portal_id.to_string(),
                team_id: team_id.to_string(),
                event_type: "removed".to_string(),
                entity_type: current_entry.entity_type.to_string(),
                entity_id: Some(current_entry.id.clone()),
                title: current_entry.title.clone(),
                status: Some(current_entry.status.clone()),
                detail: (!current_entry.detail.is_empty()).then(|| current_entry.detail.clone()),
                actor_id: actor_id.map(str::to_string),
                actor_name: actor_name.map(str::to_string),
                meta: current_entry.meta.clone(),
                created_at,
            });
        }

        if current_config != next_config {
            events.push(AvatarGovernanceEventDoc {
                id: None,
                portal_id: portal_id.to_string(),
                team_id: team_id.to_string(),
                event_type: "config_updated".to_string(),
                entity_type: "config".to_string(),
                entity_id: None,
                title: "治理配置已更新".to_string(),
                status: None,
                detail: None,
                actor_id: actor_id.map(str::to_string),
                actor_name: actor_name.map(str::to_string),
                meta: serde_json::json!({
                    "before": current_config,
                    "after": next_config,
                }),
                created_at,
            });
        }

        events
    }

    fn diff_governance_events(
        portal_id: &str,
        team_id: &str,
        current_state: &serde_json::Value,
        current_config: &serde_json::Value,
        next_state: &serde_json::Value,
        next_config: &serde_json::Value,
        actor_id: Option<&str>,
        actor_name: Option<&str>,
    ) -> Vec<AvatarGovernanceEventDoc> {
        Self::diff_governance_events_at(
            portal_id,
            team_id,
            current_state,
            current_config,
            next_state,
            next_config,
            actor_id,
            actor_name,
            Utc::now(),
        )
    }

    fn governance_queue_items_from_state(
        state: &serde_json::Value,
    ) -> Vec<AvatarGovernanceQueueItemPayload> {
        let mut rows = Vec::new();
        let Some(root) = state.as_object() else {
            return rows;
        };

        if let Some(items) = root
            .get("capabilityRequests")
            .and_then(serde_json::Value::as_array)
        {
            for item in items {
                let Some(map) = item.as_object() else {
                    continue;
                };
                let status = map
                    .get("status")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("pending");
                if status != "pending" && status != "needs_human" {
                    continue;
                }
                let source_id = map
                    .get("id")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let title = map
                    .get("title")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("")
                    .to_string();
                if source_id.is_empty() || title.is_empty() {
                    continue;
                }
                let detail = map
                    .get("detail")
                    .and_then(serde_json::Value::as_str)
                    .or_else(|| {
                        map.get("decisionReason")
                            .and_then(serde_json::Value::as_str)
                    })
                    .unwrap_or("")
                    .to_string();
                let ts = map
                    .get("updatedAt")
                    .and_then(serde_json::Value::as_str)
                    .or_else(|| map.get("createdAt").and_then(serde_json::Value::as_str))
                    .unwrap_or("")
                    .to_string();
                let mut meta = Vec::new();
                if let Some(risk) = map.get("risk").and_then(serde_json::Value::as_str) {
                    meta.push(risk.to_string());
                }
                if let Some(source) = map.get("source").and_then(serde_json::Value::as_str) {
                    meta.push(source.to_string());
                }
                rows.push(AvatarGovernanceQueueItemPayload {
                    id: format!("queue:capability:{source_id}"),
                    kind: "capability".to_string(),
                    title,
                    detail,
                    status: status.to_string(),
                    ts,
                    meta,
                    source_id,
                });
            }
        }

        if let Some(items) = root
            .get("gapProposals")
            .and_then(serde_json::Value::as_array)
        {
            for item in items {
                let Some(map) = item.as_object() else {
                    continue;
                };
                let status = map
                    .get("status")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("draft");
                if status != "pending_approval" && status != "approved" && status != "pilot" {
                    continue;
                }
                let source_id = map
                    .get("id")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let title = map
                    .get("title")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("")
                    .to_string();
                if source_id.is_empty() || title.is_empty() {
                    continue;
                }
                let detail = map
                    .get("description")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let ts = map
                    .get("updatedAt")
                    .and_then(serde_json::Value::as_str)
                    .or_else(|| map.get("createdAt").and_then(serde_json::Value::as_str))
                    .unwrap_or("")
                    .to_string();
                let meta = map
                    .get("expectedGain")
                    .and_then(serde_json::Value::as_str)
                    .map(|value| vec![value.to_string()])
                    .unwrap_or_default();
                let mut meta = meta;
                if let Some(reason) = map
                    .get("decisionReason")
                    .and_then(serde_json::Value::as_str)
                    .filter(|value| !value.trim().is_empty())
                {
                    meta.push(format!("决策说明: {}", reason.trim()));
                }
                rows.push(AvatarGovernanceQueueItemPayload {
                    id: format!("queue:proposal:{source_id}"),
                    kind: "proposal".to_string(),
                    title,
                    detail,
                    status: status.to_string(),
                    ts,
                    meta,
                    source_id,
                });
            }
        }

        if let Some(items) = root
            .get("optimizationTickets")
            .and_then(serde_json::Value::as_array)
        {
            for item in items {
                let Some(map) = item.as_object() else {
                    continue;
                };
                let status = map
                    .get("status")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("pending");
                if status != "pending" && status != "approved" && status != "experimenting" {
                    continue;
                }
                let source_id = map
                    .get("id")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let title = map
                    .get("title")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("")
                    .to_string();
                if source_id.is_empty() || title.is_empty() {
                    continue;
                }
                let detail = map
                    .get("proposal")
                    .and_then(serde_json::Value::as_str)
                    .or_else(|| map.get("evidence").and_then(serde_json::Value::as_str))
                    .unwrap_or("")
                    .to_string();
                let ts = map
                    .get("updatedAt")
                    .and_then(serde_json::Value::as_str)
                    .or_else(|| map.get("createdAt").and_then(serde_json::Value::as_str))
                    .unwrap_or("")
                    .to_string();
                let mut meta = Vec::new();
                if let Some(problem_type) =
                    map.get("problemType").and_then(serde_json::Value::as_str)
                {
                    meta.push(problem_type.to_string());
                }
                if let Some(risk) = map.get("risk").and_then(serde_json::Value::as_str) {
                    meta.push(risk.to_string());
                }
                if let Some(reason) = map
                    .get("decisionReason")
                    .and_then(serde_json::Value::as_str)
                    .filter(|value| !value.trim().is_empty())
                {
                    meta.push(format!("决策说明: {}", reason.trim()));
                }
                rows.push(AvatarGovernanceQueueItemPayload {
                    id: format!("queue:ticket:{source_id}"),
                    kind: "ticket".to_string(),
                    title,
                    detail,
                    status: status.to_string(),
                    ts,
                    meta,
                    source_id,
                });
            }
        }

        rows.sort_by(|a, b| b.ts.cmp(&a.ts));
        rows
    }

    fn avatar_portal_filter(team_oid: Option<ObjectId>) -> Document {
        let mut filter = doc! {
            "is_deleted": { "$ne": true },
            "$or": [
                { "domain": "avatar" },
                { "tags": "digital-avatar" },
                { "tags": { "$regex": "^avatar:", "$options": "i" } }
            ]
        };
        if let Some(value) = team_oid {
            filter.insert("team_id", value);
        }
        filter
    }

    fn is_avatar_portal_doc(doc: &Document) -> bool {
        if Self::doc_has_string(doc, "domain", "avatar")
            || Self::nested_doc_has_string(doc, &["settings", "domain"], "avatar")
        {
            return true;
        }

        doc.get_array("tags").ok().is_some_and(|tags| {
            tags.iter()
                .filter_map(Bson::as_str)
                .map(str::trim)
                .any(|tag| {
                    tag.eq_ignore_ascii_case("digital-avatar")
                        || tag.to_ascii_lowercase().starts_with("avatar:")
                })
        })
    }

    async fn seed_avatar_governance_events_if_missing(
        &self,
        team_id: &str,
        portal_id: &str,
        state: &serde_json::Value,
        config: &serde_json::Value,
    ) -> Result<u64, mongodb::error::Error> {
        let existing = self
            .avatar_governance_events()
            .count_documents(doc! { "team_id": team_id, "portal_id": portal_id }, None)
            .await?;
        if existing > 0 {
            return Ok(0);
        }

        let events = Self::diff_governance_events(
            portal_id,
            team_id,
            &Self::default_governance_state_json(),
            &Self::default_governance_config_json(),
            state,
            config,
            None,
            Some("system"),
        );
        if events.is_empty() {
            return Ok(0);
        }

        self.persist_avatar_governance_events(events).await
    }

    fn avatar_instance_from_portal_doc(team_id: &str, doc: &Document) -> Option<AvatarInstanceDoc> {
        let portal_id = doc.get_object_id("_id").ok()?.to_hex();
        let slug = Self::doc_string(doc, "slug")?;
        let name = Self::doc_string(doc, "name")?;
        let status = Self::doc_string(doc, "status").unwrap_or_else(|| "draft".to_string());
        let document_access_mode = Self::doc_string(doc, "document_access_mode")
            .unwrap_or_else(|| "read_only".to_string());
        let portal_updated_at = doc
            .get_datetime("updated_at")
            .ok()
            .map(|value| value.to_chrono())
            .unwrap_or_else(Utc::now);

        Some(AvatarInstanceDoc {
            id: None,
            portal_id,
            team_id: team_id.to_string(),
            slug,
            name,
            status,
            avatar_type: Self::avatar_type_from_doc(doc),
            manager_agent_id: Self::extract_portal_manager_id(doc),
            service_agent_id: Self::extract_portal_service_agent_id(doc),
            document_access_mode,
            governance_counts: Self::governance_counts_from_doc(doc),
            portal_updated_at,
            projected_at: Utc::now(),
        })
    }

    async fn infer_legacy_avatar_metadata(
        &self,
        team_id: &str,
        agent: &TeamAgent,
    ) -> Result<Option<(String, Option<String>)>, mongodb::error::Error> {
        let manager_candidate = Self::is_legacy_avatar_manager_candidate(agent);
        let service_candidate = Self::is_legacy_avatar_service_candidate(agent);
        if !manager_candidate && !service_candidate {
            return Ok(None);
        }

        let team_oid = match ObjectId::parse_str(team_id) {
            Ok(v) => v,
            Err(_) => return Ok(None),
        };
        let agent_id = agent.id.trim();
        if agent_id.is_empty() {
            return Ok(None);
        }

        let binding_filter = doc! {
            "$or": [
                { "coding_agent_id": agent_id },
                { "service_agent_id": agent_id },
                { "agent_id": agent_id },
                { "tags": format!("manager:{agent_id}") },
                { "settings.managerAgentId": agent_id },
                { "settings.managerGroupId": agent_id },
                { "settings.serviceRuntimeAgentId": agent_id }
            ]
        };
        let filter = doc! {
            "$and": [
                Self::avatar_portal_filter(Some(team_oid)),
                binding_filter,
            ]
        };

        let mut cursor = self.portals().find(filter, None).await?;
        let mut manager_match = false;
        let mut service_owner_manager_id: Option<String> = None;

        while let Some(portal) = cursor.try_next().await? {
            let explicit_manager_match = Self::doc_has_string(&portal, "coding_agent_id", agent_id)
                || Self::doc_has_manager_tag(&portal, agent_id)
                || Self::nested_doc_has_string(&portal, &["settings", "managerAgentId"], agent_id)
                || Self::nested_doc_has_string(&portal, &["settings", "managerGroupId"], agent_id);
            let explicit_service_match =
                Self::doc_has_string(&portal, "service_agent_id", agent_id)
                    || Self::nested_doc_has_string(
                        &portal,
                        &["settings", "serviceRuntimeAgentId"],
                        agent_id,
                    );
            let legacy_single_agent_match = Self::doc_has_string(&portal, "agent_id", agent_id);

            if manager_candidate && (explicit_manager_match || legacy_single_agent_match) {
                manager_match = true;
                break;
            }

            if service_candidate && (explicit_service_match || legacy_single_agent_match) {
                service_owner_manager_id = Self::extract_portal_manager_id(&portal)
                    .filter(|value| value.trim() != agent_id);
            }
        }

        if manager_match {
            return Ok(Some(("manager".to_string(), None)));
        }
        if service_candidate && service_owner_manager_id.is_some() {
            return Ok(Some(("service".to_string(), service_owner_manager_id)));
        }
        if service_candidate {
            let has_orphan_service_hint =
                Self::is_dedicated_avatar_marker(agent.description.as_deref())
                    || Self::normalize_compact_text(Some(agent.name.as_str()))
                        .ends_with("分身agent");
            if has_orphan_service_hint {
                return Ok(Some(("service".to_string(), None)));
            }
        }

        Ok(None)
    }

    async fn backfill_legacy_avatar_agent_metadata(
        &self,
        agent: &mut TeamAgent,
    ) -> Result<(), mongodb::error::Error> {
        let explicit_role = Self::explicit_avatar_marker_role(agent.description.as_deref());
        if agent.agent_domain.as_deref() == Some("digital_avatar") && agent.agent_role.is_some() {
            if explicit_role.is_none() || explicit_role == agent.agent_role.as_deref() {
                return Ok(());
            }
        }

        let inferred = self
            .infer_legacy_avatar_metadata(&agent.team_id, agent)
            .await?;
        let (role, owner_manager_agent_id) = match explicit_role {
            Some("manager") => ("manager".to_string(), None),
            Some("service") => inferred.unwrap_or_else(|| ("service".to_string(), None)),
            Some(other) => (other.to_string(), None),
            None => {
                let Some(pair) = inferred else {
                    return Ok(());
                };
                pair
            }
        };

        let mut set_doc = doc! {
            "agent_domain": "digital_avatar",
            "agent_role": role.clone(),
            "updated_at": bson::DateTime::from_chrono(Utc::now()),
        };
        match owner_manager_agent_id.clone() {
            Some(value) => {
                set_doc.insert("owner_manager_agent_id", value);
            }
            None => {
                set_doc.insert("owner_manager_agent_id", Bson::Null);
            }
        }

        self.agents()
            .update_one(
                doc! { "agent_id": &agent.id },
                doc! { "$set": set_doc },
                None,
            )
            .await?;

        agent.agent_domain = Some("digital_avatar".to_string());
        agent.agent_role = Some(role);
        agent.owner_manager_agent_id = owner_manager_agent_id;
        Ok(())
    }

    /// Whether an agent is considered a digital-avatar dedicated agent and therefore
    /// must not be reused as a generic provision template.
    pub async fn is_dedicated_avatar_agent(
        &self,
        team_id: &str,
        agent: &TeamAgent,
    ) -> Result<bool, mongodb::error::Error> {
        if agent.agent_domain.as_deref() == Some("digital_avatar") {
            return Ok(true);
        }
        if Self::is_legacy_avatar_manager_candidate(agent)
            || Self::is_legacy_avatar_service_candidate(agent)
        {
            return Ok(self
                .infer_legacy_avatar_metadata(team_id, agent)
                .await?
                .is_some());
        }
        Ok(false)
    }

    // Permission checks - query embedded members array in teams collection
    pub async fn is_team_member(
        &self,
        user_id: &str,
        team_id: &str,
    ) -> Result<bool, mongodb::error::Error> {
        // team_id is ObjectId string
        let oid = match mongodb::bson::oid::ObjectId::parse_str(team_id) {
            Ok(oid) => oid,
            Err(_) => return Ok(false),
        };
        let result = self
            .teams()
            .find_one(
                doc! {
                    "_id": oid,
                    "members.user_id": user_id
                },
                None,
            )
            .await?;
        Ok(result.is_some())
    }

    pub async fn is_team_admin(
        &self,
        user_id: &str,
        team_id: &str,
    ) -> Result<bool, mongodb::error::Error> {
        // team_id is ObjectId string
        let oid = match mongodb::bson::oid::ObjectId::parse_str(team_id) {
            Ok(oid) => oid,
            Err(_) => return Ok(false),
        };
        let result = self
            .teams()
            .find_one(
                doc! {
                    "_id": oid,
                    "members": {
                        "$elemMatch": {
                            "user_id": user_id,
                            "role": { "$in": ["admin", "owner"] }
                        }
                    }
                },
                None,
            )
            .await?;
        Ok(result.is_some())
    }

    pub async fn get_agent_team_id(
        &self,
        agent_id: &str,
    ) -> Result<Option<String>, mongodb::error::Error> {
        let result = self
            .agents()
            .find_one(doc! { "agent_id": agent_id }, None)
            .await?;
        Ok(result.map(|r| r.team_id))
    }

    pub async fn get_task_team_id(
        &self,
        task_id: &str,
    ) -> Result<Option<String>, mongodb::error::Error> {
        let result = self
            .tasks()
            .find_one(doc! { "task_id": task_id }, None)
            .await?;
        Ok(result.map(|r| r.team_id))
    }

    // Agent CRUD operations
    pub async fn create_agent(&self, req: CreateAgentRequest) -> Result<TeamAgent, ServiceError> {
        validate_name(&req.name)?;
        validate_api_url(&req.api_url)?;
        validate_model(&req.model)?;

        let id = Uuid::new_v4().to_string();
        let now = Utc::now();
        let api_format = req.api_format.as_deref().unwrap_or("openai");

        let doc = TeamAgentDoc {
            id: None,
            agent_id: id.clone(),
            team_id: req.team_id,
            name: req.name,
            description: req.description,
            avatar: req.avatar,
            system_prompt: req.system_prompt,
            api_url: req.api_url,
            model: req.model,
            api_key: req.api_key,
            api_format: api_format.to_string(),
            status: "idle".to_string(),
            last_error: None,
            enabled_extensions: req.enabled_extensions.unwrap_or_else(|| {
                BuiltinExtension::defaults()
                    .into_iter()
                    .map(|ext| AgentExtensionConfig {
                        extension: ext,
                        enabled: true,
                    })
                    .collect()
            }),
            custom_extensions: req.custom_extensions.unwrap_or_default(),
            agent_domain: req.agent_domain,
            agent_role: req.agent_role,
            owner_manager_agent_id: req.owner_manager_agent_id,
            template_source_agent_id: req.template_source_agent_id,
            allowed_groups: req.allowed_groups.unwrap_or_default(),
            max_concurrent_tasks: req.max_concurrent_tasks.unwrap_or(1),
            temperature: req.temperature,
            max_tokens: req.max_tokens,
            context_limit: req.context_limit,
            thinking_enabled: req.thinking_enabled.unwrap_or(true),
            assigned_skills: req.assigned_skills.unwrap_or_default(),
            auto_approve_chat: true,
            created_at: now,
            updated_at: now,
        };

        // Insert via raw BSON document to preserve custom extension envs.
        let mut insert_doc = mongodb::bson::to_document(&doc)
            .map_err(|e| ServiceError::Internal(format!("Serialize agent doc failed: {}", e)))?;
        insert_doc.insert(
            "custom_extensions",
            custom_extensions_to_bson(&doc.custom_extensions),
        );
        self.db
            .collection::<Document>("team_agents")
            .insert_one(insert_doc, None)
            .await?;
        self.get_agent(&id)
            .await?
            .ok_or_else(|| ServiceError::Internal(format!("Agent not found after insert: {}", id)))
    }

    pub async fn get_agent(&self, id: &str) -> Result<Option<TeamAgent>, mongodb::error::Error> {
        let doc = self
            .agents()
            .find_one(doc! { "agent_id": id }, None)
            .await?;
        let Some(doc) = doc else {
            return Ok(None);
        };
        let mut agent: TeamAgent = doc.into();
        self.backfill_legacy_avatar_agent_metadata(&mut agent)
            .await?;
        Ok(Some(agent))
    }

    /// Get agent with API key preserved (for internal server-side use only, never expose to API).
    pub async fn get_agent_with_key(
        &self,
        id: &str,
    ) -> Result<Option<TeamAgent>, mongodb::error::Error> {
        let doc = self
            .agents()
            .find_one(doc! { "agent_id": id }, None)
            .await?;
        let Some(doc) = doc else {
            return Ok(None);
        };
        let mut agent = agent_doc_with_key(doc);
        self.backfill_legacy_avatar_agent_metadata(&mut agent)
            .await?;
        Ok(Some(agent))
    }

    /// Get the first agent for a team that has an API key configured (for internal server-side use).
    pub async fn get_first_agent_with_key(
        &self,
        team_id: &str,
    ) -> Result<Option<TeamAgent>, mongodb::error::Error> {
        let options = mongodb::options::FindOneOptions::builder()
            .sort(doc! { "created_at": -1 })
            .build();
        let doc = self
            .agents()
            .find_one(
                doc! {
                    "team_id": team_id,
                    "api_key": { "$exists": true, "$nin": [null, ""] }
                },
                options,
            )
            .await?;
        Ok(doc.map(agent_doc_with_key))
    }

    pub async fn list_agents(
        &self,
        query: ListAgentsQuery,
    ) -> Result<PaginatedResponse<TeamAgent>, mongodb::error::Error> {
        let clamped_limit = query.limit.min(100);
        let limit = clamped_limit as i64;
        let skip = ((query.page.saturating_sub(1)) * clamped_limit) as u64;

        let filter = doc! { "team_id": &query.team_id };
        let total = self.agents().count_documents(filter.clone(), None).await?;

        let options = mongodb::options::FindOptions::builder()
            .sort(doc! { "created_at": -1 })
            .skip(skip)
            .limit(limit)
            .build();
        let cursor = self.agents().find(filter, options).await?;

        let docs: Vec<TeamAgentDoc> = cursor.try_collect().await?;
        let mut items: Vec<TeamAgent> = docs.into_iter().map(|d| d.into()).collect();
        for agent in &mut items {
            self.backfill_legacy_avatar_agent_metadata(agent).await?;
        }

        Ok(PaginatedResponse::new(
            items,
            total,
            query.page,
            query.limit,
        ))
    }

    pub async fn sync_avatar_instance_projections(
        &self,
        team_id: &str,
    ) -> Result<Vec<AvatarInstanceSummary>, mongodb::error::Error> {
        let projection_docs = self.derive_avatar_instance_projection_docs(team_id).await?;
        let mut projections = Vec::with_capacity(projection_docs.len());
        for projection in projection_docs {
            let portal_id = projection.portal_id.clone();
            self.avatar_instances()
                .replace_one(
                    doc! { "portal_id": &portal_id, "team_id": team_id },
                    projection.clone(),
                    mongodb::options::ReplaceOptions::builder()
                        .upsert(true)
                        .build(),
                )
                .await?;
            projections.push(AvatarInstanceSummary::from(projection));
        }

        Ok(projections)
    }

    async fn derive_avatar_instance_projection_docs(
        &self,
        team_id: &str,
    ) -> Result<Vec<AvatarInstanceDoc>, mongodb::error::Error> {
        let team_oid = match ObjectId::parse_str(team_id) {
            Ok(v) => v,
            Err(_) => return Ok(Vec::new()),
        };

        let filter = Self::avatar_portal_filter(Some(team_oid));
        let options = mongodb::options::FindOptions::builder()
            .sort(doc! { "updated_at": -1 })
            .build();
        let cursor = self.portals().find(filter, options).await?;
        let portal_docs: Vec<Document> = cursor.try_collect().await?;
        let governance_docs: Vec<AvatarGovernanceStateDoc> = self
            .avatar_governance_states()
            .find(doc! { "team_id": team_id }, None)
            .await?
            .try_collect()
            .await?;
        let governance_counts_by_portal: std::collections::HashMap<String, AvatarGovernanceCounts> =
            governance_docs
                .into_iter()
                .map(|doc| {
                    (
                        doc.portal_id,
                        Self::governance_counts_from_state_json(&doc.state),
                    )
                })
                .collect();
        let mut projections = Vec::with_capacity(portal_docs.len());

        for portal_doc in portal_docs {
            let Some(mut projection) = Self::avatar_instance_from_portal_doc(team_id, &portal_doc)
            else {
                continue;
            };
            if let Some(counts) = governance_counts_by_portal.get(&projection.portal_id) {
                projection.governance_counts = counts.clone();
            }
            projections.push(projection);
        }

        Ok(projections)
    }

    pub async fn get_avatar_governance_state(
        &self,
        team_id: &str,
        portal_id: &str,
    ) -> Result<AvatarGovernanceStatePayload, mongodb::error::Error> {
        if let Some(doc) = self
            .avatar_governance_states()
            .find_one(doc! { "team_id": team_id, "portal_id": portal_id }, None)
            .await?
        {
            return Ok(doc.into());
        }

        let portal_oid = match ObjectId::parse_str(portal_id) {
            Ok(value) => value,
            Err(_) => {
                tracing::warn!(
                    team_id = %team_id,
                    portal_id = %portal_id,
                    "avatar governance requested with invalid portal id; returning default payload"
                );
                return Ok(Self::default_avatar_governance_payload(team_id, portal_id));
            }
        };

        let team_oid = match ObjectId::parse_str(team_id) {
            Ok(value) => value,
            Err(_) => {
                tracing::warn!(
                    team_id = %team_id,
                    portal_id = %portal_id,
                    "avatar governance requested with invalid team id; returning default payload"
                );
                return Ok(Self::default_avatar_governance_payload(team_id, portal_id));
            }
        };

        let Some(portal_doc) = self
            .portals()
            .find_one(
                doc! {
                    "_id": portal_oid,
                    "team_id": team_oid,
                    "is_deleted": { "$ne": true }
                },
                None,
            )
            .await?
        else {
            tracing::warn!(
                team_id = %team_id,
                portal_id = %portal_id,
                "avatar governance requested for missing portal; returning default payload"
            );
            return Ok(Self::default_avatar_governance_payload(team_id, portal_id));
        };

        if !Self::is_avatar_portal_doc(&portal_doc) {
            tracing::warn!(
                team_id = %team_id,
                portal_id = %portal_id,
                "avatar governance requested for non-avatar portal; continuing with compatibility fallback"
            );
        }

        Ok(Self::derive_avatar_governance_payload_from_portal_doc(
            team_id,
            portal_id,
            &portal_doc,
        ))
    }

    pub async fn update_avatar_governance_state(
        &self,
        team_id: &str,
        portal_id: &str,
        next_state: Option<serde_json::Value>,
        next_config: Option<serde_json::Value>,
        actor_id: Option<&str>,
        actor_name: Option<&str>,
    ) -> Result<AvatarGovernanceStatePayload, mongodb::error::Error> {
        let current = self.get_avatar_governance_state(team_id, portal_id).await?;
        let previous_state = current.state.clone();
        let previous_config = current.config.clone();
        let state = next_state.unwrap_or(previous_state.clone());
        let config = next_config.unwrap_or(previous_config.clone());
        let updated_at = Utc::now();
        let payload = AvatarGovernanceStatePayload {
            portal_id: portal_id.to_string(),
            team_id: team_id.to_string(),
            state: state.clone(),
            config: config.clone(),
            updated_at,
        };
        let governance_events = Self::diff_governance_events(
            portal_id,
            team_id,
            &previous_state,
            &previous_config,
            &state,
            &config,
            actor_id,
            actor_name,
        );

        self.refresh_avatar_governance_state_read_model(&payload)
            .await?;

        let matched_portals = self
            .refresh_avatar_governance_projection_after_write(&payload)
            .await?;
        if matches!(matched_portals, Some(0)) {
            tracing::warn!(
                team_id = %team_id,
                portal_id = %portal_id,
                "avatar governance state updated without a matching portal document; governance projection may be orphaned"
            );
        }

        if !governance_events.is_empty() {
            let _ = self
                .persist_avatar_governance_events(governance_events)
                .await?;
        }

        Ok(payload)
    }

    pub async fn update_avatar_governance_state_if_current(
        &self,
        current: &AvatarGovernanceStatePayload,
        next_state: Option<serde_json::Value>,
        next_config: Option<serde_json::Value>,
        actor_id: Option<&str>,
        actor_name: Option<&str>,
    ) -> Result<Option<AvatarGovernanceStatePayload>, mongodb::error::Error> {
        let previous_state = current.state.clone();
        let previous_config = current.config.clone();
        let state = next_state.unwrap_or(previous_state.clone());
        let config = next_config.unwrap_or(previous_config.clone());
        let updated_at = Utc::now();
        let payload = AvatarGovernanceStatePayload {
            portal_id: current.portal_id.clone(),
            team_id: current.team_id.clone(),
            state: state.clone(),
            config: config.clone(),
            updated_at,
        };
        let governance_events = Self::diff_governance_events(
            &current.portal_id,
            &current.team_id,
            &previous_state,
            &previous_config,
            &state,
            &config,
            actor_id,
            actor_name,
        );

        let matched_portals = self
            .sync_avatar_governance_payload_to_portal_if_current(&payload, current.updated_at)
            .await?;
        if matches!(matched_portals, Some(0)) {
            return Ok(None);
        }

        self.refresh_avatar_governance_state_read_model(&payload)
            .await?;

        let _ = self
            .sync_avatar_instance_projections(&payload.team_id)
            .await?;

        if !governance_events.is_empty() {
            let _ = self
                .persist_avatar_governance_events(governance_events)
                .await?;
        }

        Ok(Some(payload))
    }

    pub async fn list_avatar_governance_events(
        &self,
        team_id: &str,
        portal_id: &str,
        limit: u32,
    ) -> Result<Vec<AvatarGovernanceEventPayload>, mongodb::error::Error> {
        let limit = limit.clamp(1, 200) as usize;
        let docs: Vec<AvatarGovernanceEventDoc> = self
            .avatar_governance_events()
            .find(
                doc! { "team_id": team_id, "portal_id": portal_id },
                mongodb::options::FindOptions::builder()
                    .sort(doc! { "created_at": -1, "_id": -1 })
                    .limit(limit as i64)
                    .build(),
            )
            .await?
            .try_collect()
            .await?;

        if !docs.is_empty() {
            return Ok(Self::sort_and_limit_avatar_governance_events(
                docs.into_iter().map(Into::into).collect(),
                limit,
            ));
        }

        Ok(Self::sort_and_limit_avatar_governance_events(
            self.derive_avatar_governance_event_payloads(team_id, portal_id)
                .await?,
            limit,
        ))
    }

    pub async fn list_team_avatar_governance_events(
        &self,
        team_id: &str,
        portal_id: Option<&str>,
        limit: u32,
    ) -> Result<Vec<AvatarGovernanceEventPayload>, mongodb::error::Error> {
        let limit = limit.clamp(1, 500) as usize;
        let mut filter = doc! { "team_id": team_id };
        if let Some(value) = portal_id.filter(|value| !value.trim().is_empty()) {
            filter.insert("portal_id", value);
        }

        let docs: Vec<AvatarGovernanceEventDoc> = self
            .avatar_governance_events()
            .find(
                filter,
                mongodb::options::FindOptions::builder()
                    .sort(doc! { "created_at": -1, "_id": -1 })
                    .limit(limit as i64)
                    .build(),
            )
            .await?
            .try_collect()
            .await?;
        let mut events: Vec<AvatarGovernanceEventPayload> =
            docs.into_iter().map(Into::into).collect();
        if events.len() < limit {
            let excluded_portal_ids = events
                .iter()
                .map(|event| event.portal_id.clone())
                .collect::<HashSet<_>>();
            events.extend(
                self.derive_team_avatar_governance_event_payloads(
                    team_id,
                    portal_id,
                    &excluded_portal_ids,
                )
                .await?,
            );
        }

        Ok(Self::sort_and_limit_avatar_governance_events(events, limit))
    }

    pub async fn list_avatar_governance_queue(
        &self,
        team_id: &str,
        portal_id: &str,
    ) -> Result<Vec<AvatarGovernanceQueueItemPayload>, mongodb::error::Error> {
        let payload = self.get_avatar_governance_state(team_id, portal_id).await?;
        Ok(Self::governance_queue_items_from_state(&payload.state))
    }

    pub async fn backfill_avatar_governance_storage(
        &self,
        team_id: Option<&str>,
    ) -> Result<AvatarGovernanceBackfillReport, ServiceError> {
        let team_oid = match team_id {
            Some(value) if !value.trim().is_empty() => {
                Some(ObjectId::parse_str(value).map_err(|e| {
                    ServiceError::Internal(format!("Invalid team id for governance backfill: {e}"))
                })?)
            }
            _ => None,
        };

        let filter = Self::avatar_portal_filter(team_oid);
        let portal_docs: Vec<Document> = self
            .portals()
            .find(
                filter,
                mongodb::options::FindOptions::builder()
                    .sort(doc! { "updated_at": -1 })
                    .build(),
            )
            .await?
            .try_collect()
            .await?;

        let mut report = AvatarGovernanceBackfillReport::default();
        let mut touched_team_ids = HashSet::new();

        for portal_doc in portal_docs {
            let Some(portal_oid) = portal_doc.get_object_id("_id").ok() else {
                continue;
            };
            let Some(portal_team_oid) = portal_doc.get_object_id("team_id").ok() else {
                continue;
            };

            let portal_id = portal_oid.to_hex();
            let portal_team_id = portal_team_oid.to_hex();
            let payload = Self::governance_payload_from_portal_doc(
                &portal_team_id,
                &portal_id,
                &portal_doc,
                Utc::now(),
            );

            report.portals_scanned += 1;
            touched_team_ids.insert(portal_team_id.clone());

            let existing = self
                .avatar_governance_states()
                .find_one(
                    doc! { "team_id": &portal_team_id, "portal_id": &portal_id },
                    None,
                )
                .await?;

            if existing.is_none() {
                self.refresh_avatar_governance_state_read_model(&payload)
                    .await?;
                report.states_created += 1;
            }

            report.events_seeded += self
                .refresh_avatar_governance_event_read_model(
                    &portal_team_id,
                    &portal_id,
                    &payload.state,
                    &payload.config,
                )
                .await?;
        }

        for team_id in touched_team_ids {
            let _ = self.sync_avatar_instance_projections(&team_id).await?;
            report.projections_synced_teams += 1;
        }

        Ok(report)
    }

    pub async fn audit_avatar_deep_water(
        &self,
        team_id: Option<&str>,
    ) -> Result<AvatarDeepWaterAuditReport, ServiceError> {
        let requested_team_id = Self::normalize_optional_scope_id(team_id);
        let team_oid = match requested_team_id.as_deref() {
            Some(value) => Some(ObjectId::parse_str(value).map_err(|e| {
                ServiceError::Internal(format!("Invalid team id for avatar deep-water audit: {e}"))
            })?),
            _ => None,
        };

        let filter = Self::avatar_portal_filter(team_oid);
        let portal_docs: Vec<Document> = self
            .portals()
            .find(
                filter,
                mongodb::options::FindOptions::builder()
                    .sort(doc! { "updated_at": -1 })
                    .build(),
            )
            .await?
            .try_collect()
            .await?;

        let mut team_ids = Vec::new();
        for portal_doc in &portal_docs {
            if let Ok(team_oid) = portal_doc.get_object_id("team_id") {
                let team_id = team_oid.to_hex();
                if !team_ids.iter().any(|value| value == &team_id) {
                    team_ids.push(team_id);
                }
            }
        }

        let governance_docs: Vec<AvatarGovernanceStateDoc> =
            if let Some(team_id) = requested_team_id.as_deref() {
                self.avatar_governance_states()
                    .find(doc! { "team_id": team_id }, None)
                    .await?
                    .try_collect()
                    .await?
            } else {
                self.avatar_governance_states()
                    .find(doc! {}, None)
                    .await?
                    .try_collect()
                    .await?
            };
        let total_state_docs = governance_docs.len() as u64;

        let agent_docs: Vec<TeamAgentDoc> = if team_ids.is_empty() {
            Vec::new()
        } else if team_ids.len() == 1 {
            self.agents()
                .find(doc! { "team_id": &team_ids[0] }, None)
                .await?
                .try_collect()
                .await?
        } else {
            self.agents()
                .find(doc! { "team_id": { "$in": &team_ids } }, None)
                .await?
                .try_collect()
                .await?
        };

        let (mut governance_map, governance_state_doc_counts, duplicate_rows, duplicate_state_docs) =
            Self::index_governance_state_docs(governance_docs);

        let agent_map: HashMap<(String, String), TeamAgentDoc> = agent_docs
            .into_iter()
            .map(|doc| ((doc.team_id.clone(), doc.agent_id.clone()), doc))
            .collect();

        let mut governance = AvatarGovernanceDivergenceAuditReport {
            total_avatar_portals: portal_docs.len() as u64,
            total_state_docs,
            duplicate_state_docs,
            duplicate_rows,
            ..AvatarGovernanceDivergenceAuditReport::default()
        };
        let mut bindings = AvatarBindingAuditReport {
            total_avatar_portals: portal_docs.len() as u64,
            ..AvatarBindingAuditReport::default()
        };

        for portal_doc in portal_docs {
            let Some(portal_oid) = portal_doc.get_object_id("_id").ok() else {
                continue;
            };
            let Some(portal_team_oid) = portal_doc.get_object_id("team_id").ok() else {
                continue;
            };

            let portal_id = portal_oid.to_hex();
            let portal_team_id = portal_team_oid.to_hex();
            let portal_name =
                Self::doc_string(&portal_doc, "name").unwrap_or_else(|| "未命名分身".to_string());
            let slug = Self::doc_string(&portal_doc, "slug").unwrap_or_default();

            let explicit_portal_state = Self::raw_portal_governance_state_json(&portal_doc);
            let explicit_portal_config = Self::raw_portal_governance_config_json(&portal_doc);
            let state_doc = governance_map.remove(&(portal_team_id.clone(), portal_id.clone()));

            let state_doc_exists = state_doc.is_some();
            let portal_state_exists = explicit_portal_state.is_some();
            let portal_config_exists = explicit_portal_config.is_some();
            let has_portal_governance = portal_state_exists || portal_config_exists;
            let state_matches_portal_state = state_doc
                .as_ref()
                .and_then(|doc| {
                    explicit_portal_state
                        .as_ref()
                        .map(|value| doc.state == *value)
                })
                .unwrap_or(false);
            let config_matches_portal_config = state_doc
                .as_ref()
                .and_then(|doc| {
                    explicit_portal_config
                        .as_ref()
                        .map(|value| doc.config == *value)
                })
                .unwrap_or(false);

            let governance_classification = Self::classify_governance_divergence(
                state_doc_exists,
                has_portal_governance,
                state_matches_portal_state,
                config_matches_portal_config,
            );

            match governance_classification {
                AvatarGovernanceDivergenceKind::DefaultOnly => governance.default_only += 1,
                AvatarGovernanceDivergenceKind::InSync => governance.in_sync += 1,
                AvatarGovernanceDivergenceKind::StateOnly => governance.state_only += 1,
                AvatarGovernanceDivergenceKind::SettingsOnly => governance.settings_only += 1,
                AvatarGovernanceDivergenceKind::Differing => governance.differing += 1,
            }

            governance.rows.push(AvatarGovernanceDivergenceRow {
                team_id: portal_team_id.clone(),
                portal_id: portal_id.clone(),
                portal_name: portal_name.clone(),
                slug: slug.clone(),
                classification: governance_classification,
                state_doc_exists,
                portal_state_exists,
                portal_config_exists,
                state_matches_portal_state,
                config_matches_portal_config,
            });

            let explicit_coding_agent_id = Self::doc_string(&portal_doc, "coding_agent_id")
                .filter(|value| !value.trim().is_empty());
            let explicit_service_agent_id = Self::doc_string(&portal_doc, "service_agent_id")
                .filter(|value| !value.trim().is_empty());
            let effective_manager_agent_id = Self::extract_portal_manager_id(&portal_doc);
            let effective_service_agent_id = Self::extract_portal_service_agent_id(&portal_doc);

            let manager_agent = effective_manager_agent_id
                .as_ref()
                .and_then(|agent_id| agent_map.get(&(portal_team_id.clone(), agent_id.clone())));
            let service_agent = effective_service_agent_id
                .as_ref()
                .and_then(|agent_id| agent_map.get(&(portal_team_id.clone(), agent_id.clone())));

            let mut issues = Vec::new();

            if explicit_coding_agent_id.is_none() {
                issues.push(AvatarBindingIssueKind::MissingExplicitManagerBinding);
                bindings.missing_explicit_manager_binding += 1;
            }
            if explicit_service_agent_id.is_none() {
                issues.push(AvatarBindingIssueKind::MissingExplicitServiceBinding);
                bindings.missing_explicit_service_binding += 1;
            }
            if effective_manager_agent_id.is_none() {
                issues.push(AvatarBindingIssueKind::MissingManagerBinding);
                bindings.missing_manager_binding += 1;
            }
            if effective_service_agent_id.is_none() {
                issues.push(AvatarBindingIssueKind::MissingServiceBinding);
                bindings.missing_service_binding += 1;
            }
            if effective_manager_agent_id.is_some() && manager_agent.is_none() {
                issues.push(AvatarBindingIssueKind::ManagerAgentNotFound);
                bindings.manager_agent_not_found += 1;
            }
            if effective_service_agent_id.is_some() && service_agent.is_none() {
                issues.push(AvatarBindingIssueKind::ServiceAgentNotFound);
                bindings.service_agent_not_found += 1;
            }
            if let Some(manager_agent) = manager_agent {
                if manager_agent.agent_domain.as_deref() != Some("digital_avatar")
                    || manager_agent.agent_role.as_deref() != Some("manager")
                {
                    issues.push(AvatarBindingIssueKind::ManagerRoleMismatch);
                    bindings.manager_role_mismatch += 1;
                }
            }
            if let Some(service_agent) = service_agent {
                if service_agent.agent_domain.as_deref() != Some("digital_avatar")
                    || service_agent.agent_role.as_deref() != Some("service")
                {
                    issues.push(AvatarBindingIssueKind::ServiceRoleMismatch);
                    bindings.service_role_mismatch += 1;
                }
                if service_agent.owner_manager_agent_id.as_deref()
                    != effective_manager_agent_id.as_deref()
                {
                    issues.push(AvatarBindingIssueKind::OwnerManagerMismatch);
                    bindings.owner_manager_mismatch += 1;
                }
            }
            if effective_manager_agent_id.is_some()
                && effective_manager_agent_id == effective_service_agent_id
            {
                issues.push(AvatarBindingIssueKind::SameAgentReused);
                bindings.same_agent_reused += 1;
            }

            let issue_messages = Self::binding_issue_messages(
                &issues,
                explicit_coding_agent_id.as_deref(),
                explicit_service_agent_id.as_deref(),
                effective_manager_agent_id.as_deref(),
                effective_service_agent_id.as_deref(),
                manager_agent,
                service_agent,
            );
            let has_shadow_invariant_issue = issues.iter().any(|issue| {
                matches!(
                    issue,
                    AvatarBindingIssueKind::ManagerRoleMismatch
                        | AvatarBindingIssueKind::ServiceRoleMismatch
                        | AvatarBindingIssueKind::OwnerManagerMismatch
                )
            });
            if has_shadow_invariant_issue {
                bindings.shadow_invariant_rows += 1;
            }

            if issues.is_empty() {
                bindings.valid += 1;
            }

            bindings.rows.push(AvatarBindingAuditRow {
                team_id: portal_team_id,
                portal_id,
                portal_name,
                slug,
                explicit_coding_agent_id,
                explicit_service_agent_id,
                effective_manager_agent_id,
                effective_service_agent_id,
                manager_agent_domain: manager_agent.and_then(|agent| agent.agent_domain.clone()),
                manager_agent_role: manager_agent.and_then(|agent| agent.agent_role.clone()),
                service_agent_domain: service_agent.and_then(|agent| agent.agent_domain.clone()),
                service_agent_role: service_agent.and_then(|agent| agent.agent_role.clone()),
                service_owner_manager_agent_id: service_agent
                    .and_then(|agent| agent.owner_manager_agent_id.clone()),
                issues,
                issue_messages,
            });
        }

        let mut orphan_rows: Vec<_> = governance_map
            .into_values()
            .map(|doc| {
                let state_key = (doc.team_id.clone(), doc.portal_id.clone());
                AvatarGovernanceOrphanStateRow {
                    team_id: doc.team_id,
                    portal_id: doc.portal_id,
                    state_doc_count: governance_state_doc_counts
                        .get(&state_key)
                        .copied()
                        .unwrap_or(1),
                    updated_at: doc.updated_at.to_rfc3339(),
                }
            })
            .collect();
        orphan_rows.sort_by(|left, right| {
            left.team_id
                .cmp(&right.team_id)
                .then_with(|| left.portal_id.cmp(&right.portal_id))
        });
        governance.orphan_state_docs = orphan_rows.iter().map(|row| row.state_doc_count).sum();
        governance.orphan_rows = orphan_rows;

        Ok(AvatarDeepWaterAuditReport {
            generated_at: Utc::now().to_rfc3339(),
            requested_team_id,
            governance,
            bindings,
            read_side_effects: Self::avatar_read_side_effect_audit_items(),
        })
    }

    pub async fn list_avatar_instance_projections(
        &self,
        team_id: &str,
    ) -> Result<Vec<AvatarInstanceSummary>, mongodb::error::Error> {
        Ok(self
            .derive_avatar_instance_projection_docs(team_id)
            .await?
            .into_iter()
            .map(Into::into)
            .collect())
    }

    fn build_avatar_manager_reports(
        latest_completed_mission: Option<&MissionListItem>,
        latest_completed_detail: Option<&MissionDoc>,
        latest_attention_mission: Option<&MissionListItem>,
        governance_events: &[AvatarGovernanceEventPayload],
        service_agent_name: &str,
        portal_id: &str,
        work_object_labels: &[String],
        delivery_outputs: &[String],
    ) -> Vec<AvatarWorkbenchReportItemPayload> {
        let mut reports = Vec::new();

        if let Some(mission) = latest_completed_mission {
            reports.push(AvatarWorkbenchReportItemPayload {
                id: format!("mission-delivery:{}", mission.mission_id),
                ts: parse_rfc3339_utc(&mission.updated_at).unwrap_or_else(Utc::now),
                kind: "delivery".to_string(),
                title: mission.goal.clone(),
                summary: latest_completed_detail
                    .and_then(|detail| detail.final_summary.clone())
                    .unwrap_or_else(|| {
                        "这项长程委托已经完成，交付摘要会优先展示在这里。".to_string()
                    }),
                status: mission_status_label(&mission.status).to_string(),
                source: service_agent_name.to_string(),
                recommendation: Some(
                    "先审阅交付结果，再决定是否继续优化、采用或交付。".to_string(),
                ),
                action_kind: Some("open_mission".to_string()),
                action_target_id: Some(mission.mission_id.clone()),
                work_objects: work_object_labels.iter().take(3).cloned().collect(),
                outputs: delivery_outputs.iter().take(3).cloned().collect(),
                needs_decision: false,
            });
        }

        if let Some(mission) = latest_attention_mission {
            let needs_decision = mission_needs_attention(&mission.status);
            reports.push(AvatarWorkbenchReportItemPayload {
                id: format!("mission-progress:{}", mission.mission_id),
                ts: parse_rfc3339_utc(&mission.updated_at).unwrap_or_else(Utc::now),
                kind: "progress".to_string(),
                title: mission.goal.clone(),
                summary: if mission.status == MissionStatus::Running {
                    "当前有复杂工作仍在处理中，可以继续跟进进度或等待下一轮汇报。".to_string()
                } else {
                    "当前有复杂工作需要恢复或继续决策，建议先查看原因再继续推进。".to_string()
                },
                status: mission_status_label(&mission.status).to_string(),
                source: service_agent_name.to_string(),
                recommendation: Some(if mission_needs_attention(&mission.status) {
                    "优先查看本次长程委托，再决定继续处理还是回到管理对话调整方案。".to_string()
                } else {
                    "先查看进度；如果对象、边界或交付目标需要变化，再回到管理对话调整。".to_string()
                }),
                action_kind: Some(if mission_needs_attention(&mission.status) {
                    "resume_mission".to_string()
                } else {
                    "open_mission".to_string()
                }),
                action_target_id: Some(mission.mission_id.clone()),
                work_objects: work_object_labels.iter().take(3).cloned().collect(),
                outputs: Vec::new(),
                needs_decision,
            });
        }

        for event in governance_events.iter().take(6) {
            let needs_decision = Self::governance_event_needs_decision(event);
            reports.push(AvatarWorkbenchReportItemPayload {
                id: format!("event:{}", event.event_id),
                ts: event.created_at,
                kind: if event.entity_type == "runtime" {
                    "runtime".to_string()
                } else {
                    "governance".to_string()
                },
                title: event.title.clone(),
                summary: event
                    .detail
                    .clone()
                    .unwrap_or_else(|| "新的治理或运行记录已经写入工作台。".to_string()),
                status: event.status.clone().unwrap_or_else(|| "logged".to_string()),
                source: event
                    .actor_name
                    .clone()
                    .or_else(|| event.actor_id.clone())
                    .unwrap_or_else(|| "系统汇总".to_string()),
                recommendation: Some(if event.entity_type == "runtime" {
                    "如果涉及失败恢复或对象补充，建议先打开日志与治理台查看细节。".to_string()
                } else {
                    "如果需要进一步确认权限、提案或策略，建议先打开治理台处理。".to_string()
                }),
                action_kind: Some(if event.entity_type == "runtime" {
                    "open_logs".to_string()
                } else {
                    "open_governance".to_string()
                }),
                action_target_id: Some(portal_id.to_string()),
                work_objects: work_object_labels.iter().take(2).cloned().collect(),
                outputs: Vec::new(),
                needs_decision,
            });
        }

        reports.sort_by(|left, right| right.ts.cmp(&left.ts));
        reports.dedup_by(|left, right| left.id == right.id);
        reports
    }

    fn merge_avatar_workbench_reports(
        derived_reports: &[AvatarWorkbenchReportItemPayload],
        persisted_reports: &[AvatarWorkbenchReportItemPayload],
        limit: usize,
    ) -> Vec<AvatarWorkbenchReportItemPayload> {
        let mut reports = persisted_reports
            .iter()
            .cloned()
            .chain(derived_reports.iter().cloned())
            .collect::<Vec<_>>();
        reports.sort_by(|left, right| right.ts.cmp(&left.ts));
        let mut seen = HashSet::new();
        let mut deduped = Vec::with_capacity(reports.len());
        for report in reports {
            if seen.insert(report.id.clone()) {
                deduped.push(report);
            }
        }
        deduped.truncate(limit);
        deduped
    }

    async fn list_persisted_avatar_manager_reports(
        &self,
        team_id: &str,
        portal_id: &str,
        limit: u32,
    ) -> Result<Vec<AvatarWorkbenchReportItemPayload>, mongodb::error::Error> {
        let docs = self
            .avatar_manager_reports()
            .find(
                doc! {
                    "team_id": team_id,
                    "portal_id": portal_id,
                    "report_source": {
                        "$exists": true,
                        "$ne": default_avatar_manager_report_source(),
                    }
                },
                mongodb::options::FindOptions::builder()
                    .sort(doc! { "created_at": -1 })
                    .limit(i64::from(limit.clamp(1, 20)))
                    .build(),
            )
            .await?
            .try_collect::<Vec<AvatarManagerReportDoc>>()
            .await?;
        Ok(docs.into_iter().map(Into::into).collect())
    }

    pub async fn upsert_avatar_manager_report(
        &self,
        team_id: &str,
        portal_id: &str,
        report: AvatarWorkbenchReportItemPayload,
        report_source: &str,
    ) -> Result<(), mongodb::error::Error> {
        let now = Utc::now();
        self.avatar_manager_reports()
            .update_one(
                doc! {
                    "team_id": team_id,
                    "portal_id": portal_id,
                    "report_id": &report.id,
                    "report_source": report_source,
                },
                doc! {
                    "$set": {
                        "team_id": team_id,
                        "portal_id": portal_id,
                        "report_id": &report.id,
                        "report_source": report_source,
                        "kind": &report.kind,
                        "title": &report.title,
                        "summary": &report.summary,
                        "status": &report.status,
                        "source": &report.source,
                        "recommendation": bson::to_bson(&report.recommendation).unwrap_or(Bson::Null),
                        "action_kind": bson::to_bson(&report.action_kind).unwrap_or(Bson::Null),
                        "action_target_id": bson::to_bson(&report.action_target_id).unwrap_or(Bson::Null),
                        "work_objects": bson::to_bson(&report.work_objects).unwrap_or_else(|_| Bson::Array(Vec::new())),
                        "outputs": bson::to_bson(&report.outputs).unwrap_or_else(|_| Bson::Array(Vec::new())),
                        "needs_decision": report.needs_decision,
                        "created_at": bson::DateTime::from_chrono(report.ts),
                        "synced_at": bson::DateTime::from_chrono(now),
                    },
                    "$setOnInsert": {
                        "_id": ObjectId::new(),
                    }
                },
                mongodb::options::UpdateOptions::builder()
                    .upsert(true)
                    .build(),
            )
            .await?;
        Ok(())
    }

    pub async fn list_avatar_manager_reports(
        &self,
        team_id: &str,
        portal_id: &str,
        limit: u32,
    ) -> Result<Vec<AvatarWorkbenchReportItemPayload>, mongodb::error::Error> {
        let docs = self
            .avatar_manager_reports()
            .find(
                doc! { "team_id": team_id, "portal_id": portal_id },
                mongodb::options::FindOptions::builder()
                    .sort(doc! { "created_at": -1 })
                    .limit(i64::from(limit.clamp(1, 20)))
                    .build(),
            )
            .await?
            .try_collect::<Vec<AvatarManagerReportDoc>>()
            .await?;
        Ok(docs.into_iter().map(Into::into).collect())
    }

    fn governance_event_needs_decision(event: &AvatarGovernanceEventPayload) -> bool {
        event
            .status
            .as_deref()
            .map(|status| {
                matches!(
                    status,
                    "pending"
                        | "review"
                        | "needs_review"
                        | "needs_human"
                        | "requires_approval"
                        | "attention"
                        | "failed"
                )
            })
            .unwrap_or(false)
            || event
                .meta
                .get("needs_decision")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false)
            || matches!(
                event
                    .meta
                    .get("risk")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default(),
                "high" | "critical"
            )
    }

    async fn resolve_work_object_labels(
        &self,
        team_oid: Option<ObjectId>,
        portal_doc: Option<&Document>,
    ) -> Result<Vec<String>, mongodb::error::Error> {
        let Some(bound_ids) = portal_doc.and_then(|doc| doc.get_array("bound_document_ids").ok())
        else {
            return Ok(Vec::new());
        };
        let object_ids = bound_ids
            .iter()
            .filter_map(|value| value.as_str())
            .filter_map(|raw| ObjectId::parse_str(raw).ok())
            .collect::<Vec<_>>();
        if object_ids.is_empty() {
            return Ok(Vec::new());
        }
        let mut filter = doc! { "_id": { "$in": object_ids }, "is_deleted": false };
        if let Some(team_oid) = team_oid {
            filter.insert("team_id", team_oid);
        }
        let docs = self
            .documents_store()
            .find(
                filter,
                mongodb::options::FindOptions::builder()
                    .sort(doc! { "updated_at": -1 })
                    .limit(6)
                    .build(),
            )
            .await?
            .try_collect::<Vec<TeamDocument>>()
            .await?;
        Ok(docs
            .into_iter()
            .map(|doc| doc.display_name.unwrap_or(doc.name))
            .collect())
    }

    async fn resolve_delivery_outputs(
        &self,
        mission_id: Option<&str>,
    ) -> Result<Vec<String>, mongodb::error::Error> {
        let Some(mission_id) = mission_id else {
            return Ok(Vec::new());
        };
        let artifacts = self.list_mission_artifacts(mission_id).await?;
        Ok(artifacts
            .into_iter()
            .rev()
            .filter_map(|artifact| {
                artifact
                    .file_path
                    .filter(|path| !path.trim().is_empty())
                    .or_else(|| (!artifact.name.trim().is_empty()).then_some(artifact.name))
            })
            .take(4)
            .collect())
    }

    pub async fn get_avatar_workbench_snapshot(
        &self,
        team_id: &str,
        portal_id: &str,
    ) -> Result<AvatarWorkbenchSnapshotPayload, mongodb::error::Error> {
        let projections = self.list_avatar_instance_projections(team_id).await?;
        let projection = projections
            .iter()
            .find(|item| item.portal_id == portal_id)
            .cloned();

        let team_oid = ObjectId::parse_str(team_id).ok();
        let portal_oid = ObjectId::parse_str(portal_id).ok();
        let portal_doc = match (team_oid, portal_oid) {
            (Some(team_oid), Some(portal_oid)) => {
                self.portals()
                    .find_one(doc! { "_id": portal_oid, "team_id": team_oid }, None)
                    .await?
            }
            (_, Some(portal_oid)) => {
                self.portals()
                    .find_one(doc! { "_id": portal_oid }, None)
                    .await?
            }
            _ => None,
        };

        let avatar_name = projection
            .as_ref()
            .map(|item| item.name.clone())
            .or_else(|| {
                portal_doc
                    .as_ref()
                    .and_then(|doc| doc.get_str("name").ok().map(ToOwned::to_owned))
            })
            .unwrap_or_else(|| "未命名岗位".to_string());
        let avatar_type = projection
            .as_ref()
            .map(|item| item.avatar_type.clone())
            .or_else(|| {
                portal_doc.as_ref().and_then(|doc| {
                    doc.get_document("settings")
                        .ok()
                        .and_then(|settings| settings.get_document("digitalAvatarProfile").ok())
                        .and_then(|profile| {
                            profile.get_str("avatar_type").ok().map(ToOwned::to_owned)
                        })
                })
            })
            .unwrap_or_else(|| "unknown".to_string());
        let avatar_status = projection
            .as_ref()
            .map(|item| item.status.clone())
            .or_else(|| {
                portal_doc
                    .as_ref()
                    .and_then(|doc| doc.get_str("status").ok().map(ToOwned::to_owned))
            })
            .unwrap_or_else(|| "draft".to_string());
        let manager_agent_id = projection
            .as_ref()
            .and_then(|item| item.manager_agent_id.clone());
        let service_agent_id = projection
            .as_ref()
            .and_then(|item| item.service_agent_id.clone());
        let document_access_mode = projection
            .as_ref()
            .map(|item| item.document_access_mode.clone())
            .or_else(|| {
                portal_doc.as_ref().and_then(|doc| {
                    doc.get_str("document_access_mode")
                        .ok()
                        .map(ToOwned::to_owned)
                })
            })
            .unwrap_or_else(|| "read_only".to_string());
        let work_object_count = portal_doc
            .as_ref()
            .and_then(|doc| doc.get_array("bound_document_ids").ok())
            .map_or(0, |items| items.len() as u32);

        let governance_events = self
            .list_avatar_governance_events(team_id, portal_id, 40)
            .await
            .unwrap_or_default();
        let queue_items = self
            .list_avatar_governance_queue(team_id, portal_id)
            .await
            .unwrap_or_default();

        let missions = if let Some(manager_id) = manager_agent_id.as_deref() {
            self.list_missions(ListMissionsQuery {
                team_id: team_id.to_string(),
                agent_id: Some(manager_id.to_string()),
                status: None,
                page: 1,
                limit: 20,
            })
            .await
            .unwrap_or_default()
        } else {
            Vec::new()
        };

        let latest_completed_mission = missions
            .iter()
            .find(|mission| mission.status == MissionStatus::Completed);
        let latest_attention_mission = missions
            .iter()
            .find(|mission| {
                mission.status == MissionStatus::Failed || mission.status == MissionStatus::Paused
            })
            .or_else(|| {
                missions
                    .iter()
                    .find(|mission| mission.status == MissionStatus::Running)
            });

        let latest_completed_detail = if let Some(mission) = latest_completed_mission {
            self.get_mission(&mission.mission_id).await?
        } else {
            None
        };
        let work_object_labels = self
            .resolve_work_object_labels(team_oid, portal_doc.as_ref())
            .await
            .unwrap_or_default();
        let delivery_outputs = self
            .resolve_delivery_outputs(
                latest_completed_mission.map(|mission| mission.mission_id.as_str()),
            )
            .await
            .unwrap_or_default();

        let running_mission_count = missions
            .iter()
            .filter(|mission| mission.status == MissionStatus::Running)
            .count() as u32;
        let needs_attention_count = missions
            .iter()
            .filter(|mission| {
                mission.status == MissionStatus::Failed
                    || mission.status == MissionStatus::Paused
                    || mission.status == MissionStatus::Running
            })
            .count() as u32;

        let agent_ids = [manager_agent_id.as_deref(), service_agent_id.as_deref()]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>();
        let agent_names = self.batch_get_agent_names(&agent_ids).await;
        let service_agent_name = service_agent_id
            .as_deref()
            .and_then(|agent_id| agent_names.get(agent_id).cloned())
            .unwrap_or_else(|| avatar_name.clone());

        let derived_reports = Self::build_avatar_manager_reports(
            latest_completed_mission,
            latest_completed_detail.as_ref(),
            latest_attention_mission,
            &governance_events,
            &service_agent_name,
            portal_id,
            &work_object_labels,
            &delivery_outputs,
        );
        let persisted_reports = self
            .list_persisted_avatar_manager_reports(team_id, portal_id, 6)
            .await
            .unwrap_or_default();
        let reports = Self::merge_avatar_workbench_reports(&derived_reports, &persisted_reports, 4);

        let mut decisions = Vec::new();
        if let Some(mission) =
            latest_attention_mission.filter(|mission| mission_needs_attention(&mission.status))
        {
            decisions.push(AvatarWorkbenchDecisionItemPayload {
                id: format!("decision-mission:{}", mission.mission_id),
                ts: parse_rfc3339_utc(&mission.updated_at).unwrap_or_else(Utc::now),
                kind: "mission".to_string(),
                title: mission.goal.clone(),
                detail: "这项长程委托当前需要管理 Agent 继续决策或恢复执行。".to_string(),
                status: mission_status_label(&mission.status).to_string(),
                risk: "medium".to_string(),
                source: service_agent_name.clone(),
                recommendation: Some(
                    "先查看这次长程委托的进度和原因，再决定继续处理、调整边界，还是回到管理对话重新规划。"
                        .to_string(),
                ),
                work_objects: work_object_labels.iter().take(3).cloned().collect(),
                action_kind: Some("resume_mission".to_string()),
                action_target_id: Some(mission.mission_id.clone()),
            });
        }

        for item in queue_items.iter().take(4) {
            let risk = queue_item_risk(&item.meta).to_string();
            decisions.push(AvatarWorkbenchDecisionItemPayload {
                id: format!("decision-queue:{}", item.id),
                ts: parse_rfc3339_utc(&item.ts).unwrap_or_else(Utc::now),
                kind: item.kind.clone(),
                title: item.title.clone(),
                detail: item.detail.clone(),
                status: item.status.clone(),
                risk: risk.clone(),
                source: queue_item_source(&item.kind, &item.meta),
                recommendation: Some(queue_item_recommendation(&item.kind, &risk)),
                work_objects: work_object_labels.iter().take(2).cloned().collect(),
                action_kind: Some("open_governance".to_string()),
                action_target_id: Some(portal_id.to_string()),
            });
        }
        decisions.sort_by(|left, right| right.ts.cmp(&left.ts));
        decisions.dedup_by(|left, right| left.id == right.id);
        decisions.truncate(4);

        let latest_activity_at = reports
            .iter()
            .map(|item| item.ts)
            .chain(decisions.iter().map(|item| item.ts))
            .chain(
                missions
                    .iter()
                    .filter_map(|mission| parse_rfc3339_utc(&mission.updated_at)),
            )
            .chain(projection.as_ref().map(|item| item.projected_at))
            .max()
            .unwrap_or_else(Utc::now);

        Ok(AvatarWorkbenchSnapshotPayload {
            portal_id: portal_id.to_string(),
            team_id: team_id.to_string(),
            summary: AvatarWorkbenchSummaryPayload {
                portal_id: portal_id.to_string(),
                team_id: team_id.to_string(),
                avatar_name,
                avatar_type,
                avatar_status,
                manager_agent_id,
                service_agent_id,
                document_access_mode,
                work_object_count,
                pending_decision_count: decisions.len() as u32,
                running_mission_count,
                needs_attention_count,
                last_activity_at: latest_activity_at,
            },
            reports,
            decisions,
        })
    }

    pub async fn update_agent(
        &self,
        id: &str,
        req: UpdateAgentRequest,
    ) -> Result<Option<TeamAgent>, mongodb::error::Error> {
        let now = Utc::now();
        let mut set_doc = doc! { "updated_at": bson::DateTime::from_chrono(now) };

        if let Some(name) = req.name {
            set_doc.insert("name", name);
        }
        if let Some(desc) = req.description {
            set_doc.insert("description", desc);
        }
        if let Some(avatar) = req.avatar {
            set_doc.insert("avatar", avatar);
        }
        if let Some(system_prompt) = req.system_prompt {
            set_doc.insert("system_prompt", system_prompt);
        }
        if let Some(api_url) = req.api_url {
            set_doc.insert("api_url", api_url);
        }
        if let Some(model) = req.model {
            set_doc.insert("model", model);
        }
        if let Some(ref api_key) = req.api_key {
            tracing::info!(
                "Updating API key for agent {}: key length = {}",
                id,
                api_key.len()
            );
            set_doc.insert("api_key", api_key.clone());
        }
        if let Some(api_format) = req.api_format {
            set_doc.insert("api_format", api_format);
        }
        if let Some(status) = req.status {
            set_doc.insert("status", status.to_string());
        }
        if let Some(ref extensions) = req.enabled_extensions {
            let ext_bson = mongodb::bson::to_bson(extensions).unwrap_or(bson::Bson::Array(vec![]));
            set_doc.insert("enabled_extensions", ext_bson);
        }
        if let Some(ref custom_ext) = req.custom_extensions {
            let ext_bson = custom_extensions_to_bson(custom_ext);
            set_doc.insert("custom_extensions", ext_bson);
        }
        if let Some(ref agent_domain) = req.agent_domain {
            set_doc.insert("agent_domain", agent_domain.clone());
        }
        if let Some(ref agent_role) = req.agent_role {
            set_doc.insert("agent_role", agent_role.clone());
        }
        if let Some(ref owner_manager_agent_id) = req.owner_manager_agent_id {
            set_doc.insert("owner_manager_agent_id", owner_manager_agent_id.clone());
        }
        if let Some(ref template_source_agent_id) = req.template_source_agent_id {
            set_doc.insert("template_source_agent_id", template_source_agent_id.clone());
        }
        if let Some(ref allowed_groups) = req.allowed_groups {
            let bson_val =
                mongodb::bson::to_bson(allowed_groups).unwrap_or(bson::Bson::Array(vec![]));
            set_doc.insert("allowed_groups", bson_val);
        }
        if let Some(max_concurrent) = req.max_concurrent_tasks {
            set_doc.insert("max_concurrent_tasks", max_concurrent as i32);
        }
        if let Some(temperature) = req.temperature {
            set_doc.insert("temperature", temperature as f64);
        }
        if let Some(max_tokens) = req.max_tokens {
            set_doc.insert("max_tokens", max_tokens);
        }
        if let Some(context_limit) = req.context_limit {
            set_doc.insert("context_limit", context_limit as i64);
        }
        if let Some(thinking_enabled) = req.thinking_enabled {
            set_doc.insert("thinking_enabled", thinking_enabled);
        }
        if let Some(ref assigned_skills) = req.assigned_skills {
            let skills_bson =
                mongodb::bson::to_bson(assigned_skills).unwrap_or(bson::Bson::Array(vec![]));
            set_doc.insert("assigned_skills", skills_bson);
        }
        if let Some(auto_approve) = req.auto_approve_chat {
            set_doc.insert("auto_approve_chat", auto_approve);
        }

        self.agents()
            .update_one(doc! { "agent_id": id }, doc! { "$set": set_doc }, None)
            .await?;

        self.get_agent(id).await
    }

    pub async fn delete_agent(&self, id: &str) -> Result<bool, mongodb::error::Error> {
        let result = self
            .agents()
            .delete_one(doc! { "agent_id": id }, None)
            .await?;
        Ok(result.deleted_count > 0)
    }

    // Task operations
    pub async fn submit_task(
        &self,
        submitter_id: &str,
        req: SubmitTaskRequest,
    ) -> Result<AgentTask, ServiceError> {
        validate_priority(req.priority)?;

        let id = Uuid::new_v4().to_string();
        let now = Utc::now();

        let doc = AgentTaskDoc {
            id: None,
            task_id: id.clone(),
            team_id: req.team_id,
            agent_id: req.agent_id,
            submitter_id: submitter_id.to_string(),
            approver_id: None,
            task_type: req.task_type.to_string(),
            content: req.content,
            status: "pending".to_string(),
            priority: req.priority,
            submitted_at: now,
            approved_at: None,
            started_at: None,
            completed_at: None,
            error_message: None,
        };

        self.tasks().insert_one(&doc, None).await?;
        self.get_task(&id)
            .await?
            .ok_or_else(|| ServiceError::Internal(format!("Task not found after insert: {}", id)))
    }

    pub async fn get_task(&self, id: &str) -> Result<Option<AgentTask>, mongodb::error::Error> {
        let doc = self.tasks().find_one(doc! { "task_id": id }, None).await?;
        Ok(doc.map(|d| d.into()))
    }

    pub async fn list_tasks(
        &self,
        query: ListTasksQuery,
    ) -> Result<PaginatedResponse<AgentTask>, mongodb::error::Error> {
        let clamped_limit = query.limit.min(100);
        let limit = clamped_limit as i64;
        let skip = ((query.page.saturating_sub(1)) * clamped_limit) as u64;

        let mut filter = doc! { "team_id": &query.team_id };
        if let Some(agent_id) = &query.agent_id {
            filter.insert("agent_id", agent_id);
        }
        if let Some(status) = &query.status {
            filter.insert("status", status.to_string());
        }

        let total = self.tasks().count_documents(filter.clone(), None).await?;

        let options = mongodb::options::FindOptions::builder()
            .sort(doc! { "priority": -1, "submitted_at": -1 })
            .skip(skip)
            .limit(limit)
            .build();
        let cursor = self.tasks().find(filter, options).await?;

        let docs: Vec<AgentTaskDoc> = cursor.try_collect().await?;
        let items: Vec<AgentTask> = docs.into_iter().map(|d| d.into()).collect();

        Ok(PaginatedResponse::new(
            items,
            total,
            query.page,
            query.limit,
        ))
    }

    pub async fn approve_task(
        &self,
        task_id: &str,
        approver_id: &str,
    ) -> Result<Option<AgentTask>, mongodb::error::Error> {
        let now = Utc::now();
        let result = self
            .tasks()
            .update_one(
                doc! { "task_id": task_id, "status": "pending" },
                doc! { "$set": {
                    "status": "approved",
                    "approver_id": approver_id,
                    "approved_at": bson::DateTime::from_chrono(now)
                }},
                None,
            )
            .await?;

        if result.modified_count == 0 {
            return Ok(None);
        }
        self.get_task(task_id).await
    }

    pub async fn reject_task(
        &self,
        task_id: &str,
        approver_id: &str,
    ) -> Result<Option<AgentTask>, mongodb::error::Error> {
        let now = Utc::now();
        let result = self
            .tasks()
            .update_one(
                doc! { "task_id": task_id, "status": "pending" },
                doc! { "$set": {
                    "status": "rejected",
                    "approver_id": approver_id,
                    "approved_at": bson::DateTime::from_chrono(now)
                }},
                None,
            )
            .await?;

        if result.modified_count == 0 {
            return Ok(None);
        }
        self.get_task(task_id).await
    }

    pub async fn cancel_task(
        &self,
        task_id: &str,
    ) -> Result<Option<AgentTask>, mongodb::error::Error> {
        let now = Utc::now();
        let result = self
            .tasks()
            .update_one(
                doc! {
                    "task_id": task_id,
                    "status": { "$in": ["pending", "approved", "running"] }
                },
                doc! { "$set": {
                    "status": "cancelled",
                    "completed_at": bson::DateTime::from_chrono(now)
                }},
                None,
            )
            .await?;

        if result.modified_count == 0 {
            return Ok(None);
        }
        self.get_task(task_id).await
    }

    /// Mark a task as failed with an error message.
    /// Only updates if current status is running or approved (won't overwrite cancelled/completed).
    pub async fn fail_task(
        &self,
        task_id: &str,
        error: &str,
    ) -> Result<Option<AgentTask>, mongodb::error::Error> {
        let now = Utc::now();
        let result = self
            .tasks()
            .update_one(
                doc! {
                    "task_id": task_id,
                    "status": { "$in": ["running", "approved"] }
                },
                doc! { "$set": {
                    "status": "failed",
                    "error_message": error,
                    "completed_at": bson::DateTime::from_chrono(now)
                }},
                None,
            )
            .await?;

        if result.modified_count == 0 {
            return Ok(None);
        }
        self.get_task(task_id).await
    }

    pub async fn get_task_results(
        &self,
        task_id: &str,
    ) -> Result<Vec<TaskResult>, mongodb::error::Error> {
        let options = mongodb::options::FindOptions::builder()
            .sort(doc! { "created_at": 1 })
            .build();
        let cursor = self
            .results()
            .find(doc! { "task_id": task_id }, options)
            .await?;

        let docs: Vec<TaskResultDoc> = cursor.try_collect().await?;
        Ok(docs.into_iter().map(|d| d.into()).collect())
    }

    /// Check if a user has access to a specific agent based on group-based access control
    pub async fn check_agent_access(
        &self,
        agent_id: &str,
        _user_id: &str,
        user_group_ids: &[String],
    ) -> Result<bool, mongodb::error::Error> {
        let agent = match self.get_agent(agent_id).await? {
            Some(a) => a,
            None => return Ok(false),
        };

        // Empty allowed_groups = all team members can use
        if agent.allowed_groups.is_empty() {
            return Ok(true);
        }
        // User must be in at least one allowed group
        Ok(user_group_ids
            .iter()
            .any(|gid| agent.allowed_groups.contains(gid)))
    }

    /// Update agent access control settings
    pub async fn update_access_control(
        &self,
        agent_id: &str,
        allowed_groups: Vec<String>,
    ) -> Result<Option<TeamAgent>, mongodb::error::Error> {
        let now = Utc::now();
        let bson_val = mongodb::bson::to_bson(&allowed_groups).unwrap_or(bson::Bson::Array(vec![]));
        let set = doc! {
            "updated_at": bson::DateTime::from_chrono(now),
            "allowed_groups": bson_val,
        };

        self.agents()
            .update_one(doc! { "agent_id": agent_id }, doc! { "$set": set }, None)
            .await?;

        self.get_agent(agent_id).await
    }

    // ========== Team Extension Bridge ==========

    /// Add a team shared extension to an agent's custom_extensions.
    /// Converts the SharedExtension from the extensions collection into a CustomExtensionConfig
    /// and appends it to the agent's custom_extensions array (with deduplication by name).
    pub async fn add_team_extension_to_agent(
        &self,
        agent_id: &str,
        extension_id: &str,
        team_id: &str,
    ) -> Result<Option<TeamAgent>, ServiceError> {
        use agime_team::services::mongo::extension_service_mongo::ExtensionService;

        let ext_service = ExtensionService::new((*self.db).clone());

        // 1. Fetch the shared extension
        let ext = ext_service
            .get(extension_id)
            .await
            .map_err(|e| ServiceError::Internal(e.to_string()))?
            .ok_or_else(|| ServiceError::Internal("Extension not found".to_string()))?;

        // Verify team ownership
        if ext.team_id.to_hex() != team_id {
            return Err(ServiceError::Internal(
                "Extension does not belong to this team".to_string(),
            ));
        }

        // 2. Get current agent
        let agent = self
            .get_agent(agent_id)
            .await?
            .ok_or_else(|| ServiceError::Internal("Agent not found".to_string()))?;

        // 3. Check for duplicate name
        if agent.custom_extensions.iter().any(|e| e.name == ext.name) {
            return Err(ServiceError::Internal(format!(
                "Extension '{}' already exists in this agent",
                ext.name
            )));
        }

        // 4. Convert SharedExtension -> CustomExtensionConfig
        let uri_or_cmd = ext
            .config
            .get_str("uri_or_cmd")
            .or_else(|_| ext.config.get_str("uriOrCmd"))
            .or_else(|_| ext.config.get_str("command"))
            .unwrap_or_default()
            .to_string();

        if uri_or_cmd.trim().is_empty() {
            return Err(ServiceError::Validation(ValidationError::ExtensionConfig));
        }

        let empty_args = Vec::new();
        let args: Vec<String> = ext
            .config
            .get_array("args")
            .unwrap_or(&empty_args)
            .iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect();

        let envs: std::collections::HashMap<String, String> = ext
            .config
            .get_document("envs")
            .map(|doc| {
                doc.iter()
                    .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                    .collect()
            })
            .unwrap_or_default();

        let ext_id_hex = ext.id.map(|id| id.to_hex()).unwrap_or_default();

        let new_ext = CustomExtensionConfig {
            name: ext.name,
            ext_type: ext.extension_type,
            uri_or_cmd,
            args,
            envs,
            enabled: true,
            source: Some("team".to_string()),
            source_extension_id: Some(ext_id_hex),
        };

        // 5. Append to custom_extensions via $push
        let ext_bson = Bson::Document(custom_extension_to_bson_document(&new_ext));

        let now = Utc::now();
        self.agents()
            .update_one(
                doc! { "agent_id": agent_id },
                doc! {
                    "$push": { "custom_extensions": ext_bson },
                    "$set": { "updated_at": bson::DateTime::from_chrono(now) }
                },
                None,
            )
            .await?;

        Ok(self.get_agent(agent_id).await?)
    }

    // ========== Team Skill Bridge ==========

    /// Add a team shared skill to an agent's assigned_skills.
    pub async fn add_team_skill_to_agent(
        &self,
        agent_id: &str,
        skill_id: &str,
        team_id: &str,
    ) -> Result<Option<TeamAgent>, ServiceError> {
        use agime_team::services::mongo::skill_service_mongo::SkillService;

        let skill_service = SkillService::new((*self.db).clone());

        // 1. Fetch the shared skill
        let skill = skill_service
            .get(skill_id)
            .await
            .map_err(|e| ServiceError::Internal(e.to_string()))?
            .ok_or_else(|| ServiceError::Internal("Skill not found".to_string()))?;

        // Verify team ownership
        if skill.team_id.to_hex() != team_id {
            return Err(ServiceError::Internal(
                "Skill does not belong to this team".to_string(),
            ));
        }

        // 2. Get current agent
        let agent = self
            .get_agent(agent_id)
            .await?
            .ok_or_else(|| ServiceError::Internal("Agent not found".to_string()))?;

        // 3. Check for duplicate skill_id
        let skill_id_hex = skill.id.map(|id| id.to_hex()).unwrap_or_default();
        if agent
            .assigned_skills
            .iter()
            .any(|s| s.skill_id == skill_id_hex)
        {
            return Err(ServiceError::Internal(format!(
                "Skill '{}' already assigned to this agent",
                skill.name
            )));
        }

        // 4. Build AgentSkillConfig
        let new_skill = AgentSkillConfig {
            skill_id: skill_id_hex,
            name: skill.name,
            description: skill.description.clone(),
            enabled: true,
            version: skill.version.clone(),
        };

        // 5. $push to assigned_skills
        let skill_bson = mongodb::bson::to_bson(&new_skill)
            .map_err(|e| ServiceError::Internal(e.to_string()))?;

        let now = Utc::now();
        self.agents()
            .update_one(
                doc! { "agent_id": agent_id },
                doc! {
                    "$push": { "assigned_skills": skill_bson },
                    "$set": { "updated_at": bson::DateTime::from_chrono(now) }
                },
                None,
            )
            .await?;

        Ok(self.get_agent(agent_id).await?)
    }

    /// Remove a skill from an agent's assigned_skills.
    pub async fn remove_skill_from_agent(
        &self,
        agent_id: &str,
        skill_id: &str,
    ) -> Result<Option<TeamAgent>, ServiceError> {
        let now = Utc::now();
        self.agents()
            .update_one(
                doc! { "agent_id": agent_id },
                doc! {
                    "$pull": { "assigned_skills": { "skill_id": skill_id } },
                    "$set": { "updated_at": bson::DateTime::from_chrono(now) }
                },
                None,
            )
            .await?;

        Ok(self.get_agent(agent_id).await?)
    }

    /// List available team skills that are not yet assigned to the agent.
    pub async fn list_available_skills(
        &self,
        agent_id: &str,
        team_id: &str,
    ) -> Result<Vec<serde_json::Value>, ServiceError> {
        use agime_team::services::mongo::skill_service_mongo::SkillService;

        let skill_service = SkillService::new((*self.db).clone());

        // Get agent's currently assigned skill IDs
        let agent = self
            .get_agent(agent_id)
            .await?
            .ok_or_else(|| ServiceError::Internal("Agent not found".to_string()))?;

        let assigned_ids: std::collections::HashSet<String> = agent
            .assigned_skills
            .iter()
            .map(|s| s.skill_id.clone())
            .collect();

        // Get all team skills via list() with large limit
        let result = skill_service
            .list(team_id, Some(1), Some(200), None, None)
            .await
            .map_err(|e| ServiceError::Internal(e.to_string()))?;

        // Filter out already assigned
        let available: Vec<serde_json::Value> = result
            .items
            .into_iter()
            .filter(|s| !assigned_ids.contains(&s.id))
            .map(|s| {
                serde_json::json!({
                    "id": s.id,
                    "name": s.name,
                    "description": s.description,
                    "version": s.version,
                })
            })
            .collect();

        Ok(available)
    }

    // ========== Session Management ==========

    /// Create a new agent session
    pub async fn create_session(
        &self,
        req: CreateSessionRequest,
    ) -> Result<AgentSessionDoc, mongodb::error::Error> {
        let effective_allowed_skill_ids = self
            .resolve_session_allowed_skill_ids(&req.agent_id, req.allowed_skill_ids.clone())
            .await?;
        let session_id = Uuid::new_v4().to_string();
        let now = bson::DateTime::now();
        let session_source =
            Self::normalize_session_source(req.session_source.clone(), req.portal_restricted);
        let hidden_from_chat_list = req.hidden_from_chat_list.unwrap_or_else(|| {
            req.portal_restricted
                || session_source == "mission"
                || session_source == "system"
                || session_source == "portal_coding"
                || session_source == "portal_manager"
        });

        let doc = AgentSessionDoc {
            id: None,
            session_id: session_id.clone(),
            team_id: req.team_id,
            agent_id: req.agent_id,
            user_id: req.user_id,
            name: req.name,
            status: "active".to_string(),
            messages_json: "[]".to_string(),
            message_count: 0,
            total_tokens: None,
            input_tokens: None,
            output_tokens: None,
            compaction_count: 0,
            disabled_extensions: Vec::new(),
            enabled_extensions: Vec::new(),
            created_at: now,
            updated_at: now,
            // Chat Track fields
            title: None,
            pinned: false,
            last_message_preview: None,
            last_message_at: None,
            is_processing: false,
            attached_document_ids: req.attached_document_ids,
            workspace_path: None,
            extra_instructions: req.extra_instructions,
            allowed_extensions: req.allowed_extensions,
            allowed_skill_ids: effective_allowed_skill_ids,
            retry_config: req.retry_config,
            max_turns: req.max_turns,
            tool_timeout_seconds: req.tool_timeout_seconds,
            max_portal_retry_rounds: req.max_portal_retry_rounds,
            require_final_report: req.require_final_report,
            portal_restricted: req.portal_restricted,
            document_access_mode: req.document_access_mode,
            portal_id: None,
            portal_slug: None,
            visitor_id: None,
            session_source,
            source_mission_id: req.source_mission_id,
            hidden_from_chat_list,
        };

        self.sessions().insert_one(&doc, None).await?;

        Ok(self
            .sessions()
            .find_one(doc! { "session_id": &session_id }, None)
            .await?
            .unwrap_or(doc))
    }

    /// Get a session by session_id
    pub async fn get_session(
        &self,
        session_id: &str,
    ) -> Result<Option<AgentSessionDoc>, mongodb::error::Error> {
        self.sessions()
            .find_one(doc! { "session_id": session_id }, None)
            .await
    }

    /// Find an active (non-deleted) session for a given user + agent pair.
    /// Returns the most recently updated session, if any.
    pub async fn find_active_session_by_user(
        &self,
        user_id: &str,
        agent_id: &str,
    ) -> Result<Option<AgentSessionDoc>, mongodb::error::Error> {
        let opts = mongodb::options::FindOneOptions::builder()
            .sort(doc! { "updated_at": -1 })
            .build();
        self.sessions()
            .find_one(
                doc! {
                    "user_id": user_id,
                    "agent_id": agent_id,
                    "status": "active",
                },
                opts,
            )
            .await
    }

    /// Find an active restricted portal session by user + agent + portal.
    /// Returns the most recently updated session, if any.
    pub async fn find_active_portal_session(
        &self,
        user_id: &str,
        agent_id: &str,
        portal_id: &str,
    ) -> Result<Option<AgentSessionDoc>, mongodb::error::Error> {
        let opts = mongodb::options::FindOneOptions::builder()
            .sort(doc! { "updated_at": -1 })
            .build();
        self.sessions()
            .find_one(
                doc! {
                    "user_id": user_id,
                    "agent_id": agent_id,
                    "status": "active",
                    "portal_restricted": true,
                    "portal_id": portal_id,
                },
                opts,
            )
            .await
    }

    /// List portal-scoped sessions for a visitor (M-4: filter by portal_id).
    pub async fn list_portal_sessions(
        &self,
        portal_id: &str,
        user_id: &str,
        limit: i64,
    ) -> Result<Vec<AgentSessionDoc>, mongodb::error::Error> {
        use futures::TryStreamExt;
        let opts = mongodb::options::FindOptions::builder()
            .sort(doc! { "updated_at": -1 })
            .limit(limit)
            .build();
        let cursor = self
            .sessions()
            .find(
                doc! {
                    "user_id": user_id,
                    "portal_id": portal_id,
                    "portal_restricted": true,
                },
                opts,
            )
            .await?;
        cursor.try_collect().await
    }

    /// Update session messages and token stats (called by executor after each loop)
    pub async fn update_session_messages(
        &self,
        session_id: &str,
        messages_json: &str,
        message_count: i32,
        total_tokens: Option<i32>,
        input_tokens: Option<i32>,
        output_tokens: Option<i32>,
    ) -> Result<(), mongodb::error::Error> {
        let now = bson::DateTime::now();
        let mut set = doc! {
            "messages_json": messages_json,
            "message_count": message_count,
            "updated_at": now,
        };
        if let Some(t) = total_tokens {
            set.insert("total_tokens", t);
        }
        if let Some(t) = input_tokens {
            set.insert("input_tokens", t);
        }
        if let Some(t) = output_tokens {
            set.insert("output_tokens", t);
        }

        self.sessions()
            .update_one(
                doc! { "session_id": session_id },
                doc! { "$set": set },
                None,
            )
            .await?;
        Ok(())
    }

    /// Increment compaction count.
    pub async fn increment_compaction_count(
        &self,
        session_id: &str,
    ) -> Result<(), mongodb::error::Error> {
        let now = bson::DateTime::now();
        self.sessions()
            .update_one(
                doc! { "session_id": session_id },
                doc! {
                    "$inc": { "compaction_count": 1 },
                    "$set": {
                        "updated_at": now,
                    }
                },
                None,
            )
            .await?;
        Ok(())
    }

    /// Update session extension overrides (disabled/enabled extensions)
    pub async fn update_session_extensions(
        &self,
        session_id: &str,
        disabled_extensions: &[String],
        enabled_extensions: &[String],
    ) -> Result<(), mongodb::error::Error> {
        let now = bson::DateTime::now();
        self.sessions()
            .update_one(
                doc! { "session_id": session_id },
                doc! {
                    "$set": {
                        "disabled_extensions": disabled_extensions,
                        "enabled_extensions": enabled_extensions,
                        "updated_at": now,
                    }
                },
                None,
            )
            .await?;
        Ok(())
    }

    /// List sessions for an agent
    pub async fn list_sessions(
        &self,
        query: SessionListQuery,
    ) -> Result<Vec<AgentSessionDoc>, mongodb::error::Error> {
        let clamped_limit = query.limit.min(100) as i64;
        let skip = ((query.page.saturating_sub(1)) * query.limit.min(100)) as u64;

        let mut filter = doc! {
            "team_id": &query.team_id,
            "agent_id": &query.agent_id,
        };
        if let Some(ref uid) = query.user_id {
            filter.insert("user_id", uid);
        }

        let options = mongodb::options::FindOptions::builder()
            .sort(doc! { "updated_at": -1 })
            .skip(skip)
            .limit(clamped_limit)
            .build();

        let cursor = self.sessions().find(filter, options).await?;
        cursor.try_collect().await
    }

    /// Archive a session
    pub async fn archive_session(&self, session_id: &str) -> Result<(), mongodb::error::Error> {
        let now = bson::DateTime::now();
        self.sessions()
            .update_one(
                doc! { "session_id": session_id },
                doc! { "$set": {
                    "status": "archived",
                    "updated_at": now,
                }},
                None,
            )
            .await?;
        Ok(())
    }

    /// Atomically archive a session only if it is not currently processing.
    pub async fn archive_session_if_idle(
        &self,
        session_id: &str,
    ) -> Result<bool, mongodb::error::Error> {
        let now = bson::DateTime::now();
        let result = self
            .sessions()
            .update_one(
                doc! { "session_id": session_id, "is_processing": false },
                doc! { "$set": {
                    "status": "archived",
                    "updated_at": now,
                }},
                None,
            )
            .await?;
        Ok(result.modified_count > 0)
    }

    // ========== Chat Track Methods (Phase 1) ==========

    /// List user sessions with lightweight items (no messages_json).
    /// Joins agent_name from team_agents collection.
    pub async fn list_user_sessions(
        &self,
        query: UserSessionListQuery,
    ) -> Result<Vec<SessionListItem>, mongodb::error::Error> {
        let clamped_limit = query.limit.min(100) as i64;
        let skip = ((query.page.saturating_sub(1)) * query.limit.min(100)) as u64;

        let mut filter = doc! { "team_id": &query.team_id };
        // C1 fix: Always filter by user_id to prevent data leakage
        if let Some(ref user_id) = query.user_id {
            filter.insert("user_id", user_id);
        }
        if let Some(ref agent_id) = query.agent_id {
            filter.insert("agent_id", agent_id);
        }
        if let Some(ref status) = query.status {
            filter.insert("status", status);
        } else {
            filter.insert("status", "active");
        }
        if !query.include_hidden {
            filter.insert(
                "$or",
                vec![
                    doc! { "hidden_from_chat_list": { "$exists": false } },
                    doc! { "hidden_from_chat_list": false },
                ],
            );
        }

        let options = mongodb::options::FindOptions::builder()
            .sort(doc! { "pinned": -1, "last_message_at": -1, "updated_at": -1 })
            .skip(skip)
            .limit(clamped_limit)
            .projection(doc! {
                "session_id": 1, "agent_id": 1, "title": 1,
                "last_message_preview": 1, "last_message_at": 1,
                "message_count": 1, "status": 1, "pinned": 1,
                "created_at": 1, "user_id": 1,
                "team_id": 1, "updated_at": 1,
            })
            .build();

        let cursor = self.sessions().find(filter, options).await?;
        let sessions: Vec<AgentSessionDoc> = cursor.try_collect().await?;

        // Batch-fetch agent names
        let agent_ids: Vec<&str> = sessions.iter().map(|s| s.agent_id.as_str()).collect();
        let agent_names = self.batch_get_agent_names(&agent_ids).await;

        let items = sessions
            .into_iter()
            .map(|s| {
                let agent_name = agent_names
                    .get(&s.agent_id)
                    .cloned()
                    .unwrap_or_else(|| s.agent_id.clone());
                SessionListItem {
                    session_id: s.session_id,
                    agent_id: s.agent_id,
                    agent_name,
                    title: s.title,
                    last_message_preview: s.last_message_preview,
                    last_message_at: s.last_message_at.map(|d| d.to_chrono().to_rfc3339()),
                    message_count: s.message_count,
                    status: s.status,
                    pinned: s.pinned,
                    created_at: s.created_at.to_chrono().to_rfc3339(),
                }
            })
            .collect();

        Ok(items)
    }

    /// Backfill session source/visibility fields for existing data.
    /// Safe to run repeatedly (idempotent best effort).
    pub async fn backfill_session_source_and_visibility(
        &self,
    ) -> Result<(), mongodb::error::Error> {
        // 1) Portal sessions -> portal source + hidden.
        let _ = self
            .sessions()
            .update_many(
                doc! { "portal_restricted": true },
                doc! { "$set": { "session_source": "portal", "hidden_from_chat_list": true } },
                None,
            )
            .await?;

        // 1.5) Portal coding sessions (legacy rows may have been stored as chat)
        // detect by presence of portal + workspace context and mark as hidden.
        let _ = self
            .sessions()
            .update_many(
                doc! {
                    "portal_restricted": { "$ne": true },
                    "portal_id": { "$exists": true, "$ne": bson::Bson::Null },
                    "workspace_path": { "$exists": true, "$ne": bson::Bson::Null },
                },
                doc! { "$set": { "session_source": "portal_coding", "hidden_from_chat_list": true } },
                None,
            )
            .await?;

        // 2) Mission-bound sessions from mission docs -> mission source + hidden + source_mission_id.
        let mission_opts = mongodb::options::FindOptions::builder()
            .projection(doc! { "mission_id": 1, "session_id": 1, "status": 1 })
            .build();
        // Use raw bson documents here to tolerate legacy mission rows that miss
        // strongly-typed fields (e.g. team_id) and keep migration best-effort.
        let missions_raw = self.db.collection::<bson::Document>("agent_missions");
        let mut mission_cursor = missions_raw.find(doc! {}, mission_opts).await?;
        while let Some(m) = mission_cursor.try_next().await? {
            let mission_id = m.get_str("mission_id").ok().map(|s| s.to_string());
            let session_id = m.get_str("session_id").ok().map(|s| s.to_string());
            if let (Some(mission_id), Some(session_id)) = (mission_id, session_id) {
                let mission_status = m.get_str("status").ok().unwrap_or_default();
                let mut set_doc = doc! {
                    "session_source": "mission",
                    "source_mission_id": mission_id,
                    "hidden_from_chat_list": true,
                };
                if matches!(mission_status, "completed" | "cancelled") {
                    set_doc.insert("status", "archived");
                }
                let _ = self
                    .sessions()
                    .update_one(
                        doc! { "session_id": &session_id },
                        doc! { "$set": set_doc },
                        None,
                    )
                    .await?;
            }
        }

        // 3) Remaining sessions with missing source -> default chat + visible.
        let _ = self
            .sessions()
            .update_many(
                doc! { "session_source": { "$exists": false } },
                doc! { "$set": { "session_source": "chat" } },
                None,
            )
            .await?;
        let _ = self
            .sessions()
            .update_many(
                doc! { "hidden_from_chat_list": { "$exists": false } },
                doc! { "$set": { "hidden_from_chat_list": false } },
                None,
            )
            .await?;
        Ok(())
    }

    /// Batch-fetch agent names by IDs
    async fn batch_get_agent_names(
        &self,
        agent_ids: &[&str],
    ) -> std::collections::HashMap<String, String> {
        let mut map = std::collections::HashMap::new();
        if agent_ids.is_empty() {
            return map;
        }
        let unique: std::collections::HashSet<&str> = agent_ids.iter().copied().collect();
        let ids_bson: Vec<bson::Bson> = unique
            .iter()
            .map(|id| bson::Bson::String(id.to_string()))
            .collect();

        let filter = doc! { "agent_id": { "$in": ids_bson } };
        let opts = mongodb::options::FindOptions::builder()
            .projection(doc! { "agent_id": 1, "name": 1 })
            .build();

        if let Ok(cursor) = self.agents().find(filter, opts).await {
            if let Ok(docs) = cursor.try_collect::<Vec<_>>().await {
                for d in docs {
                    map.insert(d.agent_id.clone(), d.name.clone());
                }
            }
        }
        map
    }

    /// Create a chat session for a specific agent, optionally with attached documents
    #[allow(clippy::too_many_arguments)]
    pub async fn create_chat_session(
        &self,
        team_id: &str,
        agent_id: &str,
        user_id: &str,
        attached_document_ids: Vec<String>,
        extra_instructions: Option<String>,
        allowed_extensions: Option<Vec<String>>,
        allowed_skill_ids: Option<Vec<String>>,
        retry_config: Option<RetryConfig>,
        max_turns: Option<i32>,
        tool_timeout_seconds: Option<u64>,
        max_portal_retry_rounds: Option<u32>,
        require_final_report: bool,
        portal_restricted: bool,
        document_access_mode: Option<String>,
        session_source: Option<String>,
        source_mission_id: Option<String>,
        hidden_from_chat_list: Option<bool>,
    ) -> Result<AgentSessionDoc, mongodb::error::Error> {
        self.create_session(CreateSessionRequest {
            team_id: team_id.to_string(),
            agent_id: agent_id.to_string(),
            user_id: user_id.to_string(),
            name: None,
            attached_document_ids,
            extra_instructions,
            allowed_extensions,
            allowed_skill_ids,
            retry_config,
            max_turns,
            tool_timeout_seconds,
            max_portal_retry_rounds,
            require_final_report,
            portal_restricted,
            document_access_mode,
            session_source,
            source_mission_id,
            hidden_from_chat_list,
        })
        .await
    }

    /// Rename a session
    pub async fn rename_session(
        &self,
        session_id: &str,
        title: &str,
    ) -> Result<(), mongodb::error::Error> {
        let now = bson::DateTime::now();
        self.sessions()
            .update_one(
                doc! { "session_id": session_id },
                doc! { "$set": { "title": title, "updated_at": now } },
                None,
            )
            .await?;
        Ok(())
    }

    /// Pin or unpin a session
    pub async fn pin_session(
        &self,
        session_id: &str,
        pinned: bool,
    ) -> Result<(), mongodb::error::Error> {
        let now = bson::DateTime::now();
        self.sessions()
            .update_one(
                doc! { "session_id": session_id },
                doc! { "$set": { "pinned": pinned, "updated_at": now } },
                None,
            )
            .await?;
        Ok(())
    }

    /// Permanently delete a session
    pub async fn delete_session(&self, session_id: &str) -> Result<bool, mongodb::error::Error> {
        let result = self
            .sessions()
            .delete_one(doc! { "session_id": session_id }, None)
            .await?;
        Ok(result.deleted_count > 0)
    }

    /// Atomically delete a session only if it is not currently processing.
    pub async fn delete_session_if_idle(
        &self,
        session_id: &str,
    ) -> Result<bool, mongodb::error::Error> {
        let result = self
            .sessions()
            .delete_one(
                doc! { "session_id": session_id, "is_processing": false },
                None,
            )
            .await?;
        Ok(result.deleted_count > 0)
    }

    /// Attach documents to a session
    pub async fn attach_documents_to_session(
        &self,
        session_id: &str,
        document_ids: &[String],
    ) -> Result<(), mongodb::error::Error> {
        let now = bson::DateTime::now();
        self.sessions()
            .update_one(
                doc! { "session_id": session_id },
                doc! {
                    "$addToSet": { "attached_document_ids": { "$each": document_ids } },
                    "$set": { "updated_at": now },
                },
                None,
            )
            .await?;
        Ok(())
    }

    /// Detach documents from a session
    pub async fn detach_documents_from_session(
        &self,
        session_id: &str,
        document_ids: &[String],
    ) -> Result<(), mongodb::error::Error> {
        let now = bson::DateTime::now();
        self.sessions()
            .update_one(
                doc! { "session_id": session_id },
                doc! {
                    "$pullAll": { "attached_document_ids": document_ids },
                    "$set": { "updated_at": now },
                },
                None,
            )
            .await?;
        Ok(())
    }

    /// Set session processing state (prevents concurrent sends)
    pub async fn set_session_processing(
        &self,
        session_id: &str,
        is_processing: bool,
    ) -> Result<(), mongodb::error::Error> {
        let now = bson::DateTime::now();
        self.sessions()
            .update_one(
                doc! { "session_id": session_id },
                doc! { "$set": {
                    "is_processing": is_processing,
                    "updated_at": now,
                }},
                None,
            )
            .await?;
        Ok(())
    }

    /// Atomically try to set is_processing = true, only if currently false.
    /// Returns Ok(true) if successfully claimed, Ok(false) if already processing.
    /// This prevents TOCTOU race conditions on concurrent send_message calls.
    pub async fn try_start_processing(
        &self,
        session_id: &str,
        user_id: &str,
    ) -> Result<bool, mongodb::error::Error> {
        let now = bson::DateTime::now();
        // Try normal claim: is_processing == false
        let result = self
            .sessions()
            .find_one_and_update(
                doc! {
                    "session_id": session_id,
                    "user_id": user_id,
                    "is_processing": false,
                },
                doc! { "$set": {
                    "is_processing": true,
                    "updated_at": now,
                }},
                None,
            )
            .await?;
        if result.is_some() {
            return Ok(true);
        }
        // Auto-recover: if stuck processing > 10 minutes, force claim
        let stale_cutoff =
            bson::DateTime::from_chrono(chrono::Utc::now() - chrono::Duration::minutes(10));
        let recovered = self
            .sessions()
            .find_one_and_update(
                doc! {
                    "session_id": session_id,
                    "user_id": user_id,
                    "is_processing": true,
                    "updated_at": { "$lt": stale_cutoff },
                },
                doc! { "$set": {
                    "is_processing": true,
                    "updated_at": now,
                }},
                None,
            )
            .await?;
        if recovered.is_some() {
            tracing::warn!("Auto-recovered stuck session {}", session_id);
        }
        Ok(recovered.is_some())
    }

    /// Reset stuck `is_processing` flags for sessions that have been processing
    /// longer than the given timeout. This recovers from crashed/timed-out executions.
    pub async fn reset_stuck_processing(
        &self,
        max_age: std::time::Duration,
    ) -> Result<u64, mongodb::error::Error> {
        let cutoff = bson::DateTime::from_chrono(
            chrono::Utc::now() - chrono::Duration::from_std(max_age).unwrap_or_default(),
        );
        let now = bson::DateTime::now();
        let result = self
            .sessions()
            .update_many(
                doc! {
                    "is_processing": true,
                    "updated_at": { "$lt": cutoff },
                },
                doc! { "$set": {
                    "is_processing": false,
                    "updated_at": now,
                }},
                None,
            )
            .await?;
        Ok(result.modified_count)
    }

    /// Update session metadata after a message completes
    pub async fn update_session_after_message(
        &self,
        session_id: &str,
        messages_json: &str,
        message_count: i32,
        last_preview: &str,
        title: Option<&str>,
        tokens: Option<i32>,
    ) -> Result<(), mongodb::error::Error> {
        let now = bson::DateTime::now();
        // H2 fix: use chars() for safe Unicode slicing
        let preview: String = if last_preview.chars().count() > 200 {
            let truncated: String = last_preview.chars().take(197).collect();
            format!("{}...", truncated)
        } else {
            last_preview.to_string()
        };

        let mut set = doc! {
            "messages_json": messages_json,
            "message_count": message_count,
            "last_message_preview": &preview,
            "last_message_at": now,
            "is_processing": false,
            "updated_at": now,
        };
        if let Some(t) = title {
            set.insert("title", t);
        }
        if let Some(t) = tokens {
            set.insert("total_tokens", t);
        }

        self.sessions()
            .update_one(
                doc! { "session_id": session_id },
                doc! { "$set": set },
                None,
            )
            .await?;
        Ok(())
    }

    /// Append a hidden assistant-visible system notice into an existing session.
    /// This is useful when runtime policy changes and the agent must treat old
    /// conversation assumptions as stale without cluttering the user's UI.
    pub async fn append_hidden_session_notice(
        &self,
        session_id: &str,
        text: &str,
    ) -> Result<(), mongodb::error::Error> {
        let notice = text.trim();
        if notice.is_empty() {
            return Ok(());
        }
        let Some(session) = self.get_session(session_id).await? else {
            return Ok(());
        };
        let mut messages: Vec<serde_json::Value> =
            serde_json::from_str(&session.messages_json).unwrap_or_default();
        let duplicate_last = messages.last().and_then(|msg| {
            let role = msg.get("role").and_then(serde_json::Value::as_str)?;
            if role != "assistant" {
                return None;
            }
            let content = msg.get("content")?.as_array()?;
            let first = content.first()?;
            first.get("text").and_then(serde_json::Value::as_str)
        }) == Some(notice);
        if duplicate_last {
            return Ok(());
        }

        messages.push(serde_json::json!({
            "id": null,
            "role": "assistant",
            "created": chrono::Utc::now().timestamp(),
            "content": [
                {
                    "type": "text",
                    "text": notice,
                }
            ],
            "metadata": {
                "userVisible": false,
                "agentVisible": true,
                "systemGenerated": true,
            }
        }));

        let messages_json =
            serde_json::to_string(&messages).unwrap_or_else(|_| session.messages_json.clone());
        let preview = session
            .last_message_preview
            .as_deref()
            .unwrap_or(notice)
            .to_string();
        self.update_session_after_message(
            session_id,
            &messages_json,
            messages.len() as i32,
            &preview,
            session.title.as_deref(),
            session.total_tokens,
        )
        .await
    }

    /// Persist chat runtime stream events for replay/analysis.
    pub async fn save_chat_stream_events(
        &self,
        items: &[(String, String, u64, StreamEvent)],
    ) -> Result<(), mongodb::error::Error> {
        if items.is_empty() {
            return Ok(());
        }

        let docs: Vec<ChatEventDoc> = items
            .iter()
            .map(|(session_id, run_id, event_id, event)| ChatEventDoc {
                id: None,
                session_id: session_id.clone(),
                run_id: Some(run_id.clone()),
                event_id: (*event_id).try_into().unwrap_or(i64::MAX),
                event_type: event.event_type().to_string(),
                payload: serde_json::to_value(event).unwrap_or_else(|_| serde_json::json!({})),
                created_at: bson::DateTime::now(),
            })
            .collect();

        // Best-effort retry for transient write failures.
        // Use unordered insert so one duplicate key does not abort the whole batch.
        let opts = mongodb::options::InsertManyOptions::builder()
            .ordered(false)
            .build();
        let mut attempt: u8 = 0;
        loop {
            match self
                .chat_events()
                .insert_many(docs.clone(), opts.clone())
                .await
            {
                Ok(_) => return Ok(()),
                Err(e) => {
                    let msg = e.to_string();
                    let duplicate_only = msg.contains("E11000")
                        || msg.to_ascii_lowercase().contains("duplicate key");
                    if duplicate_only {
                        return Ok(());
                    }

                    attempt = attempt.saturating_add(1);
                    if attempt >= 3 {
                        return Err(e);
                    }

                    let backoff_ms = 50_u64 * (attempt as u64);
                    tokio::time::sleep(std::time::Duration::from_millis(backoff_ms)).await;
                }
            }
        }
    }

    pub async fn list_chat_events(
        &self,
        session_id: &str,
        run_id: Option<&str>,
        after_event_id: Option<u64>,
        before_event_id: Option<u64>,
        limit: u32,
        descending: bool,
    ) -> Result<Vec<ChatEventDoc>, mongodb::error::Error> {
        let clamped_limit = limit.clamp(1, 2000);
        let mut filter = doc! { "session_id": session_id };
        if let Some(run) = run_id {
            filter.insert("run_id", run);
        }
        let mut event_id_filter = Document::new();
        if let Some(after) = after_event_id {
            event_id_filter.insert("$gt", after as i64);
        }
        if let Some(before) = before_event_id {
            event_id_filter.insert("$lt", before as i64);
        }
        if !event_id_filter.is_empty() {
            filter.insert("event_id", event_id_filter);
        }
        let sort_dir = if descending { -1 } else { 1 };

        let opts = mongodb::options::FindOptions::builder()
            .sort(doc! { "event_id": sort_dir, "created_at": sort_dir })
            .limit(clamped_limit as i64)
            .build();
        let mut events: Vec<ChatEventDoc> = self
            .chat_events()
            .find(filter.clone(), opts.clone())
            .await?
            .try_collect()
            .await?;

        // Handle run switches gracefully:
        // if caller sends a stale `after_event_id` from a previous run,
        // restart from beginning of current run instead of returning empty forever.
        if !descending && before_event_id.is_none() && events.is_empty() {
            if let (Some(run), Some(after)) = (run_id, after_event_id) {
                let max_opts = mongodb::options::FindOneOptions::builder()
                    .sort(doc! { "event_id": -1, "created_at": -1 })
                    .build();
                let max_doc = self
                    .chat_events()
                    .find_one(doc! { "session_id": session_id, "run_id": run }, max_opts)
                    .await?;
                if let Some(latest) = max_doc {
                    if latest.event_id < after as i64 {
                        let restart_filter = doc! { "session_id": session_id, "run_id": run };
                        events = self
                            .chat_events()
                            .find(restart_filter, opts)
                            .await?
                            .try_collect()
                            .await?;
                    }
                }
            }
        }

        Ok(events)
    }

    // ═══════════════════════════════════════════════════════
    // Mission Track (Phase 2) - Collection accessors
    // ═══════════════════════════════════════════════════════

    fn missions(&self) -> mongodb::Collection<MissionDoc> {
        self.db.collection("agent_missions")
    }

    fn artifacts(&self) -> mongodb::Collection<MissionArtifactDoc> {
        self.db.collection("agent_mission_artifacts")
    }

    fn mission_events(&self) -> mongodb::Collection<MissionEventDoc> {
        self.db.collection("agent_mission_events")
    }

    // ─── Mission CRUD ────────────────────────────────────

    pub async fn create_mission(
        &self,
        req: &CreateMissionRequest,
        team_id: &str,
        creator_id: &str,
    ) -> Result<MissionDoc, mongodb::error::Error> {
        let now = bson::DateTime::now();
        let step_timeout_seconds = req
            .step_timeout_seconds
            .filter(|v| *v > 0)
            .map(|v| v.min(7200));
        let step_max_retries = req.step_max_retries.filter(|v| *v > 0).map(|v| v.min(8));
        let mission = MissionDoc {
            id: None,
            mission_id: Uuid::new_v4().to_string(),
            team_id: team_id.to_string(),
            agent_id: req.agent_id.clone(),
            creator_id: creator_id.to_string(),
            goal: req.goal.clone(),
            context: req.context.clone(),
            status: MissionStatus::Draft,
            approval_policy: req.approval_policy.clone().unwrap_or_default(),
            steps: vec![],
            current_step: None,
            session_id: None,
            source_chat_session_id: req.source_chat_session_id.clone(),
            token_budget: req.token_budget.unwrap_or(0),
            total_tokens_used: 0,
            priority: req.priority.unwrap_or(0),
            step_timeout_seconds,
            step_max_retries,
            plan_version: 1,
            execution_mode: req.execution_mode.clone().unwrap_or_default(),
            execution_profile: req.execution_profile.clone().unwrap_or_default(),
            goal_tree: None,
            current_goal_id: None,
            total_pivots: 0,
            total_abandoned: 0,
            error_message: None,
            final_summary: None,
            created_at: now,
            updated_at: now,
            started_at: None,
            completed_at: None,
            attached_document_ids: req.attached_document_ids.clone(),
            workspace_path: None,
            current_run_id: None,
        };
        self.missions().insert_one(&mission, None).await?;
        Ok(mission)
    }

    pub async fn get_mission(
        &self,
        mission_id: &str,
    ) -> Result<Option<MissionDoc>, mongodb::error::Error> {
        self.missions()
            .find_one(doc! { "mission_id": mission_id }, None)
            .await
    }

    /// Attach documents to a mission
    pub async fn attach_documents_to_mission(
        &self,
        mission_id: &str,
        document_ids: &[String],
    ) -> Result<(), mongodb::error::Error> {
        let now = bson::DateTime::now();
        self.missions()
            .update_one(
                doc! { "mission_id": mission_id },
                doc! {
                    "$addToSet": { "attached_document_ids": { "$each": document_ids } },
                    "$set": { "updated_at": now },
                },
                None,
            )
            .await?;
        Ok(())
    }

    /// Detach documents from a mission
    pub async fn detach_documents_from_mission(
        &self,
        mission_id: &str,
        document_ids: &[String],
    ) -> Result<(), mongodb::error::Error> {
        let now = bson::DateTime::now();
        self.missions()
            .update_one(
                doc! { "mission_id": mission_id },
                doc! {
                    "$pullAll": { "attached_document_ids": document_ids },
                    "$set": { "updated_at": now },
                },
                None,
            )
            .await?;
        Ok(())
    }

    pub async fn list_missions(
        &self,
        query: ListMissionsQuery,
    ) -> Result<Vec<MissionListItem>, mongodb::error::Error> {
        let mut filter = doc! { "team_id": &query.team_id };
        if let Some(ref aid) = query.agent_id {
            filter.insert("agent_id", aid);
        }
        if let Some(ref s) = query.status {
            filter.insert("status", s);
        }

        let skip = ((query.page.max(1) - 1) * query.limit) as u64;
        let opts = mongodb::options::FindOptions::builder()
            .sort(doc! { "updated_at": -1 })
            .skip(skip)
            .limit(query.limit as i64)
            .build();

        let cursor = self.missions().find(filter, opts).await?;
        let missions: Vec<MissionDoc> = cursor.try_collect().await?;

        // Batch-fetch agent names to avoid N+1 queries
        let agent_ids: Vec<&str> = missions.iter().map(|m| m.agent_id.as_str()).collect();
        let agent_names = self.batch_get_agent_names(&agent_ids).await;

        let mut items = Vec::with_capacity(missions.len());
        for m in missions {
            let agent_name = agent_names.get(&m.agent_id).cloned().unwrap_or_default();

            let completed_steps = m
                .steps
                .iter()
                .filter(|s| s.status == StepStatus::Completed)
                .count();

            let goal_count = m
                .goal_tree
                .as_ref()
                .map(|g| {
                    g.iter()
                        .filter(|n| n.status != GoalStatus::Abandoned)
                        .count()
                })
                .unwrap_or(0);
            let completed_goals = m
                .goal_tree
                .as_ref()
                .map(|g| {
                    g.iter()
                        .filter(|n| n.status == GoalStatus::Completed)
                        .count()
                })
                .unwrap_or(0);

            let resolved_execution_profile = resolve_execution_profile(&m);
            items.push(MissionListItem {
                mission_id: m.mission_id,
                agent_id: m.agent_id,
                agent_name,
                goal: m.goal,
                status: m.status,
                approval_policy: m.approval_policy,
                step_count: m.steps.len(),
                completed_steps,
                current_step: m.current_step,
                total_tokens_used: m.total_tokens_used,
                created_at: m.created_at.to_chrono().to_rfc3339(),
                updated_at: m.updated_at.to_chrono().to_rfc3339(),
                execution_mode: m.execution_mode.clone(),
                execution_profile: m.execution_profile.clone(),
                resolved_execution_profile,
                goal_count,
                completed_goals,
                pivots: m.total_pivots,
                attached_doc_count: m.attached_document_ids.len(),
            });
        }
        Ok(items)
    }

    pub async fn delete_mission(&self, mission_id: &str) -> Result<bool, mongodb::error::Error> {
        // Only Draft, Cancelled, or Failed missions can be deleted
        let result = self
            .missions()
            .delete_one(
                doc! {
                    "mission_id": mission_id,
                    "status": { "$in": ["draft", "cancelled", "failed"] }
                },
                None,
            )
            .await?;
        Ok(result.deleted_count > 0)
    }

    // ─── Mission Status Management ───────────────────────

    /// Update mission status with atomic precondition to prevent race conditions.
    /// Returns an error when the transition precondition is not satisfied.
    pub async fn update_mission_status(
        &self,
        mission_id: &str,
        status: &MissionStatus,
    ) -> Result<(), mongodb::error::Error> {
        let now = bson::DateTime::now();
        let should_set_started_at = if matches!(status, MissionStatus::Running) {
            self.get_mission(mission_id)
                .await?
                .map(|m| m.started_at.is_none())
                .unwrap_or(true)
        } else {
            false
        };
        let status_bson = bson::to_bson(status)
            .map_err(|e| mongodb::error::Error::custom(format!("BSON serialize error: {}", e)))?;
        let mut set = doc! {
            "status": status_bson,
            "updated_at": now,
        };
        // Set timestamps for terminal states
        match status {
            MissionStatus::Running | MissionStatus::Planning => {
                // Clear terminal timestamp when mission re-enters active states.
                set.insert("completed_at", bson::Bson::Null);
                if should_set_started_at {
                    set.insert("started_at", now);
                }
                // Stamp server instance for orphaned mission recovery
                if let Ok(iid) = std::env::var("TEAM_SERVER_INSTANCE_ID") {
                    set.insert("server_instance_id", iid);
                }
            }
            MissionStatus::Completed | MissionStatus::Failed | MissionStatus::Cancelled => {
                set.insert("completed_at", now);
            }
            _ => {}
        }

        // Atomic precondition: only allow valid state transitions
        let allowed_from: Vec<&str> = match status {
            MissionStatus::Planning => vec!["draft", "planned"],
            MissionStatus::Planned => vec!["planning", "paused", "failed"],
            MissionStatus::Running => vec!["draft", "planned", "planning", "paused", "failed"],
            MissionStatus::Paused => vec!["running", "planning"],
            MissionStatus::Completed => vec!["running"],
            MissionStatus::Failed => vec!["running", "planning", "paused", "planned"],
            MissionStatus::Cancelled => vec!["draft", "planned", "running", "paused", "planning"],
            _ => vec![],
        };

        let filter = if allowed_from.is_empty() {
            doc! { "mission_id": mission_id }
        } else {
            let bson_arr: Vec<bson::Bson> = allowed_from
                .iter()
                .map(|s| bson::Bson::String(s.to_string()))
                .collect();
            doc! { "mission_id": mission_id, "status": { "$in": bson_arr } }
        };

        let result = self
            .missions()
            .update_one(filter, doc! { "$set": set }, None)
            .await?;

        if result.modified_count == 0 {
            if let Some(current) = self.get_mission(mission_id).await? {
                if current.status == *status {
                    // Idempotent transition request; treat as success.
                    return Ok(());
                }
                return Err(mongodb::error::Error::custom(format!(
                    "mission status transition rejected: mission_id={}, from={:?}, to={:?}",
                    mission_id, current.status, status
                )));
            }
            tracing::warn!(
                "update_mission_status: no update for mission {} to {:?} (precondition failed)",
                mission_id,
                status
            );
            return Err(mongodb::error::Error::custom(format!(
                "mission not found or transition rejected: mission_id={}, to={:?}",
                mission_id, status
            )));
        }
        Ok(())
    }

    pub async fn set_mission_session(
        &self,
        mission_id: &str,
        session_id: &str,
    ) -> Result<(), mongodb::error::Error> {
        self.missions()
            .update_one(
                doc! { "mission_id": mission_id },
                doc! { "$set": {
                    "session_id": session_id,
                    "updated_at": bson::DateTime::now(),
                }},
                None,
            )
            .await?;
        Ok(())
    }

    pub async fn set_mission_current_run(
        &self,
        mission_id: &str,
        run_id: &str,
    ) -> Result<(), mongodb::error::Error> {
        self.missions()
            .update_one(
                doc! { "mission_id": mission_id },
                doc! { "$set": {
                    "current_run_id": run_id,
                    "updated_at": bson::DateTime::now(),
                }},
                None,
            )
            .await?;
        Ok(())
    }

    pub async fn set_mission_workspace(
        &self,
        mission_id: &str,
        path: &str,
    ) -> Result<(), mongodb::error::Error> {
        let normalized = normalize_workspace_path(path);
        self.missions()
            .update_one(
                doc! { "mission_id": mission_id },
                doc! { "$set": {
                    "workspace_path": normalized,
                    "updated_at": bson::DateTime::now(),
                }},
                None,
            )
            .await?;
        Ok(())
    }

    pub async fn set_session_workspace(
        &self,
        session_id: &str,
        path: &str,
    ) -> Result<(), mongodb::error::Error> {
        let normalized = normalize_workspace_path(path);
        self.sessions()
            .update_one(
                doc! { "session_id": session_id },
                doc! { "$set": {
                    "workspace_path": normalized,
                    "updated_at": bson::DateTime::now(),
                }},
                None,
            )
            .await?;
        Ok(())
    }

    /// Bind an existing session to a portal context.
    pub async fn set_session_portal_context(
        &self,
        session_id: &str,
        portal_id: &str,
        portal_slug: &str,
        visitor_id: Option<&str>,
        document_access_mode: Option<&str>,
        portal_restricted: bool,
    ) -> Result<(), mongodb::error::Error> {
        let mut set_doc = doc! {
            "portal_restricted": portal_restricted,
            "portal_id": portal_id,
            "portal_slug": portal_slug,
            "updated_at": bson::DateTime::now(),
        };
        match document_access_mode {
            Some(mode) if !mode.trim().is_empty() => {
                set_doc.insert("document_access_mode", mode.trim());
            }
            _ => {
                set_doc.insert("document_access_mode", bson::Bson::Null);
            }
        }
        match visitor_id {
            Some(v) if !v.trim().is_empty() => {
                set_doc.insert("visitor_id", v.trim());
            }
            _ => {
                set_doc.insert("visitor_id", bson::Bson::Null);
            }
        }
        self.sessions()
            .update_one(
                doc! { "session_id": session_id },
                doc! { "$set": set_doc },
                None,
            )
            .await?;
        Ok(())
    }

    /// Sync portal runtime policy for an existing session so portal config changes
    /// (documents, allowlists, prompt constraints) take effect immediately.
    #[allow(clippy::too_many_arguments)]
    pub async fn sync_portal_session_policy(
        &self,
        session_id: &str,
        attached_document_ids: Vec<String>,
        extra_instructions: Option<String>,
        allowed_extensions: Option<Vec<String>>,
        allowed_skill_ids: Option<Vec<String>>,
        retry_config: Option<RetryConfig>,
        require_final_report: bool,
        document_access_mode: Option<String>,
    ) -> Result<(), mongodb::error::Error> {
        let mut set_doc = doc! {
            "attached_document_ids": attached_document_ids,
            "portal_restricted": true,
            "require_final_report": require_final_report,
            "updated_at": bson::DateTime::now(),
        };
        match document_access_mode {
            Some(v) if !v.trim().is_empty() => {
                set_doc.insert("document_access_mode", v);
            }
            _ => {
                set_doc.insert("document_access_mode", bson::Bson::Null);
            }
        }

        match extra_instructions {
            Some(v) if !v.trim().is_empty() => {
                set_doc.insert("extra_instructions", v);
            }
            _ => {
                set_doc.insert("extra_instructions", bson::Bson::Null);
            }
        }
        match allowed_extensions {
            Some(v) => {
                set_doc.insert(
                    "allowed_extensions",
                    mongodb::bson::to_bson(&v).unwrap_or(bson::Bson::Array(vec![])),
                );
            }
            None => {
                set_doc.insert("allowed_extensions", bson::Bson::Null);
            }
        }
        match allowed_skill_ids {
            Some(v) => {
                set_doc.insert(
                    "allowed_skill_ids",
                    mongodb::bson::to_bson(&v).unwrap_or(bson::Bson::Array(vec![])),
                );
            }
            None => {
                set_doc.insert("allowed_skill_ids", bson::Bson::Null);
            }
        }
        match retry_config {
            Some(v) => {
                set_doc.insert(
                    "retry_config",
                    mongodb::bson::to_bson(&v).unwrap_or(bson::Bson::Null),
                );
            }
            None => {
                set_doc.insert("retry_config", bson::Bson::Null);
            }
        }

        self.sessions()
            .update_one(
                doc! { "session_id": session_id },
                doc! { "$set": set_doc },
                None,
            )
            .await?;
        Ok(())
    }

    pub async fn save_mission_plan(
        &self,
        mission_id: &str,
        steps: Vec<MissionStep>,
    ) -> Result<(), mongodb::error::Error> {
        let steps_bson = bson::to_bson(&steps).unwrap_or_default();
        self.missions()
            .update_one(
                doc! { "mission_id": mission_id },
                doc! { "$set": {
                    "steps": steps_bson,
                    "updated_at": bson::DateTime::now(),
                }},
                None,
            )
            .await?;
        Ok(())
    }

    pub async fn set_mission_error(
        &self,
        mission_id: &str,
        error: &str,
    ) -> Result<(), mongodb::error::Error> {
        self.missions()
            .update_one(
                doc! { "mission_id": mission_id },
                doc! { "$set": {
                    "error_message": error,
                    "updated_at": bson::DateTime::now(),
                }},
                None,
            )
            .await?;
        Ok(())
    }

    pub async fn clear_mission_error(&self, mission_id: &str) -> Result<(), mongodb::error::Error> {
        self.missions()
            .update_one(
                doc! { "mission_id": mission_id },
                doc! { "$set": {
                    "error_message": bson::Bson::Null,
                    "updated_at": bson::DateTime::now(),
                }},
                None,
            )
            .await?;
        Ok(())
    }

    pub async fn set_mission_final_summary(
        &self,
        mission_id: &str,
        summary: &str,
    ) -> Result<(), mongodb::error::Error> {
        self.missions()
            .update_one(
                doc! { "mission_id": mission_id },
                doc! { "$set": {
                    "final_summary": summary,
                    "updated_at": bson::DateTime::now(),
                }},
                None,
            )
            .await?;
        Ok(())
    }

    pub async fn add_mission_tokens(
        &self,
        mission_id: &str,
        tokens_used: i32,
    ) -> Result<(), mongodb::error::Error> {
        if tokens_used <= 0 {
            return Ok(());
        }
        self.missions()
            .update_one(
                doc! { "mission_id": mission_id },
                doc! {
                    "$inc": { "total_tokens_used": tokens_used as i64 },
                    "$set": { "updated_at": bson::DateTime::now() },
                },
                None,
            )
            .await?;
        Ok(())
    }

    // ─── Step Management ─────────────────────────────────

    pub async fn update_step_status(
        &self,
        mission_id: &str,
        step_index: u32,
        status: &StepStatus,
    ) -> Result<(), mongodb::error::Error> {
        let field = format!("steps.{}.status", step_index);
        let status_bson = bson::to_bson(status)
            .map_err(|e| mongodb::error::Error::custom(format!("BSON serialize error: {}", e)))?;
        let now = bson::DateTime::now();
        let mut set_doc = doc! {
            &field: status_bson,
            "updated_at": now,
        };
        if matches!(status, StepStatus::Running) {
            let started_field = format!("steps.{}.started_at", step_index);
            set_doc.insert(started_field, now);
        }
        self.missions()
            .update_one(
                doc! { "mission_id": mission_id },
                doc! { "$set": set_doc },
                None,
            )
            .await?;
        Ok(())
    }

    pub async fn set_step_supervision(
        &self,
        mission_id: &str,
        step_index: u32,
        state: StepSupervisorState,
        last_activity_at: Option<bson::DateTime>,
        last_progress_at: Option<bson::DateTime>,
        progress_score: Option<i32>,
        current_blocker: Option<&str>,
        last_supervisor_hint: Option<&str>,
        increment_stall_count: bool,
        stall_count_override: Option<u32>,
    ) -> Result<(), mongodb::error::Error> {
        let state_field = format!("steps.{}.supervisor_state", step_index);
        let last_activity_field = format!("steps.{}.last_activity_at", step_index);
        let last_progress_field = format!("steps.{}.last_progress_at", step_index);
        let progress_score_field = format!("steps.{}.progress_score", step_index);
        let current_blocker_field = format!("steps.{}.current_blocker", step_index);
        let last_supervisor_hint_field = format!("steps.{}.last_supervisor_hint", step_index);
        let stall_count_field = format!("steps.{}.stall_count", step_index);
        let state_bson = bson::to_bson(&state)
            .map_err(|e| mongodb::error::Error::custom(format!("BSON serialize error: {}", e)))?;
        let now = bson::DateTime::now();
        let mut set_doc = doc! {
            &state_field: state_bson,
            "updated_at": now,
        };
        match last_activity_at {
            Some(ts) => {
                set_doc.insert(&last_activity_field, ts);
            }
            None => {
                set_doc.insert(&last_activity_field, Bson::Null);
            }
        }
        match last_progress_at {
            Some(ts) => {
                set_doc.insert(&last_progress_field, ts);
            }
            None => {
                set_doc.insert(&last_progress_field, Bson::Null);
            }
        }
        match progress_score {
            Some(score) => {
                set_doc.insert(&progress_score_field, score);
            }
            None => {
                set_doc.insert(&progress_score_field, Bson::Null);
            }
        }
        match current_blocker {
            Some(blocker) if !blocker.trim().is_empty() => {
                set_doc.insert(&current_blocker_field, blocker);
            }
            _ => {
                set_doc.insert(&current_blocker_field, Bson::Null);
            }
        }
        match last_supervisor_hint {
            Some(hint) if !hint.trim().is_empty() => {
                set_doc.insert(&last_supervisor_hint_field, hint);
            }
            _ => {
                set_doc.insert(&last_supervisor_hint_field, Bson::Null);
            }
        }
        if let Some(stall_count) = stall_count_override {
            set_doc.insert(&stall_count_field, stall_count as i64);
        }

        let mut update_doc = doc! { "$set": set_doc };
        if increment_stall_count && stall_count_override.is_none() {
            update_doc.insert("$inc", doc! { &stall_count_field: 1 });
        }

        self.missions()
            .update_one(doc! { "mission_id": mission_id }, update_doc, None)
            .await?;
        Ok(())
    }

    pub async fn complete_step(
        &self,
        mission_id: &str,
        step_index: u32,
        tokens_used: i32,
    ) -> Result<(), mongodb::error::Error> {
        let status_field = format!("steps.{}.status", step_index);
        let completed_field = format!("steps.{}.completed_at", step_index);
        let tokens_field = format!("steps.{}.tokens_used", step_index);
        let now = bson::DateTime::now();
        self.missions()
            .update_one(
                doc! { "mission_id": mission_id },
                doc! {
                    "$set": {
                        &status_field: "completed",
                        &completed_field: now,
                        &tokens_field: tokens_used,
                        "updated_at": now,
                    },
                    "$inc": { "total_tokens_used": tokens_used as i64 },
                },
                None,
            )
            .await?;
        Ok(())
    }

    pub async fn fail_step(
        &self,
        mission_id: &str,
        step_index: u32,
        error: &str,
    ) -> Result<(), mongodb::error::Error> {
        let status_field = format!("steps.{}.status", step_index);
        let error_field = format!("steps.{}.error_message", step_index);
        let completed_field = format!("steps.{}.completed_at", step_index);
        let now = bson::DateTime::now();
        self.missions()
            .update_one(
                doc! { "mission_id": mission_id },
                doc! { "$set": {
                    &status_field: "failed",
                    &error_field: error,
                    &completed_field: now,
                    "updated_at": now,
                }},
                None,
            )
            .await?;
        Ok(())
    }

    /// Save the structured output summary for a completed step.
    pub async fn set_step_output_summary(
        &self,
        mission_id: &str,
        step_index: u32,
        summary: &str,
    ) -> Result<(), mongodb::error::Error> {
        let field = format!("steps.{}.output_summary", step_index);
        self.missions()
            .update_one(
                doc! { "mission_id": mission_id },
                doc! { "$set": {
                    &field: summary,
                    "updated_at": bson::DateTime::now(),
                }},
                None,
            )
            .await?;
        Ok(())
    }

    /// Persist step runtime contract extracted from mission_preflight.
    pub async fn set_step_runtime_contract(
        &self,
        mission_id: &str,
        step_index: u32,
        contract: &RuntimeContract,
    ) -> Result<(), mongodb::error::Error> {
        let field = format!("steps.{}.runtime_contract", step_index);
        let contract_bson = bson::to_bson(contract)
            .map_err(|e| mongodb::error::Error::custom(format!("BSON serialize error: {}", e)))?;
        self.missions()
            .update_one(
                doc! { "mission_id": mission_id },
                doc! { "$set": {
                    &field: contract_bson,
                    "updated_at": bson::DateTime::now(),
                }},
                None,
            )
            .await?;
        Ok(())
    }

    /// Persist step contract verification result.
    pub async fn set_step_contract_verification(
        &self,
        mission_id: &str,
        step_index: u32,
        verification: &RuntimeContractVerification,
    ) -> Result<(), mongodb::error::Error> {
        let field = format!("steps.{}.contract_verification", step_index);
        let verify_bson = bson::to_bson(verification)
            .map_err(|e| mongodb::error::Error::custom(format!("BSON serialize error: {}", e)))?;
        self.missions()
            .update_one(
                doc! { "mission_id": mission_id },
                doc! { "$set": {
                    &field: verify_bson,
                    "updated_at": bson::DateTime::now(),
                }},
                None,
            )
            .await?;
        Ok(())
    }

    pub async fn set_step_tool_calls(
        &self,
        mission_id: &str,
        step_index: u32,
        tool_calls: &[super::mission_mongo::ToolCallRecord],
    ) -> Result<(), mongodb::error::Error> {
        let field = format!("steps.{}.tool_calls", step_index);
        let bson_arr = bson::to_bson(tool_calls).unwrap_or(bson::Bson::Array(vec![]));
        self.missions()
            .update_one(
                doc! { "mission_id": mission_id },
                doc! { "$set": { &field: bson_arr } },
                None,
            )
            .await?;
        Ok(())
    }

    pub async fn set_step_observability(
        &self,
        mission_id: &str,
        step_index: u32,
        recent_progress_events: &[super::mission_mongo::StepProgressEvent],
        evidence_bundle: Option<&super::mission_mongo::StepEvidenceBundle>,
    ) -> Result<(), mongodb::error::Error> {
        let events_field = format!("steps.{}.recent_progress_events", step_index);
        let bundle_field = format!("steps.{}.evidence_bundle", step_index);
        let events_bson =
            bson::to_bson(recent_progress_events).unwrap_or(bson::Bson::Array(vec![]));
        let bundle_bson = match evidence_bundle {
            Some(bundle) => bson::to_bson(bundle).unwrap_or(bson::Bson::Null),
            None => bson::Bson::Null,
        };
        self.missions()
            .update_one(
                doc! { "mission_id": mission_id },
                doc! { "$set": {
                    &events_field: events_bson,
                    &bundle_field: bundle_bson,
                    "updated_at": bson::DateTime::now(),
                }},
                None,
            )
            .await?;
        Ok(())
    }

    /// Increment the retry count for a step.
    pub async fn increment_step_retry(
        &self,
        mission_id: &str,
        step_index: u32,
    ) -> Result<(), mongodb::error::Error> {
        let field = format!("steps.{}.retry_count", step_index);
        self.missions()
            .update_one(
                doc! { "mission_id": mission_id },
                doc! {
                    "$inc": { &field: 1 },
                    "$set": { "updated_at": bson::DateTime::now() },
                },
                None,
            )
            .await?;
        Ok(())
    }

    /// Reset a step to pending so mission resume can retry from this step.
    pub async fn reset_step_for_retry(
        &self,
        mission_id: &str,
        step_index: u32,
    ) -> Result<(), mongodb::error::Error> {
        let status_field = format!("steps.{}.status", step_index);
        let error_field = format!("steps.{}.error_message", step_index);
        let started_field = format!("steps.{}.started_at", step_index);
        let completed_field = format!("steps.{}.completed_at", step_index);
        let summary_field = format!("steps.{}.output_summary", step_index);
        let tool_calls_field = format!("steps.{}.tool_calls", step_index);
        let supervisor_state_field = format!("steps.{}.supervisor_state", step_index);
        let last_activity_field = format!("steps.{}.last_activity_at", step_index);
        let last_progress_field = format!("steps.{}.last_progress_at", step_index);
        let progress_score_field = format!("steps.{}.progress_score", step_index);
        let current_blocker_field = format!("steps.{}.current_blocker", step_index);
        let last_supervisor_hint_field = format!("steps.{}.last_supervisor_hint", step_index);
        self.missions()
            .update_one(
                doc! { "mission_id": mission_id },
                doc! {
                    "$set": {
                        &status_field: "pending",
                        "updated_at": bson::DateTime::now(),
                    },
                    "$unset": {
                        &error_field: "",
                        &started_field: "",
                        &completed_field: "",
                        &summary_field: "",
                        &tool_calls_field: "",
                        &supervisor_state_field: "",
                        &last_activity_field: "",
                        &last_progress_field: "",
                        &progress_score_field: "",
                        &current_blocker_field: "",
                        &last_supervisor_hint_field: "",
                    },
                },
                None,
            )
            .await?;
        Ok(())
    }

    /// Replace remaining steps after a re-plan, incrementing plan_version.
    pub async fn replan_remaining_steps(
        &self,
        mission_id: &str,
        all_steps: Vec<MissionStep>,
    ) -> Result<(), mongodb::error::Error> {
        let steps_bson = bson::to_bson(&all_steps)
            .map_err(|e| mongodb::error::Error::custom(format!("BSON serialize error: {}", e)))?;
        self.missions()
            .update_one(
                doc! { "mission_id": mission_id },
                doc! {
                    "$set": {
                        "steps": steps_bson,
                        "updated_at": bson::DateTime::now(),
                    },
                    "$inc": { "plan_version": 1 },
                },
                None,
            )
            .await?;
        Ok(())
    }

    pub async fn approve_step(
        &self,
        mission_id: &str,
        step_index: u32,
        approver_id: &str,
    ) -> Result<(), mongodb::error::Error> {
        let status_field = format!("steps.{}.status", step_index);
        let approver_field = format!("steps.{}.approved_by", step_index);
        self.missions()
            .update_one(
                doc! { "mission_id": mission_id },
                doc! { "$set": {
                    &status_field: "pending",
                    &approver_field: approver_id,
                    "updated_at": bson::DateTime::now(),
                }},
                None,
            )
            .await?;
        Ok(())
    }

    pub async fn advance_mission_step(
        &self,
        mission_id: &str,
        next_step: u32,
    ) -> Result<(), mongodb::error::Error> {
        self.missions()
            .update_one(
                doc! { "mission_id": mission_id },
                doc! { "$set": {
                    "current_step": next_step as i32,
                    "updated_at": bson::DateTime::now(),
                }},
                None,
            )
            .await?;
        Ok(())
    }

    // ─── Artifact Management ─────────────────────────────

    pub async fn save_artifact(
        &self,
        artifact: &MissionArtifactDoc,
    ) -> Result<(), mongodb::error::Error> {
        // Upsert by mission_id + file_path to deduplicate across steps/goals
        if let Some(ref fp) = artifact.file_path {
            let filter = doc! {
                "mission_id": &artifact.mission_id,
                "file_path": fp,
            };
            let mut replacement = artifact.clone();
            replacement.id = None;
            let opts = mongodb::options::ReplaceOptions::builder()
                .upsert(true)
                .build();
            self.artifacts()
                .replace_one(filter, replacement, opts)
                .await?;
        } else {
            self.artifacts().insert_one(artifact, None).await?;
        }
        Ok(())
    }

    pub async fn list_mission_artifacts(
        &self,
        mission_id: &str,
    ) -> Result<Vec<MissionArtifactDoc>, mongodb::error::Error> {
        let opts = mongodb::options::FindOptions::builder()
            .sort(doc! { "step_index": 1, "created_at": 1 })
            .build();
        let cursor = self
            .artifacts()
            .find(doc! { "mission_id": mission_id }, opts)
            .await?;
        cursor.try_collect().await
    }

    pub async fn get_artifact(
        &self,
        artifact_id: &str,
    ) -> Result<Option<MissionArtifactDoc>, mongodb::error::Error> {
        self.artifacts()
            .find_one(doc! { "artifact_id": artifact_id }, None)
            .await
    }

    pub async fn set_artifact_document_link(
        &self,
        artifact_id: &str,
        document_id: &str,
        document_status: &str,
    ) -> Result<(), mongodb::error::Error> {
        self.artifacts()
            .update_one(
                doc! { "artifact_id": artifact_id },
                doc! {
                    "$set": {
                        "archived_document_id": document_id,
                        "archived_document_status": document_status,
                        "archived_at": bson::DateTime::now(),
                    }
                },
                None,
            )
            .await?;
        Ok(())
    }

    /// Persist mission runtime stream events for replay/analysis.
    pub async fn save_mission_stream_events(
        &self,
        items: &[(String, String, u64, StreamEvent)],
    ) -> Result<(), mongodb::error::Error> {
        if items.is_empty() {
            return Ok(());
        }

        let docs: Vec<MissionEventDoc> = items
            .iter()
            .map(|(mission_id, run_id, event_id, event)| MissionEventDoc {
                id: None,
                mission_id: mission_id.clone(),
                run_id: Some(run_id.clone()),
                event_id: (*event_id).try_into().unwrap_or(i64::MAX),
                event_type: event.event_type().to_string(),
                payload: serde_json::to_value(event).unwrap_or_else(|_| serde_json::json!({})),
                created_at: bson::DateTime::now(),
            })
            .collect();

        self.mission_events().insert_many(docs, None).await?;
        Ok(())
    }

    pub async fn list_mission_events(
        &self,
        mission_id: &str,
        run_id: Option<&str>,
        after_event_id: Option<u64>,
        limit: u32,
    ) -> Result<Vec<MissionEventDoc>, mongodb::error::Error> {
        let clamped_limit = limit.clamp(1, 2000);
        let mut filter = doc! { "mission_id": mission_id };
        if let Some(run) = run_id {
            filter.insert("run_id", run);
        }
        if let Some(after) = after_event_id {
            filter.insert("event_id", doc! { "$gt": after as i64 });
        }

        let opts = mongodb::options::FindOptions::builder()
            .sort(doc! { "event_id": 1, "created_at": 1 })
            .limit(clamped_limit as i64)
            .build();
        let mut events: Vec<MissionEventDoc> = self
            .mission_events()
            .find(filter.clone(), opts.clone())
            .await?
            .try_collect()
            .await?;

        // Handle run switches gracefully:
        // if caller sends a stale `after_event_id` from a previous run,
        // restart from beginning of current run instead of returning empty forever.
        if events.is_empty() {
            if let (Some(run), Some(after)) = (run_id, after_event_id) {
                let max_opts = mongodb::options::FindOneOptions::builder()
                    .sort(doc! { "event_id": -1, "created_at": -1 })
                    .build();
                let max_doc = self
                    .mission_events()
                    .find_one(doc! { "mission_id": mission_id, "run_id": run }, max_opts)
                    .await?;
                if let Some(latest) = max_doc {
                    if latest.event_id < after as i64 {
                        let restart_filter = doc! { "mission_id": mission_id, "run_id": run };
                        events = self
                            .mission_events()
                            .find(restart_filter, opts)
                            .await?
                            .try_collect()
                            .await?;
                    }
                }
            }
        }

        Ok(events)
    }

    // ─── Goal Tree Management (AGE) ─────────────────────

    /// Save initial goal tree for a mission.
    pub async fn save_goal_tree(
        &self,
        mission_id: &str,
        goals: Vec<GoalNode>,
    ) -> Result<(), mongodb::error::Error> {
        let goals_bson = bson::to_bson(&goals)
            .map_err(|e| mongodb::error::Error::custom(format!("BSON error: {}", e)))?;
        self.missions()
            .update_one(
                doc! { "mission_id": mission_id },
                doc! { "$set": {
                    "goal_tree": goals_bson,
                    "updated_at": bson::DateTime::now(),
                }},
                None,
            )
            .await?;
        Ok(())
    }

    /// Update a goal's status using arrayFilters.
    /// Automatically sets completed_at when status is Completed or Abandoned.
    pub async fn update_goal_status(
        &self,
        mission_id: &str,
        goal_id: &str,
        status: &GoalStatus,
    ) -> Result<(), mongodb::error::Error> {
        let status_bson = bson::to_bson(status)
            .map_err(|e| mongodb::error::Error::custom(format!("BSON error: {}", e)))?;
        let now = bson::DateTime::now();
        let opts = mongodb::options::UpdateOptions::builder()
            .array_filters(vec![doc! { "elem.goal_id": goal_id }])
            .build();

        let mut set_doc = doc! {
            "goal_tree.$[elem].status": status_bson,
            "updated_at": now,
        };

        if matches!(status, GoalStatus::Running) {
            set_doc.insert("goal_tree.$[elem].started_at", now);
            set_doc.insert("goal_tree.$[elem].last_activity_at", now);
        }

        // Set completed_at for terminal statuses
        if matches!(
            status,
            GoalStatus::Completed | GoalStatus::Abandoned | GoalStatus::Failed
        ) {
            set_doc.insert("goal_tree.$[elem].completed_at", now);
        }

        self.missions()
            .update_one(
                doc! { "mission_id": mission_id },
                doc! { "$set": set_doc },
                opts,
            )
            .await?;
        Ok(())
    }

    pub async fn set_goal_supervision(
        &self,
        mission_id: &str,
        goal_id: &str,
        last_activity_at: Option<bson::DateTime>,
        last_progress_at: Option<bson::DateTime>,
    ) -> Result<(), mongodb::error::Error> {
        let opts = mongodb::options::UpdateOptions::builder()
            .array_filters(vec![doc! { "elem.goal_id": goal_id }])
            .build();
        let now = bson::DateTime::now();
        let mut set_doc = doc! {
            "updated_at": now,
        };

        match last_activity_at {
            Some(ts) => {
                set_doc.insert("goal_tree.$[elem].last_activity_at", ts);
            }
            None => {
                set_doc.insert("goal_tree.$[elem].last_activity_at", Bson::Null);
            }
        }

        match last_progress_at {
            Some(ts) => {
                set_doc.insert("goal_tree.$[elem].last_progress_at", ts);
            }
            None => {
                set_doc.insert("goal_tree.$[elem].last_progress_at", Bson::Null);
            }
        }

        self.missions()
            .update_one(
                doc! { "mission_id": mission_id },
                doc! { "$set": set_doc },
                opts,
            )
            .await?;
        Ok(())
    }

    /// Reset a goal to pending so adaptive resume can retry it.
    pub async fn reset_goal_for_retry(
        &self,
        mission_id: &str,
        goal_id: &str,
    ) -> Result<(), mongodb::error::Error> {
        let opts = mongodb::options::UpdateOptions::builder()
            .array_filters(vec![doc! { "elem.goal_id": goal_id }])
            .build();
        self.missions()
            .update_one(
                doc! { "mission_id": mission_id },
                doc! {
                    "$set": {
                        "goal_tree.$[elem].status": "pending",
                        "updated_at": bson::DateTime::now(),
                    },
                    "$unset": {
                        "goal_tree.$[elem].started_at": "",
                        "goal_tree.$[elem].last_activity_at": "",
                        "goal_tree.$[elem].last_progress_at": "",
                        "goal_tree.$[elem].completed_at": "",
                    },
                },
                opts,
            )
            .await?;
        Ok(())
    }

    /// Append an AttemptRecord to a goal's attempts array.
    pub async fn push_goal_attempt(
        &self,
        mission_id: &str,
        goal_id: &str,
        attempt: &AttemptRecord,
    ) -> Result<(), mongodb::error::Error> {
        let attempt_bson = bson::to_bson(attempt)
            .map_err(|e| mongodb::error::Error::custom(format!("BSON error: {}", e)))?;
        let opts = mongodb::options::UpdateOptions::builder()
            .array_filters(vec![doc! { "elem.goal_id": goal_id }])
            .build();
        self.missions()
            .update_one(
                doc! { "mission_id": mission_id },
                doc! {
                    "$push": { "goal_tree.$[elem].attempts": attempt_bson },
                    "$set": {
                        "updated_at": bson::DateTime::now(),
                        "goal_tree.$[elem].last_activity_at": bson::DateTime::now(),
                        "goal_tree.$[elem].last_progress_at": bson::DateTime::now(),
                    },
                },
                opts,
            )
            .await?;
        Ok(())
    }

    /// Update the signal of the last attempt for a goal.
    pub async fn update_last_attempt_signal(
        &self,
        mission_id: &str,
        goal_id: &str,
        signal: &ProgressSignal,
    ) -> Result<(), mongodb::error::Error> {
        // Read current goal to find the last attempt index
        let mission = self.get_mission(mission_id).await?;
        if let Some(mission) = mission {
            if let Some(goals) = &mission.goal_tree {
                if let Some(goal) = goals.iter().find(|g| g.goal_id == goal_id) {
                    if !goal.attempts.is_empty() {
                        let last_idx = goal.attempts.len() - 1;
                        let signal_bson = bson::to_bson(signal).map_err(|e| {
                            mongodb::error::Error::custom(format!("BSON error: {}", e))
                        })?;
                        let field = format!("goal_tree.$[elem].attempts.{}.signal", last_idx);
                        let opts = mongodb::options::UpdateOptions::builder()
                            .array_filters(vec![doc! { "elem.goal_id": goal_id }])
                            .build();
                        self.missions()
                            .update_one(
                                doc! { "mission_id": mission_id },
                                doc! { "$set": { &field: signal_bson, "updated_at": bson::DateTime::now() } },
                                opts,
                            )
                            .await?;
                    }
                }
            }
        }
        Ok(())
    }

    /// Set a goal's output_summary.
    pub async fn set_goal_output_summary(
        &self,
        mission_id: &str,
        goal_id: &str,
        summary: &str,
    ) -> Result<(), mongodb::error::Error> {
        let opts = mongodb::options::UpdateOptions::builder()
            .array_filters(vec![doc! { "elem.goal_id": goal_id }])
            .build();
        self.missions()
            .update_one(
                doc! { "mission_id": mission_id },
                doc! { "$set": {
                    "goal_tree.$[elem].output_summary": summary,
                    "goal_tree.$[elem].last_activity_at": bson::DateTime::now(),
                    "goal_tree.$[elem].last_progress_at": bson::DateTime::now(),
                    "updated_at": bson::DateTime::now(),
                }},
                opts,
            )
            .await?;
        Ok(())
    }

    /// Persist goal runtime contract extracted from mission_preflight.
    pub async fn set_goal_runtime_contract(
        &self,
        mission_id: &str,
        goal_id: &str,
        contract: &RuntimeContract,
    ) -> Result<(), mongodb::error::Error> {
        let contract_bson = bson::to_bson(contract)
            .map_err(|e| mongodb::error::Error::custom(format!("BSON error: {}", e)))?;
        let opts = mongodb::options::UpdateOptions::builder()
            .array_filters(vec![doc! { "elem.goal_id": goal_id }])
            .build();
        self.missions()
            .update_one(
                doc! { "mission_id": mission_id },
                doc! { "$set": {
                    "goal_tree.$[elem].runtime_contract": contract_bson,
                    "goal_tree.$[elem].last_activity_at": bson::DateTime::now(),
                    "goal_tree.$[elem].last_progress_at": bson::DateTime::now(),
                    "updated_at": bson::DateTime::now(),
                }},
                opts,
            )
            .await?;
        Ok(())
    }

    /// Persist goal contract verification result.
    pub async fn set_goal_contract_verification(
        &self,
        mission_id: &str,
        goal_id: &str,
        verification: &RuntimeContractVerification,
    ) -> Result<(), mongodb::error::Error> {
        let verify_bson = bson::to_bson(verification)
            .map_err(|e| mongodb::error::Error::custom(format!("BSON error: {}", e)))?;
        let opts = mongodb::options::UpdateOptions::builder()
            .array_filters(vec![doc! { "elem.goal_id": goal_id }])
            .build();
        self.missions()
            .update_one(
                doc! { "mission_id": mission_id },
                doc! { "$set": {
                    "goal_tree.$[elem].contract_verification": verify_bson,
                    "goal_tree.$[elem].last_activity_at": bson::DateTime::now(),
                    "goal_tree.$[elem].last_progress_at": bson::DateTime::now(),
                    "updated_at": bson::DateTime::now(),
                }},
                opts,
            )
            .await?;
        Ok(())
    }

    /// Set a goal's pivot_reason.
    pub async fn set_goal_pivot(
        &self,
        mission_id: &str,
        goal_id: &str,
        reason: &str,
    ) -> Result<(), mongodb::error::Error> {
        let status_bson = bson::to_bson(&GoalStatus::Pivoting)
            .map_err(|e| mongodb::error::Error::custom(format!("BSON error: {}", e)))?;
        let opts = mongodb::options::UpdateOptions::builder()
            .array_filters(vec![doc! { "elem.goal_id": goal_id }])
            .build();
        self.missions()
            .update_one(
                doc! { "mission_id": mission_id },
                doc! { "$set": {
                    "goal_tree.$[elem].pivot_reason": reason,
                    "goal_tree.$[elem].status": status_bson,
                    "updated_at": bson::DateTime::now(),
                }},
                opts,
            )
            .await?;
        Ok(())
    }

    /// Update current_goal_id on the mission.
    pub async fn advance_mission_goal(
        &self,
        mission_id: &str,
        goal_id: &str,
    ) -> Result<(), mongodb::error::Error> {
        self.missions()
            .update_one(
                doc! { "mission_id": mission_id },
                doc! { "$set": {
                    "current_goal_id": goal_id,
                    "updated_at": bson::DateTime::now(),
                }},
                None,
            )
            .await?;
        Ok(())
    }

    /// Atomically set goal pivot status + increment total_pivots counter.
    pub async fn pivot_goal_atomic(
        &self,
        mission_id: &str,
        goal_id: &str,
        reason: &str,
    ) -> Result<(), mongodb::error::Error> {
        let status_bson = bson::to_bson(&GoalStatus::Pivoting)
            .map_err(|e| mongodb::error::Error::custom(format!("BSON error: {}", e)))?;
        let now = bson::DateTime::now();
        let opts = mongodb::options::UpdateOptions::builder()
            .array_filters(vec![doc! { "elem.goal_id": goal_id }])
            .build();
        self.missions()
            .update_one(
                doc! { "mission_id": mission_id },
                doc! {
                    "$set": {
                        "goal_tree.$[elem].pivot_reason": reason,
                        "goal_tree.$[elem].status": status_bson,
                        "updated_at": now,
                    },
                    "$inc": { "total_pivots": 1 },
                },
                opts,
            )
            .await?;
        Ok(())
    }

    /// Atomically abandon goal + increment total_abandoned counter.
    pub async fn abandon_goal_atomic(
        &self,
        mission_id: &str,
        goal_id: &str,
        reason: &str,
    ) -> Result<(), mongodb::error::Error> {
        let status_bson = bson::to_bson(&GoalStatus::Abandoned)
            .map_err(|e| mongodb::error::Error::custom(format!("BSON error: {}", e)))?;
        let now = bson::DateTime::now();
        let opts = mongodb::options::UpdateOptions::builder()
            .array_filters(vec![doc! { "elem.goal_id": goal_id }])
            .build();
        self.missions()
            .update_one(
                doc! { "mission_id": mission_id },
                doc! {
                    "$set": {
                        "goal_tree.$[elem].status": status_bson,
                        "goal_tree.$[elem].pivot_reason": reason,
                        "goal_tree.$[elem].completed_at": now,
                        "updated_at": now,
                    },
                    "$inc": { "total_abandoned": 1 },
                },
                opts,
            )
            .await?;
        Ok(())
    }

    /// Insert child goals into the goal_tree array.
    pub async fn insert_child_goals(
        &self,
        mission_id: &str,
        new_goals: Vec<GoalNode>,
    ) -> Result<(), mongodb::error::Error> {
        let goals_bson = bson::to_bson(&new_goals)
            .map_err(|e| mongodb::error::Error::custom(format!("BSON error: {}", e)))?;
        self.missions()
            .update_one(
                doc! { "mission_id": mission_id },
                doc! {
                    "$push": { "goal_tree": { "$each": goals_bson } },
                    "$set": { "updated_at": bson::DateTime::now() },
                },
                None,
            )
            .await?;
        Ok(())
    }

    /// Mark a goal as Abandoned with reason and timestamp.
    pub async fn abandon_goal(
        &self,
        mission_id: &str,
        goal_id: &str,
        reason: &str,
    ) -> Result<(), mongodb::error::Error> {
        let status_bson = bson::to_bson(&GoalStatus::Abandoned)
            .map_err(|e| mongodb::error::Error::custom(format!("BSON error: {}", e)))?;
        let now = bson::DateTime::now();
        let opts = mongodb::options::UpdateOptions::builder()
            .array_filters(vec![doc! { "elem.goal_id": goal_id }])
            .build();
        self.missions()
            .update_one(
                doc! { "mission_id": mission_id },
                doc! { "$set": {
                    "goal_tree.$[elem].status": status_bson,
                    "goal_tree.$[elem].pivot_reason": reason,
                    "goal_tree.$[elem].completed_at": now,
                    "updated_at": now,
                }},
                opts,
            )
            .await?;
        Ok(())
    }

    /// Claim orphaned Running/Planning missions on server startup so the current
    /// instance can auto-resume them instead of requiring manual recovery.
    pub async fn claim_orphaned_missions_for_resume(
        &self,
        instance_id: &str,
    ) -> Result<Vec<String>, mongodb::error::Error> {
        let now = bson::DateTime::now();
        let filter = orphaned_mission_recovery_filter(instance_id);
        let mission_ids = self
            .missions()
            .distinct("mission_id", filter, None)
            .await?
            .into_iter()
            .filter_map(|value| value.as_str().map(str::to_string))
            .collect::<Vec<_>>();

        if mission_ids.is_empty() {
            return Ok(Vec::new());
        }

        let mission_id_filter = mission_ids
            .iter()
            .cloned()
            .map(bson::Bson::String)
            .collect::<Vec<_>>();

        self.missions()
            .update_many(
                doc! {
                    "mission_id": { "$in": mission_id_filter },
                    "status": { "$in": ["running", "planning"] },
                },
                doc! { "$set": {
                    "status": "failed",
                    "updated_at": now,
                    "completed_at": now,
                    "error_message": "Server restarted while mission was in progress",
                }},
                None,
            )
            .await?;
        Ok(mission_ids)
    }

    /// Claim missions that still look active in Mongo but are no longer being
    /// tracked by the in-process MissionManager. This covers executor exits
    /// where the runtime loop stops without transitioning the persisted mission
    /// out of running/planning.
    pub async fn claim_inactive_running_missions_for_resume(
        &self,
        max_age: std::time::Duration,
        active_mission_ids: &[String],
    ) -> Result<Vec<String>, mongodb::error::Error> {
        let stale_before = Utc::now()
            - chrono::Duration::from_std(max_age).unwrap_or_else(|_| chrono::Duration::minutes(10));
        let filter = active_running_mission_scan_filter(active_mission_ids);
        let candidates = self
            .missions()
            .find(filter, None)
            .await?
            .try_collect::<Vec<MissionDoc>>()
            .await?;

        if candidates.is_empty() {
            return Ok(Vec::new());
        }

        let now = bson::DateTime::now();
        let mut claimed = Vec::new();
        for mission in candidates {
            if !mission_inactive_for_resume(&mission, stale_before) {
                continue;
            }
            let result = self
                .missions()
                .update_one(
                    doc! {
                        "mission_id": &mission.mission_id,
                        "status": { "$in": ["running", "planning"] },
                        "updated_at": mission.updated_at,
                    },
                    doc! { "$set": {
                        "status": "failed",
                        "updated_at": now,
                        "completed_at": now,
                        "error_message": "Mission supervisor detected no activity and scheduled auto-resume",
                    }},
                    None,
                )
                .await?;
            if result.modified_count > 0 {
                claimed.push(mission.mission_id);
            }
        }

        Ok(claimed)
    }

    // ─── Mission Indexes ─────────────────────────────────

    pub async fn ensure_mission_indexes(&self) {
        use mongodb::options::IndexOptions;
        use mongodb::IndexModel;

        let mission_indexes = vec![
            IndexModel::builder()
                .keys(doc! { "team_id": 1, "status": 1, "created_at": -1 })
                .build(),
            IndexModel::builder()
                .keys(doc! { "team_id": 1, "agent_id": 1, "status": 1 })
                .build(),
            IndexModel::builder()
                .keys(doc! { "creator_id": 1, "status": 1 })
                .build(),
            IndexModel::builder()
                .keys(doc! { "mission_id": 1 })
                .options(IndexOptions::builder().unique(true).build())
                .build(),
        ];

        if let Err(e) = self.missions().create_indexes(mission_indexes, None).await {
            tracing::warn!("Failed to create mission indexes: {}", e);
        }

        let artifact_indexes = vec![
            IndexModel::builder()
                .keys(doc! { "mission_id": 1, "step_index": 1 })
                .build(),
            IndexModel::builder()
                .keys(doc! { "artifact_id": 1 })
                .options(IndexOptions::builder().unique(true).build())
                .build(),
        ];

        if let Err(e) = self
            .artifacts()
            .create_indexes(artifact_indexes, None)
            .await
        {
            tracing::warn!("Failed to create artifact indexes: {}", e);
        }

        let event_indexes = vec![
            IndexModel::builder()
                .keys(doc! { "mission_id": 1, "event_id": 1 })
                .build(),
            IndexModel::builder()
                .keys(doc! { "mission_id": 1, "run_id": 1, "event_id": 1 })
                .options(
                    IndexOptions::builder()
                        .unique(true)
                        .partial_filter_expression(doc! { "run_id": { "$exists": true } })
                        .build(),
                )
                .build(),
            IndexModel::builder()
                .keys(doc! { "mission_id": 1, "created_at": -1 })
                .build(),
            IndexModel::builder()
                .keys(doc! { "mission_id": 1, "run_id": 1, "created_at": 1 })
                .build(),
        ];

        if let Err(e) = self
            .mission_events()
            .create_indexes(event_indexes, None)
            .await
        {
            tracing::warn!("Failed to create mission event indexes: {}", e);
        }
    }
}

fn orphaned_mission_recovery_filter(instance_id: &str) -> mongodb::bson::Document {
    let running_states = vec![
        bson::Bson::String("running".to_string()),
        bson::Bson::String("planning".to_string()),
    ];

    let ownership_guard = if instance_id.trim().is_empty() {
        vec![
            doc! { "server_instance_id": { "$exists": false } },
            doc! { "server_instance_id": bson::Bson::Null },
        ]
    } else {
        vec![
            doc! { "server_instance_id": { "$exists": false } },
            doc! { "server_instance_id": bson::Bson::Null },
            doc! { "server_instance_id": { "$ne": instance_id } },
        ]
    };

    doc! {
        "status": { "$in": running_states },
        "$or": ownership_guard,
    }
}

fn active_running_mission_scan_filter(active_mission_ids: &[String]) -> mongodb::bson::Document {
    let running_states = vec![
        bson::Bson::String("running".to_string()),
        bson::Bson::String("planning".to_string()),
    ];

    let mut filter = doc! {
        "status": { "$in": running_states },
    };

    if !active_mission_ids.is_empty() {
        let excluded = active_mission_ids
            .iter()
            .cloned()
            .map(bson::Bson::String)
            .collect::<Vec<_>>();
        filter.insert("mission_id", doc! { "$nin": excluded });
    }

    filter
}

fn mission_inactive_for_resume(mission: &MissionDoc, stale_before: DateTime<Utc>) -> bool {
    if !matches!(
        mission.status,
        MissionStatus::Running | MissionStatus::Planning
    ) {
        return false;
    }

    let last_observed_at =
        current_step_last_observed_at(mission)
            .or_else(|| current_goal_last_observed_at(mission))
            .unwrap_or_else(|| mission.updated_at.to_chrono());
    last_observed_at < stale_before
}

fn current_step_last_observed_at(mission: &MissionDoc) -> Option<DateTime<Utc>> {
    let step_index = mission.current_step? as usize;
    let step = mission.steps.get(step_index)?;
    [
        step.last_progress_at.as_ref().map(|dt| dt.to_chrono()),
        step.last_activity_at.as_ref().map(|dt| dt.to_chrono()),
        step.started_at.as_ref().map(|dt| dt.to_chrono()),
    ]
    .into_iter()
    .flatten()
    .max()
}

fn current_goal_last_observed_at(mission: &MissionDoc) -> Option<DateTime<Utc>> {
    let goals = mission.goal_tree.as_ref()?;
    let goal = mission
        .current_goal_id
        .as_deref()
        .and_then(|goal_id| goals.iter().find(|g| g.goal_id == goal_id))
        .or_else(|| goals.iter().find(|g| matches!(g.status, GoalStatus::Running)))?;

    [
        goal.last_progress_at.as_ref().map(|dt| dt.to_chrono()),
        goal.last_activity_at.as_ref().map(|dt| dt.to_chrono()),
        goal.started_at.as_ref().map(|dt| dt.to_chrono()),
        goal.created_at.as_ref().map(|dt| dt.to_chrono()),
    ]
    .into_iter()
    .flatten()
    .max()
}
