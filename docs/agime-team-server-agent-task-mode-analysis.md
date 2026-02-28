# agime-team-server Agent 任务模式（Mission Execution Mode）业务逻辑全量梳理

## 1. 说明与范围

### 1.1 任务定义
本次将“agent 的任务模式”定义为 **Mission Track 的执行模式 `execution_mode`**：
- `sequential`（顺序步骤模式）
- `adaptive`（自适应目标树模式 / AGE）

对应核心定义位于 `crates/agime-team-server/src/agent/mission_mongo.rs:68`。

### 1.2 本文覆盖的完整代码面
后端核心：
- `crates/agime-team-server/src/agent/mission_mongo.rs`
- `crates/agime-team-server/src/agent/service_mongo.rs`
- `crates/agime-team-server/src/agent/mission_routes.rs`
- `crates/agime-team-server/src/agent/mission_executor.rs`
- `crates/agime-team-server/src/agent/adaptive_executor.rs`
- `crates/agime-team-server/src/agent/runtime.rs`
- `crates/agime-team-server/src/agent/mission_manager.rs`
- `crates/agime-team-server/src/agent/task_manager.rs`
- `crates/agime-team-server/src/agent/executor_mongo.rs`
- `crates/agime-team-server/src/agent/mod.rs`
- `crates/agime-team-server/src/main.rs`

前端与控制台：
- `crates/agime-team-server/web-admin/src/api/mission.ts`
- `crates/agime-team-server/web-admin/src/components/mission/CreateMissionDialog.tsx`
- `crates/agime-team-server/web-admin/src/components/mission/MissionCard.tsx`
- `crates/agime-team-server/web-admin/src/components/mission/GoalTreeView.tsx`
- `crates/agime-team-server/web-admin/src/components/mission/MissionStepList.tsx`
- `crates/agime-team-server/web-admin/src/components/mission/MissionStepDetail.tsx`
- `crates/agime-team-server/web-admin/src/components/mission/StepApprovalPanel.tsx`
- `crates/agime-team-server/web-admin/src/pages/MissionBoardPage.tsx`
- `crates/agime-team-server/web-admin/src/pages/MissionDetailPage.tsx`
- `crates/agime-team-server/web-admin/src/components/team/MissionsPanel.tsx`
- `crates/agime-team-server/web-admin/src/components/documents/QuickTaskMenu.tsx`
- `crates/agime-team-server/web-admin/src/i18n/locales/en.ts`
- `crates/agime-team-server/web-admin/src/i18n/locales/zh.ts`

补充环境/装配：
- `crates/agime-team-server/src/config.rs`
- `crates/agime-team-server/README.md`

### 1.3 与“任务模式”容易混淆但非本次主轴
`TEAM_AGENT_RESOURCE_MODE` / `TEAM_AGENT_SKILL_MODE` 属于“工具与技能装配模式”，不是 Mission 的 `execution_mode`。
- 定义：`crates/agime-team-server/src/config.rs:110`
- 注入环境：`crates/agime-team-server/src/main.rs:147`
- 消费：`crates/agime-team-server/src/agent/executor_mongo.rs:154`

---

## 2. 领域模型（Mission/Step/Goal）

### 2.1 Mission 状态
`MissionStatus` 定义：`crates/agime-team-server/src/agent/mission_mongo.rs:17`
- `draft`
- `planning`
- `planned`
- `running`
- `paused`
- `completed`
- `failed`
- `cancelled`

### 2.2 执行模式
`ExecutionMode` 定义：`crates/agime-team-server/src/agent/mission_mongo.rs:68`
- `sequential`
- `adaptive`

默认值：`sequential`（`Default` 实现，`mission_mongo.rs:73`）。

### 2.3 Step 与 Goal
Step 状态：`StepStatus`（`mission_mongo.rs`）
- `pending` / `awaiting_approval` / `running` / `completed` / `failed` / `skipped`

Goal 状态：`GoalStatus`（`mission_mongo.rs:89`）
- `pending` / `running` / `awaiting_approval` / `completed` / `pivoting` / `abandoned` / `failed`

