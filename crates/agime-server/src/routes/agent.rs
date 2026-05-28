use crate::routes::errors::ErrorResponse;
use crate::routes::recipe_utils::{
    apply_recipe_to_agent, build_recipe_with_parameter_values, load_recipe_by_id, validate_recipe,
};
use crate::state::AppState;
use agime::config::PermissionManager;
use axum::response::IntoResponse;
use axum::{
    extract::{Query, State},
    http::StatusCode,
    routing::{delete, get, post},
    Json, Router,
};

use agime::agents::ExtensionConfig;
use agime::config::{AgimeMode, Config};
use agime::model::ModelConfig;
use agime::prompt_template::render_global_file;
use agime::providers::create;
use agime::recipe::Recipe;
use agime::recipe_deeplink;
use agime::session::session_manager::SessionType;
use agime::session::{Session, SessionManager};
use agime::{
    agents::{extension::ToolInfo, extension_manager::get_parameter_names},
    config::permission::PermissionLevel,
};
use rmcp::model::{CallToolRequestParams, Content};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;
use tracing::{error, warn};

#[derive(Deserialize, utoipa::ToSchema)]
pub struct UpdateFromSessionRequest {
    session_id: String,
}

#[derive(Deserialize, utoipa::ToSchema)]
pub struct UpdateProviderRequest {
    provider: String,
    model: Option<String>,
    session_id: String,
}

/// Per-session override payload accepted by `/agent/session_override`.
///
/// All fields are optional. Sending `null` (or omitting a field) leaves that
/// slot of the stored override unchanged when the override already exists; on
/// a new override the field defaults to `None`. Use `/agent/session_override`
/// (DELETE) to drop the override entirely.
#[derive(Deserialize, utoipa::ToSchema)]
pub struct SessionOverrideRequest {
    session_id: String,
    /// Provider name as registered with `agime::providers::create` (e.g.
    /// "openai", "anthropic"). When set together with `api_key` / `api_url`,
    /// the desktop will use [`crate::host_provider::create_provider_for_config`]
    /// to build a per-session provider; otherwise the registry path keeps
    /// reading env vars.
    provider: Option<String>,
    model: Option<String>,
    api_url: Option<String>,
    api_key: Option<String>,
    /// Full provider config value object. When set, it takes precedence over
    /// the flat `provider`/`model`/`api_url`/`api_key` fields and lets callers
    /// drive the four extra slots `temperature` / `max_tokens` /
    /// `context_limit` / `prompt_caching_mode` / etc. that the flat shape does
    /// not expose.
    provider_config: Option<crate::host_provider::HostProviderConfig>,
    prompt_profile_overlay: Option<String>,
    extra_instructions: Option<String>,
    turn_system_instruction: Option<String>,
    /// Pre-rendered runtime capability snapshot overlay (V2 prompt pack).
    runtime_overlay_text: Option<String>,
    /// Pre-rendered harness delegation overlay (V2 prompt pack).
    harness_delegation_overlay_text: Option<String>,
    /// Pre-rendered surface contract overlay (V2 prompt pack).
    surface_contract_overlay_text: Option<String>,
    /// Typed runtime capability snapshot. Reply path renders this via
    /// [`crate::host_prompt::build_runtime_capability_snapshot_overlay`] when
    /// `runtime_overlay_text` is unset; pre-rendered text wins when both are
    /// supplied.
    capability_snapshot: Option<crate::host_prompt::PromptCapabilitySnapshot>,
    /// Typed harness delegation overlay. Rendered via
    /// [`crate::host_prompt::build_harness_delegation_overlay_text`] when
    /// `harness_delegation_overlay_text` is unset.
    harness_overlay: Option<crate::host_prompt::PromptHarnessOverlay>,
    /// Typed surface contract. Rendered via
    /// [`crate::host_prompt::build_surface_contract_overlay_text`] when
    /// `surface_contract_overlay_text` is unset.
    surface_contract: Option<crate::host_prompt::PromptSurfaceContract>,
    /// Workspace execution boundary (workspace_root / run_dir / write-allowed
    /// roots / artifact dirs / kind). When present, the chat reply path
    /// renders the boundary instruction into the system prompt overlay and
    /// (after wiring the dispatch enforcement) refuses writes outside the
    /// allowed roots.
    workspace_context: Option<crate::host_workspace::WorkspaceExecutionContext>,
}

