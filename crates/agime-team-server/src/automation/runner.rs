use std::sync::Arc;

use agime_team::MongoDb;
use anyhow::{anyhow, Result};
use reqwest::{
    header::{HeaderMap, HeaderName, HeaderValue},
    Method,
};
use serde_json::{json, Value};

use crate::agent::{chat_executor::ChatExecutor, service_mongo::AgentService, ChatManager};

use super::service::AutomationService;
use super::{
    contract::{
        auth_headers_for_integration, derive_builder_sync_payload, extract_last_assistant_text,
        parse_execution_contract, VerifiedHttpAction,
    },
    models::{
        AutomationIntegrationDoc, AutomationModuleDoc, AutomationTaskDraftDoc, DraftStatus,
        RunMode, RunStatus,
    },
};

#[derive(Clone)]
pub struct AutomationRunner {
    db: Arc<MongoDb>,
    chat_manager: Arc<ChatManager>,
    workspace_root: String,
}

enum ExecutionTarget {
    Draft {
        team_id: String,
        draft_id: String,
    },
    Run {
        team_id: String,
        project_id: String,
        run_id: String,
    },
}

impl AutomationRunner {
    pub fn new(db: Arc<MongoDb>, chat_manager: Arc<ChatManager>, workspace_root: String) -> Self {
        Self {
            db,
            chat_manager,
            workspace_root,
        }
    }

    pub async fn start_probe_for_draft(
        &self,
        team_id: &str,
        draft: &AutomationTaskDraftDoc,
        actor_user_id: &str,
    ) -> Result<String> {
        let service = AutomationService::new(self.db.clone());
        let session_id = self
            .ensure_builder_session_for_draft(team_id, draft, actor_user_id)
            .await?;
        let integrations = service
            .get_integrations_by_ids(team_id, &draft.integration_ids)
            .await?;
        let prompt = build_probe_prompt(draft, &integrations);
        service
            .update_task_draft_probe_state(
                team_id,
                &draft.draft_id,
                DraftStatus::Probing,
                Some(session_id.clone()),
                Some(prompt.clone()),
            )
            .await?;
        self.spawn_execution(
            session_id.clone(),
            draft.driver_agent_id.clone(),
            actor_user_id.to_string(),
            prompt,
            ExecutionTarget::Draft {
                team_id: team_id.to_string(),
                draft_id: draft.draft_id.clone(),
            },
        )
        .await?;
        Ok(session_id)
    }

    pub async fn ensure_builder_session_for_draft(
        &self,
        team_id: &str,
        draft: &AutomationTaskDraftDoc,
        actor_user_id: &str,
    ) -> Result<String> {
        if let Some(existing) = draft.builder_session_id.clone() {
            return Ok(existing);
        }

        let service = AutomationService::new(self.db.clone());
        let agent_service = AgentService::new(self.db.clone());
        let integrations = service
            .get_integrations_by_ids(team_id, &draft.integration_ids)
            .await?;
        let session = agent_service
            .create_chat_session(
                team_id,
                &draft.driver_agent_id,
                actor_user_id,
                vec![],
                Some(build_builder_context_prompt(draft, &integrations)),
                Some(foundry_builder_allowed_extensions()),
                None,
                None,
                None,
                None,
                None,
                false,
                false,
                None,
                None,
                Some(automation_builder_session_source().to_string()),
                Some(true),
            )
            .await?;

        let _ = service
            .update_task_draft_probe_state(
                team_id,
                &draft.draft_id,
                draft.status.clone(),
                Some(session.session_id.clone()),
                None,
            )
            .await?;

        Ok(session.session_id)
    }

