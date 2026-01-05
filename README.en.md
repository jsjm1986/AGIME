<p align="center">
  <img src="https://img.shields.io/badge/AGIME-AI%20%2B%20Me-6366F1?style=for-the-badge" alt="AGIME">
  <img src="https://img.shields.io/badge/v2.2.0-stable-green?style=for-the-badge" alt="Version">
  <img src="https://img.shields.io/badge/License-Apache_2.0-blue?style=for-the-badge" alt="License">
</p>

<p align="center">
  <img src="./ui/desktop/src/images/icon.png" alt="AGIME Logo" width="128">
</p>

<h1 align="center">AGIME</h1>

<h3 align="center">AI + Me — Not Just Chat, An AI That Actually Works For You</h3>

<p align="center">
  <a href="https://github.com/jsjm1986/AGIME/releases"><img src="https://img.shields.io/github/downloads/jsjm1986/AGIME/total?style=flat-square&label=Downloads&color=brightgreen" alt="Downloads"></a>
  <a href="https://github.com/jsjm1986/AGIME/stargazers"><img src="https://img.shields.io/github/stars/jsjm1986/AGIME?style=flat-square&label=Stars" alt="Stars"></a>
  <a href="https://github.com/jsjm1986/AGIME/releases/latest"><img src="https://img.shields.io/github/release/jsjm1986/AGIME?style=flat-square&label=Latest" alt="Release"></a>
</p>

<p align="center">
  <a href="https://github.com/jsjm1986/AGIME/releases">
    <img src="https://img.shields.io/badge/-Windows-0078D6?style=for-the-badge&logo=windows&logoColor=white" alt="Windows">
  </a>
  <a href="https://github.com/jsjm1986/AGIME/releases">
    <img src="https://img.shields.io/badge/-macOS-000000?style=for-the-badge&logo=apple&logoColor=white" alt="macOS">
  </a>
  <a href="https://github.com/jsjm1986/AGIME/releases">
    <img src="https://img.shields.io/badge/-Linux-FCC624?style=for-the-badge&logo=linux&logoColor=black" alt="Linux">
  </a>
</p>

<p align="center">
  <a href="https://github.com/jsjm1986/AGIME/releases"><b>📥 Download</b></a> ·
  <a href="https://aiatme.cn"><b>🌐 Website</b></a> ·
  <a href="README.md"><b>中文</b></a>
</p>

---

<table>
<tr>
<td width="50%">

### ✨ Core Capabilities

- 📄 **Batch Document Processing** - Extract PDFs, generate reports
- ⏰ **Scheduled Automation** - Daily/weekly auto-execution
- 🖥️ **Computer Control** - Launch apps, browse web
- 🔒 **Privacy First** - 100% local data processing
- 🧠 **Smart Memory** - Gets smarter over time
- 🔌 **Infinite Extensions** - MCP plugin ecosystem

</td>
<td width="50%">

### 🆚 How It Differs from Traditional AI

| | Traditional AI | AGIME |
|--|:--:|:--:|
| Execute Tasks | ❌ Only advises | ✅ Does it for you |
| Local Files | ❌ No access | ✅ Full access |
| Scheduling | ❌ Not supported | ✅ Auto-execute |
| Data Privacy | ☁️ Cloud upload | 🏠 Local only |

</td>
</tr>
</table>

---

## 🎯 What Can It Do?

<table>
<tr>
<td width="50%">

### 📄 Batch Document Processing

```
💬 You: Extract signing dates and amounts
from all contract PDFs in this folder,
generate Excel

🤖 AGIME:
   ✓ Scanning folder, found 47 PDFs
   ✓ Parsing each contract
   ✓ Extracting key information
   → Saved to contracts_summary.xlsx
```

</td>
<td width="50%">

### ⏰ Scheduled Automation

```
💬 You: Every morning at 9 AM,
auto-open Slack, browser, and my
5 favorite websites

🤖 AGIME:
   ✓ Creating task: Daily 09:00
   ✓ Configuring app launch list
   ✓ Configuring browser tabs
   → Task enabled, starts tomorrow
```

</td>
</tr>
<tr>
<td width="50%">

### 📊 Data Analysis Reports

```
💬 You: Analyze this sales data, find
fastest-growing products, generate
weekly report

🤖 AGIME:
   ✓ Reading sales_2024.xlsx
   ✓ Calculating growth rates
   ✓ Generating visualizations
   → Generated weekly_report.docx
```

</td>
<td width="50%">

### 🌐 Web Information Gathering

```
💬 You: Visit these 10 competitor
websites, collect their product
prices and features

🤖 AGIME:
   ✓ Visiting 10 websites
   ✓ Extracting product info
   ✓ Creating comparison table
   → Generated competitor_analysis.xlsx
```

