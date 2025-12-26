@echo off
chcp 65001 >nul 2>&1
cd /d E:\yw\agiatme\goose

echo Loading Visual Studio environment...
call "E:\vs\VC\Auxiliary\Build\vcvars64.bat" >nul 2>&1
echo VS environment loaded.

set CMAKE_GENERATOR=Ninja
set CC=cl.exe
set CXX=cl.exe
set AWS_LC_SYS_PREBUILT_NASM=1
set "PATH=E:\yw\agiatme\goose\.devtools\cmake\bin;E:\yw\agiatme\goose\.devtools\nasm;E:\vs\Common7\IDE\CommonExtensions\Microsoft\CMake\Ninja;%PATH%"

echo Environment variables set:
echo CMAKE_GENERATOR=%CMAKE_GENERATOR%
echo CC=%CC%
echo.
echo Starting cargo build...
cargo build --release -p agime-cli -p agime-server
echo.
echo Build completed with exit code: %ERRORLEVEL%
if %ERRORLEVEL% EQU 0 (
    echo SUCCESS!
    copy /Y "E:\yw\agiatme\goose\target\release\agimed.exe" "E:\yw\agiatme\goose\ui\desktop\src\bin\agimed.exe" >nul 2>&1
    copy /Y "E:\yw\agiatme\goose\target\release\agime.exe" "E:\yw\agiatme\goose\ui\desktop\src\bin\agime.exe" >nul 2>&1
    echo Binaries copied to ui\desktop\src\bin\
) else (
    echo FAILED!
)
