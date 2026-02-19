# AGIME Team Server

独立团队协作服务端，二进制名为 `agime-team-server`。

## 快速开始

```bash
# 在仓库根目录执行
cargo build --release -p agime-team-server
./target/release/agime-team-server
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
