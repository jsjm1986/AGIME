# Team 功能架构迁移计划

## 一、目标

将 Team 功能从**互斥模式**（Cloud OR LAN）迁移到**统一多源架构**（Local + Cloud + LAN 并存），并清理旧代码，保持项目代码质量和优雅性。

## 二、当前问题

| 问题 | 说明 |
|------|------|
| 互斥模式 | Cloud 和 LAN 不能同时使用 |
| 代码重复 | `api.ts` 中大量重复的连接模式判断逻辑 |
| 分散存储 | `serverStore.ts` 和 `lanStore.ts` 独立管理 |
| 无离线支持 | 远程资源无法缓存 |
| 视图分离 | `CloudTeamView` 和 `LANTeamView` 逻辑高度重复 |

## 三、目标架构

```
┌────────────────────────────────────────────────────────────┐
│                    TeamView (统一入口)                      │
└─────────────────────────┬──────────────────────────────────┘
                          │
                          ▼
┌────────────────────────────────────────────────────────────┐
│                  UnifiedDashboard (新)                      │
│  - 显示所有数据源状态                                       │
│  - 聚合显示所有团队/资源                                    │
│  - 快速切换和管理数据源                                     │
└─────────────────────────┬──────────────────────────────────┘
                          │
                          ▼
┌────────────────────────────────────────────────────────────┐
│                   SourceManager (核心)                      │
│  ┌──────────┐  ┌──────────┐  ┌──────────┐  ┌──────────┐   │
│  │  Local   │  │ Cloud 1  │  │ Cloud 2  │  │  LAN 1   │   │
│  │ Adapter  │  │ Adapter  │  │ Adapter  │  │ Adapter  │   │
│  └──────────┘  └──────────┘  └──────────┘  └──────────┘   │
└────────────────────────────────────────────────────────────┘
```

## 四、迁移阶段

### 阶段 1: 清理与整合 (P0)

**目标**: 简化现有代码结构，为新架构做准备

#### 1.1 删除的文件

| 文件 | 原因 |
|------|------|
| `TeamModeSelector.tsx` | 不再需要模式选择，统一入口 |
| `CloudTeamView.tsx` | 被 UnifiedDashboard 替代 |
| `LANTeamView.tsx` | 被 UnifiedDashboard 替代 |
| `lan/LANTeamView.tsx` | 重复，已有 LANTeamView.tsx |

#### 1.2 保留并重构的文件

| 文件 | 修改内容 |
|------|----------|
| `api.ts` | 移除连接模式逻辑，改为接收 sourceId 参数 |
| `TeamView.tsx` | 简化为直接渲染 UnifiedDashboard |
| `servers/serverStore.ts` | 保留，由 SourceManager 调用 |
| `lan/lanStore.ts` | 保留，由 SourceManager 调用 |

#### 1.3 保留的通用组件

| 文件 | 说明 |
|------|------|
| `TeamList.tsx` | 团队列表（添加 sourceId 参数） |
| `TeamDetail.tsx` | 团队详情（添加 sourceId 参数） |
| `TeamHeader.tsx` | 团队头部 |
| `ResourceCard.tsx` | 资源卡片 |
| `ResourceDetailDialog.tsx` | 资源详情对话框 |
| `ResourceEditDialog.tsx` | 资源编辑对话框 |
| `ShareResourceDialog.tsx` | 分享资源对话框 |
| `invites/*` | 邀请系统组件 |
| `skill-package/*` | 技能包组件 |
| `servers/AddServerDialog.tsx` | 添加服务器对话框 |
| `servers/CloudServerCard.tsx` | 服务器卡片 |
| `lan/ConnectLANDialog.tsx` | 连接设备对话框 |
| `lan/LANDeviceCard.tsx` | 设备卡片 |
| `lan/LANShareSettings.tsx` | LAN 共享设置 |

### 阶段 2: 完善新架构 (P0)

**目标**: 完成 SourceManager 和适配器的完整实现

#### 2.1 增强 SourceManager

```typescript
// sources/sourceManager.ts 需要添加的功能

class SourceManager {
  // 已有
  initialize(): Promise<void>
  registerSource(source: DataSource): void
  unregisterSource(sourceId: string): boolean
  getAllSources(): DataSource[]
  checkHealth(sourceId: string): Promise<HealthStatus>
  aggregateTeams/Skills/Recipes/Extensions(...)

  // 需要添加
  migrateFromOldStorage(): void  // 从旧存储迁移数据
  getSourceConfig(sourceId: string): SourceConfig  // 获取连接配置
  updateSourceStatus(sourceId: string, status: DataSourceStatus): void
  syncSource(sourceId: string): Promise<SyncResult>  // 同步单个源
}
```

