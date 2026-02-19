# 稳定性修复审查报告

> 审查日期: 2026-02-18
> 审查范围: 19 项长程稳定性修复
> 审查方法: 6 个 Opus agent 并行审查 + 人工交叉验证
> 审查维度: 数据库/进程生命周期/并发锁/流式连接/Mission生命周期/架构全局

---

## 一、审查结论摘要

| 类别 | 数量 |
|------|------|
| 修复正确且合理 | 10 |
| 存在问题需要修正 | 6 |
| 参数需要调整 | 3 |
| 总计 | 19 |

**发现的新问题**: 7 个（含 1 个 CRITICAL, 2 个 HIGH, 4 个 MEDIUM）

---

## 二、发现的问题（需要修正）

### 问题 1: CRITICAL — pause 路由操作顺序错误导致竞态修复失效

**文件**: `crates/agime-team-server/src/agent/mission_routes.rs:280-284`

**现状**:
```rust
// pause_mission 路由
mission_manager.cancel(&mission_id).await;  // 1. 先触发 cancel token
service.update_mission_status(&mission_id, &MissionStatus::Paused).await?;  // 2. 后写 DB
```

**问题**: Fix #16（pause/cancel 竞态修复）假设 pause 路由先写 DB 再触发 cancel token，但实际顺序相反。当 executor 检查 DB 状态时，Paused 可能还没写入，导致仍然被设为 Cancelled。

**修复建议**: 交换顺序 — 先写 DB 状态为 Paused，再触发 cancel token。

---

### 问题 2: LOW — MCP Drop impl 在 runtime 关闭期间可能泄漏

**文件**: `crates/agime-team-server/src/agent/mcp_connector.rs:653-677`

**现状**: Drop impl 使用 `handle.spawn` fire-and-forget 方式清理连接；无 runtime 时仅 warn。

**问题**: `handle.spawn` 在 runtime 正在关闭时 spawn 的 future 可能被取消，导致子进程泄漏。

**注意**: 原报告提到"双重关闭风险"，经代码核实不成立 — 显式 `shutdown()` 先 `std::mem::take` 清空连接（`mcp_connector.rs:642`），`executor_mongo.rs:1394` 先 `state.mcp.take()` 取走 McpConnector，Drop 时 `connections.is_empty()` 直接 return（`mcp_connector.rs:656-658`）。双重关闭在当前实现中不会发生。

**严重程度**: LOW（仅在异常退出路径 + runtime 关闭时才可能触发）

**修复建议**: 作为安全网可接受，可后续在 warn 分支中尝试同步 kill 子进程。

---

### 问题 3: MEDIUM — TTL 索引与已有索引冲突风险

**文件**: `crates/agime-team/src/db.rs:213-236`

**现状**: 对 `audit_logs.created_at` 创建 TTL 索引（180天）。

**问题**: MongoDB 的 `createIndex` 如果同一字段已存在不同配置的索引（例如之前手动创建了非 TTL 的 `created_at` 索引），会返回错误而非静默更新。虽然 `create_indexes` 方法可能已处理此情况，但如果现有部署中已有冲突索引，升级时会失败。

**严重程度**: MEDIUM（仅影响已有部署升级）

**修复建议**: 在创建 TTL 索引前，检查是否已存在同字段的非 TTL 索引，如有则先删除。或在文档中注明升级注意事项。

---

### 问题 4: MEDIUM — recover_orphaned_missions 在滚动部署时误杀正常 mission

**文件**: `crates/agime-team-server/src/agent/service_mongo.rs` (recover_orphaned_missions)

**现状**:
```rust
pub async fn recover_orphaned_missions(&self) -> Result<u64, mongodb::error::Error> {
    self.missions().update_many(
        doc! { "status": { "$in": ["running", "planning"] } },
        doc! { "$set": { "status": "failed", "error": "Server restarted..." } },
        None,
    ).await
}
```

**问题**: 在多实例部署或滚动更新场景下，一个实例重启时会把其他实例正在执行的 mission 也标记为 failed。filter 没有区分"哪个实例在执行"。

**严重程度**: MEDIUM（单实例部署无影响，多实例部署有风险）

**修复建议**: 添加 `server_instance_id` 字段到 mission 文档，recovery 时只重置本实例的 mission。或添加 `last_heartbeat` 字段，只重置心跳超时的 mission。

---

### 问题 5: HIGH — SSE 超时 "done" 事件阻止前端重连

