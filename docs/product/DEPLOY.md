# 部署与安装

AGIME 提供灵活的部署选项，从个人单机使用到企业级私有云集群。

---

## 1. 客户端安装 (Client)

适用于个人用户或团队成员终端。

### 📥 官方下载

访问 [GitHub Releases](https://github.com/jsjm1986/AGIME/releases) 获取最新版本。

| 操作系统 | 文件名 | 说明 |
|:---|:---|:---|
| **Windows** | `AGIME-Setup-x.x.x.exe` | 推荐。自动更新，包含所有依赖。 |
| **macOS** | `AGIME-x.x.x.dmg` | 支持 Intel 和 Apple Silicon (M1/M2/M3)。 |
| **Linux** | `AGIME-x.x.x.AppImage` | 适用于 Ubuntu/Debian/Fedora，即点即用。 |

### ⚡ 首次运行配置

1. **启动应用**: 安装完成后双击图标启动。
2. **选择模型**:
   - **云端**: 填入 OpenAI / Anthropic 的 API Key。
   - **本地**: 确保 Ollama 已在后台运行 (默认地址 `http://localhost:11434`)。
3. **完成**: 你现在可以开始对话了。

---

## 2. 服务端部署 (Server)

适用于 **Cloud 模式** 的团队协作。需要一台 Linux 服务器。

### 硬件要求
- **CPU**: 2核 及以上
- **RAM**: 4GB 及以上
- **Disk**: 20GB SSD
- **OS**: Ubuntu 22.04 LTS (推荐) / CentOS 7+

### 🐳 Docker 部署 (推荐)

最简单的部署方式。

```bash
# 1. 拉取镜像
docker pull ghcr.io/jsjm1986/agime-server:latest

# 2. 启动容器
docker run -d \
  --name agime-server \
  -p 3000:3000 \
  -v /data/agime:/data \
  -e AGIME_SECRET_KEY="your_secure_random_string" \
  ghcr.io/jsjm1986/agime-server:latest
```

### 🐧 Linux 二进制部署

如果你不使用 Docker：

```bash
# 1. 下载服务端程序
wget https://github.com/jsjm1986/AGIME/releases/latest/download/agime-server-linux-x64.tar.gz
tar -xzf agime-server-linux-x64.tar.gz

# 2. 运行
cd agime-server
./agimed start
```

### ⚙️ 环境变量

| 变量名 | 默认值 | 说明 |
|:---|:---|:---|
| `PORT` | 3000 | 服务监听端口 |
| `AGIME_SECRET_KEY` | - | **必填**。用于加密 Session 的密钥 |
| `DATABASE_URL` | `sqlite://data.db` | 数据库连接串 (支持 PostgreSQL) |

---

## 3. 企业级网络配置

为了确保安全性，建议在 Nginx 反向代理后运行 AGIME Server，并配置 SSL 证书。

### Nginx 配置示例

```nginx
server {
    listen 443 ssl;
    server_name agime.your-company.com;

    ssl_certificate /etc/nginx/ssl/cert.pem;
    ssl_certificate_key /etc/nginx/ssl/key.pem;

    location / {
        proxy_pass http://127.0.0.1:3000;
        proxy_http_version 1.1;
        proxy_set_header Upgrade $http_upgrade;
        proxy_set_header Connection "upgrade";
    }
}
```