</td>
</tr>
</table>

---

## ✨ Core Features

### 🤖 Smart Conversations
Communicate naturally, like talking to an assistant. Supports context memory and understands your intent.

### 🔄 Recipe System (Workflow Reuse)
Save successful conversations as "recipes" for one-click repeat execution.

```
Scenario: Monthly report with same format
→ First time: Have AGIME help you, save as recipe
→ Every month after: Click to run, done automatically
```

### ⏱️ Task Scheduling
Set tasks to run automatically - hourly, daily, weekly, or monthly.

### 🧩 Extension System (MCP Plugins)
Infinitely extensible capabilities:

| Plugin | Function |
|--------|----------|
| **Developer** | Code analysis, file editing, command execution |
| **ComputerController** | Automate computer operations |
| **Memory** | Remember your preferences and habits |
| **Playwright** | Auto-browse web, fill forms, screenshots |

### 🧠 Smart Memory
AGIME remembers your preferences, gets smarter over time.

### ⚡ Four Work Modes

| Mode | Description | Use Case |
|------|-------------|----------|
| 🟢 **Autonomous** | AI operates freely | Trusted repetitive tasks |
| 🟡 **Smart** | High-risk actions need confirmation | Daily use (recommended) |
| 🔴 **Manual** | Every step needs confirmation | Sensitive operations |
| ⚪ **Chat Only** | Conversation only, no execution | Q&A |

---

## 🏗️ Technical Architecture

### Local-First
AGIME runs entirely on your computer. All data processing happens locally, sensitive information never leaves your machine.

### Universal Model Compatibility
Supports virtually all mainstream AI models:
- **Cloud Models** - Connect to various AI service providers via API
- **Local Models** - Run completely offline via Ollama and similar solutions

> 💡 Choose the model that best fits your task requirements and privacy needs.

### MCP Extension Protocol
Built on the Model Context Protocol standard, supporting a rich plugin ecosystem:
- 🔧 **Tool Extensions** - File operations, command execution, web browsing
- 🔗 **Service Integration** - Connect to external services and APIs
- 🎨 **Custom Capabilities** - Develop custom plugins for personalized needs

### Security Sandbox
All operations execute in a controlled environment with multi-level permission management for system security.

### Cross-Platform Support
Native support for Windows, macOS, and Linux with a unified user experience.

---

## ⬇️ Download & Install

### Direct Download

From [GitHub Releases](https://github.com/jsjm1986/AGIME/releases):

| System | Download |
|--------|----------|
| **Windows** | `.exe` installer or `.zip` portable |
| **macOS (Intel)** | `.dmg` or `.zip` |
| **macOS (Apple Silicon)** | `.dmg` or `.zip` |
| **Linux** | `.deb` / `.rpm` / `.tar.gz` |

### Quick Start

```
1️⃣ Download & Install (1 minute)
2️⃣ Configure API Key (choose a model provider, enter your key)
3️⃣ Start Using (tell it what you want in natural language)
```

---

## 💼 Enterprise Solutions

- 🏢 **Private Deployment** - Data stays within enterprise network
- 🔧 **Custom Development** - Build features for your needs
- 🔗 **System Integration** - Connect with existing systems
- 🛡️ **Technical Support** - Dedicated support channel

**Contact: agimeme (WeChat)**

---

## 🤝 Contributing

We welcome all forms of contribution:

- 🐛 [Report Bugs](https://github.com/jsjm1986/AGIME/issues)
- 💡 [Feature Requests](https://github.com/jsjm1986/AGIME/issues)
- 📖 Improve Documentation
- 🔧 Submit Code

---

## 📚 Technical Documentation

Want to learn more about AGIME?

- [How It Works](docs/HOW_IT_WORKS.en.md) - Easy-to-understand explanation
- [Architecture](docs/ARCHITECTURE.en.md) - Detailed technical architecture

---

## 📄 License

[Apache License 2.0](LICENSE) - Forever free, commercial use OK

---

## 🙏 Acknowledgments

AGIME would not exist without the power of the open source community. We thank all developers who contribute to open source—your selfless sharing makes this project possible.

Special thanks to:
- The Rust ecosystem and its excellent toolchain
- Electron and the frontend open source community
- MCP protocol and its ecosystem
- All open source libraries and tools used in this project

**Open source makes the world better.**

---

<p align="center">
  <strong>AGIME</strong> - AI + Me, Your Local AI Assistant
</p>

<p align="center">
  <a href="https://github.com/jsjm1986/AGIME/releases">⬇️ Download</a> •
  <a href="https://aiatme.cn">🌐 Website</a> •
  <a href="https://github.com/jsjm1986/AGIME/issues">💬 Feedback</a>
</p>
