use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use agime::agents::harness::completion::assistant_text_counts_as_terminal_reply;
use anyhow::{anyhow, Result};
use chrono::{DateTime, Utc};
use chrono_tz::Tz;
use croner::Cron;
use serde_json::Value;
use tokio_util::sync::CancellationToken;

use agime_team::MongoDb;

use crate::agent::chat_channel_executor::{ChatChannelExecutor, ExecuteChannelMessageRequest};
use crate::agent::chat_channel_manager::ChatChannelManager;
use crate::agent::chat_channels::{
    ChatChannelAuthorType, ChatChannelDisplayStatus, ChatChannelMessageSurface, ChatChannelService,
    ChatChannelThreadState,
};
use crate::agent::chat_manager::ChatManager;
use crate::agent::server_harness_host::ServerHarnessHost;
use crate::agent::service_mongo::AgentService;
use crate::agent::session_mongo::CreateSessionRequest;
use crate::agent::task_manager::create_task_manager;

use super::models::{
    ScheduledTaskDoc, ScheduledTaskExecutionContract, ScheduledTaskKind, ScheduledTaskOutputMode,
    ScheduledTaskProfile, ScheduledTaskPublishBehavior, ScheduledTaskRunDoc,
    ScheduledTaskRunOutcomeReason, ScheduledTaskRunStatus, ScheduledTaskSelfEvaluation,
    ScheduledTaskSelfEvaluationGrade, ScheduledTaskSourcePolicy, ScheduledTaskStatus,
};
use super::service::ScheduledTaskService;

const SCHEDULER_TICK_SECS: u64 = 5;
const LEASE_SECS: i64 = 120;
const MISSED_TASK_GRACE_SECS: i64 = 5;

fn scheduler_owner() -> String {
    let instance =
        std::env::var("TEAM_SERVER_INSTANCE_ID").unwrap_or_else(|_| "team-server".to_string());
    format!("scheduled-task-scheduler:{}", instance)
}

fn scheduled_task_session_source() -> &'static str {
    "scheduled_task"
}

fn scheduled_task_allowed_extensions(task: &ScheduledTaskDoc) -> Option<Vec<String>> {
    match task.task_profile {
        ScheduledTaskProfile::DocumentTask | ScheduledTaskProfile::HybridTask => {
            Some(vec!["document_tools".to_string(), "developer".to_string()])
        }
        ScheduledTaskProfile::WorkspaceTask => Some(vec!["developer".to_string()]),
        ScheduledTaskProfile::RetrievalTask => Some(vec!["developer".to_string()]),
    }
}

fn scheduled_task_document_policy(
    task: &ScheduledTaskDoc,
) -> (Option<String>, Option<String>, Option<String>) {
    match task.task_profile {
        ScheduledTaskProfile::DocumentTask => (
            Some("read_only".to_string()),
            Some("full".to_string()),
            Some("read_only".to_string()),
        ),
        ScheduledTaskProfile::HybridTask => (
            Some("controlled_write".to_string()),
            Some("full".to_string()),
            Some("controlled_write".to_string()),
        ),
        ScheduledTaskProfile::WorkspaceTask | ScheduledTaskProfile::RetrievalTask => {
            (None, None, None)
        }
    }
}

