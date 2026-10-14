@echo off
rem ForzaInfinity: double click to install (asks for the Azimuth folder and the server).
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0install.ps1" %*
if errorlevel 1 pause