#[derive(Deserialize, utoipa::ToSchema)]
pub struct ClearSessionOverrideRequest {
    session_id: String,
}

/// Request body for `POST /agent/capability_context`. Mirrors the team-server
/// resolver inputs but uses the desktop-flavored
/// [`crate::host_capability::ApprovalMode`] (Auto/Approve/Manual). Only
/// available when the `desktop_harness_host` cargo feature is enabled.
#[cfg(feature = "desktop_harness_host")]
#[derive(Deserialize, utoipa::ToSchema)]
pub struct CapabilityContextRequest {
    session_id: String,
    #[serde(default)]
    session_source: Option<String>,
    /// If `None`, no extension allow-list is enforced (fully permissive).
    #[serde(default)]
    allowed_extensions: Option<Vec<String>>,
    /// Tool names that surface for approval when `approval_mode == manual`.
    #[serde(default)]
    manual_approval_tools: Option<Vec<String>>,
    /// "auto" | "approve" | "manual". Defaults to `auto`.
    #[serde(default)]
    approval_mode: Option<String>,
}

#[cfg(feature = "desktop_harness_host")]
#[derive(Deserialize, utoipa::ToSchema)]
pub struct ClearCapabilityContextRequest {
    session_id: String,
}

#[derive(Deserialize, utoipa::ToSchema)]
pub struct GetToolsQuery {
    extension_name: Option<String>,
    session_id: String,
}

#[derive(Deserialize, utoipa::ToSchema)]
pub struct UpdateRouterToolSelectorRequest {
    session_id: String,
}

#[derive(Deserialize, utoipa::ToSchema)]
pub struct StartAgentRequest {
    working_dir: String,
    #[serde(default)]
    recipe: Option<Recipe>,
    #[serde(default)]
    recipe_id: Option<String>,
    #[serde(default)]
    recipe_deeplink: Option<String>,
}

#[derive(Deserialize, utoipa::ToSchema)]
pub struct StopAgentRequest {
    session_id: String,
}

#[derive(Deserialize, utoipa::ToSchema)]
pub struct ResumeAgentRequest {
    session_id: String,
    load_model_and_extensions: bool,
}

#[derive(Deserialize, utoipa::ToSchema)]
pub struct AddExtensionRequest {
    session_id: String,
    config: ExtensionConfig,
}

#[derive(Deserialize, utoipa::ToSchema)]
pub struct RemoveExtensionRequest {
    name: String,
    session_id: String,
}

#[derive(Deserialize, utoipa::ToSchema)]
pub struct ReadResourceRequest {
    session_id: String,
    extension_name: String,
    uri: String,
}

#[derive(Serialize, Deserialize, utoipa::ToSchema)]
pub struct ReadResourceResponse {
    html: String,
}

