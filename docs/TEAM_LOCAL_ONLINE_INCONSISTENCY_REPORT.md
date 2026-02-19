# AGIME Team 本地版与线上版功能不一致核查报告

生成日期：2026-02-12  
核查范围：`crates/agime-server`、`crates/agime-team`、`crates/agime-team-server`、`ui/desktop/src/components/team`、`crates/agime/src/agents/team_extension.rs`

## 1. 核心结论

1. 当前并非“同一套 Team 功能在本地/线上不同部署”，而是“SQLite 路由实现”和“Mongo 路由实现”长期并行演进，已形成两套不兼容 API。
2. Desktop 与 MCP 调用层默认把本地与线上当作同构接口，实际在 cloud/LAN 模式下存在 404、401、请求体不匹配、响应解析不匹配等风险。
3. 线上 `SQLite` 分支存在高风险可用性问题：`/api/team` 的鉴权中间件仍强依赖 Mongo。

## 2. 对比对象定义

- 本地版 Team：`agime-server` 集成 `agime-team`（SQLite），通过 `configure_with_team` 合并到本地服务。  
  关键入口：`crates/agime-server/src/routes/mod.rs:55`、`crates/agime-server/src/routes/mod.rs:63`、`crates/agime-server/src/commands/agent.rs:173`
- 线上版 Team：`agime-team-server`，默认数据库类型是 Mongo。  
  关键入口：`crates/agime-team-server/src/config.rs:8`、`crates/agime-team-server/src/config.rs:11`、`crates/agime-team-server/src/main.rs:208`

## 3. 差异清单（按严重度）

## P0

### P0-1 线上 SQLite 模式下 `/api/team` 鉴权中间件与数据库后端不匹配

- 现状：
  - 线上统一将 `/api/team` 包装到 `auth::middleware::auth_middleware`。`crates/agime-team-server/src/main.rs:223`、`crates/agime-team-server/src/main.rs:322`
  - 认证模块默认 re-export 为 Mongo 版本。`crates/agime-team-server/src/auth/mod.rs:12`
  - Mongo 中间件会调用 `state.db.as_mongodb()`，取不到会直接 `SERVICE_UNAVAILABLE`。`crates/agime-team-server/src/auth/middleware_mongo.rs:63`、`crates/agime-team-server/src/auth/middleware_mongo.rs:68`、`crates/agime-team-server/src/auth/middleware_mongo.rs:136`
- 影响：线上若启用 SQLite，Team API 可能整体不可用或大面积 503。

### P0-2 “公开邀请链接”设计与线上全局鉴权冲突

- 现状：
  - 邀请路由在 Team 侧标注为 Public。`crates/agime-team/src/routes/invites.rs:45`、`crates/agime-team/src/routes/invites.rs:46`
  - Mongo Team 路由也定义了 Public invite 路由。`crates/agime-team/src/routes/mongo/teams.rs:235`、`crates/agime-team/src/routes/mongo/teams.rs:236`
  - 但线上 `main` 把整个 `/api/team` 套上鉴权。`crates/agime-team-server/src/main.rs:223`、`crates/agime-team-server/src/main.rs:322`
- 影响：线上“未登录用户通过邀请码加入团队”链路与注释意图不一致，可能被整体拦截。

### P0-3 MCP Team 扩展调用了不存在或不匹配的端点

- 现状：
  - MCP `team_list_installed` 调用 `/installed`。`crates/agime/src/agents/team_extension.rs:1335`
  - 实际 SQLite 路由是 `/resources/installed`。`crates/agime-team/src/routes/sync.rs:112`
  - MCP `team_check_updates` POST 空 `{}`。`crates/agime/src/agents/team_extension.rs:1368`、`crates/agime/src/agents/team_extension.rs:1374`
  - 后端请求体要求 `resourceIds`（反序列化到 `resource_ids`）。`crates/agime-team/src/routes/sync.rs:22`、`crates/agime-team/src/routes/sync.rs:23`
  - MCP `team_get_recommendations` 使用 POST `/recommendations`。`crates/agime/src/agents/team_extension.rs:1620`、`crates/agime/src/agents/team_extension.rs:1622`
  - SQLite 后端是 GET `/recommendations`。`crates/agime-team/src/routes/recommendations.rs:62`
- 影响：MCP 的“已安装/检查更新/推荐”能力在不同部署下不稳定或直接失败。

## P1

### P1-0 本地与线上 Team 接口的鉴权模型不一致

- 本地 `agime-server` 对 `/api/team` 直接放行（不走全局 `X-Secret-Key` 校验）。  
  证据：`crates/agime-server/src/auth.rs:44`、`crates/agime-server/src/auth.rs:46`
- 线上 `agime-team-server` 将整个 `/api/team` 放入认证中间件。  
  证据：`crates/agime-team-server/src/main.rs:223`、`crates/agime-team-server/src/main.rs:322`
