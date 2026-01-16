# AGIME Team Server

独立的团队协作服务器，支持多用户共享 Skills、Recipes 和 Extensions。

## 快速开始

### 使用 Docker (推荐)

```bash
cd crates/agime-team-server
docker-compose up -d
```

### 本地运行

```bash
# 编译
cargo build --release -p agime-team-server

# 运行
./target/release/agime-team-server
```

## 配置

通过环境变量配置：

| 变量 | 默认值 | 说明 |
|------|--------|------|
| `TEAM_SERVER_HOST` | `0.0.0.0` | 监听地址 |
| `TEAM_SERVER_PORT` | `8080` | 监听端口 |
| `DATABASE_URL` | `sqlite://./data/team.db` | 数据库连接 |
| `DATABASE_MAX_CONNECTIONS` | `10` | 最大连接数 |
| `RUST_LOG` | `info` | 日志级别 |

## API 接口

### 公开接口

- `GET /` - 服务器信息
- `GET /health` - 健康检查
- `POST /api/auth/register` - 注册用户

### 需要认证的接口

使用 `X-API-Key` 请求头或 `Authorization: Bearer <key>` 认证。

#### 用户管理
- `GET /api/auth/me` - 获取当前用户
- `GET /api/auth/keys` - 列出 API Keys
- `POST /api/auth/keys` - 创建新 API Key
- `DELETE /api/auth/keys/{id}` - 删除 API Key

#### 团队 API
- `GET /api/team/teams` - 列出团队
- `POST /api/team/teams` - 创建团队
- 更多接口参见 agime-team 文档

## 使用示例

### 1. 注册用户

```bash
curl -X POST http://localhost:8080/api/auth/register \
  -H "Content-Type: application/json" \
  -d '{"email":"user@example.com","display_name":"Test User"}'
```

响应:
```json
{
  "user": {
    "id": "...",
    "email": "user@example.com",
    "display_name": "Test User"
  },
  "api_key": "agime_xxx_xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx",
  "message": "Save your API key securely. It will only be shown once."
}
```

### 2. 使用 API Key

```bash
curl http://localhost:8080/api/team/teams \
  -H "X-API-Key: agime_xxx_xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx"
```

## 客户端配置

在 AGIME 桌面应用中配置：

1. 打开设置 → 团队服务器
2. 输入服务器地址: `http://your-server:8080`
3. 输入 API Key
4. 点击测试连接
5. 保存

或通过环境变量：

```bash
export AGIME_TEAM_SERVER_URL=http://your-server:8080
export AGIME_TEAM_API_KEY=agime_xxx_...
```

## 安全建议

- 生产环境使用 HTTPS
- 使用反向代理 (nginx/traefik)
- 定期轮换 API Keys
- 启用防火墙限制访问
