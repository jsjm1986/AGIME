#!/bin/bash
# AGIME Team Server 一键部署脚本
# 在服务器上执行: curl -sSL https://raw.githubusercontent.com/... | bash
# 或者手动执行此脚本

set -e
WORK_DIR="/data/agime-team"

echo "=== [1/5] 创建目录 ==="
sudo mkdir -p $WORK_DIR/{data,bin}
sudo chown -R $USER:$USER $WORK_DIR
cd $WORK_DIR

echo "=== [2/5] 安装 Docker ==="
if ! command -v docker &> /dev/null; then
    curl -fsSL https://get.docker.com | sudo sh
    sudo usermod -aG docker $USER
    sudo systemctl enable docker
    sudo systemctl start docker
fi
docker --version

echo "=== [3/5] 创建 Dockerfile ==="
cat > Dockerfile << 'DOCKERFILE'
FROM rust:1.82-slim AS builder
WORKDIR /build
RUN apt-get update && apt-get install -y pkg-config libssl-dev git
RUN git clone --depth 1 https://github.com/jsjm1986/AGIME.git .
RUN cargo build --release -p agime-team-server

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y ca-certificates libssl3 curl && rm -rf /var/lib/apt/lists/*
RUN useradd -m -u 1000 agime
COPY --from=builder /build/target/release/agime-team-server /usr/local/bin/
RUN mkdir -p /data && chown agime:agime /data
USER agime
WORKDIR /data
ENV TEAM_SERVER_HOST=0.0.0.0 TEAM_SERVER_PORT=8080
ENV DATABASE_URL=sqlite:///data/team.db?mode=rwc
EXPOSE 8080
HEALTHCHECK --interval=30s --timeout=10s CMD curl -f http://localhost:8080/health || exit 1
CMD ["agime-team-server"]
DOCKERFILE

echo "=== [4/5] 构建镜像 ==="
sudo docker build -t agime-team-server:latest .

echo "=== [5/5] 启动服务 ==="
sudo docker rm -f agime-team-server 2>/dev/null || true
sudo docker run -d \
  --name agime-team-server \
  --restart unless-stopped \
  -p 8080:8080 \
  -v $WORK_DIR/data:/data \
  -e RUST_LOG=info \
  agime-team-server:latest

echo ""
echo "=== 部署完成 ==="
echo "服务地址: http://$(curl -s ifconfig.me):8080"
echo "健康检查: curl http://localhost:8080/health"
