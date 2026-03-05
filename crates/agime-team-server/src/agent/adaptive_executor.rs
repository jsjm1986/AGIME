//! Adaptive Goal Execution (AGE) engine for mission execution.
//!
//! Implements goal-tree based execution with progress evaluation
//! and pivot protocol. Reuses runtime::execute_via_bridge and
//! MissionManager infrastructure.

use agime_team::MongoDb;
use anyhow::{anyhow, Result};

use std::sync::Arc;
use std::time::Duration;
use tokio_util::sync::CancellationToken;

use super::mission_manager::MissionManager;
use super::mission_mongo::*;
use super::mission_verifier;
use super::runtime;
use super::service_mongo::AgentService;
use super::task_manager::{StreamEvent, TaskManager};

const MAX_PIVOTS_PER_GOAL: u32 = 3;
const MAX_TOTAL_PIVOTS: u32 = 15;
const DEFAULT_GOAL_EXECUTION_TIMEOUT_SECS: u64 = 1200;
const DEFAULT_MIN_GOAL_EXECUTION_TIMEOUT_SECS: u64 = 300;
const MAX_GOAL_EXECUTION_TIMEOUT_SECS: u64 = 7200;
const DEFAULT_GOAL_TIMEOUT_CANCEL_GRACE_SECS: u64 = 20;
const MAX_GOAL_TIMEOUT_CANCEL_GRACE_SECS: u64 = 120;
const DEFAULT_GOAL_TIMEOUT_RETRY_LIMIT: u32 = 1;
const MAX_GOAL_RETRY_LIMIT: u32 = 8;
const DEFAULT_MISSION_PLANNING_TIMEOUT_SECS: u64 = 300;
const MAX_MISSION_PLANNING_TIMEOUT_SECS: u64 = 1800;
const DEFAULT_PLANNING_TIMEOUT_CANCEL_GRACE_SECS: u64 = 20;
const MAX_PLANNING_TIMEOUT_CANCEL_GRACE_SECS: u64 = 120;
const RETRY_CONTEXT_TOOL_CALL_LIMIT: usize = 12;
const RETRY_CONTEXT_OUTPUT_LIMIT: usize = 1200;
const MISSION_PREFLIGHT_TOOL_NAME: &str = "mission_preflight__preflight";
const MISSION_VERIFY_CONTRACT_TOOL_NAME: &str = "mission_preflight__verify_contract";

enum PivotDecision {
    Retry { approach: String },
    Abandon { reason: String },
}

/// AGE executor that orchestrates goal-tree based task execution.
pub struct AdaptiveExecutor {
    db: Arc<MongoDb>,
    mission_manager: Arc<MissionManager>,
    agent_service: Arc<AgentService>,
    internal_task_manager: Arc<TaskManager>,
    workspace_root: String,
}

impl AdaptiveExecutor {
    pub fn new(
        db: Arc<MongoDb>,
        mission_manager: Arc<MissionManager>,
        workspace_root: String,
    ) -> Self {
        let agent_service = Arc::new(AgentService::new(db.clone()));
        let internal_task_manager = Arc::new(TaskManager::new());
        Self {
            db,
            mission_manager,
            agent_service,
            internal_task_manager,
            workspace_root,
        }
    }

    /// Execute an adaptive mission.
    /// NOTE: Cleanup (Done broadcast + mission_manager.complete) is handled by
    /// the caller MissionExecutor::execute_mission, so we do NOT duplicate it here.
    pub async fn execute_adaptive(
        &self,
        mission_id: &str,
        cancel_token: CancellationToken,
    ) -> Result<()> {
        // 1. Load mission
        let mission = self
            .agent_service
            .get_mission(mission_id)
            .await
            .map_err(|e| anyhow!("DB error: {}", e))?
            .ok_or_else(|| anyhow!("Mission not found"))?;

        let session_id;

        // Create workspace directory for this mission
        let workspace_path = runtime::create_workspace_dir(
            &self.workspace_root,
            &[
                (&mission.team_id, "team_id"),
                ("missions", "category"),
                (mission_id, "mission_id"),
            ],
        )?;
        self.agent_service
            .set_mission_workspace(mission_id, &workspace_path)
            .await
            .map_err(|e| anyhow!("Failed to set workspace: {}", e))?;

        if mission.status == MissionStatus::Draft {
            // ── Planning Phase: decompose goal into goal tree ──
            session_id = self
                .run_planning_phase(
                    mission_id,
                    &mission,
                    cancel_token.clone(),
                    Some(&workspace_path),
                )
                .await?;

            // Check approval policy: checkpoint/manual → pause for user confirmation
            if mission.approval_policy != ApprovalPolicy::Auto {
                self.agent_service
                    .update_mission_status(mission_id, &MissionStatus::Planned)
                    .await
                    .map_err(|e| anyhow!("Failed to update status: {}", e))?;

                self.mission_manager
                    .broadcast(
                        mission_id,
                        StreamEvent::Status {
                            status: r#"{"type":"mission_planned","mode":"adaptive"}"#.to_string(),
                        },
                    )
                    .await;

                // Return Ok — caller's cleanup will read actual status (Planned)
                return Ok(());
            }
        } else if mission.status == MissionStatus::Planned {
            // ── User confirmed the plan, skip planning ──
            session_id = mission
                .session_id
                .as_deref()
                .ok_or_else(|| anyhow!("Mission has no session"))?
                .to_string();
        } else {
            return Err(anyhow!(
                "Mission must be in Draft or Planned status to start"
            ));
        }

        // ── Execution Phase ──
        self.run_execution_phase(
            mission_id,
            &mission.agent_id,
            &session_id,
            cancel_token,
            Some(&workspace_path),
            None,
        )
        .await
    }

    /// Planning phase: create session, decompose goal, save goal tree.
    /// Returns the session_id on success.
    async fn run_planning_phase(
        &self,
        mission_id: &str,
        mission: &MissionDoc,
        cancel_token: CancellationToken,
        workspace_path: Option<&str>,
    ) -> Result<String> {
        // Create dedicated AgentSession
        let session = self
            .agent_service
            .create_chat_session(
                &mission.team_id,
                &mission.agent_id,
                &mission.creator_id,
                mission.attached_document_ids.clone(),
                None,
                None,
                None,
                None,
                None,
                mission.step_timeout_seconds,
                None,
                false,
                false,
                None,
                Some("mission".to_string()),
                Some(mission_id.to_string()),
                Some(true),
            )
            .await
            .map_err(|e| anyhow!("Failed to create session: {}", e))?;

        let session_id = session.session_id.clone();
        self.agent_service
            .set_mission_session(mission_id, &session_id)
            .await
            .map_err(|e| anyhow!("Failed to set session: {}", e))?;

        self.agent_service
            .update_mission_status(mission_id, &MissionStatus::Planning)
            .await
            .map_err(|e| anyhow!("Failed to update status: {}", e))?;

        self.mission_manager
            .broadcast(
                mission_id,
                StreamEvent::Status {
                    status: r#"{"type":"mission_planning","mode":"adaptive"}"#.to_string(),
                },
            )
            .await;

        let planning_timeout = Self::planning_timeout();
        let planning_cancel = CancellationToken::new();
        {
            let linked = planning_cancel.clone();
            let external = cancel_token.clone();
            tokio::spawn(async move {
                external.cancelled().await;
                linked.cancel();
            });
        }

        let goals = match tokio::time::timeout(
            planning_timeout,
            self.decompose_goal(
                mission_id,
                mission,
                &session_id,
                planning_cancel.clone(),
                workspace_path,
            ),
        )
        .await
        {
            Ok(result) => result?,
            Err(_) => {
                planning_cancel.cancel();
                tokio::time::sleep(Self::planning_timeout_cancel_grace()).await;
                return Err(anyhow!(
                    "Adaptive mission planning timed out after {}s",
                    planning_timeout.as_secs()
                ));
            }
        };

        if goals.is_empty() {
            return Err(anyhow!("Agent generated empty goal tree"));
        }

        self.agent_service
            .save_goal_tree(mission_id, goals)
            .await
            .map_err(|e| anyhow!("Failed to save goal tree: {}", e))?;

        Ok(session_id)
    }

