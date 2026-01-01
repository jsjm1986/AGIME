# AGIME 项目编译指南

## 项目概述

AGIME 是一个 AI Agent 框架，基于 Rust 构建，使用 Electron + React 前端。

- **版本**: 2.1.1-beta
- **rmcp 版本**: 0.12.0
- **Rust Edition**: 2021

## Crate 结构与依赖关系

### Crate 类型一览

| Crate | 类型 | 产物 | 说明 |
|-------|------|------|------|
| `agime` | lib | libagime.rlib | 核心库：AI Agent 逻辑、Provider、扩展管理 |
| `agime-mcp` | lib | libagime_mcp.rlib | MCP 扩展库：开发者工具、计算机控制、语法解析 |
| `agime-cli` | bin | **agime.exe** | CLI 工具：命令行交互界面 |
| `agime-server` | bin | **agimed.exe** | HTTP 服务器：为前端提供 API |
| `agime-server` | bin | generate_schema.exe | 工具：生成 OpenAPI schema 到 ui/desktop/ |
| `agime-bench` | lib | libagime_bench.rlib | 性能测试库 |
| `agime-test` | bin | capture.exe | 测试工具：MCP stdio 录制/回放 |

### 依赖关系图

```
┌─────────────────────────────────────────────────────────────────┐
│                        二进制产物层                              │
├─────────────────────────────────────────────────────────────────┤
│                                                                 │
│  ┌─────────────────┐    ┌─────────────────┐    ┌─────────────┐ │
│  │   agime-cli     │    │  agime-server   │    │ agime-test  │ │
│  │   (bin crate)   │    │   (bin crate)   │    │ (bin crate) │ │
│  │                 │    │                 │    │             │ │
│  │ → agime.exe     │    │ → agimed.exe    │    │ → capture   │ │
│  │                 │    │ → generate_     │    │     .exe    │ │
│  │                 │    │   schema.exe    │    │             │ │
│  └────────┬────────┘    └────────┬────────┘    └─────────────┘ │
│           │                      │                   │         │
│           │ 依赖                 │ 依赖              │ 独立    │
│           ▼                      ▼                   ▼         │
├─────────────────────────────────────────────────────────────────┤
│                          库层                                   │
├─────────────────────────────────────────────────────────────────┤
│                                                                 │
│  ┌─────────────────┐    ┌─────────────────┐                    │
│  │   agime-bench   │    │    agime-mcp    │                    │
│  │   (lib crate)   │    │   (lib crate)   │                    │
│  │                 │    │                 │                    │
│  │ 性能测试库      │    │ MCP 扩展库      │                    │
│  │ - 基准测试      │    │ - 开发者工具    │                    │
│  │                 │    │ - 计算机控制    │                    │
│  │                 │    │ - 语法解析      │                    │
│  └────────┬────────┘    └────────┬────────┘                    │
│           │                      │                              │
│           │ 依赖                 │ 依赖                         │
│           ▼                      ▼                              │
├─────────────────────────────────────────────────────────────────┤
│                         核心层                                  │
├─────────────────────────────────────────────────────────────────┤
│                                                                 │
│                    ┌─────────────────┐                         │
│                    │      agime      │                         │
│                    │   (lib crate)   │                         │
│                    │                 │                         │
│                    │ 核心库          │                         │
│                    │ - Agent 逻辑    │                         │
│                    │ - Provider 接口 │                         │
│                    │ - 扩展管理      │                         │
│                    │ - 配置系统      │                         │
│                    └─────────────────┘                         │
│                                                                 │
└─────────────────────────────────────────────────────────────────┘
```

### 内部依赖矩阵

| 被依赖 ↓ / 依赖者 → | agime-cli | agime-server | agime-mcp | agime-bench | agime-test |
|---------------------|:---------:|:------------:|:---------:|:-----------:|:----------:|
| **agime**           | ✓         | ✓            | ✓         | ✓           | -          |
| **agime-mcp**       | ✓         | ✓            | -         | -           | -          |
| **agime-bench**     | ✓         | -            | -         | -           | -          |

### 编译顺序

Cargo 会自动处理依赖顺序，逻辑顺序如下：