    pub async fn start_module_run(
        &self,
        team_id: &str,
        module: &AutomationModuleDoc,
        actor_user_id: &str,
        mode: RunMode,
        schedule_id: Option<String>,
        session_owner_user_id: Option<String>,
    ) -> Result<super::models::AutomationRunDoc> {
        let service = AutomationService::new(self.db.clone());
        let integrations = service
            .get_integrations_by_ids(team_id, &module.integration_ids)
            .await?;
        let runtime_session_id = self
            .ensure_runtime_session_for_module(team_id, module, actor_user_id)
            .await?;
        if mode != RunMode::Chat {
            let native_actions = native_http_actions_for_module(module);
            if !native_actions.is_empty() {
                let run = service
                    .create_run(
                        team_id,
                        &module.project_id,
                        &module.module_id,
                        module.version,
                        mode.clone(),
                        actor_user_id,
                        schedule_id.clone(),
                        None,
                    )
                    .await?;
                self.spawn_native_http_execution(
                    team_id.to_string(),
                    module.project_id.clone(),
                    run.run_id.clone(),
                    module.name.clone(),
                    mode.clone(),
                    runtime_session_id.clone(),
                    integrations,
                    native_actions,
                );
                return Ok(run);
            }
        }
        if mode == RunMode::Chat {
            let run = service
                .create_run(
                    team_id,
                    &module.project_id,
                    &module.module_id,
                    module.version,
                    mode,
                    actor_user_id,
                    schedule_id,
                    Some(runtime_session_id),
                )
                .await?;
            let _ = service
                .complete_run(
                    team_id,
                    &run.run_id,
                    RunStatus::Completed,
                    Some("app_runtime_session_ready".to_string()),
                    None,
                )
                .await;
            return Ok(service.get_run(team_id, &run.run_id).await?.unwrap_or(run));
        }
        let agent_service = AgentService::new(self.db.clone());
        let session_user_id = session_owner_user_id.unwrap_or_else(|| actor_user_id.to_string());
        let session = agent_service
            .create_chat_session(
                team_id,
                &module.driver_agent_id,
                &session_user_id,
                vec![],
                Some("这是实验室 > API 自动化运行会话，只服务当前模块执行和结果产出。".to_string()),
                Some(foundry_runtime_allowed_extensions()),
                None,
                None,
                None,
                None,
                None,
                false,
                false,
                None,
                None,
                Some(automation_run_session_source().to_string()),
                Some(true),
            )
            .await?;
        let run = service
            .create_run(
                team_id,
                &module.project_id,
                &module.module_id,
                module.version,
                mode.clone(),
                actor_user_id,
                schedule_id.clone(),
                Some(session.session_id.clone()),
            )
            .await?;
        let prompt = build_module_run_prompt(module, &integrations, &mode);
        self.spawn_execution(
            session.session_id,
            module.driver_agent_id.clone(),
            actor_user_id.to_string(),
            prompt,
            ExecutionTarget::Run {
                team_id: team_id.to_string(),
                project_id: module.project_id.clone(),
                run_id: run.run_id.clone(),
            },
        )
        .await?;
        Ok(run)
    }

    pub async fn ensure_runtime_session_for_module(
        &self,
        team_id: &str,
        module: &AutomationModuleDoc,
        actor_user_id: &str,
    ) -> Result<String> {
        if let Some(existing) = module.runtime_session_id.clone() {
            return Ok(existing);
        }
        let service = AutomationService::new(self.db.clone());
        let agent_service = AgentService::new(self.db.clone());
        let integrations = service
            .get_integrations_by_ids(team_id, &module.integration_ids)
            .await?;
        let session = agent_service
            .create_chat_session(
                team_id,
                &module.driver_agent_id,
                actor_user_id,
                vec![],
                Some(build_runtime_context_prompt(module, &integrations)),
                Some(foundry_runtime_allowed_extensions()),
                None,
                None,
                None,
                None,
                None,
                false,
                false,
                None,
                None,
                Some(automation_runtime_session_source().to_string()),
                Some(false),
            )
            .await?;
        let _ = service
            .update_module_runtime_session(team_id, &module.module_id, &session.session_id)
            .await?;
        Ok(session.session_id)
    }

