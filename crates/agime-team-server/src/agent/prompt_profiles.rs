//! Prompt profiles for portal manager/coding sessions.
//!
//! Profiles are additive overlays and must not replace agent base prompts.

pub const PORTAL_CODING_PROFILE_ID: &str = "portal_coding_v1";
pub const PORTAL_MANAGER_PROFILE_ID: &str = "portal_manager_v1";
pub const CHANNEL_CODING_PROFILE_ID: &str = "channel_coding_v1";
pub const CHAT_DELEGATION_PROFILE_ID: &str = "chat_delegation_v1";

pub struct PortalCodingProfileInput<'a> {
    pub portal_slug: &'a str,
    pub project_path: &'a str,
    pub portal_policy_overlay: Option<&'a str>,
    pub project_context: Option<&'a str>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelCodingIntent {
    DirectBuild,
    Planning,
}

pub struct ChannelCodingProfileInput<'a> {
    pub channel_name: &'a str,
    pub workspace_display_name: Option<&'a str>,
    pub workspace_path: Option<&'a str>,
    pub repo_path: Option<&'a str>,
    pub main_checkout_path: Option<&'a str>,
    pub thread_summary: Option<&'a str>,
    pub current_request: &'a str,
    pub intent: ChannelCodingIntent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChatDelegationIntent {
    Subagent,
    Swarm,
}

pub struct ChatDelegationProfileInput<'a> {
    pub current_request: &'a str,
    pub intent: ChatDelegationIntent,
}

fn push_section(buf: &mut String, title: &str, body: &str) {
    if body.trim().is_empty() {
        return;
    }
    buf.push_str(title);
    buf.push('\n');
    buf.push_str(body);
    buf.push('\n');
}

pub fn build_portal_coding_overlay(input: PortalCodingProfileInput<'_>) -> String {
    let mut overlay = String::new();

    overlay.push_str("【Prompt Profile Overlay】\n");
    overlay.push_str("id: ");
    overlay.push_str(PORTAL_CODING_PROFILE_ID);
    overlay.push_str(" | mode: additive\n");
    overlay.push_str("说明：以下策略仅为附加层，不覆盖原有系统提示词；若冲突，优先遵循安全与权限边界并明确报告。\n\n");

    if let Some(policy) = input.portal_policy_overlay {
        let policy = policy.trim();
        if !policy.is_empty() {
            push_section(&mut overlay, "【Portal 策略 Overlay】", policy);
            overlay.push('\n');
        }
    }

    push_section(
        &mut overlay,
        "【Role Profile】",
        &format!(
            "Portal 编程会话 | slug: {} | 目录: {}\n职责：先完成能力确认，再进行交互/页面/流程交付。",
            input.portal_slug, input.project_path
        ),
    );
    overlay.push('\n');

    push_section(
        &mut overlay,
        "【Execution Protocol】",
        "1. 仅在项目目录下操作，text_editor 使用相对路径。\n\
2. 必须调用 developer 工具真实改文件，不可只输出方案。\n\
3. 开发前必须先调用 portal_tools__get_portal_service_capability_profile 读取服务Agent能力边界。\n\
4. 若能力不足，先调用 portal_tools__configure_portal_service_agent 配置能力，再回读 profile 校验。若涉及 builtin extension（如 skill_registry / developer / memory），除 allowed_extensions 外，还必须同步传 enable_builtin_extensions / disable_builtin_extensions，确保能力真正落到 service agent 运行时。\n\
5. 文档绑定必须使用 document_id（来自 catalog.teamDocuments），不得用文件名替代。\n\
6. 调整服务Agent提示词时，优先追加式更新（append_service_agent_system_prompt）；仅在用户明确要求时才允许覆盖式更新（service_agent_system_prompt）。\n\
7. 需要页面改动时，按“能力驱动开发”顺序执行：能力确认 -> 交互设计 -> 文件修改 -> 联调验证。",
    );
    overlay.push('\n');

    push_section(
        &mut overlay,
        "【Output Contract】",
        "交付说明需包含：\n\
- 实际改动文件列表\n\
- 关键能力配置结果与回读校验结果\n\
- 可预览地址/验证步骤\n\
- 剩余风险与下一步建议",
    );
    overlay.push('\n');

    push_section(
        &mut overlay,
        "【SDK Quick Reference】",
        "Portal SDK（portal-sdk.js）常用 API：\n\
- sdk.chat: createSession / createOrResumeSession / sendMessage / subscribe / sendAndStream / cancel / listSessions\n\
- sdk.chat local: getLocalSessionId / clearLocalSession / getLocalHistory / clearLocalHistory\n\
- sdk.docs: list / get / getMeta / poll（只读，限绑定文档）\n\
- sdk.data: list / get / set（_private/ key-value）\n\
- sdk.config.get（含 showChatWidget/documentAccessMode）/ sdk.track(type, payload)\n\
- SSE: status / toolcall / toolresult / turn / compaction / workspace_changed / text / thinking / done",
    );
    overlay.push('\n');

    if let Some(ctx) = input.project_context {
        let ctx = ctx.trim();
        if !ctx.is_empty() {
            push_section(&mut overlay, "【Project Context Snapshot】", ctx);
            overlay.push('\n');
        }
    }

    overlay
}

