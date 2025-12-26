@echo off
chcp 65001 >nul 2>&1

echo ============================================================
echo   Cleaning build cache and rebuilding (Release)
echo ============================================================

cd /d E:\yw\agiatme\goose

echo.
echo Cleaning aws-lc-sys and ring cache...
for /d %%i in (target\release\build\aws-lc-sys*) do rmdir /s /q "%%i" 2>nul
for /d %%i in (target\release\build\ring*) do rmdir /s /q "%%i" 2>nul
echo Cache cleaned.

echo.
echo Loading Visual Studio environment...
call "E:\vs\VC\Auxiliary\Build\vcvars64.bat" >nul 2>&1

echo.
echo Setting build environment variables...
set CMAKE_GENERATOR=Ninja
set AWS_LC_SYS_PREBUILT_NASM=1
set CC=cl.exe
set CXX=cl.exe

echo CMAKE_GENERATOR=%CMAKE_GENERATOR%
echo AWS_LC_SYS_PREBUILT_NASM=%AWS_LC_SYS_PREBUILT_NASM%
echo CC=%CC%

echo.
echo Building agime-cli and agime-server (release)...
echo.

cargo build --release -p agime-cli -p agime-server

if %ERRORLEVEL% EQU 0 (
    echo.
    echo ============================================================
    echo   Build Successful!
    echo ============================================================

    if exist "target\release\agimed.exe" (
        echo Copying agimed.exe to ui\desktop\src\bin\
        copy /Y "target\release\agimed.exe" "ui\desktop\src\bin\agimed.exe" >nul
    )
    if exist "target\release\agime.exe" (
        echo Copying agime.exe to ui\desktop\src\bin\
        copy /Y "target\release\agime.exe" "ui\desktop\src\bin\agime.exe" >nul
    )
    echo.
    echo Done!
) else (
    echo.
    echo ============================================================
    echo   Build Failed!
    echo ============================================================
)
