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
  An open-source, extensible, local-first AI agent framework<br>
  Beyond code suggestions — install, execute, edit, test, with any LLM
</p>

<p align="center">
  <a href="#features">Features</a> •
  <a href="#quick-start">Quick Start</a> •
  <a href="#usage-guide">Usage Guide</a> •
  <a href="#advanced-features">Advanced Features</a> •
  <a href="#extension-system">Extensions</a>
</p>

<p align="center">
  <a href="README.md">中文</a> | <strong>English</strong>
</p>

---

## What is AGIME?

AGIME is a local-first AI agent framework that autonomously completes complex development tasks. Unlike traditional code completion tools, AGIME can:

- **Build complete projects from scratch** - Not just code snippets, but entire applications
- **Execute and debug autonomously** - Run code, analyze errors, auto-fix issues
- **Orchestrate complex workflows** - Coordinate multiple tools and APIs to complete tasks
- **Interact with external systems** - Connect to various services and tools via MCP protocol

AGIME runs in real-time within your development environment as a true "agent" — not only searching, navigating, and writing code, but also autonomously executing tasks: reading/writing files, running tests, installing dependencies, and handling various operations.

## Features

### Autonomous Task Execution
No step-by-step guidance needed. AGIME understands goals and autonomously plans, executes, and validates the entire task flow.

