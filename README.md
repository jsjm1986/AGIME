<p align="center">
  <img src="https://img.shields.io/badge/AGIME-AI%20Agent-blue?style=for-the-badge" alt="AGIME">
  <img src="https://img.shields.io/badge/Rust-000000?style=for-the-badge&logo=rust&logoColor=white" alt="Rust">
  <img src="https://img.shields.io/badge/TypeScript-007ACC?style=for-the-badge&logo=typescript&logoColor=white" alt="TypeScript">
  <img src="https://img.shields.io/badge/License-Apache_2.0-blue?style=for-the-badge" alt="License">
</p>

<h1 align="center">AGIME</h1>

<p align="center">
  <strong>Autonomous General Intelligent Multi-model Engine</strong>
</p>

<p align="center">
  一个开源、可扩展的本地化 AI 智能体框架<br>
  超越代码建议 — 安装、执行、编辑、测试，支持任意 LLM
</p>

<p align="center">
  <a href="#功能特性">功能特性</a> •
  <a href="#快速开始">快速开始</a> •
  <a href="#使用指南">使用指南</a> •
  <a href="#高级功能">高级功能</a> •
  <a href="#扩展系统">扩展系统</a>
</p>

<p align="center">
  <strong>中文</strong> | <a href="README.en.md">English</a>
</p>

---

## 什么是 AGIME？

AGIME 是一个运行在本地的 AI 智能体框架，能够自主完成复杂的开发任务。与传统的代码补全工具不同，AGIME 可以：

- **从零构建完整项目** - 不只是代码片段，而是完整的应用程序
- **自主执行和调试** - 运行代码、分析错误、自动修复
- **编排复杂工作流** - 协调多个工具和 API 完成任务
- **与外部系统交互** - 通过 MCP 协议连接各种服务和工具

AGIME 在您的开发环境中实时运行，作为真正的"智能体"——不仅能搜索、导航和编写代码，还能自主执行任务：读写文件、运行测试、安装依赖、处理各种操作。

## 功能特性

### 🤖 自主任务执行
不需要逐步指导，AGIME 可以理解目标并自主规划、执行、验证整个任务流程。

