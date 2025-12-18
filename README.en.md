<p align="center">
  <img src="https://img.shields.io/badge/AGIME-AI%20%2B%20Me-6366F1?style=for-the-badge" alt="AGIME">
  <img src="https://img.shields.io/badge/Windows-0078D6?style=for-the-badge&logo=windows&logoColor=white" alt="Windows">
  <img src="https://img.shields.io/badge/macOS-000000?style=for-the-badge&logo=apple&logoColor=white" alt="macOS">
  <img src="https://img.shields.io/badge/Linux-FCC624?style=for-the-badge&logo=linux&logoColor=black" alt="Linux">
  <img src="https://img.shields.io/badge/License-Apache_2.0-blue?style=for-the-badge" alt="License">
</p>

<h1 align="center">AGIME</h1>

<p align="center">
  <strong>AI + Me, Your Local AI Partner</strong>
</p>

<p align="center">
  AI shouldn't just chat — it should do the work for you<br>
  Local data processing · Forever free & open source · Actually executes tasks
</p>

<p align="center">
  <a href="https://aiatme.cn">Website</a> •
  <a href="#features">Features</a> •
  <a href="#download">Download</a> •
  <a href="#quick-start">Quick Start</a> •
  <a href="#supported-models">Supported Models</a>
</p>

<p align="center">
  <a href="README.md">中文</a> | <strong>English</strong>
</p>

---

## What is AGIME?

**AGIME** = **A**I + **Me**, meaning "AI and Me".

AGIME is an AI partner that runs on your computer. Unlike AI assistants that only chat, AGIME can:

- **Read and process your local files** - Batch processing of PDFs, Word, Excel
- **Control your computer** - Automate repetitive operations
- **Execute scheduled tasks** - Set it once, runs automatically
- **Automatically collect information** - Web browsing, data extraction, organization
- **Analyze data and generate reports** - Let your data speak

**Core Advantages:**

- **Data Security** - All processing happens locally, sensitive data never uploads to the cloud
- **Forever Free** - Open source software, only pay for AI model API usage
- **Offline Capable** - Supports local models, works without internet
- **Actually Does Work** - Not just suggestions, but executes tasks for you

## Features

### Office Scenarios - Batch Document Processing

> "Extract signing dates and amounts from all PDFs in this folder and create an Excel file"

AGIME can directly read your local files, batch process them, and generate summary reports.

### Productivity Scenarios - Automation

> "Every morning at 9 AM, automatically open all the software I need for work"

Operates your computer like you would, automating repetitive work.

### Research Scenarios - Information Collection

> "Go to these 10 websites and collect their product prices and feature comparisons"

Automatically browses web pages, extracts information, and organizes it into your desired format.

### Analysis Scenarios - Data Reports

> "Analyze this sales data and find the fastest-growing products"

Helps you analyze data, discover patterns, and generate professional charts and reports.

### And More...

- Code writing and debugging
- Batch image processing
- Automatic email replies
- File format conversion
- System monitoring and alerts
- Database queries
- API integrations

Through the MCP plugin system, AGIME's capabilities can be infinitely extended.

## Download

### System Requirements

- **Operating System**: Windows 10/11, macOS 10.15+, Linux (Ubuntu 20.04+, Fedora 34+)
- **Memory**: 8GB+ RAM (16GB recommended)
- **Storage**: 500MB available space

### Download Links

