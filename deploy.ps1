# AGIME Team 服务器部署脚本 - Windows版本
# 使用PowerShell执行

$SERVER = "101.36.109.148"
$USER = "ubuntu"
$REMOTE_DIR = "/data/agime"
$LOCAL_BIN = "target\release\agimed.exe"

Write-Host "===== AGIME Team 部署脚本 =====" -ForegroundColor Green
Write-Host "目标服务器: $SERVER"
Write-Host "用户: $USER"
Write-Host ""

# 检查本地二进制文件
Write-Host "[1/4] 检查本地编译文件..." -ForegroundColor Cyan
if (-not (Test-Path $LOCAL_BIN)) {
    Write-Host "错误: 未找到编译后的文件 $LOCAL_BIN" -ForegroundColor Red
    Write-Host "请先运行: cargo build --release --features team" -ForegroundColor Yellow
    exit 1
}
Write-Host "✓ 文件已找到: $LOCAL_BIN" -ForegroundColor Green

# 使用scp上传文件（需要密码：jsjm4082880）
Write-Host ""
Write-Host "[2/4] 上传agimed到服务器..." -ForegroundColor Cyan
Write-Host "提示: 输入密码 jsjm4082880" -ForegroundColor Yellow

# 先创建远程目录
ssh ${USER}@${SERVER} "mkdir -p ${REMOTE_DIR}/bin"

# 上传二进制文件（重命名去掉.exe后缀）
scp $LOCAL_BIN ${USER}@${SERVER}:${REMOTE_DIR}/bin/agimed

# 上传部署脚本
Write-Host ""
Write-Host "[3/4] 上传部署脚本..." -ForegroundColor Cyan
scp deploy-server.sh ${USER}@${SERVER}:${REMOTE_DIR}/deploy-server.sh

# 执行部署脚本
Write-Host ""
Write-Host "[4/4] 执行服务器配置..." -ForegroundColor Cyan
ssh ${USER}@${SERVER} "cd ${REMOTE_DIR} && chmod +x deploy-server.sh && sudo ./deploy-server.sh"

Write-Host ""
Write-Host "===== 部署完成 =====" -ForegroundColor Green
Write-Host "服务地址: http://${SERVER}:7778" -ForegroundColor Cyan
Write-Host ""
Write-Host "测试健康检查:" -ForegroundColor Yellow
Write-Host "  curl http://${SERVER}:7778/api/team/health"
Write-Host ""
Write-Host "SSH到服务器查看状态:" -ForegroundColor Yellow  
Write-Host "  ssh ${USER}@${SERVER}"
Write-Host "  sudo systemctl status agimed"
Write-Host "  tail -f /data/agime/logs/agimed.log"