```
阶段 1: 核心库 (无本地依赖)
    ┌─────────┐
    │  agime  │  ← 首先编译，所有其他 crate 依赖它
    └────┬────┘
         │
阶段 2: 扩展库 (可并行编译)
    ┌────┴────┐
    ▼         ▼
┌─────────┐ ┌───────────┐
│agime-mcp│ │agime-bench│  ← 都只依赖 agime，可并行
└────┬────┘ └─────┬─────┘
     │            │
阶段 3: 二进制 (可并行编译)
     │            │
     ▼            ▼
┌─────────────────────────┐  ┌─────────────┐
│       agime-cli         │  │ agime-test  │
│ (依赖 agime, mcp, bench)│  │   (独立)    │
└─────────────────────────┘  └─────────────┘
           │
           ▼
┌─────────────────────────┐
│      agime-server       │
│   (依赖 agime, mcp)     │
└─────────────────────────┘
```

### 推荐编译命令

```batch
:: 完整编译 (推荐)
cargo build --release --workspace -j 4

:: 仅编译主要二进制 (更快)
cargo build --release -p agime-cli -p agime-server -j 4

:: 仅编译 CLI
cargo build --release -p agime-cli -j 4

:: 仅编译服务器
cargo build --release -p agime-server -j 4
```

### 主要二进制文件

| 文件 | 大小 | 来源 | 用途 |
|------|------|------|------|
| **agime.exe** | ~107 MB | agime-cli | CLI 命令行工具，用户主要交互入口 |
| **agimed.exe** | ~94 MB | agime-server | HTTP 服务器，为 Electron 前端提供 API |
| generate_schema.exe | ~5 MB | agime-server | 生成 OpenAPI schema 到 ui/desktop/openapi.json |
| capture.exe | ~1 MB | agime-test | MCP stdio 协议录制/回放测试工具 |

### 关键外部依赖

| 依赖 | 版本 | 使用者 | 用途 |
|------|------|--------|------|
| rmcp | 0.12.0 | agime, agime-mcp, agime-cli, agime-server | MCP 协议客户端/服务端 |
| tree-sitter | 0.21 | agime-mcp | 代码语法解析基础库 |
| tree-sitter-kotlin | 0.3.8 | agime-mcp | Kotlin 语法解析 |
| devgen-tree-sitter-swift | 0.21.0 | agime-mcp | Swift 语法解析 |
| tree-sitter-{python,rust,go,java,javascript,ruby} | 0.21.x | agime-mcp | 各语言语法解析 |
| aws-lc-sys | 0.30.0 | (间接依赖) | AWS 加密库 |
| reqwest | 0.11/0.12 | 多个 crate | HTTP 客户端 |
| axum | 0.8.1 | agime-server | Web 框架 |
| tokio | 1.43 | 所有 crate | 异步运行时 |

### rmcp 特性配置

各 crate 的 rmcp 特性配置：

| Crate | rmcp 特性 | 说明 |
|-------|-----------|------|
| **agime** | `client`, `reqwest`, `transport-child-process`, `transport-streamable-http-client`, `transport-streamable-http-client-reqwest` | 完整客户端功能 |
| **agime-mcp** | `server`, `client`, `transport-io`, `macros` | 服务端 + 客户端 |
| **agime-cli** | *(workspace 默认)* | 继承 `schemars`, `auth` |
| **agime-server** | *(workspace 默认)* | 继承 `schemars`, `auth` |
| **agime-bench** | *(workspace 默认)* | 继承 `schemars`, `auth` |

**Workspace 共享配置** (`Cargo.toml`):
```toml
[workspace.dependencies]
rmcp = { version = "0.12.0", features = ["schemars", "auth"] }
```

## 前后端集成

### 技术栈

| 层 | 技术 | 版本 |
|----|------|------|
| **前端框架** | React + TypeScript | 19.2.0 / 5.9.3 |
| **桌面框架** | Electron + Vite | Forge 7.10.2 / Vite 7.2.6 |
| **后端服务** | agimed.exe (Rust/Axum) | 0.8.1 |
| **API 规范** | OpenAPI 3.0 | 自动生成 |
| **API 客户端** | @hey-api/client-fetch | 类型安全 |

### 前后端通信架构

