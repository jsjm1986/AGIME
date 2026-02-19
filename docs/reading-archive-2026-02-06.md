# AGIME 阅读归档（2026-02-06）

## 归档说明
- 归档时间: 2026-02-06
- 归档人: Codex
- 归档目的: 保存“全面阅读项目”的已读范围与理解结果
- 状态: 已保存

## 已读文档与源码清单
### 1. 前端会话与上下文（UI）
- `ui/desktop/src/components/Hub.tsx`
- `ui/desktop/src/components/Pair.tsx`
- `ui/desktop/src/components/BaseChat.tsx`
- `ui/desktop/src/hooks/useChatStream.ts`
- `ui/desktop/src/services/ChatStreamManager.ts`
- `ui/desktop/src/sessions.ts`
- `ui/desktop/src/renderer.tsx`
- `ui/desktop/src/renderer-web.tsx`
- `ui/desktop/src/hooks/useAgent.ts`
- `ui/desktop/src/components/ConfigContext.tsx`
- `ui/desktop/src/components/ChatInput.tsx`
- `ui/desktop/src/contexts/ChatContext.tsx`
- `ui/desktop/src/hooks/useNavigation.ts`
- `ui/desktop/src/components/ModelAndProviderContext.tsx`
- `ui/desktop/src/sharedSessions.ts`
- `ui/desktop/src/sessionLinks.ts`
- `ui/desktop/src/App.tsx`

### 2. 桌面主进程与平台桥接
- `ui/desktop/src/agimed.ts`
- `ui/desktop/src/main.ts`
- `ui/desktop/src/preload.ts`
- `ui/desktop/src/platform/types.ts`
- `ui/desktop/src/platform/web.ts`

### 3. 本地服务（agime-server）
- `crates/agime-server/src/routes/reply.rs`
- `crates/agime-server/src/routes/agent.rs`
- `crates/agime-server/src/routes/session.rs`
- `crates/agime-server/src/routes/shared_session.rs`
- `crates/agime-server/src/routes/mod.rs`
- `crates/agime-server/src/routes/web_ui.rs`
- `crates/agime-server/src/state.rs`
- `crates/agime-server/src/auth.rs`
- `crates/agime-server/src/commands/agent.rs`

### 4. AGIME 核心执行链路（agime）
- `crates/agime/src/execution/manager.rs`
- `crates/agime/src/agents/agent.rs`
- `crates/agime/src/agents/reply_parts.rs`
- `crates/agime/src/agents/tool_execution.rs`
- `crates/agime/src/agents/schedule_tool.rs`
- `crates/agime/src/agents/extension_manager.rs`
- `crates/agime/src/tool_inspection.rs`
- `crates/agime/src/security/security_inspector.rs`
- `crates/agime/src/security/scanner.rs`
- `crates/agime/src/permission/permission_inspector.rs`
- `crates/agime/src/permission/permission_judge.rs`
- `crates/agime/src/session/session_manager.rs`
- `crates/agime/src/scheduler.rs`
- `crates/agime/src/scheduler_trait.rs`
- `crates/agime/src/providers/factory.rs`
- `crates/agime/src/providers/provider_registry.rs`
- `crates/agime/src/providers/base.rs`
- `crates/agime/src/providers/pricing.rs`
- `crates/agime/src/model.rs`
- `crates/agime/src/config/base.rs`
- `crates/agime/src/config/agime_mode.rs`
- `crates/agime/src/config/extensions.rs`
- `crates/agime/src/capabilities/mod.rs`
- `crates/agime/src/capabilities/registry.rs`
- `crates/agime/src/capabilities/runtime.rs`
- `crates/agime/src/capabilities/types.rs`
- `crates/agime/src/lib.rs`
- `crates/agime/src/conversation/mod.rs`
- `crates/agime/src/context_mgmt/mod.rs`
- `crates/agime/src/hints/load_hints.rs`
- `crates/agime/src/oauth/mod.rs`

### 5. CLI 与 ACP
- `crates/agime-cli/src/main.rs`
- `crates/agime-cli/src/cli.rs`
- `crates/agime-cli/src/commands/acp.rs`
- `crates/agime-cli/src/commands/web.rs`

