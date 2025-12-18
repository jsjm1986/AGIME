<p align="center">
  <img src="https://img.shields.io/badge/AGIME-AI%20%2B%20Me-6366F1?style=for-the-badge" alt="AGIME">
  <img src="https://img.shields.io/badge/Windows-0078D6?style=for-the-badge&logo=windows&logoColor=white" alt="Windows">
  <img src="https://img.shields.io/badge/macOS-000000?style=for-the-badge&logo=apple&logoColor=white" alt="macOS">
  <img src="https://img.shields.io/badge/Linux-FCC624?style=for-the-badge&logo=linux&logoColor=black" alt="Linux">
  <img src="https://img.shields.io/badge/License-Apache_2.0-blue?style=for-the-badge" alt="License">
</p>

<h1 align="center">AGIME</h1>

<p align="center">
  <strong>AI + Me，你的本地 AI 协作伙伴</strong>
</p>

<p align="center">
  AI 不该只是聊天，它应该帮你干活<br>
  数据完全本地处理 · 永久免费开源 · 真正能执行任务
</p>

<p align="center">
  <a href="https://aiatme.cn">🌐 官方网站</a> •
  <a href="#功能特性">功能特性</a> •
  <a href="#下载安装">下载安装</a> •
  <a href="#快速开始">快速开始</a> •
  <a href="#支持的模型">支持的模型</a>
</p>

<p align="center">
  <strong>中文</strong> | <a href="README.en.md">English</a>
</p>

---

## 什么是 AGIME？

**AGIME** = **A**I + **Me**，意为"AI 与我"。

AGIME 是一款运行在你电脑上的 AI 协作伙伴。不同于只能聊天的 AI 助手，AGIME 可以：

- 📄 **读取和处理你的本地文件** - PDF、Word、Excel 批量处理
- 🖱️ **操作你的电脑** - 自动化重复性操作，解放双手
- ⏰ **执行定时任务** - 设置一次，自动完成
- 🌐 **自动收集信息** - 网页浏览、数据提取、整理汇总
- 📊 **分析数据生成报告** - 让数据说话

**核心优势：**

- 🔒 **数据安全** - 所有处理都在本地完成，敏感数据不上传云端
- 💰 **永久免费** - 软件本身免费开源，只需按量付 AI 模型费用
- 🔌 **离线可用** - 支持本地模型，没网也能用
- 🚀 **真正能干活** - 不只是给建议，而是帮你执行任务

## 功能特性

### 办公场景 - 批量处理文档

> "把这个文件夹里所有 PDF 的签约日期和金额提取出来，生成 Excel"

AGIME 可以直接读取你的本地文件，批量处理，生成汇总报告。

### 效率场景 - 自动化操作

> "每天早上 9 点，自动打开工作需要的所有软件"

像你一样操作电脑，把重复工作自动化。

### 调研场景 - 信息收集

> "去这 10 个网站，收集他们的产品价格和功能对比"

自动浏览网页、提取信息、整理成你要的格式。

### 分析场景 - 数据报告

> "分析这份销售数据，找出增长最快的产品"

帮你分析数据、发现规律、生成专业的图表和报告。

### 更多可能...

- 代码编写调试
- 图片批量处理
- 邮件自动回复
- 文件格式转换
- 系统监控报警
- 数据库查询
- API 调用集成

通过 MCP 插件机制，AGIME 的能力可以无限扩展。

## 下载安装

### 系统要求

- **操作系统**: Windows 10/11, macOS 10.15+, Linux (Ubuntu 20.04+, Fedora 34+)
- **内存**: 8GB+ RAM（推荐 16GB）
- **存储**: 500MB 可用空间

### 下载地址

