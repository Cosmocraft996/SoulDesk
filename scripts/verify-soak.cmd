@echo off
setlocal
cd /d "%~dp0.."
node scripts\verify-soak.mjs 1800
set "SoulDeskVerifyExit=%ERRORLEVEL%"
pause
exit /b %SoulDeskVerifyExit%
