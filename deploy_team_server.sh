#!/bin/bash
# AGIME Team Server 部署脚本
# 服务器: 101.36.109.148

set -e

echo "=========================================="
echo "  AGIME Team Server 部署脚本"
echo "=========================================="

# 创建工作目录
WORK_DIR="/data/agime-team"
mkdir -p $WORK_DIR
cd $WORK_DIR

echo "[1/6] 检查系统环境..."
echo "内存: $(free -h | grep Mem | awk '{print $2}')"
echo "磁盘: $(df -h / | tail -1 | awk '{print $4}') 可用"

echo "[2/6] 安装 Docker..."
if ! command -v docker &> /dev/null; then
    echo "正在安装 Docker..."
    curl -fsSL https://get.docker.com | sh
    sudo usermod -aG docker $USER
    sudo systemctl enable docker
    sudo systemctl start docker
    echo "Docker 安装完成"
else
    echo "Docker 已安装: $(docker --version)"
fi

echo "[3/6] 创建数据目录..."
mkdir -p $WORK_DIR/data
chmod 777 $WORK_DIR/data

echo "[4/6] 创建 docker-compose.yml..."
cat > $WORK_DIR/docker-compose.yml << 'EOF'
version: '3.8'
services:
  team-server:
    image: rust:1.82-slim
    container_name: agime-team-server
    working_dir: /app
    command: >
      bash -c "
        apt-get update && apt-get install -y pkg-config libssl-dev sqlite3 curl &&
        if [ ! -f /app/agime-team-server ]; then
          echo 'Binary not found, please upload it first';
          sleep infinity;
        else
          chmod +x /app/agime-team-server &&
          /app/agime-team-server;
        fi
      "
    ports:
      - "8080:8080"
    environment:
      - TEAM_SERVER_HOST=0.0.0.0
      - TEAM_SERVER_PORT=8080
      - DATABASE_URL=sqlite:///data/team.db?mode=rwc
      - DATABASE_MAX_CONNECTIONS=10
      - RUST_LOG=agime_team_server=info,tower_http=debug
    volumes:
      - ./data:/data
      - ./bin:/app
    restart: unless-stopped
    healthcheck:
      test: ["CMD", "curl", "-f", "http://localhost:8080/health"]
      interval: 30s
      timeout: 10s
      retries: 3
      start_period: 30s
EOF

echo "[5/6] 创建简化版服务（使用预编译二进制）..."
mkdir -p $WORK_DIR/bin

echo "[6/6] 部署完成！"
echo ""
echo "下一步："
echo "1. 上传 agime-team-server 二进制文件到 $WORK_DIR/bin/"
echo "2. 运行: cd $WORK_DIR && docker-compose up -d"
echo "3. 检查: curl http://localhost:8080/health"
echo ""
echo "服务地址: http://101.36.109.148:8080"
