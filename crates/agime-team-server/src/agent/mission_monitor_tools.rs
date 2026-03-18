use std::sync::Arc;

use agime::agents::mcp_client::McpClientTrait;
use agime_team::db::MongoDb;
use anyhow::{anyhow, Result};
use rmcp::model::*;
use rmcp::ServiceError;
use serde_json::json;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use super::mission_manager::MissionManager;
use super::mission_mongo::{MissionStrategyPatch, MonitorActionRequest};
use super::mission_monitor::{
    build_monitor_feedback, build_monitor_snapshot, execute_monitor_action,
    normalize_monitor_action,
};
use super::runtime::reconcile_workspace_artifacts;
use super::service_mongo::AgentService;

pub struct MissionMonitorToolsProvider {
    db: Arc<MongoDb>,
    mission_manager: Arc<MissionManager>,
    team_id: String,
    actor_user_id: Option<String>,
    current_mission_id: Option<String>,
    workspace_root: String,
    info: InitializeResult,
}

impl MissionMonitorToolsProvider {
    pub fn new(
        db: Arc<MongoDb>,
        mission_manager: Arc<MissionManager>,
        team_id: String,
        actor_user_id: Option<String>,
        current_mission_id: Option<String>,
        workspace_root: String,
    ) -> Self {
        let info = InitializeResult {
            protocol_version: ProtocolVersion::V_2025_03_26,
            capabilities: ServerCapabilities {
                tools: Some(ToolsCapability {
                    list_changed: Some(false),
                }),
                resources: None,
                prompts: None,
                completions: None,
                experimental: None,
                logging: None,
                extensions: None,
                tasks: None,
            },
            server_info: Implementation {
                name: "mission_monitor".to_string(),
                title: Some("Mission Monitor".to_string()),
                version: "1.0.0".to_string(),
                description: None,
                icons: None,
                website_url: None,
            },
            instructions: Some(
                "Use mission monitor tools to inspect structured mission runtime state and request controlled recovery actions. Prefer evidence-driven suggestions over rigid completion assumptions."
                    .to_string(),
            ),
        };
        Self {
            db,
            mission_manager,
            team_id,
            actor_user_id,
            current_mission_id,
            workspace_root,
            info,
        }
    }

