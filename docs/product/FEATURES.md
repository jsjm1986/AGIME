# AGIME 核心功能

AGIME 的核心在于 **全栈自动化** 与 **无限扩展性**。它不仅仅是一个对话框，而是一个连接了操作系统、文件系统和互联网的智能中枢。

---

## 1. 自动化 (Automation)

AGIME 能够直接控制计算机执行任务，而非仅仅给出建议。我们通过多种技术手段实现了这一能力。

### 📄 文档批量处理
基于视觉模型 (Vision-Language Models)，AGIME 能像人眼一样阅读 PDF、图片和扫描件。

**场景案例**：
```
💬 你说：把这个文件夹里所有合同 PDF 的签约日期、金额提取出来，生成 Excel

🤖 AGIME：
   ✓ 扫描文件夹，发现 47 个 PDF
   ✓ 逐个解析合同内容
   ✓ 提取关键信息
   → 已保存到 合同汇总.xlsx
```

### 🖥️ 系统级操作 (Computer Use)
安全地控制鼠标和键盘，模拟人类操作。适用于没有 API 的传统软件。

**场景案例**：
```
💬 你说：每天早上 9 点，自动打开微信、钉钉，并打开我常用的 5 个网站

🤖 AGIME：
   ✓ 创建定时任务：每天 09:00
   ✓ 配置启动程序列表
   ✓ 配置浏览器标签页
   → 任务已启用，明天开始执行
```

### 📊 数据分析报告
内置 Python 解释器，可以直接运行数据分析代码。

**场景案例**：
```
💬 你说：分析这份 sales_2024.xlsx，找出增长最快的产品，生成周报

🤖 AGIME：
   ✓ 读取 sales_2024.xlsx
   ✓ 计算各产品增长率
   ✓ 生成可视化图表 (Matplotlib)
   → 已生成 销售周报.docx
```

### 🌐 网页信息收集
内置 Playwright/Puppeteer 控制器，可以像人类一样浏览复杂网页。

**场景案例**：
```
💬 你说：去这 10 个竞品官网，收集他们的产品价格和功能列表

🤖 AGIME：
   ✓ 依次访问 10 个网站
   ✓ 提取产品信息
   ✓ 整理对比表格
   → 已生成 竞品分析.xlsx
```

---

## 2. 记忆与知识 (Memory & Knowledge)

AGIME 的记忆系统由三部分组成：

1.  **短期记忆 (Context)**: 在当前对话窗口内，基于 Token 窗口滑动机制。
2.  **长期记忆 (Vector DB)**: 将历史对话向量化存储。当你再次提及"上次那个项目"时，它能从数月前的对话中找回上下文。
3.  **用户画像 (Profile)**: 显式记录你的偏好（如："我是 Java 程序员"、"喜欢简洁的代码"）。

---

## 3. 扩展系统 (MCP Ecosystem)

AGIME 完整支持 **Model Context Protocol (MCP)**。这是一个开放标准，允许 AI 连接任何数据源。

### 官方插件库

| 插件名 | 功能描述 | 权限要求 |
|:---|:---|:---|
| **@agime/fs** | 文件系统访问 (读/写/监听) | 需授权目录 |
| **@agime/browser** | 浏览器控制 (Headless/GUI) | 网络访问 |
| **@agime/terminal** | 终端命令执行 | 系统管理员 |
| **@agime/postgres** | 连接 PostgreSQL 数据库 | 数据库凭证 |

### 🛠️ 工作流复用 (Recipes)
你可以将一次成功的交互保存为"食谱" (Recipe)。

```yaml
# Recipe: Generate Monthly Report
steps:
  1. Open email client and check for subject "Monthly Stats"
  2. Download attached CSV
  3. Run analysis script
  4. Reply with summary PDF
```
一旦保存，下次只需说 *"运行月报流程"* 即可自动执行复刻。