    /// Execution phase: run goal loop, check for pause, synthesize results.
    async fn run_execution_phase(
        &self,
        mission_id: &str,
        agent_id: &str,
        session_id: &str,
        cancel_token: CancellationToken,
        workspace_path: Option<&str>,
        operator_hint: Option<&str>,
    ) -> Result<()> {
        self.agent_service
            .update_mission_status(mission_id, &MissionStatus::Running)
            .await
            .map_err(|e| anyhow!("Failed to update status: {}", e))?;

        self.execute_goal_loop(
            mission_id,
            agent_id,
            session_id,
            cancel_token.clone(),
            workspace_path,
            operator_hint,
        )
        .await?;

        self.synthesize_and_complete(
            mission_id,
            agent_id,
            session_id,
            cancel_token,
            workspace_path,
        )
        .await
    }

    /// Post-loop: skip synthesis if mission already reached a terminal/pause state,
    /// otherwise synthesize results and mark completed.
    async fn synthesize_and_complete(
        &self,
        mission_id: &str,
        agent_id: &str,
        session_id: &str,
        cancel_token: CancellationToken,
        workspace_path: Option<&str>,
    ) -> Result<()> {
        if let Ok(Some(m)) = self.agent_service.get_mission(mission_id).await {
            if matches!(
                m.status,
                MissionStatus::Paused
                    | MissionStatus::Cancelled
                    | MissionStatus::Failed
                    | MissionStatus::Completed
            ) {
                return Ok(());
            }
        }

        self.synthesize_results(
            mission_id,
            agent_id,
            session_id,
            cancel_token,
            workspace_path,
        )
        .await?;

        if let Err(e) = self
            .agent_service
            .update_mission_status(mission_id, &MissionStatus::Completed)
            .await
        {
            tracing::warn!("Failed to mark mission {} completed: {}", mission_id, e);
        }

        Ok(())
    }