    fn tool_definitions() -> Vec<Tool> {
        vec![
            Tool {
                name: "get_runtime_snapshot".into(),
                title: None,
                description: Some(
                    "Read the current mission's structured runtime snapshot, including active goal/step, recent progress, evidence bundle, and runtime status."
                        .into(),
                ),
                input_schema: serde_json::from_value(json!({
                    "type": "object",
                    "properties": {
                        "mission_id": {
                            "type": "string",
                            "description": "Optional mission id. Defaults to current mission context."
                        }
                    }
                }))
                .unwrap_or_default(),
                output_schema: None,
                annotations: None,
                execution: None,
                icons: None,
                meta: None,
            },
            Tool {
                name: "request_action".into(),
                title: None,
                description: Some(
                    "Request a controlled monitor action such as continue, resume, split, replan, or complete-if-sufficient."
                        .into(),
                ),
                input_schema: serde_json::from_value(json!({
                    "type": "object",
                    "properties": {
                        "mission_id": {
                            "type": "string",
                            "description": "Optional mission id. Defaults to current mission context."
                        },
                        "action": {
                            "type": "string",
                            "description": "One of continue_current, repair_deliverables, repair_contract, continue_with_replan, mark_waiting_external, partial_handoff, complete_if_sufficient, blocked_by_environment, blocked_by_tooling"
                        },
                        "feedback": {
                            "type": "string",
                            "description": "Optional diagnosis or intervention note."
                        },
                        "semantic_tags": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "Optional open semantic labels from the monitor agent."
                        },
                        "observed_evidence": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "Optional evidence notes supporting the action request."
                        },
                        "missing_core_deliverables": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "Optional list of still-missing core deliverables."
                        },
                        "confidence": {
                            "type": "number",
                            "description": "Optional monitor confidence score in the selected action."
                        },
                        "strategy_patch": {
                            "type": "object",
                            "description": "Optional strategy rewrite payload explaining how the mission approach changed."
                        },
                        "subagent_recommended": {
                            "type": "boolean",
                            "description": "Whether bounded worker-side delegation is recommended."
                        },
                        "parallelism_budget": {
                            "type": "integer",
                            "description": "Optional worker-side subtask budget for the next decision round."
                        }
                    },
                    "required": ["action"]
                }))
                .unwrap_or_default(),
                output_schema: None,
                annotations: None,
                execution: None,
                icons: None,
                meta: None,
            },
        ]
    }

    fn resolve_target_mission_id(&self, args: &JsonObject) -> Result<String> {
        if let Some(mission_id) = args
            .get("mission_id")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            return Ok(mission_id.to_string());
        }

        self.current_mission_id
            .clone()
            .ok_or_else(|| anyhow!("mission_id is required outside mission context"))
    }

    async fn load_target_mission(
        &self,
        mission_id: &str,
    ) -> Result<(AgentService, super::mission_mongo::MissionDoc)> {
        let service = AgentService::new(self.db.clone());
        let mission = service
            .get_mission(mission_id)
            .await?
            .ok_or_else(|| anyhow!("Mission not found: {}", mission_id))?;

        if mission.team_id != self.team_id {
            return Err(anyhow!(
                "Mission {} does not belong to current team context",
                mission_id
            ));
        }

        let same_mission = self.current_mission_id.as_deref() == Some(mission_id);
        if !same_mission {
            let actor_user_id = self.actor_user_id.as_deref().ok_or_else(|| {
                anyhow!("cross-mission monitor access requires actor user context")
            })?;
            let is_admin = service.is_team_admin(actor_user_id, &self.team_id).await?;
            if !is_admin {
                return Err(anyhow!(
                    "cross-mission monitor access requires team admin privileges"
                ));
            }
        }

        Ok((service, mission))
    }

    async fn handle_get_runtime_snapshot(&self, args: &JsonObject) -> Result<String> {
        let mission_id = self.resolve_target_mission_id(args)?;
        let (service, mission) = self.load_target_mission(&mission_id).await?;
        if let Some(workspace_path) = mission.workspace_path.as_deref() {
            if let Err(err) =
                reconcile_workspace_artifacts(
                    &service,
                    &mission_id,
                    mission.current_step.unwrap_or(0),
                    workspace_path,
                )
                .await
            {
                tracing::warn!(
                    "Failed to reconcile workspace artifacts before monitor snapshot for mission {}: {}",
                    mission_id,
                    err
                );
            }
        }
        let artifacts = service
            .list_mission_artifacts(&mission_id)
            .await
            .map_err(|e| anyhow!("Failed to load mission artifacts: {}", e))?;
        let snapshot = build_monitor_snapshot(
            &mission,
            &artifacts,
            self.mission_manager.is_active(&mission_id).await,
        );
        Ok(serde_json::to_string_pretty(&snapshot)?)
    }

    async fn handle_request_action(&self, args: &JsonObject) -> Result<String> {
        let mission_id = self.resolve_target_mission_id(args)?;
        let action = args
            .get("action")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| anyhow!("action is required"))?;
        let action = normalize_monitor_action(action)
            .ok_or_else(|| anyhow!("Unsupported monitor action"))?;
        let feedback = args
            .get("feedback")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());
        let semantic_tags = args
            .get("semantic_tags")
            .and_then(|v| v.as_array())
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| item.as_str().map(str::trim))
                    .filter(|s| !s.is_empty())
                    .map(|s| s.to_string())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let observed_evidence = args
            .get("observed_evidence")
            .and_then(|v| v.as_array())
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| item.as_str().map(str::trim))
                    .filter(|s| !s.is_empty())
                    .map(|s| s.to_string())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let missing_core_deliverables = args
            .get("missing_core_deliverables")
            .and_then(|v| v.as_array())
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| item.as_str().map(str::trim))
                    .filter(|s| !s.is_empty())
                    .map(|s| s.to_string())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let confidence = args.get("confidence").and_then(|v| v.as_f64());
        let strategy_patch = args
            .get("strategy_patch")
            .cloned()
            .map(serde_json::from_value::<MissionStrategyPatch>)
            .transpose()
            .map_err(|e| anyhow!("Invalid strategy_patch: {}", e))?;
        let subagent_recommended = args
            .get("subagent_recommended")
            .and_then(|v| v.as_bool());
        let parallelism_budget = args
            .get("parallelism_budget")
            .and_then(|v| v.as_u64())
            .and_then(|value| u32::try_from(value).ok());
        let body = MonitorActionRequest {
            action: action.clone(),
            feedback,
            semantic_tags,
            observed_evidence,
            missing_core_deliverables,
            confidence,
            strategy_patch,
            subagent_recommended,
            parallelism_budget,
        };
        let feedback = build_monitor_feedback(&action, &body);
        let (service, mission) = self.load_target_mission(&mission_id).await?;
        let outcome = execute_monitor_action(
            &service,
            &self.db,
            &self.mission_manager,
            &self.workspace_root,
            &mission,
            action,
            feedback,
            body.observed_evidence.clone(),
            body.semantic_tags.clone(),
            body.missing_core_deliverables.clone(),
            body.confidence,
            body.strategy_patch.clone(),
            body.subagent_recommended,
            body.parallelism_budget,
        )
        .await?;

        Ok(serde_json::to_string_pretty(&json!({
            "mission_id": mission_id,
            "status": outcome.status,
            "action": outcome.action,
            "applied": outcome.applied,
        }))?)
    }
}

