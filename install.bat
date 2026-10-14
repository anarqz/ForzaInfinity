@echo off
setlocal
rem Instala o azimuth-tls.dll do FH2Online no Azimuth (com backup do original).
set "HERE=%~dp0"
set "BIN=%HERE%..\azimuth-windows\data\bin"
set "EO=%HERE%..\ExtraOptionsRelease\Azimuth"
if not exist "%BIN%\azimuth-tls.dll" ( echo Nao achei %BIN%\azimuth-tls.dll & pause & exit /b 1 )
if not exist "%BIN%\azimuth-tls.dll.stock" copy /y "%BIN%\azimuth-tls.dll" "%BIN%\azimuth-tls.dll.stock" >nul
copy /y "%HERE%bin\azimuth-tls.dll" "%BIN%\azimuth-tls.dll" >nul
if not exist "%BIN%\FH2_ExtraOptions.ini" (
  copy /y "%EO%\FH2_ExtraOptions.ini" "%BIN%\FH2_ExtraOptions.ini" >nul
  type "%HERE%bin\network-section.ini" >> "%BIN%\FH2_ExtraOptions.ini"
)
if not exist "%BIN%\Gamerpic.png" if exist "%EO%\Gamerpic.png" copy /y "%EO%\Gamerpic.png" "%BIN%\" >nul
echo Instalado. Edite [Network] em %BIN%\FH2_ExtraOptions.ini
pause
