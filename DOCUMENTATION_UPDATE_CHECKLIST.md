# 文档更新清单 - 数字分身功能

基于代码审查报告，以下文档需要更新以反映数字分身（Digital Avatar）的新功能。

---

## 📋 需要更新的文档清单

### 优先级 1 - 核心文档（必须更新）

#### 1. **WEB_ADMIN.md** - Web 管理前端文档
**现状**: 已有基础框架，但缺少数字分身相关内容

**需要添加/修改的章节**:
- [ ] 新增 **"数字分身系统"** 章节
  - 数字分身概念与架构
  - Avatar Instance 生命周期
  - Avatar Type 分类（Dedicated/Shared/Managed）
  - Avatar Governance 治理系统

- [ ] 新增 **"AvatarAgentManagerPage"** 页面文档
  - 路由: `/teams/:teamId/avatar-manager`
  - 功能: Avatar 创建、编辑、删除、发布
  - 组件结构: CreateAvatarDialog, CreateManagerAgentDialog, DigitalAvatarGuide

- [ ] 新增 **"DigitalAvatarSection"** 组件文档
  - 在 TeamDetailPage 中的 Tab 位置
  - 功能: Avatar 列表、实例管理、Governance 队列
  - 子组件: AvatarTypeBadge, AvatarWorkbench, GovernancePanel

- [ ] 新增 **"Avatar Governance UI"** 章节
  - Governance 事件处理
  - Automation Config 管理
  - Runtime Log 查看
  - Capability Gap 提案流程

**文件位置**: `E:\yw\agiatme\goose\docs\WEB_ADMIN.md`

---

#### 2. **TEAM_SERVER.md** - Team Server 后端文档
**现状**: 已有完整的后端架构，但缺少数字分身相关 API

**需要添加/修改的章节**:
- [ ] 新增 **"数字分身系统"** 章节
  - Avatar 数据模型
  - Avatar Instance 管理
  - Avatar Governance 系统
  - Avatar Type 分类与隔离

- [ ] 新增 **"Avatar API 路由"** 章节
  - `POST /api/team/avatar/avatars` - 创建 Avatar
  - `GET /api/team/avatar/avatars` - 列表 Avatar
  - `GET/PUT/DELETE /api/team/avatar/avatars/{id}` - Avatar CRUD
  - `POST /api/team/avatar/avatars/{id}/publish` - 发布 Avatar
  - `GET /api/team/avatar/avatars/{id}/governance` - Governance 状态
  - `POST /api/team/avatar/avatars/{id}/governance/queue` - Governance 队列操作

- [ ] 新增 **"Avatar Governance 系统"** 章节
  - Governance 事件类型
  - Automation Config 结构
  - Runtime Log 追踪
  - Capability Gap 检测与提案

- [ ] 新增 **"Avatar Type 隔离"** 章节
  - Dedicated Avatar 隔离机制
  - Service Agent 绑定
  - Portal 与 Avatar 关系
  - Agent Isolation 规则

**文件位置**: `E:\yw\agiatme\goose\docs\TEAM_SERVER.md`

---

#### 3. **TEAM_SYSTEM.md** - 团队系统文档
**现状**: 已有团队协作框架，需要补充数字分身概念

**需要添加/修改的章节**:
- [ ] 新增 **"数字分身（Digital Avatar）"** 章节
  - 定义与用途
  - Avatar 与 Agent 的关系
  - Avatar 与 Portal 的关系
  - Avatar Governance 治理模型

- [ ] 修改 **"核心概念"** 图表
  - 添加 Avatar 到资源类型
  - 显示 Avatar → Agent → Portal 的关系链

- [ ] 新增 **"Avatar 数据模型"** 章节
  ```rust
  pub struct AvatarInstance {
      pub avatar_id: String,
      pub team_id: String,
      pub name: String,
      pub avatar_type: AvatarType,  // Dedicated | Shared | Managed
      pub manager_agent_id: Option<String>,
      pub service_agent_id: Option<String>,
      pub governance_state: AvatarGovernanceState,
      pub created_at: DateTime,
  }
  ```

- [ ] 新增 **"Avatar Governance 系统"** 章节
  - Governance 事件队列
  - Automation Config
  - Runtime Log
  - Capability Gap 检测

**文件位置**: `E:\yw\agiatme\goose\docs\TEAM_SYSTEM.md`

---

### 优先级 2 - 相关文档（应该更新）

#### 4. **ARCHITECTURE.md** - 系统架构文档
**现状**: 已有整体架构，需要补充数字分身层

**需要添加/修改的章节**:
- [ ] 修改 **"agime-team-server 内部架构"** 图表
  - 添加 Avatar 管理层
  - 显示 Avatar Governance 执行流程

- [ ] 新增 **"Avatar 执行流程"** 章节
  - Avatar 创建流程
  - Avatar 发布流程
  - Governance 事件处理流程

**文件位置**: `E:\yw\agiatme\goose\docs\ARCHITECTURE.md`

---

#### 5. **API_REFERENCE.md** - API 参考文档
**现状**: 需要补充 Avatar 相关 API

**需要添加/修改的章节**:
- [ ] 新增 **"Avatar API"** 部分
  - Avatar CRUD 端点
  - Avatar Governance 端点
  - Avatar 发布端点
  - 请求/响应示例

**文件位置**: `E:\yw\agiatme\goose\docs\API_REFERENCE.md`

---

#### 6. **DATABASE_SCHEMA.md** - 数据库架构文档
**现状**: 需要补充 Avatar 相关表/集合