#[derive(Deserialize, utoipa::ToSchema)]
pub struct CallToolRequest {
    session_id: String,
    name: String,
    arguments: Value,
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct CallToolResponse {
    content: Vec<Content>,
    structured_content: Option<Value>,
    is_error: bool,
}

#[utoipa::path(
    post,
    path = "/agent/start",
    request_body = StartAgentRequest,
    responses(
        (status = 200, description = "Agent started successfully", body = Session),
        (status = 400, description = "Bad request", body = ErrorResponse),
        (status = 401, description = "Unauthorized - invalid secret key"),
        (status = 500, description = "Internal server error", body = ErrorResponse)
    )
)]
async fn start_agent(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<StartAgentRequest>,
) -> Result<Json<Session>, ErrorResponse> {
    agime::posthog::set_session_context("desktop", false);

    let StartAgentRequest {
        working_dir,
        recipe,
        recipe_id,
        recipe_deeplink,
    } = payload;

    let original_recipe = if let Some(deeplink) = recipe_deeplink {
        match recipe_deeplink::decode(&deeplink) {
            Ok(recipe) => Some(recipe),
            Err(err) => {
                error!("Failed to decode recipe deeplink: {}", err);
                return Err(ErrorResponse {
                    message: err.to_string(),
                    status: StatusCode::BAD_REQUEST,
                });
            }
        }
    } else if let Some(id) = recipe_id {
        match load_recipe_by_id(state.as_ref(), &id).await {
            Ok(recipe) => Some(recipe),
            Err(err) => return Err(err),
        }
    } else {
        recipe
    };

    if let Some(ref recipe) = original_recipe {
        if let Err(err) = validate_recipe(recipe) {
            return Err(ErrorResponse {
                message: err.message,
                status: err.status,
            });
        }
    }

    let counter = state.session_counter.fetch_add(1, Ordering::SeqCst) + 1;
    let name = format!("New session {}", counter);

    let mut session =
        SessionManager::create_session(PathBuf::from(&working_dir), name, SessionType::User)
            .await
            .map_err(|err| {
                error!("Failed to create session: {}", err);
                ErrorResponse {
                    message: format!("Failed to create session: {}", err),
                    status: StatusCode::BAD_REQUEST,
                }
            })?;

    if let Some(recipe) = original_recipe {
        SessionManager::update_session(&session.id)
            .recipe(Some(recipe))
            .apply()
            .await
            .map_err(|err| {
                error!("Failed to update session with recipe: {}", err);
                ErrorResponse {
                    message: format!("Failed to update session with recipe: {}", err),
                    status: StatusCode::INTERNAL_SERVER_ERROR,
                }
            })?;

        session = SessionManager::get_session(&session.id, false)
            .await
            .map_err(|err| {
                error!("Failed to get updated session: {}", err);
                ErrorResponse {
                    message: format!("Failed to get updated session: {}", err),
                    status: StatusCode::INTERNAL_SERVER_ERROR,
                }
            })?;
    }

    Ok(Json(session))
}

#[utoipa::path(
    post,
    path = "/agent/resume",
    request_body = ResumeAgentRequest,
    responses(
        (status = 200, description = "Agent started successfully", body = Session),
        (status = 400, description = "Bad request - invalid working directory"),
        (status = 401, description = "Unauthorized - invalid secret key"),
        (status = 500, description = "Internal server error")
    )
)]
async fn resume_agent(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<ResumeAgentRequest>,
) -> Result<Json<Session>, ErrorResponse> {
    agime::posthog::set_session_context("desktop", true);

    let session = SessionManager::get_session(&payload.session_id, true)
        .await
        .map_err(|err| {
            error!("Failed to resume session {}: {}", payload.session_id, err);
            ErrorResponse {
                message: format!("Failed to resume session: {}", err),
                status: StatusCode::NOT_FOUND,
            }
        })?;

    if payload.load_model_and_extensions {
        let agent = state
            .get_agent_for_route(payload.session_id.clone())
            .await
            .map_err(|code| ErrorResponse {
                message: "Failed to get agent for route".into(),
                status: code,
            })?;

        let config = Config::global();

        let provider_result = async {
            let provider_name = session
                .provider_name
                .clone()
                .or_else(|| config.get_agime_provider().ok())
                .ok_or_else(|| ErrorResponse {
                    message: "Could not configure agent: missing provider".into(),
                    status: StatusCode::INTERNAL_SERVER_ERROR,
                })?;

            let model_config = match session.model_config.clone() {
                Some(saved_config) => saved_config,
                None => {
                    let model_name = config.get_agime_model().map_err(|_| ErrorResponse {
                        message: "Could not configure agent: missing model".into(),
                        status: StatusCode::INTERNAL_SERVER_ERROR,
                    })?;
                    ModelConfig::new(&model_name).map_err(|e| ErrorResponse {
                        message: format!("Could not configure agent: invalid model {}", e),
                        status: StatusCode::INTERNAL_SERVER_ERROR,
                    })?
                }
            };

            let provider =
                create(&provider_name, model_config)
                    .await
                    .map_err(|e| ErrorResponse {
                        message: format!("Could not create provider: {}", e),
                        status: StatusCode::INTERNAL_SERVER_ERROR,
                    })?;

            agent
                .update_provider(provider, &payload.session_id)
                .await
                .map_err(|e| ErrorResponse {
                    message: format!("Could not configure agent: {}", e),
                    status: StatusCode::INTERNAL_SERVER_ERROR,
                })
        };

        let extensions_result = async {
            let enabled_configs = agime::config::get_enabled_extensions();
            let agent_clone = agent.clone();

            let extension_futures = enabled_configs
                .into_iter()
                .map(|config| {
                    let config_clone = config.clone();
                    let agent_ref = agent_clone.clone();

                    async move {
                        if let Err(e) = agent_ref.add_extension(config_clone.clone()).await {
                            warn!("Failed to load extension {}: {}", config_clone.name(), e);
                        }
                        Ok::<_, ErrorResponse>(())
                    }
                })
                .collect::<Vec<_>>();

            futures::future::join_all(extension_futures).await;
            Ok::<(), ErrorResponse>(()) // Fixed type annotation
        };

        let (provider_result, _) = tokio::join!(provider_result, extensions_result);
        provider_result?;
    }

    Ok(Json(session))
}

