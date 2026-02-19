@echo off
call "E:\vs\VC\Auxiliary\Build\vcvarsall.bat" x64 >nul 2>&1

cd /d E:\yw\agiatme\goose

set "DEVTOOLS_DIR=E:\yw\agiatme\goose\.devtools"
set "CARGO_HOME=%DEVTOOLS_DIR%\rust\cargo"
set "RUSTUP_HOME=%DEVTOOLS_DIR%\rust\rustup"
set "NINJA_PATH=E:\vs\Common7\IDE\CommonExtensions\Microsoft\CMake\Ninja"
set "PATH=%NINJA_PATH%;%DEVTOOLS_DIR%\cmake\bin;%DEVTOOLS_DIR%\nasm;%CARGO_HOME%\bin;%PATH%"

set CMAKE_GENERATOR=Ninja
set CC=cl.exe
set CXX=cl.exe
set AWS_LC_SYS_PREBUILT_NASM=1

echo === Building agime-team-server === > E:\yw\agiatme\goose\_build.log 2>&1
cargo build --release -p agime-team-server >> E:\yw\agiatme\goose\_build.log 2>&1
if %ERRORLEVEL% NEQ 0 (
    echo BUILD_FAILED >> E:\yw\agiatme\goose\_build.log
) else (
    echo BUILD_SUCCESS >> E:\yw\agiatme\goose\_build.log
)
