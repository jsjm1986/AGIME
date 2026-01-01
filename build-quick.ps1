# Quick build script for agime
$ErrorActionPreference = "Stop"
$env:DEVTOOLS_DIR = "E:\yw\agiatme\goose\.devtools"
$env:CARGO_HOME = "$env:DEVTOOLS_DIR\rust\cargo"
$env:RUSTUP_HOME = "$env:DEVTOOLS_DIR\rust\rustup"
$env:CMAKE_GENERATOR = "Ninja"
$env:CC = "cl.exe"
$env:CXX = "cl.exe"
$env:AWS_LC_SYS_PREBUILT_NASM = "1"

$ninjaPath = "E:\vs\Common7\IDE\CommonExtensions\Microsoft\CMake\Ninja"
$env:PATH = "$ninjaPath;$env:DEVTOOLS_DIR\cmake\bin;$env:DEVTOOLS_DIR\nasm;$env:CARGO_HOME\bin;E:\vs\VC\Tools\MSVC\14.44.35207\bin\HostX64\x64;C:\Program Files (x86)\Windows Kits\10\bin\10.0.26100.0\x64;$env:PATH"

# Load VS environment
Push-Location "E:\vs\VC\Auxiliary\Build"
cmd /c "vcvars64.bat && set" | ForEach-Object {
    if ($_ -match "^([^=]+)=(.*)$") {
        [Environment]::SetEnvironmentVariable($matches[1], $matches[2], "Process")
    }
}
Pop-Location

Write-Host "Building agime and agime-server..." -ForegroundColor Cyan
Set-Location "E:\yw\agiatme\goose"
cargo build --release -p agime -p agime-server -j 4
if ($LASTEXITCODE -eq 0) {
    Write-Host "Build SUCCESS" -ForegroundColor Green
    Copy-Item -Force "target\release\agimed.exe" "ui\desktop\src\bin\agimed.exe"
    Write-Host "Copied agimed.exe"
} else {
    Write-Host "Build FAILED" -ForegroundColor Red
    exit 1
}
