@echo off
call "E:\vs\VC\Auxiliary\Build\vcvarsall.bat" x64
cd /d E:\yw\agiatme\goose
set CMAKE_GENERATOR=Ninja
set CMAKE_C_COMPILER=cl.exe
set CMAKE_CXX_COMPILER=cl.exe
cargo build --release -p agime-team-server
