@echo off
call "E:\vs\VC\Auxiliary\Build\vcvars64.bat"
set DEVTOOLS_DIR=E:\yw\agiatme\goose\.devtools
set CARGO_HOME=%DEVTOOLS_DIR%\rust\cargo
set RUSTUP_HOME=%DEVTOOLS_DIR%\rust\rustup
set CMAKE_GENERATOR=Ninja
set AWS_LC_SYS_PREBUILT_NASM=1
set PATH=E:\vs\Common7\IDE\CommonExtensions\Microsoft\CMake\Ninja;%DEVTOOLS_DIR%\cmake\bin;%DEVTOOLS_DIR%\nasm;%CARGO_HOME%\bin;%PATH%
cd /d E:\yw\agiatme\goose
cargo build --release -p agime -p agime-server -j 4
if %ERRORLEVEL%==0 (
    echo Build SUCCESS
    copy /y target\release\agimed.exe ui\desktop\src\bin\agimed.exe
    echo Copied agimed.exe
) else (
    echo Build FAILED
    exit /b 1
)