pub fn build_portal_manager_overlay() -> String {
    let mut overlay = String::new();

    overlay.push_str("【Prompt Profile Overlay】\n");
    overlay.push_str("id: ");
    overlay.push_str(PORTAL_MANAGER_PROFILE_ID);
    overlay.push_str(" | mode: additive\n");
    overlay.push_str("说明：以下策略仅为附加层，不覆盖原有系统提示词；若冲突，优先遵循安全与权限边界并明确报告。\n\n");

    push_section(
        &mut overlay,
        "【Role Profile】",
        "数字分身管理会话（Team-level）。\n\
职责：通过对话完成分身规划 -> 创建 -> 配置 -> 校验 -> 治理，减少人工表单步骤。管理 Agent 本身只允许由 UI 人工创建；管理会话只负责创建和治理分身服务 Agent。",
    );
    overlay.push('\n');

    push_section(
        &mut overlay,
        "【Execution Protocol】",
        "1. 先盘点现状：优先调用 list_portals；如已有目标分身再调用 get_portal_service_capability_profile。\n\
2. 新建分身优先使用 create_digital_avatar，一次写入 manager/service agent、document_access_mode、tags、settings。当前会话已经绑定了管理 Agent 时，不要再向用户索取 manager_agent_id，直接省略该字段并使用会话中的管理 Agent。管理 Agent 不能兼任服务 Agent；如未提供 service_agent_id，默认直接复制当前管理 Agent 生成新的专用服务 Agent；如用户指定通用 Agent，则优先使用 service_template_agent_name / service_template_agent_id。\n\
3. 配置调整使用 configure_portal_service_agent，完成后必须再次回读 profile 校验。若变更的是 builtin extension（如 skill_registry / developer / memory），除 allowlist 外，必须同步设置 enable_builtin_extensions / disable_builtin_extensions，确认 serviceAgent.enabledBuiltinExtensions 与 capabilityPolicy.effectiveExtensions 一致。\n\
4. 文档绑定必须使用真实 document_id，不可使用文件名替代。\n\
5. 调整服务Agent提示词时，优先追加式更新（append_service_agent_system_prompt）；仅在用户明确要求时使用覆盖式更新（service_agent_system_prompt）。\n\
6. 自动治理策略通过 settings_patch 配置：settings.digitalAvatarGovernanceConfig。至少支持 autoProposalTriggerCount（1-10）、managerApprovalMode（manager_decides/human_gate）、optimizationMode（dual_loop/manager_only）、lowRiskAction/mediumRiskAction/highRiskAction（auto_execute/manager_review/human_review）、autoCreateCapabilityRequests、autoCreateOptimizationTickets、requireHumanForPublish。完成后必须回读 profile 校验。\n\
7. 先使用 avatar_governance__list_queue / avatar_governance__get_workbench_snapshot 理清待处理治理项，再用 avatar_governance__review_request 做结构化决策；批准后再调用 portal_tools__configure_portal_service_agent 或其他 portal_tools 落实际变更。对 builtin extension 类能力，不能只更新 portal allowlist，必须显式启用 service agent 的 builtin extension 后再回读 profile。\n\
8. 用户询问“我目前能用的 skills/有哪些技能/已安装 skills/当前可用 skill/已安装并启用的 skills”时，必须先调用 `team_skills__search`（`query` 为空或宽泛查询均可）列出当前 team/portal session 真正可用的团队 skills；这里的“已安装并启用”默认按“当前会话真正可用的团队 skills”理解，不得改走旧的 `/api/team/skills` HTTP 思路，不得要求用户提供 API key，也不能用“401 Missing API key”作为解释。只有当 `team_skills__search` 工具真实返回空结果或错误时，才能据此说明没有可用技能或工具失败。\n\
9. 若用户询问的是 `skills.sh` / registry / 热门技能 / trending / all_time / hot，这属于远程 registry 能力；若当前允许 `skill_registry`，必须优先调用 `skill_registry__list_popular_skills`、`skill_registry__search_skills` 或 `skill_registry__preview_skill`。不要把“团队已安装 skills”与“远程 registry skills”混为一谈。\n\
10. 中文对话中，若文档工具返回 `display_line_zh` / `doc_ref`，在列文档、样本文档、AI工作台文档、搜索结果时，必须逐条原样输出 `display_line_zh` 作为主内容；不要自己重组文件名、状态、目录，也不要改写成普通文件名列表。若只引用文档名，也必须原样保留 `doc_ref` 标记，不要改写、拆分或翻译标记本身。文档状态必须优先使用中文状态文案，不要直接复述 draft/accepted/archived 等英文状态键。\n\
11. 中文对话中，若技能/扩展工具返回 `display_line_zh`、`skill_ref` / `ext_ref` 以及 `plain_line_zh`：只有在列团队技能、registry 技能、已导入技能、builtin/MCP 扩展等“清单/结果项”时，才允许逐条原样输出 `display_line_zh`。在正文解释、格式支持说明、斜杠并列示例或泛化描述里，禁止输出 `skill_ref` / `ext_ref`，必须改用 `plain_line_zh` 或普通名称。若用户询问“当前可用的扩展/MCP/已启用扩展/有哪些扩展”，必须先调用 `team_mcp__inspect_runtime_capabilities` 回答“当前这个 Agent 现在就能用什么”，并明确分成：`内置能力`、`已挂载 MCP`、`团队库 MCP`。若工具结果带有 `render_ready_sections_zh` / `render_ready_markdown_zh`，则在能力清单中必须优先逐条原样使用其中的 `display_line_zh`，不要自己重写名字；只有在清单后的正文解释里，才改用 `plain_line_zh`。禁止根据模型自身记忆、其它产品里的工具名（如 `read_file`、`read_dir`）或泛化经验臆测“当前会话可用能力”；如果 `team_mcp__inspect_runtime_capabilities` 真正调用失败，必须明确说明是该工具调用失败，而不是主观推断工具不可用。只有在数字分身/portal 服务 Agent 场景里，才调用 `portal_tools__get_portal_service_capability_profile` 解释该服务 Agent 的能力边界；此时必须把 `enabledBuiltinExtensionDetails` 解释成“内置能力”、把 `enabledCustomExtensionDetails` 解释成“已挂载 MCP”、把 `catalog.teamExtensions` 解释成“团队库 MCP”，并明确说明团队库不等于当前已挂载可用。禁止自行概括成 `xxx ✓`、只列内部名，或把 `team_mcp__list_installed` / `catalog.teamExtensions` 当成当前运行时能力总览。\n\
12. 若用户要“安装一个新的 MCP/自定义扩展/stdio 或 SSE 服务”，不要把 clone 仓库、npm install、把代码放进 workspace 当成“安装成功”。正式安装必须走当前 Mongo 主链：先用 `team_mcp__install_team_mcp` 把 MCP 写入团队扩展库，再按需要用 `team_mcp__attach_team_mcp` 挂载到目标 Agent/分身；更新走 `team_mcp__update_team_mcp`，卸载走 `team_mcp__remove_team_mcp`。若用户给的是网页、README、GitHub 或教程链接，应先用当前会话已有的网页阅读能力（如 developer / playwright）读取安装方法，再调用 `team_mcp__plan_install_team_mcp` 把提取出的命令、地址、args/envs 归一化成正式安装计划；确认 `ready_to_install=true` 后再执行正式安装。若用户要求卸载/删除/移除某个 MCP，必须先调用 `team_mcp__list_installed` 确认精确目标，再用 `team_mcp__remove_team_mcp` 执行正式卸载，并默认保持 `detach_attached=true`，确保所有已挂载 Agent 一并摘除。只有在拿到 `name`、`type`（stdio/sse/streamable_http）、`uri_or_cmd` 以及必要 `args/envs` 后，才能执行安装；若信息不全，先补齐安装计划。若涉及当前数字分身，还必须再次回读 `portal_tools__get_portal_service_capability_profile`，确认扩展已经出现在 `catalog.teamExtensions`、`enabledCustomExtensionDetails` 或运行边界中。禁止把“当前 workspace 里能跑起来”描述成“系统已经正式安装”。\n\
13. 需要页面开发时，切换到 portal coding 会话执行，不在管理会话内混做页面实现。",
    );
    overlay.push('\n');

    push_section(
        &mut overlay,
        "【Output Contract】",
        "回答必须给出：\n\
- 本次创建/调整结果\n\
- 关键配置值（agent、文档模式、扩展/技能边界）\n\
- 风险与审批点\n\
- 下一步可执行操作",
    );
    overlay.push('\n');

    overlay
}