#[utoipa::path(
    post,
    path = "/agent/update_from_session",
    request_body = UpdateFromSessionRequest,
    responses(
        (status = 200, description = "Update agent from session data successfully"),
        (status = 401, description = "Unauthorized - invalid secret key"),
        (status = 424, description = "Agent not initialized"),
    ),
)]
async fn update_from_session(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<UpdateFromSessionRequest>,
) -> Result<StatusCode, ErrorResponse> {
    let agent = state
        .get_agent_for_route(payload.session_id.clone())
        .await
        .map_err(|status| ErrorResponse {
            message: format!("Failed to get agent: {}", status),
            status,
        })?;
    let session = SessionManager::get_session(&payload.session_id, false)
        .await
        .map_err(|err| ErrorResponse {
            message: format!("Failed to get session: {}", err),
            status: StatusCode::INTERNAL_SERVER_ERROR,
        })?;

    // Resolve the active prompt: if AGIME_ACTIVE_SYSTEM_PROMPT is set, the
    // legacy contract is that it *replaces* system.md entirely. Preserve that
    // behavior by overriding the agent's system prompt directly and skipping
    // the composer (which treats custom_prompt as an `<agent_instructions>`
    // overlay rather than a replacement).
    let config = Config::global();
    let active_prompt = config
        .get_param::<String>("AGIME_ACTIVE_SYSTEM_PROMPT")
        .ok()
        .filter(|value| !value.is_empty());
    if let Some(prompt) = active_prompt.as_ref() {
        agent.override_system_prompt(prompt.clone()).await;
    }

    // Compose the desktop_prompt overlay (and recipe overlay, if any) into a
    // single `extra_instructions` blob. Even when the legacy active_prompt
    // path is taken above, the desktop_prompt / recipe overlays still flow
    // through `extend_system_prompt` exactly as before.
    let context: HashMap<&str, Value> = HashMap::new();
    let desktop_prompt =
        render_global_file("desktop_prompt.md", &context).expect("Prompt should render");
    let mut composer_extra = desktop_prompt;
    if let Some(recipe) = session.recipe {
        match build_recipe_with_parameter_values(
            &recipe,
            session.user_recipe_values.unwrap_or_default(),
        )
        .await
        {
            Ok(Some(recipe)) => {
                if let Some(prompt) = apply_recipe_to_agent(&agent, &recipe, true).await {
                    composer_extra = prompt;
                }
            }
            Ok(None) => {
                // Recipe has missing parameters - keep desktop_prompt as the extra body.
            }
            Err(e) => {
                return Err(ErrorResponse {
                    message: e.to_string(),
                    status: StatusCode::INTERNAL_SERVER_ERROR,
                });
            }
        }
    }

    // Push the desktop_prompt + recipe overlay through the
    // `system_prompt_extras` channel. The session_override overlays are
    // applied separately on every chat turn (see `routes::reply`) via the
    // `host_override_extras` channel, so here we deliberately do *not*
    // touch any session_override fields. Those two channels render in the
    // same final prompt block but are cleared on different cadences (this
    // path runs once per `update_from_session`; the reply path re-applies
    // every turn).
    agent.extend_system_prompt(composer_extra).await;

    Ok(StatusCode::OK)
}