**需要添加/修改的章节**:
- [ ] 新增 **"Avatar Collections (MongoDB)"** 章节
  - `avatars` - Avatar 实例
  - `avatar_governance_states` - Governance 状态
  - `avatar_governance_events` - Governance 事件队列
  - `avatar_runtime_logs` - 运行时日志

- [ ] 新增 **"Avatar Tables (SQLite)"** 章节
  - 对应的 SQLite 表结构

**文件位置**: `E:\yw\agiatme\goose\docs\DATABASE_SCHEMA.md`

---

#### 7. **USER_GUIDE.md** - 用户指南
**现状**: 需要补充数字分身使用说明

**需要添加/修改的章节**:
- [ ] 新增 **"数字分身使用指南"** 章节
  - 如何创建 Avatar
  - 如何配置 Avatar
  - 如何发布 Avatar
  - Governance 系统使用

**文件位置**: `E:\yw\agiatme\goose\docs\USER_GUIDE.md`

---

### 优先级 3 - 参考文档（可选更新）

#### 8. **README.md** - 项目主文档
**现状**: 可添加数字分身功能亮点

**需要添加/修改的章节**:
- [ ] 在功能列表中添加数字分身
- [ ] 在架构图中体现数字分身

**文件位置**: `E:\yw\agiatme\goose\docs\README.md`

---

#### 9. **CHANGELOG.md** - 变更日志
**现状**: 需要记录数字分身功能发布

**需要添加/修改的章节**:
- [ ] 新增版本条目，记录数字分身功能

**文件位置**: `E:\yw\agiatme\goose\docs\CHANGELOG.md`

---

## 📊 优先级排序与工作量估计

| 优先级 | 文档 | 工作量 | 关键性 |
|--------|------|--------|--------|
| 1 | WEB_ADMIN.md | 中 | 🔴 必须 |
| 1 | TEAM_SERVER.md | 大 | 🔴 必须 |
| 1 | TEAM_SYSTEM.md | 中 | 🔴 必须 |
| 2 | ARCHITECTURE.md | 小 | 🟡 重要 |
| 2 | API_REFERENCE.md | 中 | 🟡 重要 |
| 2 | DATABASE_SCHEMA.md | 中 | 🟡 重要 |
| 2 | USER_GUIDE.md | 中 | 🟡 重要 |
| 3 | README.md | 小 | 🟢 可选 |
| 3 | CHANGELOG.md | 小 | 🟢 可选 |

---

## 🔑 关键概念需要在文档中说明

### 1. Avatar Type 分类
- **Dedicated Avatar**: 专属数字分身，独立的 Manager Agent 和 Service Agent
- **Shared Avatar**: 共享数字分身，多个 Portal 共用
- **Managed Avatar**: 托管数字分身，由系统自动管理

### 2. Avatar Governance 系统
- **Governance State**: 追踪 Avatar 的治理状态
- **Governance Events**: 记录 Avatar 的治理事件
- **Automation Config**: 自动化治理配置
- **Runtime Log**: 运行时日志记录
- **Capability Gap**: 能力缺口检测与提案

### 3. Avatar 与其他组件的关系
```
Avatar Instance
├── Manager Agent (管理代理)
├── Service Agent (服务代理)
├── Portal (门户)
│   ├── Coding Agent (编码代理)
│   └── Service Agent (服务代理)
└── Governance State (治理状态)
    ├── Automation Config
    ├── Runtime Log
    └── Capability Gap Proposals
```

### 4. Avatar 生命周期
1. **创建** - 创建 Avatar 实例
2. **配置** - 配置 Manager/Service Agent
3. **发布** - 发布到 Portal
4. **运行** - 处理用户请求
5. **治理** - 监控与优化
6. **归档** - 归档或删除

---

## 📝 新增文件建议

考虑创建以下新文档以补充现有文档：

- [ ] **DIGITAL_AVATAR_GUIDE.md** - 数字分身完整指南
  - 概念介绍
  - 创建与配置
  - Governance 系统
  - 最佳实践
  - 故障排查

- [ ] **AVATAR_GOVERNANCE_SYSTEM.md** - Avatar Governance 详细文档
  - Governance 架构
  - 事件类型
  - Automation Config
  - Runtime Log 分析

---

## ✅ 验收标准

文档更新完成后应满足以下条件：

1. ✓ 所有优先级 1 文档已更新
2. ✓ 新增内容与代码实现保持一致
3. ✓ 包含清晰的概念图表和数据模型
4. ✓ 提供 API 端点完整列表
5. ✓ 包含使用示例和最佳实践
6. ✓ 所有链接和交叉引用正确
7. ✓ 中英文内容一致（如适用）

---

## 📌 后续步骤

1. **第一阶段**: 更新优先级 1 文档（WEB_ADMIN.md, TEAM_SERVER.md, TEAM_SYSTEM.md）
2. **第二阶段**: 更新优先级 2 文档（ARCHITECTURE.md, API_REFERENCE.md, DATABASE_SCHEMA.md, USER_GUIDE.md）
3. **第三阶段**: 更新优先级 3 文档（README.md, CHANGELOG.md）
4. **第四阶段**: 创建新增文档（DIGITAL_AVATAR_GUIDE.md, AVATAR_GOVERNANCE_SYSTEM.md）
5. **验证阶段**: 审查所有更新，确保一致性和完整性

---

**生成时间**: 2026-03-10
**状态**: 待执行