fn scheduled_task_contract_instructions(task: &ScheduledTaskDoc) -> String {
    let mut lines = vec![
        "这是一个无人值守的定时任务执行面。".to_string(),
        "当前已经处于任务执行阶段，不要再次创建、解析、更新、删除或手动触发任何定时任务。".to_string(),
        "禁止调用 team_mcp__parse_scheduled_task、team_mcp__create_scheduled_task、team_mcp__update_scheduled_task、team_mcp__remove_scheduled_task、team_mcp__run_scheduled_task_now。".to_string(),
        "不要创建内部任务板，不要调用 tasks__TaskCreate / TaskUpdate / TaskOutput。".to_string(),
        "不要输出“我现在开始”“让我先”等过程性文本；最终必须给出用户可展示的结论。".to_string(),
        format!("任务 profile：{:?}", task.task_profile),
        format!(
            "输出模式：{:?}；必须最终答复：{}；允许部分结果：{}。",
            task.execution_contract.output_mode,
            task.execution_contract.must_return_final_text,
            task.execution_contract.allow_partial_result
        ),
    ];
    if let Some(artifact_path) = task
        .execution_contract
        .artifact_path
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        lines.push(format!(
            "如果需要产物，必须把最终文件写到这个精确的工作区相对路径：{}。",
            artifact_path
        ));
        lines.push(
            "只创建目录、不写最终文件，或写到其它文件名/路径，都不算完成，会被判定为 contract violation。"
                .to_string(),
        );
    }
    match task.execution_contract.publish_behavior {
        ScheduledTaskPublishBehavior::PublishWorkspaceArtifact => lines.push(
            "产物生成后需要发布回正式文档区，优先使用 publish_workspace_artifact。".to_string(),
        ),
        ScheduledTaskPublishBehavior::CreateDocumentFromFile => lines.push(
            "产物生成后需要从工作区文件创建正式文档，优先使用 create_document_from_file。"
                .to_string(),
        ),
        ScheduledTaskPublishBehavior::None => {}
    }
    match task.task_profile {
        ScheduledTaskProfile::DocumentTask | ScheduledTaskProfile::HybridTask => lines.push(
            "文档工具只负责建立文档访问并把文件落到工作区；这不是最终结果。拿到 workspace path 后，必须继续用本地/工作区工具读取、分析、加工，再给最终答复。".to_string(),
        ),
        ScheduledTaskProfile::WorkspaceTask => lines.push(
            "围绕工作区产物执行，不要偏离到无关检索或内部任务编排。先写出目标文件，再给最终总结；如果只创建目录或只描述计划，任务会失败。".to_string(),
        ),
        ScheduledTaskProfile::RetrievalTask => {
            lines.push(
                "这是一个无人值守的深度检索任务。你的目标不是访问某个网站，而是基于多来源产出稳定、可信、可展示的最终结论。".to_string(),
            );
            lines.push(
                "可以使用 developer 能力在当前工作区临时编写和执行 Python 脚本完成抓取、抽取、过滤、验证和汇总；这些脚本只服务于本次任务，不要把它们当成平台工具。".to_string(),
            );
            lines.push(
                "默认主路径：先在当前工作区写一个临时 Python 检索脚本，再用它执行多来源抓取与汇总。不要把零散 shell one-liner 当成主要检索实现。".to_string(),
            );
            lines.push(
                "这个临时 Python 脚本至少要负责：来源发现、内容抓取、正文/摘要提取、失败记录、结果汇总。必要时可以再拆成临时辅助文件。".to_string(),
            );
            lines.push(
                "优先按以下顺序获取信息：结构化源（RSS/Atom/release feed/sitemap）-> 官方源（官方 blog/公告/文档/release notes）-> 官方站内索引页 -> 次级来源/聚合源 -> 宽泛搜索 fallback。".to_string(),
            );
            lines.push(
                "不要一开始就依赖脆弱的自由网页抓取；优先写临时 Python 脚本读取结构化源和官方索引。".to_string(),
            );
            lines.push(
                "可以用 shell 来执行 Python、查看文件、做少量诊断；但主检索循环和结果汇总应由临时 Python 脚本承担。".to_string(),
            );
            lines.push(
                "单一来源失败不能直接结束。一个网站失败时必须换同类来源；一个查询失败时必须改写 query 再试；只有达到最小来源尝试数后，才允许输出降级总结。".to_string(),
            );
            if let Some(policy) = task.execution_contract.source_policy {
                lines.push(match policy {
                    ScheduledTaskSourcePolicy::OfficialFirst => {
                        "信息源策略：official_first。优先官方公告、官方博客、官方文档、release notes 和 changelog，不限制地域。".to_string()
                    }
                    ScheduledTaskSourcePolicy::DomesticPreferred => {
                        "信息源策略：domestic_preferred。最终用中文输出，优先国内/中文来源；但若主题涉及官方主体，仍优先一手官方源。".to_string()
                    }
                    ScheduledTaskSourcePolicy::GlobalPreferred => {
                        "信息源策略：global_preferred。优先国际来源、英文官方博客和全球主流一手源；最终仍用中文总结。".to_string()
                    }
                    ScheduledTaskSourcePolicy::Mixed => {
                        "信息源策略：mixed。国内外来源混合，但优先官方/一手来源；不要只用单一地区来源。".to_string()
                    }
                });
            }
            if let Some(attempts) = task.execution_contract.minimum_source_attempts {
                lines.push(format!(
                    "最少来源尝试数：{}。单一网站失败不能直接结束。",
                    attempts
                ));
            }
            if let Some(successes) = task.execution_contract.minimum_successful_sources {
                lines.push(format!(
                    "最少成功来源数：{}。未达到前必须继续换源或换查询。",
                    successes
                ));
            }
            if task.execution_contract.prefer_structured_sources == Some(true) {
                lines.push("优先使用 RSS、Atom、sitemap、官方 blog/release 列表。".to_string());
            }
            if task.execution_contract.allow_query_retry == Some(true) {
                lines.push("如果首轮关键词没有足够结果，必须改写查询后重试。".to_string());
            }
            if task.execution_contract.fallback_to_secondary_sources == Some(true) {
                lines.push("官方/结构化源不足时，允许退到次级来源和宽泛搜索 fallback，但最终总结必须区分已确认事实、局限和失败尝试。".to_string());
            }
            lines.push(
                "不要只看 snippet。要尽量验证来源是否真的回答问题、是否足够可信、是否在时间窗口内。".to_string(),
            );
            lines.push(
                "在最终总结前，尽量在工作区保留一个简短的来源尝试记录（例如 JSON 或 Markdown），记录成功来源、失败来源和失败原因，便于后续审计与复现。".to_string(),
            );
            lines.push(
                "最终结果必须尽量分成四部分：已确认结论、补充发现、失败尝试/局限、下一步建议。".to_string(),
            );
        }
    }
    if matches!(
        task.execution_contract.publish_behavior,
        ScheduledTaskPublishBehavior::PublishWorkspaceArtifact
            | ScheduledTaskPublishBehavior::CreateDocumentFromFile
    ) {
        lines.push("发布前必须先生成目标产物文件；没有产物时不要直接进入发布步骤。".to_string());
    }
    lines.push(
        "如果工具部分失败，但已经能给出部分结论，必须输出当前结果、缺口和下一步建议，不能沉默结束。"
            .to_string(),
    );
    lines.join("\n")
}

fn artifact_exists_for_contract(
    contract: &ScheduledTaskExecutionContract,
    workspace_path: Option<&str>,
) -> bool {
    if !matches!(
        contract.output_mode,
        ScheduledTaskOutputMode::SummaryAndArtifact
    ) {
        return true;
    }
    let Some(base) = workspace_path
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return false;
    };
    let Some(relative) = contract
        .artifact_path
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return false;
    };
    Path::new(base).join(relative).is_file()
}

fn parse_self_evaluation_grade(value: &str) -> ScheduledTaskSelfEvaluationGrade {
    match value.trim().to_ascii_lowercase().as_str() {
        "excellent" => ScheduledTaskSelfEvaluationGrade::Excellent,
        "good" => ScheduledTaskSelfEvaluationGrade::Good,
        "acceptable" => ScheduledTaskSelfEvaluationGrade::Acceptable,
        "weak" => ScheduledTaskSelfEvaluationGrade::Weak,
        _ => ScheduledTaskSelfEvaluationGrade::Failed,
    }
}

fn extract_first_json_object(text: &str) -> Option<Value> {
    let start = text.find('{')?;
    let end = text.rfind('}')?;
    if end <= start {
        return None;
    }
    serde_json::from_str::<Value>(&text[start..=end]).ok()
}

