# AGIME 常见问题 (FAQ)

**问题分类导航**:

| 分类 | 相关问题 |
|------|----------|
| 📘 **基础问题** | [AGIME是什么](#agime-是什么) · [与其他AI工具区别](#agime-与其他-ai-工具的区别) · [使用要求](#需要什么才能使用) |
| 🚀 **安装配置** | [如何安装](#如何安装-agime) · [支持的模型](#支持哪些-ai-模型) · [API Key配置](#如何配置-api-key) |
| ⚙️ **使用问题** | [执行任务](#如何让-agime-执行任务) · [食谱Recipe](#什么是食谱recipe) · [权限模式](#什么是权限模式) · [定时任务](#如何定时执行任务) |
| 👥 **团队协作** | [Team Server](#什么是-team-server) · [MongoDB vs SQLite](#mongodb-和-sqlite-如何选择) · [部署方式](#如何部署-team-server) |
| 🔌 **扩展插件** | [MCP扩展](#什么是-mcp-扩展) · [安装扩展](#如何安装扩展) · [开发扩展](#如何开发自定义扩展) |
| ⚡ **性能优化** | [提高速度](#如何提高响应速度) · [减少费用](#如何减少-api-费用) · [内存管理](#内存占用过高怎么办) |
| 🔒 **安全隐私** | [数据存储](#数据存储在哪里) · [API Key保护](#api-key-如何保护) · [操作安全](#如何确保操作安全) |
| 🔧 **故障排查** | [启动问题](#应用无法启动) · [API失败](#api-调用失败) · [工具失败](#工具执行失败) |

---

## 基础问题

### AGIME 是什么？

AGIME 是一个本地优先的 AI Agent 框架，可以帮你自动执行任务、处理文档、操控电脑等。与传统 AI 聊天工具不同，AGIME 不仅能对话，还能实际执行操作。

### AGIME 与其他 AI 工具的区别？

| 特性 | 传统 AI | AGIME |
|------|---------|-------|
| 执行任务 | ❌ 只能建议 | ✅ 直接执行 |
| 本地文件 | ❌ 无法访问 | ✅ 读写自如 |
| 定时任务 | ❌ 不支持 | ✅ 自动执行 |
| 数据隐私 | ☁️ 上传云端 | 🏠 本地处理 |

### 需要什么才能使用？

**必需**：
- 操作系统：Windows 10+, macOS 10.15+, 或 Linux
- AI 模型 API Key（如 Anthropic Claude, OpenAI GPT）

**可选**：
- MongoDB（用于团队协作）
- 网络连接（使用云端模型时）

## 安装和配置

### 如何安装 AGIME？

1. 从 [GitHub Releases](https://github.com/jsjm1986/AGIME/releases) 下载
2. 安装对应平台的安装包
3. 首次启动时配置 API Key

### 支持哪些 AI 模型？

**云端模型**：
- Anthropic Claude（推荐）
- OpenAI GPT
- Google Gemini
- Azure OpenAI
- 其他 14+ 提供商

**本地模型**：
- Ollama（完全离线）

### 如何配置 API Key？

**方式 1：通过界面**
1. 打开设置
2. 选择 Provider
3. 输入 API Key

**方式 2：通过环境变量**
```bash
export AGIME_ANTHROPIC_API_KEY="your-key"
export AGIME_OPENAI_API_KEY="your-key"
```

## 使用问题

### 如何让 AGIME 执行任务？

直接用自然语言描述任务：
```
帮我把这个文件夹里的所有 PDF 提取文字，生成 Excel 表格
```

AGIME 会自动：
1. 理解任务
2. 调用工具
3. 执行操作
4. 返回结果

### 什么是食谱（Recipe）？

食谱是可复用的任务模板。把一次成功的对话保存为食谱，下次一键执行。

**创建食谱**：
```bash
agime recipe create --from-session <session-id>
```

**使用食谱**：
```bash
agime recipe run <recipe-name>
```

### 什么是权限模式？

| 模式 | 说明 | 适用场景 |
|------|------|----------|
| 🟢 自主 | AI 自由操作 | 信任的重复任务 |
| 🟡 智能 | 高风险需确认 | 日常使用推荐 |
| 🔴 手动 | 每步都确认 | 敏感操作 |
| ⚪ 聊天 | 仅对话 | 咨询问答 |

### 如何定时执行任务？

```bash
# 创建定时任务
agime schedule create \
  --cron "0 9 * * *" \
  --recipe daily-report

# 列出定时任务
agime schedule list
```

## 团队协作

### 什么是 Team Server？

Team Server 是团队协作后端，支持：
- 共享技能和食谱
- 团队文档管理
- Portal 对外发布
- 统一权限管理

### MongoDB 和 SQLite 如何选择？

| 功能 | MongoDB | SQLite |
|------|---------|--------|
| 基础协作 | ✅ | ✅ |
| Team Agent | ✅ | ❌ |
| Portal 系统 | ✅ | ❌ |
| 审计统计 | ✅ | ❌ |

**建议**：团队正式使用选择 MongoDB。

### 如何部署 Team Server？

**Docker 部署**：
```bash
cd crates/agime-team-server
docker-compose up -d
```

**手动部署**：
```bash
# 构建
cargo build --release --bin agime-team-server

# 运行
MONGODB_URI="mongodb://localhost:27017" \
./target/release/agime-team-server
```

## 扩展和插件

### 什么是 MCP 扩展？

MCP (Model Context Protocol) 扩展为 AGIME 提供额外能力：
- Developer：代码分析、文件编辑
- ComputerController：自动化操作电脑
- Memory：智能记忆
- Playwright：网页自动化

### 如何安装扩展？

**内置扩展**：自动可用

**自定义扩展**：
```bash
# 添加扩展配置
agime extension add \
  --name my-extension \
  --command "node /path/to/extension"
```

### 如何开发自定义扩展？

参考 [MCP 协议文档](MCP_PROTOCOL.md) 和官方 SDK：
- Python: FastMCP
- TypeScript: MCP SDK

## 性能和优化

### 如何提高响应速度？

1. **使用更快的模型**：如 Claude 3.5 Haiku
2. **启用上下文压缩**：自动触发
3. **减少扩展数量**：只启用必需的
4. **使用本地模型**：Ollama 完全离线

### 如何减少 API 费用？

1. **选择合适的模型**：简单任务用小模型
2. **启用上下文管理**：自动压缩历史
3. **使用食谱**：避免重复对话
4. **本地模型**：Ollama 零费用

### 内存占用过高怎么办？

1. 关闭不用的会话
2. 禁用不必要的扩展
3. 定期清理历史
4. 重启应用

## 安全和隐私

### 数据存储在哪里？

**桌面版**：
- 会话数据：`~/.agime/sessions/`
- 配置文件：`~/.agime/config.yaml`
- 完全本地，不上传云端

**Team Server**：
- MongoDB 或 SQLite 数据库
- 可部署在内网

### API Key 如何保护？

- 使用系统 Keyring 加密存储
- Windows: Credential Manager
- macOS: Keychain
- Linux: Secret Service

### 如何确保操作安全？

1. **权限系统**：控制工具执行权限
2. **路径验证**：防止路径穿越
3. **命令检查**：识别危险操作
4. **审计日志**：记录关键操作

## 故障排查

### 应用无法启动？

查看 [故障排查手册](TROUBLESHOOTING.md) 的"安装和启动问题"章节。

### API 调用失败？

1. 检查 API Key 是否有效
2. 验证网络连接
3. 查看错误日志

### 工具执行失败？

1. 检查权限模式设置
2. 验证工作目录权限
3. 查看工具日志

## 更多资源

- [用户指南](USER_GUIDE.md) - 详细使用说明
- [架构文档](ARCHITECTURE.md) - 技术架构
- [API 参考](API_REFERENCE.md) - API 接口
- [故障排查](TROUBLESHOOTING.md) - 问题诊断
- [贡献指南](CONTRIBUTING.md) - 参与开发

## 获取帮助

- GitHub Issues: https://github.com/jsjm1986/AGIME/issues
- GitHub Discussions: 讨论功能和想法
- 微信: agimeme（企业服务）