### 🔌 MCP 扩展系统
基于 [Model Context Protocol](https://modelcontextprotocol.io/) 的模块化扩展架构，轻松连接 GitHub、Google Drive、数据库等各种工具和服务。

### 🔄 多模型协作 (Lead/Worker)
智能双模型配置：使用强力模型（如 GPT-4、Claude Opus）进行规划，使用快速模型（如 GPT-4o-mini）执行任务，优化成本与性能。

### 📋 预设任务 (Recipes)
可复用的自动化工作流配置，支持定时触发、参数化执行，让 AI 智能体为您自动处理重复性任务。

### 🖥️ 多种使用方式
- **桌面应用** - 美观的图形界面，支持 Windows 和 macOS
- **命令行工具** - 强大的 CLI，适合终端爱好者和自动化场景

### 🔒 本地优先，隐私安全
所有处理都在本地完成，敏感数据不会发送到第三方服务器。特别适合金融、医疗、政府等对数据隐私要求严格的行业。

### 🌍 多语言支持
原生支持中文和英文界面，更多语言持续添加中。

## 快速开始

### 系统要求

- **操作系统**: Windows 10/11, macOS 10.15+
- **内存**: 8GB+ RAM（推荐 16GB）
- **存储**: 500MB 可用空间
- **网络**: 需要连接到 LLM 提供商的 API

### 安装方式

#### 方式一：桌面应用（推荐新手）

从 [Releases](https://github.com/agiemem/agime/releases) 页面下载适合您系统的安装包：

| 系统 | 下载链接 |
|------|----------|
| Windows | `AGIME-Setup-x.x.x.exe` |
| macOS (Intel) | `AGIME-x.x.x-x64.dmg` |
| macOS (Apple Silicon) | `AGIME-x.x.x-arm64.dmg` |

#### 方式二：命令行安装

**Windows (PowerShell):**
```powershell
# 下载并运行安装脚本
irm https://raw.githubusercontent.com/agiemem/agime/main/download_cli.ps1 | iex
```

**macOS / Linux:**
```bash
# 下载并运行安装脚本
curl -fsSL https://raw.githubusercontent.com/agiemem/agime/main/download_cli.sh | bash
```

**从源码构建:**
```bash
# 克隆仓库
git clone https://github.com/agiemem/agime.git
cd agime

# 构建 CLI
cargo build --release -p goose-cli

# 可执行文件位于 target/release/goose
```

### 首次配置

1. **启动 AGIME**
   ```bash
   goose configure
   ```

2. **选择 LLM 提供商**

   AGIME 支持多种 LLM 提供商：

   | 提供商 | 环境变量 | 说明 |
   |--------|----------|------|
   | OpenAI | `OPENAI_API_KEY` | GPT-4, GPT-4o 等 |
   | Anthropic | `ANTHROPIC_API_KEY` | Claude 3.5, Claude 4 等 |
   | Google | `GOOGLE_API_KEY` | Gemini 系列 |
   | Ollama | （本地运行） | 本地模型，无需 API |

3. **设置 API 密钥**
   ```bash
   # 方式一：环境变量
   export OPENAI_API_KEY="your-api-key"

   # 方式二：通过配置向导
   goose configure
   ```

4. **开始第一次对话**
   ```bash
   goose session
   ```

### Hello World 示例

```bash
# 启动 AGIME
goose session

# AGIME 启动后，尝试以下指令：
> 创建一个简单的 Python Flask 应用，包含一个返回 "Hello, AGIME!" 的 API 端点
```

AGIME 将自动：
1. 创建项目目录结构
2. 编写 Flask 应用代码
3. 创建 requirements.txt
4. 安装依赖
5. 运行并测试应用

## 使用指南

### CLI 命令参考

```bash
# 会话管理
goose session                    # 启动新会话
goose session --resume           # 恢复上一个会话
goose session -n "项目名"        # 使用指定名称启动会话

# 一次性执行
goose run --text "你的指令"      # 执行单个任务后退出
goose run --instructions file.md # 从文件读取指令

# 会话列表
goose session list               # 列出所有会话
goose session list --format json # JSON 格式输出
goose session remove             # 交互式删除会话

# 配置
goose configure                  # 配置向导
goose info                       # 显示当前配置

# 扩展
goose mcp                        # 管理 MCP 扩展

# 预设任务
goose recipe validate recipe.yaml  # 验证预设任务
goose recipe open recipe-name      # 在桌面应用打开

# 帮助
goose --help                     # 显示帮助
goose <command> --help           # 显示特定命令帮助
```

### 会话内命令

在 AGIME 会话中，可以使用以下斜杠命令：

| 命令 | 说明 |
|------|------|
| `/help` | 显示帮助信息 |
| `/mode <name>` | 设置运行模式（auto, approve, chat） |
| `/extension <cmd>` | 添加扩展 |
| `/builtin <names>` | 启用内置扩展 |
| `/plan` | 进入计划模式 |
| `/recipe` | 从当前会话生成预设任务 |
| `/compact` | 压缩对话历史 |
| `/clear` | 清空当前会话 |

### 运行模式

AGIME 支持多种运行模式，适应不同场景：

| 模式 | 说明 | 适用场景 |
|------|------|----------|
| `auto` | 自动执行所有操作 | 信任的自动化任务 |
| `approve` | 每个操作需要确认 | 敏感操作、学习过程 |
| `smart_approve` | 智能判断是否需要确认 | 日常开发 |
| `chat` | 仅对话，不执行操作 | 咨询、规划 |

```bash
# 设置默认模式
goose configure

# 会话中切换模式
/mode approve
```

## 高级功能

### Lead/Worker 多模型设置

Lead/Worker 模式让您可以组合使用两个不同的模型：

- **Lead 模型**: 负责初始规划和复杂推理
- **Worker 模型**: 负责执行具体任务

这种配置可以显著降低成本，同时保持高质量输出。

#### 配置方式

**环境变量:**
```bash
export GOOSE_PROVIDER="openai"
export GOOSE_MODEL="gpt-4o-mini"           # Worker 模型
export GOOSE_LEAD_MODEL="gpt-4o"           # Lead 模型
export GOOSE_LEAD_TURNS="3"                # 初始使用 Lead 的轮数
export GOOSE_LEAD_FAILURE_THRESHOLD="2"    # 失败多少次切换回 Lead
```

**桌面应用:**

设置 → 模型 → Lead/Worker 设置

#### 推荐配置

| 场景 | Lead 模型 | Worker 模型 |
|------|-----------|-------------|
| 高质量开发 | Claude Opus | Claude Sonnet |
| 成本优化 | GPT-4o | GPT-4o-mini |
| 跨厂商 | Claude Opus | GPT-4o-mini |

### 预设任务 (Recipes)

Recipes 是可复用的自动化工作流配置，支持：

- 预定义的任务指令
- 参数化配置
- 扩展预加载
- 定时触发

#### Recipe 文件格式

```yaml
# my-recipe.yaml
version: 1.0.0
title: "代码审查助手"
description: "自动审查 PR 并提供改进建议"

# 预加载的扩展
extensions:
  - name: developer
    type: builtin

# 初始提示
prompt: |
  请审查以下代码变更，关注：
  1. 代码质量和可读性
  2. 潜在的 bug
  3. 性能问题
  4. 安全隐患

  变更内容：{{changes}}

# 参数定义
parameters:
  - name: changes
    description: "代码变更内容"
    required: true
```

#### 使用 Recipe

```bash
# 验证 Recipe
goose recipe validate my-recipe.yaml

# 生成深度链接
goose recipe deeplink my-recipe.yaml -p changes="$(git diff)"

# 在桌面应用打开
goose recipe open my-recipe.yaml
```

### 定时任务调度

AGIME 支持基于 Cron 表达式的定时任务：

```bash
# 添加定时任务
goose schedule add \
  --schedule-id "daily-review" \
  --cron "0 9 * * *" \
  --recipe-source "./daily-review.yaml"

# 列出所有定时任务
goose schedule list

# 立即执行
goose schedule run-now --schedule-id "daily-review"

# 删除定时任务
goose schedule remove --schedule-id "daily-review"
```

#### Cron 表达式示例

| 表达式 | 说明 |
|--------|------|
| `0 * * * *` | 每小时整点 |
| `0 9 * * *` | 每天上午 9 点 |
| `0 9 * * 1` | 每周一上午 9 点 |
| `0 0 1 * *` | 每月 1 号凌晨 |

## 扩展系统

AGIME 使用 [Model Context Protocol (MCP)](https://modelcontextprotocol.io/) 作为扩展协议，支持三种类型的扩展：

### 内置扩展

| 扩展 | 说明 | 启用命令 |
|------|------|----------|
| `developer` | 文件操作、代码分析、Shell 命令 | `/builtin developer` |
| `memory` | 会话记忆和上下文管理 | `/builtin memory` |
| `computercontroller` | 系统控制、浏览器自动化 | `/builtin computercontroller` |
| `autovisualiser` | 数据可视化 | `/builtin autovisualiser` |
| `tutorial` | 交互式教程 | `/builtin tutorial` |

### 命令行扩展

添加任何支持 MCP 协议的命令行工具：

```bash
# 在会话中添加
/extension npx -y @modelcontextprotocol/server-github

# 或在配置文件中
# ~/.config/goose/config.yaml
extensions:
  - name: github
    type: stdio
    command: npx
    args:
      - "-y"
      - "@modelcontextprotocol/server-github"
    env:
      GITHUB_TOKEN: "your-token"
```

### 远程扩展 (SSE)

连接到远程 MCP 服务器：

```yaml
extensions:
  - name: remote-service
    type: sse
    uri: "https://mcp.example.com/sse"
```

### 常用 MCP 扩展

| 扩展 | 安装命令 | 功能 |
|------|----------|------|
| GitHub | `npx @modelcontextprotocol/server-github` | GitHub 仓库操作 |
| Filesystem | `npx @modelcontextprotocol/server-filesystem` | 文件系统访问 |
| PostgreSQL | `npx @modelcontextprotocol/server-postgres` | 数据库操作 |
| Slack | `npx @modelcontextprotocol/server-slack` | Slack 集成 |

## 支持的 LLM 提供商

### 云服务商

| 提供商 | 支持模型 | 配置 |
|--------|----------|------|
| **OpenAI** | GPT-4o, GPT-4o-mini, o1, o3 | `OPENAI_API_KEY` |
| **Anthropic** | Claude 4 Opus, Claude 4 Sonnet, Claude 3.5 | `ANTHROPIC_API_KEY` |
| **Google** | Gemini 2.5 Pro, Gemini 2.5 Flash | `GOOGLE_API_KEY` |
| **Azure OpenAI** | 所有 Azure 部署的模型 | `AZURE_OPENAI_API_KEY` |
| **AWS Bedrock** | Claude, Llama 等 | AWS 凭证 |
| **OpenRouter** | 100+ 模型 | `OPENROUTER_API_KEY` |

### 本地模型

| 方案 | 说明 | 配置 |
|------|------|------|
| **Ollama** | 本地运行开源模型 | `OLLAMA_HOST` |
| **LM Studio** | 图形化本地模型管理 | OpenAI 兼容 API |

### 配置示例

```bash
# OpenAI
export GOOSE_PROVIDER="openai"
export GOOSE_MODEL="gpt-4o"
export OPENAI_API_KEY="sk-..."

# Anthropic
export GOOSE_PROVIDER="anthropic"
export GOOSE_MODEL="claude-sonnet-4-20250514"
export ANTHROPIC_API_KEY="sk-ant-..."

# Ollama（本地）
export GOOSE_PROVIDER="ollama"
export GOOSE_MODEL="llama3.2"
export OLLAMA_HOST="http://localhost:11434"
```

## 项目结构

```
agime/
├── crates/
│   ├── goose/           # 核心库：Agent、Provider、配置
│   ├── goose-cli/       # 命令行工具
│   ├── goose-server/    # HTTP API 服务器 (goosed)
│   ├── goose-mcp/       # 内置 MCP 扩展
│   ├── goose-bench/     # 基准测试框架
│   └── goose-test/      # 测试工具
│
├── ui/
│   └── desktop/         # Electron 桌面应用
│
└── documentation/       # 文档
```

## 常见问题

### Q: AGIME 和其他 AI 编程助手有什么区别？

AGIME 是一个**智能体**而非简单的代码补全工具。它可以自主规划、执行、验证整个任务流程，而不需要您逐步指导。

### Q: 我的代码会被发送到云端吗？

取决于您选择的 LLM 提供商。如果使用云服务（OpenAI、Anthropic 等），代码会发送到他们的 API。如果需要完全本地运行，可以使用 Ollama 等本地模型方案。

### Q: 如何降低 API 成本？

1. 使用 Lead/Worker 模式，用便宜的模型执行大部分任务
2. 使用 `/compact` 命令压缩对话历史
3. 选择更经济的模型（如 GPT-4o-mini）
4. 考虑使用本地模型

### Q: 支持哪些编程语言？

AGIME 支持所有编程语言。内置的 `developer` 扩展对以下语言有增强的代码分析支持：
- Python, JavaScript/TypeScript, Rust, Go
- Java, Kotlin, Ruby, Swift
- 以及更多...

## 开发与贡献

### 开发环境搭建

```bash
# 克隆仓库
git clone https://github.com/agiemem/agime.git
cd agime

# 安装 Rust（如果尚未安装）
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# 构建所有 crate
cargo build

# 运行测试
cargo test

# 构建桌面应用
cd ui/desktop
npm install
npm run make
```

### 贡献指南

我们欢迎各种形式的贡献：

- 🐛 报告 Bug
- 💡 功能建议
- 📖 文档改进
- 🔧 代码贡献

请参阅 [CONTRIBUTING.md](CONTRIBUTING.md) 了解详情。

## 许可证

本项目基于 [Apache License 2.0](LICENSE) 开源。

## 致谢

AGIME 基于 [Block](https://block.xyz/) 开源的 [goose](https://github.com/block/goose) 项目二次开发。

感谢 Block 团队创建了这个优秀的 AI 智能体框架，以及以下技术和项目：

- [Model Context Protocol (MCP)](https://modelcontextprotocol.io/) - Anthropic
- [Rust](https://www.rust-lang.org/) 编程语言
- [Electron](https://www.electronjs.org/) 桌面应用框架
- 所有 LLM 提供商的 API 服务

---

<p align="center">
  <strong>AGIME</strong> - 让 AI 成为您的自主开发伙伴
</p>

<p align="center">
  由 <a href="https://github.com/agiemem">agiemem</a> 维护
</p>
