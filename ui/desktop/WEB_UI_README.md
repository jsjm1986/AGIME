# AGIME Web UI

Web UI 实现，允许通过浏览器远程访问 AGIME AI 助手。

## 功能概述

Web UI 是一个独立于 Electron 桌面应用的纯浏览器版本，支持：
- 通过隧道 (Tunnel) 远程访问
- 安全的密钥认证机制
- 与桌面端一致的聊天体验

## 架构设计

### Platform 抽象层

```
ui/desktop/src/platform/
├── index.ts      # 平台检测和导出
├── types.ts      # 类型定义
├── electron.ts   # Electron 平台实现
└── web.ts        # Web 平台实现
```

**核心设计思想**：通过抽象层统一 Electron 和 Web 的 API 差异，使业务代码无需关心运行环境。

```typescript
import { platform, isElectron, isWeb } from './platform';

// 统一调用，自动适配平台
platform.logInfo('Hello');
platform.openExternalUrl('https://example.com');
```

### 入口文件

| 文件 | 用途 |
|------|------|
| `index.html` | Electron 桌面端入口 |
| `index-web.html` | Web 浏览器端入口 |
| `renderer.tsx` | Electron 渲染进程 |
| `renderer-web.tsx` | Web 端渲染入口 |

### 构建配置

| 文件 | 用途 |
|------|------|
| `vite.config.mts` | Electron 构建配置 |
| `vite.config.web.mts` | Web 构建配置 |

## 安全特性

### 认证机制

1. **URL Hash 认证** (推荐)
   ```
   https://your-tunnel.example.com/web/#secret=YOUR_SECRET_KEY
   ```
   - Secret 不会发送到服务器（hash 部分不包含在 HTTP 请求中）
   - 认证后自动从 URL 中移除

2. **内存存储**
   - Secret 存储在内存中，而非 sessionStorage
   - 防止 XSS 攻击窃取凭证

3. **SHA-256 哈希**
   - Recipe 数据使用 Web Crypto API 进行 SHA-256 哈希
   - 安全可靠的浏览器原生加密

### Content Security Policy (CSP)

```html
<meta http-equiv="Content-Security-Policy" content="
  default-src 'self';
  script-src 'self';
  style-src 'self' 'unsafe-inline';
  connect-src 'self' http://127.0.0.1:* https://*.trycloudflare.com wss://*.trycloudflare.com;
">
```

### 后端安全

- Symlink 检测防止路径遍历攻击
- Secret Key 环境变量警告
- Signal handler 优雅处理

## 使用方法

### 开发模式

```bash
cd ui/desktop

# 启动 Web 开发服务器
npm run dev:web

# 访问 http://127.0.0.1:5174/web/
```

### 生产构建

```bash
cd ui/desktop

# 构建 Web 版本
npm run build:web

# 输出目录: dist-web/
```

### 后端服务

```bash
# 设置环境变量（支持 AGIME_SERVER__SECRET_KEY 或兼容的 GOOSE_SERVER__SECRET_KEY）
export AGIME_SERVER__SECRET_KEY="your-secure-secret-key"

# 启动服务
cargo run -p agime-server -- agent
```

### 完整访问流程

1. 启动后端服务 (`agimed agent`)
2. 在桌面端 AGIME 中：设置 → 应用 → 远程访问
3. 启动隧道，获取访问链接
4. 在浏览器中打开链接，自动认证

## 目录结构

```
ui/desktop/
├── dist-web/                    # Web 构建输出
│   ├── assets/                  # JS/CSS 资源
│   ├── favicon.png              # 网站图标
│   └── index-web.html           # 入口 HTML
├── public-web/                  # Web 静态资源
│   └── favicon.png              # 网站图标源文件
├── src/
│   ├── platform/                # 平台抽象层
│   ├── renderer-web.tsx         # Web 入口
│   ├── theme-init.ts            # 主题初始化 (CSP 兼容)
│   └── styles/
│       └── web-overrides.css    # Web 端样式覆盖
├── index-web.html               # Web 入口 HTML
└── vite.config.web.mts          # Web Vite 配置
```

## API 差异处理

### Electron 专有功能的 Web 替代

| Electron API | Web 替代方案 |
|--------------|--------------|
| `ipcRenderer.invoke()` | HTTP/WebSocket API |
| `shell.openExternal()` | `window.open()` |
| `clipboard.writeText()` | `navigator.clipboard.writeText()` |
| `app.getPath()` | 不支持 (返回空字符串) |
| `BrowserWindow` | 不支持 (窗口操作忽略) |

### 条件渲染

```typescript
import { isElectron, isWeb } from './platform';

// 仅在 Electron 中显示
{isElectron && <WindowControls />}

// 仅在 Web 中显示
{isWeb && <AuthRequired />}
```

## 审计修复记录

### Critical (已修复)

| 编号 | 问题 | 修复方案 |
|------|------|----------|
| C1 | XSS - sessionStorage 存储 secret | 改用内存存储 |
| C2 | 路径遍历 - 无 symlink 保护 | 添加 symlink 检测 |
| C3 | favicon 缺失 | 创建 public-web/favicon.png |
| C4 | CSP unsafe-inline | 分离 theme-init.ts |

### High (已修复)

| 编号 | 问题 | 修复方案 |
|------|------|----------|
| H4-H5 | 后端安全隐患 | Signal handler + secret key 警告 |
| H6 | 浏览器兼容性 | ES2020 target |

### Medium (已修复)

| 编号 | 问题 | 修复方案 |
|------|------|----------|
| M1-M3 | 前端安全 | SHA-256 hash + URL hash 认证 |

### Low (已修复)

| 编号 | 问题 | 修复方案 |
|------|------|----------|
| L1 | 内存泄漏 | clearAllEventListeners |
| L5 | meta 标签缺失 | 添加 viewport/description |

## 浏览器兼容性

- Chrome 80+
- Firefox 78+
- Safari 14+
- Edge 89+

## 注意事项

1. **Secret Key**: 生产环境必须设置 `AGIME_SERVER__SECRET_KEY` 环境变量（也支持兼容的 `GOOSE_SERVER__SECRET_KEY`）
2. **HTTPS**: 生产环境建议使用 HTTPS 以保护传输安全
3. **隧道服务**: 推荐使用 Cloudflare Tunnel 进行远程访问

## 测试状态

- [x] Platform 抽象层正常工作
- [x] Web 入口文件加载正确
- [x] 认证页面显示正常
- [x] UI 样式渲染正确
- [x] 中英双语支持
- [ ] 完整后端联调 (需要 Rust 编译环境)

---

*最后更新: 2024-12-22*
