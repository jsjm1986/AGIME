@echo off
chcp 65001 >nul 2>&1
cd /d E:\yw\agiatme\goose

echo Loading VS environment...
call "E:\vs\VC\Auxiliary\Build\vcvars64.bat"
if errorlevel 1 (
    echo Failed to load VS environment
    exit /b 1
)

set CMAKE_GENERATOR=Ninja
set CC=cl.exe
set CXX=cl.exe
set AWS_LC_SYS_PREBUILT_NASM=1
set "PATH=E:\yw\agiatme\goose\.devtools\cmake\bin;E:\yw\agiatme\goose\.devtools\nasm;E:\vs\Common7\IDE\CommonExtensions\Microsoft\CMake\Ninja;%PATH%"

echo.
echo Starting cargo build...
cargo build --release -p agime-cli -p agime-server

if %ERRORLEVEL% EQU 0 (
    echo.
    echo Build SUCCESS!
    copy /Y "target\release\agimed.exe" "ui\desktop\src\bin\agimed.exe" >nul 2>&1
    copy /Y "target\release\agime.exe" "ui\desktop\src\bin\agime.exe" >nul 2>&1
    echo Binaries copied.
) else (
    echo.
    echo Build FAILED!
)
