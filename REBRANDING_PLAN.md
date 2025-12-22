# Goose → AGIME 品牌重命名审计报告

**审计日期**: 2024-12-22
**审计范围**: 全代码库
**风险评估**: 完整

---

## 目录

1. [执行摘要](#执行摘要)
2. [优先级分类](#优先级分类)
3. [详细发现](#详细发现)
4. [修改计划](#修改计划)
5. [风险分析](#风险分析)
6. [实施建议](#实施建议)

---

## 执行摘要

| 分类 | 数量 | 用户可见 | 风险等级 |
|------|------|----------|----------|
| UI 组件 | 64 个文件 | 是 | 高 |
| i18n 翻译 | 5 处引用 | 是 | 高 |
| 文件名 | 34 个文件 | 部分 | 中 |
| Rust crate | 6 个包 | 否 | 低 |
| 环境变量 | 50+ 处 | 是 | 高 |
| 外部 URL | 30+ 处 | 是 | 高 |
| 二进制文件 | 2 个 | 是 | 中 |
| GitHub 配置 | 15+ 处 | 否 | 低 |

---

## 优先级分类

### P0 - 立即修复 (用户直接可见)

#### 1. i18n 翻译文件 - 品牌名称
```
ui/desktop/src/i18n/locales/en/sessions.json:51  "goose": "AGIME"
ui/desktop/src/i18n/locales/zh-CN/sessions.json:51  "goose": "AGIME"
```
**状态**: ✅ 已修复 (映射到 AGIME)

#### 2. i18n 中的技术引用
```
ui/desktop/src/i18n/locales/en/extensions.json:47
  "skills": "Load and use skills from .claude/skills or .goose/skills directories"

ui/desktop/src/i18n/locales/zh-CN/extensions.json:47
  "skills": "从 .claude/skills 或 .goose/skills 目录加载和使用技能"
```
**建议**: 改为 `.agime/skills` 或保留 `.goose/skills` 作为兼容路径

#### 3. localStorage 键名
```
ui/desktop/src/i18n/index.ts:87
  lookupLocalStorage: 'goose-language'
```
**建议**: 改为 `agime-language`，需要数据迁移逻辑

---

### P1 - 高优先级 (用户界面元素)

#### 4. React 组件文件名 (需要重命名)

| 当前文件名 | 建议新名称 | 用户可见 |
|-----------|-----------|----------|
| `GooseLogo.tsx` | `AgimeLogo.tsx` | 是 |
| `GooseMessage.tsx` | `AgimeMessage.tsx` | 是 |
| `GooseSidebar/` | `AgimeSidebar/` | 是 |
| `icons/Goose.tsx` | `icons/Agime.tsx` | 是 |
| `LoadingGoose.tsx` | `LoadingAgime.tsx` | 是 |
| `WelcomeGooseLogo.tsx` | `WelcomeAgimeLogo.tsx` | 是 |
| `GoosehintsModal.tsx` | `AgimehintsModal.tsx` | 是 |
| `GoosehintsSection.tsx` | `AgimehintsSection.tsx` | 是 |

#### 5. 图片资源
```
ui/desktop/src/assets/battle-game/goose.png
ui/desktop/src/images/loading-goose/
```
**建议**: 替换为 AGIME 品牌图片

#### 6. 配置文件目录名
```
.goosehints (项目根目录)
ui/desktop/.goosehints
```
**建议**:
- 方案 A: 重命名为 `.agimehints`
- 方案 B: 同时支持 `.goosehints` 和 `.agimehints`

---

### P2 - 中优先级 (环境变量和配置)

#### 7. 环境变量 (GOOSE_ 前缀)

**download_cli.sh / download_cli.ps1:**
```
GOOSE_BIN_DIR
GOOSE_VERSION
GOOSE_PROVIDER
GOOSE_MODEL
```

**Justfile:**
```
GOOSE_PATH_ROOT
GOOSE_EXTERNAL_BACKEND
```

**Rust 后端:**
```
GOOSE_SERVER__SECRET_KEY
GOOSE_PROVIDER__TYPE
GOOSE_MODEL__TYPE
```

**建议**:
- 方案 A: 全部改为 `AGIME_*` 前缀
- 方案 B: 支持两种前缀，优先 `AGIME_*`

**影响范围**: 用户配置、CI/CD、文档

#### 8. 二进制文件名
```
ui/desktop/src/bin/goose.exe
ui/desktop/src/bin/goosed.exe
ui/desktop/src/bin/goose-package/
ui/desktop/src/bin/goose-windows.zip
```

**建议**:
- `goose.exe` → `agime.exe`
- `goosed.exe` → `agimed.exe` (daemon)

**风险**: 需要同步修改:
- Electron 主进程代码 (`goosed.ts`)
- GitHub Actions 构建脚本
- 安装脚本

---

### P3 - 低优先级 (内部代码)

#### 9. Rust Crate 名称

| 当前名称 | 保留/修改 | 原因 |
|---------|-----------|------|
| `goose` | 保留 | 内部依赖，不影响用户 |
| `goose-cli` | 保留 | Cargo.toml 内部引用 |
| `goose-server` | 保留 | 内部包名 |
| `goose-mcp` | 保留 | 内部包名 |
| `goose-bench` | 保留 | 开发工具 |
| `goose-test` | 保留 | 测试工具 |

**建议**: 内部 crate 名称不改，避免大规模重构风险

#### 10. 源代码文件名 (Electron 主进程)
```
ui/desktop/src/goosed.ts
```
**建议**: 改为 `agimed.ts`

#### 11. 测试快照文件
```
crates/goose/src/agents/snapshots/goose__agents__*.snap
```
**建议**: 保留，这是内部测试文件

---

### P4 - 外部资源 (需要协调)

#### 12. GitHub URLs
```
github.com/block/goose → 需要 fork 说明
block.github.io/goose/docs/ → 建议建立独立文档站
discord.gg/goose-oss → 建议建立独立社区
```

#### 13. 需要更新的外部链接位置
```
.github/CODEOWNERS
.github/ISSUE_TEMPLATE/config.yml
.github/ISSUE_TEMPLATE/bug_report.md
.github/ISSUE_TEMPLATE/submit-recipe.yml
.github/DISCUSSION_TEMPLATE/qa.yml
.github/pull_request_template.md
.github/workflows/*.yml
Cargo.toml (repository URL)
Dockerfile (LABEL)
download_cli.sh
download_cli.ps1
NOTICE
README.md / README.en.md
```

---

## 修改计划

### 阶段 1: 用户可见品牌 (高优先级)

```bash
# 1.1 i18n 技术引用更新
files=(
  "ui/desktop/src/i18n/locales/en/extensions.json"
  "ui/desktop/src/i18n/locales/zh-CN/extensions.json"
)
# 将 .goose/skills 改为 .agime/skills 或保留兼容

# 1.2 localStorage 键名
# ui/desktop/src/i18n/index.ts: goose-language → agime-language

# 1.3 React 组件重命名 (使用 git mv 保留历史)
git mv src/components/GooseLogo.tsx src/components/AgimeLogo.tsx
git mv src/components/GooseMessage.tsx src/components/AgimeMessage.tsx
git mv src/components/GooseSidebar src/components/AgimeSidebar
git mv src/components/icons/Goose.tsx src/components/icons/Agime.tsx
git mv src/components/LoadingGoose.tsx src/components/LoadingAgime.tsx
git mv src/components/WelcomeGooseLogo.tsx src/components/WelcomeAgimeLogo.tsx
git mv src/components/settings/chat/GoosehintsModal.tsx src/components/settings/chat/AgimehintsModal.tsx
git mv src/components/settings/chat/GoosehintsSection.tsx src/components/settings/chat/AgimehintsSection.tsx
```

### 阶段 2: 环境变量兼容层

```typescript
// utils/env-compat.ts
export function getEnv(name: string): string | undefined {
  // 优先 AGIME_ 前缀
  const agimeKey = name.replace(/^GOOSE_/, 'AGIME_');
  return process.env[agimeKey] || process.env[name];
}
```

```rust
// Rust 兼容层
fn get_env_compat(name: &str) -> Option<String> {
    let agime_name = name.replace("GOOSE_", "AGIME_");
    std::env::var(&agime_name).ok().or_else(|| std::env::var(name).ok())
}
```

### 阶段 3: 二进制文件重命名

```bash
# 需要同步修改的文件
# 1. .github/workflows/build-all-platforms.yml
# 2. .github/workflows/bundle-desktop-*.yml
# 3. ui/desktop/src/goosed.ts
# 4. ui/desktop/forge.config.ts
# 5. download_cli.sh / download_cli.ps1
```

### 阶段 4: 配置文件目录

```typescript
// 支持两种配置文件名
const configPaths = [
  '.agimehints',  // 新名称优先
  '.goosehints',  // 向后兼容
];
```

---

## 风险分析

### 高风险项

| 项目 | 风险 | 缓解措施 |
|------|------|----------|
| 环境变量重命名 | 破坏现有用户配置 | 实现兼容层，两种前缀都支持 |
| 二进制文件重命名 | 破坏安装脚本 | 分阶段部署，保留旧名称符号链接 |
| localStorage 键名 | 丢失用户语言偏好 | 实现数据迁移逻辑 |
| 外部 URL | 文档链接失效 | 建立独立文档站或重定向 |

### 中风险项

| 项目 | 风险 | 缓解措施 |
|------|------|----------|
| React 组件重命名 | import 路径失效 | 使用 IDE 批量重构 + 全面测试 |
| 配置目录重命名 | 用户配置丢失 | 同时支持新旧目录名 |

### 低风险项

| 项目 | 风险 | 缓解措施 |
|------|------|----------|
| 内部 crate 名称 | 编译错误 | 不修改，保持内部稳定 |
| 测试快照 | 测试失败 | 不修改或重新生成 |

---

## 实施建议

### 推荐实施顺序

1. **第一批 (低风险，高可见)**
   - i18n 翻译文件
   - 图片资源替换
   - README 更新

2. **第二批 (中风险)**
   - React 组件重命名
   - localStorage 键名 + 迁移逻辑
   - 配置目录兼容层

3. **第三批 (高风险，需要测试)**
   - 环境变量兼容层
   - 二进制文件重命名
   - 安装脚本更新

4. **第四批 (外部依赖)**
   - 建立独立文档站
   - 建立独立 Discord 社区
   - 更新 GitHub 模板

### 测试检查清单

- [ ] 全量 TypeScript 编译通过
- [ ] 全量 Rust 编译通过
- [ ] Electron 应用正常启动
- [ ] Web UI 正常加载
- [ ] i18n 所有语言正常显示
- [ ] 环境变量 GOOSE_* 仍然有效
- [ ] 环境变量 AGIME_* 正常工作
- [ ] 新安装用户体验正常
- [ ] 现有用户升级无数据丢失

---

## 保留项 (不修改)

以下内容建议保留，不进行重命名：

1. **内部 Rust crate 名称** - 修改成本高，用户不可见
2. **测试快照文件** - 内部测试用途
3. **Git 历史** - 保留完整历史记录
4. **上游引用** - README 中保留对原 goose 项目的致谢

---

## 附录: 完整文件清单

### 需要重命名的文件 (34 个)

```
.goosehints
ui/desktop/.goosehints
ui/desktop/src/components/GooseLogo.tsx
ui/desktop/src/components/GooseMessage.tsx
ui/desktop/src/components/GooseSidebar/
ui/desktop/src/components/icons/Goose.tsx
ui/desktop/src/components/LoadingGoose.tsx
ui/desktop/src/components/WelcomeGooseLogo.tsx
ui/desktop/src/components/settings/chat/GoosehintsModal.tsx
ui/desktop/src/components/settings/chat/GoosehintsSection.tsx
ui/desktop/src/goosed.ts
ui/desktop/src/images/loading-goose/
ui/desktop/src/assets/battle-game/goose.png
ui/desktop/src/bin/goose.exe
ui/desktop/src/bin/goosed.exe
ui/desktop/src/bin/goose-package/
ui/desktop/src/bin/goose-windows.zip
scripts/goose-db-helper.sh
goose-self-test.yaml
crates/goose/src/config/goose_mode.rs
```

### 需要修改内容的文件 (64+ 个)

详见上文各章节。

---

*最后更新: 2024-12-22*