```
┌──────────────────────────────────────────────────────────────┐
│                     Electron 应用                            │
├──────────────────────────────────────────────────────────────┤
│                                                              │
│  ┌─────────────────────┐      IPC       ┌────────────────┐  │
│  │   Renderer Process  │◄──────────────►│  Main Process  │  │
│  │                     │                │                │  │
│  │  React App          │                │  - 窗口管理    │  │
│  │  - ChatProvider     │                │  - 进程管理    │  │
│  │  - API Service      │                │  - agimed 启动 │  │
│  │                     │                │                │  │
│  └──────────┬──────────┘                └───────┬────────┘  │
│             │                                   │            │
│             │ HTTP/WebSocket                    │ spawn      │
│             │ (127.0.0.1:port)                  │            │
│             ▼                                   ▼            │
│  ┌──────────────────────────────────────────────────────────┐│
│  │                     agimed.exe                           ││
│  │                                                          ││
│  │  ┌──────────────┐  ┌───────────────┐  ┌──────────────┐  ││
│  │  │  REST API    │  │   WebSocket   │  │  MCP Server  │  ││
│  │  │  /agent/*    │  │   /ws         │  │              │  ││
│  │  └──────────────┘  └───────────────┘  └──────────────┘  ││
│  │                                                          ││
│  │  ┌──────────────────────────────────────────────────┐   ││
│  │  │              agime (核心库)                       │   ││
│  │  │  - Agent 逻辑  - Provider 接口  - 扩展管理        │   ││
│  │  └──────────────────────────────────────────────────┘   ││
│  └──────────────────────────────────────────────────────────┘│
│                                                              │
└──────────────────────────────────────────────────────────────┘
```

### 二进制文件位置

| 阶段 | 位置 | 说明 |
|------|------|------|
| **编译输出** | `target/release/` | Cargo 编译产物 |
| **开发运行** | `ui/desktop/src/bin/` | 前端开发时查找 |
| **打包发布** | `resources/bin/` | Electron ASAR 内 |

### 启动流程

```
1. npm run start-gui
   │
   ▼
2. Electron Main Process 启动
   │
   ▼
3. getAgimedBinaryPath() 查找二进制
   │  优先级: src/bin/ → bin/ → target/debug/ → target/release/
   │
   ▼
4. findAvailablePort() 获取可用端口
   │
   ▼
5. spawn agimed.exe --agent
   │  环境变量: AGIME_PORT, AGIME_SERVER__SECRET_KEY
   │
   ▼
6. checkServerStatus() 轮询 /status 端点 (最多 10 秒)
   │
   ▼
7. 创建 BrowserWindow 加载 React 应用
   │
   ▼
8. React 通过 Fetch API 与 agimed 通信
```

### API 代码生成

```bash
# 1. 后端生成 OpenAPI schema
cargo run --release -p agime-server --bin generate_schema

# 2. 前端生成 TypeScript 客户端
cd ui/desktop
npm run generate-api

# 生成文件:
#   ui/desktop/openapi.json      ← 后端输出
#   ui/desktop/src/api/          ← 前端客户端代码
#     ├── client.gen.ts
#     ├── sdk.gen.ts
#     └── types.gen.ts
```

## Windows 编译环境

### 必需工具

1. **Visual Studio 2022** (Build Tools)
   - 路径: `E:\vs`
   - 需要组件: MSVC v14.44+, Windows SDK 10.0.26100.0

2. **Ninja** (构建系统)
   - 路径: `E:\vs\Common7\IDE\CommonExtensions\Microsoft\CMake\Ninja`

3. **CMake** (3.x+)
   - 路径: `.devtools\cmake\bin`

4. **NASM** (汇编器)
   - 路径: `.devtools\nasm`
   - 用于: aws-lc-sys 编译

5. **Rust** (stable)
   - 路径: `.devtools\rust`

### 环境变量

```batch
set CMAKE_GENERATOR=Ninja
set CC=cl.exe
set CXX=cl.exe
set AWS_LC_SYS_PREBUILT_NASM=1
```

## 编译步骤

### 方法一：使用 full-build.bat (推荐)

```batch
cd E:\yw\agiatme\goose
full-build.bat
```

脚本会自动：
1. 加载 Visual Studio 环境
2. 设置 PATH 和环境变量
3. 清理 aws-lc-sys 和 ring 缓存
4. 使用 `-j 4` 并行编译 (适合 12GB 内存)
5. 复制 agime.exe 和 agimed.exe 到 `ui/desktop/src/bin/`

### 方法二：手动编译