#### 2.2 增强适配器

每个适配器需要实现完整的 CRUD 操作：

```typescript
interface DataSourceAdapter {
  // 已有
  getSource(): DataSource
  isAvailable(): Promise<boolean>
  checkHealth(): Promise<HealthStatus>
  listTeams/Skills/Recipes/Extensions(...)
  getTeam/Skill/Recipe/Extension(...)

  // 需要添加
  createTeam(data: CreateTeamRequest): Promise<Team>
  updateTeam(teamId: string, data: UpdateTeamRequest): Promise<Team>
  deleteTeam(teamId: string): Promise<void>

  shareSkill(data: ShareSkillRequest): Promise<SharedSkill>
  installSkill(skillId: string): Promise<InstallResult>
  // ... 其他 CRUD 操作
}
```

#### 2.3 重构 api.ts

将 `api.ts` 改造为适配器的统一调用层：

```typescript
// api.ts 新结构

import { sourceManager } from './sources';

// 获取指定源的适配器
function getAdapter(sourceId?: string): DataSourceAdapter {
  if (sourceId) {
    return sourceManager.getAdapter(sourceId);
  }
  // 默认使用本地适配器
  return sourceManager.getAdapter('local');
}

// 所有 API 函数添加 sourceId 参数
export async function listTeams(sourceId?: string, params?: ListParams) {
  const adapter = getAdapter(sourceId);
  return adapter.listTeams(params);
}

export async function getTeam(teamId: string, sourceId?: string) {
  const adapter = getAdapter(sourceId);
  return adapter.getTeam(teamId);
}

// ... 其他函数类似
```

### 阶段 3: 统一视图层 (P1)

**目标**: 用新组件替换旧的分离视图

#### 3.1 重构 UnifiedDashboard

```tsx
// UnifiedDashboard.tsx 新结构

const UnifiedDashboard: React.FC = () => {
  const sourceManager = useSourceManager();
  const [sources, setSources] = useState<DataSource[]>([]);
  const [selectedSource, setSelectedSource] = useState<string | null>(null);
  const [selectedTeam, setSelectedTeam] = useState<TeamSummary | null>(null);

  // 视图状态
  type ViewState = 'dashboard' | 'teams' | 'team-detail' | 'add-source';
  const [viewState, setViewState] = useState<ViewState>('dashboard');

  return (
    <div>
      {viewState === 'dashboard' && (
        <DashboardView
          sources={sources}
          onSelectSource={handleSelectSource}
          onAddSource={() => setViewState('add-source')}
        />
      )}

      {viewState === 'teams' && selectedSource && (
        <TeamsView
          sourceId={selectedSource}
          onBack={() => setViewState('dashboard')}
          onSelectTeam={handleSelectTeam}
        />
      )}

      {viewState === 'team-detail' && selectedTeam && (
        <TeamDetail
          team={selectedTeam}
          sourceId={selectedSource}
          onBack={() => setViewState('teams')}
        />
      )}

      {viewState === 'add-source' && (
        <AddSourceDialog
          onClose={() => setViewState('dashboard')}
          onSuccess={handleAddSourceSuccess}
        />
      )}
    </div>
  );
};
```

#### 3.2 创建新的子组件

| 组件 | 功能 |
|------|------|
| `DashboardView.tsx` | 仪表板主视图，显示所有源和统计 |
| `SourceCard.tsx` | 数据源卡片（统一 Cloud/LAN） |
| `AddSourceDialog.tsx` | 添加数据源对话框（统一 Cloud/LAN） |
| `TeamsView.tsx` | 团队列表视图 |
| `ResourcesView.tsx` | 资源聚合视图 |

### 阶段 4: 存储迁移 (P1)

**目标**: 将旧存储格式迁移到新格式

#### 4.1 迁移函数