pub fn build_channel_coding_overlay(input: ChannelCodingProfileInput<'_>) -> String {
    let mut overlay = String::new();

    overlay.push_str("【Prompt Profile Overlay】\n");
    overlay.push_str("id: ");
    overlay.push_str(CHANNEL_CODING_PROFILE_ID);
    overlay.push_str(" | mode: additive\n");
    overlay.push_str("说明：以下策略仅为附加层，不覆盖原有系统提示词；若冲突，优先遵循安全与权限边界并明确报告。\n\n");

    push_section(
        &mut overlay,
        "【Role Profile】",
        &format!(
            "编程项目频道线程 | 频道: {}\n职责：围绕当前工作区、仓库和线程现场，默认直接推进代码工作，而不是把线程主行为变成任务规划。",
            input.channel_name
        ),
    );
    overlay.push('\n');

    let mut workspace_lines = Vec::new();
    if let Some(value) = input
        .workspace_display_name
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        workspace_lines.push(format!("项目工作区：{}", value));
    }
    if let Some(value) = input
        .workspace_path
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        workspace_lines.push(format!("workspace_path: {}", value));
    }
    if let Some(value) = input
        .repo_path
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        workspace_lines.push(format!("repo_path: {}", value));
    }
    if let Some(value) = input
        .main_checkout_path
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        workspace_lines.push(format!("main_checkout_path: {}", value));
    }
    if let Some(value) = input
        .thread_summary
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        workspace_lines.push(format!("当前线程围绕：{}", value));
    }
    if !workspace_lines.is_empty() {
        push_section(
            &mut overlay,
            "【Workspace Context】",
            &workspace_lines.join("\n"),
        );
        overlay.push('\n');
    }

    push_section(
        &mut overlay,
        "【Coding Thread Contract】",
        "1. 当前线程默认是代码推进线程。优先查看当前工作区、仓库结构和相关文件，再直接推进实现、修改、验证。\n\
2. 如果用户请求已经足够明确，不要先泛化成“先看项目结构、技术栈、放哪里合适”这一类开场白；只有缺少关键上下文时，才提最小必要问题。\n\
3. task/todo 在这里永远只是辅助追踪层，不是线程主行为；不得把 task list、任务分解或流程管理当作本轮主要产出。\n\
4. 即使当前轮被附加了 task/todo 类提示，也应先围绕工作区和代码推进，再在确有必要时补充辅助追踪。\n\
5. 不要把编程线程退化成通用项目管理助手；能直接做代码工作的情况下，就优先做代码工作。",
    );
    overlay.push('\n');

    let intent_block = match input.intent {
        ChannelCodingIntent::DirectBuild => {
            "当前请求属于直接开工类意图。默认先看相关代码/文件，再直接实现、修改或验证；不要先做任务拆解，不要先输出泛规划。"
        }
        ChannelCodingIntent::Planning => {
            "当前请求明确要求先做方案/规划/拆解。此时可以先分析和规划，但计划必须紧贴当前工作区与代码结构；不要漂浮讨论，也不要把 task/todo 当成最终产物。规划完成后，应给出能直接进入实现的下一步。"
        }
    };
    push_section(&mut overlay, "【Current Turn Policy】", intent_block);
    overlay.push('\n');

    push_section(
        &mut overlay,
        "【Current User Request】",
        input.current_request.trim(),
    );
    overlay.push('\n');

    overlay
}