- 影响：同一调用在本地可匿名通过、线上需 API Key/Session，客户端若未显式区分认证策略会出现“仅线上失败”。

### P1-1 本地与线上路由面不一致（核心业务接口分叉）

- SQLite（本地）有但 Mongo（线上）没有的关键能力：
  - 技能包/本地安装链路：`/skills/import`、`/skills/validate-package`、`/skills/install-local`、`/skills/local`、`/skills/{id}/export`、`/skills/{id}/files`、`/skills/{id}/convert-to-package`、`/skills/{id}/verify-access`  
    证据：`crates/agime-team/src/routes/skills.rs:285` 到 `crates/agime-team/src/routes/skills.rs:305`
  - 本地安装桥接：`/recipes/install-local`、`/extensions/install-local`  
    证据：`crates/agime-team/src/routes/recipes.rs:130`、`crates/agime-team/src/routes/extensions.rs:147`
  - 同步/安装资源接口：`/resources/check-updates`、`/resources/batch-install`、`/resources/installed`  
    证据：`crates/agime-team/src/routes/sync.rs:110` 到 `crates/agime-team/src/routes/sync.rs:112`
  - 推荐与统一源：`/recommendations`、`/unified/sources`  
    证据：`crates/agime-team/src/routes/recommendations.rs:62`、`crates/agime-team/src/routes/unified.rs:107`
- Mongo（线上）有但 SQLite（本地）没有的关键能力：
  - 统计/趋势/推荐：`/teams/{team_id}/stats`、`/teams/{team_id}/trending`、`/teams/{team_id}/recommendations/*`  
    证据：`crates/agime-team/src/routes/mongo/stats.rs:23` 到 `crates/agime-team/src/routes/mongo/stats.rs:26`
  - 同步接口形态不同：`/teams/{team_id}/sync/check`（GET + since）  
    证据：`crates/agime-team/src/routes/mongo/sync.rs:18`、`crates/agime-team/src/routes/mongo/sync.rs:35`
  - 统一搜索接口不同：`/teams/{team_id}/search`  
    证据：`crates/agime-team/src/routes/mongo/unified.rs:48`
  - 额外模块：folders/audit/user_groups/smart_log  
    证据：`crates/agime-team/src/routes/mongo/mod.rs:29` 到 `crates/agime-team/src/routes/mongo/mod.rs:35`

### P1-2 同名接口的响应契约不一致（UI/SDK 易解析失败）

- 以 `GET /skills/{id}` 为例：
  - SQLite：返回 `SkillResponse` 扁平对象。`crates/agime-team/src/routes/skills.rs:400`、`crates/agime-team/src/routes/skills.rs:408`
  - Mongo：返回 `{ "skill": { ... } }` 包装对象。`crates/agime-team/src/routes/mongo/skills.rs:292`、`crates/agime-team/src/routes/mongo/skills.rs:325`
- `recipes` 与 `extensions` 同样存在 wrapper 差异：  
  `crates/agime-team/src/routes/recipes.rs:211`、`crates/agime-team/src/routes/recipes.rs:219` 对比 `crates/agime-team/src/routes/mongo/recipes.rs:287`、`crates/agime-team/src/routes/mongo/recipes.rs:315`；  
  `crates/agime-team/src/routes/extensions.rs:235`、`crates/agime-team/src/routes/extensions.rs:243` 对比 `crates/agime-team/src/routes/mongo/extensions.rs:313`、`crates/agime-team/src/routes/mongo/extensions.rs:344`

### P1-3 列表查询参数要求不一致（Mongo 强制 teamId，SQLite 可选）

- Mongo list 路由强制 `teamId`，缺失则 400。  
  证据：`crates/agime-team/src/routes/mongo/skills.rs:118`、`crates/agime-team/src/routes/mongo/recipes.rs:115`、`crates/agime-team/src/routes/mongo/extensions.rs:131`
- SQLite 对应查询结构是 `Option<String>`。  
  证据：`crates/agime-team/src/routes/skills.rs:30`、`crates/agime-team/src/routes/recipes.rs:26`、`crates/agime-team/src/routes/extensions.rs:26`

### P1-4 Desktop 在 remote 模式默认假设“本地/线上接口同构”

- `fetchApi` 统一拼 `/api/team`，不做后端类型适配。`ui/desktop/src/components/team/api.ts:35`、`ui/desktop/src/components/team/api.ts:137`
- `getSkill/getRecipe/getExtension` 直接按扁平对象解析。  
  证据：`ui/desktop/src/components/team/api.ts:497`、`ui/desktop/src/components/team/api.ts:815`、`ui/desktop/src/components/team/api.ts:936`
