//! Prompt profiles for portal manager/coding sessions.
//!
//! Profiles are additive overlays and must not replace agent base prompts.

pub const PORTAL_CODING_PROFILE_ID: &str = "portal_coding_v1";
pub const PORTAL_MANAGER_PROFILE_ID: &str = "portal_manager_v1";

pub struct PortalCodingProfileInput<'a> {
    pub portal_slug: &'a str,
    pub project_path: &'a str,
    pub portal_policy_overlay: Option<&'a str>,
    pub project_context: Option<&'a str>,
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
4. 若能力不足，先调用 portal_tools__configure_portal_service_agent 配置能力，再回读 profile 校验。\n\
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
职责：通过对话完成分身规划 -> 创建 -> 配置 -> 校验 -> 治理，减少人工表单步骤。",
    );
    overlay.push('\n');

    push_section(
        &mut overlay,
        "【Execution Protocol】",
        "1. 先盘点现状：优先调用 list_portals；如已有目标分身再调用 get_portal_service_capability_profile。\n\
2. 新建分身优先使用 create_digital_avatar，一次写入 manager/service agent、document_access_mode、tags、settings。\n\
3. 配置调整使用 configure_portal_service_agent，完成后必须再次回读 profile 校验。\n\
4. 文档绑定必须使用真实 document_id，不可使用文件名替代。\n\
5. 调整服务Agent提示词时，优先追加式更新（append_service_agent_system_prompt）；仅在用户明确要求时使用覆盖式更新（service_agent_system_prompt）。\n\
6. 自动治理阈值通过 settings_patch 配置：settings.digitalAvatarGovernanceConfig.autoProposalTriggerCount（1-10）。推荐策略：3=激进（快速提案）、5=平衡（默认）、7=保守（减少提案频率）。\n\
7. 需要页面开发时，切换到 portal coding 会话执行，不在管理会话内混做页面实现。",
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