Mission 主文档：`MissionDoc`（`mission_mongo.rs:211`）
- 核心字段：`execution_mode`、`steps`、`goal_tree`、`current_step`、`current_goal_id`
- 预算/可靠性字段：`token_budget`、`step_timeout_seconds`、`step_max_retries`
- 结果字段：`error_message`、`final_summary`
- 工作区字段：`workspace_path`

---

## 3. 数据库层（Service）业务规则

### 3.1 创建任务
入口：`create_mission`（`service_mongo.rs:1836`）
- `execution_mode = req.execution_mode.unwrap_or_default()`（默认顺序）
- `status` 初始化为 `draft`
- 对 `step_timeout_seconds`、`step_max_retries` 做上限裁剪（7200 秒、8 次）

### 3.2 状态迁移原子前置条件
入口：`update_mission_status`（`service_mongo.rs:2035`）

允许来源状态（代码硬编码）：
- `planning` <- `draft|planned`
- `planned` <- `planning`
- `running` <- `draft|planned|paused|failed`
- `paused` <- `running|planning`
- `completed` <- `running`
- `failed` <- `running|planning|paused|planned`
- `cancelled` <- `draft|planned|running|paused|planning`

额外行为：
- 进入 `running|planning` 时会清空 `completed_at`
- 进入终态 `completed|failed|cancelled` 时写入 `completed_at`
- `running` 且首次时写入 `started_at`
- 尝试写入 `server_instance_id`（用于孤儿恢复标记）

### 3.3 计划、步骤、目标树持久化
- 保存计划：`save_mission_plan`（`service_mongo.rs:2283`）
- 重规划替换后半段步骤：`replan_remaining_steps`（`service_mongo.rs:2553`）
- 保存目标树：`save_goal_tree`（同文件 Goal Tree 区段）
- 推进当前目标：`advance_mission_goal`（`service_mongo.rs:2863`）
- pivot/abandon 原子计数：`pivot_goal_atomic`（`service_mongo.rs:2882`）、`abandon_goal_atomic`（`service_mongo.rs:2912`）

### 3.4 启动恢复
`recover_orphaned_missions`（`service_mongo.rs:2993`）
- 会把 `running|planning` 任务批量改为 `failed`
- 参数 `_instance_id` 当前未参与 filter

---

## 4. API 层（mission_routes）

路由装配：`mission_router`（`mission_routes.rs:81`），挂载路径 `/api/team/agent/mission`。

### 4.1 生命周期端点
- 创建：`create_mission`（`mission_routes.rs:125`）
- 启动：`start_mission`（`mission_routes.rs:244`）
- 暂停：`pause_mission`（`mission_routes.rs:301`）
- 恢复：`resume_mission_handler`（`mission_routes.rs:350`）
- 取消：`cancel_mission`（`mission_routes.rs:420`）

权限规则（关键）：
- 创建/查询：团队成员即可
- 启动/暂停/恢复/取消：创建者或团队管理员
- 步骤审批、目标审批：管理员（admin）

### 4.2 步骤审批流（顺序模式）
- 通过：`approve_step`（`mission_routes.rs:465`）
- 拒绝：`reject_step`
- 跳过：`skip_step`

`approve_step` 成功后会注册 mission 运行句柄并 `spawn resume_mission`。

### 4.3 目标审批流（自适应模式）
- 通过：`approve_goal`（`mission_routes.rs:640`）
- 拒绝：`reject_goal`
- 转向：`pivot_goal`（`mission_routes.rs:784`）
- 放弃：`abandon_goal_handler`

`approve_goal` 额外调用 `advance_mission_goal`，避免恢复后立即再次 pause。

### 4.4 SSE 流
`stream_mission`（`mission_routes.rs:919`）
- 支持 `last_event_id`（query/header）续流
- 若任务非 live（draft/planned/paused/completed/failed/cancelled），直接返回 one-shot `done`
- 若任务宣称 live 但 manager 不存在，返回 `mission_stream_unavailable` 状态事件
- 长连接寿命通过 `TEAM_SSE_MAX_LIFETIME_SECS` 控制，默认 2 小时

### 4.5 from-chat 默认模式
`create_from_chat`（`mission_routes.rs:1187`）构造请求时 `execution_mode: None`，因此走后端默认 `sequential`。

