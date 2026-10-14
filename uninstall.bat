@echo off
set "BIN=%~dp0..\azimuth-windows\data\bin"
if exist "%BIN%\azimuth-tls.dll.stock" ( copy /y "%BIN%\azimuth-tls.dll.stock" "%BIN%\azimuth-tls.dll" >nul & echo DLL original do Azimuth restaurado. ) else echo Sem backup .stock
pause