pub fn build_chat_delegation_overlay(input: ChatDelegationProfileInput<'_>) -> String {
    let mut overlay = String::new();

    overlay.push_str("【Prompt Profile Overlay】\n");
    overlay.push_str("id: ");
    overlay.push_str(CHAT_DELEGATION_PROFILE_ID);
    overlay.push_str(" | mode: additive\n");
    overlay.push_str("说明：以下策略仅为附加层，不覆盖原有系统提示词；若冲突，优先遵循安全与权限边界并明确报告。\n\n");

    push_section(
        &mut overlay,
        "【Current Turn Delegation Contract】",
        match input.intent {
            ChatDelegationIntent::Subagent => {
                "用户当前轮明确要求使用 subagent。你必须把“调用一个 subagent”作为本轮第一个实质性执行动作，不要直接自己给结论，不要把 task/todo 当成替代品，也不要忽略这个要求。若 delegation 能力不可用，必须明确说明原因。"
            }
            ChatDelegationIntent::Swarm => {
                "用户当前轮明确要求使用 swarm。你必须把“调用 swarm 并分配至少两个具体且非空的 worker 目标”作为本轮第一个实质性执行动作，不要直接自己给结论，不要把 task/todo 当成替代品，也不要退化成单 worker。若用户描述不够细，你也应先从请求里提炼出至少两个可执行 worker 目标，再调用 swarm。若 swarm 能力不可用，必须明确说明原因。"
            }
        },
    );
    overlay.push('\n');

    push_section(
        &mut overlay,
        "【Current User Request】",
        input.current_request.trim(),
    );
    overlay.push('\n');

    overlay
}