---

## 5. 顺序模式（sequential）执行逻辑

核心执行器：`MissionExecutor`（`mission_executor.rs`）

### 5.1 模式分流
`execute_mission_inner`（`mission_executor.rs:171`）
- `if mission.execution_mode == Adaptive` 则委托 `AdaptiveExecutor`
- 否则执行顺序模式

### 5.2 启动主流程
1. 校验状态必须 `draft|planned`
2. 创建 mission 独立 workspace（`runtime::create_workspace_dir`）
3. 创建 mission 绑定 session
4. 置状态 `planning`，广播 `mission_planning`
5. 调用 LLM 生成步骤计划（`generate_plan`，`mission_executor.rs:1321`）
6. 保存 `steps`
7. 置状态 `running`
8. 进入 `execute_steps`（`mission_executor.rs:346`）

### 5.3 计划生成与解析
- Prompt 要求 2-8 步，支持字段：`max_retries`、`timeout_seconds`、`required_artifacts`、`completion_checks`、`use_subagent`
- JSON 解析入口：`parse_steps_json`（`mission_executor.rs:1436`）
- 标准化逻辑：
  - 重试次数上限 8
  - 超时上限 7200 秒
  - `required_artifacts` 路径清洗
  - `completion_checks` 数量与长度裁剪

### 5.4 步骤执行循环
`execute_steps`（`mission_executor.rs:346`）
- 每步前：检查 cancel 与 token budget
- 审批策略判断：
  - `manual`：每步都 pause 等审批
  - `checkpoint`：仅 checkpoint step pause
  - `auto`：不 pause
- pause 时写 `StepStatus::AwaitingApproval` + `MissionStatus::Paused` 并广播

### 5.5 单步执行与可靠性
`run_single_step`（`mission_executor.rs:599`）
- 置 step 为 running，推进 current_step
- 通过 bridge 模式执行（`runtime::execute_via_bridge`）
- 支持：
  - 超时重试
  - 可重试错误判定
  - 指数退避
  - retry playbook（注入最近工具调用与上一轮输出）

失败收口：`finalize_step_failure`（`mission_executor.rs:1206`）
- 记 token
- step 置 failed
- mission 置 failed
- 保存 mission error_message

### 5.6 完成校验（重要）
`validate_step_completion`（`mission_executor.rs:1149`）
- assistant summary 不能为空
- 若输出明显“只在计划”且无成功工具调用，判失败
- `required_artifacts` 必须存在（且是安全相对路径）
- `completion_checks` shell 命令必须全部 exit 0

### 5.7 重规划（replan）
`evaluate_replan`（`mission_executor.rs:1801`）
- 仅 checkpoint 步后触发，最多 `MAX_REPLAN_COUNT=5`
- LLM 决策：`keep` 或 `replan`
- 若 replan 成功，替换剩余步骤并 `plan_version + 1`

### 5.8 收尾
- 最终总结：`synthesize_mission_summary`（`mission_executor.rs:1640`）
- 状态置 `completed`
- 对外 Done 事件在外层 wrapper 统一发送

### 5.9 恢复
`resume_mission`（`mission_executor.rs:1941`）
- 若 adaptive 直接委托 adaptive resume
- 顺序模式走 `resume_mission_sequential`（`mission_executor.rs:2066`）
- failed 恢复会重置 `failed|running` 步骤为 pending，并清 error
- `resume_feedback` 作为 operator guidance 注入后续 prompt

---

## 6. 自适应模式（adaptive / AGE）执行逻辑

核心执行器：`AdaptiveExecutor`（`adaptive_executor.rs`）

### 6.1 启动与阶段划分
- 入口：`execute_adaptive`（`adaptive_executor.rs:69`）
- 分两阶段：
  - `run_planning_phase`（`adaptive_executor.rs:164`）
  - `run_execution_phase`（`adaptive_executor.rs:259`）

### 6.2 规划阶段
- 创建 session、置 mission 为 planning
- 通过 LLM 分解 goal tree（`decompose_goal`）
- 保存 `goal_tree`
- 若审批策略不是 auto：
  - 状态置 `planned`
  - 广播 `mission_planned`（mode=adaptive）
  - 返回，等待用户“确认执行”