fn parse_self_evaluation_payload(value: Value) -> Option<ScheduledTaskSelfEvaluation> {
    let obj = value.as_object()?;
    let score = obj.get("score")?.as_i64()? as i32;
    let confidence = obj.get("confidence")?.as_f64()? as f32;
    let summary = obj.get("summary")?.as_str()?.trim().to_string();
    if summary.is_empty() {
        return None;
    }
    let list = |key: &str| -> Vec<String> {
        obj.get(key)
            .and_then(|value| value.as_array())
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| item.as_str())
                    .map(str::trim)
                    .filter(|item| !item.is_empty())
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    };
    Some(ScheduledTaskSelfEvaluation {
        score,
        grade: parse_self_evaluation_grade(
            obj.get("grade")
                .and_then(|value| value.as_str())
                .unwrap_or("failed"),
        ),
        goal_completion: obj.get("goal_completion")?.as_i64()? as i32,
        result_quality: obj.get("result_quality")?.as_i64()? as i32,
        evidence_quality: obj.get("evidence_quality")?.as_i64()? as i32,
        execution_stability: obj.get("execution_stability")?.as_i64()? as i32,
        contract_compliance: obj.get("contract_compliance")?.as_i64()? as i32,
        summary,
        completed_steps: list("completed_steps"),
        failed_steps: list("failed_steps"),
        risks: list("risks"),
        confidence,
    })
}

fn self_evaluation_prompt(
    task: &ScheduledTaskDoc,
    status: ScheduledTaskRunStatus,
    outcome_reason: Option<ScheduledTaskRunOutcomeReason>,
    warning_count: i32,
    summary: Option<&str>,
    error: Option<&str>,
) -> String {
    let summary = summary.unwrap_or("无最终总结");
    let error = error.unwrap_or("无");
    format!(
        "你正在为一条已执行完成的定时任务做自评。请基于任务目标、当前结果和执行情况，输出一个严格 JSON 对象，不要输出 Markdown，不要解释。\n\
JSON schema:\n\
{{\n  \"score\": 0-100 的整数,\n  \"grade\": \"excellent|good|acceptable|weak|failed\",\n  \"goal_completion\": 0-5,\n  \"result_quality\": 0-5,\n  \"evidence_quality\": 0-5,\n  \"execution_stability\": 0-5,\n  \"contract_compliance\": 0-5,\n  \"summary\": \"一句话评价\",\n  \"completed_steps\": [\"...\"],\n  \"failed_steps\": [\"...\"],\n  \"risks\": [\"...\"],\n  \"confidence\": 0-1\n}}\n\
评分时以交付结果为主，不要因为风格问题随意扣分；若结果已完成较好，可给高分。若存在明显不足，要在 failed_steps 和 risks 中写明。\n\
任务信息：\n\
- 标题：{title}\n\
- profile：{profile:?}\n\
- 原始任务：{prompt}\n\
- 最终状态：{status:?}\n\
- outcome_reason：{outcome_reason:?}\n\
- warning_count：{warning_count}\n\
- summary：{summary}\n\
- error：{error}",
        title = task.title,
        profile = task.task_profile,
        prompt = task.prompt,
        status = status,
        outcome_reason = outcome_reason,
        warning_count = warning_count,
        summary = summary,
        error = error,
    )
}

fn should_trigger_improvement_loop(evaluation: &ScheduledTaskSelfEvaluation) -> bool {
    evaluation.score < 75
        || matches!(
            evaluation.grade,
            ScheduledTaskSelfEvaluationGrade::Acceptable
                | ScheduledTaskSelfEvaluationGrade::Weak
                | ScheduledTaskSelfEvaluationGrade::Failed
        )
        || evaluation.goal_completion <= 3
        || evaluation.result_quality <= 3
        || evaluation.evidence_quality <= 3
        || evaluation.execution_stability <= 2
}

fn improvement_loop_prompt(
    task: &ScheduledTaskDoc,
    evaluation: &ScheduledTaskSelfEvaluation,
) -> String {
    format!(
        "你刚完成了一次定时任务，但质量自评偏低。现在只允许做 **一次** 补全改进回合，不要从头随意重跑，也不要再次创建任务。\n\
目标：根据自评里的薄弱项，补全结果质量，并产出一个更好的最终答复。\n\
原任务：{prompt}\n\
当前评分：{score} / 100，等级：{grade:?}\n\
主要问题：{failed_steps}\n\
风险：{risks}\n\
要求：\n\
- 优先修复 failed_steps 和 risks 中最关键的问题\n\
- 如果是检索/研究任务，补充来源、补全局限说明或整理结论；不要无意义重复已有步骤\n\
- 如果结果已经基本可用，重点做结构和结论增强，而不是大幅偏题\n\
- 只允许这一轮改进；完成后直接给出改进后的最终用户可见结果",
        prompt = task.prompt,
        score = evaluation.score,
        grade = evaluation.grade,
        failed_steps = if evaluation.failed_steps.is_empty() {
            "无".to_string()
        } else {
            evaluation.failed_steps.join("；")
        },
        risks = if evaluation.risks.is_empty() {
            "无".to_string()
        } else {
            evaluation.risks.join("；")
        },
    )
}

async fn generate_self_evaluation(
    db: Arc<MongoDb>,
    task: &ScheduledTaskDoc,
    agent_service: &AgentService,
    runtime_workspace_path: Option<&str>,
    status: ScheduledTaskRunStatus,
    outcome_reason: Option<ScheduledTaskRunOutcomeReason>,
    warning_count: i32,
    summary: Option<&str>,
    error: Option<&str>,
) -> Option<ScheduledTaskSelfEvaluation> {
    let agent = agent_service
        .get_agent(&task.agent_id)
        .await
        .ok()
        .flatten()?;
    let host = ServerHarnessHost::new(
        db.clone(),
        Arc::new(AgentService::new(db)),
        create_task_manager(),
    );
    let eval_session = agent_service
        .create_session(CreateSessionRequest {
            team_id: task.team_id.clone(),
            agent_id: task.agent_id.clone(),
            user_id: task.owner_user_id.clone(),
            name: Some(format!("Scheduled Task Self Eval · {}", task.title)),
            attached_document_ids: Vec::new(),
            extra_instructions: Some(
                "这是一个隐藏的定时任务质量自评回合。不要调用任何工具，不要输出 Markdown，只返回严格 JSON 对象。"
                    .to_string(),
            ),
            allowed_extensions: Some(Vec::new()),
            allowed_skill_ids: None,
            retry_config: None,
            max_turns: Some(1),
            tool_timeout_seconds: Some(30),
            max_portal_retry_rounds: Some(0),
            require_final_report: false,
            portal_restricted: false,
            document_access_mode: None,
            document_scope_mode: None,
            document_write_mode: None,
            delegation_policy_override: None,
            session_source: Some("chat".to_string()),
            source_channel_id: None,
            source_channel_name: None,
            source_thread_root_id: None,
            hidden_from_chat_list: Some(true),
        })
        .await
        .ok()?;
    let eval_session_doc = agent_service
        .get_session(&eval_session.session_id)
        .await
        .ok()
        .flatten()?;
    let outcome = host
        .execute_chat_host(
            &eval_session_doc,
            &agent,
            &self_evaluation_prompt(
                task,
                status,
                outcome_reason,
                warning_count,
                summary,
                error,
            ),
            runtime_workspace_path.unwrap_or("/opt/agime").to_string(),
            Some("这是一个只输出 JSON 的质量自评回合。不要调用工具，不要写 Markdown，不要进行任何外部动作。"),
            Vec::new(),
            vec!["Return strict JSON only.".to_string()],
            false,
            CancellationToken::new(),
            Arc::new(ChatManager::new()),
        )
        .await
        .ok()?;
    let payload = outcome
        .last_assistant_text
        .as_deref()
        .and_then(extract_first_json_object)
        .and_then(parse_self_evaluation_payload);
    payload
}