    async fn spawn_execution(
        &self,
        session_id: String,
        agent_id: String,
        actor_user_id: String,
        prompt: String,
        target: ExecutionTarget,
    ) -> Result<()> {
        let agent_service = AgentService::new(self.db.clone());
        let (cancel_token, _stream_tx) = self
            .chat_manager
            .register(&session_id)
            .await
            .ok_or_else(|| anyhow!("Session is already processing"))?;
        let claimed = agent_service
            .try_start_processing(&session_id, &actor_user_id)
            .await?;
        if !claimed {
            return Err(anyhow!("Session is already processing"));
        }

        let db = self.db.clone();
        let chat_manager = self.chat_manager.clone();
        let workspace_root = self.workspace_root.clone();
        tokio::spawn(async move {
            let executor = ChatExecutor::new(db.clone(), chat_manager.clone(), workspace_root);
            let result = executor
                .execute_chat(&session_id, &agent_id, &prompt, cancel_token)
                .await;
            let automation_service = AutomationService::new(db.clone());
            let agent_service = AgentService::new(db.clone());
            let session = agent_service.get_session(&session_id).await.ok().flatten();
            match target {
                ExecutionTarget::Draft { team_id, draft_id } => {
                    if let Some(session) = session.as_ref() {
                        if let Ok(Some(draft_doc)) =
                            automation_service.get_task_draft(&team_id, &draft_id).await
                        {
                            let integrations = automation_service
                                .get_integrations_by_ids(&team_id, &draft_doc.integration_ids)
                                .await
                                .unwrap_or_default();
                            let sync_payload = derive_builder_sync_payload(
                                &session_id,
                                &session.messages_json,
                                session.last_message_preview.as_deref(),
                                session.last_execution_status.as_deref().unwrap_or(
                                    if result.is_ok() {
                                        "completed"
                                    } else {
                                        "failed"
                                    },
                                ),
                                draft_doc.status,
                                &integrations,
                            );
                            let _ = automation_service
                                .complete_task_draft_probe(
                                    &team_id,
                                    &draft_id,
                                    sync_payload.status,
                                    sync_payload.probe_report,
                                    sync_payload.candidate_plan,
                                )
                                .await;
                        }
                    }
                }
                ExecutionTarget::Run {
                    team_id,
                    project_id,
                    run_id,
                } => {
                    let run_status = if result.is_ok() {
                        RunStatus::Completed
                    } else {
                        RunStatus::Failed
                    };
                    let error_text = result.as_ref().err().map(|err| err.to_string());
                    let preview = session
                        .as_ref()
                        .and_then(|s| s.last_message_preview.clone());
                    let last_text = session
                        .as_ref()
                        .and_then(|s| extract_last_assistant_text(&s.messages_json));
                    if let Some(text) = last_text.clone() {
                        let _ = automation_service
                            .create_artifact(
                                &team_id,
                                &project_id,
                                &run_id,
                                super::models::ArtifactKind::Deliverable,
                                "run-summary.md",
                                Some("text/markdown".to_string()),
                                Some(text.chars().take(16000).collect()),
                                serde_json::json!({ "session_id": session_id }),
                            )
                            .await;
                    }
                    let _ = automation_service
                        .complete_run(
                            &team_id,
                            &run_id,
                            run_status,
                            preview.clone(),
                            error_text.clone(),
                        )
                        .await;
                    if let Ok(Some(run_doc)) = automation_service.get_run(&team_id, &run_id).await {
                        if let Ok(Some(module_doc)) = automation_service
                            .get_module(&team_id, &run_doc.module_id)
                            .await
                        {
                            if let Some(runtime_session_id) =
                                module_doc.runtime_session_id.as_deref()
                            {
                                let notice = format_runtime_notice(
                                    &module_doc.name,
                                    &run_doc.mode,
                                    last_text.as_deref().or(preview.as_deref()),
                                    error_text.as_deref(),
                                );
                                let _ = agent_service
                                    .append_visible_session_notice(runtime_session_id, &notice)
                                    .await;
                            }
                        }
                    }
                }
            }
        });
        Ok(())
    }