#[utoipa::path(
    get,
    path = "/agent/tools",
    params(
        ("extension_name" = Option<String>, Query, description = "Optional extension name to filter tools"),
        ("session_id" = String, Query, description = "Required session ID to scope tools to a specific session")
    ),
    responses(
        (status = 200, description = "Tools retrieved successfully", body = Vec<ToolInfo>),
        (status = 401, description = "Unauthorized - invalid secret key"),
        (status = 424, description = "Agent not initialized"),
        (status = 500, description = "Internal server error")
    )
)]
async fn get_tools(
    State(state): State<Arc<AppState>>,
    Query(query): Query<GetToolsQuery>,
) -> Result<Json<Vec<ToolInfo>>, StatusCode> {
    let config = Config::global();
    let agime_mode = config.get_agime_mode().unwrap_or(AgimeMode::Auto);
    let agent = state.get_agent_for_route(query.session_id).await?;
    let permission_manager = PermissionManager::default();

    let mut tools: Vec<ToolInfo> = agent
        .list_tools(query.extension_name)
        .await
        .into_iter()
        .map(|tool| {
            let permission = permission_manager
                .get_user_permission(&tool.name)
                .or_else(|| {
                    if agime_mode == AgimeMode::SmartApprove {
                        permission_manager.get_smart_approve_permission(&tool.name)
                    } else if agime_mode == AgimeMode::Approve {
                        Some(PermissionLevel::AskBefore)
                    } else {
                        None
                    }
                });

            ToolInfo::new(
                &tool.name,
                tool.description
                    .as_ref()
                    .map(|d| d.as_ref())
                    .unwrap_or_default(),
                get_parameter_names(&tool),
                permission,
            )
        })
        .collect::<Vec<ToolInfo>>();
    tools.sort_by(|a, b| a.name.cmp(&b.name));

    Ok(Json(tools))
}

#[utoipa::path(
    post,
    path = "/agent/update_provider",
    request_body = UpdateProviderRequest,
    responses(
        (status = 200, description = "Provider updated successfully"),
        (status = 400, description = "Bad request - missing or invalid parameters"),
        (status = 401, description = "Unauthorized - invalid secret key"),
        (status = 424, description = "Agent not initialized"),
        (status = 500, description = "Internal server error")
    )
)]
async fn update_agent_provider(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<UpdateProviderRequest>,
) -> Result<(), impl IntoResponse> {
    let agent = state
        .get_agent_for_route(payload.session_id.clone())
        .await
        .map_err(|e| (e, "No agent for session id".to_owned()))?;

    let config = Config::global();
    let model = match payload.model.or_else(|| config.get_agime_model().ok()) {
        Some(m) => m,
        None => {
            return Err((StatusCode::BAD_REQUEST, "No model specified".to_owned()));
        }
    };

    let model_config = ModelConfig::new(&model).map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            format!("Invalid model config: {}", e),
        )
    })?;

    // Per-session override path: when the caller has registered a
    // [`crate::host_provider::HostProviderConfig`] via
    // `/agent/session_override`, build the provider directly from that value
    // object. Otherwise fall through to the registry path, which reads
    // per-provider api_url / api_key from env via each provider's
    // `from_env(model)` constructor (preserves existing behavior for
    // Anthropic / OpenAI users that rely on env keys).
    let session_override = state.session_override(&payload.session_id).await;
    let new_provider = if let Some(cfg) = session_override.as_ref().and_then(|o| {
        o.provider_config
            .as_ref()
            .filter(|c| c.name.eq_ignore_ascii_case(&payload.provider))
    }) {
        let mut cfg = cfg.clone();
        cfg.model.get_or_insert_with(|| model.clone());
        crate::host_provider::create_provider_for_config(&cfg).map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                format!(
                    "Failed to create {} provider from override: {}",
                    &payload.provider, e
                ),
            )
        })?
    } else {
        create(&payload.provider, model_config).await.map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                format!("Failed to create {} provider: {}", &payload.provider, e),
            )
        })?
    };

    agent
        .update_provider(new_provider, &payload.session_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to update provider: {}", e),
            )
        })?;

    // Keep the per-session "already applied" fingerprint in sync so the
    // chat reply path doesn't redundantly call `update_provider` again on
    // the next turn (or, conversely, doesn't miss a provider change made
    // here while a fingerprint from an earlier override is still cached).
    if let Some(cfg) = session_override
        .as_ref()
        .and_then(|o| o.provider_config.as_ref())
        .filter(|c| c.name.eq_ignore_ascii_case(&payload.provider))
    {
        state
            .record_applied_provider(&payload.session_id, cfg)
            .await;
    } else {
        state
            .clear_applied_provider_fingerprint(&payload.session_id)
            .await;
    }

    Ok(())
}

