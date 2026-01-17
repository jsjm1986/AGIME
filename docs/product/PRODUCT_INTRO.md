# AGIME 产品介绍

**AI + Me** —— 不只是陪你聊天的 AI，而是直接帮你干活的本地 AI 助理。

[下载 v2.5.0](https://github.com/jsjm1986/AGIME/releases)

---

## 核心差异

| 功能 | 传统 AI 服务 | AGIME |
|:-----|:-------------|:------|
| **对话交流** | 支持 | 支持 |
| **读写本地文件** | 无法访问 | **直接读写** |
| **执行系统操作** | 只能建议 | **自动执行** |
| **定时任务** | 不支持 | **自动调度** |
| **数据隐私** | 上传云端 | **100% 本地处理** |
| **记忆能力** | 仅限当前会话 | **长期记忆 + 知识库** |

---

## 核心功能

### 1. 批量文档处理

**场景**：你有 100 个发票 PDF 需要重命名并提取金额到 Excel。

- **传统方式**：手动一个个打开，复制粘贴。耗时 2 小时。
- **AGIME**：*"把这个文件夹里的所有发票重命名为[公司名]_[日期].pdf，并把金额汇总到 excel 表格中"*。耗时 30 秒。

### 2. 自动化工作流

**场景**：每天早上需要检查邮件、整理待办、发送日报。

- **AGIME**：设置定时任务，每天 9:00 自动执行，你上班时日报草稿已在桌面上。

### 3. PC 深度控制

不只是浏览器，AGIME 可以控制你的鼠标键盘，操作任何软件（需授权）。

- *"打开 Photoshop，把这张图调整大小为 1024x1024"*
- *"在 VSCode 中打开当前项目，并运行测试"*

### 4. 团队协作 (v2.5.0)

不仅仅是个人助理，现在支持团队共享。

- **LAN 模式**：局域网内 P2P 直连，分享技能和工作流。
- **Cloud 模式**：跨地域协作，企业级权限管理。

<div class="mermaid">
graph TD
    subgraph LAN[LAN 模式]
        A1[电脑 A] <-->|P2P| A2[电脑 B]
        A2 <-->|P2P| A3[电脑 C]
        style LAN fill:#0f172a,stroke:#3b82f6,stroke-width:1px
    end
    subgraph CLOUD[Cloud 模式]
        S[服务器]
        B1[团队 A] <-->|加密| S
        B2[团队 B] <-->|加密| S
        style CLOUD fill:#0f172a,stroke:#06b6d4,stroke-width:1px
    end
    style S fill:#1e293b,stroke:#fff
</div>

---

## 四种工作模式

### 1. 自主模式 (Autonomous)
**全自动代理**。你给出一个复杂目标，AI 自动拆解步骤、执行操作、直到完成。
- *"帮我调研一下市场上开源的 RAG 框架，写一份对比报告"*

### 2. 智能模式 (Copilot)
**人机协作**。在执行敏感操作（如删除文件、发送邮件）前，AI 会询问你的确认。
- *"帮我清理一下 Downloads 文件夹，把 oversized 的文件删掉"* -> *AI: 发现 5 个超过 1GB 的文件，确认删除吗？*

### 3. 手动模式 (Manual)
**指令执行**。AI 严格按照你定义的步骤执行，不进行任何发散。适合固定的自动化脚本。

### 4. 聊天模式 (Chat)
**纯对话**。如同 ChatGPT，用于咨询问题、润色文本，不执行系统操作。

---

## AI 模型支持

AGIME 采用 **混合模型架构**，确保最佳的性能与隐私平衡。

### 推荐配置

- **主模型 (思考/推理)**: 建议使用 Claude 3.5 Sonnet 或 GPT-4o。（处理复杂逻辑最强）
- **本地模型 (隐私/快速)**: 支持通过 Ollama 调用 Llama 3, Qwen 2.5, DeepSeek Coder。

### 100% 隐私方案

如果你对数据极度敏感，可以配置 **全本地模型**。AGIME 所有功能均可离线运行。

---

## 扩展系统 (MCP)

AGIME 完整支持 **Model Context Protocol (MCP)** 标准。这意味着你可以接入任何支持 MCP 的工具：

- **数据库**: PostgreSQL, SQLite, MySQL
- **开发工具**: GitHub, GitLab, Sentry
- **企业应用**: Slack, Google Drive, Notion

---

## 系统要求

- **OS**: Windows 10/11, macOS 12+, Linux (Ubuntu 20.04+)
- **CPU**: 建议 4 核以上
- **RAM**: 建议 16GB (如果运行本地 LLM)
- **Disk**: 500MB 空间

---

## 常见问题

<details>
<summary>AGIME 是免费的吗？</summary>
是的，AGIME 个人版和团队版 (LAN模式) 完全免费且开源 (Apache 2.0)。
</details>

<details>
<summary>可以完全离线使用吗？</summary>
可以。只要你配置了本地模型 (如通过 Ollama)，AGIME 不需要连网即可工作。
</details>

<details>
<summary>会不会乱删我的文件？</summary>
不会。在"智能模式"下，所有敏感操作都需要你确认。你也可以设置"只读"权限。
</details>