### 6.3 目标循环
`execute_goal_loop`（`adaptive_executor.rs:434`）
- 每轮重载 mission（含 goal_tree）
- `find_next_goal` 选取规则：叶子优先（depth 高优先），再按 order
- 审批策略：
  - `manual`：所有 goal 先 pause
  - `checkpoint`：checkpoint goal pause
  - `auto`：不 pause
- 执行单个 goal：`run_single_goal`（`adaptive_executor.rs:733`）

### 6.4 进展评估与分支
- `evaluate_goal` 返回 `advancing|stalled|blocked`
- `advancing` -> `complete_goal`
- `stalled|blocked` -> `handle_pivot` 或重置 pending 重试

### 6.5 Pivot 协议
`pivot_protocol`（`adaptive_executor.rs:1389`）
强制 abandon 条件：
- 探索预算耗尽
- 单 goal pivot 次数超 `MAX_PIVOTS_PER_GOAL=3`
- 任务总 pivot 超 `MAX_TOTAL_PIVOTS=15`

否则让模型决策：`retry` 或 `abandon`。
执行动作：
- `pivot_goal_atomic`（计数 `total_pivots`）
- `abandon_goal_atomic`（计数 `total_abandoned`）

### 6.6 结果汇总与恢复
- 汇总：`synthesize_results`（`adaptive_executor.rs:1519`）
- 置 mission completed
- 恢复入口：`resume_adaptive`（`adaptive_executor.rs:1603`）
- failed 恢复会重置 `failed|running` goals 为 pending，并清 error

---

## 7. 执行桥接与上下文注入

### 7.1 Bridge 机制
`runtime::execute_via_bridge`（`runtime.rs:115`）
核心流程：
1. 从 session 拿 team/user
2. 创建临时 task（`create_temp_task`）
3. 自动 approve
4. 内部 `TaskManager` 注册
5. 桥接事件到 `MissionManager`
6. 调 `TaskExecutor::execute_task`
7. 收桥接、清理临时 task

### 7.2 Mission 上下文注入系统提示词
`executor_mongo.rs`：
- `MissionPromptContext`（`executor_mongo.rs:207`）
- `build_system_prompt`（`executor_mongo.rs:238`）
- 读取 task.content.mission_context（`executor_mongo.rs:1339`）

注入内容包括：goal、approval_policy、current_step、total_steps。

---

## 8. SSE 事件模型与容错

### 8.1 事件类型
定义：`StreamEvent`（`task_manager.rs:15`）
Mission 特有：
- `goal_start`
- `goal_complete`
- `pivot`
- `goal_abandoned`

### 8.2 事件缓存与续流
`MissionManager`（`mission_manager.rs`）
- 历史缓存上限 `EVENT_HISTORY_LIMIT=400`（`mission_manager.rs:14`）
- `subscribe_with_history` 支持 `after_id`
- `signal_cancel` 只发 cancel token，不立刻移除条目

### 8.3 主进程后台修复任务
`main.rs`：
- 启动时 orphaned mission 恢复（`main.rs:519`）
- 周期性清理 stale mission（`main.rs:539`）
- stale 后会尝试把 mission 写成 failed 并写 error message

---

## 9. 前端执行模式链路

### 9.1 API 类型与调用
`mission.ts`：
- `ExecutionMode = 'sequential' | 'adaptive'`（`mission.ts:24`）
- `createMission` 支持 `execution_mode?`（`mission.ts:155`）
- `resumeMission` 支持反馈参数（`mission.ts:211`）
- `streamMission` 支持 `last_event_id`（`mission.ts:296`）

### 9.2 创建任务
`CreateMissionDialog.tsx`：
- 默认 `executionMode='sequential'`（`CreateMissionDialog.tsx:35`）
- 提交时传 `execution_mode`（`CreateMissionDialog.tsx:64`）

### 9.3 列表与详情显示
- 卡片按 mode 分支进度：`MissionCard.tsx:62`
- 详情页 mode 分支：
  - adaptive 显示 GoalTree
  - sequential 显示 StepList/StepApproval
  - 见 `MissionDetailPage.tsx:514`、`MissionDetailPage.tsx:581`
