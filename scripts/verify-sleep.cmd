@echo off
setlocal
cd /d "%~dp0.."
artifacts\sleep-probe.exe
set "SoulDeskVerifyExit=%ERRORLEVEL%"
pause
exit /b %SoulDeskVerifyExit%
