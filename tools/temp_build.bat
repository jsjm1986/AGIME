@echo off
call "E:\vs\VC\Auxiliary\Build\vcvars64.bat"
set PATH=%PATH%;E:\yw\agiatme\goose\tools\nasm-2.16.03
set CMAKE_GENERATOR=Ninja
cd /d E:\yw\agiatme\goose
cargo build -p agime-server -p agime-cli -j 4 2>&1