```batch
:: 1. 加载 VS 环境
call "E:\vs\VC\Auxiliary\Build\vcvars64.bat"

:: 2. 设置环境变量
set CMAKE_GENERATOR=Ninja
set PATH=E:\vs\Common7\IDE\CommonExtensions\Microsoft\CMake\Ninja;%PATH%

:: 3. 编译全部
cargo build --release --workspace -j 4

:: 4. 或者只编译主要二进制
cargo build --release -p agime-cli -p agime-server -j 4
```

## 常见问题

### 1. tree-sitter-kotlin 权限拒绝 (os error 5)

**原因**: Windows Defender 拦截了构建脚本

**解决方案**:
- 方法 A: 临时关闭 Windows Defender 实时保护
- 方法 B: 添加排除项 (需管理员权限)
  ```powershell
  Add-MpPreference -ExclusionPath "E:\yw\agiatme\goose\target"
  ```

**清理缓存**:
```bash
rm -rf target/release/build/tree-sitter-kotlin*
rm -rf target/release/build/devgen-tree-sitter-swift*
```

### 2. CMAKE_GENERATOR 不匹配

**错误**: `Does not match the generator used previously: Ninja`

**解决方案**: 清理 CMake 缓存
```bash
rm -rf target/release/build/aws-lc-sys*
rm -rf target/release/build/ring*
```

### 3. 链接错误 LNK2019 (tree_sitter_swift)

**原因**: devgen-tree-sitter-swift 构建不完整

**解决方案**: 清理并重新编译
```bash
rm -rf target/release/build/devgen-tree-sitter-swift*
cargo build --release --workspace -j 4
```

### 4. 内存不足

**解决方案**: 降低并行度
```batch
cargo build --release --workspace -j 2
```

## rmcp 0.12.0 API 变化

从 rmcp 0.9.x 升级到 0.12.0 需要注意：

### 1. SseClientTransport 已移除

**旧代码**:
```rust
use rmcp::transport::SseClientTransport;
let transport = SseClientTransport::start(url).await?;
```

**新代码**:
```rust
use rmcp::transport::StreamableHttpClientTransport;
let transport = StreamableHttpClientTransport::from_uri(url);
```

### 2. transport-sse 功能已移除

**旧 Cargo.toml**:
```toml
rmcp = { version = "0.9", features = ["transport-sse"] }
```

**新 Cargo.toml**:
```toml
rmcp = { version = "0.12", features = [
    "transport-streamable-http-client",
    "transport-streamable-http-client-reqwest",
] }
```

### 3. 分页结果需要 meta 字段

`ListResourcesResult`, `ListToolsResult`, `ListPromptsResult` 等现在需要 `meta` 字段：

```rust
Ok(ListResourcesResult {
    resources,
    next_cursor: None,
    meta: None,  // 新增必填字段
})
```

## 开发模式

编译完成后，运行前端开发服务器：

```bash
cd ui/desktop
npm install
npm run start-gui
```

## 目录结构

```
agime/
├── crates/
│   ├── agime/                  # 核心库
│   │   └── src/
│   │       ├── agents/         # Agent 实现、扩展管理
│   │       ├── providers/      # AI Provider (Anthropic, OpenAI, etc.)
│   │       └── ...
│   ├── agime-cli/              # CLI 工具 → agime.exe
│   │   └── src/main.rs
│   ├── agime-server/           # HTTP 服务器 → agimed.exe, generate_schema.exe
│   │   └── src/
│   │       ├── main.rs
│   │       └── bin/generate_schema.rs
│   ├── agime-mcp/              # MCP 扩展库 (开发者工具、计算机控制)
│   │   └── src/
│   │       ├── developer/      # 开发者工具 (shell, 文件操作等)
│   │       └── computercontroller/  # 计算机控制
│   ├── agime-bench/            # 性能测试库
│   └── agime-test/             # 测试工具 → capture.exe
├── ui/desktop/                 # Electron + React 前端
│   └── src/bin/                # 编译后的二进制文件
├── .devtools/                  # 本地开发工具 (Rust, CMake, NASM)
├── full-build.bat              # 完整编译脚本
└── BUILD-GUIDE.md              # 本文档
```

## 更新日志

- **2026-01-01**: 全面验证项目结构，添加详细依赖图、rmcp 配置、前后端集成文档
- **2024-12-31**: 升级 rmcp 到 0.12.0，修复 API 兼容性问题
- **2024-12-31**: 整理编译文档，清理临时脚本