    fn spawn_native_http_execution(
        &self,
        team_id: String,
        project_id: String,
        run_id: String,
        module_name: String,
        mode: RunMode,
        runtime_session_id: String,
        integrations: Vec<AutomationIntegrationDoc>,
        actions: Vec<VerifiedHttpAction>,
    ) {
        let db = self.db.clone();
        tokio::spawn(async move {
            let automation_service = AutomationService::new(db.clone());
            let agent_service = AgentService::new(db.clone());
            let outcome = execute_native_http_actions(&integrations, &actions).await;
            match outcome {
                Ok(result) => {
                    let _ = automation_service
                        .create_artifact(
                            &team_id,
                            &project_id,
                            &run_id,
                            super::models::ArtifactKind::Deliverable,
                            "run-summary.md",
                            Some("text/markdown".to_string()),
                            Some(result.summary_markdown.clone()),
                            json!({ "execution_mode": "native_http" }),
                        )
                        .await;
                    let _ = automation_service
                        .create_artifact(
                            &team_id,
                            &project_id,
                            &run_id,
                            super::models::ArtifactKind::Supporting,
                            "http-results.json",
                            Some("application/json".to_string()),
                            Some(
                                serde_json::to_string_pretty(&result.results)
                                    .unwrap_or_else(|_| "[]".to_string()),
                            ),
                            json!({ "execution_mode": "native_http" }),
                        )
                        .await;
                    let _ = automation_service
                        .complete_run(
                            &team_id,
                            &run_id,
                            RunStatus::Completed,
                            Some(result.summary_markdown.chars().take(4000).collect()),
                            None,
                        )
                        .await;
                    let _ = agent_service
                        .append_visible_session_notice(
                            &runtime_session_id,
                            &format_runtime_notice(
                                &module_name,
                                &mode,
                                Some(result.summary_markdown.as_str()),
                                None,
                            ),
                        )
                        .await;
                }
                Err(error) => {
                    let error_text = error.to_string();
                    let _ = automation_service
                        .complete_run(
                            &team_id,
                            &run_id,
                            RunStatus::Failed,
                            Some("native_http_execution_failed".to_string()),
                            Some(error_text.clone()),
                        )
                        .await;
                    let _ = agent_service
                        .append_visible_session_notice(
                            &runtime_session_id,
                            &format_runtime_notice(
                                &module_name,
                                &mode,
                                None,
                                Some(error_text.as_str()),
                            ),
                        )
                        .await;
                }
            }
        });
    }
}

#[derive(Debug)]
struct NativeHttpExecutionResult {
    summary_markdown: String,
    results: Vec<Value>,
}

fn native_http_actions_for_module(module: &AutomationModuleDoc) -> Vec<VerifiedHttpAction> {
    parse_execution_contract(&module.execution_contract)
        .filter(|contract| contract.native_execution_ready)
        .map(|contract| contract.verified_http_actions)
        .unwrap_or_default()
}

async fn execute_native_http_actions(
    integrations: &[AutomationIntegrationDoc],
    actions: &[VerifiedHttpAction],
) -> Result<NativeHttpExecutionResult> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .build()?;
    let mut results = Vec::new();
    let mut summary_sections = vec!["## 原生 API 执行摘要".to_string()];

    for (index, action) in actions.iter().enumerate() {
        let mut request = client.request(
            Method::from_bytes(action.request.method.as_bytes()).unwrap_or(Method::GET),
            &action.request.url,
        );

        let mut headers = HeaderMap::new();
        for (name, value) in &action.request.headers {
            if let (Ok(header_name), Ok(header_value)) = (
                HeaderName::from_bytes(name.as_bytes()),
                HeaderValue::from_str(value),
            ) {
                headers.insert(header_name, header_value);
            }
        }

        if let Some(integration_id) = action.request.integration_id.as_ref() {
            if let Some(integration) = integrations
                .iter()
                .find(|item| &item.integration_id == integration_id)
            {
                for (name, value) in auth_headers_for_integration(integration) {
                    if let (Ok(header_name), Ok(header_value)) = (
                        HeaderName::from_bytes(name.as_bytes()),
                        HeaderValue::from_str(&value),
                    ) {
                        headers.entry(header_name).or_insert(header_value);
                    }
                }
            }
        }

        if !headers.is_empty() {
            request = request.headers(headers);
        }
        if let Some(body) = action.request.body.as_ref() {
            request = request.body(body.clone());
        }

        let response = request.send().await?;
        let status_code = response.status().as_u16();
        let response_headers = response.headers().clone();
        let body_text = response.text().await?;
        let parsed_json = serde_json::from_str::<Value>(&body_text).ok();
        let content_type = response_headers
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .map(|value| value.to_string());

        let result = json!({
            "index": index,
            "request": {
                "method": action.request.method,
                "url": action.request.url,
                "integration_id": action.request.integration_id,
            },
            "response": {
                "status_code": status_code,
                "content_type": content_type.clone(),
                "body_text": body_text.chars().take(4000).collect::<String>(),
                "json_body": parsed_json.clone(),
            },
        });
        results.push(result);

        summary_sections.push(format!(
            "### 调用 {index}\n- 方法：`{method}`\n- URL：`{url}`\n- 状态码：`{status}`\n- Content-Type：`{content_type}`",
            index = index + 1,
            method = action.request.method,
            url = action.request.url,
            status = status_code,
            content_type = content_type.unwrap_or_else(|| "unknown".to_string()),
        ));
        summary_sections.push(format!(
            "```json\n{}\n```",
            parsed_json
                .clone()
                .unwrap_or_else(|| json!(body_text.chars().take(2000).collect::<String>()))
        ));

        if status_code >= 400 {
            return Err(anyhow!(
                "Native API call failed for {} {} with status {}",
                action.request.method,
                action.request.url,
                status_code
            ));
        }
    }

    Ok(NativeHttpExecutionResult {
        summary_markdown: summary_sections.join("\n\n"),
        results,
    })
}

