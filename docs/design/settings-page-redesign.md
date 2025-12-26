# AGIME 设置页面 Material Design 重设计方案

## 1. 设计目标

- **统一性**：所有设置项使用一致的间距、字号、颜色
- **层次感**：通过视觉层级区分区块标题、子项标题、描述文本
- **双色适配**：完美支持浅色/深色模式，确保两种模式下的可读性和美观度
- **Material Design**：遵循 Material Design 3 的设计原则

---

## 2. 设计令牌 (Design Tokens)

### 2.1 字体规范

| 元素 | 字号 | 字重 | 行高 | 类名 |
|------|------|------|------|------|
| 区块标题 | 16px | 600 (semibold) | 24px | `text-base font-semibold leading-6` |
| 子项标题 | 14px | 500 (medium) | 20px | `text-sm font-medium leading-5` |
| 描述文本 | 12px | 400 (normal) | 16px | `text-xs leading-4` |
| 辅助文本 | 11px | 400 (normal) | 14px | `text-[11px] leading-[14px]` |

### 2.2 颜色规范

#### 浅色模式 (Light Mode)

| 元素 | 颜色变量 | 实际色值 | 用途 |
|------|----------|----------|------|
| 区块标题 | `--text-default` | #3f434b | 主要标题文字 |
| 子项标题 | `--text-default` | #3f434b | 设置项名称 |
| 描述文本 | `--text-muted` | #878787 | 说明文字 |
| 卡片背景 | `--background-card` | #ffffff | 设置卡片 |
| 悬停背景 | `--background-muted` | #f4f6f7 | 鼠标悬停 |
| 分割线 | `--border-default` | #e3e6ea | 区域分隔 |
| 强调色 | `--color-block-teal` | #13bbaf | 开关激活态 |

#### 深色模式 (Dark Mode)

| 元素 | 颜色变量 | 实际色值 | 用途 |
|------|----------|----------|------|
| 区块标题 | `--text-default` | #ffffff | 主要标题文字 |
| 子项标题 | `--text-default` | #ffffff | 设置项名称 |
| 描述文本 | `--text-muted` | #878787 | 说明文字 |
| 卡片背景 | `--background-card` | #22252a | 设置卡片 |
| 悬停背景 | `--background-muted` | #3f434b | 鼠标悬停 |
| 分割线 | `--border-default` | #32353b | 区域分隔 |
| 强调色 | `--color-block-teal` | #13bbaf | 开关激活态 |

### 2.3 间距规范

| 间距类型 | 尺寸 | Tailwind 类 | 用途 |
|----------|------|-------------|------|
| 卡片内边距 | 16px | `p-4` | Card 内部填充 |
| 区块间距 | 24px | `space-y-6` | Card 之间的垂直距离 |
| 子项间距 | 12px | `space-y-3` | 设置项之间的垂直距离 |
| 标题-描述间距 | 4px | `mt-1` | 标题与描述之间 |
| 描述-控件间距 | 12px | `mt-3` | 描述与输入控件之间 |
| 图标-文字间距 | 12px | `gap-3` | 图标与文字之间 |

### 2.4 圆角规范

| 元素 | 圆角 | Tailwind 类 |
|------|------|-------------|
| 卡片 | 12px | `rounded-xl` |
| 按钮 | 8px | `rounded-lg` |
| 输入框 | 6px | `rounded-md` |
| 开关 | 全圆 | `rounded-full` |

---

## 3. 组件层级结构

```
设置页面
├── 区块卡片 (Card) ─────────────────────────────────
│   ├── 区块头部 (CardHeader)
│   │   ├── 图标 (可选)
│   │   ├── 区块标题 (CardTitle) ← text-base font-semibold
│   │   └── 区块描述 (CardDescription) ← text-xs text-text-muted mt-1
│   │
│   └── 区块内容 (CardContent)
│       ├── 设置项 (SettingsItem) ─────────────────
│       │   ├── 子项标题 ← text-sm font-medium
│       │   ├── 子项描述 ← text-xs text-text-muted mt-0.5
│       │   └── 控件 (开关/按钮/输入框)
│       │
│       ├── 设置项...
│       └── 设置项...
│
├── 区块卡片...
└── 区块卡片...
```

---

## 4. 标准组件样式

### 4.1 区块卡片 (SettingsCard)

```tsx
// 新建组件: ui/desktop/src/components/settings/common/SettingsCard.tsx

interface SettingsCardProps {
  icon?: React.ReactNode;
  title: string;
  description?: string;
  children: React.ReactNode;
}

// 样式定义
const cardStyles = {
  wrapper: "rounded-xl border border-border-default bg-background-card",
  header: "p-4 pb-0",
  headerWithIcon: "flex items-start gap-3",
  icon: "flex-shrink-0 w-5 h-5 text-text-muted mt-0.5",
  title: "text-base font-semibold text-text-default leading-6",
  description: "text-xs text-text-muted mt-1 leading-4",
  content: "p-4 pt-4 space-y-3",
};
```

