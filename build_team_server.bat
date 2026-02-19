@echo off
call "E:\vs\VC\Auxiliary\Build\vcvarsall.bat" x64
cd /d E:\yw\agiatme\goose

set "DEVTOOLS_DIR=%~dp0.devtools"
set "CARGO_HOME=%DEVTOOLS_DIR%\rust\cargo"
set "RUSTUP_HOME=%DEVTOOLS_DIR%\rust\rustup"
set "NINJA_PATH=E:\vs\Common7\IDE\CommonExtensions\Microsoft\CMake\Ninja"
set "PATH=%NINJA_PATH%;%DEVTOOLS_DIR%\cmake\bin;%DEVTOOLS_DIR%\nasm;%CARGO_HOME%\bin;%PATH%"

set CMAKE_GENERATOR=Ninja
set CC=cl.exe
set CXX=cl.exe
set AWS_LC_SYS_PREBUILT_NASM=1

echo Building agime-team-server...
cargo build --release -p agime-team-server
if %ERRORLEVEL% NEQ 0 (
    echo Build agime-team-server FAILED
    goto :end
)

echo Building agime-cli (required for agent MCP extensions)...
cargo build --release -p agime-cli
if %ERRORLEVEL% EQU 0 (
    echo Build SUCCESS
) else (
    echo Build agime-cli FAILED
)

:end
