@echo off
rem Runs the server listening on the network (LAN play). The log shows here and is also kept in
rem logs\server-<date>-<time>.log, so it can be read while (or after) the server runs.
cd /d "%~dp0"
if not exist target\release\fh2-server.exe ( echo Rode build.bat primeiro. & pause & exit /b 1 )
if not exist logs mkdir logs
powershell -NoProfile -ExecutionPolicy Bypass -Command "$log = 'logs\server-' + (Get-Date -Format 'yyyyMMdd-HHmmss') + '.log'; Write-Host ('log: ' + $log); & '.\target\release\fh2-server.exe' --listen 0.0.0.0:8081 --root . --capture 2>&1 | ForEach-Object { $line = \"$_\"; Write-Host $line; Add-Content -Path $log -Value $line -Encoding UTF8 }"
pause