**文件**: `crates/agime-team-server/src/agent/chat_routes.rs:606-613`, `portal_public.rs:1023-1029`, `mission_routes.rs:710-716`, `streamer.rs:73-83`

**现状**: 当 30 分钟 SSE deadline 触发时，后端发送 `event("done").data({"type":"Done","status":"timeout","error":"Stream exceeded maximum connection lifetime"})`。

**问题**: 前端 `ChatConversation.tsx:614-647` 的 "done" 事件处理器检查 `data?.error`，如果为真则：
1. 将错误信息显示为消息内容
2. 设置 `isProcessingRef.current = false`
3. 不触发重连（`onerror` 处理器在 line 653 检查 `isProcessingRef.current`）

结果：后端任务继续运行，但用户看到错误且 SSE 流断开无法自动重连。

**严重程度**: HIGH（用户在长任务中会丢失实时反馈且无法恢复）

**修复建议**: 超时时不发送 "done" 事件，直接关闭流。前端已有的 `onerror` 处理器会触发带 `last_event_id` 的自动重连逻辑。

---

### 问题 6: MEDIUM — Fix #18 写锁在异步 shutdown 期间未释放

**文件**: `crates/agime-team-server/src/agent/executor_mongo.rs:1392-1397`

**现状**:
```rust
{
    let mut state = dynamic_state.write().await;  // 获取写锁
    if let Some(m) = state.mcp.take() {
        m.shutdown().await;  // 写锁仍然持有！
    }
}  // 写锁在这里才释放
```

**问题**: `m.shutdown().await` 涉及网络 I/O（对每个 MCP 连接调用 `client.cancel().await`），写锁在整个过程中持有。如果某个 MCP server 响应慢，会阻塞所有其他访问 `dynamic_state` 的代码。

**严重程度**: MEDIUM（实际场景中此代码在 session 结束后运行，竞争概率低）

**修复建议**: 先 take 再释放锁，然后 shutdown：
```rust
let mcp = {
    let mut state = dynamic_state.write().await;
    state.mcp.take()
};
if let Some(m) = mcp {
    m.shutdown().await;
}
```

---

### 问题 7: MEDIUM — update_mission_status 原子前置条件允许从 Paused 转为 Cancelled

**文件**: `crates/agime-team-server/src/agent/service_mongo.rs:1963`

**现状**:
```rust
MissionStatus::Cancelled => vec!["draft", "planned", "running", "paused", "planning"],
```

**问题**: `"paused"` 在允许列表中意味着 executor 的 TOCTOU 检查（先读 DB 再写 Cancelled）可被绕过 — 即使 executor 读到 Paused 并跳过写入，在读和写之间 pause 路由可能刚设完 Paused，而另一个并发路径仍可写入 Cancelled。

**严重程度**: MEDIUM（与问题 1 的 pause 路由顺序修复配合后风险降低，但防御不完整）

**修复建议**: 从 Cancelled 的 `allowed_from` 列表中移除 `"paused"`。cancel_mission 路由需要先将状态改为其他值再取消，或为路由发起的取消和 executor 发起的取消使用不同的前置条件。

---

## 三、参数合理性审查

### 3.1 需要调整的参数

| 参数 | 当前值 | 问题 | 建议 |
|------|--------|------|------|
| SSE deadline | 30 分钟 | Mission 执行可能持续数小时，30分钟会断开活跃连接 | 改为 2-4 小时，或改为可配置 |
| Provider stream chunk timeout | 5 分钟 | Extended thinking 模式下 LLM 可能 3-5 分钟无输出 | 改为 10 分钟更安全 |
| Shell hard timeout | 30 分钟 | 某些构建任务（如大型 Rust 项目编译）可能超过 30 分钟 | 改为可配置，默认 60 分钟 |

### 3.2 参数合理的修复

| 参数 | 当前值 | 评估 |
|------|--------|------|
| TTL: audit_logs | 180 天 | ✅ 合理，审计日志保留半年符合常规 |
| TTL: auth_audit_logs | 90 天 | ✅ 合理 |
| TTL: portal_interactions | 90 天 | ✅ 合理 |
| Broadcast buffer | 512 | ✅ 合理，每个 StreamEvent 约 200-500 bytes，512 × 500B ≈ 250KB |
| Task cleanup interval | 5 分钟 / 2 小时 max age | ✅ 合理 |
| Session cleanup interval | 10 分钟 | ✅ 合理，作为 MongoDB TTL 的安全网 |
| Mission cleanup interval | 2 分钟 / 1 小时 max age | ✅ 合理 |
| Tool call timeout | 默认 5 分钟（可通过 `TEAM_AGENT_TOOL_TIMEOUT_SECS` 或 session 参数覆盖） | ✅ 合理，已可配置 |

