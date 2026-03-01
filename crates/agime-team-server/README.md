# AGIME Team Server

独立团队协作服务端，二进制名为 `agime-team-server`。

## 快速开始

```bash
# 在仓库根目录执行
cargo build --release -p agime-team-server
./target/release/agime-team-server
```

可选：先准备环境变量文件（团队服务目录内）。

```bash
cp crates/agime-team-server/.env.example crates/agime-team-server/.env
```

TLS 后端（对外 HTTP/MCP 连接）默认使用 `rustls`。如需切换系统证书链的 `native-tls`：

```bash
# native-tls 构建（注意关闭默认 feature）
cargo build --release -p agime-team-server --no-default-features --features tls-native
```

### 升级回归（Canary）命令

```bash
# 默认 rustls
cargo check -p agime-team-server

# native-tls 可选链路
cargo check -p agime-team-server --no-default-features --features tls-native

# MCP 关键分支单测
cargo test -p agime-team-server mcp_connector::tests
```

可选参数：

```bash
./target/release/agime-team-server --port 9090
```

还支持 MCP 子命令（stdio）：

```bash
./target/release/agime-team-server mcp developer
./target/release/agime-team-server mcp memory
./target/release/agime-team-server mcp computercontroller
./target/release/agime-team-server mcp tutorial
./target/release/agime-team-server mcp autovisualiser
```

## 环境变量配置

以下为 `src/config.rs` 中的主要配置项与默认值：

| 变量 | 默认值 | 说明 |
|------|--------|------|
| `DATABASE_TYPE` | `mongodb` | `mongodb` 或 `sqlite` |
| `TEAM_SERVER_HOST` | `0.0.0.0` | 监听地址 |
| `TEAM_SERVER_PORT` | `8080` | 监听端口 |
| `DATABASE_URL` | `mongodb://localhost:27017` | 数据库连接串 |
| `DATABASE_NAME` | `agime_team` | 数据库名（MongoDB） |
| `DATABASE_MAX_CONNECTIONS` | `10` | 最大连接数 |
| `ADMIN_EMAILS` | 空 | 自动授予 admin 的邮箱列表（逗号分隔） |
| `REGISTRATION_MODE` | `open` | `open` / `approval` / `disabled` |
| `MAX_API_KEYS_PER_USER` | `10` | 每用户最大 API Key 数 |
| `LOGIN_MAX_FAILURES` | `5` | 最大失败次数 |
| `LOGIN_LOCKOUT_MINUTES` | `15` | 锁定时长（分钟） |
| `SESSION_SLIDING_WINDOW_HOURS` | `2` | Session 滑动续期窗口 |
| `SECURE_COOKIES` | `false` | HTTPS 下建议开启 |
| `BASE_URL` | 空 | 用于邀请链接和 Portal URL |
| `PORTAL_TEST_BASE_URL` | 空 | Portal 测试地址 |
| `CORS_ALLOWED_ORIGINS` | 空 | 逗号分隔白名单；空时 mirror_request |
| `WORKSPACE_ROOT` | `./data/workspaces` | Mission/Session 工作目录 |
| `TEAM_AGENT_RESOURCE_MODE` | `explicit` | Agent 资源模式 |
| `TEAM_AGENT_SKILL_MODE` | `on_demand` | Agent 技能模式 |
| `TEAM_AGENT_AUTO_EXTENSION_POLICY` | `reviewed_only` | 自动扩展策略 |
| `TEAM_AGENT_AUTO_INSTALL_EXTENSIONS` | `true` | 缺失扩展时是否自动安装 |
| `TEAM_AGENT_EXTENSION_CACHE_ROOT` | `./data/runtime/extensions` | 扩展缓存目录 |
| `TEAM_MCP_ENABLE_TASK_CALLS` | `true` | 是否对声明 `taskSupport` 的 MCP 工具启用任务调用模式 |
| `TEAM_MCP_ENABLE_ELICITATION` | `false` | 是否向 MCP 服务端声明 elicitation 能力（默认关闭） |
| `TEAM_MCP_ELICITATION_DEFAULT_ACTION` | `cancel` | 无交互桥接时的 elicitation 默认动作：`cancel` / `decline` |
| `MCP_TASK_TIMEOUT_SECS` | `600` | MCP 任务调用总超时（`CreateTaskResult` 轮询 `tasks/get` 到 `tasks/result`）。兼容旧变量 `TEAM_MCP_TOOL_TIMEOUT_SECS`（优先读取 `MCP_TASK_TIMEOUT_SECS`） |
| `AGIME_EXTENSION_TOOL_CACHE_TTL_SECS` | `5` | 扩展工具列表缓存 TTL（不支持 `tools/list_changed` 时） |
| `AGIME_EXTENSION_TOOL_CACHE_TTL_LIST_CHANGED_SECS` | `300` | 扩展工具列表缓存 TTL（支持 `tools/list_changed` 时） |

