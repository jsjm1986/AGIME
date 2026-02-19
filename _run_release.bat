@echo off
cd /d E:\yw\agiatme\goose
if not exist data\logs mkdir data\logs

set DATABASE_TYPE=mongodb
set DATABASE_URL=mongodb://localhost:27017
set DATABASE_NAME=agime_team
set TEAM_SERVER_HOST=0.0.0.0
set TEAM_SERVER_PORT=8080

target\release\agime-team-server.exe > data\logs\server-new.log 2>&1
