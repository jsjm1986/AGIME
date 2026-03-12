# AGIME 更新日志

## v2.8.0-team (2026-03)

### 🎭 数字分身系统
- 新增 Avatar 实例管理功能
- 支持 Dedicated/Shared/Managed 三种 Avatar 类型
- 实现双层代理架构（Manager Agent + Service Agent）
- 添加 Governance 治理系统
- 自动化策略配置（低/中/高风险操作）
- 能力缺口检测与优化提案
- 运行日志分析与审计

### 🚀 Team Server 增强
- 完成 MCP 协议集成的待办事项
- 实现参数获取桥接（elicitation bridge）
- 统一超时环境变量配置
- 优化工具缓存刷新机制

### 🔧 CI/CD 改进
- 修复 Clippy 严格检查问题
- 稳定化测试矩阵
- 优化 Docker 构建缓存策略
- 改进 Windows/macOS 打包流程

### 📦 依赖升级
- 升级到 rmcp 0.15
- 强制采样工具选择
- 连接 provider 选项

---

## v2.7.0-team (2026-02)

### ✨ Mission 工作流
- 实现 Mission 完成摘要流程
- 添加重试分析与自定义提示
- 优化 Mission 状态处理
- 改进完成 UX 体验

### 🎨 Web Admin 更新
- 更新 Mission 流程 UI
- 同步网站页面设计
- 优化文档路由
- 增强智能日志功能

---

## v2.6.0-team (2026-01)

### 🔌 MCP 协议完善
- 完成 MCP 服务器集成
- 实现工具缓存机制
- 优化参数获取策略
- 改进超时处理

### 🏗️ 架构优化
- 导出团队迁移功能
- 统一构建特性配置
- 优化模块依赖关系

---

## v2.5.0-team (2025-12)

### 👥 团队协作系统
- 推出 Team Server 后端
- MongoDB/SQLite 双数据库支持
- 实现 Portal 系统
- 添加团队文档管理
- 支持技能和食谱共享

### 🔐 认证与权限
- Session Cookie 认证
- API Key 支持
- 多级权限控制
- 访客限制机制

---

## v2.2.0 (2025-11)

### 🎯 核心功能
- 实现 Adaptive Goal Execution (AGE)
- 支持目标树解析
- 添加 Pivot 协议
- 智能失败重试机制

### 📊 性能优化
- 优化 Token 计数缓存
- 并行化扩展加载
- 改进代码分析性能
- 实现 LRU 缓存策略

---

## v2.1.0 (2025-10)

### 🔧 Provider 系统
- 支持 14+ AI 模型提供商
- 实现 Lead-Worker 模式
- 添加 Provider 工厂模式
- 统一 Provider 接口

### 🌐 扩展生态
- 5 个专用 MCP Server
- Developer Server (代码分析)
- Computer Controller (自动化)
- Memory Server (智能记忆)
- Auto Visualiser (可视化)
- Tutorial Server (教程系统)

---

## v2.0.0 (2025-09)

### 🎉 重大重构
- 全新的架构设计
- Rust + TypeScript 技术栈
- 模块化 Crate 结构

### 💻 多端支持
- CLI 命令行工具
- REST API 服务器
- Electron 桌面应用
- Web 管理界面

### 🧠 智能特性
- 上下文压缩系统
- 多轮对话管理
- Subagent 委派
- Recipe 食谱系统

---

## 文档系统更新 (2026-03-05)

### 🎨 界面优化
- 统一导航图标样式
- 修复浅色模式显示问题
- 集成项目官方 logo
- 添加 GitHub 和官网链接
- 优化头部布局和样式

### ✨ 功能增强
- 主题偏好记忆
- 代码块复制按钮
- 回到顶部按钮
- 目录跳转功能
- 锚点链接支持
