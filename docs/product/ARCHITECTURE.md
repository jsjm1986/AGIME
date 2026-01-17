# 技术架构

AGIME 采用 **Local-First (本地优先)** 架构，旨在最大化隐私安全、响应速度和离线可用性。

---

## 1. 系统概览

AGIME 的核心是一个高性能的 Rust 守护进程 (`agimed`)，它负责调度所有 AI 任务、管理记忆和控制扩展。

<div class="mermaid">
graph TD
    subgraph UI[前端交互层]
        Desktop[桌面客户端 (Electron)]
        CLI[命令行工具]
    end
    
    subgraph Core[核心引擎 (Rust)]
        Agent[智能体调度器]
        Memory[记忆系统的 (Vector DB)]
        Planner[任务规划器]
    end
    
    subgraph Extensions[扩展层 (MCP)]
        FS[文件系统]
        Browser[浏览器控制]
        Custom[自定义扩展]
    end
    
    UI <-->|gRPC| Core
    Core <-->|JSON-RPC| Extensions
</div>

---

## 2. 核心组件

### 🧠 智能体 (Agent)
负责理解自然语言，拆解任务，并调用工具。
- **ReAct 循环**: 思考 (Reason) -> 行动 (Act) -> 观察 (Observe) 的循环机制。
- **上下文管理**: 自动压缩历史对话，确保不超出 Token 限制。

### 📚 记忆系统 (Memory)
- **Short-term**: 当前会话的上下文。
- **Long-term**: 基于 LanceDB 的向量数据库，存储历史对话和知识库。
- **Semantic Search**: 通过语义搜索快速找回相关记忆。

### 🔌 MCP 客户端
AGIME 内置了一个全功能的 MCP Client。
- **自动发现**: 自动扫描本地安装的 MCP Server。
- **安全沙箱**: 限制每个扩展的权限（如文件读写范围）。

---

## 3. 数据流与隐私

### 100% 本地闭环
在配置本地模型 (Local LLM) 的情况下，AGIME 不需要任何互联网连接。

1. **输入**: 用户输入 "这周的待办事项"。
2. **检索**: Agent 在本地向量库中搜索 "待办", "Todo"。
3. **推理**: 本地 LLM 生成回答。
4. **输出**: 显示结果。

**全程无数据出网。**

### 混合模式 (Hybrid)
对于需要更强推理能力的场景，可以配置云端模型 (如 GPT-4)。
- **脱敏**: 敏感 PII 信息在本地被识别并替换为 `[REDACTED]` 后才发送给云端。
- **加密**: 所有 API 请求强制使用 TLS 1.3。

---

## 4. 技术栈

| 组件 | 技术选型 | 理由 |
|:---|:---|:---|
| **Core** | Rust | 内存安全，极致性能，无 GC 停顿 |
| **GUI** | Electron + React | 跨平台 UI 开发效率高 |
| **Vector DB** | LanceDB | 下一代列式向量库，无需独立服务 |
| **Protocol** | gRPC / MCP | 高效的进程间通信 |