#[utoipa::path(
    post,
    path = "/agent/update_router_tool_selector",
    request_body = UpdateRouterToolSelectorRequest,
    responses(
        (status = 200, description = "Tool selection strategy updated successfully", body = String),
        (status = 401, description = "Unauthorized - invalid secret key"),
        (status = 424, description = "Agent not initialized"),
        (status = 500, description = "Internal server error")
    )
)]
async fn update_router_tool_selector(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<UpdateRouterToolSelectorRequest>,
) -> Result<Json<String>, StatusCode> {
    let agent = state.get_agent_for_route(payload.session_id).await?;
    agent
        .update_router_tool_selector(None, Some(true))
        .await
        .map_err(|e| {
            error!("Failed to update tool selection strategy: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    Ok(Json(
        "Tool selection strategy updated successfully".to_string(),
    ))
}

#[utoipa::path(
    post,
    path = "/agent/add_extension",
    request_body = AddExtensionRequest,
    responses(
        (status = 200, description = "Extension added", body = String),
        (status = 401, description = "Unauthorized - invalid secret key"),
        (status = 424, description = "Agent not initialized"),
        (status = 500, description = "Internal server error")
    )
)]
async fn agent_add_extension(
    State(state): State<Arc<AppState>>,
    Json(request): Json<AddExtensionRequest>,
) -> Result<StatusCode, ErrorResponse> {
    let agent = state.get_agent(request.session_id).await?;
    agent
        .add_extension(request.config)
        .await
        .map_err(|e| ErrorResponse::internal(format!("Failed to add extension: {}", e)))?;
    Ok(StatusCode::OK)
}

#[utoipa::path(
    post,
    path = "/agent/remove_extension",
    request_body = RemoveExtensionRequest,
    responses(
        (status = 200, description = "Extension removed", body = String),
        (status = 401, description = "Unauthorized - invalid secret key"),
        (status = 424, description = "Agent not initialized"),
        (status = 500, description = "Internal server error")
    )
)]
async fn agent_remove_extension(
    State(state): State<Arc<AppState>>,
    Json(request): Json<RemoveExtensionRequest>,
) -> Result<StatusCode, ErrorResponse> {
    let agent = state.get_agent(request.session_id).await?;
    agent.remove_extension(&request.name).await?;
    Ok(StatusCode::OK)
}

#[utoipa::path(
    post,
    path = "/agent/stop",
    request_body = StopAgentRequest,
    responses(
        (status = 200, description = "Agent stopped successfully", body = String),
        (status = 401, description = "Unauthorized - invalid secret key"),
        (status = 404, description = "Session not found"),
        (status = 500, description = "Internal server error")
    )
)]
async fn stop_agent(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<StopAgentRequest>,
) -> Result<StatusCode, ErrorResponse> {
    let session_id = payload.session_id;
    state
        .agent_manager
        .remove_session(&session_id)
        .await
        .map_err(|e| ErrorResponse {
            message: format!("Failed to stop agent for session {}: {}", session_id, e),
            status: StatusCode::NOT_FOUND,
        })?;

    Ok(StatusCode::OK)
}