从 [GitHub Releases](https://github.com/jsjm1986/AGIME/releases) 下载适合您系统的安装包：

| 系统 | 架构 | 下载格式 |
|------|------|----------|
| **Windows** | x64 | ZIP / Installer |
| **macOS** | Intel (x64) | ZIP / DMG |
| **macOS** | Apple Silicon (ARM64) | ZIP / DMG |
| **Linux** | x64 | tar.gz / DEB / RPM |
| **Linux** | ARM64 | tar.gz / DEB / RPM |

### Linux 安装

**Debian/Ubuntu (DEB):**
```bash
sudo dpkg -i AGIME-linux-x64.deb
# 或 ARM64 版本
sudo dpkg -i AGIME-linux-arm64.deb
```

**Fedora/RHEL (RPM):**
```bash
sudo rpm -i AGIME-linux-x64.rpm
# 或 ARM64 版本
sudo rpm -i AGIME-linux-arm64.rpm
```

**通用安装 (tar.gz):**
```bash
tar -xzf AGIME-linux-x64.tar.gz
cd AGIME-linux-x64
./AGIME
```

## 快速开始

### 三步开始使用

#### 1️⃣ 下载安装

选择你的操作系统，下载对应版本。安装包不到 200MB，安装只需 1 分钟。

#### 2️⃣ 配置模型

选择一个 AI 模型，填入 API Key。推荐国产模型，注册就送免费额度。

#### 3️⃣ 开始使用

用自然语言告诉 AGIME 你要做什么，它会帮你完成。就像和助理对话一样。

### 示例任务

启动 AGIME 后，试试这些指令：

```
帮我整理桌面上的文件，按项目分类
```

```
把这个文件夹里所有 PDF 的签约日期和金额提取出来
```

```
每天下午 6 点，把今天修改过的文件备份到移动硬盘
```

## 支持的模型

### 🇨🇳 国产大模型（推荐）

响应快、中文好、价格实惠

| 模型 | 说明 |
|------|------|
| **通义千问 Qwen3** | 阿里云旗舰模型 |
| **DeepSeek V3** | 推理能力强 |
| **智谱 GLM-4.6** | 国内最强 Coding |
| **豆包 1.6** | 字节跳动，市场份额第一 |
| **Kimi K2** | 万亿参数 Agent 模型 |
| **文心一言** | 百度 |

### 🌍 国际模型

性能强劲，适合复杂任务

| 模型 | 说明 |
|------|------|
| **OpenAI GPT-5.2** | 最新旗舰模型 |
| **Claude Opus 4.5** | 编码能力最强 |
| **Gemini 3** | Google 最新 |

### 💻 本地模型

完全离线，数据绝不外传

| 方案 | 说明 |
|------|------|
| **Ollama** | 一键部署本地模型 |
| **Qwen3 本地版** | 通义千问本地版 |
| **Llama 3** | Meta 开源模型 |

> 💡 **小贴士**：不知道选哪个？推荐 **通义千问**（阿里云百炼送100万免费Token）或通过 **硅基流动**（注册送2000万Token）使用各种模型。

## 常见问题

### AGIME 真的免费吗？

软件本身永久免费，代码开源。但调用 AI 模型需要付费（按使用量计费，不是订阅）。你也可以使用完全免费的本地模型。实际使用下来，比订阅 ChatGPT Plus 便宜很多。

### 我的数据安全吗？

AGIME 运行在你的电脑上，数据处理完全在本地完成。你的文件不会上传到我们的服务器。如果你使用云端模型，对话内容会发送到模型提供商（和你直接用他们的服务一样）。如果用本地模型，数据完全不出你的电脑。

### 需要什么配置的电脑？

使用云端模型对电脑配置要求不高，普通办公电脑就能用。如果想运行本地模型，建议有 16GB 以上内存，有独立显卡更好（但不是必须）。

### 和 ChatGPT 有什么区别？

ChatGPT 是云端聊天工具，只能对话。AGIME 是本地运行的 AI 助手，能读取你的文件、操作你的电脑、执行实际任务。简单说：**ChatGPT 教你做，AGIME 帮你做**。

## 企业服务

需要私有化部署或定制功能？

- 🏢 **私有化部署** - 在企业内网部署，数据完全隔离
- 🔧 **功能定制** - 根据业务需求开发专属功能
- 🔗 **系统集成** - 对接企业现有系统和数据库
- 🛡️ **技术支持** - 专属支持通道，快速响应

**微信联系：agimeme**

## 开发与贡献

### 从源码构建

```bash
# 克隆仓库
git clone https://github.com/jsjm1986/AGIME.git
cd AGIME

# 安装 Rust（如果尚未安装）
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# 构建
cargo build --release

# 构建桌面应用
cd ui/desktop
npm install
npm run make
```

### 贡献指南

我们欢迎各种形式的贡献：

- 🐛 报告 Bug - [GitHub Issues](https://github.com/jsjm1986/AGIME/issues)
- 💡 功能建议
- 📖 文档改进
- 🔧 代码贡献

## 许可证

本项目基于 [Apache License 2.0](LICENSE) 开源。

## 致谢

AGIME 基于 [Block](https://block.xyz/) 开源的 [goose](https://github.com/block/goose) 项目二次开发。

感谢 Block 团队创建了这个优秀的 AI 智能体框架！

---

<p align="center">
  <strong>AGIME</strong> - AI + Me，你的本地 AI 协作伙伴
</p>

<p align="center">
  🌐 <a href="https://aiatme.cn">官方网站</a> •
  📦 <a href="https://github.com/jsjm1986/AGIME/releases">下载</a> •
  🐛 <a href="https://github.com/jsjm1986/AGIME/issues">问题反馈</a>
</p>
