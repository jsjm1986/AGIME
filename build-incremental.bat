@echo off
chcp 65001 >nul 2>&1
REM ============================================================
REM AGIME Incremental Build Script (No Clean)
REM ============================================================

echo ============================================================
echo   AGIME Incremental Build Script
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
REM Add Windows SDK tools (rc.exe, mt.exe, etc.)
set "PATH=C:\Program Files (x86)\Windows Kits\10\bin\10.0.26100.0\x64;%PATH%"
REM Add MSVC compiler
set "PATH=E:\vs\VC\Tools\MSVC\14.44.35207\bin\HostX64\x64;%PATH%"

REM ============================================================
REM Set Visual Studio environment
REM ============================================================
echo Loading Visual Studio environment...
if exist "E:\vs\VC\Auxiliary\Build\vcvars64.bat" (
    call "E:\vs\VC\Auxiliary\Build\vcvars64.bat" >nul 2>&1
    if errorlevel 1 (
        echo vcvars64.bat failed, using manual environment setup
        goto :manual_env
    )
    echo Visual Studio environment loaded
    goto :env_done
) else (
    echo vcvars64.bat not found, using manual environment setup
    goto :manual_env
)

:manual_env
REM Manual environment setup as fallback
set "INCLUDE=E:\vs\VC\Tools\MSVC\14.44.35207\include;C:\Program Files (x86)\Windows Kits\10\Include\10.0.26100.0\ucrt;C:\Program Files (x86)\Windows Kits\10\Include\10.0.26100.0\um;C:\Program Files (x86)\Windows Kits\10\Include\10.0.26100.0\shared"
set "LIB=E:\vs\VC\Tools\MSVC\14.44.35207\lib\x64;C:\Program Files (x86)\Windows Kits\10\Lib\10.0.26100.0\ucrt\x64;C:\Program Files (x86)\Windows Kits\10\Lib\10.0.26100.0\um\x64"
echo Manual environment configured

:env_done

REM ============================================================
REM Set build options for aws-lc-sys
REM ============================================================
set "AWS_LC_SYS_NO_ASM=1"
set "CMAKE_GENERATOR=Ninja"
set "CC=cl.exe"
set "CXX=cl.exe"
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
REM Build (Incremental - No Clean!)
REM ============================================================
cd /d "%PROJECT_ROOT%"

echo.
echo Building agime-cli and agime-server (incremental)...
echo This should be fast if most files are already compiled...
echo.

cargo build -p goose-cli -p goose-server

REM ============================================================
REM Check result
REM ============================================================
if %ERRORLEVEL% EQU 0 (
    echo.
    echo ============================================================
    echo   Build Successful!
    echo ============================================================
    echo.

    if exist "%PROJECT_ROOT%\target\debug\goosed.exe" (
        echo Copying agimed.exe to ui\desktop\src\bin\
        copy /Y "%PROJECT_ROOT%\target\debug\goosed.exe" "%PROJECT_ROOT%\ui\desktop\src\bin\agimed.exe" >nul
    )
    if exist "%PROJECT_ROOT%\target\debug\goose.exe" (
        echo Copying agime.exe to ui\desktop\src\bin\
        copy /Y "%PROJECT_ROOT%\target\debug\goose.exe" "%PROJECT_ROOT%\ui\desktop\src\bin\agime.exe" >nul
    )

    echo.
    echo Done! You can now run:
    echo   cd ui\desktop
    echo   npm run start-gui
    echo.
    echo Or run the full app:
    echo   run-app.bat
    echo.
) else (
    echo.
    echo ============================================================
    echo   Build Failed!
    echo ============================================================
    echo.
    echo Please check the error messages above.
    echo.
)

pause