### 4.2 设置项 (SettingsItem)

```tsx
// 新建组件: ui/desktop/src/components/settings/common/SettingsItem.tsx

interface SettingsItemProps {
  title: string;
  description?: string;
  control?: React.ReactNode;  // 右侧控件
  children?: React.ReactNode; // 展开内容
  onClick?: () => void;
}

// 样式定义
const itemStyles = {
  wrapper: "py-2 px-2 rounded-lg hover:bg-background-muted transition-colors",
  clickable: "cursor-pointer",
  layout: "flex items-center justify-between gap-4",
  textArea: "flex-1 min-w-0",
  title: "text-sm font-medium text-text-default leading-5",
  description: "text-xs text-text-muted mt-0.5 leading-4",
  control: "flex-shrink-0",
  expandedContent: "mt-3 space-y-3",
};
```

### 4.3 深色模式特殊处理

```css
/* 悬停效果 - 双色模式适配 */
.settings-item:hover {
  /* 浅色模式：微妙的灰色背景 */
  background-color: var(--background-muted);
}

.dark .settings-item:hover {
  /* 深色模式：略亮的背景，增加可见性 */
  background-color: var(--background-muted);
}

/* 卡片阴影 - 双色模式适配 */
.settings-card {
  /* 浅色模式：柔和阴影 */
  box-shadow: 0 1px 3px rgba(0, 0, 0, 0.08);
}

.dark .settings-card {
  /* 深色模式：边框代替阴影，避免"漂浮"感 */
  box-shadow: none;
  border-color: var(--border-default);
}
```

---

## 5. 需要修改的文件清单

### 5.1 新建文件

| 文件路径 | 用途 |
|----------|------|
| `components/settings/common/SettingsCard.tsx` | 统一的设置卡片组件 |
| `components/settings/common/SettingsItem.tsx` | 统一的设置项组件 |
| `components/settings/common/SettingsSection.tsx` | 设置区块容器 |
| `components/settings/common/index.ts` | 导出文件 |

### 5.2 需要重构的文件

| 文件 | 修改内容 |
|------|----------|
| `components/ui/card.tsx` | 添加 settings 变体样式 |
| `components/settings/app/AppSettingsSection.tsx` | 使用新组件重构 |
| `components/settings/chat/ChatSettingsSection.tsx` | 使用新组件重构 |
| `components/settings/mode/ModeSection.tsx` | 使用新组件重构 |
| `components/settings/mode/ModeSelectionItem.tsx` | 统一样式 |
| `components/settings/thinking/ThinkingModeToggle.tsx` | 统一样式 |
| `components/settings/security/SecurityToggle.tsx` | 统一样式 |
| `components/settings/response_styles/ResponseStylesSection.tsx` | 使用新组件重构 |
| `components/settings/dictation/VoiceDictationToggle.tsx` | 统一样式 |
| `components/settings/prompts/PromptsSection.tsx` | 统一样式 |
| `components/settings/sessions/SessionSharingSection.tsx` | 统一样式 |
| `components/settings/tunnel/TunnelSection.tsx` | 统一样式 |
| `components/settings/config/ConfigSettings.tsx` | 统一样式 |

---

## 6. 视觉对比示例

### 6.1 修改前（当前状态）

```
模式                          ← text-xs (12px) 太小
配置 AGIME 与工具和扩展...     ← 间距不一致

自主模式                       ← text-sm
完全自主执行...                ← mt-[2px] 间距太小

─────────────────────────────

扩展思考模式                   ← text-sm + 有图标
为支持的模型启用...            ← 间距正常
```

### 6.2 修改后（目标状态）

```
┌─────────────────────────────────────────────┐
│ ⚙ 模式                      ← text-base font-semibold (16px)
│   配置 AGIME 与工具和扩展... ← text-xs mt-1
│
│   ○ 自主模式                 ← text-sm font-medium
│     完全自主执行...          ← text-xs mt-0.5
│                              ← space-y-3 (12px 间距)
│   ○ 手动模式
│     所有工具、扩展...
│
│   ○ 智能模式
│     根据操作风险级别...
└─────────────────────────────────────────────┘
                               ← space-y-6 (24px 间距)
┌─────────────────────────────────────────────┐
│ 🧠 扩展思考模式              ← text-base font-semibold
│   为支持的模型启用...        ← text-xs mt-1
│
│   思考预算 (token 数量)      ← text-sm font-medium
│   思考的最大 token 数量...   ← text-xs mt-0.5
│   ┌─────────────────┐
│   │ 20000           │        ← 输入框 mt-3
│   └─────────────────┘
└─────────────────────────────────────────────┘
```

---

## 7. 实施步骤

### Phase 1: 基础组件创建 (Day 1)
1. 创建 `SettingsCard` 组件
2. 创建 `SettingsItem` 组件
3. 创建 `SettingsSection` 容器组件
4. 更新 `card.tsx` 添加 settings 变体