### MCP Extension System
Modular extension architecture based on [Model Context Protocol](https://modelcontextprotocol.io/). Easily connect to GitHub, Google Drive, databases, and various other tools and services.

### Multi-Model Collaboration (Lead/Worker)
Intelligent dual-model configuration: use powerful models (GPT-4, Claude Opus) for planning, and fast models (GPT-4o-mini) for task execution. Optimize cost and performance.

### Recipes (Preset Tasks)
Reusable automated workflow configurations with scheduled triggers and parameterized execution. Let AI agents automatically handle repetitive tasks for you.

### Multiple Usage Methods
- **Desktop App** - Beautiful GUI supporting Windows and macOS
- **Command Line Tool** - Powerful CLI for terminal enthusiasts and automation scenarios

### Local-First, Privacy-Secure
All processing happens locally. Sensitive data is never sent to third-party servers. Especially suitable for industries with strict data privacy requirements like finance, healthcare, and government.

### Multi-Language Support
Native support for Chinese and English interfaces. More languages coming soon.

## Quick Start

### System Requirements

- **OS**: Windows 10/11, macOS 10.15+
- **Memory**: 8GB+ RAM (16GB recommended)
- **Storage**: 500MB free space
- **Network**: Connection to LLM provider API required

### Installation

#### Option 1: Desktop App (Recommended for beginners)

Download the installer for your system from the [Releases](https://github.com/agiemem/agime/releases) page:

| System | Download |
|--------|----------|
| Windows | `AGIME-Setup-x.x.x.exe` |
| macOS (Intel) | `AGIME-x.x.x-x64.dmg` |
| macOS (Apple Silicon) | `AGIME-x.x.x-arm64.dmg` |

#### Option 2: Command Line Installation

**Windows (PowerShell):**
```powershell
# Download and run install script
irm https://raw.githubusercontent.com/agiemem/agime/main/download_cli.ps1 | iex
```

**macOS / Linux:**
```bash
# Download and run install script
curl -fsSL https://raw.githubusercontent.com/agiemem/agime/main/download_cli.sh | bash
```

**Build from source:**
```bash
# Clone repository
git clone https://github.com/agiemem/agime.git
cd agime

# Build CLI
cargo build --release -p goose-cli

# Executable located at target/release/goose
```

### Initial Configuration

1. **Launch AGIME**
   ```bash
   goose configure
   ```

2. **Select LLM Provider**

   AGIME supports multiple LLM providers:

   | Provider | Environment Variable | Description |
   |----------|---------------------|-------------|
   | OpenAI | `OPENAI_API_KEY` | GPT-4, GPT-4o, etc. |
   | Anthropic | `ANTHROPIC_API_KEY` | Claude 3.5, Claude 4, etc. |
   | Google | `GOOGLE_API_KEY` | Gemini series |
   | Ollama | (runs locally) | Local models, no API needed |

3. **Set API Key**
   ```bash
   # Option 1: Environment variable
   export OPENAI_API_KEY="your-api-key"

   # Option 2: Configuration wizard
   goose configure
   ```

4. **Start First Conversation**
   ```bash
   goose session
   ```

### Hello World Example

```bash
# Launch AGIME
goose session

# After AGIME starts, try this command:
> Create a simple Python Flask app with an API endpoint that returns "Hello, AGIME!"
```

AGIME will automatically:
1. Create project directory structure
2. Write Flask application code
3. Create requirements.txt
4. Install dependencies
5. Run and test the application

## Usage Guide

### CLI Command Reference

```bash
# Session Management
goose session                    # Start new session
goose session --resume           # Resume last session
goose session -n "project-name"  # Start session with specified name

# One-time Execution
goose run --text "your command"      # Execute single task then exit
goose run --instructions file.md     # Read instructions from file

# Session List
goose session list               # List all sessions
goose session list --format json # JSON format output
goose session remove             # Interactive session deletion

# Configuration
goose configure                  # Configuration wizard
goose info                       # Show current configuration

# Extensions
goose mcp                        # Manage MCP extensions

# Recipes
goose recipe validate recipe.yaml  # Validate recipe
goose recipe open recipe-name      # Open in desktop app

# Help
goose --help                     # Show help
goose <command> --help           # Show specific command help
```

### In-Session Commands

Within an AGIME session, you can use these slash commands:

| Command | Description |
|---------|-------------|
| `/help` | Show help information |
| `/mode <name>` | Set run mode (auto, approve, chat) |
| `/extension <cmd>` | Add extension |
| `/builtin <names>` | Enable built-in extensions |
| `/plan` | Enter planning mode |
| `/recipe` | Generate recipe from current session |
| `/compact` | Compress conversation history |
| `/clear` | Clear current session |

### Run Modes

AGIME supports multiple run modes for different scenarios:

| Mode | Description | Use Case |
|------|-------------|----------|
| `auto` | Execute all operations automatically | Trusted automation tasks |
| `approve` | Confirm each operation | Sensitive operations, learning |
| `smart_approve` | Smart judgment on confirmation needs | Daily development |
| `chat` | Conversation only, no execution | Consultation, planning |

```bash
# Set default mode
goose configure

# Switch mode in session
/mode approve
```

## Advanced Features

### Lead/Worker Multi-Model Setup

Lead/Worker mode lets you combine two different models:

- **Lead Model**: Responsible for initial planning and complex reasoning
- **Worker Model**: Responsible for executing specific tasks

This configuration can significantly reduce costs while maintaining high-quality output.

#### Configuration

**Environment Variables:**
```bash
export GOOSE_PROVIDER="openai"
export GOOSE_MODEL="gpt-4o-mini"           # Worker model
export GOOSE_LEAD_MODEL="gpt-4o"           # Lead model
export GOOSE_LEAD_TURNS="3"                # Initial turns using Lead
export GOOSE_LEAD_FAILURE_THRESHOLD="2"    # Failures before switching back to Lead
```

**Desktop App:**

Settings → Models → Lead/Worker Settings

#### Recommended Configurations

| Scenario | Lead Model | Worker Model |
|----------|------------|--------------|
| High-quality development | Claude Opus | Claude Sonnet |
| Cost optimization | GPT-4o | GPT-4o-mini |
| Cross-provider | Claude Opus | GPT-4o-mini |

### Recipes (Preset Tasks)

Recipes are reusable automated workflow configurations supporting:

- Predefined task instructions
- Parameterized configuration
- Extension preloading
- Scheduled triggers

#### Recipe File Format

```yaml
# my-recipe.yaml
version: 1.0.0
title: "Code Review Assistant"
description: "Automatically review PRs and provide improvement suggestions"

# Preloaded extensions
extensions:
  - name: developer
    type: builtin

# Initial prompt
prompt: |
  Please review the following code changes, focusing on:
  1. Code quality and readability
  2. Potential bugs
  3. Performance issues
  4. Security vulnerabilities

  Changes: {{changes}}

# Parameter definitions
parameters:
  - name: changes
    description: "Code changes content"
    required: true
```

#### Using Recipes

```bash
# Validate recipe
goose recipe validate my-recipe.yaml

# Generate deep link
goose recipe deeplink my-recipe.yaml -p changes="$(git diff)"

# Open in desktop app
goose recipe open my-recipe.yaml
```

### Scheduled Tasks

AGIME supports cron-based scheduled tasks:

```bash
# Add scheduled task
goose schedule add \
  --schedule-id "daily-review" \
  --cron "0 9 * * *" \
  --recipe-source "./daily-review.yaml"

# List all scheduled tasks
goose schedule list

# Run immediately
goose schedule run-now --schedule-id "daily-review"

# Remove scheduled task
goose schedule remove --schedule-id "daily-review"
```

#### Cron Expression Examples

| Expression | Description |
|------------|-------------|
| `0 * * * *` | Every hour on the hour |
| `0 9 * * *` | Every day at 9 AM |
| `0 9 * * 1` | Every Monday at 9 AM |
| `0 0 1 * *` | First day of every month at midnight |

## Extension System

AGIME uses [Model Context Protocol (MCP)](https://modelcontextprotocol.io/) as its extension protocol, supporting three types of extensions:

### Built-in Extensions

| Extension | Description | Enable Command |
|-----------|-------------|----------------|
| `developer` | File operations, code analysis, shell commands | `/builtin developer` |
| `memory` | Session memory and context management | `/builtin memory` |
| `computercontroller` | System control, browser automation | `/builtin computercontroller` |
| `autovisualiser` | Data visualization | `/builtin autovisualiser` |
| `tutorial` | Interactive tutorial | `/builtin tutorial` |

### Command Line Extensions

Add any MCP-compatible command line tool:

```bash
# Add in session
/extension npx -y @modelcontextprotocol/server-github

# Or in config file
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

### Remote Extensions (SSE)

Connect to remote MCP servers:

```yaml
extensions:
  - name: remote-service
    type: sse
    uri: "https://mcp.example.com/sse"
```

### Popular MCP Extensions

| Extension | Install Command | Function |
|-----------|-----------------|----------|
| GitHub | `npx @modelcontextprotocol/server-github` | GitHub repository operations |
| Filesystem | `npx @modelcontextprotocol/server-filesystem` | File system access |
| PostgreSQL | `npx @modelcontextprotocol/server-postgres` | Database operations |
| Slack | `npx @modelcontextprotocol/server-slack` | Slack integration |

## Supported LLM Providers

### Cloud Services

| Provider | Supported Models | Configuration |
|----------|-----------------|---------------|
| **OpenAI** | GPT-4o, GPT-4o-mini, o1, o3 | `OPENAI_API_KEY` |
| **Anthropic** | Claude 4 Opus, Claude 4 Sonnet, Claude 3.5 | `ANTHROPIC_API_KEY` |
| **Google** | Gemini 2.5 Pro, Gemini 2.5 Flash | `GOOGLE_API_KEY` |
| **Azure OpenAI** | All Azure-deployed models | `AZURE_OPENAI_API_KEY` |
| **AWS Bedrock** | Claude, Llama, etc. | AWS credentials |
| **OpenRouter** | 100+ models | `OPENROUTER_API_KEY` |

### Local Models

| Solution | Description | Configuration |
|----------|-------------|---------------|
| **Ollama** | Run open-source models locally | `OLLAMA_HOST` |
| **LM Studio** | GUI-based local model management | OpenAI-compatible API |

### Configuration Examples

```bash
# OpenAI
export GOOSE_PROVIDER="openai"
export GOOSE_MODEL="gpt-4o"
export OPENAI_API_KEY="sk-..."

# Anthropic
export GOOSE_PROVIDER="anthropic"
export GOOSE_MODEL="claude-sonnet-4-20250514"
export ANTHROPIC_API_KEY="sk-ant-..."

# Ollama (local)
export GOOSE_PROVIDER="ollama"
export GOOSE_MODEL="llama3.2"
export OLLAMA_HOST="http://localhost:11434"
```

## Project Structure

```
agime/
├── crates/
│   ├── goose/           # Core library: Agent, Provider, Config
│   ├── goose-cli/       # Command line tool
│   ├── goose-server/    # HTTP API server (goosed)
│   ├── goose-mcp/       # Built-in MCP extensions
│   ├── goose-bench/     # Benchmark framework
│   └── goose-test/      # Testing utilities
│
├── ui/
│   └── desktop/         # Electron desktop app
│
└── documentation/       # Documentation
```

## FAQ

### Q: What's the difference between AGIME and other AI coding assistants?

AGIME is an **agent**, not a simple code completion tool. It can autonomously plan, execute, and validate entire task flows without requiring step-by-step guidance.

### Q: Will my code be sent to the cloud?

It depends on your chosen LLM provider. If using cloud services (OpenAI, Anthropic, etc.), code will be sent to their APIs. For completely local operation, you can use local model solutions like Ollama.

### Q: How can I reduce API costs?

1. Use Lead/Worker mode with cheaper models for most tasks
2. Use `/compact` command to compress conversation history
3. Choose more economical models (like GPT-4o-mini)
4. Consider using local models

### Q: Which programming languages are supported?

AGIME supports all programming languages. The built-in `developer` extension has enhanced code analysis support for:
- Python, JavaScript/TypeScript, Rust, Go
- Java, Kotlin, Ruby, Swift
- And more...

## Development & Contributing

### Development Environment Setup

```bash
# Clone repository
git clone https://github.com/agiemem/agime.git
cd agime

# Install Rust (if not already installed)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Build all crates
cargo build

# Run tests
cargo test

# Build desktop app
cd ui/desktop
npm install
npm run make
```

### Contributing

We welcome all forms of contribution:

- Bug reports
- Feature suggestions
- Documentation improvements
- Code contributions

Please see [CONTRIBUTING.md](CONTRIBUTING.md) for details.

## License

This project is open-sourced under the [Apache License 2.0](LICENSE).

## Acknowledgments

AGIME is based on the [goose](https://github.com/block/goose) project open-sourced by [Block](https://block.xyz/).

Thanks to the Block team for creating this excellent AI agent framework, and to these technologies and projects:

- [Model Context Protocol (MCP)](https://modelcontextprotocol.io/) - Anthropic
- [Rust](https://www.rust-lang.org/) programming language
- [Electron](https://www.electronjs.org/) desktop application framework
- All LLM provider API services

---

<p align="center">
  <strong>AGIME</strong> - Let AI become your autonomous development partner
</p>

<p align="center">
  Maintained by <a href="https://github.com/agiemem">agiemem</a>
</p>
