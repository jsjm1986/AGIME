@echo off
chcp 65001 >nul 2>&1
REM ============================================================
REM AGIME Full Build Script (Memory-Friendly)
REM
REM Features:
REM   - Builds ALL crates (not just cli and server)
REM   - Limited parallelism (-j 4) to control memory usage
REM   - Cleans problematic caches before build
REM   - Copies binaries to UI directory
REM ============================================================

echo ============================================================
echo   AGIME Full Build Script
echo   Memory Mode: -j 4 (适合 12GB 内存)
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
REM Set PATH - use local tools (Ninja path is critical!)
REM ============================================================
set "NINJA_PATH=E:\vs\Common7\IDE\CommonExtensions\Microsoft\CMake\Ninja"
set "PATH=%NINJA_PATH%;%DEVTOOLS_DIR%\cmake\bin;%DEVTOOLS_DIR%\nasm;%CARGO_HOME%\bin;%PATH%"
REM Add Windows SDK tools (rc.exe, mt.exe, etc.)
set "PATH=C:\Program Files (x86)\Windows Kits\10\bin\10.0.26100.0\x64;%PATH%"
REM Add MSVC compiler
set "PATH=E:\vs\VC\Tools\MSVC\14.44.35207\bin\HostX64\x64;%PATH%"

REM ============================================================
REM Load Visual Studio environment
REM ============================================================
echo [1/6] Loading Visual Studio environment...
if exist "E:\vs\VC\Auxiliary\Build\vcvars64.bat" (
    call "E:\vs\VC\Auxiliary\Build\vcvars64.bat" >nul 2>&1
    if errorlevel 1 (
        echo WARNING: vcvars64.bat failed, using manual environment setup
        goto :manual_env
    )
    echo       Visual Studio environment loaded
    goto :env_done
) else (
    echo WARNING: vcvars64.bat not found, using manual environment setup
    goto :manual_env
)

:manual_env
REM Manual environment setup as fallback
set "INCLUDE=E:\vs\VC\Tools\MSVC\14.44.35207\include;C:\Program Files (x86)\Windows Kits\10\Include\10.0.26100.0\ucrt;C:\Program Files (x86)\Windows Kits\10\Include\10.0.26100.0\um;C:\Program Files (x86)\Windows Kits\10\Include\10.0.26100.0\shared"
set "LIB=E:\vs\VC\Tools\MSVC\14.44.35207\lib\x64;C:\Program Files (x86)\Windows Kits\10\Lib\10.0.26100.0\ucrt\x64;C:\Program Files (x86)\Windows Kits\10\Lib\10.0.26100.0\um\x64"
echo       Manual environment configured

:env_done

REM ============================================================
REM Set build options (NO trailing spaces!)
REM ============================================================
set CMAKE_GENERATOR=Ninja
set CC=cl.exe
set CXX=cl.exe
set AWS_LC_SYS_PREBUILT_NASM=1

REM ============================================================
REM Show tool versions
REM ============================================================
echo.
echo [2/6] Checking tool versions...
echo ----------------------------------------
rustc --version
cargo --version
cmake --version 2>&1 | findstr "cmake version"
ninja --version 2>nul || echo WARNING: ninja not found in PATH
where cl.exe >nul 2>&1 && echo cl.exe: found || echo WARNING: cl.exe not found
echo CMAKE_GENERATOR=%CMAKE_GENERATOR%
echo ----------------------------------------
echo.

REM ============================================================
REM Clean problematic caches (FULL clean for aws-lc-sys)
REM ============================================================
echo [3/6] Cleaning build caches...
cd /d "%PROJECT_ROOT%"

echo       Cleaning aws-lc-sys cache...
for /d %%i in (target\release\build\aws-lc-sys*) do (
    echo       Removing %%i
    rmdir /s /q "%%i" 2>nul
)
for /d %%i in (target\debug\build\aws-lc-sys*) do (
    echo       Removing %%i
    rmdir /s /q "%%i" 2>nul
)

echo       Cleaning ring cache...
for /d %%i in (target\release\build\ring*) do (
    echo       Removing %%i
    rmdir /s /q "%%i" 2>nul
)
for /d %%i in (target\debug\build\ring*) do (
    echo       Removing %%i
    rmdir /s /q "%%i" 2>nul
)

echo       Cache cleaned.
echo.

REM ============================================================
REM Build ALL crates with limited parallelism
REM ============================================================
echo [4/6] Building ALL crates (release mode, -j 4)...
echo       This may take 15-25 minutes...
echo.

cargo build --release --workspace -j 4

REM ============================================================
REM Check build result
REM ============================================================
if %ERRORLEVEL% NEQ 0 (
    echo.
    echo ============================================================
    echo   BUILD FAILED!
    echo ============================================================
    echo.
    echo Please check the error messages above.
    echo.
    goto :end
)

echo.
echo [5/6] Build successful! Copying binaries...

REM ============================================================
REM Copy binaries to UI directory
REM ============================================================
if not exist "%PROJECT_ROOT%\ui\desktop\src\bin" (
    mkdir "%PROJECT_ROOT%\ui\desktop\src\bin"
)

if exist "%PROJECT_ROOT%\target\release\agimed.exe" (
    echo       Copying agimed.exe...
    copy /Y "%PROJECT_ROOT%\target\release\agimed.exe" "%PROJECT_ROOT%\ui\desktop\src\bin\agimed.exe" >nul
) else (
    echo WARNING: agimed.exe not found!
)

if exist "%PROJECT_ROOT%\target\release\agime.exe" (
    echo       Copying agime.exe...
    copy /Y "%PROJECT_ROOT%\target\release\agime.exe" "%PROJECT_ROOT%\ui\desktop\src\bin\agime.exe" >nul
) else (
    echo WARNING: agime.exe not found!
)

echo.
echo [6/6] Verifying binaries...
echo ----------------------------------------
if exist "%PROJECT_ROOT%\ui\desktop\src\bin\agimed.exe" (
    echo agimed.exe: OK
    for %%A in ("%PROJECT_ROOT%\ui\desktop\src\bin\agimed.exe") do echo   Size: %%~zA bytes
) else (
    echo agimed.exe: MISSING
)
if exist "%PROJECT_ROOT%\ui\desktop\src\bin\agime.exe" (
    echo agime.exe:  OK
    for %%A in ("%PROJECT_ROOT%\ui\desktop\src\bin\agime.exe") do echo   Size: %%~zA bytes
) else (
    echo agime.exe:  MISSING
)
echo ----------------------------------------

echo.
echo ============================================================
echo   BUILD COMPLETE!
echo ============================================================
echo.
echo To run in developer mode:
echo   cd ui\desktop
echo   npm run start-gui
echo.

:end
pause