fn count_runtime_warnings(messages_json: &str) -> i32 {
    let Ok(payload) = serde_json::from_str::<serde_json::Value>(messages_json) else {
        return 0;
    };
    let Some(messages) = payload.as_array() else {
        return 0;
    };
    let mut warnings = 0_i32;
    for message in messages {
        let Some(content) = message.get("content").and_then(|value| value.as_array()) else {
            continue;
        };
        for block in content {
            if block.get("type").and_then(|value| value.as_str()) == Some("toolResponse") {
                let is_error = block
                    .get("toolResult")
                    .and_then(|value| value.get("isError"))
                    .and_then(|value| value.as_bool())
                    .unwrap_or(false);
                if is_error {
                    warnings += 1;
                    continue;
                }
                let text = block
                    .get("toolResult")
                    .and_then(|value| value.get("value"))
                    .and_then(|value| value.get("content"))
                    .and_then(|value| value.as_array())
                    .map(|items| {
                        items
                            .iter()
                            .filter_map(|item| item.get("text").and_then(|value| value.as_str()))
                            .collect::<Vec<_>>()
                            .join("\n")
                    })
                    .unwrap_or_default();
                let lowered = text.to_ascii_lowercase();
                if lowered.contains("error")
                    || lowered.contains("failed")
                    || lowered.contains("rg:")
                    || lowered.contains("denied")
                {
                    warnings += 1;
                }
            }
        }
    }
    warnings
}

fn classify_run_completion(
    task: &ScheduledTaskDoc,
    status_text: &str,
    summary: Option<String>,
    error: Option<String>,
    workspace_path: Option<&str>,
    messages_json: Option<&str>,
) -> (
    ScheduledTaskRunStatus,
    Option<ScheduledTaskRunOutcomeReason>,
    i32,
    Option<String>,
    Option<String>,
) {
    let normalized_summary = summary.and_then(|value| {
        let trimmed = value.trim().to_string();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed)
        }
    });
    let terminal_summary = normalized_summary
        .clone()
        .filter(|value| assistant_text_counts_as_terminal_reply(value));
    let warnings = messages_json.map(count_runtime_warnings).unwrap_or(0);
    let normalized_error = error.and_then(|value| {
        let trimmed = value.trim().to_string();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed)
        }
    });

    if status_text.eq_ignore_ascii_case("cancelled") {
        return (
            ScheduledTaskRunStatus::Cancelled,
            Some(ScheduledTaskRunOutcomeReason::Cancelled),
            warnings,
            normalized_summary,
            None,
        );
    }
    if !status_text.eq_ignore_ascii_case("completed") {
        let reason = normalized_error
            .as_deref()
            .map(|value| value.to_ascii_lowercase());
        if reason.as_deref().is_some_and(|value| {
            value.contains("not allowed")
                || value.contains("permission denied")
                || value.contains("disabled by scheduled task")
                || value.contains("capability")
        }) {
            return (
                ScheduledTaskRunStatus::Failed,
                Some(ScheduledTaskRunOutcomeReason::BlockedCapabilityPolicy),
                warnings,
                terminal_summary,
                normalized_error,
            );
        }
        if task.execution_contract.must_return_final_text && terminal_summary.is_none() {
            return (
                ScheduledTaskRunStatus::Failed,
                Some(ScheduledTaskRunOutcomeReason::FailedNoFinalAnswer),
                warnings,
                None,
                normalized_error.or_else(|| {
                    Some("runtime exited without a user-visible assistant response".to_string())
                }),
            );
        }
        return (
            ScheduledTaskRunStatus::Failed,
            Some(ScheduledTaskRunOutcomeReason::FailedContractViolation),
            warnings,
            terminal_summary,
            normalized_error,
        );
    }

    if task.execution_contract.must_return_final_text && terminal_summary.is_none() {
        return (
            ScheduledTaskRunStatus::Failed,
            Some(ScheduledTaskRunOutcomeReason::FailedNoFinalAnswer),
            warnings,
            None,
            Some("runtime completed without a user-visible final answer".to_string()),
        );
    }

    if !artifact_exists_for_contract(&task.execution_contract, workspace_path) {
        return (
            ScheduledTaskRunStatus::Failed,
            Some(ScheduledTaskRunOutcomeReason::FailedContractViolation),
            warnings,
            terminal_summary,
            Some("required workspace artifact was not produced".to_string()),
        );
    }

    if warnings > 0 && task.execution_contract.allow_partial_result {
        return (
            ScheduledTaskRunStatus::Completed,
            Some(ScheduledTaskRunOutcomeReason::CompletedWithWarnings),
            warnings,
            terminal_summary,
            normalized_error,
        );
    }

    (
        ScheduledTaskRunStatus::Completed,
        Some(ScheduledTaskRunOutcomeReason::Completed),
        warnings,
        terminal_summary,
        normalized_error,
    )
}

