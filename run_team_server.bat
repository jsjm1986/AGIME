@echo off
cd /d E:\yw\agiatme\goose

echo Setting up Visual Studio environment...
call "E:\vs\VC\Auxiliary\Build\vcvarsall.bat" x64

echo.
echo Adding Rust toolchain to PATH...
set "PATH=%~dp0.devtools\rust\cargo\bin;%PATH%"

echo.
echo === Server Configuration ===
echo DATABASE_TYPE: mongodb (default)
echo DATABASE_URL: mongodb://localhost:27017
echo.

set DATABASE_TYPE=mongodb
set DATABASE_URL=mongodb://localhost:27017
set DATABASE_NAME=agime_team
set TEAM_SERVER_HOST=0.0.0.0
set TEAM_SERVER_PORT=8080

echo Starting agime-team-server...
echo.
cargo run -p agime-team-server

pause