#[async_trait::async_trait]
impl McpClientTrait for MissionMonitorToolsProvider {
    async fn list_resources(
        &self,
        _next_cursor: Option<String>,
        _cancel_token: CancellationToken,
    ) -> std::result::Result<ListResourcesResult, ServiceError> {
        Ok(ListResourcesResult {
            resources: vec![],
            next_cursor: None,
            meta: None,
        })
    }

    async fn read_resource(
        &self,
        _uri: &str,
        _cancel_token: CancellationToken,
    ) -> std::result::Result<ReadResourceResult, ServiceError> {
        Err(ServiceError::TransportClosed)
    }

    async fn list_tools(
        &self,
        _next_cursor: Option<String>,
        _cancel_token: CancellationToken,
    ) -> std::result::Result<ListToolsResult, ServiceError> {
        Ok(ListToolsResult {
            tools: Self::tool_definitions(),
            next_cursor: None,
            meta: None,
        })
    }

    async fn call_tool(
        &self,
        name: &str,
        arguments: Option<JsonObject>,
        _cancel_token: CancellationToken,
    ) -> std::result::Result<CallToolResult, ServiceError> {
        let args = arguments.unwrap_or_default();
        let result = match name {
            "get_runtime_snapshot" => self.handle_get_runtime_snapshot(&args).await,
            "request_action" => self.handle_request_action(&args).await,
            _ => Err(anyhow!("Unknown tool: {}", name)),
        };

        match result {
            Ok(text) => Ok(CallToolResult::success(vec![Content::text(text)])),
            Err(e) => Ok(CallToolResult::error(vec![Content::text(e.to_string())])),
        }
    }

    async fn list_tasks(
        &self,
        _cursor: Option<String>,
        _cancel_token: CancellationToken,
    ) -> std::result::Result<ListTasksResult, ServiceError> {
        Err(ServiceError::TransportClosed)
    }

    async fn get_task_info(
        &self,
        _task_id: &str,
        _cancel_token: CancellationToken,
    ) -> std::result::Result<GetTaskInfoResult, ServiceError> {
        Err(ServiceError::TransportClosed)
    }

    async fn get_task_result(
        &self,
        _task_id: &str,
        _cancel_token: CancellationToken,
    ) -> std::result::Result<TaskResult, ServiceError> {
        Err(ServiceError::TransportClosed)
    }

    async fn cancel_task(
        &self,
        _task_id: &str,
        _cancel_token: CancellationToken,
    ) -> std::result::Result<(), ServiceError> {
        Err(ServiceError::TransportClosed)
    }

    async fn list_prompts(
        &self,
        _next_cursor: Option<String>,
        _cancel_token: CancellationToken,
    ) -> std::result::Result<ListPromptsResult, ServiceError> {
        Ok(ListPromptsResult {
            prompts: vec![],
            next_cursor: None,
            meta: None,
        })
    }

    async fn get_prompt(
        &self,
        _name: &str,
        _arguments: serde_json::Value,
        _cancel_token: CancellationToken,
    ) -> std::result::Result<GetPromptResult, ServiceError> {
        Err(ServiceError::TransportClosed)
    }

    async fn subscribe(&self) -> mpsc::Receiver<ServerNotification> {
        let (_tx, rx) = mpsc::channel(1);
        rx
    }

    fn get_info(&self) -> Option<&InitializeResult> {
        Some(&self.info)
    }
}