pub fn compute_next_fire_at(
    task: &ScheduledTaskDoc,
    now: DateTime<Utc>,
) -> Result<Option<DateTime<Utc>>> {
    match task.task_kind {
        ScheduledTaskKind::OneShot => Ok(task.one_shot_at.map(|value| value.to_chrono())),
        ScheduledTaskKind::Cron => {
            let tz_name = task.timezone.trim();
            let timezone: Tz = tz_name
                .parse()
                .map_err(|_| anyhow!("invalid timezone: {}", tz_name))?;
            let expr = task
                .cron_expression
                .as_deref()
                .ok_or_else(|| anyhow!("missing cron expression"))?;
            let cron = Cron::new(expr)
                .parse()
                .map_err(|error| anyhow!("invalid cron expression: {}", error))?;
            let localized_now = now.with_timezone(&timezone);
            let next = cron
                .find_next_occurrence(&localized_now, false)
                .map_err(|error| anyhow!("failed to compute next occurrence: {}", error))?;
            Ok(Some(next.with_timezone(&Utc)))
        }
    }
}

fn schedule_summary(task: &ScheduledTaskDoc) -> String {
    match task.task_kind {
        ScheduledTaskKind::OneShot => {
            let when = task
                .one_shot_at
                .map(|value| value.to_chrono().to_rfc3339())
                .unwrap_or_else(|| "未设置".to_string());
            format!("一次性任务，计划时间：{} ({})", when, task.timezone)
        }
        ScheduledTaskKind::Cron => format!(
            "周期任务，Cron：{} ({})",
            task.cron_expression.as_deref().unwrap_or(""),
            task.timezone
        ),
    }
}

fn initial_task_message(task: &ScheduledTaskDoc) -> String {
    format!(
        "定时任务已创建\n标题：{}\n调度：{}\n状态：{:?}",
        task.title,
        schedule_summary(task),
        task.status
    )
}

fn fire_message_text(task: &ScheduledTaskDoc, trigger_source: &str, now: DateTime<Utc>) -> String {
    format!(
        "定时任务触发\n标题：{}\n触发来源：{}\n触发时间：{}\n调度：{}",
        task.title,
        trigger_source,
        now.to_rfc3339(),
        schedule_summary(task)
    )
}

fn next_fire_after_run(
    task: &ScheduledTaskDoc,
    trigger_source: &str,
    now: DateTime<Utc>,
) -> Result<Option<DateTime<Utc>>> {
    match task.status {
        ScheduledTaskStatus::Draft => Ok(None),
        ScheduledTaskStatus::Paused => Ok(task.next_fire_at.map(|value| value.to_chrono())),
        ScheduledTaskStatus::Completed => Ok(None),
        ScheduledTaskStatus::Deleted => Ok(None),
        ScheduledTaskStatus::Active => match task.task_kind {
            ScheduledTaskKind::OneShot => Ok(None),
            ScheduledTaskKind::Cron => {
                if trigger_source == "manual" {
                    if let Some(existing) = task.next_fire_at.map(|value| value.to_chrono()) {
                        if existing > now {
                            return Ok(Some(existing));
                        }
                    }
                }
                compute_next_fire_at(task, now)
            }
        },
    }
}

fn expected_fire_at(task: &ScheduledTaskDoc) -> Option<DateTime<Utc>> {
    task.next_fire_at.map(|value| value.to_chrono())
}

fn scheduled_trigger_source(task: &ScheduledTaskDoc, now: DateTime<Utc>) -> &'static str {
    match expected_fire_at(task) {
        Some(expected) if expected + chrono::Duration::seconds(MISSED_TASK_GRACE_SECS) < now => {
            "missed_recovery"
        }
        _ => "schedule",
    }
}

async fn create_runtime_session(
    service: &AgentService,
    task: &ScheduledTaskDoc,
    channel_name: &str,
    fire_message_id: &str,
) -> Result<String> {
    let (document_access_mode, document_scope_mode, document_write_mode) =
        scheduled_task_document_policy(task);
    let session = service
        .create_session(CreateSessionRequest {
            team_id: task.team_id.clone(),
            agent_id: task.agent_id.clone(),
            user_id: task.owner_user_id.clone(),
            name: Some(format!("Scheduled Task · {}", task.title)),
            attached_document_ids: Vec::new(),
            extra_instructions: Some(scheduled_task_contract_instructions(task)),
            allowed_extensions: scheduled_task_allowed_extensions(task),
            allowed_skill_ids: None,
            retry_config: None,
            max_turns: None,
            tool_timeout_seconds: None,
            max_portal_retry_rounds: None,
            require_final_report: false,
            portal_restricted: false,
            document_access_mode,
            document_scope_mode,
            document_write_mode,
            delegation_policy_override: None,
            session_source: Some(scheduled_task_session_source().to_string()),
            source_channel_id: Some(task.channel_id.clone()),
            source_channel_name: Some(channel_name.to_string()),
            source_thread_root_id: Some(fire_message_id.to_string()),
            hidden_from_chat_list: Some(true),
        })
        .await?;
    Ok(session.session_id)
}

pub async fn create_initial_channel_message(
    db: Arc<MongoDb>,
    task: &ScheduledTaskDoc,
) -> Result<()> {
    let channel_service = ChatChannelService::new(db);
    let Some(channel) = channel_service.get_channel(&task.channel_id).await? else {
        return Ok(());
    };
    let text = initial_task_message(task);
    channel_service
        .create_message(
            &channel,
            None,
            None,
            ChatChannelMessageSurface::Issue,
            ChatChannelThreadState::Active,
            Some(ChatChannelDisplayStatus::Active),
            ChatChannelAuthorType::System,
            None,
            None,
            "System".to_string(),
            Some(task.agent_id.clone()),
            text.clone(),
            serde_json::json!([{ "type": "text", "text": text }]),
            serde_json::json!({
                "scheduled_task": {
                    "task_id": task.task_id.clone(),
                    "kind": task.task_kind,
                    "status": task.status,
                    "schedule_summary": schedule_summary(task),
                }
            }),
        )
        .await?;
    channel_service
        .refresh_channel_stats(&task.channel_id)
        .await?;
    Ok(())
}

