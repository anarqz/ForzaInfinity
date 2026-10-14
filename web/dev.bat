@echo off
rem Modo desenvolvimento: recarrega ao editar o codigo. Precisa do fh2-server rodando (8081).
rem Abra http://localhost:5173/ui/
set "NODE=%USERPROFILE%\Downloads\Tools\node-v24.21.0-win-x64"
if exist "%NODE%\node.exe" set "PATH=%NODE%;%PATH%"
where npm >nul 2>nul || ( echo Node nao encontrado. Ajuste NODE neste arquivo. & pause & exit /b 1 )
cd /d "%~dp0"
if not exist node_modules call npm install || ( pause & exit /b 1 )
call npm run dev