fn build_probe_prompt(
    draft: &AutomationTaskDraftDoc,
    integrations: &[super::models::AutomationIntegrationDoc],
) -> String {
    let integration_lines = integrations
        .iter()
        .map(compact_integration_context)
        .collect::<Vec<_>>()
        .join("\n\n");

    format!(
        "你正在为一个 Agent App 草稿生成最小可用方案。\n\n任务名称：{name}\n目标：{goal}\n约束：{constraints}\n成功标准：{success}\n风险偏好：{risk}\n\n规则：\n- 优先使用 `api_tools__http_request` 做真实 API 调用；必要时才回退到 developer。\n- 不要使用浏览器/Playwright。\n- 只选择支撑当前目标的最小必要接口。\n- 信息不足时先列缺失项，不要编造接口。\n- 输出尽量短，不要复述整段接口文档。\n- 如果资料中已经明确给出了 base_url 和可执行 endpoint，在给出“可发布/ready”结论前，必须至少完成一次真实 `api_tools__http_request`。\n- 没有真实 HTTP 验证证据时，不得宣称应用已经 ready 或可直接发布。\n\n请按这个顺序输出：\n1. 可行性\n2. 已验证或推荐的调用路径\n3. 缺失信息\n4. 下一步\n5. 推荐运行方式\n\n{integration_lines}",
        name = draft.name,
        goal = draft.goal,
        constraints = if draft.constraints.is_empty() {
            "（无）".to_string()
        } else {
            draft.constraints.join("；")
        },
        success = if draft.success_criteria.is_empty() {
            "（无）".to_string()
        } else {
            draft.success_criteria.join("；")
        },
        risk = draft.risk_preference,
        integration_lines = integration_lines
    )
}

