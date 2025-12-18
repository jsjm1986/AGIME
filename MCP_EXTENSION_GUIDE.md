# Goose MCP 扩展添加机制完整指南

本文档详细说明如何在 Goose 项目中添加 MCP（Model Context Protocol）扩展。

## 目录

- [概述](#概述)
- [外部添加 MCP 扩展的方式](#外部添加-mcp-扩展的方式)
- [Deep Link URL 格式](#deep-link-url-格式)
- [扩展类型详解](#扩展类型详解)
- [添加流程架构](#添加流程架构)
- [核心代码文件](#核心代码文件)
- [数据结构](#数据结构)
- [安全机制](#安全机制)
- [实战示例](#实战示例)
- [API 参考](#api-参考)

---

## 概述

Goose 通过 **Deep Link URL Scheme** (`goose://extension?...`) 来添加 MCP 扩展。这套机制支持多种扩展类型，并具有完善的安全验证流程。

### 支持的扩展类型

| 类型 | 传输方式 | 用途 | 示例 |
|------|---------|------|------|
| **stdio** | 子进程标准 I/O | 本地命令执行 | npx, docker, uvx |
| **sse** | Server-Sent Events | 远程扩展（推送） | MCP 服务器 |
| **streamable_http** | HTTP 流 | 远程扩展（请求-响应） | 带 OAuth 的服务 |
| **builtin** | 内进程 | 内置 MCP 服务 | developer, computercontroller |
| **platform** | 内进程 | 平台扩展 | todo, skills, chatrecall |

---

## 外部添加 MCP 扩展的方式

用户可以通过以下 **5 种方式** 从外部添加 MCP 扩展到 Goose：

### 方式 1：网页 Deep Link（推荐）

在网页或文档中放置一个可点击的安装链接，用户点击后自动打开 Goose 并安装扩展。

**HTML 示例：**
```html
<a href="goose://extension?cmd=npx&arg=-y&arg=@modelcontextprotocol/server-github&name=GitHub&description=GitHub%20integration&env=GITHUB_PERSONAL_ACCESS_TOKEN=Your%20GitHub%20PAT">
  Install GitHub MCP in Goose
</a>
```

**Markdown 示例：**
```markdown
[Install GitHub MCP](goose://extension?cmd=npx&arg=-y&arg=@modelcontextprotocol/server-github&name=GitHub&description=GitHub%20integration)
```

**生成链接的工具函数（TypeScript）：**
```typescript
// documentation/src/utils/install-links.ts
export function getGooseInstallLink(server: MCPServer): string {
  if (server.is_builtin) {
    // 内置扩展
    return `goose://extension?cmd=goosed&arg=mcp&arg=${encodeURIComponent(server.id)}&description=${encodeURIComponent(server.id)}`;
  }

  if (server.url) {
    // 远程 URL 扩展
    const queryParams = [
      ...(server.type === "streamable-http" ? [`type=streamable_http`] : []),
      `url=${encodeURIComponent(server.url)}`,
      `id=${encodeURIComponent(server.id)}`,
      `name=${encodeURIComponent(server.name)}`,
      `description=${encodeURIComponent(server.description)}`,
      ...server.environmentVariables
        .filter((env) => env.required)
        .map((env) => `env=${encodeURIComponent(`${env.name}=${env.description}`)}`)
    ].join("&");
    return `goose://extension?${queryParams}`;
  }

  // Stdio 命令扩展
  const parts = server.command.split(" ");
  const baseCmd = parts[0];
  const args = parts.slice(1);
  const queryParams = [
    `cmd=${encodeURIComponent(baseCmd)}`,
    ...args.map((arg) => `arg=${encodeURIComponent(arg)}`),
    `id=${encodeURIComponent(server.id)}`,
    `name=${encodeURIComponent(server.name)}`,
    `description=${encodeURIComponent(server.description)}`,
    ...server.environmentVariables
      .filter((env) => env.required)
      .map((env) => `env=${encodeURIComponent(`${env.name}=${env.description}`)}`)
  ].join("&");
  return `goose://extension?${queryParams}`;
}
```

---

### 方式 2：CLI 命令行

#### A. 交互式配置
```bash
goose configure
```
在菜单中选择添加扩展类型：
1. **Built-in Extension** - 使用预装的扩展
2. **Command-line Extension** - 运行本地命令（stdio）
3. **Remote Extension (SSE)** - 连接 SSE 远程服务
4. **Remote Extension (Streaming HTTP)** - 连接 Streaming HTTP 远程服务

#### B. 启动会话时添加

**添加内置扩展：**
```bash
goose session --with-builtin "developer,computercontroller"
```

**添加外部命令扩展：**
```bash
goose session --with-extension "npx -y @modelcontextprotocol/server-fetch"
```

**带环境变量：**
```bash
goose session --with-extension "GITHUB_PERSONAL_ACCESS_TOKEN=<token> npx -y @modelcontextprotocol/server-github"
```

**添加远程 SSE 扩展：**
```bash
goose session --with-remote-extension "http://localhost:8080/sse"
```

**添加 Streaming HTTP 扩展：**
```bash
goose session --with-streamable-http-extension "https://example.com/mcp"
```

---

### 方式 3：直接编辑配置文件

编辑 `~/.config/goose/config.yaml`（Linux/macOS）或 `%APPDATA%\Block\goose\config\config.yaml`（Windows）：

```yaml
extensions:
  github:
    enabled: true
    type: stdio
    name: GitHub
    description: GitHub integration
    cmd: npx
    args:
      - "-y"
      - "@modelcontextprotocol/server-github"
    envs:
      GITHUB_PERSONAL_ACCESS_TOKEN: "your-token-here"
    timeout: 300

  my_remote_service:
    enabled: true
    type: sse
    name: My Remote Service
    description: A remote MCP service via SSE
    uri: https://api.example.com/mcp/sse
    timeout: 300

  my_http_service:
    enabled: true
    type: streamable_http
    name: My HTTP Service
    description: A remote MCP service via HTTP
    uri: https://api.example.com/mcp
    headers:
      Authorization: "Bearer ${API_TOKEN}"
    envs:
      API_TOKEN: "your-api-token"
    timeout: 300
```

---

### 方式 4：REST API

通过 Goose Server 的 REST API 添加扩展：

**添加扩展：**
```bash
curl -X POST http://localhost:3000/config/extensions \
  -H "Content-Type: application/json" \
  -d '{
    "name": "GitHub",
    "enabled": true,
    "config": {
      "type": "stdio",
      "name": "GitHub",
      "description": "GitHub integration",
      "cmd": "npx",
      "args": ["-y", "@modelcontextprotocol/server-github"],
      "envs": {
        "GITHUB_PERSONAL_ACCESS_TOKEN": "your-token"
      },
      "timeout": 300
    }
  }'
```

**添加到活跃会话：**
```bash
curl -X POST http://localhost:3000/agent/add_extension \
  -H "Content-Type: application/json" \
  -d '{
    "session_id": "your-session-id",
    "config": {
      "type": "stdio",
      "name": "GitHub",
      "description": "GitHub integration",
      "cmd": "npx",
      "args": ["-y", "@modelcontextprotocol/server-github"],
      "timeout": 300
    }
  }'
```

---

### 方式 5：文档网站扩展目录

访问 Goose 文档网站的扩展目录页面（`/extensions`），浏览所有可用扩展并点击 "Install" 按钮。

**扩展注册表文件：** `documentation/static/servers.json`

```json
{
  "id": "github-mcp",
  "name": "GitHub",
  "description": "GitHub API integration for repos, issues, PRs",
  "command": "npx -y @modelcontextprotocol/server-github",
  "link": "https://github.com/modelcontextprotocol/servers",
  "is_builtin": false,
  "endorsed": true,
  "environmentVariables": [
    {
      "name": "GITHUB_PERSONAL_ACCESS_TOKEN",
      "description": "Your GitHub Personal Access Token",
      "required": true
    }
  ]
}
```

---

### 方式对比

| 方式 | 适用场景 | 优点 | 缺点 |
|------|---------|------|------|
| **Deep Link** | 网页/文档分享 | 一键安装、用户体验好 | 需要 Goose 已安装 |
| **CLI 命令** | 开发者/脚本化 | 灵活、可脚本化 | 需要命令行操作 |
| **配置文件** | 批量配置/备份 | 完全控制、可版本化 | 手动操作、易出错 |
| **REST API** | 程序化集成 | 可编程、自动化 | 需要服务器运行 |
| **文档网站** | 发现新扩展 | 可视化、有描述 | 需要访问网站 |

---

### 为第三方 MCP 创建安装链接

如果你开发了一个 MCP 服务器并想让用户一键安装，按以下格式创建链接：

**NPM 包：**
```
goose://extension?cmd=npx&arg=-y&arg=your-package-name&name=Your%20Extension&description=Your%20description&env=API_KEY=Your%20API%20key%20description
```

**Docker 容器：**
```
goose://extension?cmd=docker&arg=run&arg=-i&arg=--rm&arg=your-image:tag&name=Your%20Extension&description=Your%20description
```

**Python（uvx）：**
```
goose://extension?cmd=uvx&arg=your-python-package&name=Your%20Extension&description=Your%20description
```

**远程 SSE 服务：**
```
goose://extension?url=https://your-server.com/mcp/sse&name=Your%20Extension&description=Your%20description
```

**远程 Streaming HTTP 服务：**
```
goose://extension?url=https://your-server.com/mcp&type=streamable_http&name=Your%20Extension&description=Your%20description&header=Authorization=Bearer%20${TOKEN}&env=TOKEN=Your%20auth%20token
```

---

## Deep Link URL 格式

### 基本结构

```
goose://extension?<参数列表>
```

### 通用参数

| 参数 | 必需 | 说明 |
|------|------|------|
| `name` | ✅ | 扩展显示名称 |
| `id` | 可选 | 扩展唯一标识 |
| `description` | 可选 | 扩展描述 |
| `timeout` | 可选 | 超时时间（秒），默认 300 |
| `env` | 可选 | 环境变量（格式: `KEY=description`，可重复） |
| `installation_notes` | 可选 | 安装说明 |

---

## 扩展类型详解

### 1. Stdio 扩展（本地命令执行）

**URL 格式：**
```
goose://extension?cmd=<命令>&arg=<参数>&name=<名称>&description=<描述>&timeout=<超时>&env=<环境变量>
```

**特有参数：**
| 参数 | 必需 | 说明 |
|------|------|------|
| `cmd` | ✅ | 要执行的命令（必须在白名单中） |
| `arg` | 可选 | 命令参数（可重复多次） |

**允许的命令（白名单）：**
```
cu, docker, jbang, npx, uvx, goosed, npx.cmd, i-ching-mcp-server
```

**示例 - GitHub MCP：**
```
goose://extension?cmd=npx&arg=-y&arg=@modelcontextprotocol/server-github&id=github&name=GitHub&description=GitHub%20integration&env=GITHUB_PERSONAL_ACCESS_TOKEN=Your%20GitHub%20PAT
```

**示例 - Filesystem MCP：**
```
goose://extension?cmd=npx&arg=-y&arg=@modelcontextprotocol/server-filesystem&arg=/path/to/allowed/dir&id=filesystem&name=Filesystem&description=File%20system%20access
```

---

### 2. SSE 扩展（Server-Sent Events）

**URL 格式：**
```
goose://extension?url=<远程URL>&name=<名称>&description=<描述>&timeout=<超时>&env=<环境变量>
```

**特有参数：**
| 参数 | 必需 | 说明 |
|------|------|------|
| `url` | ✅ | 远程 MCP 服务器 URL |

**示例：**
```
goose://extension?url=https://api.example.com/mcp/sse&name=MyService&description=My%20MCP%20Service&timeout=300
```

---

### 3. Streamable HTTP 扩展

**URL 格式：**
```
goose://extension?url=<远程URL>&type=streamable_http&name=<名称>&description=<描述>&header=<HTTP头>&env=<环境变量>
```

**特有参数：**
| 参数 | 必需 | 说明 |
|------|------|------|
| `url` | ✅ | 远程 MCP 服务器 URL |
| `type` 或 `transport` | ✅ | 必须为 `streamable_http` |
| `header` | 可选 | HTTP 头（格式: `KEY=value`，可重复） |

**示例 - 带认证的服务：**
```
goose://extension?url=https://api.example.com/mcp&type=streamable_http&name=OAuth%20Service&header=Authorization=Bearer%20${API_TOKEN}&env=API_TOKEN=Your%20API%20token
```

---

### 4. 内置扩展

**URL 格式：**
```
goose://extension?cmd=goosed&arg=mcp&arg=<扩展ID>&description=<描述>
```

**可用的内置扩展：**
- `developer` - 开发工具
- `computercontroller` - 系统自动化
- `autovisualiser` - 数据可视化
- `memory` - 内存管理
- `tutorial` - 教程系统

**示例：**
```
goose://extension?cmd=goosed&arg=mcp&arg=developer&description=developer
```

---

## 添加流程架构

```
┌─────────────────────────────────────────────────────────────────────────────┐
│  1. 用户点击 goose://extension?... 链接                                       │
└─────────────────────────────┬───────────────────────────────────────────────┘
                              ▼
┌─────────────────────────────────────────────────────────────────────────────┐
│  2. 操作系统/浏览器触发 Goose 协议处理                                          │
│     main.ts: app.setAsDefaultProtocolClient('goose')                        │
└─────────────────────────────┬───────────────────────────────────────────────┘
                              ▼
┌─────────────────────────────────────────────────────────────────────────────┐
│  3. Electron 主进程处理                                                       │
│     main.ts: handleProtocolUrl(url)                                         │
│     → window.webContents.send('add-extension', pendingDeepLink)             │
└─────────────────────────────┬───────────────────────────────────────────────┘
                              ▼
┌─────────────────────────────────────────────────────────────────────────────┐
│  4. 渲染进程监听 IPC 事件                                                      │
│     ExtensionInstallModal.tsx:                                               │
│     window.electron.on('add-extension', handleAddExtension)                  │
└─────────────────────────────┬───────────────────────────────────────────────┘
                              ▼
┌─────────────────────────────────────────────────────────────────────────────┐
│  5. 安全检查 & 权限确认                                                        │
│     determineModalType() → trusted | untrusted | blocked                    │
└─────────────────────────────┬───────────────────────────────────────────────┘
                              ▼
┌─────────────────────────────────────────────────────────────────────────────┐
│  6. 解析 Deep Link & 构建配置                                                  │
│     deeplink.ts: addExtensionFromDeepLink()                                 │
│     → 验证协议、字段、命令白名单                                                 │
│     → 构建 ExtensionConfig 对象                                               │
└─────────────────────────────┬───────────────────────────────────────────────┘
                              ▼
                    ┌─────────┴─────────┐
                    ▼                   ▼
┌─────────────────────────┐   ┌─────────────────────────────────────┐
│  需要配置环境变量/头部    │   │  无需配置                            │
│  → 跳转到设置页面         │   │  → 直接保存                          │
└─────────────────────────┘   └──────────────┬──────────────────────┘
                                              ▼
┌─────────────────────────────────────────────────────────────────────────────┐
│  7. 调用后端 API 保存配置                                                      │
│     POST /config/extensions { name, config, enabled }                       │
│     → 保存到 config.yaml 的 extensions 字段                                   │
└─────────────────────────────┬───────────────────────────────────────────────┘
                              ▼
┌─────────────────────────────────────────────────────────────────────────────┐
│  8. （如果有活跃会话）添加到运行时 Agent                                         │
│     POST /agent/add_extension { session_id, config }                        │
│     → 根据类型创建 MCP 客户端连接                                               │
└─────────────────────────────────────────────────────────────────────────────┘
```

---

## 核心代码文件

| 功能 | 文件路径 |
|------|---------|
| **Deep Link 解析** | `ui/desktop/src/components/settings/extensions/deeplink.ts` |
| **安装确认模态框** | `ui/desktop/src/components/ExtensionInstallModal.tsx` |
| **URL 生成工具** | `documentation/src/utils/install-links.ts` |
| **Electron 协议处理** | `ui/desktop/src/main.ts` |
| **扩展配置数据结构** | `crates/goose/src/agents/extension.rs` |
| **配置持久化** | `crates/goose/src/config/extensions.rs` |
| **配置 API 端点** | `crates/goose-server/src/routes/config_management.rs` |
| **Agent 运行时 API** | `crates/goose-server/src/routes/agent.rs` |
| **扩展管理器** | `crates/goose/src/agents/extension_manager.rs` |

---

## 数据结构

### ExtensionConfig (Rust)

```rust
// crates/goose/src/agents/extension.rs
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "type")]
pub enum ExtensionConfig {
    #[serde(rename = "stdio")]
    Stdio {
        name: String,
        description: String,
        cmd: String,
        args: Vec<String>,
        envs: Envs,              // 环境变量 { "KEY": "value" }
        env_keys: Vec<String>,   // 从密钥存储获取的环境变量键
        timeout: Option<u64>,
        bundled: Option<bool>,
        available_tools: Vec<String>,
    },

    #[serde(rename = "sse")]
    Sse {
        name: String,
        description: String,
        uri: String,
        envs: Envs,
        env_keys: Vec<String>,
        timeout: Option<u64>,
        bundled: Option<bool>,
        available_tools: Vec<String>,
    },

    #[serde(rename = "streamable_http")]
    StreamableHttp {
        name: String,
        description: String,
        uri: String,
        envs: Envs,
        env_keys: Vec<String>,
        headers: HashMap<String, String>,  // 支持 ${VAR} 环境变量替换
        timeout: Option<u64>,
        bundled: Option<bool>,
        available_tools: Vec<String>,
    },

    #[serde(rename = "builtin")]
    Builtin {
        name: String,
        description: String,
        display_name: Option<String>,
        timeout: Option<u64>,
        bundled: Option<bool>,
        available_tools: Vec<String>,
    },

    #[serde(rename = "platform")]
    Platform {
        name: String,
        description: String,
        bundled: Option<bool>,
        available_tools: Vec<String>,
    },

    #[serde(rename = "frontend")]
    Frontend {
        name: String,
        description: String,
        tools: Vec<Tool>,
        instructions: Option<String>,
        bundled: Option<bool>,
        available_tools: Vec<String>,
    },

    #[serde(rename = "inline_python")]
    InlinePython {
        name: String,
        description: String,
        code: String,
        timeout: Option<u64>,
        dependencies: Option<Vec<String>>,
        available_tools: Vec<String>,
    },
}
```

### ExtensionConfig (TypeScript)

```typescript
// ui/desktop/src/api/types.gen.ts
export type ExtensionConfig = {
  type: 'stdio';
  name: string;
  description: string;
  cmd: string;
  args?: Array<string>;
  envs?: { [key: string]: string };
  env_keys?: Array<string>;
  timeout?: number;
  bundled?: boolean;
  available_tools?: Array<string>;
} | {
  type: 'sse';
  name: string;
  description: string;
  uri: string;
  envs?: { [key: string]: string };
  env_keys?: Array<string>;
  timeout?: number;
  bundled?: boolean;
  available_tools?: Array<string>;
} | {
  type: 'streamable_http';
  name: string;
  description: string;
  uri: string;
  envs?: { [key: string]: string };
  env_keys?: Array<string>;
  headers?: { [key: string]: string };
  timeout?: number;
  bundled?: boolean;
  available_tools?: Array<string>;
} | // ... 其他类型
```

### 配置存储格式 (config.yaml)

```yaml
extensions:
  github:
    enabled: true
    type: stdio
    name: GitHub
    description: GitHub integration
    cmd: npx
    args:
      - "-y"
      - "@modelcontextprotocol/server-github"
    envs:
      GITHUB_PERSONAL_ACCESS_TOKEN: "your-token-here"
    timeout: 300

  my_remote_service:
    enabled: true
    type: streamable_http
    name: My Remote Service
    description: A remote MCP service
    uri: https://api.example.com/mcp
    headers:
      Authorization: "Bearer ${API_TOKEN}"
    envs:
      API_TOKEN: "your-api-token"
    timeout: 300
```

---

## 安全机制

### 1. 命令白名单

只允许执行以下命令：

```typescript
// ui/desktop/src/components/settings/extensions/deeplink.ts
const allowedCommands = [
  'cu',
  'docker',
  'jbang',
  'npx',
  'uvx',
  'goosed',
  'npx.cmd',
  'i-ching-mcp-server',
];
```

### 2. 危险参数检测

```typescript
// 阻止 npx -c 命令注入
if (cmd === 'npx' && args.includes('-c')) {
  throw new Error('npx with -c argument can lead to code injection');
}
```

### 3. 环境变量限制

以下 31 个危险环境变量被禁止设置：

```rust
// crates/goose/src/agents/extension.rs
const DISALLOWED_KEYS: [&'static str; 31] = [
    // 二进制路径操纵
    "PATH", "PATHEXT", "SystemRoot", "windir",
    // 动态链接器劫持 (Linux/macOS)
    "LD_LIBRARY_PATH", "LD_PRELOAD", "LD_AUDIT", "LD_DEBUG",
    "LD_BIND_NOW", "LD_ASSUME_KERNEL",
    // macOS 动态链接器
    "DYLD_LIBRARY_PATH", "DYLD_INSERT_LIBRARIES", "DYLD_FRAMEWORK_PATH",
    // 语言特定劫持
    "PYTHONPATH", "PYTHONHOME", "NODE_OPTIONS", "RUBYOPT",
    "GEM_PATH", "GEM_HOME", "CLASSPATH", "GO111MODULE", "GOROOT",
    // Windows DLL 劫持
    "APPINIT_DLLS", "SESSIONNAME", "ComSpec",
    "TEMP", "TMP", "LOCALAPPDATA", "USERPROFILE", "HOMEDRIVE", "HOMEPATH",
];
```

### 4. 权限确认模态框

三级安全模式：
- **trusted**: 已授权的扩展，显示确认对话框
- **untrusted**: 需要用户明确确认的未受信扩展
- **blocked**: 被白名单阻止的扩展，不允许安装

### 5. 外部白名单（可选）

可通过 `GOOSE_ALLOWLIST` 环境变量配置外部白名单 URL：

```yaml
# 白名单 YAML 格式
extensions:
  - id: github
    command: npx -y @modelcontextprotocol/server-github
  - id: filesystem
    command: npx -y @modelcontextprotocol/server-filesystem
```

---

## 实战示例

### 示例 1：添加 GitHub MCP

```
goose://extension?cmd=npx&arg=-y&arg=@modelcontextprotocol/server-github&id=github&name=GitHub&description=GitHub%20API%20integration%20for%20repos%2C%20issues%2C%20PRs&env=GITHUB_PERSONAL_ACCESS_TOKEN=Your%20GitHub%20Personal%20Access%20Token
```

### 示例 2：添加 Slack MCP

```
goose://extension?cmd=npx&arg=-y&arg=@modelcontextprotocol/server-slack&id=slack&name=Slack&description=Slack%20workspace%20integration&env=SLACK_BOT_TOKEN=Your%20Slack%20Bot%20Token&env=SLACK_TEAM_ID=Your%20Slack%20Team%20ID
```

### 示例 3：添加远程 MCP 服务

```
goose://extension?url=https://mcp.example.com/api&type=streamable_http&name=Custom%20Service&description=My%20custom%20MCP%20service&header=Authorization=Bearer%20${SERVICE_TOKEN}&env=SERVICE_TOKEN=Your%20service%20authentication%20token
```

### 示例 4：添加 Docker 容器中的 MCP

```
goose://extension?cmd=docker&arg=run&arg=-i&arg=--rm&arg=mcp/my-server&id=docker-mcp&name=Docker%20MCP&description=MCP%20server%20in%20Docker
```

### 示例 5：使用 uvx（Python）

```
goose://extension?cmd=uvx&arg=my-python-mcp-server&id=python-mcp&name=Python%20MCP&description=Python-based%20MCP%20server&env=OPENAI_API_KEY=Your%20OpenAI%20API%20key
```

---

## API 参考

### 配置管理 API

#### 获取所有扩展
```http
GET /config/extensions
```

**响应：**
```json
{
  "extensions": [
    {
      "enabled": true,
      "type": "stdio",
      "name": "GitHub",
      "cmd": "npx",
      "args": ["-y", "@modelcontextprotocol/server-github"],
      ...
    }
  ]
}
```

#### 添加/更新扩展
```http
POST /config/extensions
Content-Type: application/json

{
  "name": "GitHub",
  "enabled": true,
  "config": {
    "type": "stdio",
    "name": "GitHub",
    "description": "GitHub integration",
    "cmd": "npx",
    "args": ["-y", "@modelcontextprotocol/server-github"],
    "envs": {
      "GITHUB_PERSONAL_ACCESS_TOKEN": "your-token"
    },
    "timeout": 300
  }
}
```

#### 删除扩展
```http
DELETE /config/extensions/{name}
```

### Agent 运行时 API

#### 添加扩展到活跃会话
```http
POST /agent/add_extension
Content-Type: application/json

{
  "session_id": "session-uuid",
  "config": {
    "type": "stdio",
    "name": "GitHub",
    ...
  }
}
```

#### 从会话移除扩展
```http
POST /agent/remove_extension
Content-Type: application/json

{
  "session_id": "session-uuid",
  "name": "GitHub"
}
```

---

## 常见问题

### Q: 如何添加自定义命令到白名单？

修改 `ui/desktop/src/components/settings/extensions/deeplink.ts` 中的 `allowedCommands` 数组：

```typescript
const allowedCommands = [
  'cu', 'docker', 'jbang', 'npx', 'uvx', 'goosed', 'npx.cmd',
  'i-ching-mcp-server',
  'my-custom-command',  // 添加自定义命令
];
```

### Q: 如何支持新的传输类型？

1. 在 `crates/goose/src/agents/extension.rs` 中添加新的 `ExtensionConfig` 变体
2. 在 `crates/goose/src/agents/extension_manager.rs` 的 `add_extension` 方法中处理新类型
3. 在 `ui/desktop/src/components/settings/extensions/deeplink.ts` 中添加解析逻辑

### Q: 环境变量如何安全存储？

敏感环境变量通过 `env_keys` 字段指定，系统会从安全的密钥存储中获取值，而不是直接存储在配置文件中。

---

## 更新日志

- **2024-12**: 初始版本，支持 stdio、sse、streamable_http 类型