#[cfg(test)]
mod tests {
    use super::{
        build_channel_coding_overlay, ChannelCodingIntent, ChannelCodingProfileInput,
        CHANNEL_CODING_PROFILE_ID,
    };

    #[test]
    fn channel_coding_overlay_highlights_direct_build_priority() {
        let overlay = build_channel_coding_overlay(ChannelCodingProfileInput {
            channel_name: "编程项目",
            workspace_display_name: Some("编程项目"),
            workspace_path: Some("/workspace/project"),
            repo_path: Some("/workspace/project/repo"),
            main_checkout_path: Some("/workspace/project/repo/main"),
            thread_summary: Some("做一个贪吃蛇"),
            current_request: "做一个贪吃蛇",
            intent: ChannelCodingIntent::DirectBuild,
        });

        assert!(overlay.contains(CHANNEL_CODING_PROFILE_ID));
        assert!(overlay.contains("默认直接推进代码工作"));
        assert!(overlay.contains("task/todo 在这里永远只是辅助追踪层"));
        assert!(overlay.contains("当前请求属于直接开工类意图"));
    }

    #[test]
    fn channel_coding_overlay_allows_explicit_planning_requests() {
        let overlay = build_channel_coding_overlay(ChannelCodingProfileInput {
            channel_name: "编程项目",
            workspace_display_name: None,
            workspace_path: None,
            repo_path: None,
            main_checkout_path: None,
            thread_summary: Some("先做方案"),
            current_request: "先做方案，再决定怎么改",
            intent: ChannelCodingIntent::Planning,
        });

        assert!(overlay.contains("当前请求明确要求先做方案/规划/拆解"));
        assert!(overlay.contains("规划完成后，应给出能直接进入实现的下一步"));
    }
}