```typescript
// sources/migration.ts

export function migrateToUnifiedStorage(): void {
  // 检查是否已迁移
  if (localStorage.getItem('AGIME_TEAM_STORAGE_MIGRATED')) {
    return;
  }

  // 迁移云端服务器
  const servers = getServers(); // 从 serverStore
  for (const server of servers) {
    const dataSource: DataSource = {
      id: `cloud-${server.id}`,
      type: 'cloud',
      name: server.name,
      status: server.status === 'online' ? 'online' : 'offline',
      connection: {
        url: server.url,
        authType: 'api-key',
        credentialRef: `cloud-${server.id}`,
      },
      // ...
    };
    sourceManager.registerSource(dataSource);
    storeCredential(`cloud-${server.id}`, server.apiKey);
  }

  // 迁移 LAN 连接
  const connections = getConnections(); // 从 lanStore
  for (const conn of connections) {
    const dataSource: DataSource = {
      id: `lan-${conn.id}`,
      type: 'lan',
      name: conn.name,
      // ...
    };
    sourceManager.registerSource(dataSource);
    storeCredential(`lan-${conn.id}`, conn.secretKey);
  }

  // 标记迁移完成
  localStorage.setItem('AGIME_TEAM_STORAGE_MIGRATED', 'true');
}
```

#### 4.2 清理旧存储键

迁移完成后，可以安全删除：
- `AGIME_TEAM_CONNECTION_MODE`
- `AGIME_TEAM_LAN_SERVER_URL`
- `AGIME_TEAM_LAN_SECRET_KEY`
- `AGIME_TEAM_SERVER_URL` (legacy)
- `AGIME_TEAM_API_KEY` (legacy)

保留（由新系统继续使用）：
- `AGIME_TEAM_CLOUD_SERVERS` → 转换为 DataSource
- `AGIME_TEAM_LAN_CONNECTIONS` → 转换为 DataSource
- `AGIME_TEAM_RECENT_TEAMS` → 保留

### 阶段 5: 后端优化 (P1)

**目标**: 优化后端以支持新架构

#### 5.1 统一路由

```rust
// crates/agime-team/src/routes/unified.rs

// 添加更多聚合端点
pub fn routes(state: TeamState) -> Router {
    Router::new()
        .route("/unified/sources", get(list_sources))
        .route("/unified/teams", get(aggregate_teams))
        .route("/unified/skills", get(aggregate_skills))
        .route("/unified/recipes", get(aggregate_recipes))
        .route("/unified/extensions", get(aggregate_extensions))
        .route("/unified/cache/:resource_type/:resource_id", get(get_cached))
        .route("/unified/cache/:resource_type/:resource_id", post(cache_resource))
        .with_state(state)
}
```

#### 5.2 缓存管理

增强 `cache.rs`：

```rust
impl LocalCacheManager {
    // 已有
    pub async fn cache_resource(...) -> Result<()>
    pub async fn get_cached(...) -> Result<Option<CachedResource>>

    // 需要添加
    pub async fn list_cached(source_id: &str) -> Result<Vec<CachedResource>>
    pub async fn clear_expired() -> Result<u32>
    pub async fn clear_source(source_id: &str) -> Result<u32>
    pub async fn get_sync_status(source_id: &str) -> Result<SyncStatus>
}
```

## 五、文件变更清单

### 删除的文件 (8个)

```
ui/desktop/src/components/team/
├── TeamModeSelector.tsx          # 模式选择器，不再需要
├── CloudTeamView.tsx             # 云端视图，被统一视图替代
├── UnifiedDashboard.tsx          # 当前的混合入口，重写
├── ConnectionStatusBar.tsx       # 旧的连接状态栏
├── ServiceStatusIndicator.tsx    # 旧的服务状态指示器
├── servers/CloudServerList.tsx   # 被 SourceList 替代
└── lan/LANDeviceList.tsx         # 被 SourceList 替代
```

### 重构的文件 (10个)

```
ui/desktop/src/components/team/
├── api.ts                        # 移除模式逻辑，添加 sourceId 参数
├── types.ts                      # 添加新的聚合类型
├── TeamView.tsx                  # 简化为渲染 UnifiedDashboard
├── TeamList.tsx                  # 添加 sourceId 参数
├── TeamDetail.tsx                # 添加 sourceId 参数
├── sources/sourceManager.ts      # 完善实现
├── sources/adapters/localAdapter.ts   # 完善 CRUD
├── sources/adapters/cloudAdapter.ts   # 完善 CRUD
├── sources/adapters/lanAdapter.ts     # 完善 CRUD
└── auth/authAdapter.ts           # 完善凭据管理
```

### 新增的文件 (8个)

