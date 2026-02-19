//! Smart Log trigger implementation
//!
//! Records team activity log entries with static summaries.
//! Deep document analysis is handled separately by document_analysis.rs.

use agime_team::models::mongo::{build_fallback_summary, SmartLogContext, SmartLogTrigger};
use agime_team::services::mongo::{SmartLogService, TeamService};
use agime_team::MongoDb;
use std::sync::Arc;

use super::service_mongo::AgentService;

pub struct SmartLogTriggerImpl {
    db: Arc<MongoDb>,
    _agent_service: Arc<AgentService>,
}

impl SmartLogTriggerImpl {
    pub fn new(db: Arc<MongoDb>, agent_service: Arc<AgentService>) -> Self {
        Self {
            db,
            _agent_service: agent_service,
        }
    }
}

impl SmartLogTrigger for SmartLogTriggerImpl {
    fn trigger(&self, ctx: SmartLogContext) {
        let db = self.db.clone();

        tokio::spawn(async move {
            if let Err(e) = process_smart_log(db, ctx).await {
                tracing::warn!("Smart log processing failed: {}", e);
            }
        });
    }
}

async fn process_smart_log(db: Arc<MongoDb>, mut ctx: SmartLogContext) -> anyhow::Result<()> {
    // Resolve user_name from team member list if not provided
    if ctx.user_name.is_none() {
        let team_svc = TeamService::new((*db).clone());
        if let Ok(Some(team)) = team_svc.get(&ctx.team_id).await {
            if let Some(member) = team.members.iter().find(|m| m.user_id == ctx.user_id) {
                ctx.user_name = Some(member.display_name.clone());
            }
        }
    }

    let svc = SmartLogService::new((*db).clone());

    // Insert entry with static fallback summary (no LLM call)
    let entry_id = svc.create(&ctx).await?;

    if ctx.content_for_ai.is_some() {
        let summary =
            build_fallback_summary(&ctx.action, &ctx.resource_type, &ctx.resource_name);
        svc.update_ai_summary(&entry_id, &summary, "completed")
            .await?;
    }

    Ok(())
}