#[utoipa::path(
    post,
    path = "/agent/read_resource",
    request_body = ReadResourceRequest,
    responses(
        (status = 200, description = "Resource read successfully", body = ReadResourceResponse),
        (status = 401, description = "Unauthorized - invalid secret key"),
        (status = 424, description = "Agent not initialized"),
        (status = 404, description = "Resource not found"),
        (status = 500, description = "Internal server error")
    )
)]
async fn read_resource(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<ReadResourceRequest>,
) -> Result<Json<ReadResourceResponse>, StatusCode> {
    let agent = state
        .get_agent_for_route(payload.session_id.clone())
        .await?;

    let html = agent
        .extension_manager
        .read_ui_resource(
            &payload.uri,
            &payload.extension_name,
            CancellationToken::default(),
        )
        .await
        .map_err(|_e| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(ReadResourceResponse { html }))
}

#[utoipa::path(
    post,
    path = "/agent/call_tool",
    request_body = CallToolRequest,
    responses(
        (status = 200, description = "Resource read successfully", body = CallToolResponse),
        (status = 401, description = "Unauthorized - invalid secret key"),
        (status = 424, description = "Agent not initialized"),
        (status = 404, description = "Resource not found"),
        (status = 500, description = "Internal server error")
    )
)]
async fn call_tool(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<CallToolRequest>,
) -> Result<Json<CallToolResponse>, StatusCode> {
    let agent = state
        .get_agent_for_route(payload.session_id.clone())
        .await?;

    let arguments = match payload.arguments {
        Value::Object(map) => Some(map),
        _ => None,
    };

    let tool_call = CallToolRequestParams {
        name: payload.name.into(),
        arguments,
        meta: None,
        task: None,
    };

    let tool_result = agent
        .extension_manager
        .dispatch_tool_call(tool_call, CancellationToken::default())
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let result = tool_result
        .result
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(CallToolResponse {
        content: result.content,
        structured_content: result.structured_content,
        is_error: result.is_error.unwrap_or(false),
    }))
}

#[utoipa::path(
    post,
    path = "/agent/session_override",
    request_body = SessionOverrideRequest,
    responses(
        (status = 200, description = "Session override stored"),
        (status = 401, description = "Unauthorized - invalid secret key"),
    ),
)]
async fn set_session_override(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<SessionOverrideRequest>,
) -> StatusCode {
    use crate::host_provider::{host_api_format_from_provider_name, HostProviderConfig};
    use crate::state::SessionOverride;

    // Provider config: prefer the explicit value object, fall back to the
    // flat fields. The flat path mirrors the team-server admin's "type a
    // provider name + api key" wizard; the value-object path is for callers
    // (or future UI surfaces) that want the full ModelConfig knobs.
    let provider_config = payload.provider_config.clone().or_else(|| {
        payload.provider.as_deref().and_then(|name| {
            host_api_format_from_provider_name(name).map(|api_format| HostProviderConfig {
                name: name.to_string(),
                api_format,
                api_url: payload.api_url.clone(),
                api_key: payload.api_key.clone(),
                model: payload.model.clone(),
                ..Default::default()
            })
        })
    });

    let override_value = SessionOverride {
        provider_config,
        prompt_profile_overlay: payload.prompt_profile_overlay,
        extra_instructions: payload.extra_instructions,
        turn_system_instruction: payload.turn_system_instruction,
        runtime_overlay_text: payload.runtime_overlay_text,
        harness_delegation_overlay_text: payload.harness_delegation_overlay_text,
        surface_contract_overlay_text: payload.surface_contract_overlay_text,
        capability_snapshot: payload.capability_snapshot,
        harness_overlay: payload.harness_overlay,
        surface_contract: payload.surface_contract,
        workspace_context: payload.workspace_context,
    };

    state
        .set_session_override(payload.session_id, override_value)
        .await;

    StatusCode::OK
}

