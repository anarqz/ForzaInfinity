@echo off
rem ForzaInfinity: close the game, then double click to update to the latest release.
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0update.ps1" %*
if errorlevel 1 pause
