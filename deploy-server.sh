#!/bin/bash
# AGIME Team 服务器部署脚本
# 服务器: 101.36.109.148

set -e  # 遇到错误立即退出

echo "===== AGIME Team 服务器部署脚本 ====="
echo "目标服务器: 101.36.109.148"
echo "部署目录: /data/agime/"
echo ""

# 1. 创建目录结构
echo "[1/7] 创建目录结构..."
sudo mkdir -p /data/agime/{bin,data,backups,logs}
sudo chown -R ubuntu:ubuntu /data/agime

# 2. 检查二进制文件
echo "[2/7] 检查agimed文件..."
if [ ! -f "/data/agime/bin/agimed" ]; then
    echo "错误: agimed文件未找到，请先上传文件"
    exit 1
fi
sudo chmod +x /data/agime/bin/agimed

# 3. 创建systemd服务文件
echo "[3/7] 创建systemd服务..."
sudo tee /etc/systemd/system/agimed.service > /dev/null <<EOF
[Unit]
Description=AGIME Team Server
After=network.target

[Service]
Type=simple
User=ubuntu
Group=ubuntu
WorkingDirectory=/data/agime
Environment="RUST_LOG=info"
Environment="AGIME_SERVER_ADDR=0.0.0.0:7778"
Environment="AGIME_TEAM_API_URL=http://101.36.109.148:7778"
Environment="DATABASE_PATH=/data/agime/data/team_data.db"
ExecStart=/data/agime/bin/agimed
Restart=always
RestartSec=10
StandardOutput=append:/data/agime/logs/agimed.log
StandardError=append:/data/agime/logs/agimed.log

[Install]
WantedBy=multi-user.target
EOF

# 4. 创建备份脚本
echo "[4/7] 创建备份脚本..."
sudo tee /data/agime/backup.sh > /dev/null <<'EOF'
#!/bin/bash
BACKUP_DIR="/data/agime/backups"
DB_PATH="/data/agime/data/team_data.db"
DATE=$(date +%Y%m%d_%H%M%S)

mkdir -p $BACKUP_DIR
if [ -f "$DB_PATH" ]; then
    sqlite3 $DB_PATH ".backup $BACKUP_DIR/team_data_$DATE.db"
    echo "Backup created: $BACKUP_DIR/team_data_$DATE.db"
    
    # 保留最近7天的备份
    find $BACKUP_DIR -name "team_data_*.db" -mtime +7 -delete
fi
EOF

sudo chmod +x /data/agime/backup.sh
sudo chown ubuntu:ubuntu /data/agime/backup.sh

# 5. 配置防火墙
echo "[5/7] 配置防火墙..."
if command -v ufw &> /dev/null; then
    sudo ufw allow 7778/tcp comment 'AGIME Team Server'
    echo "防火墙规则已添加"
else
    echo "UFW未安装，跳过防火墙配置"
fi

# 6. 启动服务
echo "[6/7] 启动服务..."
sudo systemctl daemon-reload
sudo systemctl enable agimed
sudo systemctl start agimed

# 7. 等待服务启动
echo "[7/7] 等待服务启动..."
sleep 3

# 检查服务状态
echo ""
echo "===== 服务状态 ====="
sudo systemctl status agimed --no-pager || true

echo ""
echo "===== 最近日志 ====="
tail -n 20 /data/agime/logs/agimed.log || echo "日志文件暂未生成"

echo ""
echo "===== 部署完成 ====="
echo "服务地址: http://101.36.109.148:7778"
echo "健康检查: curl http://101.36.109.148:7778/api/team/health"
echo ""
echo "常用命令:"
echo "  查看状态: sudo systemctl status agimed"
echo "  查看日志: tail -f /data/agime/logs/agimed.log"
echo "  重启服务: sudo systemctl restart agimed"
echo "  停止服务: sudo systemctl stop agimed"
