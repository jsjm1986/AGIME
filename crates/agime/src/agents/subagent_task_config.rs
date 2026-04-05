use crate::agents::harness::TaskRuntime;
use crate::agents::harness::{CoordinatorRole, DelegationMode};
use crate::agents::platform_tools::PLATFORM_MANAGE_SCHEDULE_TOOL_NAME;
use crate::agents::ExtensionConfig;
use crate::providers::base::Provider;
use std::env;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Default maximum number of turns for task execution
pub const DEFAULT_SUBAGENT_MAX_TURNS: usize = 25;

/// Environment variable name for configuring max turns
pub const AGIME_SUBAGENT_MAX_TURNS_ENV_VAR: &str = "AGIME_SUBAGENT_MAX_TURNS";

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WorkerCapabilityContext {
    pub allowed_builtin_tools: Vec<String>,
    pub allowed_extension_tools: Vec<String>,
    pub runtime_tool_surface: Vec<String>,
    pub hidden_coordinator_tools: Vec<String>,
    pub permission_policy: String,
    pub allow_worker_messaging: bool,
    pub peer_worker_addresses: Vec<String>,
    pub peer_worker_catalog: Vec<String>,
    pub leader_address: Option<String>,
    pub current_worker_address: Option<String>,
}

impl WorkerCapabilityContext {
    pub fn runtime_tool_surface_text(&self) -> String {
        if self.runtime_tool_surface.is_empty() {
            "none".to_string()
        } else {
            self.runtime_tool_surface.join(", ")
        }
    }

    pub fn hidden_tools_text(&self) -> String {
        if self.hidden_coordinator_tools.is_empty() {
            "none".to_string()
        } else {
            self.hidden_coordinator_tools.join(", ")
        }
    }
}

fn extension_surface_summary(extension: &ExtensionConfig) -> String {
    match extension {
        ExtensionConfig::Frontend { name, tools, .. } => {
            let tool_names = tools
                .iter()
                .map(|tool| tool.name.to_string())
                .collect::<Vec<_>>();
            if tool_names.is_empty() {
                format!("{} (frontend runtime tools)", name)
            } else {
                format!("{} (frontend tools: {})", name, tool_names.join(", "))
            }
        }
        ExtensionConfig::Sse {
            name,
            available_tools,
            ..
        }
        | ExtensionConfig::StreamableHttp {
            name,
            available_tools,
            ..
        }
        | ExtensionConfig::Stdio {
            name,
            available_tools,
            ..
        }
        | ExtensionConfig::Builtin {
            name,
            available_tools,
            ..
        }
        | ExtensionConfig::Platform {
            name,
            available_tools,
            ..
        }
        | ExtensionConfig::InlinePython {
            name,
            available_tools,
            ..
        } => {
            if available_tools.is_empty() {
                format!("{} (all configured extension tools)", name)
            } else {
                format!("{} (tools: {})", name, available_tools.join(", "))
            }
        }
    }
}

/// Configuration for task execution with all necessary dependencies
#[derive(Clone)]
pub struct TaskConfig {
    pub provider: Arc<dyn Provider>,
    pub parent_session_id: String,
    pub parent_working_dir: PathBuf,
    pub extensions: Vec<ExtensionConfig>,
    pub max_turns: Option<usize>,
    pub current_depth: u32,
    pub max_depth: u32,
    pub force_summary_only: bool,
    pub write_scope: Vec<String>,
    pub target_artifacts: Vec<String>,
    pub result_contract: Vec<String>,
    pub delegation_mode: DelegationMode,
    pub parallelism_budget: Option<u32>,
    pub swarm_budget: Option<u32>,
    pub validation_mode: bool,
    pub task_board_session_id: Option<String>,
    pub swarm_run_id: Option<String>,
    pub worker_name: Option<String>,
    pub leader_name: Option<String>,
    pub coordinator_role: CoordinatorRole,
    pub scratchpad_dir: Option<PathBuf>,
    pub mailbox_dir: Option<PathBuf>,
    pub permission_dir: Option<PathBuf>,
    pub enable_permission_bridge: bool,
    pub allow_worker_messaging: bool,
    pub peer_worker_addresses: Vec<String>,
    pub peer_worker_catalog: Vec<String>,
    pub task_runtime: Option<Arc<TaskRuntime>>,
    pub current_task_id: Option<String>,
}