fn build_builder_context_prompt(
    draft: &AutomationTaskDraftDoc,
    integrations: &[super::models::AutomationIntegrationDoc],
) -> String {
    let integration_lines = if integrations.is_empty() {
        "当前还没有导入可用 API 资料。需要时请主动要求用户补充 API 文档、链接、base_url 或 auth 信息。".to_string()
    } else {
        integrations
            .iter()
            .map(compact_integration_context)
            .collect::<Vec<_>>()
            .join("\n\n")
    };

    format!(
        "你正在 Agentify｜万物智能 的 Builder 会话中工作。\n\n当前草稿：{name}\n目标：{goal}\n约束：{constraints}\n成功标准：{success}\n风险偏好：{risk}\n\n职责：\n1. 理解用户导入的 API 资料。\n2. 为目标生成一个可持续对话、可运行的 Agent App。\n3. 找出最小可行 API 路径并真实验证。\n4. 允许多软件组合编排，但不要硬编码行业模板。\n5. 信息不足时优先追问。\n6. 默认输出高层摘要，不要长篇复述接口文档。\n7. 优先使用 `api_tools__http_request`；必要时才回退到 developer。\n8. 不使用浏览器/Playwright。\n9. 如果资料里已经给出明确的 base_url 和 endpoint，就先完成真实 `api_tools__http_request` 验证，再谈是否可发布。\n10. 没有真实 HTTP 验证证据时，不要说 ready、不要说可以直接发布。\n\n回答优先给：已确认能力、缺失信息、下一步建议、是否可发布。\n\n{integration_lines}",
        name = draft.name,
        goal = draft.goal,
        constraints = if draft.constraints.is_empty() {
            "（无）".to_string()
        } else {
            draft.constraints.join("；")
        },
        success = if draft.success_criteria.is_empty() {
            "（无）".to_string()
        } else {
            draft.success_criteria.join("；")
        },
        risk = draft.risk_preference,
        integration_lines = integration_lines
    )
}