### 3.3 缺少环境变量配置的参数

以下硬编码参数建议改为可通过环境变量配置：

1. **SSE deadline** (30min) → `TEAM_SSE_MAX_LIFETIME_SECS`
2. **Provider stream chunk timeout** (5min) → `TEAM_PROVIDER_CHUNK_TIMEOUT_SECS`
3. **Shell hard timeout** (30min) → 已有 `TEAM_SHELL_TIMEOUT_SECS`? 需确认

---

## 四、修复正确性逐项审查

### Fix #1-3: TTL 索引 ✅ 正确

- TTL 字段 `created_at` 与文档结构一致
- `expire_after` 使用正确的秒数计算
- 索引创建是幂等的（MongoDB 对相同配置的重复创建返回 OK）
- **注意**: 见问题 3 关于冲突索引的风险

### Fix #4: Graceful Shutdown ✅ 正确

- `shutdown_signal()` 正确处理 Ctrl+C 和 SIGTERM（Unix）
- `with_graceful_shutdown` 正确传递给 axum::serve
- 跨平台处理正确（`#[cfg(unix)]` / `#[cfg(not(unix))]`）

### Fix #5: MCP Drop impl ⚠️ 部分正确

- Drop 中使用 `std::mem::take` 避免 double-free ✅
- `handle.spawn` 是非阻塞的 ✅
- 但 runtime 关闭时 spawn 的 future 可能被取消 ⚠️（见问题 2）

### Fix #6: TaskManager cancel write lock ✅ 正确

- 原代码确实使用 `write()` 但没有 `remove()`，只是 cancel token
- 修复后 `cancel()` 同时 remove 条目，防止内存泄漏
- 这是一个真实的内存泄漏修复，不是过度改造

### Fix #7: cleanup_stale 激活 ✅ 正确

- 5 分钟间隔、2 小时 max age 合理
- 与 ChatManager 的清理模式一致

### Fix #8: Tool call timeout ✅ 正确

- 5 分钟超时对 MCP tool call 合理
- 使用 `tokio::time::timeout` 包装正确

### Fix #9: Shell hard timeout ⚠️ 参数偏保守

- 30 分钟对大多数 shell 命令足够
- 但大型编译任务可能超时
- 建议改为可配置

### Fix #10-11: Broadcast buffer 512 ✅ 正确

- 100 确实可能在高频事件场景下导致 `RecvError::Lagged`
- 512 是合理的折中（内存开销约 250KB per channel）
- 不是过度改造

### Fix #12: SSE deadline 30 分钟 ⚠️ 参数偏保守

- 对普通 chat 会话 30 分钟合理
- 但 mission 执行可能持续数小时
- 前端是否有自动重连逻辑？如果没有，30 分钟会中断长任务的实时反馈
- 建议改为 2 小时或可配置

### Fix #13-14: Chat/Mission SSE deadline ⚠️ 同上

### Fix #15: Session cleanup ✅ 正确

- 作为 MongoDB TTL 的安全网，10 分钟间隔合理
- 错误处理正确（warn 级别，不会 panic）

### Fix #16: Pause/Cancel 竞态修复 ❌ 修复不完整

- 修复逻辑本身正确（检查 DB 状态再决定是否设 Cancelled）
- **但 pause 路由的操作顺序错误**（见问题 1），导致修复在实际场景中可能失效
- 需要同时修复 pause 路由的操作顺序

### Fix #17: Orphaned mission recovery ⚠️ 单实例正确，多实例有风险

- 单实例部署完全正确
- 多实例部署会误杀其他实例的 mission（见问题 4）
- `tokio::spawn` 用于启动恢复是合理的（非阻塞）

### Fix #18: MCP 连接泄漏修复 ⚠️ 逻辑正确，锁持有范围偏大

- `write().await + take()` 比 `Arc::try_unwrap` 更可靠 ✅
- `take()` 后 state.mcp 为 None，后续访问安全 ✅
- 与 Drop impl 不冲突（take 后 connections 为空，Drop 是 no-op）✅
- **写锁在 shutdown I/O 期间未释放**（见问题 6），建议重构为先 take 再释放锁

### Fix #19: Provider stream inactivity timeout ⚠️ 参数偏保守