impl fmt::Debug for TaskConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TaskConfig")
            .field("provider", &"<dyn Provider>")
            .field("parent_session_id", &self.parent_session_id)
            .field("parent_working_dir", &self.parent_working_dir)
            .field("max_turns", &self.max_turns)
            .field("current_depth", &self.current_depth)
            .field("max_depth", &self.max_depth)
            .field("force_summary_only", &self.force_summary_only)
            .field("write_scope", &self.write_scope)
            .field("target_artifacts", &self.target_artifacts)
            .field("result_contract", &self.result_contract)
            .field("delegation_mode", &self.delegation_mode)
            .field("parallelism_budget", &self.parallelism_budget)
            .field("swarm_budget", &self.swarm_budget)
            .field("validation_mode", &self.validation_mode)
            .field("task_board_session_id", &self.task_board_session_id)
            .field("swarm_run_id", &self.swarm_run_id)
            .field("worker_name", &self.worker_name)
            .field("leader_name", &self.leader_name)
            .field("coordinator_role", &self.coordinator_role)
            .field("scratchpad_dir", &self.scratchpad_dir)
            .field("mailbox_dir", &self.mailbox_dir)
            .field("permission_dir", &self.permission_dir)
            .field("enable_permission_bridge", &self.enable_permission_bridge)
            .field("allow_worker_messaging", &self.allow_worker_messaging)
            .field("peer_worker_addresses", &self.peer_worker_addresses)
            .field("peer_worker_catalog", &self.peer_worker_catalog)
            .field(
                "task_runtime",
                &self.task_runtime.as_ref().map(|_| "<task runtime>"),
            )
            .field("current_task_id", &self.current_task_id)
            .field("extensions", &self.extensions)
            .finish()
    }
}

impl TaskConfig {
    pub fn new(
        provider: Arc<dyn Provider>,
        parent_session_id: &str,
        parent_working_dir: &Path,
        extensions: Vec<ExtensionConfig>,
    ) -> Self {
        Self {
            provider,
            parent_session_id: parent_session_id.to_owned(),
            parent_working_dir: parent_working_dir.to_owned(),
            extensions,
            max_turns: Some(
                env::var(AGIME_SUBAGENT_MAX_TURNS_ENV_VAR)
                    .ok()
                    .and_then(|val| val.parse::<usize>().ok())
                    .unwrap_or(DEFAULT_SUBAGENT_MAX_TURNS),
            ),
            current_depth: 0,
            max_depth: std::env::var("AGIME_HARNESS_MAX_SUBAGENT_DEPTH")
                .ok()
                .and_then(|val| val.parse::<u32>().ok())
                .filter(|value| *value > 0)
                .unwrap_or(1),
            force_summary_only: true,
            write_scope: Vec::new(),
            target_artifacts: Vec::new(),
            result_contract: Vec::new(),
            delegation_mode: DelegationMode::Subagent,
            parallelism_budget: None,
            swarm_budget: None,
            validation_mode: false,
            task_board_session_id: None,
            swarm_run_id: None,
            worker_name: None,
            leader_name: None,
            coordinator_role: CoordinatorRole::Leader,
            scratchpad_dir: None,
            mailbox_dir: None,
            permission_dir: None,
            enable_permission_bridge: false,
            allow_worker_messaging: false,
            peer_worker_addresses: Vec::new(),
            peer_worker_catalog: Vec::new(),
            task_runtime: None,
            current_task_id: None,
        }
    }

    pub fn with_delegation_depth(mut self, current_depth: u32, max_depth: u32) -> Self {
        self.current_depth = current_depth;
        self.max_depth = max_depth.max(1);
        self
    }

    pub fn with_summary_only(mut self, force_summary_only: bool) -> Self {
        self.force_summary_only = force_summary_only;
        self
    }

    pub fn with_write_scope(mut self, write_scope: Vec<String>) -> Self {
        self.write_scope = write_scope;
        self
    }

    pub fn with_runtime_contract(
        mut self,
        target_artifacts: Vec<String>,
        result_contract: Vec<String>,
    ) -> Self {
        self.target_artifacts = target_artifacts;
        self.result_contract = result_contract;
        self
    }

    pub fn with_delegation_mode(mut self, delegation_mode: DelegationMode) -> Self {
        self.delegation_mode = delegation_mode;
        self
    }

    pub fn with_worker_messaging_policy(mut self, allow_worker_messaging: bool) -> Self {
        self.allow_worker_messaging = allow_worker_messaging;
        self
    }

    pub fn with_task_board_session_id(mut self, task_board_session_id: Option<String>) -> Self {
        self.task_board_session_id = task_board_session_id;
        self
    }

