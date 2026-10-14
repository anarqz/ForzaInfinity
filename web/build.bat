@echo off
rem Compila a interface (web\dist), que o fh2-server serve em http://127.0.0.1:8081/ui/
rem Node portatil em Downloads\Tools; troque NODE se estiver em outro lugar.
set "NODE=%USERPROFILE%\Downloads\Tools\node-v24.21.0-win-x64"
if exist "%NODE%\node.exe" set "PATH=%NODE%;%PATH%"
where npm >nul 2>nul || ( echo Node nao encontrado. Ajuste NODE neste arquivo. & pause & exit /b 1 )
cd /d "%~dp0"
if not exist node_modules call npm install || ( pause & exit /b 1 )
call npm run build || ( pause & exit /b 1 )
echo OK: web\dist. Abra http://127.0.0.1:8081/ui/ com o servidor rodando.
pause
