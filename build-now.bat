@echo off
chcp 65001 >nul 2>&1

set "PROJECT_ROOT=%~dp0"
set "PROJECT_ROOT=%PROJECT_ROOT:~0,-1%"
set "DEVTOOLS_DIR=%PROJECT_ROOT%\.devtools"

set "CARGO_HOME=%DEVTOOLS_DIR%\rust\cargo"
set "RUSTUP_HOME=%DEVTOOLS_DIR%\rust\rustup"

set "PATH=%DEVTOOLS_DIR%\cmake\bin;%PATH%"
set "PATH=%DEVTOOLS_DIR%\nasm;%PATH%"
set "PATH=%CARGO_HOME%\bin;%PATH%"
set "PATH=E:\vs\Common7\IDE\CommonExtensions\Microsoft\CMake\Ninja;%PATH%"
set "PATH=C:\Program Files (x86)\Windows Kits\10\bin\10.0.26100.0\x64;%PATH%"
set "PATH=E:\vs\VC\Tools\MSVC\14.44.35207\bin\HostX64\x64;%PATH%"

call "E:\vs\VC\Auxiliary\Build\vcvars64.bat" >nul 2>&1

set "CMAKE_GENERATOR=Ninja"
set "CC=cl.exe"
set "CXX=cl.exe"
set "AWS_LC_SYS_PREBUILT_NASM=1"

cd /d "%PROJECT_ROOT%"

echo Building agime-cli and agime-server...
cargo build --release -p agime-cli -p agime-server

if %ERRORLEVEL% EQU 0 (
    echo Build Successful!
    if exist "%PROJECT_ROOT%\target\release\agimed.exe" (
        echo Copying agimed.exe to ui\desktop\src\bin\
        copy /Y "%PROJECT_ROOT%\target\release\agimed.exe" "%PROJECT_ROOT%\ui\desktop\src\bin\agimed.exe" >nul
    )
    if exist "%PROJECT_ROOT%\target\release\agime.exe" (
        echo Copying agime.exe to ui\desktop\src\bin\
        copy /Y "%PROJECT_ROOT%\target\release\agime.exe" "%PROJECT_ROOT%\ui\desktop\src\bin\agime.exe" >nul
    )
    echo Done!
) else (
    echo Build Failed!
)