    /// Decompose mission goal into a goal tree via LLM.
    async fn decompose_goal(
        &self,
        mission_id: &str,
        mission: &MissionDoc,
        session_id: &str,
        cancel_token: CancellationToken,
        workspace_path: Option<&str>,
    ) -> Result<Vec<GoalNode>> {
        let context_section = mission
            .context
            .as_deref()
            .map(|c| format!("\n## Additional Context\n{}", c))
            .unwrap_or_default();

        let prompt = format!(
            r#"You are decomposing a mission goal. Analyze the following goal and create a tree of 2-8 sub-goals.

## Goal
{}
{}

## Output Format
Output a JSON array wrapped in ```json code block. Each goal:
[{{"goal_id": "g-1", "parent_id": null, "title": "...", "description": "...", "success_criteria": "How to verify this goal is complete", "is_checkpoint": false, "order": 0}}]

Rules:
- goal_id format: "g-1", "g-2", "g-1-1" (sub-goals use parent ID prefix)
- parent_id is null for top-level goals
- success_criteria must be concrete and verifiable
- Set is_checkpoint: true for steps requiring human review
- Each goal should be an independently executable unit of work"#,
            mission.goal, context_section
        );

        self.execute_via_bridge(
            &mission.agent_id,
            session_id,
            mission_id,
            &prompt,
            cancel_token,
            workspace_path,
            None, // no mission_context during planning
        )
        .await?;

        // Parse goal tree from session messages
        let session = self
            .agent_service
            .get_session(session_id)
            .await
            .map_err(|e| anyhow!("DB error: {}", e))?
            .ok_or_else(|| anyhow!("Session not found"))?;

        let text = match runtime::extract_last_assistant_text(&session.messages_json) {
            Some(text) => text,
            None => {
                tracing::warn!(
                    "Mission {} adaptive planning has no assistant response, using fallback goal",
                    mission.mission_id
                );
                return Ok(vec![self.fallback_goal_from_mission(mission)]);
            }
        };

        let json_str = runtime::extract_json_block(&text);
        match self.parse_goal_tree_json(&json_str) {
            Ok(goals) if !goals.is_empty() => Ok(goals),
            Ok(_) => {
                tracing::warn!(
                    "Mission {} adaptive planning produced empty goal tree, using fallback goal",
                    mission.mission_id
                );
                Ok(vec![self.fallback_goal_from_mission(mission)])
            }
            Err(e) => {
                tracing::warn!(
                    "Mission {} adaptive planning JSON parse failed: {}. Using fallback goal",
                    mission.mission_id,
                    e
                );
                Ok(vec![self.fallback_goal_from_mission(mission)])
            }
        }
    }

    /// Parse goal tree JSON into GoalNode entries.
    fn parse_goal_tree_json(&self, json_str: &str) -> Result<Vec<GoalNode>> {
        #[derive(serde::Deserialize)]
        struct RawGoal {
            goal_id: String,
            parent_id: Option<String>,
            title: String,
            description: String,
            success_criteria: String,
            #[serde(default)]
            is_checkpoint: bool,
            #[serde(default)]
            order: u32,
        }

        fn parse_raw_goals_value(
            value: serde_json::Value,
        ) -> Result<Vec<RawGoal>, serde_json::Error> {
            if value.is_array() {
                return serde_json::from_value(value);
            }
            if let Some(arr) = value
                .get("goals")
                .or_else(|| value.get("goal_tree"))
                .or_else(|| value.get("steps"))
                .and_then(|v| v.as_array())
            {
                return serde_json::from_value(serde_json::Value::Array(arr.clone()));
            }
            serde_json::from_value(value)
        }

        let normalized = runtime::normalize_loose_json(json_str);
        let candidates: [&str; 2] = [json_str, &normalized];
        let mut raw: Option<Vec<RawGoal>> = None;
        let mut last_err = None;
        for candidate in candidates {
            match serde_json::from_str::<serde_json::Value>(candidate)
                .and_then(parse_raw_goals_value)
            {
                Ok(goals) => {
                    raw = Some(goals);
                    break;
                }
                Err(e) => {
                    last_err = Some(e.to_string());
                }
            }
        }

        let raw = raw.ok_or_else(|| {
            anyhow!(
                "Failed to parse goal tree JSON: {}",
                last_err.unwrap_or_else(|| "unknown error".to_string())
            )
        })?;

        let goals = raw
            .into_iter()
            .map(|r| {
                let depth = if r.parent_id.is_none() {
                    0
                } else {
                    let dashes = r.goal_id.matches('-').count() as u32;
                    dashes.saturating_sub(1)
                };
                GoalNode {
                    goal_id: r.goal_id,
                    parent_id: r.parent_id,
                    title: r.title,
                    description: r.description,
                    success_criteria: r.success_criteria,
                    status: GoalStatus::Pending,
                    depth,
                    order: r.order,
                    exploration_budget: 3,
                    attempts: vec![],
                    output_summary: None,
                    runtime_contract: None,
                    contract_verification: None,
                    pivot_reason: None,
                    is_checkpoint: r.is_checkpoint,
                    created_at: Some(bson::DateTime::now()),
                    completed_at: None,
                }
            })
            .collect();

        Ok(goals)
    }

    fn fallback_goal_from_mission(&self, mission: &MissionDoc) -> GoalNode {
        GoalNode {
            goal_id: "g-1".to_string(),
            parent_id: None,
            title: "执行核心目标".to_string(),
            description: mission.goal.clone(),
            success_criteria: "给出可验证的最终结果或明确失败原因".to_string(),
            status: GoalStatus::Pending,
            depth: 0,
            order: 0,
            exploration_budget: 3,
            attempts: vec![],
            output_summary: None,
            runtime_contract: None,
            contract_verification: None,
            pivot_reason: None,
            is_checkpoint: false,
            created_at: Some(bson::DateTime::now()),
            completed_at: None,
        }
    }

    /// Core execution loop — iterates over goal tree using state machine pattern.
    async fn execute_goal_loop(
        &self,
        mission_id: &str,
        agent_id: &str,
        session_id: &str,
        cancel_token: CancellationToken,
        workspace_path: Option<&str>,
        operator_hint: Option<&str>,
    ) -> Result<()> {
        loop {
            // 1. Reload goal tree from DB
            let mission = self
                .agent_service
                .get_mission(mission_id)
                .await
                .map_err(|e| anyhow!("DB error: {}", e))?
                .ok_or_else(|| anyhow!("Mission not found"))?;

            let goals = mission.goal_tree.as_deref().unwrap_or(&[]);

            // 2. Find next executable goal
            let next = Self::find_next_goal(goals);
            let goal = match next {
                Some(g) => g.clone(),
                None => break, // All done or abandoned
            };

            // 3. Check cancellation — return Ok so outer cleanup reads actual DB status
            if cancel_token.is_cancelled() {
                // Only set Cancelled if not already Paused (pause route sets Paused before cancelling token)
                if let Ok(Some(m)) = self.agent_service.get_mission(mission_id).await {
                    if m.status != MissionStatus::Paused {
                        if let Err(e) = self
                            .agent_service
                            .update_mission_status(mission_id, &MissionStatus::Cancelled)
                            .await
                        {
                            tracing::warn!(
                                "Failed to mark mission {} cancelled during adaptive loop: {}",
                                mission_id,
                                e
                            );
                        }
                    }
                }
                return Ok(());
            }

            // 4. Check token budget
            if mission.token_budget > 0 && mission.total_tokens_used >= mission.token_budget {
                if let Err(e) = self
                    .agent_service
                    .update_mission_status(mission_id, &MissionStatus::Failed)
                    .await
                {
                    tracing::warn!("Failed to set mission {} failed: {}", mission_id, e);
                }
                return Err(anyhow!("Token budget exceeded"));
            }

            // 5. Check approval policy for goals.
            // A goal approved via route marks current_goal_id to bypass re-pause.
            let goal_approved = mission.current_goal_id.as_deref() == Some(goal.goal_id.as_str())
                && goal.status == GoalStatus::Pending;
            let needs_approval = match mission.approval_policy {
                ApprovalPolicy::Auto => false,
                ApprovalPolicy::Checkpoint => goal.is_checkpoint,
                ApprovalPolicy::Manual => true,
            };
            if needs_approval && goal.status == GoalStatus::Pending && !goal_approved {
                if let Err(e) = self
                    .agent_service
                    .update_goal_status(mission_id, &goal.goal_id, &GoalStatus::AwaitingApproval)
                    .await
                {
                    tracing::warn!(
                        "Failed to set goal {} awaiting_approval: {}",
                        goal.goal_id,
                        e
                    );
                }
                if let Err(e) = self
                    .agent_service
                    .update_mission_status(mission_id, &MissionStatus::Paused)
                    .await
                {
                    tracing::warn!("Failed to pause mission {}: {}", mission_id, e);
                }
                self.mission_manager
                    .broadcast(
                        mission_id,
                        StreamEvent::Status {
                            status: format!(
                                r#"{{"type":"mission_paused","goal_id":"{}","reason":"{}"}}"#,
                                goal.goal_id,
                                if mission.approval_policy == ApprovalPolicy::Manual {
                                    "manual"
                                } else {
                                    "checkpoint"
                                }
                            ),
                        },
                    )
                    .await;
                return Ok(());
            }

            // 6. Collect completed goals for context
            let completed_goals: Vec<&GoalNode> = goals
                .iter()
                .filter(|g| g.status == GoalStatus::Completed)
                .collect();
            let goal_step_index = completed_goals.len() as u32;

            let policy_str = match mission.approval_policy {
                ApprovalPolicy::Auto => "auto",
                ApprovalPolicy::Checkpoint => "checkpoint",
                ApprovalPolicy::Manual => "manual",
            };

            // 7. Execute goal
            let workspace_before = match workspace_path {
                Some(wp) => runtime::snapshot_workspace_files(wp).ok(),
                None => None,
            };
            let goal_contract = self
                .run_single_goal(
                    mission_id,
                    agent_id,
                    session_id,
                    &goal,
                    &completed_goals,
                    cancel_token.clone(),
                    workspace_path,
                    policy_str,
                    completed_goals.len() + 1,
                    goals.len(),
                    mission.step_timeout_seconds,
                    mission.step_max_retries,
                    operator_hint,
                )
                .await?;

            // Pause/cancel can happen while goal is executing.
            // If so, stop the loop without evaluating progress.
            if let Ok(Some(current)) = self.agent_service.get_mission(mission_id).await {
                if matches!(
                    current.status,
                    MissionStatus::Paused | MissionStatus::Cancelled
                ) {
                    return Ok(());
                }
            }

            // 8. Evaluate progress
            let signal = self
                .evaluate_goal(
                    mission_id,
                    agent_id,
                    session_id,
                    &goal,
                    cancel_token.clone(),
                    workspace_path,
                )
                .await?;

            // 8.1 Update the last attempt's signal with actual evaluation result
            if let Err(e) = self
                .agent_service
                .update_last_attempt_signal(mission_id, &goal.goal_id, &signal)
                .await
            {
                tracing::warn!(
                    "Failed to update attempt signal for goal {}: {}",
                    goal.goal_id,
                    e
                );
            }

            // 9. Handle signal
            match signal {
                ProgressSignal::Advancing => {
                    self.complete_goal(
                        mission_id,
                        &goal,
                        goal_step_index,
                        &goal_contract,
                        workspace_path,
                        workspace_before.as_ref(),
                    )
                    .await?;
                }
                ProgressSignal::Stalled => {
                    // Check exploration budget
                    let attempt_count = goal.attempts.len() as u32 + 1;
                    if attempt_count >= goal.exploration_budget {
                        self.handle_pivot(
                            mission_id,
                            agent_id,
                            session_id,
                            &goal,
                            &signal,
                            cancel_token.clone(),
                            workspace_path,
                        )
                        .await?;
                    } else {
                        // Reset to Pending so find_next_goal picks it up again
                        if let Err(e) = self
                            .agent_service
                            .update_goal_status(mission_id, &goal.goal_id, &GoalStatus::Pending)
                            .await
                        {
                            tracing::warn!(
                                "Failed to reset goal {} to pending: {}",
                                goal.goal_id,
                                e
                            );
                        }
                    }
                }
                ProgressSignal::Blocked => {
                    self.handle_pivot(
                        mission_id,
                        agent_id,
                        session_id,
                        &goal,
                        &signal,
                        cancel_token.clone(),
                        workspace_path,
                    )
                    .await?;
                }
            }
        }

        Ok(())
    }

    /// Bridge to TaskExecutor (same pattern as MissionExecutor).
    #[allow(clippy::too_many_arguments)]
    async fn execute_via_bridge(
        &self,
        agent_id: &str,
        session_id: &str,
        mission_id: &str,
        user_message: &str,
        cancel_token: CancellationToken,
        workspace_path: Option<&str>,
        mission_context: Option<serde_json::Value>,
    ) -> Result<()> {
        runtime::execute_via_bridge(
            &self.db,
            &self.agent_service,
            &self.internal_task_manager,
            &self.mission_manager,
            mission_id,
            agent_id,
            session_id,
            user_message,
            cancel_token,
            workspace_path,
            Some(mission_id),
            None,
            mission_context,
        )
        .await
    }

    /// Find the next executable goal from the goal tree.
    /// Priority: leaf nodes first (highest depth), then by order.
    fn find_next_goal(goals: &[GoalNode]) -> Option<&GoalNode> {
        // Collect IDs of goals that have non-terminal children
        let parent_ids_with_pending: Vec<&str> = goals
            .iter()
            .filter(|g| {
                matches!(
                    g.status,
                    GoalStatus::Pending
                        | GoalStatus::Pivoting
                        | GoalStatus::Running
                        | GoalStatus::AwaitingApproval
                )
            })
            .filter_map(|g| g.parent_id.as_deref())
            .collect();

        let mut candidates: Vec<&GoalNode> = goals
            .iter()
            .filter(|g| g.status == GoalStatus::Pending || g.status == GoalStatus::Pivoting)
            .filter(|g| {
                // Skip parents that have pending children
                !parent_ids_with_pending.contains(&g.goal_id.as_str())
            })
            .collect();

        // Sort by depth DESC, order ASC
        candidates.sort_by(|a, b| b.depth.cmp(&a.depth).then(a.order.cmp(&b.order)));

        candidates.first().copied()
    }

    /// Execute a single goal via bridge.
    #[allow(clippy::too_many_arguments)]
    async fn run_single_goal(
        &self,
        mission_id: &str,
        agent_id: &str,
        session_id: &str,
        goal: &GoalNode,
        completed_goals: &[&GoalNode],
        cancel_token: CancellationToken,
        workspace_path: Option<&str>,
        approval_policy: &str,
        current_step: usize,
        total_steps: usize,
        mission_step_timeout_seconds: Option<u64>,
        mission_step_max_retries: Option<u32>,
        operator_hint: Option<&str>,
    ) -> Result<runtime::MissionPreflightContract> {
        let tokens_before = self.get_session_total_tokens(session_id).await;
        let messages_before = self
            .agent_service
            .get_session(session_id)
            .await
            .ok()
            .flatten()
            .map(|s| runtime::count_session_messages(&s.messages_json))
            .unwrap_or(0);

        // Mark as Running
        if let Err(e) = self
            .agent_service
            .update_goal_status(mission_id, &goal.goal_id, &GoalStatus::Running)
            .await
        {
            tracing::warn!("Failed to set goal {} running: {}", goal.goal_id, e);
        }
        if let Err(e) = self
            .agent_service
            .advance_mission_goal(mission_id, &goal.goal_id)
            .await
        {
            tracing::warn!("Failed to advance mission goal to {}: {}", goal.goal_id, e);
        }

        // Broadcast GoalStart
        self.mission_manager
            .broadcast(
                mission_id,
                StreamEvent::GoalStart {
                    goal_id: goal.goal_id.clone(),
                    title: goal.title.clone(),
                    depth: goal.depth,
                },
            )
            .await;

        // Build prompt
        let base_prompt =
            Self::build_goal_prompt(goal, completed_goals, workspace_path, operator_hint);

        // Execute via bridge with mission context + retry/timeout protection
        let mc_json = serde_json::json!({
            "goal": goal.title,
            "approval_policy": approval_policy,
            "total_steps": total_steps,
            "current_step": current_step,
        });

        let max_retries = Self::resolve_goal_max_retries(mission_step_max_retries);
        let goal_timeout = Self::resolve_goal_timeout(mission_step_timeout_seconds);
        let timeout_retry_limit = Self::goal_timeout_retry_limit().min(max_retries);
        let timeout_cancel_grace = Self::goal_timeout_cancel_grace();
        let mut timeout_retries_used: u32 = 0;
        let mut last_err: Option<anyhow::Error> = None;

        for attempt in 0..=max_retries {
            let prompt = if attempt == 0 {
                base_prompt.clone()
            } else {
                let prev_err = last_err
                    .as_ref()
                    .map(|e| e.to_string())
                    .unwrap_or_else(|| "unknown error".to_string());
                let (recent_tool_calls, previous_output) =
                    match self.agent_service.get_session(session_id).await {
                        Ok(Some(sess)) => (
                            runtime::recent_tool_calls_for_retry(
                                &sess.messages_json,
                                RETRY_CONTEXT_TOOL_CALL_LIMIT,
                            ),
                            runtime::latest_assistant_output_for_retry(
                                &sess.messages_json,
                                RETRY_CONTEXT_OUTPUT_LIMIT,
                            ),
                        ),
                        Ok(None) => (Vec::new(), None),
                        Err(err) => {
                            tracing::debug!(
                                "Failed to load session {} for goal retry context: {}",
                                session_id,
                                err
                            );
                            (Vec::new(), None)
                        }
                    };
                let playbook = runtime::render_retry_playbook(&runtime::RetryPlaybookContext {
                    mode_label: "goal".to_string(),
                    unit_title: goal.title.clone(),
                    attempt_number: attempt + 1,
                    max_attempts: max_retries + 1,
                    failure_message: prev_err,
                    workspace_path: workspace_path.map(|s| s.to_string()),
                    previous_output,
                    recent_tool_calls,
                });
                format!("{}\n\n{}", base_prompt, playbook)
            };

            if attempt > 0 {
                self.mission_manager
                    .broadcast(
                        mission_id,
                        StreamEvent::Status {
                            status: format!(
                                r#"{{"type":"goal_retry","goal_id":"{}","attempt":{}}}"#,
                                goal.goal_id, attempt
                            ),
                        },
                    )
                    .await;

                // 2s, 4s, 8s, 16s, 16s...
                let delay = Duration::from_secs(2u64.saturating_pow(attempt.min(4)));
                tokio::time::sleep(delay).await;
            }

            let attempt_cancel = cancel_token.child_token();
            let exec_fut = self.execute_via_bridge(
                agent_id,
                session_id,
                mission_id,
                &prompt,
                attempt_cancel.clone(),
                workspace_path,
                Some(mc_json.clone()),
            );
            tokio::pin!(exec_fut);

            let attempt_result = match tokio::time::timeout(goal_timeout, &mut exec_fut).await {
                Ok(res) => res,
                Err(_) => {
                    attempt_cancel.cancel();
                    match tokio::time::timeout(timeout_cancel_grace, &mut exec_fut).await {
                        Ok(Ok(_)) => {
                            tracing::warn!(
                                "Mission {} goal {} exceeded {}s timeout but completed during {}s cancel grace",
                                mission_id,
                                goal.goal_id,
                                goal_timeout.as_secs(),
                                timeout_cancel_grace.as_secs()
                            );
                        }
                        Ok(Err(err)) => {
                            tracing::debug!(
                                "Mission {} goal {} stopped after timeout cancellation: {}",
                                mission_id,
                                goal.goal_id,
                                err
                            );
                        }
                        Err(_) => {
                            tracing::warn!(
                                "Mission {} goal {} did not stop within {}s cancel grace after timeout",
                                mission_id,
                                goal.goal_id,
                                timeout_cancel_grace.as_secs()
                            );
                        }
                    }

                    Err(anyhow!(
                        "Goal {} timed out after {}s",
                        goal.goal_id,
                        goal_timeout.as_secs()
                    ))
                }
            };

            match attempt_result {
                Ok(_) => {
                    let mut goal_tool_calls: Vec<ToolCallRecord> = Vec::new();
                    let mut preflight_contract: Option<runtime::MissionPreflightContract> = None;
                    let mut verify_contract_status: Option<bool> = None;
                    if let Ok(Some(sess)) = self.agent_service.get_session(session_id).await {
                        preflight_contract = runtime::extract_latest_preflight_contract_since(
                            &sess.messages_json,
                            messages_before,
                            MISSION_PREFLIGHT_TOOL_NAME,
                        );
                        verify_contract_status =
                            runtime::extract_latest_verify_contract_status_since(
                                &sess.messages_json,
                                messages_before,
                                MISSION_VERIFY_CONTRACT_TOOL_NAME,
                            );
                        goal_tool_calls = mission_verifier::from_tool_tuples(
                            runtime::extract_tool_calls_since(&sess.messages_json, messages_before),
                        );
                    }
                    let effective_contract = match mission_verifier::resolve_effective_contract(
                        preflight_contract,
                        MISSION_PREFLIGHT_TOOL_NAME,
                        mission_verifier::VerifierLimits {
                            max_required_artifacts: 16,
                            max_completion_checks: 8,
                            max_completion_check_cmd_len: 300,
                        },
                    ) {
                        Ok(contract) => contract,
                        Err(check_err) => {
                            self.mission_manager
                                    .broadcast(
                                        mission_id,
                                        StreamEvent::Status {
                                            status: format!(
                                                r#"{{"type":"goal_validation_failed","goal_id":"{}","attempt":{},"reason":"{}"}}"#,
                                                goal.goal_id,
                                                attempt + 1,
                                                check_err
                                                    .to_string()
                                                    .replace('"', r#"\""#)
                                                    .replace('\n', " ")
                                            ),
                                        },
                                    )
                                    .await;

                            if attempt < max_retries {
                                tracing::warn!(
                                        "Goal {} attempt {} failed preflight validation (will retry): {}",
                                        goal.goal_id,
                                        attempt + 1,
                                        check_err
                                    );
                                last_err = Some(anyhow!(
                                    "Goal preflight validation failed: {}",
                                    check_err
                                ));
                                continue;
                            }
                            return Err(anyhow!("Goal preflight validation failed: {}", check_err));
                        }
                    };
                    if let Err(e) = self
                        .agent_service
                        .set_goal_runtime_contract(
                            mission_id,
                            &goal.goal_id,
                            &Self::to_runtime_contract_doc(&effective_contract),
                        )
                        .await
                    {
                        tracing::warn!(
                            "Failed to persist runtime contract for mission {} goal {}: {}",
                            mission_id,
                            goal.goal_id,
                            e
                        );
                    }

                    // Extract summary and validate declared contract against workspace.
                    let summary = self.extract_step_summary(session_id).await;
                    if let Err(check_err) = mission_verifier::validate_contract_outputs(
                        &effective_contract,
                        workspace_path,
                        summary.as_deref(),
                        &goal_tool_calls,
                        0,
                        MISSION_PREFLIGHT_TOOL_NAME,
                        mission_verifier::CompletionCheckMode::ExistsOnly,
                        false,
                    )
                    .await
                    {
                        self.mission_manager
                            .broadcast(
                                mission_id,
                                StreamEvent::Status {
                                    status: format!(
                                        r#"{{"type":"goal_validation_failed","goal_id":"{}","attempt":{},"reason":"{}"}}"#,
                                        goal.goal_id,
                                        attempt + 1,
                                        check_err
                                            .to_string()
                                            .replace('"', r#"\""#)
                                            .replace('\n', " ")
                                    ),
                                },
                            )
                            .await;

                        if attempt < max_retries {
                            tracing::warn!(
                                "Goal {} attempt {} failed completion validation (will retry): {}",
                                goal.goal_id,
                                attempt + 1,
                                check_err
                            );
                            last_err =
                                Some(anyhow!("Goal completion validation failed: {}", check_err));
                            continue;
                        }
                        return Err(anyhow!("Goal completion validation failed: {}", check_err));
                    }

                    let gate_mode = runtime::contract_verify_gate_mode();
                    let verify_tool_called = mission_verifier::has_verify_contract_tool_call(
                        &goal_tool_calls,
                        MISSION_VERIFY_CONTRACT_TOOL_NAME,
                    );
                    let verify_status_label = mission_verifier::verify_contract_status_label(
                        verify_tool_called,
                        verify_contract_status,
                    );
                    let gate_error = mission_verifier::enforce_verify_contract_gate(
                        gate_mode,
                        verify_tool_called,
                        verify_contract_status,
                        MISSION_VERIFY_CONTRACT_TOOL_NAME,
                    )
                    .err();
                    let gate_reason = gate_error
                        .as_ref()
                        .map(|e| e.to_string())
                        .unwrap_or_default();
                    if let Err(e) = self
                        .agent_service
                        .set_goal_contract_verification(
                            mission_id,
                            &goal.goal_id,
                            &RuntimeContractVerification {
                                tool_called: verify_tool_called,
                                status: Some(verify_status_label.to_string()),
                                gate_mode: Some(
                                    runtime::contract_verify_gate_mode_label(gate_mode).to_string(),
                                ),
                                accepted: Some(gate_error.is_none()),
                                reason: if gate_reason.trim().is_empty() {
                                    None
                                } else {
                                    Some(gate_reason.clone())
                                },
                                checked_at: Some(mongodb::bson::DateTime::now()),
                            },
                        )
                        .await
                    {
                        tracing::warn!(
                            "Failed to persist contract verification for mission {} goal {}: {}",
                            mission_id,
                            goal.goal_id,
                            e
                        );
                    }
                    self.mission_manager
                        .broadcast(
                            mission_id,
                            StreamEvent::Status {
                                status: format!(
                                    r#"{{"type":"goal_contract_verification","goal_id":"{}","attempt":{},"gate":"{}","tool_called":{},"verify_status":"{}","accepted":{},"reason":"{}"}}"#,
                                    goal.goal_id,
                                    attempt + 1,
                                    runtime::contract_verify_gate_mode_label(gate_mode),
                                    verify_tool_called,
                                    verify_status_label,
                                    gate_error.is_none(),
                                    gate_reason.replace('"', r#"\""#).replace('\n', " ")
                                ),
                            },
                        )
                        .await;
                    if let Some(gate_err) = gate_error {
                        if attempt < max_retries {
                            tracing::warn!(
                                "Goal {} attempt {} failed contract verify gate (will retry): {}",
                                goal.goal_id,
                                attempt + 1,
                                gate_err
                            );
                            last_err = Some(anyhow!(
                                "Goal contract verification gate failed: {}",
                                gate_err
                            ));
                            continue;
                        }
                        return Err(anyhow!(
                            "Goal contract verification gate failed: {}",
                            gate_err
                        ));
                    }

                    let tokens_after = self.get_session_total_tokens(session_id).await;
                    let tokens_used = (tokens_after - tokens_before).max(0);

                    // Record attempt
                    let attempt = AttemptRecord {
                        attempt_number: goal.attempts.len() as u32 + 1,
                        approach: goal
                            .pivot_reason
                            .clone()
                            .unwrap_or_else(|| "initial".to_string()),
                        signal: ProgressSignal::Advancing, // will be updated by evaluate
                        learnings: summary.clone().unwrap_or_default(),
                        tokens_used,
                        started_at: Some(bson::DateTime::now()),
                        completed_at: Some(bson::DateTime::now()),
                    };

                    if let Err(e) = self
                        .agent_service
                        .push_goal_attempt(mission_id, &goal.goal_id, &attempt)
                        .await
                    {
                        tracing::warn!("Failed to push attempt for goal {}: {}", goal.goal_id, e);
                    }

                    if let Some(ref s) = summary {
                        if let Err(e) = self
                            .agent_service
                            .set_goal_output_summary(mission_id, &goal.goal_id, s)
                            .await
                        {
                            tracing::warn!(
                                "Failed to set output summary for goal {}: {}",
                                goal.goal_id,
                                e
                            );
                        }
                    }

                    if let Err(e) = self
                        .agent_service
                        .add_mission_tokens(mission_id, tokens_used)
                        .await
                    {
                        tracing::warn!(
                            "Failed to add mission {} tokens after goal {}: {}",
                            mission_id,
                            goal.goal_id,
                            e
                        );
                    }
                    return Ok(effective_contract);
                }
                Err(e) => {
                    if cancel_token.is_cancelled() {
                        if let Ok(Some(current)) = self.agent_service.get_mission(mission_id).await
                        {
                            if matches!(
                                current.status,
                                MissionStatus::Paused | MissionStatus::Cancelled
                            ) {
                                if let Err(err) = self
                                    .agent_service
                                    .update_goal_status(
                                        mission_id,
                                        &goal.goal_id,
                                        &GoalStatus::Pending,
                                    )
                                    .await
                                {
                                    tracing::warn!(
                                        "Failed to reset goal {} to pending for mission {} after cancel: {}",
                                        goal.goal_id,
                                        mission_id,
                                        err
                                    );
                                }
                                return Ok(runtime::MissionPreflightContract {
                                    required_artifacts: Vec::new(),
                                    completion_checks: Vec::new(),
                                    no_artifact_reason: Some(
                                        "mission paused_or_cancelled".to_string(),
                                    ),
                                });
                            }
                        }
                    }

                    let is_timeout = Self::is_timeout_error(&e);
                    let is_retryable = runtime::is_retryable_error(&e);
                    let can_retry_timeout =
                        !is_timeout || timeout_retries_used < timeout_retry_limit;
                    if is_retryable && can_retry_timeout && attempt < max_retries {
                        if is_timeout {
                            timeout_retries_used = timeout_retries_used.saturating_add(1);
                        }
                        tracing::warn!(
                            "Goal {} attempt {} failed (retryable, timeout={}, timeout_retries={}/{}): {}",
                            goal.goal_id,
                            attempt + 1,
                            is_timeout,
                            timeout_retries_used,
                            timeout_retry_limit,
                            e
                        );
                        last_err = Some(e);
                        continue;
                    }

                    let tokens_after = self.get_session_total_tokens(session_id).await;
                    let tokens_used = (tokens_after - tokens_before).max(0);
                    if let Err(err) = self
                        .agent_service
                        .add_mission_tokens(mission_id, tokens_used)
                        .await
                    {
                        tracing::warn!(
                            "Failed to add mission {} tokens after failed goal {}: {}",
                            mission_id,
                            goal.goal_id,
                            err
                        );
                    }
                    return Err(e);
                }
            }
        }

        Err(last_err.unwrap_or_else(|| anyhow!("Goal failed after retries")))
    }

    async fn get_session_total_tokens(&self, session_id: &str) -> i32 {
        self.agent_service
            .get_session(session_id)
            .await
            .ok()
            .flatten()
            .and_then(|s| s.total_tokens)
            .unwrap_or(0)
    }

    fn env_u64(name: &str) -> Option<u64> {
        std::env::var(name)
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .filter(|v| *v > 0)
    }

    fn env_u32(name: &str) -> Option<u32> {
        std::env::var(name)
            .ok()
            .and_then(|v| v.parse::<u32>().ok())
            .filter(|v| *v > 0)
    }

    fn planning_timeout() -> Duration {
        let secs = Self::env_u64("TEAM_MISSION_PLANNING_TIMEOUT_SECS")
            .unwrap_or(DEFAULT_MISSION_PLANNING_TIMEOUT_SECS)
            .min(MAX_MISSION_PLANNING_TIMEOUT_SECS);
        Duration::from_secs(secs)
    }

    fn planning_timeout_cancel_grace() -> Duration {
        let secs = Self::env_u64("TEAM_MISSION_PLANNING_CANCEL_GRACE_SECS")
            .unwrap_or(DEFAULT_PLANNING_TIMEOUT_CANCEL_GRACE_SECS)
            .min(MAX_PLANNING_TIMEOUT_CANCEL_GRACE_SECS);
        Duration::from_secs(secs)
    }

    fn clamp_goal_timeout_secs(timeout_secs: u64) -> u64 {
        timeout_secs.clamp(1, MAX_GOAL_EXECUTION_TIMEOUT_SECS)
    }

    fn resolve_min_goal_timeout_secs() -> u64 {
        Self::env_u64("TEAM_MISSION_MIN_GOAL_TIMEOUT_SECS")
            .unwrap_or(DEFAULT_MIN_GOAL_EXECUTION_TIMEOUT_SECS)
            .clamp(1, MAX_GOAL_EXECUTION_TIMEOUT_SECS)
    }

    fn resolve_goal_timeout(mission_step_timeout_seconds: Option<u64>) -> Duration {
        let configured_secs = mission_step_timeout_seconds
            .or_else(|| Self::env_u64("TEAM_MISSION_STEP_TIMEOUT_SECS"))
            .unwrap_or(DEFAULT_GOAL_EXECUTION_TIMEOUT_SECS);
        let clamped_secs = Self::clamp_goal_timeout_secs(configured_secs);
        let min_goal_secs = Self::resolve_min_goal_timeout_secs();
        Duration::from_secs(clamped_secs.max(min_goal_secs))
    }

    fn resolve_goal_max_retries(mission_step_max_retries: Option<u32>) -> u32 {
        mission_step_max_retries
            .or_else(|| Self::env_u32("TEAM_MISSION_DEFAULT_RETRIES"))
            .unwrap_or(2)
            .min(MAX_GOAL_RETRY_LIMIT)
    }

    fn goal_timeout_cancel_grace() -> Duration {
        let secs = Self::env_u64("TEAM_MISSION_TIMEOUT_CANCEL_GRACE_SECS")
            .unwrap_or(DEFAULT_GOAL_TIMEOUT_CANCEL_GRACE_SECS)
            .min(MAX_GOAL_TIMEOUT_CANCEL_GRACE_SECS);
        Duration::from_secs(secs)
    }

    fn goal_timeout_retry_limit() -> u32 {
        Self::env_u32("TEAM_MISSION_TIMEOUT_RETRY_LIMIT")
            .unwrap_or(DEFAULT_GOAL_TIMEOUT_RETRY_LIMIT)
            .min(MAX_GOAL_RETRY_LIMIT)
    }

    fn is_timeout_error(e: &anyhow::Error) -> bool {
        let msg = e.to_string().to_ascii_lowercase();
        msg.contains("timed out") || msg.contains("timeout")
    }

    /// Build prompt for executing a single goal.
    fn build_goal_prompt(
        goal: &GoalNode,
        completed_goals: &[&GoalNode],
        workspace_path: Option<&str>,
        operator_hint: Option<&str>,
    ) -> String {
        let mut prompt = format!(
            "## Goal: {}\n{}\n\n## Success Criteria\n{}\n",
            goal.title, goal.description, goal.success_criteria
        );

        if !completed_goals.is_empty() {
            prompt.push_str("\n## Completed Related Goals\n");
            for cg in completed_goals {
                let full = cg.output_summary.as_deref().unwrap_or("(no summary)");
                let summary = if full.chars().count() > 300 {
                    let truncated: String = full.chars().take(297).collect();
                    format!("{}...", truncated)
                } else {
                    full.to_string()
                };
                prompt.push_str(&format!(
                    "- Goal {}: {} → {}\n",
                    cg.goal_id, cg.title, summary
                ));
            }
        }

        if !goal.attempts.is_empty() {
            prompt.push_str("\n## Previous Attempts\n");
            for a in &goal.attempts {
                prompt.push_str(&format!(
                    "- Attempt {} ({}): {}\n",
                    a.attempt_number, a.approach, a.learnings
                ));
            }
        }

        if let Some(hint) = operator_hint.map(str::trim).filter(|h| !h.is_empty()) {
            prompt.push_str("\n## Operator Guidance (Highest Priority)\n");
            prompt.push_str(hint);
            prompt.push('\n');
        }

        prompt.push_str("\n## Mandatory Preflight Gate (Must Run First)\n");
        prompt.push_str(&format!(
            "- Before any other tool call, you MUST call `{}`.\n",
            MISSION_PREFLIGHT_TOOL_NAME
        ));
        prompt.push_str("- If preflight is skipped, this goal attempt will be retried.\n");
        prompt.push_str("- In preflight, you MUST declare a contract: `required_artifacts` and/or `completion_checks`; for non-file outcomes, provide `no_artifact_reason`.\n");
        let preflight_goal_title = Self::escape_json_for_prompt(&goal.title);
        let preflight_goal_desc = Self::escape_json_for_prompt(&goal.description);
        let preflight_workspace = Self::escape_json_for_prompt(workspace_path.unwrap_or_default());
        prompt.push_str("```json\n");
        prompt.push_str("{\n");
        prompt.push_str(&format!(
            "  \"step_title\": \"{}\",\n",
            preflight_goal_title
        ));
        prompt.push_str(&format!("  \"step_goal\": \"{}\",\n", preflight_goal_desc));
        prompt.push_str(&format!(
            "  \"workspace_path\": \"{}\",\n",
            preflight_workspace
        ));
        prompt.push_str("  \"required_artifacts\": [],\n");
        prompt.push_str("  \"completion_checks\": [],\n");
        prompt.push_str("  \"no_artifact_reason\": \"\",\n");
        prompt.push_str("  \"attempt\": 1,\n");
        prompt.push_str("  \"last_error\": \"\"\n");
        prompt.push_str("}\n");
        prompt.push_str("```\n");
        prompt.push_str("- Optional but recommended: call `mission_preflight__workspace_overview` to inspect current workspace before execution.\n");
        prompt.push_str("- Before final completion response, call `mission_preflight__verify_contract` with your final contract to self-verify outputs.\n");
        if runtime::contract_verify_gate_mode() == runtime::ContractVerifyGateMode::Hard {
            prompt.push_str("- HARD GATE ENABLED: calling `mission_preflight__verify_contract` and getting `status=pass` is mandatory before completion.\n");
        }

        prompt.push_str("\nExecute this goal. Focus on meeting the success criteria.");
        prompt
    }

    fn to_runtime_contract_doc(contract: &runtime::MissionPreflightContract) -> RuntimeContract {
        RuntimeContract {
            required_artifacts: contract.required_artifacts.clone(),
            completion_checks: contract.completion_checks.clone(),
            no_artifact_reason: contract.no_artifact_reason.clone(),
            source: Some(MISSION_PREFLIGHT_TOOL_NAME.to_string()),
            captured_at: Some(mongodb::bson::DateTime::now()),
        }
    }

    fn escape_json_for_prompt(input: &str) -> String {
        input
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('\n', "\\n")
            .replace('\r', "")
    }

    /// Extract the full output text from the last assistant message.
    /// Saved as-is for debugging; truncated only when injected into prompts.
    async fn extract_step_summary(&self, session_id: &str) -> Option<String> {
        let session = match self.agent_service.get_session(session_id).await {
            Ok(s) => s?,
            Err(e) => {
                tracing::warn!("Failed to get session {} for summary: {}", session_id, e);
                return None;
            }
        };
        runtime::extract_last_assistant_text(&session.messages_json).filter(|t| !t.is_empty())
    }

    /// Evaluate whether a goal has been achieved.
    async fn evaluate_goal(
        &self,
        mission_id: &str,
        agent_id: &str,
        session_id: &str,
        goal: &GoalNode,
        cancel_token: CancellationToken,
        workspace_path: Option<&str>,
    ) -> Result<ProgressSignal> {
        let prompt = format!(
            r#"Evaluate whether the goal you just executed has been achieved.

## Goal: {}
## Success Criteria: {}

Assess:
- advancing: Success criteria met or substantial progress made
- stalled: Some progress but criteria not met, may need a different approach
- blocked: Encountered insurmountable obstacle, current direction is not viable

Output JSON only: {{"signal": "advancing|stalled|blocked", "reasoning": "...", "learnings": "..."}}"#,
            goal.title, goal.success_criteria
        );

        self.execute_via_bridge(
            agent_id,
            session_id,
            mission_id,
            &prompt,
            cancel_token,
            workspace_path,
            None,
        )
        .await?;

        // Parse response
        let session = self
            .agent_service
            .get_session(session_id)
            .await
            .map_err(|e| anyhow!("DB error: {}", e))?
            .ok_or_else(|| anyhow!("Session not found"))?;

        let text = runtime::extract_last_assistant_text(&session.messages_json).unwrap_or_default();
        let json_str = runtime::extract_json_block(&text);

        // Parse signal
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(&json_str) {
            let signal_str = val
                .get("signal")
                .and_then(|s| s.as_str())
                .unwrap_or("stalled");
            match signal_str {
                "advancing" => Ok(ProgressSignal::Advancing),
                "blocked" => Ok(ProgressSignal::Blocked),
                _ => Ok(ProgressSignal::Stalled),
            }
        } else {
            // Default to stalled if parse fails — safer than assuming success
            Ok(ProgressSignal::Stalled)
        }
    }

    /// Mark a goal as completed.
    async fn complete_goal(
        &self,
        mission_id: &str,
        goal: &GoalNode,
        step_index: u32,
        contract: &runtime::MissionPreflightContract,
        workspace_path: Option<&str>,
        before: Option<&runtime::WorkspaceSnapshot>,
    ) -> Result<()> {
        if let Err(e) = self
            .agent_service
            .update_goal_status(mission_id, &goal.goal_id, &GoalStatus::Completed)
            .await
        {
            tracing::warn!("Failed to complete goal {}: {}", goal.goal_id, e);
        }

        self.mission_manager
            .broadcast(
                mission_id,
                StreamEvent::GoalComplete {
                    goal_id: goal.goal_id.clone(),
                    signal: "advancing".to_string(),
                },
            )
            .await;

        if let Some(wp) = workspace_path {
            if let Err(e) = self
                .register_goal_artifacts(
                    mission_id,
                    goal,
                    step_index,
                    &contract.required_artifacts,
                    wp,
                    before,
                )
                .await
            {
                tracing::warn!(
                    "Artifact scan failed for mission {} goal {}: {}",
                    mission_id,
                    goal.goal_id,
                    e
                );
            }
        }

        Ok(())
    }

    async fn register_goal_artifacts(
        &self,
        mission_id: &str,
        _goal: &GoalNode,
        step_index: u32,
        required_artifacts: &[String],
        workspace_path: &str,
        before: Option<&runtime::WorkspaceSnapshot>,
    ) -> Result<()> {
        runtime::save_scanned_artifacts(
            &self.agent_service,
            mission_id,
            step_index,
            workspace_path,
            before,
            Some(required_artifacts),
        )
        .await?;
        runtime::save_required_artifacts(
            &self.agent_service,
            mission_id,
            step_index,
            workspace_path,
            required_artifacts,
        )
        .await
    }

    /// Handle pivot decision for a stalled/blocked goal.
    #[allow(clippy::too_many_arguments)]
    async fn handle_pivot(
        &self,
        mission_id: &str,
        agent_id: &str,
        session_id: &str,
        goal: &GoalNode,
        signal: &ProgressSignal,
        cancel_token: CancellationToken,
        workspace_path: Option<&str>,
    ) -> Result<()> {
        let decision = self
            .pivot_protocol(
                mission_id,
                agent_id,
                session_id,
                goal,
                signal,
                cancel_token,
                workspace_path,
            )
            .await?;

        match decision {
            PivotDecision::Retry { approach } => {
                if let Err(e) = self
                    .agent_service
                    .pivot_goal_atomic(mission_id, &goal.goal_id, &approach)
                    .await
                {
                    tracing::warn!("Failed to pivot goal {}: {}", goal.goal_id, e);
                }

                let from = goal
                    .pivot_reason
                    .clone()
                    .unwrap_or_else(|| "initial".to_string());

                let last_learnings = goal
                    .attempts
                    .last()
                    .map(|a| a.learnings.clone())
                    .unwrap_or_default();

                self.mission_manager
                    .broadcast(
                        mission_id,
                        StreamEvent::Pivot {
                            goal_id: goal.goal_id.clone(),
                            from_approach: from,
                            to_approach: approach,
                            learnings: last_learnings,
                        },
                    )
                    .await;
            }
            PivotDecision::Abandon { reason } => {
                if let Err(e) = self
                    .agent_service
                    .abandon_goal_atomic(mission_id, &goal.goal_id, &reason)
                    .await
                {
                    tracing::warn!("Failed to abandon goal {}: {}", goal.goal_id, e);
                }

                self.mission_manager
                    .broadcast(
                        mission_id,
                        StreamEvent::GoalAbandoned {
                            goal_id: goal.goal_id.clone(),
                            reason,
                        },
                    )
                    .await;
            }
        }

        Ok(())
    }

    /// Pivot protocol — decide whether to retry with new approach or abandon.
    #[allow(clippy::too_many_arguments)]
    async fn pivot_protocol(
        &self,
        mission_id: &str,
        agent_id: &str,
        session_id: &str,
        goal: &GoalNode,
        _signal: &ProgressSignal,
        cancel_token: CancellationToken,
        workspace_path: Option<&str>,
    ) -> Result<PivotDecision> {
        // Force abandon if budget exhausted
        let attempt_count = goal.attempts.len() as u32;
        if attempt_count >= goal.exploration_budget {
            return Ok(PivotDecision::Abandon {
                reason: format!(
                    "Exploration budget exhausted ({}/{} attempts)",
                    attempt_count, goal.exploration_budget
                ),
            });
        }

        // Force abandon if per-goal pivot limit exceeded
        let goal_pivots = goal
            .attempts
            .iter()
            .filter(|a| a.approach != "initial")
            .count() as u32;
        if goal_pivots >= MAX_PIVOTS_PER_GOAL {
            return Ok(PivotDecision::Abandon {
                reason: format!(
                    "Per-goal pivot limit reached ({}/{} pivots)",
                    goal_pivots, MAX_PIVOTS_PER_GOAL
                ),
            });
        }

        // Force abandon if total pivots exceeded
        let mission = self
            .agent_service
            .get_mission(mission_id)
            .await
            .map_err(|e| anyhow!("DB error: {}", e))?
            .ok_or_else(|| anyhow!("Mission not found"))?;

        if mission.total_pivots >= MAX_TOTAL_PIVOTS {
            return Ok(PivotDecision::Abandon {
                reason: format!(
                    "Total pivot limit reached ({}/{})",
                    mission.total_pivots, MAX_TOTAL_PIVOTS
                ),
            });
        }

        // Build pivot prompt
        let mut attempts_desc = String::new();
        for a in &goal.attempts {
            attempts_desc.push_str(&format!(
                "- Approach: {} → Result: {}\n",
                a.approach, a.learnings
            ));
        }

        let prompt = format!(
            r#"Goal "{}" has encountered an obstacle with the current approach.

## Attempted Approaches
{}

## Decision
Choose one:
1. Propose a new alternative approach (different from those already attempted)
2. If this goal is truly infeasible, recommend abandoning it

Output JSON:
- Alternative approach: {{"decision": "retry", "approach": "new approach description", "rationale": "why it could work"}}
- Abandon: {{"decision": "abandon", "reason": "reason for abandoning"}}"#,
            goal.title, attempts_desc
        );

        self.execute_via_bridge(
            agent_id,
            session_id,
            mission_id,
            &prompt,
            cancel_token,
            workspace_path,
            None,
        )
        .await?;

        // Parse response
        let session = self
            .agent_service
            .get_session(session_id)
            .await
            .map_err(|e| anyhow!("DB error: {}", e))?
            .ok_or_else(|| anyhow!("Session not found"))?;

        let text = runtime::extract_last_assistant_text(&session.messages_json).unwrap_or_default();
        let json_str = runtime::extract_json_block(&text);

        if let Ok(val) = serde_json::from_str::<serde_json::Value>(&json_str) {
            let decision = val
                .get("decision")
                .and_then(|d| d.as_str())
                .unwrap_or("abandon");

            if decision == "retry" {
                let approach = val
                    .get("approach")
                    .and_then(|a| a.as_str())
                    .unwrap_or("alternative approach")
                    .to_string();
                Ok(PivotDecision::Retry { approach })
            } else {
                let reason = val
                    .get("reason")
                    .and_then(|r| r.as_str())
                    .unwrap_or("Goal deemed infeasible")
                    .to_string();
                Ok(PivotDecision::Abandon { reason })
            }
        } else {
            Ok(PivotDecision::Abandon {
                reason: "Failed to parse pivot decision".to_string(),
            })
        }
    }

    /// Synthesize final results from all completed/abandoned goals.
    async fn synthesize_results(
        &self,
        mission_id: &str,
        agent_id: &str,
        session_id: &str,
        cancel_token: CancellationToken,
        workspace_path: Option<&str>,
    ) -> Result<()> {
        let mission = self
            .agent_service
            .get_mission(mission_id)
            .await
            .map_err(|e| anyhow!("DB error: {}", e))?
            .ok_or_else(|| anyhow!("Mission not found"))?;

        let goals = mission.goal_tree.as_deref().unwrap_or(&[]);
        if goals.is_empty() {
            return Ok(());
        }

        let mut summary_parts = String::new();
        for g in goals {
            let status_label = match g.status {
                GoalStatus::Completed => "completed",
                GoalStatus::Abandoned => "abandoned",
                _ => "other",
            };
            let output = g.output_summary.as_deref().unwrap_or("(no output)");
            let truncated_output = if output.chars().count() > 500 {
                let t: String = output.chars().take(497).collect();
                format!("{}...", t)
            } else {
                output.to_string()
            };
            summary_parts.push_str(&format!(
                "- {} [{}]: {}\n",
                g.title, status_label, truncated_output
            ));
        }

        let prompt = format!(
            "All goals have been processed. Please synthesize the final results.\n\n\
             ## Goal Execution Results\n{}\n\n\
             Provide a concise final summary including key achievements and any incomplete parts.",
            summary_parts
        );

        // Best-effort synthesis; failure is non-fatal
        let synthesis_ok = if let Err(e) = self
            .execute_via_bridge(
                agent_id,
                session_id,
                mission_id,
                &prompt,
                cancel_token,
                workspace_path,
                None,
            )
            .await
        {
            tracing::warn!("Mission {} synthesis failed: {}", mission_id, e);
            false
        } else {
            true
        };

        if synthesis_ok {
            if let Some(summary) = self.extract_step_summary(session_id).await {
                if let Err(e) = self
                    .agent_service
                    .set_mission_final_summary(mission_id, &summary)
                    .await
                {
                    tracing::warn!("Failed to save mission {} final summary: {}", mission_id, e);
                }
            }
        }

        Ok(())
    }

    /// Resume a paused adaptive mission (with guaranteed cleanup).
    /// NOTE: resume_mission in MissionExecutor has no cleanup wrapper,
    /// so we must handle Done broadcast + mission_manager.complete here.
    pub async fn resume_adaptive(
        &self,
        mission_id: &str,
        cancel_token: CancellationToken,
        resume_feedback: Option<String>,
    ) -> Result<()> {
        let result = self
            .resume_adaptive_inner(mission_id, cancel_token, resume_feedback)
            .await;

        // Read actual mission status from DB to determine the correct Done event
        // (handles re-pause at checkpoint, completed, cancelled, etc.)
        match &result {
            Ok(_) => {
                let done_status = match self.agent_service.get_mission(mission_id).await {
                    Ok(Some(m)) => match m.status {
                        MissionStatus::Paused => "paused",
                        MissionStatus::Completed => "completed",
                        MissionStatus::Cancelled => "cancelled",
                        _ => "completed",
                    },
                    Ok(None) => {
                        tracing::warn!("Mission {} not found during cleanup", mission_id);
                        "completed"
                    }
                    Err(e) => {
                        tracing::warn!(
                            "Failed to read mission {} status during cleanup: {}",
                            mission_id,
                            e
                        );
                        "completed"
                    }
                };

                self.mission_manager
                    .broadcast(
                        mission_id,
                        StreamEvent::Done {
                            status: done_status.to_string(),
                            error: None,
                        },
                    )
                    .await;
            }
            Err(e) => {
                let mut done_status = "failed";
                let mut done_error = Some(e.to_string());
                let mut should_persist_failure = true;

                if let Ok(Some(mission)) = self.agent_service.get_mission(mission_id).await {
                    match mission.status {
                        MissionStatus::Paused => {
                            done_status = "paused";
                            done_error = None;
                            should_persist_failure = false;
                        }
                        MissionStatus::Cancelled => {
                            done_status = "cancelled";
                            done_error = None;
                            should_persist_failure = false;
                        }
                        _ => {}
                    }
                }

                if should_persist_failure {
                    self.persist_failure_state(mission_id, &e.to_string()).await;
                }
                self.mission_manager
                    .broadcast(
                        mission_id,
                        StreamEvent::Done {
                            status: done_status.to_string(),
                            error: done_error,
                        },
                    )
                    .await;
            }
        }

        self.mission_manager.complete(mission_id).await;
        result
    }

    async fn persist_failure_state(&self, mission_id: &str, error_message: &str) {
        if let Err(e) = self
            .agent_service
            .update_mission_status(mission_id, &MissionStatus::Failed)
            .await
        {
            tracing::warn!(
                "Failed to mark mission {} as failed during adaptive cleanup: {}",
                mission_id,
                e
            );
        }

        if let Err(e) = self
            .agent_service
            .set_mission_error(mission_id, error_message)
            .await
        {
            tracing::warn!(
                "Failed to persist mission {} error message during adaptive cleanup: {}",
                mission_id,
                e
            );
        }
    }

    async fn resume_adaptive_inner(
        &self,
        mission_id: &str,
        cancel_token: CancellationToken,
        resume_feedback: Option<String>,
    ) -> Result<()> {
        let mission = self
            .agent_service
            .get_mission(mission_id)
            .await
            .map_err(|e| anyhow!("DB error: {}", e))?
            .ok_or_else(|| anyhow!("Mission not found"))?;

        if !matches!(
            mission.status,
            MissionStatus::Paused | MissionStatus::Failed
        ) {
            return Err(anyhow!("Mission is not paused/failed"));
        }

        let session_id = mission
            .session_id
            .as_deref()
            .ok_or_else(|| anyhow!("Mission has no session"))?
            .to_string();

        // Read workspace_path from mission doc (set during initial execution)
        let workspace_path = mission.workspace_path.clone();

        if mission.status == MissionStatus::Failed {
            if let Err(e) = self.agent_service.clear_mission_error(mission_id).await {
                tracing::warn!(
                    "Failed to clear mission {} error before adaptive resume: {}",
                    mission_id,
                    e
                );
            }
        }
        if let Some(goals) = mission.goal_tree.as_ref() {
            for goal in goals {
                let should_reset = if mission.status == MissionStatus::Failed {
                    matches!(goal.status, GoalStatus::Failed | GoalStatus::Running)
                } else {
                    // Mission paused: clean up stale running goal left by interrupted pause flow.
                    matches!(goal.status, GoalStatus::Running)
                };
                if !should_reset {
                    continue;
                }
                if let Err(e) = self
                    .agent_service
                    .reset_goal_for_retry(mission_id, &goal.goal_id)
                    .await
                {
                    tracing::warn!(
                        "Failed to reset mission {} goal {} for retry: {}",
                        mission_id,
                        goal.goal_id,
                        e
                    );
                }
            }
        }

        // Update status to Running
        if let Err(e) = self
            .agent_service
            .update_mission_status(mission_id, &MissionStatus::Running)
            .await
        {
            tracing::warn!(
                "Failed to set mission {} running on resume: {}",
                mission_id,
                e
            );
        }

        // Resume goal loop (skips completed/abandoned goals automatically)
        self.execute_goal_loop(
            mission_id,
            &mission.agent_id,
            &session_id,
            cancel_token.clone(),
            workspace_path.as_deref(),
            resume_feedback.as_deref(),
        )
        .await?;

        // Check terminal/pause states — don't synthesize in these cases.
        let current = self
            .agent_service
            .get_mission(mission_id)
            .await
            .map_err(|e| anyhow!("DB error: {}", e))?;
        if let Some(m) = current.as_ref() {
            if matches!(
                m.status,
                MissionStatus::Paused
                    | MissionStatus::Cancelled
                    | MissionStatus::Failed
                    | MissionStatus::Completed
            ) {
                return Ok(());
            }
        }

        // Synthesize results
        self.synthesize_results(
            mission_id,
            &mission.agent_id,
            &session_id,
            cancel_token,
            workspace_path.as_deref(),
        )
        .await?;

        if let Err(e) = self
            .agent_service
            .update_mission_status(mission_id, &MissionStatus::Completed)
            .await
        {
            tracing::warn!(
                "Failed to mark mission {} completed on resume: {}",
                mission_id,
                e
            );
        }

        Ok(())
    }
}