### 6. Team 前端与 Team Server 主链路
- `ui/desktop/src/components/team/api.ts`
- `ui/desktop/src/components/team/auth/authAdapter.ts`
- `ui/desktop/src/components/team/sources/sourceManager.ts`
- `ui/desktop/src/components/team/sources/adapters/localAdapter.ts`
- `ui/desktop/src/components/team/sources/adapters/lanAdapter.ts`
- `ui/desktop/src/components/team/sources/adapters/cloudAdapter.ts`
- `crates/agime-team/src/lib.rs`
- `crates/agime-team/src/routes/mod.rs`
- `crates/agime-team-server/src/main.rs`
- `crates/agime-team-server/src/config.rs`
- `crates/agime-team-server/src/state.rs`
- `crates/agime-team-server/src/auth/mod.rs`
- `crates/agime-team-server/src/auth/middleware_mongo.rs`
- `crates/agime-team-server/src/agent/mod.rs`

### 7. Team 服务层（SQLite）
- `crates/agime-team/src/services/mod.rs`
- `crates/agime-team/src/services/mongo.rs`
- `crates/agime-team/src/services/team_service.rs`
- `crates/agime-team/src/services/member_service.rs`
- `crates/agime-team/src/services/invite_service.rs`
- `crates/agime-team/src/services/audit_service.rs`
- `crates/agime-team/src/services/cleanup_service.rs`
- `crates/agime-team/src/services/concurrency.rs`
- `crates/agime-team/src/services/document_service.rs`
- `crates/agime-team/src/services/skill_service.rs`
- `crates/agime-team/src/services/recipe_service.rs`
- `crates/agime-team/src/services/extension_service.rs`
- `crates/agime-team/src/services/install_service.rs`
- `crates/agime-team/src/services/package_service.rs`
- `crates/agime-team/src/services/stats_service.rs`
- `crates/agime-team/src/services/recommendation_service.rs`

### 8. Team 服务层（Mongo）
- `crates/agime-team/src/services/team_service_mongo.rs`
- `crates/agime-team/src/services/document_service_mongo.rs`
- `crates/agime-team/src/services/skill_service_mongo.rs`
- `crates/agime-team/src/services/recipe_service_mongo.rs`
- `crates/agime-team/src/services/extension_service_mongo.rs`
- `crates/agime-team/src/services/folder_service_mongo.rs`
- `crates/agime-team/src/services/audit_service_mongo.rs`
- `crates/agime-team/src/services/stats_service_mongo.rs`
- `crates/agime-team/src/services/recommendation_service_mongo.rs`
- `crates/agime-team/src/services/cleanup_service_mongo.rs`

### 9. MCP 扩展核心（agime-mcp）
- `crates/agime-mcp/src/lib.rs`
- `crates/agime-mcp/src/mcp_server_runner.rs`
- `crates/agime-mcp/src/developer/rmcp_developer.rs`
- `crates/agime-mcp/src/developer/text_editor.rs`
- `crates/agime-mcp/src/developer/shell.rs`
- `crates/agime-mcp/src/autovisualiser/mod.rs`
- `crates/agime-mcp/src/computercontroller/mod.rs`
- `crates/agime-mcp/src/computercontroller/xlsx_tool.rs`
- `crates/agime-mcp/src/computercontroller/pdf_tool.rs`
- `crates/agime-mcp/src/computercontroller/docx_tool.rs`
- `crates/agime-mcp/src/computercontroller/platform/mod.rs`
- `crates/agime-mcp/src/computercontroller/platform/linux.rs`
- `crates/agime-mcp/src/computercontroller/platform/windows.rs`
- `crates/agime-mcp/src/computercontroller/platform/macos.rs`

### 10. 架构与工程文档
- `README.md`
- `docs/HOW_IT_WORKS.md`
- `docs/ARCHITECTURE.md`
- `Cargo.toml`
- `crates/agime/Cargo.toml`
- `crates/agime-cli/Cargo.toml`
- `crates/agime-server/Cargo.toml`
- `crates/agime-mcp/Cargo.toml`
- `crates/agime-team/Cargo.toml`
- `crates/agime-team-server/Cargo.toml`
- `crates/agime-bench/Cargo.toml`
- `crates/agime-test/Cargo.toml`

## 阅读结论摘要
- 已完成核心执行链路的端到端理解: 前端输入与流式会话 -> server SSE -> agime agent/tool loop -> 会话持久化与调度。
- 已完成 Team 子系统的双存储实现理解: SQLite 与 Mongo 的服务层职责和差异。
- 已完成 MCP 三大扩展的关键行为理解: developer、autovisualiser、computercontroller。
- 已完成项目级架构文档与实际代码链路的交叉校验，结论一致。

## 说明
- 本归档为阅读记录与理解范围清单，不包含代码改动。
- 静态资源与图标类文件未逐行精读（不影响主系统行为理解）。
