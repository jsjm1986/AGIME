# Goose → AGIME 重命名状态报告

## 一、用户可感知的修改 (已完成)

### ✅ 已修改 - 用户可直接看到

| 位置 | 原内容 | 新内容 | 用户可见场景 |
|------|--------|--------|--------------|
| 窗口标题 | AGIME (已是) | AGIME | 应用标题栏 |
| Web 标题 | AGIME Web | AGIME Web | 浏览器标签 |
| 应用名称 | agime-app | agime-app | 系统进程/任务栏 |
| 语言存储 | goose-language | agime-language | localStorage (不可见，但技术相关) |
| 技能目录 | .goose/skills | .agime/skills | 扩展描述文字 |
| Docker 镜像 | goose CLI | AGIME CLI | docker images 列表 |
| GitHub 模板 | goose cookbook | AGIME cookbook | Issue 提交页面 |
| Discord 名称 | goose Discord | AGIME Discord | Issue 联系链接 |
| 项目提示文件 | 仅 .goosehints | .agimehints + .goosehints 兼容 | 设置页面 |

---

## 二、.agimehints / .goosehints 兼容性实现 ✅ 已完成

### 后端修改 (Rust)

1. **`crates/goose/src/hints/load_hints.rs`**:
   - 新增 `AGIME_HINTS_FILENAME = ".agimehints"` 常量
   - 导出新常量供其他模块使用

2. **`crates/goose/src/agents/prompt_manager.rs`**:
   - 默认配置优先读取 `.agimehints`，然后读取 `.goosehints`
   ```rust
   vec![
       AGIME_HINTS_FILENAME.to_string(),  // .agimehints (优先)
       GOOSE_HINTS_FILENAME.to_string(),  // .goosehints (兼容)
       AGENTS_MD_FILENAME.to_string(),    // AGENTS.md
   ]
   ```

### 前端修改 (React)

1. **`ui/desktop/src/components/settings/chat/GoosehintsModal.tsx`**:
   - 定义双文件名常量
   - 优先检查 `.agimehints`，不存在则检查 `.goosehints`
   - 新建文件时使用 `.agimehints`

### i18n 翻译更新

1. **英文 (en/settings.json)**:
   - `"title": "Configure Project Hints"` (移除固定文件名)
   - `"sectionDescription": "Configure your project's hints file (.agimehints or .goosehints)..."`
   - `"helpText1": "Project hints files (.agimehints or .goosehints) are text files..."`

2. **中文 (zh-CN/settings.json)**:
   - `"title": "配置项目提示"`
   - `"sectionDescription": "配置项目的提示文件 (.agimehints 或 .goosehints)..."`
   - `"helpText1": "项目提示文件 (.agimehints 或 .goosehints) 是文本文件..."`

---

用户运行 `goose --help` 时会看到：

```
cli.rs:433  "Configure goose settings"
cli.rs:437  "Display goose information"
cli.rs:445  "Run one of the mcp servers bundled with goose"
cli.rs:452  "Run goose as an ACP agent server on stdio"
cli.rs:545  "builtin extensions that are bundled with goose"
cli.rs:578  "Input text to provide to goose directly"
cli.rs:800  "Update the goose CLI version"
cli.rs:854  "Terminal-integrated goose session"
cli.rs:894  "Make goose the default handler"
```

**风险评估**:
- 🟡 **中风险**: Rust 代码修改需要重新编译
- 📍 **文件**: `crates/goose-cli/src/cli.rs`

## 三、待处理 - CLI 帮助文本

### ⚠️ 中可见 - 错误和日志消息

```rust
// commands/project.rs
println!("Failed to run goose. Exit code: {:?}", status.code());
```

---

## 三、内部引用 (用户不可见，建议保留)

### 🟢 低风险 - 可保留不改

| 类型 | 内容 | 原因 |
|------|------|------|
| JSON key | `"gooseMessage"` | 代码引用键名，用户看不到 |
| JSON key | `"gooseServer"` | 代码引用键名 |
| JSON key | `"aboutGoose": "About AGIME"` | 值已是 AGIME |
| JSON key | `"askGoose": "Ask AGIME"` | 值已是 AGIME |
| React 文件名 | GooseLogo.tsx | 导出已是 AgimeLogo |
| Rust crate | goose, goose-cli | 内部包名 |
| Docker 用户 | goose | 系统用户名 |
| 测试快照 | goose__*.snap | 测试内部文件 |

---

## 四、外部链接 (需要独立基础设施)

| 链接 | 当前 | 需要 |
|------|------|------|
| Discord | discord.gg/goose-oss | 创建独立 AGIME Discord |
| 文档 | block.github.io/goose | 创建独立文档站 |
| 仓库 | github.com/block/goose | Fork 说明已有 |
| 更新脚本 | block/goose releases | 需要独立发布渠道 |

---

## 五、修改建议

### 方案 A: 最小改动 (推荐)

保持 `.goosehints` 不变，理由：
1. 已有用户可能有 `.goosehints` 文件
2. 改名需要后端 + 前端 + 文档同步
3. 技术文件名 (以 `.` 开头) 用户接受度高

只修改 CLI 帮助文本中的 "goose" → "AGIME"

### 方案 B: 完整改名

1. `.goosehints` → `.agimehints`
2. 后端支持两种文件名 (兼容)
3. CLI 帮助文本全部改为 AGIME
4. 需要数据迁移逻辑

---

## 六、风险矩阵

| 修改项 | 用户可见 | 技术风险 | 工作量 | 建议 |
|--------|----------|----------|--------|------|
| .goosehints UI 文字 | 高 | 高 | 中 | 暂缓或全改 |
| CLI 帮助文本 | 高 | 低 | 低 | ✅ 建议修改 |
| 环境变量 GOOSE_* | 高 | 高 | 高 | 需兼容层 |
| JSON key 名称 | 无 | 低 | 低 | ❌ 不改 |
| 文件名 | 无 | 中 | 中 | ❌ 不改 |
| 外部链接 | 中 | 无 | 高 | 需要基础设施 |

---

## 七、当前状态汇总

### 普通用户可感知到的变化

1. ✅ **应用标题**: AGIME
2. ✅ **扩展描述**: .agime/skills
3. ✅ **GitHub 模板**: AGIME cookbook
4. ✅ **项目提示文件**: 支持 .agimehints (优先) 和 .goosehints (兼容)
5. ⚠️ **CLI 帮助**: 仍显示 goose (建议修改)

### 开发者可感知到的变化

1. ✅ **Docker 镜像标签**: AGIME CLI
2. ✅ **localStorage key**: agime-language
3. ⚠️ **环境变量**: 仍是 GOOSE_* (需要兼容层)

---

*更新时间: 2024-12-22*