- `tokio::time::timeout` 嵌套在 `tokio::select!` 中语法正确
- 5 分钟对普通 streaming 合理
- 但 extended thinking 模式下 LLM 可能长时间无输出
- 建议改为 10 分钟

---

## 五、过度改造评估

| 修复 | 是否过度改造 | 说明 |
|------|-------------|------|
| TTL 索引 | 否 | 无 TTL 的集合确实会无限增长 |
| Graceful shutdown | 否 | 基础设施必备 |
| MCP Drop | 否 | 子进程泄漏是真实问题 |
| TaskManager cancel | 否 | 内存泄漏是真实问题 |
| cleanup_stale 激活 | 否 | 死代码激活，不是新增复杂度 |
| Broadcast buffer | 否 | 简单参数调整 |
| SSE deadline | 否 | 连接无限存活是真实风险 |
| Session cleanup | 轻微 | MongoDB TTL 已覆盖，但作为安全网可接受 |
| Pause/cancel 竞态 | 否 | 竞态条件是真实 bug |
| Orphaned recovery | 否 | 服务器重启后 mission 卡死是真实问题 |
| MCP leak fix | 否 | Arc::try_unwrap 失败是真实泄漏 |
| Stream timeout | 否 | 无限等待是真实风险 |

**结论**: 没有过度改造的修复。Session cleanup（Fix #15）与 MongoDB TTL 有轻微重叠，但作为防御性编程可以接受。

---

## 六、部署风险评估

| 风险 | 严重程度 | 说明 |
|------|---------|------|
| TTL 索引冲突 | MEDIUM | 如果已有同字段非 TTL 索引，创建会失败 |
| Orphaned recovery 误杀 | MEDIUM | 多实例部署时会影响其他实例的 mission |
| SSE 断连 | LOW | 30 分钟后长任务的 SSE 连接会断开 |
| Shell timeout | LOW | 超长编译任务可能被 kill |

---

## 七、行动建议（按优先级排序）

### 必须修复（在合并前） — 全部已完成 ✅

1. ✅ **修复 pause 路由操作顺序** — 交换 `mission_routes.rs` 的两行代码顺序（先写 DB 再 cancel token）
2. ✅ **修复 SSE 超时事件** — 4 个文件（`chat_routes.rs`, `portal_public.rs`, `mission_routes.rs`, `streamer.rs`）超时时不发送 "done" 事件，直接 break
3. ✅ **修复 Fix #18 写锁范围** — `executor_mongo.rs` 先 take 释放锁再 shutdown
4. ✅ **调整 SSE deadline** — 4 个文件改为可配置（`TEAM_SSE_MAX_LIFETIME_SECS`，默认 2 小时）
5. ✅ **调整 provider stream timeout** — 可配置（`TEAM_PROVIDER_CHUNK_TIMEOUT_SECS`，默认 10 分钟）

### 建议修复（可后续处理）

6. ✅ **executor cancel 返回 Ok** — 3 处 cancel 检查从 `Err` 改为 `Ok(())`，让外层 cleanup 读取实际 DB 状态
7. ✅ **shell timeout 可配置** — `TEAM_SHELL_TIMEOUT_SECS`，默认 60 分钟（`rmcp_developer.rs`）
8. ⏭️ **MCP Drop sync kill** — 跳过，McpConnection 不持有子进程 PID，需 rmcp 库层面支持
9. ✅ **recover_orphaned_missions 实例标识** — 启动时生成 UUID 实例 ID，missions 写入 `server_instance_id`，recovery 仅重置本实例或无标识的遗留任务
10. ✅ **cleanup_stale max_age 可配置** — `TEAM_MISSION_STALE_SECS`，默认 3 小时
11. 📝 **TTL 索引升级文档** — 待用户需要时补充

### 无需修改

- 其余 12 项修复经审查确认正确且合理，无需调整

---

## 八、审查团队

| Agent | 审查维度 | 状态 |
|-------|---------|------|
| db-reviewer | 数据库 TTL 索引、MongoDB 操作 | ✅ 完成 |
| process-reviewer | 进程生命周期、子进程管理 | ✅ 完成 |
| concurrency-reviewer | 并发锁、buffer sizing | ✅ 完成 |
| streaming-reviewer | SSE 流式连接、超时 | ✅ 完成 |
| mission-reviewer | Mission pause/cancel 竞态 | ✅ 完成 |
| architect-reviewer | 全局架构、过度改造、参数合理性 | ✅ 完成 |