```
ui/desktop/src/components/team/
├── dashboard/
│   ├── DashboardView.tsx         # 仪表板主视图
│   ├── SourceCard.tsx            # 统一的数据源卡片
│   ├── SourceList.tsx            # 数据源列表
│   ├── AddSourceDialog.tsx       # 统一的添加数据源对话框
│   ├── StatsOverview.tsx         # 统计概览
│   └── index.ts
├── sources/
│   └── migration.ts              # 存储迁移工具
└── UnifiedDashboard.tsx          # 重写的统一仪表板
```

### 保留不变的文件 (20+个)

```
ui/desktop/src/components/team/
├── types.ts (部分修改)
├── index.ts
├── TeamHeader.tsx
├── TeamUIComponents.tsx
├── ResourceCard.tsx
├── ResourceDetailDialog.tsx
├── ResourceEditDialog.tsx
├── ShareResourceDialog.tsx
├── SyncStatusIndicator.tsx
├── UpdateNotification.tsx
├── Recommendations.tsx
├── recentTeamsStore.ts
├── invites/
│   ├── InviteMemberDialog.tsx
│   ├── InviteListDialog.tsx
│   ├── JoinTeamDialog.tsx
│   └── index.ts
├── skill-package/
│   ├── SkillPackageUploader.tsx
│   ├── FileTreeView.tsx
│   └── index.ts
├── servers/
│   ├── serverStore.ts (保留，由 SourceManager 使用)
│   ├── AddServerDialog.tsx (保留，AddSourceDialog 可复用)
│   ├── CloudServerCard.tsx (保留，SourceCard 可复用)
│   └── types.ts
└── lan/
    ├── lanStore.ts (保留，由 SourceManager 使用)
    ├── ConnectLANDialog.tsx (保留，AddSourceDialog 可复用)
    ├── LANDeviceCard.tsx (保留，SourceCard 可复用)
    ├── LANShareSettings.tsx
    └── types.ts
```

## 六、实施顺序

### Week 1: 基础设施

1. **Day 1-2**: 完善 `sourceManager.ts`
   - 添加 `migrateFromOldStorage()`
   - 添加 `getSourceConfig()`
   - 添加完整的错误处理

2. **Day 3-4**: 完善适配器
   - `localAdapter.ts` 完整 CRUD
   - `cloudAdapter.ts` 完整 CRUD
   - `lanAdapter.ts` 完整 CRUD

3. **Day 5**: 重构 `api.ts`
   - 添加 `sourceId` 参数到所有函数
   - 移除 `getConnectionMode()` 相关逻辑

### Week 2: 视图层

1. **Day 1-2**: 创建新的 `dashboard/` 组件
   - `DashboardView.tsx`
   - `SourceCard.tsx`
   - `SourceList.tsx`
   - `AddSourceDialog.tsx`

2. **Day 3-4**: 重写 `UnifiedDashboard.tsx`
   - 集成新的 dashboard 组件
   - 实现完整的导航流程

3. **Day 5**: 修改 `TeamView.tsx`
   - 移除模式选择逻辑
   - 直接渲染 UnifiedDashboard

### Week 3: 清理与测试

1. **Day 1-2**: 存储迁移
   - 实现 `migration.ts`
   - 测试迁移逻辑

2. **Day 3-4**: 删除旧文件
   - 删除不再需要的文件
   - 更新所有导入

3. **Day 5**: 测试
   - 端到端测试
   - 回归测试

## 七、风险与回滚

### 风险

1. **数据丢失**: 迁移过程中可能丢失配置
   - 缓解: 迁移前备份 localStorage

2. **功能退化**: 新架构可能遗漏某些功能
   - 缓解: 详细的功能对照表

3. **性能问题**: 多源聚合可能导致性能下降
   - 缓解: 添加分页和懒加载

### 回滚计划

1. 保留 `AGIME_TEAM_STORAGE_MIGRATED` 标记
2. 如需回滚，清除该标记即可使用旧存储
3. 旧的 `serverStore.ts` 和 `lanStore.ts` 保留，随时可用

## 八、验收标准

- [ ] 用户可以同时连接多个云端服务器
- [ ] 用户可以同时连接多个 LAN 设备
- [ ] 仪表板显示所有数据源的状态
- [ ] 可以聚合查看所有源的资源
- [ ] 可以按数据源过滤资源
- [ ] 旧的连接配置自动迁移到新格式
- [ ] 代码无冗余，结构清晰
- [ ] 所有现有功能正常工作