pub async fn start_task_run(
    db: Arc<MongoDb>,
    channel_manager: Arc<ChatChannelManager>,
    workspace_root: String,
    task: ScheduledTaskDoc,
    trigger_source: &str,
    lease_owner: String,
) -> Result<ScheduledTaskRunDoc> {
    let scheduled_service = ScheduledTaskService::new(db.clone());
    let agent_service = AgentService::new(db.clone());
    let channel_service = ChatChannelService::new(db.clone());
    let now = Utc::now();
    let channel = channel_service
        .get_channel(&task.channel_id)
        .await?
        .ok_or_else(|| anyhow!("scheduled task channel not found"))?;
    let fire_text = fire_message_text(&task, trigger_source, now);
    let fire_message = channel_service
        .create_message(
            &channel,
            None,
            None,
            ChatChannelMessageSurface::Issue,
            ChatChannelThreadState::Active,
            Some(ChatChannelDisplayStatus::Active),
            ChatChannelAuthorType::System,
            None,
            None,
            "System".to_string(),
            Some(task.agent_id.clone()),
            fire_text.clone(),
            serde_json::json!([{ "type": "text", "text": fire_text }]),
            serde_json::json!({
                "scheduled_task_fire": {
                    "task_id": task.task_id.clone(),
                    "trigger_source": trigger_source,
                    "fired_at": now.to_rfc3339(),
                }
            }),
        )
        .await?;
    let runtime_session_id = create_runtime_session(
        &agent_service,
        &task,
        &channel.name,
        &fire_message.message_id,
    )
    .await?;
    let run = scheduled_service
        .create_run(
            &task,
            trigger_source,
            &fire_message.message_id,
            &runtime_session_id,
            expected_fire_at(&task),
        )
        .await?;
    let (cancel_token, _, channel_run_id) = channel_manager
        .register(
            &task.channel_id,
            fire_message.message_id.clone(),
            Some(fire_message.message_id.clone()),
            Some(fire_message.message_id.clone()),
        )
        .await
        .ok_or_else(|| anyhow!("scheduled task channel already has an active run"))?;

    if let Err(error) = channel_service
        .set_processing(&task.channel_id, &channel_run_id, true)
        .await
    {
        channel_manager
            .unregister(&task.channel_id, Some(&fire_message.message_id))
            .await;
        return Err(error.into());
    }

    let workspace_root_for_eval = workspace_root.clone();
    let executor = ChatChannelExecutor::new(db.clone(), channel_manager.clone(), workspace_root);
    let request = ExecuteChannelMessageRequest {
        channel_id: task.channel_id.clone(),
        root_message_id: fire_message.message_id.clone(),
        run_scope_id: fire_message.message_id.clone(),
        thread_root_id: Some(fire_message.message_id.clone()),
        assistant_parent_message_id: fire_message.message_id.clone(),
        surface: ChatChannelMessageSurface::Issue,
        thread_state: ChatChannelThreadState::Active,
        interaction_mode: crate::agent::chat_channels::ChatChannelInteractionMode::Execution,
        internal_session_id: runtime_session_id.clone(),
        agent_id: task.agent_id.clone(),
        user_message: task.prompt.clone(),
    };
    let improvement_request = request.clone();

    let task_snapshot = task.clone();
    let run_id = run.run_id.clone();
    let trigger_source = trigger_source.to_string();
    tokio::spawn(async move {
        let exec_result = executor
            .execute_channel_message(request, cancel_token)
            .await;
        let service = ScheduledTaskService::new(db.clone());
        let agent_service = AgentService::new(db.clone());
        let session = agent_service
            .get_session(&runtime_session_id)
            .await
            .ok()
            .flatten();
        let status_text = session
            .as_ref()
            .and_then(|item| item.last_execution_status.as_deref())
            .unwrap_or_else(|| {
                if exec_result.is_ok() {
                    "completed"
                } else {
                    "failed"
                }
            });
        let error = session
            .as_ref()
            .and_then(|item| item.last_execution_error.clone())
            .or_else(|| exec_result.as_ref().err().map(|err| err.to_string()));
        let summary = session
            .as_ref()
            .and_then(|item| item.last_message_preview.clone());
        let (status, outcome_reason, warning_count, summary, error) = classify_run_completion(
            &task_snapshot,
            status_text,
            summary,
            error,
            session
                .as_ref()
                .and_then(|item| item.workspace_path.as_deref()),
            session.as_ref().map(|item| item.messages_json.as_str()),
        );
        let summary_for_eval = summary.clone();
        let error_for_eval = error.clone();
        let next_fire_at = next_fire_after_run(&task_snapshot, &trigger_source, Utc::now())
            .ok()
            .flatten();
        if let Err(error) = service
            .complete_run(
                &task_snapshot,
                &run_id,
                status,
                outcome_reason,
                warning_count,
                summary,
                error,
                None,
                next_fire_at,
                &lease_owner,
            )
            .await
        {
            tracing::error!(
                "scheduled task completion persistence failed (task={}, run={}): {}",
                task_snapshot.task_id,
                run_id,
                error
            );
        }
        let db_for_eval = db.clone();
        let task_for_eval = task_snapshot.clone();
        let run_id_for_eval = run_id.clone();
        let runtime_session_for_eval = runtime_session_id.clone();
        let status_for_eval = status;
        let outcome_reason_for_eval = outcome_reason;
        let warning_count_for_eval = warning_count;
        let channel_manager_for_eval = channel_manager.clone();
        tokio::spawn(async move {
            let eval_service = ScheduledTaskService::new(db_for_eval.clone());
            let eval_agent_service = AgentService::new(db_for_eval.clone());
            let eval_session = eval_agent_service
                .get_session(&runtime_session_for_eval)
                .await
                .ok()
                .flatten();
            let self_evaluation = generate_self_evaluation(
                db_for_eval.clone(),
                &task_for_eval,
                &eval_agent_service,
                eval_session
                    .as_ref()
                    .and_then(|item| item.workspace_path.as_deref()),
                status_for_eval,
                outcome_reason_for_eval,
                warning_count_for_eval,
                summary_for_eval.as_deref(),
                error_for_eval.as_deref(),
            )
            .await;
            let mut final_evaluation = self_evaluation.clone();
            let mut initial_evaluation = None;
            let mut improvement_loop_applied = false;
            let mut improvement_loop_count = 0;
            if let Some(ref evaluation) = self_evaluation {
                tracing::info!(
                    "scheduled task self evaluation completed (task={}, run={}, score={}, grade={:?}, goal_completion={}, result_quality={}, evidence_quality={}, execution_stability={})",
                    task_for_eval.task_id,
                    run_id_for_eval,
                    evaluation.score,
                    evaluation.grade,
                    evaluation.goal_completion,
                    evaluation.result_quality,
                    evaluation.evidence_quality,
                    evaluation.execution_stability,
                );
                if should_trigger_improvement_loop(evaluation) {
                    tracing::info!(
                        "scheduled task improvement loop triggered (task={}, run={}, score={}, grade={:?})",
                        task_for_eval.task_id,
                        run_id_for_eval,
                        evaluation.score,
                        evaluation.grade,
                    );
                    initial_evaluation = Some(evaluation.clone());
                    improvement_loop_applied = true;
                    improvement_loop_count = 1;
                    let mut retry_request = improvement_request.clone();
                    retry_request.user_message =
                        improvement_loop_prompt(&task_for_eval, evaluation);
                    let improvement_result = if let Some((retry_cancel, _, _)) =
                        channel_manager_for_eval
                            .register(
                                &retry_request.channel_id,
                                retry_request.run_scope_id.clone(),
                                retry_request.thread_root_id.clone(),
                                Some(retry_request.root_message_id.clone()),
                            )
                            .await
                    {
                        let improvement_executor = ChatChannelExecutor::new(
                            db_for_eval.clone(),
                            channel_manager_for_eval.clone(),
                            workspace_root_for_eval,
                        );
                        improvement_executor
                            .execute_channel_message(retry_request, retry_cancel)
                            .await
                    } else {
                        Err(anyhow!(
                            "scheduled task improvement loop could not register an active channel run"
                        ))
                    };
                    if improvement_result.is_ok() {
                        let refreshed_session = eval_agent_service
                            .get_session(&runtime_session_for_eval)
                            .await
                            .ok()
                            .flatten();
                        let refreshed_status_text = refreshed_session
                            .as_ref()
                            .and_then(|item| item.last_execution_status.as_deref())
                            .unwrap_or("completed");
                        let refreshed_error = refreshed_session
                            .as_ref()
                            .and_then(|item| item.last_execution_error.clone());
                        let refreshed_summary = refreshed_session
                            .as_ref()
                            .and_then(|item| item.last_message_preview.clone());
                        let (
                            refreshed_status,
                            refreshed_outcome_reason,
                            refreshed_warning_count,
                            refreshed_summary,
                            refreshed_error,
                        ) = classify_run_completion(
                            &task_for_eval,
                            refreshed_status_text,
                            refreshed_summary,
                            refreshed_error,
                            refreshed_session
                                .as_ref()
                                .and_then(|item| item.workspace_path.as_deref()),
                            refreshed_session
                                .as_ref()
                                .map(|item| item.messages_json.as_str()),
                        );
                        if let Err(error) = eval_service
                            .update_run_result_projection(
                                &task_for_eval.task_id,
                                &run_id_for_eval,
                                refreshed_status,
                                refreshed_outcome_reason,
                                refreshed_warning_count,
                                refreshed_summary.clone(),
                                refreshed_error.clone(),
                            )
                            .await
                        {
                            tracing::warn!(
                                "scheduled task improvement projection update failed (task={}, run={}): {}",
                                task_for_eval.task_id,
                                run_id_for_eval,
                                error
                            );
                        } else {
                            tracing::info!(
                                "scheduled task improvement projection updated (task={}, run={}, status={:?}, outcome_reason={:?})",
                                task_for_eval.task_id,
                                run_id_for_eval,
                                refreshed_status,
                                refreshed_outcome_reason,
                            );
                        }
                        final_evaluation = generate_self_evaluation(
                            db_for_eval.clone(),
                            &task_for_eval,
                            &eval_agent_service,
                            refreshed_session
                                .as_ref()
                                .and_then(|item| item.workspace_path.as_deref()),
                            refreshed_status,
                            refreshed_outcome_reason,
                            refreshed_warning_count,
                            refreshed_summary.as_deref(),
                            refreshed_error.as_deref(),
                        )
                        .await;
                    } else if let Err(error) = improvement_result {
                        tracing::warn!(
                            "scheduled task improvement loop execution failed (task={}, run={}): {}",
                            task_for_eval.task_id,
                            run_id_for_eval,
                            error
                        );
                    }
                } else {
                    tracing::info!(
                        "scheduled task improvement loop skipped (task={}, run={}, score={}, grade={:?})",
                        task_for_eval.task_id,
                        run_id_for_eval,
                        evaluation.score,
                        evaluation.grade,
                    );
                }
            }
            if let Some(self_evaluation) = final_evaluation {
                if let Err(error) = eval_service
                    .update_run_self_evaluation(
                        &task_for_eval.task_id,
                        &run_id_for_eval,
                        &self_evaluation,
                        initial_evaluation.as_ref(),
                        improvement_loop_applied,
                        improvement_loop_count,
                    )
                    .await
                {
                    tracing::warn!(
                        "scheduled task self evaluation persistence failed (task={}, run={}): {}",
                        task_for_eval.task_id,
                        run_id_for_eval,
                        error
                    );
                }
            }
        });
    });

    Ok(run)
}

