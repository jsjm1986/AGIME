//! Mission executor for multi-step autonomous task execution (Phase 2)
//!
//! MissionExecutor orchestrates mission lifecycle:
//! 1. Create dedicated AgentSession for cross-step context
//! 2. Generate execution plan via Agent (Planning phase)
//! 3. Execute steps sequentially, bridging to TaskExecutor
//! 4. Handle checkpoints, approvals, and cancellation
//! 5. Track token budget and artifacts

use agime_team::MongoDb;
use anyhow::{anyhow, Result};

use std::sync::Arc;
use std::time::Duration;
use tokio_util::sync::CancellationToken;

use super::adaptive_executor::AdaptiveExecutor;
use super::mission_manager::MissionManager;
use super::mission_mongo::{
    ApprovalPolicy, ExecutionMode, MissionDoc, MissionStatus, MissionStep, StepStatus,
};
use super::runtime;
use super::service_mongo::AgentService;
use super::task_manager::{StreamEvent, TaskManager};

/// Maximum number of re-plan evaluations per mission execution.
const MAX_REPLAN_COUNT: u32 = 5;

/// Timeout for a single step execution (10 minutes).
const STEP_EXECUTION_TIMEOUT: Duration = Duration::from_secs(600);

/// Mission executor that orchestrates multi-step task execution
pub struct MissionExecutor {
    db: Arc<MongoDb>,
    mission_manager: Arc<MissionManager>,
    agent_service: Arc<AgentService>,
    internal_task_manager: Arc<TaskManager>,
    workspace_root: String,
}

impl MissionExecutor {
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

