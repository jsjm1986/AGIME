@echo off
chcp 65001 >nul 2>&1
cd /d E:\yw\agiatme\goose

set "DEVTOOLS_DIR=%~dp0.devtools"
set "CARGO_HOME=%DEVTOOLS_DIR%\rust\cargo"
set "RUSTUP_HOME=%DEVTOOLS_DIR%\rust\rustup"
set "NINJA_PATH=E:\vs\Common7\IDE\CommonExtensions\Microsoft\CMake\Ninja"
set "PATH=%NINJA_PATH%;%DEVTOOLS_DIR%\cmake\bin;%DEVTOOLS_DIR%\nasm;%CARGO_HOME%\bin;%PATH%"
set "PATH=C:\Program Files (x86)\Windows Kits\10\bin\10.0.26100.0\x64;%PATH%"
set "PATH=E:\vs\VC\Tools\MSVC\14.44.35207\bin\HostX64\x64;%PATH%"

call "E:\vs\VC\Auxiliary\Build\vcvars64.bat" >nul 2>&1

set CMAKE_GENERATOR=Ninja
set CC=cl.exe
set CXX=cl.exe
set AWS_LC_SYS_PREBUILT_NASM=1

echo ========================================
echo Starting full workspace build...
echo ========================================
echo.

cargo build --release --workspace -j 4

if %ERRORLEVEL% EQU 0 (
    echo.
    echo ========================================
    echo BUILD SUCCESS!
    echo ========================================
    if not exist "ui\desktop\src\bin" mkdir "ui\desktop\src\bin"
    copy /Y target\release\agimed.exe ui\desktop\src\bin\agimed.exe >nul
    copy /Y target\release\agime.exe ui\desktop\src\bin\agime.exe >nul
    echo Binaries copied to ui\desktop\src\bin\
    dir ui\desktop\src\bin\*.exe
) else (
    echo.
    echo ========================================
    echo BUILD FAILED with error code %ERRORLEVEL%
    echo ========================================
)