    pub fn with_parallelism_budget(
        mut self,
        parallelism_budget: Option<u32>,
        swarm_budget: Option<u32>,
    ) -> Self {
        self.parallelism_budget = parallelism_budget;
        self.swarm_budget = swarm_budget;
        self
    }

    #[allow(clippy::too_many_arguments)]
    pub fn with_worker_runtime(
        mut self,
        swarm_run_id: Option<String>,
        worker_name: Option<String>,
        leader_name: Option<String>,
        coordinator_role: CoordinatorRole,
        scratchpad_dir: Option<PathBuf>,
        mailbox_dir: Option<PathBuf>,
        permission_dir: Option<PathBuf>,
        enable_permission_bridge: bool,
        allow_worker_messaging: bool,
        peer_worker_addresses: Vec<String>,
        peer_worker_catalog: Vec<String>,
        validation_mode: bool,
    ) -> Self {
        self.swarm_run_id = swarm_run_id;
        self.worker_name = worker_name;
        self.leader_name = leader_name;
        self.coordinator_role = coordinator_role;
        self.scratchpad_dir = scratchpad_dir;
        self.mailbox_dir = mailbox_dir;
        self.permission_dir = permission_dir;
        self.enable_permission_bridge = enable_permission_bridge;
        self.allow_worker_messaging = allow_worker_messaging;
        self.peer_worker_addresses = peer_worker_addresses;
        self.peer_worker_catalog = peer_worker_catalog;
        self.validation_mode = validation_mode;
        self
    }

    pub fn with_task_runtime(
        mut self,
        task_runtime: Option<Arc<TaskRuntime>>,
        current_task_id: Option<String>,
    ) -> Self {
        self.task_runtime = task_runtime;
        self.current_task_id = current_task_id;
        self
    }

    pub fn worker_capability_context(&self) -> Option<WorkerCapabilityContext> {
        if self.coordinator_role != CoordinatorRole::Worker {
            return None;
        }

        let mut runtime_tool_surface = self
            .extensions
            .iter()
            .map(extension_surface_summary)
            .collect::<Vec<_>>();
        if runtime_tool_surface.is_empty() {
            runtime_tool_surface.push("no extra extension tools configured".to_string());
        }

        let permission_policy = if self.validation_mode && self.enable_permission_bridge {
            "Read-only validation worker. Runtime approval resolution may unblock gated inspection tools, but writes and delegation remain out of scope."
                .to_string()
        } else if self.validation_mode {
            "Read-only validation worker. Do not request file writes, delegation, or schedule management."
                .to_string()
        } else if self.enable_permission_bridge {
            "Use only the declared worker tools and bounded write scope. Runtime approval resolution will settle gated tool calls inside that scope through the leader path when available, or headless fallback when no leader resolver exists."
                .to_string()
        } else {
            "Use only the declared worker tools and bounded write scope. Coordinator-only tools are hidden at runtime."
                .to_string()
        };

        let mut allowed_builtin_tools = vec!["recipe__final_output".to_string()];
        if self.allow_worker_messaging {
            allowed_builtin_tools.push("send_message".to_string());
        }

        Some(WorkerCapabilityContext {
            allowed_builtin_tools,
            allowed_extension_tools: self
                .extensions
                .iter()
                .flat_map(|extension| match extension {
                    ExtensionConfig::Frontend { tools, .. } => tools
                        .iter()
                        .map(|tool| tool.name.to_string())
                        .collect::<Vec<_>>(),
                    ExtensionConfig::Sse {
                        available_tools, ..
                    }
                    | ExtensionConfig::StreamableHttp {
                        available_tools, ..
                    }
                    | ExtensionConfig::Stdio {
                        available_tools, ..
                    }
                    | ExtensionConfig::Builtin {
                        available_tools, ..
                    }
                    | ExtensionConfig::Platform {
                        available_tools, ..
                    }
                    | ExtensionConfig::InlinePython {
                        available_tools, ..
                    } => available_tools.clone(),
                })
                .collect(),
            runtime_tool_surface,
            hidden_coordinator_tools: vec![
                "subagent".to_string(),
                "swarm".to_string(),
                PLATFORM_MANAGE_SCHEDULE_TOOL_NAME.to_string(),
            ],
            permission_policy,
            allow_worker_messaging: self.allow_worker_messaging,
            peer_worker_addresses: self.peer_worker_addresses.clone(),
            peer_worker_catalog: self.peer_worker_catalog.clone(),
            leader_address: self.leader_name.clone(),
            current_worker_address: self.worker_name.clone(),
        })
    }
}
