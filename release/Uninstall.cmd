@echo off
rem ForzaInfinity: double click to put back the original azimuth-tls.dll.
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0uninstall.ps1" %*
if errorlevel 1 pause
