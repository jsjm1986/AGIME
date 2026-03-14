//! Avatar governance tools provider.
//!
//! Bridges digital-avatar governance state into structured tools for:
//! - service agent runtime escalation
//! - manager agent governance review

use agime::agents::mcp_client::McpClientTrait;
use agime_team::db::MongoDb;
use agime_team::models::mongo::Portal;
use agime_team::services::mongo::PortalService;
use anyhow::{anyhow, Result};
use chrono::Utc;
use rmcp::model::*;
use rmcp::ServiceError;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value as JsonValue};
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use super::service_mongo::{
    AgentService, AvatarGovernanceQueueItemPayload, AvatarGovernanceStatePayload,
    AvatarWorkbenchSnapshotPayload,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AvatarGovernanceRole {
    Service,
    Manager,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct GovernanceState {
    #[serde(default)]
    capability_requests: Vec<CapabilityGapRequest>,
    #[serde(default)]
    gap_proposals: Vec<GapProposal>,
    #[serde(default)]
    optimization_tickets: Vec<OptimizationTicket>,
    #[serde(default)]
    runtime_logs: Vec<RuntimeLogEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CapabilityGapRequest {
    id: String,
    title: String,
    detail: String,
    #[serde(default)]
    requested_scope: Vec<String>,
    risk: String,
    status: String,
    source: String,
    created_at: String,
    updated_at: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    decision: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    decision_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GapProposal {
    id: String,
    title: String,
    description: String,
    expected_gain: String,
    status: String,
    proposed_by: String,
    created_at: String,
    updated_at: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    decision_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct OptimizationTicket {
    id: String,
    title: String,
    problem_type: String,
    evidence: String,
    proposal: String,
    expected_gain: String,
    risk: String,
    status: String,
    created_at: String,
    updated_at: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    decision_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RuntimeLogEntry {
    id: String,
    title: String,
    evidence: String,
    proposal: String,
    expected_gain: String,
    risk: String,
    problem_type: String,
    status: String,
    created_at: String,
}

pub struct AvatarGovernanceToolsProvider {
    db: Arc<MongoDb>,
    team_id: String,
    role: AvatarGovernanceRole,
    session_source: Option<String>,
    session_id: Option<String>,
    actor_id: String,
    actor_name: Option<String>,
    info: InitializeResult,
}

impl AvatarGovernanceToolsProvider {
    pub fn new(
        db: Arc<MongoDb>,
        team_id: String,
        role: AvatarGovernanceRole,
        agent_id: Option<String>,
        session_source: Option<String>,
        session_id: Option<String>,
    ) -> Self {
        let (actor_id, actor_name, title, instructions) = match role {
            AvatarGovernanceRole::Service => (
                agent_id.unwrap_or_else(|| "service-agent".to_string()),
                Some("分身 Agent".to_string()),
                "Avatar Governance (Service)".to_string(),
                "Use get_runtime_boundary before escalation. When current capability or authority is insufficient, submit_capability_request / submit_gap_proposal / submit_human_review_request instead of guessing or overreaching. For recurring prompt/tool/skill/policy issues, submit_optimization_ticket. When the user asks about progress or a previous escalation result, call get_latest_governance_update first so you can explain the latest manager/human status clearly. If the user retries a request that was previously blocked, pending, or escalated in this conversation, do not rely on the earlier refusal alone: first call get_latest_governance_update and get_runtime_boundary again, then continue with the newly approved tool if the boundary has been widened. Tell the user the request has been handed to 管理 Agent, and if not approved may move to human review.".to_string(),
            ),
            AvatarGovernanceRole::Manager => (
                agent_id.unwrap_or_else(|| "manager-agent".to_string()),
                Some("管理 Agent".to_string()),
                "Avatar Governance (Manager)".to_string(),
                "Use list_queue and get_workbench_snapshot to inspect pending avatar governance work. Review items with review_request before changing service-agent runtime policy. After approval, use portal_tools to apply the actual service-agent/runtime changes. Keep decisions structured and auditable.".to_string(),
            ),
        };

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
                name: "avatar_governance".to_string(),
                title: Some(title),
                version: "1.0.0".to_string(),
                description: None,
                icons: None,
                website_url: None,
            },
            instructions: Some(instructions),
        };

        Self {
            db,
            team_id,
            role,
            session_source,
            session_id,
            actor_id,
            actor_name,
            info,
        }
    }

    fn agent_service(&self) -> AgentService {
        AgentService::new(self.db.clone())
    }

    fn portal_service(&self) -> PortalService {
        PortalService::new((*self.db).clone())
    }

    fn now_rfc3339() -> String {
        Utc::now().to_rfc3339()
    }

    fn get_str_arg<'a>(args: &'a JsonObject, keys: &[&str]) -> Option<&'a str> {
        keys.iter()
            .find_map(|key| args.get(*key).and_then(JsonValue::as_str))
            .map(str::trim)
            .filter(|value| !value.is_empty())
    }

    fn get_string_array_arg(args: &JsonObject, keys: &[&str]) -> Vec<String> {
        keys.iter()
            .find_map(|key| args.get(*key).and_then(JsonValue::as_array))
            .map(|items| {
                items
                    .iter()
                    .filter_map(JsonValue::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(ToOwned::to_owned)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    }

    fn get_u64_arg(args: &JsonObject, keys: &[&str], default: u64, min: u64, max: u64) -> u64 {
        let value = keys
            .iter()
            .find_map(|key| args.get(*key).and_then(JsonValue::as_u64))
            .unwrap_or(default);
        value.clamp(min, max)
    }

    fn tool_definitions(&self) -> Vec<Tool> {
        let mut tools = vec![
            Tool {
                name: "get_runtime_boundary".into(),
                title: None,
                description: Some("Inspect current digital-avatar runtime boundary, available documents/capabilities, and escalation policy.".into()),
                input_schema: serde_json::from_value(json!({
                    "type": "object",
                    "properties": {
                        "portal_id": { "type": "string", "description": "Optional portal id. Omit when current session is already bound to one avatar." },
                        "portal_slug": { "type": "string", "description": "Optional portal slug fallback." }
                    }
                })).unwrap_or_default(),
                output_schema: None,
                annotations: None,
                execution: None,
                icons: None,
                meta: None,
            },
            Tool {
                name: "list_request_status".into(),
                title: None,
                description: Some("List governance requests and their latest status for the current avatar.".into()),
                input_schema: serde_json::from_value(json!({
                    "type": "object",
                    "properties": {
                        "portal_id": { "type": "string", "description": "Optional portal id. Required only when session is not bound to a portal." },
                        "portal_slug": { "type": "string", "description": "Optional portal slug fallback." },
                        "kind": {
                            "type": "string",
                            "enum": ["all", "capability", "proposal", "ticket", "runtime"],
                            "description": "Optional request type filter."
                        },
                        "status": { "type": "string", "description": "Optional status filter." },
                        "limit": { "type": "integer", "description": "Max items (default 20, max 100)." }
                    }
                })).unwrap_or_default(),
                output_schema: None,
                annotations: None,
                execution: None,
                icons: None,
                meta: None,
            },
        ];

        match self.role {
            AvatarGovernanceRole::Service => {
                tools.extend([
                    Tool {
                        name: "get_latest_governance_update".into(),
                        title: None,
                        description: Some("Return the latest governance decision or queue update in a user-facing way so the avatar can explain current progress clearly.".into()),
                        input_schema: serde_json::from_value(json!({
                            "type": "object",
                            "properties": {
                                "portal_id": { "type": "string", "description": "Optional portal id override." },
                                "portal_slug": { "type": "string", "description": "Optional portal slug fallback." },
                                "request_id": { "type": "string", "description": "Optional governance request id for a specific item." },
                                "kind": {
                                    "type": "string",
                                    "enum": ["capability", "proposal", "ticket"],
                                    "description": "Optional request kind filter."
                                }
                            }
                        })).unwrap_or_default(),
                        output_schema: None,
                        annotations: None,
                        execution: None,
                        icons: None,
                        meta: None,
                    },
                    Tool {
                        name: "submit_capability_request".into(),
                        title: None,
                        description: Some("Submit a capability/boundary request when the current avatar cannot safely complete the task.".into()),
                        input_schema: serde_json::from_value(json!({
                            "type": "object",
                            "properties": {
                                "portal_id": { "type": "string", "description": "Optional portal id override." },
                                "title": { "type": "string", "description": "Short request title." },
                                "detail": { "type": "string", "description": "What is missing and why the avatar cannot continue safely." },
                                "requested_scope": {
                                    "type": "array",
                                    "items": { "type": "string" },
                                    "description": "Requested capabilities, documents, or policy changes."
                                },
                                "risk": {
                                    "type": "string",
                                    "enum": ["low", "medium", "high"],
                                    "description": "Risk level for this request."
                                },
                                "source": {
                                    "type": "string",
                                    "enum": ["avatar", "manager", "user"],
                                    "description": "Who originated this request. Defaults to avatar."
                                }
                            },
                            "required": ["title", "detail"]
                        })).unwrap_or_default(),
                        output_schema: None,
                        annotations: None,
                        execution: None,
                        icons: None,
                        meta: None,
                    },
                    Tool {
                        name: "submit_gap_proposal".into(),
                        title: None,
                        description: Some("Submit a structured improvement proposal for recurring demand or capability gaps.".into()),
                        input_schema: serde_json::from_value(json!({
                            "type": "object",
                            "properties": {
                                "portal_id": { "type": "string", "description": "Optional portal id override." },
                                "title": { "type": "string", "description": "Proposal title." },
                                "description": { "type": "string", "description": "Why this gap matters and what should improve." },
                                "expected_gain": { "type": "string", "description": "Expected operational/user benefit." }
                            },
                            "required": ["title", "description"]
                        })).unwrap_or_default(),
                        output_schema: None,
                        annotations: None,
                        execution: None,
                        icons: None,
                        meta: None,
                    },
                    Tool {
                        name: "submit_human_review_request".into(),
                        title: None,
                        description: Some("Escalate a blocked or high-risk case for manager review and possible human approval.".into()),
                        input_schema: serde_json::from_value(json!({
                            "type": "object",
                            "properties": {
                                "portal_id": { "type": "string", "description": "Optional portal id override." },
                                "title": { "type": "string", "description": "Escalation title." },
                                "detail": { "type": "string", "description": "Explain why this needs manager/human review." },
                                "requested_scope": {
                                    "type": "array",
                                    "items": { "type": "string" },
                                    "description": "Optional requested capabilities or actions."
                                },
                                "risk": {
                                    "type": "string",
                                    "enum": ["low", "medium", "high"],
                                    "description": "Risk level. Defaults to high."
                                }
                            },
                            "required": ["title", "detail"]
                        })).unwrap_or_default(),
                        output_schema: None,
                        annotations: None,
                        execution: None,
                        icons: None,
                        meta: None,
                    },
                    Tool {
                        name: "submit_optimization_ticket".into(),
                        title: None,
                        description: Some("Submit a structured optimization ticket when recurring prompt/tool/skill/policy problems should enter the governance improvement loop.".into()),
                        input_schema: serde_json::from_value(json!({
                            "type": "object",
                            "properties": {
                                "portal_id": { "type": "string", "description": "Optional portal id override." },
                                "title": { "type": "string", "description": "Short optimization ticket title." },
                                "problem_type": {
                                    "type": "string",
                                    "enum": ["prompt", "tool", "skill", "policy", "team_prompt"],
                                    "description": "Problem category."
                                },
                                "evidence": { "type": "string", "description": "Observed runtime evidence or repeated failure mode." },
                                "proposal": { "type": "string", "description": "Suggested optimization direction." },
                                "expected_gain": { "type": "string", "description": "Expected quality or efficiency gain." },
                                "risk": {
                                    "type": "string",
                                    "enum": ["low", "medium", "high"],
                                    "description": "Risk level. Defaults to medium."
                                }
                            },
                            "required": ["title", "problem_type", "evidence", "proposal"]
                        })).unwrap_or_default(),
                        output_schema: None,
                        annotations: None,
                        execution: None,
                        icons: None,
                        meta: None,
                    },
                ]);
            }
            AvatarGovernanceRole::Manager => {
                tools.extend([
                    Tool {
                        name: "list_queue".into(),
                        title: None,
                        description: Some("List pending governance queue items for a digital avatar.".into()),
                        input_schema: serde_json::from_value(json!({
                            "type": "object",
                            "properties": {
                                "portal_id": { "type": "string", "description": "Optional portal id. Required only when session is not already bound to a portal." },
                                "portal_slug": { "type": "string", "description": "Optional portal slug fallback." }
                            }
                        })).unwrap_or_default(),
                        output_schema: None,
                        annotations: None,
                        execution: None,
                        icons: None,
                        meta: None,
                    },
                    Tool {
                        name: "get_workbench_snapshot".into(),
                        title: None,
                        description: Some("Load the current governance workbench snapshot for a digital avatar.".into()),
                        input_schema: serde_json::from_value(json!({
                            "type": "object",
                            "properties": {
                                "portal_id": { "type": "string", "description": "Optional portal id. Required only when session is not already bound to a portal." },
                                "portal_slug": { "type": "string", "description": "Optional portal slug fallback." }
                            }
                        })).unwrap_or_default(),
                        output_schema: None,
                        annotations: None,
                        execution: None,
                        icons: None,
                        meta: None,
                    },
                    Tool {
                        name: "review_request".into(),
                        title: None,
                        description: Some("Review a governance request and update its structured status.".into()),
                        input_schema: serde_json::from_value(json!({
                            "type": "object",
                            "properties": {
                                "portal_id": { "type": "string", "description": "Optional portal id. Required only when session is not already bound to a portal." },
                                "portal_slug": { "type": "string", "description": "Optional portal slug fallback." },
                                "kind": {
                                    "type": "string",
                                    "enum": ["capability", "proposal", "ticket"],
                                    "description": "Request kind."
                                },
                                "request_id": { "type": "string", "description": "Target request id." },
                                "decision": { "type": "string", "description": "Decision keyword. capability: approve/approve_sandbox/needs_human/reject; proposal: approve/pilot/active/reject; ticket: approve/experiment/deploy/rollback/reject." },
                                "decision_reason": { "type": "string", "description": "Structured decision note shown in governance history." },
                                "decision_mode": {
                                    "type": "string",
                                    "enum": ["approve_direct", "approve_sandbox", "require_human_confirm", "deny"],
                                    "description": "Optional capability-request decision mode override."
                                }
                            },
                            "required": ["kind", "request_id", "decision"]
                        })).unwrap_or_default(),
                        output_schema: None,
                        annotations: None,
                        execution: None,
                        icons: None,
                        meta: None,
                    },
                ]);
            }
        }

        tools
    }

    async fn resolve_bound_portal(
        &self,
        args: &JsonObject,
    ) -> Result<(Portal, Option<super::session_mongo::AgentSessionDoc>)> {
        let portal_service = self.portal_service();

        if let Some(portal_id) = Self::get_str_arg(args, &["portal_id", "portalId"]) {
            let portal = portal_service.get(&self.team_id, portal_id).await?;
            if !PortalService::is_digital_avatar_portal(&portal) {
                return Err(anyhow!("Portal '{}' is not a digital avatar.", portal_id));
            }
            return Ok((portal, None));
        }

        if let Some(portal_slug) = Self::get_str_arg(args, &["portal_slug", "portalSlug"]) {
            let portal = portal_service.get_by_slug(portal_slug).await?;
            if portal.team_id.to_hex() != self.team_id {
                return Err(anyhow!(
                    "Portal slug '{}' does not belong to current team context.",
                    portal_slug
                ));
            }
            if !PortalService::is_digital_avatar_portal(&portal) {
                return Err(anyhow!("Portal '{}' is not a digital avatar.", portal_slug));
            }
            return Ok((portal, None));
        }

        let session_id = self.session_id.as_deref().ok_or_else(|| {
            anyhow!(
                "portal_id is required because current session is not bound to a digital avatar."
            )
        })?;
        let session = self
            .agent_service()
            .get_session(session_id)
            .await?
            .ok_or_else(|| anyhow!("Session '{}' not found.", session_id))?;

        if let Some(portal_id) = session.portal_id.as_deref() {
            let portal = portal_service.get(&self.team_id, portal_id).await?;
            if !PortalService::is_digital_avatar_portal(&portal) {
                return Err(anyhow!(
                    "Bound portal '{}' is not a digital avatar.",
                    portal_id
                ));
            }
            return Ok((portal, Some(session)));
        }

        if let Some(portal_slug) = session.portal_slug.as_deref() {
            let portal = portal_service.get_by_slug(portal_slug).await?;
            if portal.team_id.to_hex() != self.team_id {
                return Err(anyhow!(
                    "Bound portal slug '{}' does not belong to current team context.",
                    portal_slug
                ));
            }
            if !PortalService::is_digital_avatar_portal(&portal) {
                return Err(anyhow!(
                    "Bound portal '{}' is not a digital avatar.",
                    portal_slug
                ));
            }
            return Ok((portal, Some(session)));
        }

        Err(anyhow!(
            "Current session is not bound to a digital avatar. Provide portal_id explicitly."
        ))
    }

    async fn load_state(
        &self,
        portal_id: &str,
    ) -> Result<(AvatarGovernanceStatePayload, GovernanceState)> {
        let payload = self
            .agent_service()
            .get_avatar_governance_state(&self.team_id, portal_id)
            .await?;
        let state =
            serde_json::from_value::<GovernanceState>(payload.state.clone()).unwrap_or_default();
        Ok((payload, state))
    }

    async fn mutate_state_with_retry<T, F>(
        &self,
        portal_id: &str,
        mut mutate: F,
    ) -> Result<(AvatarGovernanceStatePayload, T)>
    where
        F: FnMut(&mut GovernanceState, &str) -> Result<T>,
    {
        const MAX_ATTEMPTS: usize = 6;

        for attempt in 0..MAX_ATTEMPTS {
            let (current_payload, mut state) = self.load_state(portal_id).await?;
            let now = Self::now_rfc3339();
            let result = mutate(&mut state, &now)?;
            let next_state = serde_json::to_value(&state)?;
            if let Some(payload) = self
                .agent_service()
                .update_avatar_governance_state_if_current(
                    &current_payload,
                    Some(next_state),
                    None,
                    Some(&self.actor_id),
                    self.actor_name.as_deref(),
                )
                .await?
            {
                return Ok((payload, result));
            }
            if attempt + 1 == MAX_ATTEMPTS {
                break;
            }
        }

        Err(anyhow!(
            "Avatar governance state changed concurrently. Please retry the governance action."
        ))
    }

    fn queue_summary(state: &GovernanceState) -> JsonValue {
        let capability_pending = state
            .capability_requests
            .iter()
            .filter(|item| matches!(item.status.as_str(), "pending" | "needs_human"))
            .count();
        let proposal_pending = state
            .gap_proposals
            .iter()
            .filter(|item| {
                matches!(
                    item.status.as_str(),
                    "pending_approval" | "approved" | "pilot"
                )
            })
            .count();
        let ticket_pending = state
            .optimization_tickets
            .iter()
            .filter(|item| {
                matches!(
                    item.status.as_str(),
                    "pending" | "approved" | "experimenting"
                )
            })
            .count();
        let runtime_pending = state
            .runtime_logs
            .iter()
            .filter(|item| item.status == "pending")
            .count();

        json!({
            "pendingCapabilityRequests": capability_pending,
            "pendingGapProposals": proposal_pending,
            "pendingOptimizationTickets": ticket_pending,
            "pendingRuntimeLogs": runtime_pending,
        })
    }

    fn review_follow_up(kind: &str, decision: &str, portal_id: &str, title: &str) -> JsonValue {
        if matches!(
            decision,
            "reject" | "rejected" | "deny" | "rollback" | "rolled_back"
        ) {
            return json!({
                "stage": "decision_closed",
                "guidance": "这项治理请求已被拒绝或回滚。应向 service/user 明确说明原因，并在必要时转人工处理。",
                "recommendedTools": ["avatar_governance__list_request_status"],
                "suggestedPrompt": format!(
                    "请同步治理结果：请求“{title}”已被拒绝或回滚。portal_id={portal_id}。请说明原因、影响范围，以及是否需要转人工继续处理。"
                ),
            });
        }
        if matches!(
            decision,
            "needs_human" | "human" | "escalate" | "human_review" | "require_human_confirm"
        ) {
            return json!({
                "stage": "handoff_to_human",
                "guidance": "这项治理请求需要进一步人工审核。应整理上下文、风险和建议下一步，再通知 service/user 当前进入人工路径。",
                "recommendedTools": ["avatar_governance__list_request_status"],
                "suggestedPrompt": format!(
                    "请整理治理结果并准备人工审核交接。请求“{title}”已进入人工审核路径，portal_id={portal_id}。请总结背景、风险和建议下一步。"
                ),
            });
        }
        match kind {
            "capability" => json!({
                "stage": "apply_service_boundary",
                "guidance": "能力请求完成结构化审批后，应继续通过 portal_tools 落地最小权限/能力变更，并回读 capability profile 校验结果。若涉及 builtin extension（如 skill_registry / developer / memory），不能只更新 portal allowlist，还必须同步启用 service agent 的 builtin extension。",
                "recommendedTools": [
                    "portal_tools__configure_portal_service_agent",
                    "portal_tools__get_portal_service_capability_profile"
                ],
                "suggestedPrompt": format!(
                    "请继续处理能力请求“{title}”。portal_id={portal_id}，决策={decision}。优先调用 portal_tools__configure_portal_service_agent 完成最小变更；若涉及 builtin extension，请同时设置 enable_builtin_extensions/disable_builtin_extensions 与 allowed_extensions；然后调用 portal_tools__get_portal_service_capability_profile 回读校验，并总结风险与回滚建议。"
                ),
            }),
            "proposal" => json!({
                "stage": "turn_proposal_into_execution_plan",
                "guidance": "岗位提案获批后，应把提案转换成可执行方案：明确能力、文档范围、权限边界、试运行/生效路径，再决定是否落到新的分身配置。",
                "recommendedTools": [
                    "portal_tools__configure_portal_service_agent",
                    "portal_tools__get_portal_service_capability_profile"
                ],
                "suggestedPrompt": format!(
                    "请继续把提案“{title}”落实为执行方案。portal_id={portal_id}，目标状态={decision}。先整理能力/文档/权限边界和验证标准，再按需要调用 portal_tools 完成实际配置，并回读 profile 校验。"
                ),
            }),
            _ => json!({
                "stage": "execute_optimization",
                "guidance": "优化工单进入批准/实验/部署后，应继续执行变更、验证效果并确认是否需要回滚。",
                "recommendedTools": [
                    "portal_tools__configure_portal_service_agent",
                    "portal_tools__get_portal_service_capability_profile"
                ],
                "suggestedPrompt": format!(
                    "请继续执行优化工单“{title}”。portal_id={portal_id}，目标状态={decision}。按工单方案落实变更，完成验证并说明是否继续推进、维持实验，或准备回滚。"
                ),
            }),
        }
    }

    async fn handle_get_runtime_boundary(&self, args: &JsonObject) -> Result<String> {
        let (portal, bound_session) = self.resolve_bound_portal(args).await?;
        let portal_id = portal
            .id
            .as_ref()
            .map(|id| id.to_hex())
            .ok_or_else(|| anyhow!("Portal is missing id."))?;
        let portal_service = self.portal_service();
        let effective = portal_service
            .resolve_effective_public_config(&portal)
            .await?;
        let (payload, state) = self.load_state(&portal_id).await?;
        let session = match bound_session {
            Some(value) => Some(value),
            None => {
                if let Some(session_id) = self.session_id.as_deref() {
                    self.agent_service().get_session(session_id).await?
                } else {
                    None
                }
            }
        };

        let manager_agent_id = portal.coding_agent_id.clone().or(portal.agent_id.clone());
        let avatar_type = portal
            .settings
            .get("avatarType")
            .and_then(JsonValue::as_str)
            .unwrap_or("external_service")
            .to_string();
        let guidance = match self.role {
            AvatarGovernanceRole::Service => {
                "当前边界外的需求，应先提交治理请求；管理 Agent 未批准时再进入人工审核。"
            }
            AvatarGovernanceRole::Manager => {
                "先查看待处理队列与工作台，再做治理决策；高风险项应转人工审核。"
            }
        };

        let body = json!({
            "portal": {
                "portalId": portal_id,
                "slug": portal.slug,
                "name": portal.name,
                "status": portal.status,
                "avatarType": avatar_type,
            },
            "agents": {
                "serviceAgentId": PortalService::resolve_service_agent_id(&portal),
                "managerAgentId": manager_agent_id,
            },
            "documents": {
                "documentAccessMode": PortalService::document_access_mode_key(effective.effective_document_access_mode),
                "boundDocumentCount": portal.bound_document_ids.len(),
                "currentSessionDocumentCount": session.as_ref().map(|item| item.attached_document_ids.len()).unwrap_or(portal.bound_document_ids.len()),
                "portalRestricted": session.as_ref().map(|item| item.portal_restricted).unwrap_or(false),
            },
            "capabilities": {
                "allowedExtensions": effective.effective_allowed_extensions,
                "allowedSkillIds": effective.effective_allowed_skill_ids,
                "allowedSkillNames": effective.effective_allowed_skill_names,
                "extensionsInherited": effective.extensions_inherited,
                "skillsInherited": effective.skills_inherited,
            },
            "governance": {
                "approvalMode": payload.config.get("managerApprovalMode").cloned().unwrap_or(JsonValue::Null),
                "optimizationMode": payload.config.get("optimizationMode").cloned().unwrap_or(JsonValue::Null),
                "highRiskAction": payload.config.get("highRiskAction").cloned().unwrap_or(JsonValue::Null),
                "queueSummary": Self::queue_summary(&state),
                "serviceSessionSource": self.session_source.clone(),
                "path": "service_agent -> manager_agent -> human_review",
            },
            "guidance": guidance,
        });
        Ok(serde_json::to_string_pretty(&body)?)
    }

    async fn handle_submit_capability_request(&self, args: &JsonObject) -> Result<String> {
        let (portal, _) = self.resolve_bound_portal(args).await?;
        let portal_id = portal
            .id
            .as_ref()
            .map(|id| id.to_hex())
            .ok_or_else(|| anyhow!("Portal is missing id."))?;
        let title = Self::get_str_arg(args, &["title"])
            .ok_or_else(|| anyhow!("title is required"))?
            .to_string();
        let detail = Self::get_str_arg(args, &["detail"])
            .ok_or_else(|| anyhow!("detail is required"))?
            .to_string();
        let requested_scope =
            Self::get_string_array_arg(args, &["requested_scope", "requestedScope"]);
        let risk = Self::get_str_arg(args, &["risk"])
            .unwrap_or("medium")
            .to_ascii_lowercase();
        let source = Self::get_str_arg(args, &["source"])
            .unwrap_or("avatar")
            .to_ascii_lowercase();
        let request_id = Uuid::new_v4().to_string();
        let normalized_scope = requested_scope;
        let normalized_risk = match risk.as_str() {
            "low" | "high" => risk,
            _ => "medium".to_string(),
        };
        let normalized_source = match source.as_str() {
            "manager" | "user" => source,
            _ => "avatar".to_string(),
        };
        let (payload, request) = self
            .mutate_state_with_retry(&portal_id, |state, now| {
                let request = CapabilityGapRequest {
                    id: request_id.clone(),
                    title: title.clone(),
                    detail: detail.clone(),
                    requested_scope: normalized_scope.clone(),
                    risk: normalized_risk.clone(),
                    status: "pending".to_string(),
                    source: normalized_source.clone(),
                    created_at: now.to_string(),
                    updated_at: now.to_string(),
                    decision: None,
                    decision_reason: None,
                };
                state.capability_requests.push(request.clone());
                Ok(request)
            })
            .await?;
        let queue = self
            .agent_service()
            .list_avatar_governance_queue(&self.team_id, &portal_id)
            .await
            .unwrap_or_default();
        let body = json!({
            "ok": true,
            "portalId": portal_id,
            "request": request,
            "message": "能力请求已提交给管理 Agent；如未获批准，将进入人工审核。",
            "governancePath": "service_agent -> manager_agent -> human_review",
            "queueSize": queue.len(),
            "updatedAt": payload.updated_at,
        });
        Ok(serde_json::to_string_pretty(&body)?)
    }

    async fn handle_submit_gap_proposal(&self, args: &JsonObject) -> Result<String> {
        let (portal, _) = self.resolve_bound_portal(args).await?;
        let portal_id = portal
            .id
            .as_ref()
            .map(|id| id.to_hex())
            .ok_or_else(|| anyhow!("Portal is missing id."))?;
        let title = Self::get_str_arg(args, &["title"])
            .ok_or_else(|| anyhow!("title is required"))?
            .to_string();
        let description = Self::get_str_arg(args, &["description"])
            .ok_or_else(|| anyhow!("description is required"))?
            .to_string();
        let expected_gain = Self::get_str_arg(args, &["expected_gain", "expectedGain"])
            .unwrap_or("")
            .to_string();
        let proposal_id = Uuid::new_v4().to_string();
        let (payload, proposal) = self
            .mutate_state_with_retry(&portal_id, |state, now| {
                let proposal = GapProposal {
                    id: proposal_id.clone(),
                    title: title.clone(),
                    description: description.clone(),
                    expected_gain: expected_gain.clone(),
                    status: "pending_approval".to_string(),
                    proposed_by: "avatar".to_string(),
                    created_at: now.to_string(),
                    updated_at: now.to_string(),
                    decision_reason: None,
                };
                state.gap_proposals.push(proposal.clone());
                Ok(proposal)
            })
            .await?;
        let body = json!({
            "ok": true,
            "portalId": portal_id,
            "proposal": proposal,
            "message": "改进提案已提交给管理 Agent 评估。",
            "governancePath": "service_agent -> manager_agent -> human_review",
            "updatedAt": payload.updated_at,
        });
        Ok(serde_json::to_string_pretty(&body)?)
    }

    async fn handle_submit_human_review_request(&self, args: &JsonObject) -> Result<String> {
        let (portal, _) = self.resolve_bound_portal(args).await?;
        let portal_id = portal
            .id
            .as_ref()
            .map(|id| id.to_hex())
            .ok_or_else(|| anyhow!("Portal is missing id."))?;
        let title = Self::get_str_arg(args, &["title"])
            .ok_or_else(|| anyhow!("title is required"))?
            .to_string();
        let detail = Self::get_str_arg(args, &["detail"])
            .ok_or_else(|| anyhow!("detail is required"))?
            .to_string();
        let requested_scope =
            Self::get_string_array_arg(args, &["requested_scope", "requestedScope"]);
        let risk = Self::get_str_arg(args, &["risk"])
            .unwrap_or("high")
            .to_ascii_lowercase();
        let request_id = Uuid::new_v4().to_string();
        let (payload, request) = self
            .mutate_state_with_retry(&portal_id, |state, now| {
                let request = CapabilityGapRequest {
                    id: request_id.clone(),
                    title: title.clone(),
                    detail: detail.clone(),
                    requested_scope: requested_scope.clone(),
                    risk: match risk.as_str() {
                        "low" | "medium" => risk.clone(),
                        _ => "high".to_string(),
                    },
                    status: "needs_human".to_string(),
                    source: "avatar".to_string(),
                    created_at: now.to_string(),
                    updated_at: now.to_string(),
                    decision: Some("require_human_confirm".to_string()),
                    decision_reason: Some(
                        "Service agent escalated to manager/human review".to_string(),
                    ),
                };
                state.capability_requests.push(request.clone());
                Ok(request)
            })
            .await?;
        let body = json!({
            "ok": true,
            "portalId": portal_id,
            "request": request,
            "message": "请求已进入管理 Agent / 人工审核路径。",
            "governancePath": "service_agent -> manager_agent -> human_review",
            "updatedAt": payload.updated_at,
        });
        Ok(serde_json::to_string_pretty(&body)?)
    }

    async fn handle_submit_optimization_ticket(&self, args: &JsonObject) -> Result<String> {
        let (portal, _) = self.resolve_bound_portal(args).await?;
        let portal_id = portal
            .id
            .as_ref()
            .map(|id| id.to_hex())
            .ok_or_else(|| anyhow!("Portal is missing id."))?;
        let title = Self::get_str_arg(args, &["title"])
            .ok_or_else(|| anyhow!("title is required"))?
            .to_string();
        let problem_type = Self::get_str_arg(args, &["problem_type", "problemType"])
            .unwrap_or("prompt")
            .to_ascii_lowercase();
        let evidence = Self::get_str_arg(args, &["evidence"])
            .ok_or_else(|| anyhow!("evidence is required"))?
            .to_string();
        let proposal = Self::get_str_arg(args, &["proposal"])
            .ok_or_else(|| anyhow!("proposal is required"))?
            .to_string();
        let expected_gain = Self::get_str_arg(args, &["expected_gain", "expectedGain"])
            .unwrap_or("")
            .to_string();
        let risk = Self::get_str_arg(args, &["risk"])
            .unwrap_or("medium")
            .to_ascii_lowercase();
        let ticket_id = Uuid::new_v4().to_string();
        let (payload, ticket) = self
            .mutate_state_with_retry(&portal_id, |state, now| {
                let ticket = OptimizationTicket {
                    id: ticket_id.clone(),
                    title: title.clone(),
                    problem_type: match problem_type.as_str() {
                        "tool" | "skill" | "policy" | "team_prompt" => problem_type.clone(),
                        _ => "prompt".to_string(),
                    },
                    evidence: evidence.clone(),
                    proposal: proposal.clone(),
                    expected_gain: expected_gain.clone(),
                    risk: match risk.as_str() {
                        "low" | "high" => risk.clone(),
                        _ => "medium".to_string(),
                    },
                    status: "pending".to_string(),
                    created_at: now.to_string(),
                    updated_at: now.to_string(),
                    decision_reason: None,
                };
                state.optimization_tickets.push(ticket.clone());
                Ok(ticket)
            })
            .await?;
        let queue = self
            .agent_service()
            .list_avatar_governance_queue(&self.team_id, &portal_id)
            .await
            .unwrap_or_default();
        let body = json!({
            "ok": true,
            "portalId": portal_id,
            "ticket": ticket,
            "message": "优化工单已进入治理队列，等待管理 Agent 评审。",
            "governancePath": "service_agent -> manager_agent -> human_review",
            "queueSize": queue.len(),
            "updatedAt": payload.updated_at,
        });
        Ok(serde_json::to_string_pretty(&body)?)
    }

    async fn handle_get_latest_governance_update(&self, args: &JsonObject) -> Result<String> {
        let (portal, _) = self.resolve_bound_portal(args).await?;
        let portal_id = portal
            .id
            .as_ref()
            .map(|id| id.to_hex())
            .ok_or_else(|| anyhow!("Portal is missing id."))?;
        let (_, state) = self.load_state(&portal_id).await?;
        let request_id =
            Self::get_str_arg(args, &["request_id", "requestId"]).map(ToOwned::to_owned);
        let kind_filter =
            Self::get_str_arg(args, &["kind"]).map(|value| value.to_ascii_lowercase());

        let mut items: Vec<JsonValue> = Vec::new();
        for item in &state.capability_requests {
            items.push(json!({
                "kind": "capability",
                "id": item.id,
                "title": item.title,
                "status": item.status,
                "detail": item.detail,
                "decision": item.decision,
                "decisionReason": item.decision_reason,
                "updatedAt": item.updated_at,
                "createdAt": item.created_at,
            }));
        }
        for item in &state.gap_proposals {
            items.push(json!({
                "kind": "proposal",
                "id": item.id,
                "title": item.title,
                "status": item.status,
                "detail": item.description,
                "decisionReason": item.decision_reason,
                "updatedAt": item.updated_at,
                "createdAt": item.created_at,
            }));
        }
        for item in &state.optimization_tickets {
            items.push(json!({
                "kind": "ticket",
                "id": item.id,
                "title": item.title,
                "status": item.status,
                "detail": item.proposal,
                "decisionReason": item.decision_reason,
                "updatedAt": item.updated_at,
                "createdAt": item.created_at,
            }));
        }

        if let Some(kind) = kind_filter.as_ref() {
            items
                .retain(|item| item.get("kind").and_then(JsonValue::as_str) == Some(kind.as_str()));
        }
        if let Some(request_id) = request_id.as_ref() {
            items.retain(|item| {
                item.get("id").and_then(JsonValue::as_str) == Some(request_id.as_str())
            });
        }

        items.sort_by(|a, b| {
            let left = a
                .get("updatedAt")
                .or_else(|| a.get("createdAt"))
                .and_then(JsonValue::as_str)
                .unwrap_or("");
            let right = b
                .get("updatedAt")
                .or_else(|| b.get("createdAt"))
                .and_then(JsonValue::as_str)
                .unwrap_or("");
            right.cmp(left)
        });

        let latest = items.into_iter().next();
        let Some(latest) = latest else {
            let body = json!({
                "portalId": portal_id,
                "hasUpdate": false,
                "message": "当前没有可回写的治理进展。",
            });
            return Ok(serde_json::to_string_pretty(&body)?);
        };

        let kind = latest
            .get("kind")
            .and_then(JsonValue::as_str)
            .unwrap_or("capability");
        let status = latest
            .get("status")
            .and_then(JsonValue::as_str)
            .unwrap_or("pending");
        let decision_reason = latest
            .get("decisionReason")
            .and_then(JsonValue::as_str)
            .unwrap_or("")
            .trim()
            .to_string();
        let updated_at = latest
            .get("updatedAt")
            .or_else(|| latest.get("createdAt"))
            .and_then(JsonValue::as_str)
            .unwrap_or("");

        let (progress_label, user_facing_summary, next_action) = match (kind, status) {
            ("capability", "approved") => (
                "管理 Agent 已批准",
                "管理 Agent 已批准这项能力请求。接下来会继续按当前批准范围落地，并向你同步结果。",
                "如需继续推进，可直接说明你现在要继续处理哪一步。",
            ),
            ("capability", "needs_human") => (
                "等待人工审核",
                "这项请求已经交给人工审核，当前不会直接放开能力边界。",
                "如果你有补充资料或背景，可以现在补充，便于人工继续判断。",
            ),
            ("capability", "rejected") => (
                "未获批准",
                "管理 Agent 当前没有批准这项能力请求，所以我不会直接越过当前边界处理。",
                "如果仍需推进，请补充理由、资料，或改为人工继续处理。",
            ),
            ("proposal", "approved") => (
                "提案已通过",
                "这项岗位/能力提案已经通过，接下来会进入执行方案整理。",
                "如需继续推进，可说明你更关注能力、文档范围还是发布边界。",
            ),
            ("proposal", "pilot") => (
                "进入试运行",
                "这项提案已经进入试运行阶段，会先在受控范围内验证。",
                "你可以继续描述希望试运行先覆盖的场景。",
            ),
            ("proposal", "active") => (
                "已生效",
                "这项提案已经正式生效，后续会按新的岗位方案继续处理。",
                "你现在可以直接继续描述下一步要处理的任务。",
            ),
            ("proposal", "rejected") => (
                "提案未通过",
                "这项提案当前没有通过，暂时不会按该方向继续扩展。",
                "如果仍需推进，请补充收益、风险说明，或转人工继续判断。",
            ),
            ("ticket", "approved") => (
                "优化已批准",
                "这项优化工单已经通过，接下来会进入执行和验证。",
                "如果你观察到相同问题还在继续出现，可以继续告诉我具体表现。",
            ),
            ("ticket", "experimenting") => (
                "优化实验中",
                "这项优化正在实验验证中，当前会先观察效果再决定是否全面生效。",
                "你可以继续描述实验期间的表现或异常反馈。",
            ),
            ("ticket", "deployed") => (
                "优化已部署",
                "这项优化已经部署，后续会按新策略继续处理。",
                "你可以直接继续当前任务，我会按最新配置来处理。",
            ),
            ("ticket", "rolled_back") => (
                "优化已回滚",
                "这项优化已经回滚，当前会回到更稳的处理方式。",
                "如果问题仍然存在，请继续描述具体异常，方便重新评估。",
            ),
            ("ticket", "rejected") => (
                "优化未通过",
                "这项优化工单当前没有通过，所以不会直接套用这次变更。",
                "如果需要继续推进，请补充更多证据或让人工继续审核。",
            ),
            _ => (
                "等待管理 Agent",
                "这项治理请求已经进入管理 Agent 队列，正在等待进一步判断。",
                "如需加快判断，可以补充更明确的目标、资料或风险说明。",
            ),
        };

        let body = json!({
            "portalId": portal_id,
            "hasUpdate": true,
            "latest": latest,
            "progressLabel": progress_label,
            "userFacingSummary": user_facing_summary,
            "decisionReason": decision_reason,
            "nextAction": next_action,
            "updatedAt": updated_at,
        });
        Ok(serde_json::to_string_pretty(&body)?)
    }

    async fn handle_list_request_status(&self, args: &JsonObject) -> Result<String> {
        let (portal, _) = self.resolve_bound_portal(args).await?;
        let portal_id = portal
            .id
            .as_ref()
            .map(|id| id.to_hex())
            .ok_or_else(|| anyhow!("Portal is missing id."))?;
        let (_, state) = self.load_state(&portal_id).await?;
        let kind_filter = Self::get_str_arg(args, &["kind"])
            .unwrap_or("all")
            .to_ascii_lowercase();
        let status_filter =
            Self::get_str_arg(args, &["status"]).map(|value| value.to_ascii_lowercase());
        let limit = Self::get_u64_arg(args, &["limit"], 20, 1, 100) as usize;

        let mut items = Vec::new();
        for item in &state.capability_requests {
            items.push(json!({
                "kind": "capability",
                "id": item.id,
                "title": item.title,
                "detail": item.detail,
                "status": item.status,
                "updatedAt": item.updated_at,
                "createdAt": item.created_at,
                "risk": item.risk,
                "source": item.source,
                "requestedScope": item.requested_scope,
                "decision": item.decision,
                "decisionReason": item.decision_reason,
            }));
        }
        for item in &state.gap_proposals {
            items.push(json!({
                "kind": "proposal",
                "id": item.id,
                "title": item.title,
                "detail": item.description,
                "status": item.status,
                "updatedAt": item.updated_at,
                "createdAt": item.created_at,
                "expectedGain": item.expected_gain,
                "proposedBy": item.proposed_by,
                "decisionReason": item.decision_reason,
            }));
        }
        for item in &state.optimization_tickets {
            items.push(json!({
                "kind": "ticket",
                "id": item.id,
                "title": item.title,
                "detail": item.proposal,
                "status": item.status,
                "updatedAt": item.updated_at,
                "createdAt": item.created_at,
                "risk": item.risk,
                "problemType": item.problem_type,
                "expectedGain": item.expected_gain,
                "decisionReason": item.decision_reason,
            }));
        }
        for item in &state.runtime_logs {
            items.push(json!({
                "kind": "runtime",
                "id": item.id,
                "title": item.title,
                "detail": item.proposal,
                "status": item.status,
                "createdAt": item.created_at,
                "risk": item.risk,
                "problemType": item.problem_type,
                "expectedGain": item.expected_gain,
            }));
        }

        items.retain(|item| {
            let kind_ok = kind_filter == "all"
                || item
                    .get("kind")
                    .and_then(JsonValue::as_str)
                    .is_some_and(|kind| kind == kind_filter);
            let status_ok = status_filter.as_ref().is_none_or(|expected| {
                item.get("status")
                    .and_then(JsonValue::as_str)
                    .is_some_and(|status| status.eq_ignore_ascii_case(expected))
            });
            kind_ok && status_ok
        });

        items.sort_by(|a, b| {
            let left = a
                .get("updatedAt")
                .or_else(|| a.get("createdAt"))
                .and_then(JsonValue::as_str)
                .unwrap_or("");
            let right = b
                .get("updatedAt")
                .or_else(|| b.get("createdAt"))
                .and_then(JsonValue::as_str)
                .unwrap_or("");
            right.cmp(left)
        });
        items.truncate(limit);

        let body = json!({
            "portalId": portal_id,
            "summary": Self::queue_summary(&state),
            "items": items,
        });
        Ok(serde_json::to_string_pretty(&body)?)
    }

    async fn handle_list_queue(&self, args: &JsonObject) -> Result<String> {
        let (portal, _) = self.resolve_bound_portal(args).await?;
        let portal_id = portal
            .id
            .as_ref()
            .map(|id| id.to_hex())
            .ok_or_else(|| anyhow!("Portal is missing id."))?;
        let queue = self
            .agent_service()
            .list_avatar_governance_queue(&self.team_id, &portal_id)
            .await?;
        let body = json!({
            "portalId": portal_id,
            "queue": queue,
        });
        Ok(serde_json::to_string_pretty(&body)?)
    }

    async fn handle_get_workbench_snapshot(&self, args: &JsonObject) -> Result<String> {
        let (portal, _) = self.resolve_bound_portal(args).await?;
        let portal_id = portal
            .id
            .as_ref()
            .map(|id| id.to_hex())
            .ok_or_else(|| anyhow!("Portal is missing id."))?;
        let snapshot: AvatarWorkbenchSnapshotPayload = self
            .agent_service()
            .get_avatar_workbench_snapshot(&self.team_id, &portal_id)
            .await?;
        Ok(serde_json::to_string_pretty(&snapshot)?)
    }

    async fn handle_review_request(&self, args: &JsonObject) -> Result<String> {
        let (portal, _) = self.resolve_bound_portal(args).await?;
        let portal_id = portal
            .id
            .as_ref()
            .map(|id| id.to_hex())
            .ok_or_else(|| anyhow!("Portal is missing id."))?;
        let kind = Self::get_str_arg(args, &["kind"])
            .ok_or_else(|| anyhow!("kind is required"))?
            .to_ascii_lowercase();
        let request_id = Self::get_str_arg(args, &["request_id", "requestId"])
            .ok_or_else(|| anyhow!("request_id is required"))?
            .to_string();
        let decision = Self::get_str_arg(args, &["decision"])
            .ok_or_else(|| anyhow!("decision is required"))?
            .to_ascii_lowercase();
        let reason =
            Self::get_str_arg(args, &["decision_reason", "decisionReason"]).map(ToOwned::to_owned);
        let decision_mode = Self::get_str_arg(args, &["decision_mode", "decisionMode"])
            .map(|value| value.to_ascii_lowercase());

        let (payload, updated_item) = self
            .mutate_state_with_retry(&portal_id, |state, now| {
                let updated_item = match kind.as_str() {
                    "capability" => {
                        let item = state
                            .capability_requests
                            .iter_mut()
                            .find(|entry| entry.id == request_id)
                            .ok_or_else(|| anyhow!("Capability request '{}' not found.", request_id))?;
                        let (next_status, next_decision) = match decision.as_str() {
                            "approve" | "approved" | "approve_direct" => (
                                "approved".to_string(),
                                decision_mode
                                    .clone()
                                    .unwrap_or_else(|| "approve_direct".to_string()),
                            ),
                            "sandbox" | "approve_sandbox" => {
                                ("approved".to_string(), "approve_sandbox".to_string())
                            }
                            "needs_human" | "human" | "escalate" | "human_review" => (
                                "needs_human".to_string(),
                                "require_human_confirm".to_string(),
                            ),
                            "reject" | "rejected" | "deny" => {
                                ("rejected".to_string(), "deny".to_string())
                            }
                            other => {
                                return Err(anyhow!(
                                    "Unsupported capability decision '{}'. Use approve / approve_sandbox / needs_human / reject.",
                                    other
                                ));
                            }
                        };
                        item.status = next_status;
                        item.decision = Some(next_decision);
                        item.decision_reason = reason.clone();
                        item.updated_at = now.to_string();
                        json!({
                            "kind": kind,
                            "id": item.id,
                            "title": item.title,
                            "status": item.status,
                            "decision": item.decision,
                            "decisionReason": item.decision_reason,
                            "updatedAt": item.updated_at,
                        })
                    }
                    "proposal" => {
                        let item = state
                            .gap_proposals
                            .iter_mut()
                            .find(|entry| entry.id == request_id)
                            .ok_or_else(|| anyhow!("Gap proposal '{}' not found.", request_id))?;
                        item.status = match decision.as_str() {
                            "approve" | "approved" => "approved".to_string(),
                            "pilot" => "pilot".to_string(),
                            "active" | "activate" => "active".to_string(),
                            "reject" | "rejected" => "rejected".to_string(),
                            other => {
                                return Err(anyhow!(
                                    "Unsupported proposal decision '{}'. Use approve / pilot / active / reject.",
                                    other
                                ));
                            }
                        };
                        item.updated_at = now.to_string();
                        item.decision_reason = reason.clone();
                        json!({
                            "kind": kind,
                            "id": item.id,
                            "title": item.title,
                            "status": item.status,
                            "decisionReason": item.decision_reason,
                            "updatedAt": item.updated_at,
                        })
                    }
                    "ticket" => {
                        let item = state
                            .optimization_tickets
                            .iter_mut()
                            .find(|entry| entry.id == request_id)
                            .ok_or_else(|| anyhow!("Optimization ticket '{}' not found.", request_id))?;
                        item.status = match decision.as_str() {
                            "approve" | "approved" => "approved".to_string(),
                            "experiment" | "experimenting" => "experimenting".to_string(),
                            "deploy" | "deployed" => "deployed".to_string(),
                            "rollback" | "rolled_back" => "rolled_back".to_string(),
                            "reject" | "rejected" => "rejected".to_string(),
                            other => {
                                return Err(anyhow!(
                                    "Unsupported ticket decision '{}'. Use approve / experiment / deploy / rollback / reject.",
                                    other
                                ));
                            }
                        };
                        item.updated_at = now.to_string();
                        item.decision_reason = reason.clone();
                        json!({
                            "kind": kind,
                            "id": item.id,
                            "title": item.title,
                            "status": item.status,
                            "decisionReason": item.decision_reason,
                            "updatedAt": item.updated_at,
                        })
                    }
                    other => {
                        return Err(anyhow!(
                            "Unsupported request kind '{}'. Use capability / proposal / ticket.",
                            other
                        ));
                    }
                };
                Ok(updated_item)
            })
            .await?;
        let queue: Vec<AvatarGovernanceQueueItemPayload> = self
            .agent_service()
            .list_avatar_governance_queue(&self.team_id, &portal_id)
            .await
            .unwrap_or_default();
        let body = json!({
            "ok": true,
            "portalId": portal_id,
            "updated": updated_item,
            "nextStep": Self::review_follow_up(
                &kind,
                &decision,
                &portal_id,
                updated_item
                    .get("title")
                    .and_then(JsonValue::as_str)
                    .unwrap_or("治理请求"),
            ),
            "queueSize": queue.len(),
            "updatedAt": payload.updated_at,
        });
        Ok(serde_json::to_string_pretty(&body)?)
    }
}

#[async_trait::async_trait]
impl McpClientTrait for AvatarGovernanceToolsProvider {
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
            tools: self.tool_definitions(),
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
            "get_runtime_boundary" => self.handle_get_runtime_boundary(&args).await,
            "list_request_status" => self.handle_list_request_status(&args).await,
            "get_latest_governance_update" if self.role == AvatarGovernanceRole::Service => {
                self.handle_get_latest_governance_update(&args).await
            }
            "submit_capability_request" if self.role == AvatarGovernanceRole::Service => {
                self.handle_submit_capability_request(&args).await
            }
            "submit_gap_proposal" if self.role == AvatarGovernanceRole::Service => {
                self.handle_submit_gap_proposal(&args).await
            }
            "submit_human_review_request" if self.role == AvatarGovernanceRole::Service => {
                self.handle_submit_human_review_request(&args).await
            }
            "submit_optimization_ticket" if self.role == AvatarGovernanceRole::Service => {
                self.handle_submit_optimization_ticket(&args).await
            }
            "list_queue" if self.role == AvatarGovernanceRole::Manager => {
                self.handle_list_queue(&args).await
            }
            "get_workbench_snapshot" if self.role == AvatarGovernanceRole::Manager => {
                self.handle_get_workbench_snapshot(&args).await
            }
            "review_request" if self.role == AvatarGovernanceRole::Manager => {
                self.handle_review_request(&args).await
            }
            _ => Err(anyhow!("Unknown or unauthorized tool: {}", name)),
        };

        match result {
            Ok(text) => Ok(CallToolResult::success(vec![Content::text(text)])),
            Err(error) => Ok(CallToolResult::error(vec![Content::text(
                error.to_string(),
            )])),
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
        _arguments: JsonValue,
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
