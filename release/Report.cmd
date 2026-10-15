@echo off
rem ForzaInfinity: writes ForzaInfinity-Report.txt (the last launch) on the Desktop, to send to whoever helps you.
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0report.ps1" %*
pause
