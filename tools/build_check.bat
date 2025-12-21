@echo off
setlocal enabledelayedexpansion

echo Setting up VS environment... > E:\yw\agiatme\goose\tools\build_output.txt
call "E:\vs\VC\Auxiliary\Build\vcvars64.bat" >> E:\yw\agiatme\goose\tools\build_output.txt 2>&1
if errorlevel 1 (
    echo Failed to set up VS environment >> E:\yw\agiatme\goose\tools\build_output.txt
    exit /b 1
)

echo Adding NASM to PATH... >> E:\yw\agiatme\goose\tools\build_output.txt
set "PATH=%PATH%;E:\yw\agiatme\goose\tools\nasm-2.16.03"

echo Changing directory... >> E:\yw\agiatme\goose\tools\build_output.txt
cd /d E:\yw\agiatme\goose

echo Running cargo check... >> E:\yw\agiatme\goose\tools\build_output.txt
cargo check -p goose --lib >> E:\yw\agiatme\goose\tools\build_output.txt 2>&1

echo. >> E:\yw\agiatme\goose\tools\build_output.txt
echo Build check completed. Exit code: %ERRORLEVEL% >> E:\yw\agiatme\goose\tools\build_output.txt