Download the installer for your system from [GitHub Releases](https://github.com/jsjm1986/AGIME/releases):

| System | Architecture | Download Format |
|--------|--------------|-----------------|
| **Windows** | x64 | ZIP / Installer |
| **macOS** | Intel (x64) | ZIP / DMG |
| **macOS** | Apple Silicon (ARM64) | ZIP / DMG |
| **Linux** | x64 | tar.gz / DEB / RPM |
| **Linux** | ARM64 | tar.gz / DEB / RPM |

### Linux Installation

**Debian/Ubuntu (DEB):**
```bash
sudo dpkg -i AGIME-linux-x64.deb
# Or ARM64 version
sudo dpkg -i AGIME-linux-arm64.deb
```

**Fedora/RHEL (RPM):**
```bash
sudo rpm -i AGIME-linux-x64.rpm
# Or ARM64 version
sudo rpm -i AGIME-linux-arm64.rpm
```

**Universal (tar.gz):**
```bash
tar -xzf AGIME-linux-x64.tar.gz
cd AGIME-linux-x64
./AGIME
```

## Quick Start

### Three Steps to Get Started

#### 1. Download and Install

Choose your operating system and download the corresponding version. Less than 200MB, installs in 1 minute.

#### 2. Configure Model

Select an AI model and enter your API Key. Domestic models recommended - registration often includes free credits.

#### 3. Start Using

Tell AGIME what you want to do in natural language, and it will help you complete it. Just like talking to an assistant.

### Example Tasks

After launching AGIME, try these commands:

```
Help me organize the files on my desktop by project
```

```
Extract signing dates and amounts from all PDFs in this folder
```

```
Every day at 6 PM, backup files modified today to my external drive
```

## Supported Models

### Chinese Models (Recommended for Chinese users)

Fast response, excellent Chinese support, affordable pricing

| Model | Description |
|-------|-------------|
| **Qwen3** | Alibaba Cloud flagship |
| **DeepSeek V3** | Strong reasoning |
| **GLM-4.6** | Best Coding in China |
| **Doubao 1.6** | ByteDance, #1 market share |
| **Kimi K2** | Trillion-param Agent model |
| **ERNIE** | Baidu |

### International Models

Powerful performance, suitable for complex tasks

| Model | Description |
|-------|-------------|
| **OpenAI GPT-5.2** | Latest flagship |
| **Claude Opus 4.5** | Best at coding |
| **Gemini 3** | Google latest |

### Local Models

Completely offline, data never leaves your computer

| Solution | Description |
|----------|-------------|
| **Ollama** | One-click local model deployment |
| **Qwen3 Local** | Local version of Qwen3 |
| **Llama 3** | Meta open source |

> **Tip**: Not sure which to choose? Try **Qwen3** (Alibaba Cloud gives 1M free tokens) or **SiliconFlow** (20M free tokens on signup).

## FAQ

### Is AGIME really free?

The software itself is forever free and open source. However, using AI models requires API fees (pay-per-use, not subscription). You can also use completely free local models. In practice, it's much cheaper than subscribing to ChatGPT Plus.

### Is my data safe?

AGIME runs on your computer, and data processing happens entirely locally. Your files are never uploaded to our servers. If you use cloud models, conversation content is sent to the model provider (same as using their service directly). If using local models, data never leaves your computer.

### What computer specs do I need?

Using cloud models doesn't require much - a typical office computer works fine. For running local models, we recommend 16GB+ RAM, and a dedicated GPU helps (but isn't required).

### What's the difference from ChatGPT?

ChatGPT is a cloud-based chat tool that can only converse. AGIME is a locally-running AI assistant that can read your files, control your computer, and execute actual tasks. Simply put: **ChatGPT teaches you how, AGIME does it for you**.

## Enterprise Services

Need private deployment or custom features?

- **Private Deployment** - Deploy within your enterprise network, complete data isolation
- **Custom Features** - Develop exclusive features based on business needs
- **System Integration** - Connect with existing enterprise systems and databases
- **Technical Support** - Dedicated support channel, rapid response

**WeChat Contact: agimeme**

## Development & Contributing

### Building from Source

```bash
# Clone repository
git clone https://github.com/jsjm1986/AGIME.git
cd AGIME

# Install Rust (if not already installed)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Build
cargo build --release

# Build desktop app
cd ui/desktop
npm install
npm run make
```

### Contributing

We welcome all forms of contribution:

- Bug reports - [GitHub Issues](https://github.com/jsjm1986/AGIME/issues)
- Feature suggestions
- Documentation improvements
- Code contributions

## License

This project is open-sourced under the [Apache License 2.0](LICENSE).

## Acknowledgments

AGIME is based on the [goose](https://github.com/block/goose) project open-sourced by [Block](https://block.xyz/).

Thanks to the Block team for creating this excellent AI agent framework!

---

<p align="center">
  <strong>AGIME</strong> - AI + Me, Your Local AI Partner
</p>

<p align="center">
  <a href="https://aiatme.cn">Website</a> •
  <a href="https://github.com/jsjm1986/AGIME/releases">Download</a> •
  <a href="https://github.com/jsjm1986/AGIME/issues">Issues</a>
</p>