- remote 安装是“两步法”：先从 remote 拉资源，再回本地调用 `install-local`。  
  证据：`ui/desktop/src/components/team/api.ts:443` 到 `ui/desktop/src/components/team/api.ts:461`、`ui/desktop/src/components/team/api.ts:764` 到 `ui/desktop/src/components/team/api.ts:782`、`ui/desktop/src/components/team/api.ts:884` 到 `ui/desktop/src/components/team/api.ts:902`
- 同步端点按 SQLite 设计：`/resources/installed`、`/resources/check-updates`、`/resources/batch-install`。  
  证据：`ui/desktop/src/components/team/api.ts:973`、`ui/desktop/src/components/team/api.ts:997`、`ui/desktop/src/components/team/api.ts:1045`

### P1-5 `batch-install` 请求体在 UI 与后端定义不一致

- UI 发送 `{ resourceIds: string[] }`。`ui/desktop/src/components/team/api.ts:1032`、`ui/desktop/src/components/team/api.ts:1047`
- 后端定义为 `{ resources: ResourceRefApi[] }`。`crates/agime-team/src/routes/sync.rs:29`、`crates/agime-team/src/routes/sync.rs:30`
- 影响：即使在 SQLite 路径也可能反序列化失败或无法按预期批量安装。

### P1-6 健康检查字段语义不一致（前端状态误判）

- local adapter：读 `/status`，期待纯文本 `ok`。  
  证据：`ui/desktop/src/components/team/sources/adapters/localAdapter.ts:125`、`ui/desktop/src/components/team/sources/adapters/localAdapter.ts:139`
- cloud/lan adapter：读 `/health`，将 `data.database === 'ok'` 视为正常。  
  证据：`ui/desktop/src/components/team/sources/adapters/cloudAdapter.ts:79`、`ui/desktop/src/components/team/sources/adapters/cloudAdapter.ts:94`、`ui/desktop/src/components/team/sources/adapters/lanAdapter.ts:79`、`ui/desktop/src/components/team/sources/adapters/lanAdapter.ts:94`
- team-server `/health` 返回的是 `"database": "mongodb" | "sqlite"`。  
  证据：`crates/agime-team-server/src/main.rs:360` 到 `crates/agime-team-server/src/main.rs:373`

## P2

### P2-1 `verify-access` 用户标识来源不一致，存在隐式行为差异

- `verify_skill_access` 使用 `state.user_id`，不读请求扩展用户。  
  证据：`crates/agime-team/src/routes/skills.rs:1028`、`crates/agime-team/src/routes/skills.rs:1041`
- 本地集成时 `user_id` 默认可落到 `local-user`。`crates/agime-server/src/commands/agent.rs:170`
- team-server 的 `configure_routes` 初始 `user_id` 为空字符串，注释依赖中间件覆盖。`crates/agime-team/src/routes/mod.rs:52`、`crates/agime-team/src/routes/mod.rs:56`
- 影响：不同运行模式下授权 token 的 user 语义可能不一致。

## 4. 业务影响汇总

1. 用户侧：同一 UI 操作在本地成功、线上失败（或返回结构不同导致 UI 异常），带来明显“功能不一致”体验。
2. 运维侧：线上数据库切换到 SQLite 时存在直接不可用风险。
3. 生态侧：MCP Team 工具与 Desktop 都存在对接口稳定性的硬编码假设，回归成本高。

## 5. 修复建议（执行顺序）

1. P0：拆分并正确选择 Team 鉴权中间件（Mongo/SQLite），避免 SQLite 落到 Mongo 中间件。
2. P0：为 invite public 路由提供白名单或单独 public router，保证未登录邀请链路可用。
3. P0：修正 MCP Team 扩展端点与方法（`/resources/installed`、`check-updates` 请求体、`GET /recommendations`）。
4. P1：定义统一 Team API 契约（建议 OpenAPI + 契约测试），先统一技能/配方/扩展详情响应结构。
5. P1：给 Team Server 增加兼容层（alias 路由 + 响应适配），承接现有 Desktop 调用。
6. P1：修正 Desktop `batchInstall` 请求体为 `resources`，并按后端能力做 endpoint 选择。
7. P1：修正 cloud/lan 健康检查解析逻辑，`database` 字段按枚举处理。
8. P2：统一 `verify-access` 的用户来源（优先请求上下文中的认证用户）。

## 6. 建议验收用例（最小集）

1. 本地 SQLite：完整跑通 `skills/recipes/extensions` 的分享、详情、安装、卸载、推荐、同步。
2. 线上 Mongo：同样用例跑通，并验证与本地响应结构一致性（至少保证兼容）。
3. 线上 SQLite：覆盖 `/api/team` 任意受保护路由，验证不再出现中间件数据库类型错误。
4. 邀请链路：未登录用户访问 `GET /api/team/invites/{code}` 与 `POST /api/team/invites/{code}/accept`。
5. MCP 回归：`team_list_installed`、`team_check_updates`、`team_get_recommendations`、`team_get_stats`。
