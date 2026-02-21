//! Adaptive Goal Execution (AGE) engine for mission execution.
//!
//! Implements goal-tree based execution with progress evaluation
//! and pivot protocol. Reuses runtime::execute_via_bridge and
//! MissionManager infrastructure.

use agime_team::MongoDb;
use anyhow::{anyhow, Result};

use std::sync::Arc;
use tokio_util::sync::CancellationToken;

use super::mission_manager::MissionManager;
use super::mission_mongo::*;
use super::runtime;
use super::service_mongo::AgentService;
use super::task_manager::{StreamEvent, TaskManager};

const MAX_PIVOTS_PER_GOAL: u32 = 3;
const MAX_TOTAL_PIVOTS: u32 = 15;

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
        self.execute_adaptive_inner(mission_id, cancel_token).await
    }

    /// Inner execution logic for adaptive mission lifecycle.
    async fn execute_adaptive_inner(
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
                Vec::new(),
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                false,
                false,
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

        let goals = self
            .decompose_goal(
                mission_id,
                mission,
                &session_id,
                cancel_token,
                workspace_path,
            )
            .await?;

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
        )
        .await?;

        // Check if mission was paused (checkpoint goal) — don't synthesize
        let current = self
            .agent_service
            .get_mission(mission_id)
            .await
            .map_err(|e| anyhow!("DB error: {}", e))?;
        if current.as_ref().map(|m| &m.status) == Some(&MissionStatus::Paused) {
            return Ok(());
        }

        // Convergence — synthesize results
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

        let text = runtime::extract_last_assistant_text(&session.messages_json)
            .ok_or_else(|| anyhow!("No assistant response for goal decomposition"))?;

        let json_str = runtime::extract_json_block(&text);
        self.parse_goal_tree_json(&json_str)
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

        let raw: Vec<RawGoal> = serde_json::from_str(json_str)
            .map_err(|e| anyhow!("Failed to parse goal tree JSON: {}", e))?;

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
                    pivot_reason: None,
                    is_checkpoint: r.is_checkpoint,
                    created_at: Some(bson::DateTime::now()),
                    completed_at: None,
                }
            })
            .collect();

        Ok(goals)
    }

    /// Core execution loop — iterates over goal tree using state machine pattern.
    async fn execute_goal_loop(
        &self,
        mission_id: &str,
        agent_id: &str,
        session_id: &str,
        cancel_token: CancellationToken,
        workspace_path: Option<&str>,
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
                        self.agent_service
                            .update_mission_status(mission_id, &MissionStatus::Cancelled)
                            .await
                            .ok();
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

            // 5. Check approval for checkpoint goals
            if goal.is_checkpoint && goal.status == GoalStatus::Pending {
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
                                r#"{{"type":"mission_paused","goal_id":"{}","reason":"checkpoint"}}"#,
                                goal.goal_id
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

            let policy_str = match mission.approval_policy {
                ApprovalPolicy::Auto => "auto",
                ApprovalPolicy::Checkpoint => "checkpoint",
                ApprovalPolicy::Manual => "manual",
            };

            // 7. Execute goal
            self.run_single_goal(
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
            )
            .await?;

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
                    self.complete_goal(mission_id, &goal, session_id).await?;
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

        candidates.into_iter().next()
    }

    /// Execute a single goal via bridge.
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
    ) -> Result<()> {
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
        let prompt = Self::build_goal_prompt(goal, completed_goals);

        // Execute via bridge with mission context
        let mc_json = serde_json::json!({
            "goal": goal.title,
            "approval_policy": approval_policy,
            "total_steps": total_steps,
            "current_step": current_step,
        });
        self.execute_via_bridge(
            agent_id,
            session_id,
            mission_id,
            &prompt,
            cancel_token,
            workspace_path,
            Some(mc_json),
        )
        .await?;

        // Extract summary and record attempt
        let summary = self.extract_step_summary(session_id).await;
        let attempt = AttemptRecord {
            attempt_number: goal.attempts.len() as u32 + 1,
            approach: goal
                .pivot_reason
                .clone()
                .unwrap_or_else(|| "initial".to_string()),
            signal: ProgressSignal::Advancing, // will be updated by evaluate
            learnings: summary.clone().unwrap_or_default(),
            tokens_used: 0,
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

        Ok(())
    }

    /// Build prompt for executing a single goal.
    fn build_goal_prompt(goal: &GoalNode, completed_goals: &[&GoalNode]) -> String {
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

        prompt.push_str("\nExecute this goal. Focus on meeting the success criteria.");
        prompt
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
        let text = runtime::extract_last_assistant_text(&session.messages_json)?;

        if text.is_empty() {
            None
        } else {
            Some(text)
        }
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
        _session_id: &str,
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

        Ok(())
    }

    /// Handle pivot decision for a stalled/blocked goal.
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
            attempts_desc.push_str(&format!("- Approach: {} → Result: {}\n", a.approach, a.learnings));
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
        if let Err(e) = self
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
    ) -> Result<()> {
        let result = self.resume_adaptive_inner(mission_id, cancel_token).await;

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
                self.mission_manager
                    .broadcast(
                        mission_id,
                        StreamEvent::Done {
                            status: "failed".to_string(),
                            error: Some(e.to_string()),
                        },
                    )
                    .await;
            }
        }

        self.mission_manager.complete(mission_id).await;
        result
    }

    async fn resume_adaptive_inner(
        &self,
        mission_id: &str,
        cancel_token: CancellationToken,
    ) -> Result<()> {
        let mission = self
            .agent_service
            .get_mission(mission_id)
            .await
            .map_err(|e| anyhow!("DB error: {}", e))?
            .ok_or_else(|| anyhow!("Mission not found"))?;

        if mission.status != MissionStatus::Paused {
            return Err(anyhow!("Mission is not paused"));
        }

        let session_id = mission
            .session_id
            .as_deref()
            .ok_or_else(|| anyhow!("Mission has no session"))?
            .to_string();

        // Read workspace_path from mission doc (set during initial execution)
        let workspace_path = mission.workspace_path.clone();

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
        )
        .await?;

        // Check if mission was re-paused (checkpoint goal) — don't synthesize
        let current = self
            .agent_service
            .get_mission(mission_id)
            .await
            .map_err(|e| anyhow!("DB error: {}", e))?;
        if current.as_ref().map(|m| &m.status) == Some(&MissionStatus::Paused) {
            return Ok(());
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
