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
echo TEAM_MISSION_MIN_STEP_TIMEOUT_SECS: 300
echo TEAM_MISSION_COMPLEX_STEP_TIMEOUT_SECS: 900
echo TEAM_MISSION_MIN_GOAL_TIMEOUT_SECS: 600
echo.

set DATABASE_TYPE=mongodb
set DATABASE_URL=mongodb://localhost:27017
set DATABASE_NAME=agime_team
set TEAM_SERVER_HOST=0.0.0.0
set TEAM_SERVER_PORT=8080
set TEAM_MISSION_MIN_STEP_TIMEOUT_SECS=300
set TEAM_MISSION_COMPLEX_STEP_TIMEOUT_SECS=900
set TEAM_MISSION_MIN_GOAL_TIMEOUT_SECS=600

echo Starting agime-team-server...
echo.
cargo run -p agime-team-server

pause