- 团队页内嵌面板同样分支：`MissionsPanel.tsx:1096`

### 9.4 任务启动按钮语义
当状态是 `planned + adaptive` 时，按钮文案为“确认执行”，对应后端从计划阶段进入执行阶段。
- `MissionDetailPage.tsx:548`
- `MissionsPanel.tsx:1065`

### 9.5 文档快捷任务默认模式
`QuickTaskMenu.tsx` 创建任务不传 `execution_mode`（`QuickTaskMenu.tsx:39`），因此默认顺序模式。

---

## 10. 配置与环境变量（与任务模式直接相关）

### 10.1 Mission 执行相关
- `TEAM_SSE_MAX_LIFETIME_SECS`：SSE 生命周期上限（mission stream）
- `TEAM_MISSION_STALE_SECS`：stale mission 判定阈值（后台清理）
- `TEAM_MISSION_STEP_TIMEOUT_SECS`：默认 step/goal 超时
- `TEAM_MISSION_TIMEOUT_CANCEL_GRACE_SECS`：超时后取消宽限
- `TEAM_MISSION_TIMEOUT_RETRY_LIMIT`：超时可重试次数上限
- `TEAM_MISSION_PLANNING_TIMEOUT_SECS`：规划阶段超时
- `TEAM_MISSION_PLANNING_CANCEL_GRACE_SECS`：规划超时取消宽限
- `TEAM_MISSION_COMPLETION_CHECK_TIMEOUT_SECS`：completion check 命令超时

### 10.2 工作区
- `WORKSPACE_ROOT`：mission/session 工作目录根（README 文档有声明）

---

## 11. 关键业务结论

1. 任务模式核心分流点只有一个：`MissionExecutor` 中按 `execution_mode` 分派到顺序执行器或自适应执行器。
2. 顺序模式强调“步骤计划 + checkpoint 审批 + 可验证完成合同 + 可重规划”。
3. 自适应模式强调“目标树 + 进展信号 + pivot 协议 + 预算约束”。
4. 两种模式共享同一套 bridge 执行底座（TaskExecutor），因此工具调用、流式事件、会话持久化行为一致。
5. 前端已经完整支持 execution mode 的创建、展示、审批、SSE 反馈，但 from-chat 和 quick-task 路径默认仍是 sequential。

---

## 12. 风险与观察（基于当前代码事实）

1. `recover_orphaned_missions` 的 `_instance_id` 参数未参与过滤，当前实现是全局恢复 `running|planning`（`service_mongo.rs:2993`）。
2. `cancelled` 允许从 `paused` 迁移（`service_mongo.rs:2035` 处状态表），并发路径下需重点关注语义一致性。
3. `create_from_chat` 当前不支持传 execution_mode（固定走默认 sequential，`mission_routes.rs:1187`）。
4. Web Admin 的 `CreateMissionRequest` 类型未暴露 `step_timeout_seconds` 与 `step_max_retries`，但后端接口已支持。

---

## 13. 代码索引速查

后端主索引：
- 模型定义：`crates/agime-team-server/src/agent/mission_mongo.rs:17`
- 路由入口：`crates/agime-team-server/src/agent/mission_routes.rs:81`
- Service 入口：`crates/agime-team-server/src/agent/service_mongo.rs:1836`
- 顺序执行器入口：`crates/agime-team-server/src/agent/mission_executor.rs:90`
- 自适应执行器入口：`crates/agime-team-server/src/agent/adaptive_executor.rs:69`
- Bridge 入口：`crates/agime-team-server/src/agent/runtime.rs:115`
- MissionManager：`crates/agime-team-server/src/agent/mission_manager.rs:53`

前端主索引：
- API 类型：`crates/agime-team-server/web-admin/src/api/mission.ts:24`
- 创建弹窗：`crates/agime-team-server/web-admin/src/components/mission/CreateMissionDialog.tsx:35`
- 详情页分支：`crates/agime-team-server/web-admin/src/pages/MissionDetailPage.tsx:514`
- 团队内嵌面板分支：`crates/agime-team-server/web-admin/src/components/team/MissionsPanel.tsx:1096`

