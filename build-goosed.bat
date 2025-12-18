@echo off
chcp 65001 >nul 2>&1
REM ============================================================
REM Goose Backend Build Script
REM ============================================================

echo ============================================================
echo   Goose Backend Build Script
echo ============================================================
echo.

REM Get script directory (project root)
set "PROJECT_ROOT=%~dp0"
set "PROJECT_ROOT=%PROJECT_ROOT:~0,-1%"
set "DEVTOOLS_DIR=%PROJECT_ROOT%\.devtools"

REM ============================================================
REM Set local Rust environment
REM ============================================================
set "CARGO_HOME=%DEVTOOLS_DIR%\rust\cargo"
set "RUSTUP_HOME=%DEVTOOLS_DIR%\rust\rustup"

REM ============================================================
REM Set PATH - use local tools
REM ============================================================
set "PATH=%DEVTOOLS_DIR%\cmake\bin;%PATH%"
set "PATH=%DEVTOOLS_DIR%\nasm;%PATH%"
set "PATH=%CARGO_HOME%\bin;%PATH%"
REM Add Ninja from Visual Studio
set "PATH=E:\vs\Common7\IDE\CommonExtensions\Microsoft\CMake\Ninja;%PATH%"

REM ============================================================
REM Set Visual Studio environment
REM ============================================================
echo Loading Visual Studio environment...
if exist "E:\vs\VC\Auxiliary\Build\vcvars64.bat" (
    call "E:\vs\VC\Auxiliary\Build\vcvars64.bat" >nul 2>&1
    echo Visual Studio environment loaded
) else (
    echo Error: Visual Studio not found
    echo Please make sure Visual Studio is installed at E:\vs
    pause
    exit /b 1
)

REM ============================================================
REM Set build options for aws-lc-sys
REM ============================================================
set "AWS_LC_SYS_NO_ASM=1"

REM Use Ninja generator instead of Visual Studio generator
REM This fixes the "No CMAKE_C_COMPILER could be found" error
set "CMAKE_GENERATOR=Ninja"

REM Explicitly set C/C++ compiler paths
set "CC=cl.exe"
set "CXX=cl.exe"

REM Use prebuilt NASM objects to avoid NASM compilation issues
set "AWS_LC_SYS_PREBUILT_NASM=1"

REM ============================================================
REM Show tool versions
REM ============================================================
echo.
echo Tool versions:
echo ----------------------------------------
rustc --version
cargo --version
cmake --version 2>&1 | findstr "cmake version"
ninja --version
echo CC=%CC%
echo CXX=%CXX%
echo CMAKE_GENERATOR=%CMAKE_GENERATOR%
echo ----------------------------------------
echo.

REM ============================================================
REM Build
REM ============================================================
cd /d "%PROJECT_ROOT%"

echo Cleaning old build files...
cargo clean

echo.
echo Building goose-server...
echo This may take a few minutes, please wait...
echo.

cargo build -p goose-server

REM ============================================================
REM Check result
REM ============================================================
if exist "%PROJECT_ROOT%\target\debug\goosed.exe" (
    echo.
    echo ============================================================
    echo   Build Successful!
    echo ============================================================
    echo.
    echo Copying goosed.exe to ui\desktop\src\bin\
    copy /Y "%PROJECT_ROOT%\target\debug\goosed.exe" "%PROJECT_ROOT%\ui\desktop\src\bin\goosed.exe"
    echo.
    echo Done! You can now run:
    echo   cd ui\desktop
    echo   npm run start-gui
    echo.
) else (
    echo.
    echo ============================================================
    echo   Build Failed!
    echo ============================================================
    echo.
    echo goosed.exe not found. Please check the error messages above.
    echo.
)

pause
