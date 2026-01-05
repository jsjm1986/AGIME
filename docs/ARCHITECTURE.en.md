# AGIME Technical Architecture

> This document is for developers and tech enthusiasts, providing detailed information about AGIME's technical architecture and design principles.

## Table of Contents

- [Overall Architecture](#overall-architecture)
- [Core Components](#core-components)
- [Data Flow](#data-flow)
- [Security Model](#security-model)
- [Extension Mechanism](#extension-mechanism)
- [Cross-Platform Support](#cross-platform-support)

---

## Overall Architecture

AGIME uses a layered architecture design, separating concerns to ensure maintainability and extensibility.

```
┌─────────────────────────────────────────────────────────┐
│                    User Interface Layer                  │
│              (Electron + React + TypeScript)            │
├─────────────────────────────────────────────────────────┤
│                   Application Service Layer              │
│                      (Rust Backend)                      │
│  ┌──────────┐ ┌──────────┐ ┌──────────┐ ┌──────────┐   │
│  │ Session  │ │  Recipe  │ │Scheduler │ │  Memory  │   │
│  │ Manager  │ │  Engine  │ │  System  │ │  System  │   │
│  └──────────┘ └──────────┘ └──────────┘ └──────────┘   │
├─────────────────────────────────────────────────────────┤
│                       AI Core Layer                      │
│  ┌──────────────────────────────────────────────────┐   │
│  │              Agent Execution Engine               │   │
│  │  ┌─────────┐ ┌─────────┐ ┌─────────┐            │   │
│  │  │ Planner │ │Executor │ │Observer │            │   │
│  │  └─────────┘ └─────────┘ └─────────┘            │   │
│  └──────────────────────────────────────────────────┘   │
├─────────────────────────────────────────────────────────┤
│                    Model Adapter Layer                   │
│  ┌──────────┐ ┌──────────┐ ┌──────────┐ ┌──────────┐   │
│  │ OpenAI   │ │Anthropic │ │ Chinese  │ │  Ollama  │   │
│  │Compatible│ │Compatible│ │  Models  │ │  Local   │   │
│  └──────────┘ └──────────┘ └──────────┘ └──────────┘   │
├─────────────────────────────────────────────────────────┤
│                   Tool Execution Layer                   │
│                     (MCP Protocol)                       │
│  ┌──────────┐ ┌──────────┐ ┌──────────┐ ┌──────────┐   │
│  │Developer │ │Computer  │ │Playwright│ │  Memory  │   │
│  │  Tools   │ │Controller│ │ Browser  │ │ Storage  │   │
│  └──────────┘ └──────────┘ └──────────┘ └──────────┘   │
└─────────────────────────────────────────────────────────┘
```

---

## Core Components

### 1. User Interface Layer

**Tech Stack**: Electron + React + TypeScript + Tailwind CSS

- **Main Window**: Chat interface, message rendering, input handling
- **Sidebar**: Session management, recipe list, settings entry
- **Settings Panel**: Model config, extension management, preferences
- **System Tray**: Background running, quick actions

### 2. Application Service Layer

**Tech Stack**: Rust + Axum

- **Session Manager**: Manages conversation context, history, state persistence
- **Recipe Engine**: Parses, stores, executes reusable workflows
- **Scheduler System**: Cron expression parsing, scheduled task triggering, background execution
- **Memory System**: User preference storage, context memory, knowledge graph

### 3. AI Core Layer

The Agent Execution Engine is AGIME's core, using the ReAct (Reasoning + Acting) pattern:

```
┌─────────────────────────────────────────┐
│              Agent Loop                  │
│                                         │
│   ┌─────────┐                          │
│   │  User   │                          │
│   │  Input  │                          │
│   └────┬────┘                          │
│        ▼                               │
│   ┌─────────┐                          │
│   │  Think  │ ◄─────────────┐         │
│   │(Reason) │               │         │
│   └────┬────┘               │         │
│        ▼                    │         │
│   ┌─────────┐               │         │
│   │   Act   │               │         │
│   │         │               │         │
│   └────┬────┘               │         │
│        ▼                    │         │
│   ┌─────────┐               │         │
│   │ Observe │───────────────┘         │
│   │         │                          │
│   └────┬────┘                          │
│        ▼                               │
│   ┌─────────┐                          │
│   │Complete/│                          │
│   │Continue │                          │
│   └─────────┘                          │
└─────────────────────────────────────────┘
```

### 4. Model Adapter Layer

Unified model interface abstraction supporting multiple backends:

```rust
trait ModelProvider {
    async fn chat(&self, messages: Vec<Message>) -> Result<Response>;
    async fn stream(&self, messages: Vec<Message>) -> Result<Stream>;
    fn supports_tools(&self) -> bool;
    fn supports_vision(&self) -> bool;
}
```

**Supported Protocols**:
- OpenAI API compatible protocol (most model providers)
- Anthropic API protocol
- Ollama local protocol

### 5. Tool Execution Layer

Extension system based on MCP (Model Context Protocol) standard:

```
┌─────────────────────────────────────────┐
│              MCP Host                    │
│           (AGIME Core)                   │
├─────────────────────────────────────────┤
│    │           │           │            │
│    ▼           ▼           ▼            │
│ ┌─────┐    ┌─────┐    ┌─────┐          │
│ │ MCP │    │ MCP │    │ MCP │          │
│ │Server│   │Server│   │Server│  ...    │
│ │  A  │    │  B  │    │  C  │          │
│ └─────┘    └─────┘    └─────┘          │
│                                         │
│ Built-in Extensions + User Extensions   │
└─────────────────────────────────────────┘
```

---

## Data Flow

### User Request Processing Flow

```
User Input
    │
    ▼
┌─────────────────┐
│ Frontend Process│  Message formatting, attachment handling
└────────┬────────┘
         │
         ▼
┌─────────────────┐
│ Backend Routing │  Session identification, permission check
└────────┬────────┘
         │
         ▼
┌─────────────────┐
│ Context Building│  History, system prompt, tool definitions
└────────┬────────┘
         │
         ▼
┌─────────────────┐
│   Model Call    │  Send request to AI model
└────────┬────────┘
         │
         ▼
┌─────────────────┐
│ Response Parse  │  Extract text, tool calls
└────────┬────────┘
         │
    ┌────┴────┐
    │         │
    ▼         ▼
┌───────┐ ┌───────┐
│ Text  │ │ Tool  │
│Output │ │ Call  │
└───────┘ └───┬───┘
              │
              ▼
        ┌───────────┐
        │Execute    │
        │Tool       │
        └─────┬─────┘
              │
              ▼
        ┌───────────┐
        │ Continue  │  (Loop until complete)
        │ Dialogue  │
        └───────────┘
```

### Data Storage

```
~/.config/agime/
├── config.yaml          # Global config
├── profiles/            # Model configs
│   └── default.yaml
├── sessions/            # Session history
│   └── {session_id}/
│       ├── messages.json
│       └── metadata.json
├── recipes/             # Recipe storage
│   └── {recipe_id}.yaml
├── memory/              # Memory data
│   └── knowledge.db
└── extensions/          # Extension configs
    └── {extension_id}/
```

---

## Security Model

### Permission Levels

AGIME uses four-level permission control:

| Level | Name | Description | Needs Confirm |
|-------|------|-------------|---------------|
| 0 | Read-only | Read files, view info | No |
| 1 | Low-risk | Create files, network requests | Configurable |
| 2 | Medium-risk | Modify files, execute commands | Yes |
| 3 | High-risk | Delete files, system operations | Required |

### Work Modes and Permissions

```
┌─────────────┬─────────────────────────────────┐
│  Work Mode  │       Permission Behavior        │
├─────────────┼─────────────────────────────────┤
│ Autonomous  │ All operations auto-execute      │
│ Smart       │ Level 2+ needs confirmation      │
│ Manual      │ All operations need confirmation │
│ Chat Only   │ All tool calls disabled          │
└─────────────┴─────────────────────────────────┘
```

### Sandbox Isolation

- **Process Isolation**: MCP extensions run in separate processes
- **Path Restrictions**: Configurable allowed directory access
- **Network Control**: Configurable allowed domains
- **Timeout Mechanism**: Tool execution auto-terminates on timeout

---

## Extension Mechanism

### MCP Protocol

AGIME implements the extension system based on Model Context Protocol:

```yaml
# Extension definition example
name: my-extension
version: 1.0.0
description: Custom extension

tools:
  - name: my_tool
    description: Tool description
    parameters:
      type: object
      properties:
        param1:
          type: string
          description: Parameter description

resources:
  - name: my_resource
    description: Resource description
    uri: file:///path/to/resource
```

### Extension Types

1. **Built-in Extensions**: Installed with AGIME, core functionality
2. **Official Extensions**: Officially maintained, optional install
3. **Community Extensions**: Third-party developed, needs review
4. **Private Extensions**: User-developed

### Extension Communication

```
AGIME Core                    MCP Server
    │                             │
    │  ──── initialize ────►      │
    │  ◄─── capabilities ────     │
    │                             │
    │  ──── tools/list ────►      │
    │  ◄─── tool definitions ──   │
    │                             │
    │  ──── tools/call ────►      │
    │  ◄─── result ────           │
    │                             │
```

---

## Cross-Platform Support

### Technical Implementation

| Platform | Package Format | Special Handling |
|----------|----------------|------------------|
| Windows | `.exe` / `.zip` | Registry, startup |
| macOS | `.dmg` / `.zip` | Signing, notarization, sandbox |
| Linux | `.deb` / `.rpm` / `.tar.gz` | Multi-distro compatibility |

### Platform Adaptation

- **Path Handling**: Platform-agnostic path API
- **Shortcuts**: Auto-adapt by platform (Cmd/Ctrl)
- **System Integration**: Tray icon, notifications, startup
- **Native Features**: File dialogs, system theme

---

## Performance Optimization

### Startup Optimization

- Lazy loading of non-critical modules
- Pre-compiled Rust core
- Cached extension metadata

### Runtime Optimization

- Message streaming for real-time display
- Incremental tool result updates
- Paginated history loading

### Memory Management

- Sliding session context window
- Large file chunked processing
- Resource release when idle

---

## Development Guide

If you want to contribute to AGIME or build your own extensions, see:

- [Development Setup](./DEVELOPMENT.md)
- [Extension Development Guide](./EXTENSION_GUIDE.md)
- [Contributing Guide](../CONTRIBUTING.md)

---

<p align="center">
  <a href="../README.en.md">← Back to Home</a>
</p>