## API 概览

公开接口：

- `GET /`
- `GET /health`
- `POST /api/auth/register`
- `POST /api/auth/login`
- `POST /api/auth/logout`
- `GET /api/auth/session`
- `POST /api/auth/login/password`（MongoDB）

受保护接口（认证中间件支持 `agime_session` Cookie / `X-API-Key` / `Authorization: Bearer`）：

- `/api/auth/me`
- `/api/auth/keys`
- `/api/team/*`
- `/api/team/agent/*`（MongoDB）
- `/api/team/agent/chat/*`（MongoDB）
- `/api/team/agent/mission/*`（MongoDB）
- `/api/teams/*`（AI Describe，MongoDB）

管理接口：

- `/api/auth/admin/registrations`
- `/api/auth/admin/registrations/{id}/approve`
- `/api/auth/admin/registrations/{id}/reject`

## Portal SDK（对外嵌入）

公开门户会提供内置 SDK：`GET /p/{slug}/portal-sdk.js`。推荐在门户页面中直接引用该地址。

```html
<script src="portal-sdk.js"></script>
<script>
  const sdk = new PortalSDK({ slug: "your-portal-slug" });
</script>
```

### Chat API（当前能力）

- `sdk.chat.createSession()`
- `sdk.chat.createOrResumeSession()`
- `sdk.chat.sendMessage(sessionId, text)`
- `sdk.chat.subscribe(sessionId, lastEventId?)`
- `sdk.chat.sendAndStream(text, handlers)`
- `sdk.chat.cancel(sessionId)`
- `sdk.chat.listSessions()`
- 本地会话辅助：`getLocalSessionId()` / `clearLocalSession()` / `getLocalHistory()` / `clearLocalHistory()`

### SSE 事件（实时状态）

除 `text` / `thinking` / `done` 外，还会有：

- `status`
- `toolcall`
- `toolresult`
- `turn`
- `compaction`
- `workspace_changed`

建议前端把 `status` 作为主进度文案，并在长耗时阶段展示“仍在处理中”类提示。

### 会话持久化行为

- 访客标识、session id、消息历史使用 `localStorage` 持久化（按 `slug + visitor_id` 隔离）。
- 历史消息默认保留最近 200 条。
- 会自动迁移旧版 `sessionStorage` 键，刷新页面后可恢复会话与上下文。

### 配置项（`/p/{slug}/api/config`）

- `showChatWidget`: 是否注入默认悬浮聊天窗（默认 `true`）。
- `documentAccessMode`: 文档访问模式（`read_only` / `co_edit_draft` / `controlled_write`）。
- `agentWelcomeMessage`、`chatApi` 等基础字段。

## 功能差异（MongoDB vs SQLite）

| 能力 | MongoDB | SQLite |
|------|:-------:|:------:|
| 团队/成员/邀请/资源共享 | ✅ | ✅ |
| 审计/统计/智能日志 | ✅ | ❌ |
| Portal 管理与公开访问 | ✅ | ❌ |
| Team Agent / Chat / Mission | ✅ | ❌ |
| AI Describe 路由 | ✅ | ❌ |
| Admin 审批注册 | ✅ | ❌ |

## Web Admin

服务端会在以下目录自动查找前端构建产物并挂载到 `/admin`：

- `./web-admin/dist`
- `./crates/agime-team-server/web-admin/dist`

构建方式：

```bash
cd crates/agime-team-server/web-admin
npm install
npm run build
```

## 生产部署建议

- 使用 HTTPS 反向代理（Nginx/Traefik/Caddy）
- `SECURE_COOKIES=true`（HTTPS 场景）
- 配置 `CORS_ALLOWED_ORIGINS`
- 仅暴露必要端口，限制数据库访问来源