    /// Execute a mission (outer method with guaranteed cleanup).
    pub async fn execute_mission(
        &self,
        mission_id: &str,
        cancel_token: CancellationToken,
    ) -> Result<()> {
        let exec_result = self.execute_mission_inner(mission_id, cancel_token).await;

        // Guaranteed cleanup: send done event and complete in manager.
        // Read actual mission status to determine the correct Done event
        // (handles Planned pause, checkpoint Paused, Completed, etc.)
        match &exec_result {
            Ok(_) => {
                let done_status = self
                    .agent_service
                    .get_mission(mission_id)
                    .await
                    .ok()
                    .flatten()
                    .map(|m| match m.status {
                        MissionStatus::Planned => "planned",
                        MissionStatus::Paused => "paused",
                        MissionStatus::Completed => "completed",
                        MissionStatus::Cancelled => "cancelled",
                        _ => "completed",
                    })
                    .unwrap_or("completed");

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
        exec_result
    }

    /// Inner execution logic for mission lifecycle.
    async fn execute_mission_inner(
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

        // Dispatch to AdaptiveExecutor for adaptive mode
        if mission.execution_mode == ExecutionMode::Adaptive {
            let adaptive = AdaptiveExecutor::new(
                self.db.clone(),
                self.mission_manager.clone(),
                self.workspace_root.clone(),
            );
            return adaptive.execute_adaptive(mission_id, cancel_token).await;
        }

        if mission.status != MissionStatus::Draft && mission.status != MissionStatus::Planned {
            return Err(anyhow!(
                "Mission must be in Draft or Planned status to start"
            ));
        }

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

        // 2. Create dedicated AgentSession (with mission's attached documents)
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

        // 3. Planning phase
        self.agent_service
            .update_mission_status(mission_id, &MissionStatus::Planning)
            .await
            .map_err(|e| anyhow!("Failed to update status: {}", e))?;

        self.mission_manager
            .broadcast(
                mission_id,
                StreamEvent::Status {
                    status: r#"{"type":"mission_planning"}"#.to_string(),
                },
            )
            .await;

        // 4. Generate plan
        let steps = self
            .generate_plan(
                mission_id,
                &mission,
                &session_id,
                cancel_token.clone(),
                Some(&workspace_path),
            )
            .await?;

        if steps.is_empty() {
            return Err(anyhow!("Agent generated empty plan"));
        }

        // 5. Save steps
        self.agent_service
            .save_mission_plan(mission_id, steps.clone())
            .await
            .map_err(|e| anyhow!("Failed to save plan: {}", e))?;

        // Check cancellation — return Ok so outer cleanup reads actual DB status
        if cancel_token.is_cancelled() {
            // Only set Cancelled if not already Paused (pause route sets Paused before cancelling token)
            if let Ok(Some(current)) = self.agent_service.get_mission(mission_id).await {
                if current.status != MissionStatus::Paused {
                    self.agent_service
                        .update_mission_status(mission_id, &MissionStatus::Cancelled)
                        .await
                        .ok();
                }
            }
            return Ok(());
        }

        // 6. Running phase - execute steps
        self.agent_service
            .update_mission_status(mission_id, &MissionStatus::Running)
            .await
            .map_err(|e| anyhow!("Failed to update status: {}", e))?;

        self.execute_steps(
            mission_id,
            &mission,
            &session_id,
            steps,
            vec![],
            cancel_token,
            Some(&workspace_path),
        )
        .await
    }

    /// Execute steps sequentially with checkpoint/approval handling.
    /// Tracks completed steps for structured context passing (P0),
    /// evaluates re-planning after checkpoint steps (P1),
    /// and supports dynamic step replacement mid-execution.
    async fn execute_steps(
        &self,
        mission_id: &str,
        mission: &MissionDoc,
        session_id: &str,
        steps: Vec<MissionStep>,
        prior_completed: Vec<MissionStep>,
        cancel_token: CancellationToken,
        workspace_path: Option<&str>,
    ) -> Result<()> {
        let mut current_steps = steps;
        let mut completed_steps: Vec<MissionStep> = prior_completed;
        let mut replan_count: u32 = 0;
        let mut i = 0;
        let mut total_steps = completed_steps.len() + current_steps.len();

        while i < current_steps.len() {
            let step = &current_steps[i];
            let idx = step.index;
            let total = total_steps;

            // Check cancellation — return Ok so outer cleanup reads actual DB status
            if cancel_token.is_cancelled() {
                // Only set Cancelled if not already Paused
                if let Ok(Some(current)) = self.agent_service.get_mission(mission_id).await {
                    if current.status != MissionStatus::Paused {
                        self.agent_service
                            .update_mission_status(mission_id, &MissionStatus::Cancelled)
                            .await
                            .ok();
                    }
                }
                return Ok(());
            }

            // Check token budget
            if mission.token_budget > 0 {
                let m = self
                    .agent_service
                    .get_mission(mission_id)
                    .await
                    .ok()
                    .flatten();
                if let Some(m) = m {
                    if m.total_tokens_used >= mission.token_budget {
                        self.agent_service
                            .update_mission_status(mission_id, &MissionStatus::Failed)
                            .await
                            .ok();
                        return Err(anyhow!("Token budget exceeded"));
                    }
                }
            }

            // Check if approval is needed
            let needs_approval = match mission.approval_policy {
                ApprovalPolicy::Manual => true,
                ApprovalPolicy::Checkpoint => step.is_checkpoint,
                ApprovalPolicy::Auto => false,
            };

            if needs_approval {
                // Pause for approval
                self.agent_service
                    .update_step_status(mission_id, idx, &StepStatus::AwaitingApproval)
                    .await
                    .ok();
                self.agent_service
                    .update_mission_status(mission_id, &MissionStatus::Paused)
                    .await
                    .ok();

                let reason = if step.is_checkpoint {
                    "checkpoint"
                } else {
                    "manual"
                };
                self.mission_manager
                    .broadcast(
                        mission_id,
                        StreamEvent::Status {
                            status: format!(
                                r#"{{"type":"mission_paused","step_index":{},"reason":"{}"}}"#,
                                idx, reason
                            ),
                        },
                    )
                    .await;

                // Return - will be resumed via resume_mission
                return Ok(());
            }

            // Execute step with completed steps context
            let step_clone = step.clone();
            let policy_str = match mission.approval_policy {
                ApprovalPolicy::Auto => "auto",
                ApprovalPolicy::Checkpoint => "checkpoint",
                ApprovalPolicy::Manual => "manual",
            };
            self.run_single_step(
                mission_id,
                &mission.agent_id,
                session_id,
                idx,
                &step_clone,
                total,
                &completed_steps,
                cancel_token.clone(),
                workspace_path,
                &mission.goal,
                policy_str,
            )
            .await?;

            // Reload step from DB to get the saved output_summary
            let updated = self
                .agent_service
                .get_mission(mission_id)
                .await
                .ok()
                .flatten();
            if let Some(ref m) = updated {
                if let Some(s) = m.steps.iter().find(|s| s.index == step_clone.index) {
                    completed_steps.push(s.clone());
                    // Truncate old summaries to bound memory (only last 3 need full text)
                    let len = completed_steps.len();
                    if len > 3 {
                        for old in &mut completed_steps[..len - 3] {
                            old.output_summary = None;
                        }
                    }
                }
            }

            // P1: Evaluate re-planning after checkpoint steps
            // D1: Cap replan attempts to avoid infinite loops
            if step_clone.is_checkpoint
                && i + 1 < current_steps.len()
                && replan_count < MAX_REPLAN_COUNT
            {
                // B3: Replan failure is non-fatal — log and continue
                match self
                    .evaluate_replan(
                        mission_id,
                        &mission.agent_id,
                        session_id,
                        &completed_steps,
                        &current_steps[i + 1..],
                        cancel_token.clone(),
                        workspace_path,
                    )
                    .await
                {
                    Ok(Some(new_remaining)) if !new_remaining.is_empty() => {
                        replan_count += 1;

                        // Replace remaining steps with re-planned ones
                        let mut all_steps = completed_steps
                            .iter()
                            .map(|s| {
                                let mut cs = s.clone();
                                cs.status = StepStatus::Completed;
                                cs
                            })
                            .collect::<Vec<_>>();
                        all_steps.extend(new_remaining.clone());

                        self.agent_service
                            .replan_remaining_steps(mission_id, all_steps)
                            .await
                            .ok();

                        // Continue with new remaining steps
                        current_steps = new_remaining;
                        total_steps = completed_steps.len() + current_steps.len();
                        i = 0;
                        continue;
                    }
                    Ok(Some(_)) => {
                        tracing::warn!("Mission {} replan returned empty steps, keeping current plan", mission_id);
                    }
                    Ok(None) => { /* keep current plan */ }
                    Err(e) => {
                        tracing::warn!(
                            "Mission {} replan evaluation failed, keeping current plan: {}",
                            mission_id,
                            e
                        );
                    }
                }
            }

            i += 1;
        }

        // All steps completed
        self.agent_service
            .update_mission_status(mission_id, &MissionStatus::Completed)
            .await
            .ok();

        Ok(())
    }

    /// Execute a single step by bridging to TaskExecutor.
    /// Includes retry logic for transient failures and output summary extraction.
    async fn run_single_step(
        &self,
        mission_id: &str,
        agent_id: &str,
        session_id: &str,
        step_index: u32,
        step: &MissionStep,
        total_steps: usize,
        completed_steps: &[MissionStep],
        cancel_token: CancellationToken,
        workspace_path: Option<&str>,
        mission_goal: &str,
        approval_policy: &str,
    ) -> Result<()> {
        // Mark step as running
        self.agent_service
            .update_step_status(mission_id, step_index, &StepStatus::Running)
            .await
            .ok();
        self.agent_service
            .advance_mission_step(mission_id, step_index)
            .await
            .ok();

        // Broadcast step_start
        self.mission_manager
            .broadcast(
                mission_id,
                StreamEvent::Status {
                    status: format!(
                        r#"{{"type":"step_start","step_index":{},"step_title":"{}","total_steps":{}}}"#,
                        step_index,
                        step.title.replace('"', r#"\""#),
                        total_steps
                    ),
                },
            )
            .await;

        // Build base step prompt with previous step summaries (P0)
        let base_prompt = Self::build_step_prompt(mission_goal, step_index, step, total_steps, completed_steps);

        // Build mission context for system prompt injection
        let mc_json = serde_json::json!({
            "goal": mission_goal,
            "approval_policy": approval_policy,
            "total_steps": total_steps,
            "current_step": step_index + 1,
        });

        // Execute with retry logic (P2)
        let max_retries = step.max_retries;
        let mut last_err: Option<anyhow::Error> = None;

        for attempt in 0..=max_retries {
            // B4: On retry, build prompt with error context instead of duplicating
            let prompt = if attempt == 0 {
                base_prompt.clone()
            } else {
                let prev_err = last_err.as_ref().map(|e| e.to_string()).unwrap_or_default();
                format!(
                    "{}\n\n## Retry Context\n\
                     The previous attempt (#{}) failed with: {}\n\
                     Please retry this step, addressing the error if possible.",
                    base_prompt, attempt, prev_err
                )
            };

            if attempt > 0 {
                // Record retry
                self.agent_service
                    .increment_step_retry(mission_id, step_index)
                    .await
                    .ok();
                self.mission_manager
                    .broadcast(
                        mission_id,
                        StreamEvent::Status {
                            status: format!(
                                r#"{{"type":"step_retry","step_index":{},"attempt":{}}}"#,
                                step_index, attempt
                            ),
                        },
                    )
                    .await;

                // Exponential backoff: 2s, 4s
                let delay = std::time::Duration::from_secs(2u64.pow(attempt));
                tokio::time::sleep(delay).await;
            }

            match tokio::time::timeout(
                STEP_EXECUTION_TIMEOUT,
                self.execute_via_bridge(
                    agent_id,
                    session_id,
                    mission_id,
                    &prompt,
                    cancel_token.clone(),
                    workspace_path,
                    Some(mc_json.clone()),
                ),
            )
            .await
            {
                Ok(Ok(_)) => {
                    // Extract and save output summary (P0)
                    let summary = self.extract_step_summary(session_id).await;
                    if let Some(ref s) = summary {
                        self.agent_service
                            .set_step_output_summary(mission_id, step_index, s)
                            .await
                            .ok();
                    }

                    self.agent_service
                        .complete_step(mission_id, step_index, 0)
                        .await
                        .ok();
                    self.mission_manager
                        .broadcast(
                            mission_id,
                            StreamEvent::Status {
                                status: format!(
                                    r#"{{"type":"step_complete","step_index":{},"tokens_used":0}}"#,
                                    step_index
                                ),
                            },
                        )
                        .await;
                    return Ok(());
                }
                Ok(Err(e)) => {
                    let is_retryable = Self::is_retryable_error(&e);
                    if is_retryable && attempt < max_retries {
                        tracing::warn!(
                            "Step {}/{} attempt {} failed (retryable): {}",
                            step_index,
                            total_steps,
                            attempt,
                            e
                        );
                        last_err = Some(e);
                        continue;
                    }
                    // Non-retryable or exhausted retries
                    let err_msg = e.to_string();
                    self.agent_service
                        .fail_step(mission_id, step_index, &err_msg)
                        .await
                        .ok();
                    self.agent_service
                        .update_mission_status(mission_id, &MissionStatus::Failed)
                        .await
                        .ok();
                    self.agent_service
                        .set_mission_error(mission_id, &err_msg)
                        .await
                        .ok();
                    return Err(e);
                }
                Err(_elapsed) => {
                    let err_msg = format!("Step {} timed out after {}s", step_index + 1, STEP_EXECUTION_TIMEOUT.as_secs());
                    self.agent_service
                        .fail_step(mission_id, step_index, &err_msg)
                        .await
                        .ok();
                    self.agent_service
                        .update_mission_status(mission_id, &MissionStatus::Failed)
                        .await
                        .ok();
                    self.agent_service
                        .set_mission_error(mission_id, &err_msg)
                        .await
                        .ok();
                    return Err(anyhow!(err_msg));
                }
            }
        }

        // Should not reach here, but handle exhausted retries
        Err(last_err.unwrap_or_else(|| anyhow!("Step failed after retries")))
    }

    /// Bridge to TaskExecutor: create temp task, execute, forward events.
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

    /// Generate execution plan by asking the Agent.
    async fn generate_plan(
        &self,
        mission_id: &str,
        mission: &MissionDoc,
        session_id: &str,
        cancel_token: CancellationToken,
        workspace_path: Option<&str>,
    ) -> Result<Vec<MissionStep>> {
        let context_section = mission
            .context
            .as_deref()
            .map(|c| format!("\n## Additional Context\n{}", c))
            .unwrap_or_default();

        let prompt = format!(
            r#"You are planning a mission. Before creating the plan, analyze the goal carefully.

## Mission Goal
{}
{}

## Instructions
1. First, think about what information you need and what approach to take
2. Consider potential challenges and dependencies between steps
3. Then create a concrete execution plan

## Output Format
Output a JSON array wrapped in ```json code block. Each step:
```json
[{{"title": "Step title", "description": "What to do and expected outcome", "is_checkpoint": false}}]
```

Rules:
- 2-8 steps, each should be a concrete actionable unit
- Description should include expected outcome so completion can be verified
- Set is_checkpoint: true for steps with irreversible side effects
- Steps should be ordered by dependency — earlier steps should not depend on later ones"#,
            mission.goal, context_section
        );

        // Execute via bridge to get Agent response
        self.execute_via_bridge(
            &mission.agent_id,
            session_id,
            mission_id,
            &prompt,
            cancel_token,
            workspace_path,
            None, // no mission_context during planning phase
        )
        .await?;

        // Parse plan from session messages
        let session = self
            .agent_service
            .get_session(session_id)
            .await
            .map_err(|e| anyhow!("DB error: {}", e))?
            .ok_or_else(|| anyhow!("Session not found"))?;

        let steps = self.parse_plan_from_messages(&session.messages_json)?;
        Ok(steps)
    }

    /// Parse plan JSON from the last assistant message.
    fn parse_plan_from_messages(&self, messages_json: &str) -> Result<Vec<MissionStep>> {
        let msgs: Vec<serde_json::Value> =
            serde_json::from_str(messages_json).map_err(|e| anyhow!("Invalid messages: {}", e))?;

        // Find last assistant message
        let assistant_text = msgs
            .iter()
            .rev()
            .find(|m| m.get("role").and_then(|r| r.as_str()) == Some("assistant"))
            .and_then(|m| {
                let content = m.get("content")?;
                if let Some(s) = content.as_str() {
                    return Some(s.to_string());
                }
                // Handle array content
                content.as_array().and_then(|arr| {
                    arr.iter().find_map(|item| {
                        item.get("text").and_then(|t| t.as_str()).map(String::from)
                    })
                })
            })
            .ok_or_else(|| anyhow!("No assistant response found"))?;

        // Extract JSON from ```json ... ``` block or try direct parse
        let json_str = Self::extract_json_block(&assistant_text);

        Self::parse_steps_json(&json_str, 0)
    }

    /// Extract JSON from ```json ... ``` code block, or return the whole string.
    fn extract_json_block(text: &str) -> String {
        runtime::extract_json_block(text)
    }

    /// Shared: parse a JSON string of step definitions into MissionStep entries.
    /// `start_index` offsets the step indices (0 for initial plan, N for replan).
    /// Tolerant of missing fields: title defaults to "Step N", description defaults to title.
    fn parse_steps_json(json_str: &str, start_index: usize) -> Result<Vec<MissionStep>> {
        #[derive(serde::Deserialize)]
        struct PlanStep {
            #[serde(default)]
            title: Option<String>,
            #[serde(default)]
            description: Option<String>,
            #[serde(default)]
            is_checkpoint: bool,
        }

        let plan_steps: Vec<PlanStep> = serde_json::from_str(json_str)
            .map_err(|e| anyhow!("Failed to parse steps JSON: {}", e))?;

        let steps = plan_steps
            .into_iter()
            .enumerate()
            .map(|(i, ps)| {
                let default_title = format!("Step {}", start_index + i + 1);
                let title = ps.title.unwrap_or(default_title);
                let description = ps.description.unwrap_or_else(|| title.clone());
                MissionStep {
                    index: (start_index + i) as u32,
                    title,
                    description,
                    status: StepStatus::Pending,
                    is_checkpoint: ps.is_checkpoint,
                    approved_by: None,
                    started_at: None,
                    completed_at: None,
                    error_message: None,
                    tokens_used: 0,
                    output_summary: None,
                    retry_count: 0,
                    max_retries: 2,
                }
            })
            .collect();

        Ok(steps)
    }

    /// Build step prompt with mission goal and previous step summaries.
    fn build_step_prompt(
        mission_goal: &str,
        step_index: u32,
        step: &MissionStep,
        total_steps: usize,
        completed_steps: &[MissionStep],
    ) -> String {
        let mut prompt = format!(
            "## Current Task: Step {}/{} — {}\n\n{}\n",
            step_index + 1,
            total_steps,
            step.title,
            step.description
        );

        prompt.push_str(&format!(
            "\n## Mission Goal (for reference)\n{}\n",
            mission_goal
        ));

        // Only keep last 3 steps to avoid context bloat
        if !completed_steps.is_empty() {
            prompt.push_str("\n## Previous Steps Summary\n");
            let recent: Vec<_> = completed_steps.iter().rev().take(3).collect();
            for cs in recent.into_iter().rev() {
                let full = cs.output_summary.as_deref().unwrap_or("(no summary)");
                let summary = if full.chars().count() > 300 {
                    let truncated: String = full.chars().take(297).collect();
                    format!("{}...", truncated)
                } else {
                    full.to_string()
                };
                prompt.push_str(&format!(
                    "- Step {}: {} → {}\n",
                    cs.index + 1,
                    cs.title,
                    summary
                ));
            }
            prompt.push('\n');
        }

        prompt.push_str("## Instructions\n");
        prompt.push_str("- Complete this step as described above\n");
        prompt.push_str("- Verify your work matches the expected outcome in the description\n");
        prompt.push_str("- Be concise — your response will be saved as step summary");
        prompt
    }

    /// Extract the full output text from the last assistant message in the session.
    /// Saved as-is to output_summary for debugging; truncated only when injected into prompts.
    async fn extract_step_summary(&self, session_id: &str) -> Option<String> {
        let session = self.agent_service.get_session(session_id).await.ok()??;
        let text = Self::extract_last_assistant_text(&session.messages_json)?;

        if text.is_empty() {
            None
        } else {
            Some(text)
        }
    }

    /// Extract text content from the last assistant message in a messages JSON array.
    fn extract_last_assistant_text(messages_json: &str) -> Option<String> {
        runtime::extract_last_assistant_text(messages_json)
    }

    /// Classify whether an error is transient and worth retrying.
    fn is_retryable_error(e: &anyhow::Error) -> bool {
        runtime::is_retryable_error(e)
    }

    /// Build the prompt for re-plan evaluation after a checkpoint step.
    fn build_replan_prompt(
        completed_steps: &[MissionStep],
        remaining_steps: &[MissionStep],
    ) -> String {
        let mut prompt = String::from(
            "## Re-plan Evaluation\n\n\
             A checkpoint step has just completed. Review the progress so far \
             and decide whether the remaining plan needs adjustment.\n\n\
             ### Completed Steps\n",
        );

        for cs in completed_steps {
            let full = cs.output_summary.as_deref().unwrap_or("(no summary)");
            let summary = if full.chars().count() > 500 {
                let truncated: String = full.chars().take(497).collect();
                format!("{}...", truncated)
            } else {
                full.to_string()
            };
            prompt.push_str(&format!(
                "- Step {}: {} → {}\n",
                cs.index + 1,
                cs.title,
                summary
            ));
        }

        prompt.push_str("\n### Current Remaining Plan\n");
        for rs in remaining_steps {
            prompt.push_str(&format!(
                "- Step {}: {} — {}\n",
                rs.index + 1,
                rs.title,
                rs.description
            ));
        }

        prompt.push_str(
            "\n### Decision\n\
             Based on the results so far, should the remaining plan be adjusted?\n\n\
             ## Output Format\n\
             Respond with a JSON object in a ```json code block:\n\
             - To keep current plan: `{\"decision\": \"keep\"}`\n\
             - To replace remaining steps: `{\"decision\": \"replan\", \"steps\": [{\"title\": \"...\", \"description\": \"...\", \"is_checkpoint\": false}]}`\n",
        );

        prompt
    }

    /// Parse the Agent's re-plan response into new MissionStep entries.
    /// `start_index` is the index offset for the new steps (= number of completed steps).
    fn parse_replan_response(
        &self,
        response: &str,
        start_index: usize,
    ) -> Result<Vec<MissionStep>> {
        let json_str = Self::extract_json_block(response);
        let steps = Self::parse_steps_json(&json_str, start_index)?;
        if steps.is_empty() {
            return Err(anyhow!("Re-plan produced empty step list"));
        }
        Ok(steps)
    }

    /// P1: Evaluate whether remaining steps need re-planning after a checkpoint.
    ///
    /// Sends a structured prompt to the Agent with completed step summaries
    /// and the current remaining plan. The Agent decides:
    /// - "keep" → no change, continue with existing plan
    /// - JSON array → replacement steps for the remaining work
    ///
    /// Returns `Ok(None)` if no re-plan needed, `Ok(Some(steps))` if re-planned.
    async fn evaluate_replan(
        &self,
        mission_id: &str,
        agent_id: &str,
        session_id: &str,
        completed_steps: &[MissionStep],
        remaining_steps: &[MissionStep],
        cancel_token: CancellationToken,
        workspace_path: Option<&str>,
    ) -> Result<Option<Vec<MissionStep>>> {
        // Build the evaluation prompt
        let prompt = Self::build_replan_prompt(completed_steps, remaining_steps);

        // Execute via bridge
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

        let response =
            Self::extract_last_assistant_text(&session.messages_json).unwrap_or_default();

        // Try structured JSON parsing first
        let json_str = Self::extract_json_block(&response);
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(&json_str) {
            if val.get("decision").and_then(|d| d.as_str()) == Some("keep") {
                tracing::info!(
                    "Mission {} replan evaluation: keep current plan",
                    mission_id
                );
                return Ok(None);
            }
            // Extract steps from structured response
            if let Some(steps_val) = val.get("steps") {
                let steps_str = steps_val.to_string();
                match Self::parse_steps_json(&steps_str, completed_steps.len()) {
                    Ok(new_steps) if !new_steps.is_empty() => {
                        tracing::info!(
                            "Mission {} re-planned: {} remaining steps replaced with {}",
                            mission_id,
                            remaining_steps.len(),
                            new_steps.len()
                        );
                        self.mission_manager
                            .broadcast(
                                mission_id,
                                StreamEvent::Status {
                                    status: format!(
                                        r#"{{"type":"mission_replanned","new_step_count":{}}}"#,
                                        new_steps.len()
                                    ),
                                },
                            )
                            .await;
                        return Ok(Some(new_steps));
                    }
                    _ => {}
                }
            }
        }

        // Fallback: check for "keep" text
        let trimmed = response.trim().to_lowercase();
        if trimmed == "keep" || trimmed.starts_with("keep") {
            tracing::info!(
                "Mission {} replan evaluation: keep current plan (text fallback)",
                mission_id
            );
            return Ok(None);
        }

        // Fallback: try to parse as raw steps array
        match self.parse_replan_response(&response, completed_steps.len()) {
            Ok(new_steps) => {
                tracing::info!(
                    "Mission {} re-planned: {} remaining steps replaced with {}",
                    mission_id,
                    remaining_steps.len(),
                    new_steps.len()
                );
                self.mission_manager
                    .broadcast(
                        mission_id,
                        StreamEvent::Status {
                            status: format!(
                                r#"{{"type":"mission_replanned","new_step_count":{}}}"#,
                                new_steps.len()
                            ),
                        },
                    )
                    .await;
                Ok(Some(new_steps))
            }
            Err(e) => {
                // Parse failure is non-fatal; keep current plan
                tracing::warn!(
                    "Mission {} replan parse failed, keeping current plan: {}",
                    mission_id,
                    e
                );
                Ok(None)
            }
        }
    }

    /// Resume a paused mission from the next pending step.
    pub async fn resume_mission(
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

        // Dispatch to AdaptiveExecutor for adaptive mode
        // (resume_adaptive has its own cleanup wrapper)
        if mission.execution_mode == ExecutionMode::Adaptive {
            let adaptive = AdaptiveExecutor::new(
                self.db.clone(),
                self.mission_manager.clone(),
                self.workspace_root.clone(),
            );
            return adaptive.resume_adaptive(mission_id, cancel_token).await;
        }

        // Sequential resume with guaranteed cleanup
        let exec_result = self
            .resume_mission_sequential(mission_id, &mission, cancel_token)
            .await;

        // Guaranteed cleanup: broadcast Done + complete in manager
        match &exec_result {
            Ok(_) => {
                let done_status = self
                    .agent_service
                    .get_mission(mission_id)
                    .await
                    .ok()
                    .flatten()
                    .map(|m| match m.status {
                        MissionStatus::Paused => "paused",
                        MissionStatus::Completed => "completed",
                        MissionStatus::Cancelled => "cancelled",
                        _ => "completed",
                    })
                    .unwrap_or("completed");

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
        exec_result
    }

    /// Inner sequential resume logic (separated for cleanup wrapper).
    async fn resume_mission_sequential(
        &self,
        mission_id: &str,
        mission: &MissionDoc,
        cancel_token: CancellationToken,
    ) -> Result<()> {
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
        self.agent_service
            .update_mission_status(mission_id, &MissionStatus::Running)
            .await
            .ok();

        // Collect completed steps for context injection on resume
        let prior_completed: Vec<MissionStep> = mission
            .steps
            .iter()
            .filter(|s| s.status == StepStatus::Completed)
            .cloned()
            .collect();

        // Find remaining steps starting from current
        let remaining: Vec<MissionStep> = mission
            .steps
            .iter()
            .filter(|s| s.status == StepStatus::Pending || s.status == StepStatus::AwaitingApproval)
            .cloned()
            .collect();

        self.execute_steps(
            mission_id,
            mission,
            &session_id,
            remaining,
            prior_completed,
            cancel_token,
            workspace_path.as_deref(),
        )
        .await
    }
}