pub fn spawn_scheduler(
    db: Arc<MongoDb>,
    channel_manager: Arc<ChatChannelManager>,
    workspace_root: String,
) {
    tokio::spawn(async move {
        let service = ScheduledTaskService::new(db.clone());
        if let Err(error) = service.ensure_indexes().await {
            tracing::error!(
                "scheduled task scheduler failed to ensure indexes: {}",
                error
            );
            return;
        }
        tracing::info!("scheduled task scheduler started");
        loop {
            tokio::time::sleep(Duration::from_secs(SCHEDULER_TICK_SECS)).await;
            let now = Utc::now();
            match service.expire_session_scoped_tasks(now).await {
                Ok(expired) if !expired.is_empty() => {
                    tracing::info!(
                        expired_count = expired.len(),
                        "scheduled task scheduler expired session-scoped tasks"
                    );
                }
                Ok(_) => {}
                Err(error) => {
                    tracing::error!(
                        "scheduled task scheduler failed to expire session-scoped tasks: {}",
                        error
                    );
                }
            }
            let due = match service
                .claim_due_tasks(now, &scheduler_owner(), LEASE_SECS, 8)
                .await
            {
                Ok(items) => items,
                Err(error) => {
                    tracing::error!("scheduled task claim_due_tasks failed: {}", error);
                    continue;
                }
            };
            for task in due {
                let trigger_source = scheduled_trigger_source(&task, now);
                if let Err(error) = start_task_run(
                    db.clone(),
                    channel_manager.clone(),
                    workspace_root.clone(),
                    task.clone(),
                    trigger_source,
                    scheduler_owner(),
                )
                .await
                {
                    tracing::error!(
                        "scheduled task fire failed (task={}): {}",
                        task.task_id,
                        error
                    );
                    let _ = service
                        .release_lease(&task.team_id, &task.task_id, &scheduler_owner())
                        .await;
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scheduled_tasks::models::{
        ScheduledTaskDeliveryPlanKind, ScheduledTaskDeliveryTier, ScheduledTaskPayloadKind,
        ScheduledTaskSessionBinding,
    };

    fn sample_task(kind: ScheduledTaskKind) -> ScheduledTaskDoc {
        let now = bson::DateTime::now();
        ScheduledTaskDoc {
            id: None,
            task_id: "task-1".to_string(),
            team_id: "team-1".to_string(),
            channel_id: "channel-1".to_string(),
            owner_user_id: "user-1".to_string(),
            agent_id: "agent-1".to_string(),
            title: "Task".to_string(),
            prompt: "Do the thing".to_string(),
            task_kind: kind,
            task_profile: ScheduledTaskProfile::WorkspaceTask,
            payload_kind: ScheduledTaskPayloadKind::SystemSummary,
            session_binding: ScheduledTaskSessionBinding::IsolatedTask,
            delivery_plan: ScheduledTaskDeliveryPlanKind::ChannelOnly,
            execution_contract: ScheduledTaskExecutionContract {
                output_mode: ScheduledTaskOutputMode::SummaryOnly,
                must_return_final_text: true,
                allow_partial_result: true,
                artifact_path: None,
                publish_behavior: ScheduledTaskPublishBehavior::None,
                source_scope: super::super::models::ScheduledTaskSourceScope::WorkspaceOnly,
                source_policy: None,
                minimum_source_attempts: None,
                minimum_successful_sources: None,
                prefer_structured_sources: None,
                allow_query_retry: None,
                fallback_to_secondary_sources: None,
                required_sections: Vec::new(),
            },
            delivery_tier: ScheduledTaskDeliveryTier::Durable,
            owner_session_id: None,
            one_shot_at: Some(bson::DateTime::from_chrono(
                Utc::now() + chrono::Duration::minutes(5),
            )),
            cron_expression: Some("0 9 * * *".to_string()),
            schedule_config: None,
            timezone: "Asia/Shanghai".to_string(),
            status: ScheduledTaskStatus::Active,
            next_fire_at: None,
            last_fire_at: None,
            last_run_id: None,
            last_expected_fire_at: None,
            last_missed_at: None,
            missed_fire_count: 0,
            lease_owner: None,
            lease_expires_at: None,
            created_at: now,
            updated_at: now,
        }
    }

    #[test]
    fn compute_next_fire_for_one_shot_uses_explicit_timestamp() {
        let task = sample_task(ScheduledTaskKind::OneShot);
        let next = compute_next_fire_at(&task, Utc::now()).unwrap();
        assert_eq!(next, task.one_shot_at.map(|value| value.to_chrono()));
    }

    #[test]
    fn compute_next_fire_for_cron_respects_timezone() {
        let mut task = sample_task(ScheduledTaskKind::Cron);
        task.cron_expression = Some("0 9 * * *".to_string());
        task.timezone = "Asia/Shanghai".to_string();
        let now = chrono::DateTime::parse_from_rfc3339("2026-04-12T02:30:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let next = compute_next_fire_at(&task, now).unwrap().unwrap();
        assert_eq!(
            next,
            chrono::DateTime::parse_from_rfc3339("2026-04-13T01:00:00Z")
                .unwrap()
                .with_timezone(&Utc)
        );
    }

    #[test]
    fn scheduled_trigger_source_marks_overdue_tasks_as_missed_recovery() {
        let mut task = sample_task(ScheduledTaskKind::OneShot);
        task.next_fire_at = Some(bson::DateTime::from_chrono(
            Utc::now() - chrono::Duration::seconds(MISSED_TASK_GRACE_SECS + 10),
        ));
        assert_eq!(
            scheduled_trigger_source(&task, Utc::now()),
            "missed_recovery"
        );
    }

    #[test]
    fn scheduled_trigger_source_keeps_recent_due_task_as_schedule() {
        let mut task = sample_task(ScheduledTaskKind::OneShot);
        task.next_fire_at = Some(bson::DateTime::from_chrono(
            Utc::now() - chrono::Duration::seconds(1),
        ));
        assert_eq!(scheduled_trigger_source(&task, Utc::now()), "schedule");
    }

    #[test]
    fn classify_run_completion_rejects_document_workspace_handoff_summary() {
        let mut task = sample_task(ScheduledTaskKind::OneShot);
        task.task_profile = ScheduledTaskProfile::DocumentTask;

        let (status, outcome_reason, _, summary, error) = classify_run_completion(
            &task,
            "completed",
            Some(
                "Document access established and the file was materialized into the workspace. Use developer shell, MCP, or another local tool to read the file content from the workspace path."
                    .to_string(),
            ),
            None,
            None,
            None,
        );

        assert_eq!(status, ScheduledTaskRunStatus::Failed);
        assert_eq!(
            outcome_reason,
            Some(ScheduledTaskRunOutcomeReason::FailedNoFinalAnswer)
        );
        assert_eq!(summary, None);
        assert_eq!(
            error.as_deref(),
            Some("runtime completed without a user-visible final answer")
        );
    }
}