#[utoipa::path(
    post,
    path = "/agent/session_override/clear",
    request_body = ClearSessionOverrideRequest,
    responses(
        (status = 200, description = "Session override cleared (or did not exist)"),
        (status = 401, description = "Unauthorized - invalid secret key"),
    ),
)]
async fn clear_session_override(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<ClearSessionOverrideRequest>,
) -> StatusCode {
    state.clear_session_override(&payload.session_id).await;
    StatusCode::OK
}

/// REST-style alias: `DELETE /agent/session_override?session_id=...`. Same
/// behavior as the legacy `POST /agent/session_override/clear`. Prefer the
/// DELETE form for new clients; the POST form is kept as a back-compat alias.
#[utoipa::path(
    delete,
    path = "/agent/session_override",
    params(
        ("session_id" = String, Query, description = "Session whose override should be cleared"),
    ),
    responses(
        (status = 200, description = "Session override cleared (or did not exist)"),
        (status = 401, description = "Unauthorized - invalid secret key"),
    ),
)]
async fn delete_session_override(
    State(state): State<Arc<AppState>>,
    Query(payload): Query<ClearSessionOverrideRequest>,
) -> StatusCode {
    state.clear_session_override(&payload.session_id).await;
    StatusCode::OK
}

/// Install or replace the capability policy snapshot for a session. Mirror
/// of team-server's `set_delegation_capability_context` entrypoint, scoped
/// to the desktop's permissive default model.
#[cfg(feature = "desktop_harness_host")]
#[utoipa::path(
    post,
    path = "/agent/capability_context",
    request_body = CapabilityContextRequest,
    responses(
        (status = 200, description = "Capability context installed"),
        (status = 401, description = "Unauthorized - invalid secret key"),
    ),
)]
async fn set_capability_context(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<CapabilityContextRequest>,
) -> StatusCode {
    use crate::host_capability::{ApprovalMode, HostSessionPolicyContext};

    let approval_mode = payload
        .approval_mode
        .as_deref()
        .and_then(ApprovalMode::from_str)
        .unwrap_or_default();

    let ctx = HostSessionPolicyContext {
        session_source: payload.session_source.unwrap_or_default(),
        allowed_extensions: payload.allowed_extensions,
        manual_approval_tools: payload.manual_approval_tools,
        approval_mode,
    };
    state.set_capability_context(payload.session_id, ctx).await;
    StatusCode::OK
}

/// Drop the capability policy for a session. Equivalent to "fully permissive".
#[cfg(feature = "desktop_harness_host")]
#[utoipa::path(
    delete,
    path = "/agent/capability_context",
    params(("session_id" = String, Query, description = "Session whose capability context to clear")),
    responses(
        (status = 200, description = "Capability context cleared (or did not exist)"),
        (status = 401, description = "Unauthorized - invalid secret key"),
    ),
)]
async fn delete_capability_context(
    State(state): State<Arc<AppState>>,
    Query(payload): Query<ClearCapabilityContextRequest>,
) -> StatusCode {
    state.clear_capability_context(&payload.session_id).await;
    StatusCode::OK
}

pub fn routes(state: Arc<AppState>) -> Router {
    let router = Router::new()
        .route("/agent/start", post(start_agent))
        .route("/agent/resume", post(resume_agent))
        .route("/agent/tools", get(get_tools))
        .route("/agent/read_resource", post(read_resource))
        .route("/agent/call_tool", post(call_tool))
        .route("/agent/update_provider", post(update_agent_provider))
        .route(
            "/agent/update_router_tool_selector",
            post(update_router_tool_selector),
        )
        .route("/agent/update_from_session", post(update_from_session))
        .route("/agent/add_extension", post(agent_add_extension))
        .route("/agent/remove_extension", post(agent_remove_extension))
        .route("/agent/session_override", post(set_session_override))
        .route("/agent/session_override", delete(delete_session_override))
        .route(
            "/agent/session_override/clear",
            post(clear_session_override),
        )
        .route("/agent/stop", post(stop_agent));

    #[cfg(feature = "desktop_harness_host")]
    let router = router
        .route("/agent/capability_context", post(set_capability_context))
        .route(
            "/agent/capability_context",
            delete(delete_capability_context),
        );

    router.with_state(state)
}
