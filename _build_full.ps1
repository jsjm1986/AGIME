# Full build script with manual VS environment
$ErrorActionPreference = "Continue"
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8

Write-Host "========================================" -ForegroundColor Cyan
Write-Host "  AGIME Full Build (PowerShell)"
Write-Host "========================================" -ForegroundColor Cyan

# Project paths
$ProjectRoot = "E:\yw\agiatme\goose"
$DevToolsDir = "$ProjectRoot\.devtools"

# Set Rust paths
$env:CARGO_HOME = "$DevToolsDir\rust\cargo"
$env:RUSTUP_HOME = "$DevToolsDir\rust\rustup"

# Visual Studio paths
$VSPath = "E:\vs"
$MSVCPath = "$VSPath\VC\Tools\MSVC\14.44.35207"
$WinSDKPath = "C:\Program Files (x86)\Windows Kits\10"
$WinSDKVersion = "10.0.26100.0"

# Set INCLUDE
$env:INCLUDE = @(
    "$MSVCPath\include",
    "$WinSDKPath\Include\$WinSDKVersion\ucrt",
    "$WinSDKPath\Include\$WinSDKVersion\um",
    "$WinSDKPath\Include\$WinSDKVersion\shared"
) -join ";"

# Set LIB
$env:LIB = @(
    "$MSVCPath\lib\x64",
    "$WinSDKPath\Lib\$WinSDKVersion\ucrt\x64",
    "$WinSDKPath\Lib\$WinSDKVersion\um\x64"
) -join ";"

# Set build options
$env:CMAKE_GENERATOR = "Ninja"
$env:CC = "cl.exe"
$env:CXX = "cl.exe"
$env:AWS_LC_SYS_PREBUILT_NASM = "1"

# Set PATH
$NinjaPath = "$VSPath\Common7\IDE\CommonExtensions\Microsoft\CMake\Ninja"
$env:PATH = @(
    "$NinjaPath",
    "$DevToolsDir\cmake\bin",
    "$DevToolsDir\nasm",
    "$env:CARGO_HOME\bin",
    "$MSVCPath\bin\HostX64\x64",
    "$WinSDKPath\bin\$WinSDKVersion\x64",
    $env:PATH
) -join ";"

Write-Host "[1/5] Environment configured manually" -ForegroundColor Yellow
Write-Host "      INCLUDE: $($env:INCLUDE.Substring(0, [Math]::Min(60, $env:INCLUDE.Length)))..." -ForegroundColor Gray
Write-Host "      LIB: $($env:LIB.Substring(0, [Math]::Min(50, $env:LIB.Length)))..." -ForegroundColor Gray

Write-Host "[2/5] Checking tool versions..." -ForegroundColor Yellow
& "$env:CARGO_HOME\bin\rustc.exe" --version
& "$env:CARGO_HOME\bin\cargo.exe" --version
cmake --version 2>&1 | Select-String "cmake version"
ninja --version 2>$null
cl.exe 2>&1 | Select-String "Microsoft" | Select-Object -First 1
Write-Host "      CMAKE_GENERATOR: $env:CMAKE_GENERATOR"

Write-Host "[3/5] Cleaning problematic caches..." -ForegroundColor Yellow
Set-Location $ProjectRoot
Get-ChildItem -Path "target\release\build" -Directory -Filter "aws-lc-sys*" -ErrorAction SilentlyContinue | Remove-Item -Recurse -Force
Get-ChildItem -Path "target\release\build" -Directory -Filter "ring*" -ErrorAction SilentlyContinue | Remove-Item -Recurse -Force
Write-Host "      Cache cleaned."

Write-Host "[4/5] Building ALL crates (release mode, -j 4)..." -ForegroundColor Yellow
Write-Host "      This may take 15-25 minutes..." -ForegroundColor Gray

# Build workspace first
& "$env:CARGO_HOME\bin\cargo.exe" build --release --workspace -j 4

# Then build agime-server with team feature enabled
if ($LASTEXITCODE -eq 0) {
    Write-Host "[4.5/5] Building agime-server with team feature..." -ForegroundColor Yellow
    & "$env:CARGO_HOME\bin\cargo.exe" build --release -p agime-server --features team -j 4
}

if ($LASTEXITCODE -eq 0) {
    Write-Host ""
    Write-Host "[5/5] Build successful! Copying binaries..." -ForegroundColor Green

    $binDir = "$ProjectRoot\ui\desktop\src\bin"
    if (-not (Test-Path $binDir)) {
        New-Item -ItemType Directory -Path $binDir -Force | Out-Null
    }

    Copy-Item -Force "$ProjectRoot\target\release\agimed.exe" "$binDir\agimed.exe"
    Copy-Item -Force "$ProjectRoot\target\release\agime.exe" "$binDir\agime.exe"

    # Copy Playwright runtime to target/release for development mode
    # Both Playwright modes (normal and extension mode) share the same runtime
    $playwrightSrc = "$ProjectRoot\ui\desktop\src\playwright"
    $playwrightDst = "$ProjectRoot\target\release\playwright"
    if (Test-Path $playwrightSrc) {
        Write-Host "      Copying Playwright runtime..." -ForegroundColor Gray
        if (Test-Path $playwrightDst) {
            Remove-Item -Recurse -Force $playwrightDst
        }
        Copy-Item -Recurse -Force $playwrightSrc $playwrightDst
        Write-Host "      Playwright runtime copied to target/release/playwright"
    } else {
        Write-Host "      Playwright runtime not found at $playwrightSrc" -ForegroundColor Yellow
        Write-Host "      Run 'npm run playwright:build' in ui/desktop to build it" -ForegroundColor Yellow
    }

    Write-Host "========================================"
    Write-Host "  BUILD COMPLETE!" -ForegroundColor Green
    Write-Host "========================================"

    Get-ChildItem "$binDir\*.exe" | ForEach-Object {
        Write-Host "  $($_.Name): $([Math]::Round($_.Length / 1MB, 1)) MB"
    }
} else {
    Write-Host ""
    Write-Host "========================================"
    Write-Host "  BUILD FAILED!" -ForegroundColor Red
    Write-Host "========================================"
    exit 1
}
