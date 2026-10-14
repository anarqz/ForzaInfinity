@echo off
cd /d "%~dp0"
if not exist target\release\fh2-server.exe ( echo Rode build.bat primeiro. & pause & exit /b 1 )
target\release\fh2-server.exe --listen 0.0.0.0:8081 --root . --capture
pause
