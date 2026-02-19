@echo off
cd /d E:\yw\agiatme\goose
if not exist data\logs mkdir data\logs
call run_team_server.bat > data\logs\team-server.log 2>&1