### Phase 2: 核心页面重构 (Day 2-3)
1. 重构 `AppSettingsSection.tsx`
2. 重构 `ChatSettingsSection.tsx`
3. 重构 `ModeSection.tsx` 和 `ModeSelectionItem.tsx`

### Phase 3: 功能组件统一 (Day 4-5)
1. 统一 `ThinkingModeToggle.tsx`
2. 统一 `SecurityToggle.tsx`
3. 统一 `VoiceDictationToggle.tsx`
4. 统一 `ResponseStylesSection.tsx`

### Phase 4: 其他区块 (Day 6)
1. 统一 `PromptsSection.tsx`
2. 统一 `SessionSharingSection.tsx`
3. 统一 `TunnelSection.tsx`
4. 统一 `ConfigSettings.tsx`

### Phase 5: 测试与优化 (Day 7)
1. 浅色模式测试
2. 深色模式测试
3. 响应式布局测试
4. 动画过渡优化

---

## 8. 关键设计决策

### 8.1 为什么选择 16px 区块标题？
- Material Design 3 推荐的 Title Medium 尺寸
- 与 14px 子项标题形成明显的视觉层级
- 在两种颜色模式下都有良好的可读性

### 8.2 为什么间距使用 12px？
- 8px 太紧凑，16px 太松散
- 12px 是 4px 基础网格的 3 倍，符合 Material Design 的 4dp 网格系统
- 在高密度信息展示中保持舒适感

### 8.3 深色模式特殊考虑
- **对比度**：深色背景上的 #878787 灰色文字对比度为 4.6:1，符合 WCAG AA 标准
- **层级感**：使用边框而非阴影来区分卡片，避免"漂浮"感
- **悬停效果**：深色模式下使用更亮的背景色，确保可见性

---

## 9. 代码示例

### 9.1 SettingsCard 组件

```tsx
import React from 'react';
import { cn } from '@/lib/utils';

interface SettingsCardProps {
  icon?: React.ReactNode;
  title: string;
  description?: string;
  children: React.ReactNode;
  className?: string;
}

export const SettingsCard: React.FC<SettingsCardProps> = ({
  icon,
  title,
  description,
  children,
  className,
}) => {
  return (
    <div className={cn(
      "rounded-xl border border-border-default bg-background-card",
      "shadow-[0_1px_3px_rgba(0,0,0,0.08)] dark:shadow-none",
      className
    )}>
      {/* Header */}
      <div className="p-4 pb-0">
        <div className={cn("flex", icon && "items-start gap-3")}>
          {icon && (
            <div className="flex-shrink-0 w-5 h-5 text-text-muted mt-0.5">
              {icon}
            </div>
          )}
          <div>
            <h3 className="text-base font-semibold text-text-default leading-6">
              {title}
            </h3>
            {description && (
              <p className="text-xs text-text-muted mt-1 leading-4">
                {description}
              </p>
            )}
          </div>
        </div>
      </div>

      {/* Content */}
      <div className="p-4 pt-4 space-y-3">
        {children}
      </div>
    </div>
  );
};
```

### 9.2 SettingsItem 组件

```tsx
import React from 'react';
import { cn } from '@/lib/utils';

interface SettingsItemProps {
  title: string;
  description?: string;
  control?: React.ReactNode;
  children?: React.ReactNode;
  onClick?: () => void;
  className?: string;
}

export const SettingsItem: React.FC<SettingsItemProps> = ({
  title,
  description,
  control,
  children,
  onClick,
  className,
}) => {
  return (
    <div className={cn("space-y-0", className)}>
      <div
        className={cn(
          "py-2 px-2 rounded-lg transition-colors",
          "hover:bg-background-muted",
          onClick && "cursor-pointer"
        )}
        onClick={onClick}
      >
        <div className="flex items-center justify-between gap-4">
          <div className="flex-1 min-w-0">
            <h4 className="text-sm font-medium text-text-default leading-5">
              {title}
            </h4>
            {description && (
              <p className="text-xs text-text-muted mt-0.5 leading-4">
                {description}
              </p>
            )}
          </div>
          {control && (
            <div className="flex-shrink-0">
              {control}
            </div>
          )}
        </div>
      </div>

      {/* Expanded Content */}
      {children && (
        <div className="mt-3 px-2 space-y-3">
          {children}
        </div>
      )}
    </div>
  );
};
```

---

## 10. 验收标准

- [ ] 所有区块标题使用 16px semibold
- [ ] 所有子项标题使用 14px medium
- [ ] 所有描述文本使用 12px，颜色为 text-muted
- [ ] 标题与描述间距统一为 4px (mt-1) 或 2px (mt-0.5)
- [ ] 设置项之间间距统一为 12px (space-y-3)
- [ ] 卡片之间间距统一为 24px (space-y-6)
- [ ] 浅色模式下视觉效果符合预期
- [ ] 深色模式下视觉效果符合预期
- [ ] 悬停效果在两种模式下都清晰可见
- [ ] 无样式回归问题