fn build_runtime_context_prompt(
    module: &AutomationModuleDoc,
    integrations: &[super::models::AutomationIntegrationDoc],
) -> String {
    let integration_lines = if integrations.is_empty() {
        "当前没有绑定可用资料。需要时先提醒用户补充集成信息。".to_string()
    } else {
        integrations
            .iter()
            .map(|integration| {
                format!(
                    "- {name} ({kind:?}) {base}",
                    name = integration.name,
                    kind = integration.spec_kind,
                    base = integration
                        .base_url
                        .clone()
                        .unwrap_or_else(|| "(未设置 base_url)".to_string()),
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    format!(
        "你正在实验室 > Agentify｜万物智能 的已发布 Agent App 运行会话中工作。\n\n应用名称：{name}\n版本：v{version}\n\n应用意图：\n```json\n{intent}\n```\n\n已接入系统：\n{integrations}\n\n工作原则：\n- 这是一个持续对话应用，不是一次性任务。\n- 优先沿用已验证的 API 路径与 execution contract。\n- 优先使用 `api_tools__http_request` 做真实 API 调用；必要时才回退到 developer 工具。\n- 不依赖浏览器/Playwright。\n- 用户可以自然语言让你查询、执行、解释或继续构建跨系统协同动作。\n- 如果需要更多连接信息或写操作风险确认，先明确说明。\n",
        name = module.name,
        version = module.version,
        intent = serde_json::to_string_pretty(&module.intent_snapshot).unwrap_or_default(),
        integrations = integration_lines,
    )
}

fn build_module_run_prompt(
    module: &AutomationModuleDoc,
    integrations: &[super::models::AutomationIntegrationDoc],
    mode: &RunMode,
) -> String {
    let intent_snapshot = serde_json::to_string_pretty(&module.intent_snapshot).unwrap_or_default();
    let integration_lines = integrations
        .iter()
        .map(|integration| {
            format!(
                "- {name} ({kind:?}) {base}",
                name = integration.name,
                kind = integration.spec_kind,
                base = integration
                    .base_url
                    .clone()
                    .unwrap_or_else(|| "(无 base_url)".to_string())
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "你正在执行 Agent App 的一次运行。\n应用：{name} v{version}\n模式：{mode:?}\n\n应用意图快照：\n```json\n{intent}\n```\n\n可用集成：\n{integrations}\n\n执行原则：优先使用 `api_tools__http_request` 做直接 API 调用；必要时才回退到 developer；不要依赖浏览器/Playwright；如果应用已有已验证路径，优先沿用。\n\n请输出短结果：\n1. 执行摘要\n2. 关键结果\n3. 失败时的下一步建议",
        name = module.name,
        version = module.version,
        mode = mode,
        intent = intent_snapshot,
        integrations = integration_lines
    )
}

fn compact_integration_context(integration: &AutomationIntegrationDoc) -> String {
    let hints = integration
        .context_summary
        .clone()
        .unwrap_or_else(|| extract_spec_hints(&integration.spec_content));
    format!(
        "## 集成：{name}\n- spec_kind: {kind:?}\n- 关键线索：\n{hints}",
        name = integration.name,
        kind = integration.spec_kind,
        hints = hints,
    )
}

fn extract_spec_hints(spec: &str) -> String {
    let mut hints = Vec::new();
    for raw_line in spec.lines() {
        let line = raw_line.trim();
        if line.is_empty() {
            continue;
        }
        let lower = line.to_ascii_lowercase();
        if lower.starts_with("operationid:")
            || lower.starts_with("- url:")
            || line.starts_with('/')
            || lower.contains(" get:")
            || lower.contains(" post:")
            || lower.contains(" put:")
            || lower.contains(" patch:")
            || lower.contains(" delete:")
            || lower.starts_with("get /")
            || lower.starts_with("post /")
            || lower.starts_with("put /")
            || lower.starts_with("patch /")
            || lower.starts_with("delete /")
        {
            hints.push(format!("- {}", line));
        }
        if hints.len() >= 8 {
            break;
        }
    }
    if hints.is_empty() {
        let excerpt = spec.chars().take(800).collect::<String>();
        format!("```text\n{}\n```", excerpt)
    } else {
        hints.join("\n")
    }
}

fn format_runtime_notice(
    module_name: &str,
    mode: &RunMode,
    summary: Option<&str>,
    error: Option<&str>,
) -> String {
    let title = format!("应用「{}」刚完成一次 {:?} 运行。", module_name, mode);
    let body = if let Some(error) = error {
        format!("结果：失败\n\n错误摘要：{}", error)
    } else if let Some(summary) = summary {
        format!(
            "结果：成功\n\n{}",
            summary.chars().take(1800).collect::<String>()
        )
    } else {
        "结果：成功，但没有额外摘要。".to_string()
    };
    format!("{}\n\n{}", title, body)
}

fn foundry_builder_allowed_extensions() -> Vec<String> {
    vec!["api_tools".to_string(), "document_tools".to_string()]
}

fn foundry_runtime_allowed_extensions() -> Vec<String> {
    vec![
        "api_tools".to_string(),
        "developer".to_string(),
        "document_tools".to_string(),
    ]
}

fn automation_builder_session_source() -> &'static str {
    // Builder probing stays on a dedicated conversation-like surface that
    // suppresses delegation/swarm upgrades while keeping normal chat semantics.
    "automation_builder"
}

fn automation_runtime_session_source() -> &'static str {
    // Published app runtime sessions are side-effect free containers.
    // The first real user message is the only thing that should trigger execution.
    "automation_runtime"
}

fn automation_run_session_source() -> &'static str {
    // Background module runs remain hidden system-owned execution sessions.
    "system"
}

#[cfg(test)]
mod tests {
    use super::{
        automation_builder_session_source, automation_run_session_source,
        automation_runtime_session_source, foundry_builder_allowed_extensions,
        foundry_runtime_allowed_extensions,
    };

    #[test]
    fn automation_builder_session_stays_on_chat_surface() {
        assert_eq!(automation_builder_session_source(), "automation_builder");
    }

    #[test]
    fn automation_run_session_stays_on_system_surface() {
        assert_eq!(automation_run_session_source(), "system");
    }

    #[test]
    fn automation_runtime_session_uses_distinct_runtime_surface() {
        assert_eq!(automation_runtime_session_source(), "automation_runtime");
    }

    #[test]
    fn automation_builder_extensions_prefer_structured_api_tools() {
        let extensions = foundry_builder_allowed_extensions();
        assert!(extensions.iter().any(|item| item == "api_tools"));
        assert!(extensions.iter().any(|item| item == "document_tools"));
        assert!(!extensions.iter().any(|item| item == "developer"));
    }

    #[test]
    fn automation_runtime_extensions_keep_developer_fallback() {
        let extensions = foundry_runtime_allowed_extensions();
        assert!(extensions.iter().any(|item| item == "api_tools"));
        assert!(extensions.iter().any(|item| item == "developer"));
        assert!(extensions.iter().any(|item| item == "document_tools"));
    }
}
