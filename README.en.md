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
  AI shouldn't just chat - it should work for you<br>
  100% Local Processing · Forever Free & Open Source · Actually Gets Things Done
</p>

<p align="center">
  <a href="https://aiatme.cn">Website</a> •
  <a href="#features">Features</a> •
  <a href="#download">Download</a> •
  <a href="#quick-start">Quick Start</a> •
  <a href="#supported-models">Models</a>
</p>

<p align="center">
  <a href="README.md">中文</a> | <strong>English</strong>
</p>

---

## What is AGIME?

**AGIME** = **A**I + **Me**, meaning "AI and Me".

AGIME is an AI partner that runs on your computer. Unlike chat-only AI assistants, AGIME can:

- 📄 **Read and process your local files** - Batch process PDFs, Word docs, Excel files
- 🖱️ **Control your computer** - Automate repetitive tasks, free your hands
- ⏰ **Run scheduled tasks** - Set it once, run automatically
- 🌐 **Collect information** - Browse web, extract data, organize results
- 📊 **Analyze data and generate reports** - Let data speak

**Why AGIME?**

- 🔒 **Data Privacy** - Everything processed locally, sensitive data never leaves your computer
- 💰 **Forever Free** - Open source software, only pay for AI model usage
- 🔌 **Works Offline** - Supports local models, works without internet
- 🚀 **Actually Works** - Not just suggestions, but real execution

## Features

### Office - Batch Document Processing

> "Extract signing dates and amounts from all PDFs in this folder, generate an Excel report"

AGIME can read your local files, batch process them, and generate summary reports.

### Productivity - Automation

> "Every morning at 9 AM, automatically open all the apps I need for work"

Like having a virtual assistant that operates your computer, automating repetitive work.

### Research - Information Gathering

> "Visit these 10 websites, collect their product prices and feature comparisons"

Automatically browse websites, extract information, and organize it into your desired format.

### Analytics - Data Reports

> "Analyze this sales data, find the fastest-growing products"

Helps you analyze data, discover patterns, and generate professional charts and reports.

### And More...

- Code writing and debugging
- Batch image processing
- Email auto-replies
- File format conversion
- System monitoring
- Database queries
- API integration

Through the MCP plugin system, AGIME's capabilities are infinitely extensible.

## Download

### System Requirements

- **OS**: Windows 10/11, macOS 10.15+, Linux (Ubuntu 20.04+, Fedora 34+)
- **RAM**: 8GB+ (16GB recommended)
- **Storage**: 500MB free space

### Download Links

Download from [GitHub Releases](https://github.com/jsjm1986/AGIME/releases):

| System | Architecture | Format |
|--------|--------------|--------|
| **Windows** | x64 | ZIP / Installer |
| **macOS** | Intel (x64) | ZIP / DMG |
| **macOS** | Apple Silicon (ARM64) | ZIP / DMG |
| **Linux** | x64 | tar.gz / DEB / RPM |
| **Linux** | ARM64 | tar.gz / DEB / RPM |

### Linux Installation

**Debian/Ubuntu (DEB):**
```bash
sudo dpkg -i AGIME-linux-x64.deb
# Or for ARM64
sudo dpkg -i AGIME-linux-arm64.deb
```

**Fedora/RHEL (RPM):**
```bash
sudo rpm -i AGIME-linux-x64.rpm
# Or for ARM64
sudo rpm -i AGIME-linux-arm64.rpm
```

**Generic (tar.gz):**
```bash
tar -xzf AGIME-linux-x64.tar.gz
cd AGIME-linux-x64
./AGIME
```

## Quick Start

### Three Steps to Get Started

#### 1️⃣ Download & Install

Choose your operating system and download the corresponding version. Installation package is under 200MB, takes only 1 minute.

#### 2️⃣ Configure Model

Choose an AI model and enter your API Key. We recommend trying local models or cloud providers with free tiers.

#### 3️⃣ Start Using

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
Every day at 6 PM, back up today's modified files to my external drive
```

## Supported Models

### 🇨🇳 Chinese Models

Fast response, excellent Chinese support, affordable

| Model | Description |
|-------|-------------|
| **Qwen3** | Alibaba Cloud flagship |
| **DeepSeek V3** | Strong reasoning |
| **GLM-4.6** | Best for coding |
| **Doubao 1.6** | ByteDance |
| **Kimi K2** | Trillion parameter agent |
| **ERNIE** | Baidu |

### 🌍 International Models

Powerful performance for complex tasks

| Model | Description |
|-------|-------------|
| **OpenAI GPT-5.2** | Latest flagship |
| **Claude Opus 4.5** | Best for coding |
| **Gemini 3** | Google's latest |

### 💻 Local Models

Completely offline, your data never leaves your computer

| Solution | Description |
|----------|-------------|
| **Ollama** | One-click local deployment |
| **Qwen3 Local** | Qwen local version |
| **Llama 3** | Meta open source |

> 💡 **Tip**: Not sure which to choose? We recommend **Ollama** for privacy-first local processing, or **OpenAI/Claude** for best performance on complex tasks.

## FAQ

### Is AGIME really free?

The software itself is forever free and open source. However, using AI models costs money (pay-per-use, not subscription). You can also use completely free local models. In practice, it's much cheaper than a ChatGPT Plus subscription.

### Is my data safe?

AGIME runs on your computer, all processing happens locally. Your files are never uploaded to our servers. If you use cloud models, conversation content is sent to the model provider (same as using their service directly). With local models, data never leaves your computer.

### What computer specs do I need?

Using cloud models has low requirements - any modern office computer works. For local models, we recommend 16GB+ RAM. A dedicated GPU helps but isn't required.

### How is this different from ChatGPT?

ChatGPT is a cloud chat tool that can only have conversations. AGIME is a local AI assistant that can read your files, control your computer, and execute real tasks. Simply put: **ChatGPT teaches you how, AGIME does it for you**.

## Enterprise Solutions

Need private deployment or custom features?

- 🏢 **Private Deployment** - Deploy within your enterprise network, complete data isolation
- 🔧 **Custom Features** - Develop features specific to your business needs
- 🔗 **System Integration** - Connect with your existing systems and databases
- 🛡️ **Technical Support** - Dedicated support channel, fast response

**Contact: agimeme (WeChat)**

## Development & Contributing

### Build from Source

```bash
# Clone the repository
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

We welcome all forms of contributions:

- 🐛 Bug Reports - [GitHub Issues](https://github.com/jsjm1986/AGIME/issues)
- 💡 Feature Requests
- 📖 Documentation Improvements
- 🔧 Code Contributions

## License

This project is open source under the [Apache License 2.0](LICENSE).

## Acknowledgments

AGIME is built upon [Goose](https://github.com/block/goose), an open source project by [Block](https://block.xyz/).

Thanks to the Block team for creating this excellent AI agent framework!

---

<p align="center">
  <strong>AGIME</strong> - AI + Me, Your Local AI Partner
</p>

<p align="center">
  🌐 <a href="https://aiatme.cn">Website</a> •
  📦 <a href="https://github.com/jsjm1986/AGIME/releases">Download</a> •
  🐛 <a href="https://github.com/jsjm1986/AGIME/issues">Issues</a>
</p>
