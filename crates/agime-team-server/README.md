# AGIME Team Server

独立团队协作服务端，二进制名为 `agime-team-server`。它与桌面版 AGIME 相互独立、单独版本化发布（镜像 tag 形如 `team-vX.Y.Z`，当前 `1.0.0`）。

## 部署（Docker Compose，推荐）

面向生产/团队部署的最快路径。`docker-compose.yml` 已内置一个 `mongo:7` 服务，
一条命令即可拉起「Team Server + MongoDB」整套环境，默认从 GHCR 拉取官方镜像
`ghcr.io/jsjm1986/agime-team-server`，无需在本机编译。

```bash
# 1. 进入团队服务目录
cd crates/agime-team-server

# 2. 准备环境变量（BOOTSTRAP_ADMIN_PASSWORD 必填，否则拒绝启动）
cp .env.example .env
#   编辑 .env，至少设置一个强口令：
#   BOOTSTRAP_ADMIN_PASSWORD=<your-strong-password>

# 3. 拉起整套服务（Team Server + MongoDB）
docker compose up -d

# 4. 查看状态 / 日志
docker compose ps
docker compose logs -f team-server
```

启动后：

- 健康检查：`http://<host>:8080/health`
- Web 管理台：`http://<host>:8080/admin`
- 用 `.env` 里的 `BOOTSTRAP_ADMIN_USERNAME` / `BOOTSTRAP_ADMIN_PASSWORD` 登录

升级到新版本：

```bash
docker compose pull        # 拉取最新镜像
docker compose up -d        # 平滑重启，MongoDB 数据保留在 mongo-data 卷
```

> 默认从 GHCR 拉取已发布镜像。若想改为本机从源码构建，编辑 `docker-compose.yml`，
> 注释掉 `team-server` 服务的 `image:` 行，并取消其下方 `build:` 块的注释。
>
> 也可直接指定镜像版本，例如 `ghcr.io/jsjm1986/agime-team-server:1.0.0`，
> 而非 `:latest`，以获得可复现的部署。

### 仅运行容器（已有外部 MongoDB）

如果你已有独立的 MongoDB，不需要内置的那个，可直接跑容器：

```bash
docker run -d --name agime-team-server \
  -p 8080:8080 \
  -e DATABASE_TYPE=mongodb \
  -e DATABASE_URL='mongodb://<your-mongo-host>:27017' \
  -e DATABASE_NAME=agime_team \
  -e BOOTSTRAP_ADMIN_USERNAME=admin \
  -e BOOTSTRAP_ADMIN_PASSWORD='<your-strong-password>' \
  -v agime-team-data:/data \
  ghcr.io/jsjm1986/agime-team-server:latest
```

## 源码构建（开发 / 自定义）

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
| `TEAM_MCP_ELICITATION_DEFAULT_ACTION` | `cancel` | 无交互桥接时的 elicitation 默认动作：`cancel` / `decline` / `accept` |
| `TEAM_MCP_ELICITATION_DEFAULT_CONTENT_JSON` | _(unset)_ | 当默认动作为 `accept` 时使用的 JSON 对象内容（主要用于 form elicitation）。未设置时 form 会自动回退为 `cancel` 以避免无效响应 |
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

- **数据库**：正式团队使用 MongoDB（默认即是），以获得完整能力（Agent 路由、AI 触发、审计、Portal、数字分身等）；SQLite 仅适合单机、功能精简的轻量场景。
- **管理员口令**：务必通过 `.env` 的 `BOOTSTRAP_ADMIN_PASSWORD` 设置强口令，切勿沿用内置默认值。compose 在未设置该变量时会拒绝启动。
- **HTTPS**：前置 HTTPS 反向代理（Nginx/Traefik/Caddy），并设置 `SECURE_COOKIES=true`、`BASE_URL=https://<your-domain>`。
- **跨域**：按需配置 `CORS_ALLOWED_ORIGINS` 白名单。
- **网络与数据**：仅对外暴露必要端口；MongoDB 不要直接暴露公网；定期备份 `mongo-data` 卷。
- **可复现部署**：镜像建议钉到具体版本（如 `:1.0.0`）而非 `:latest`。
