@echo off
setlocal
cd /d "%~dp0.."
node scripts\verify-upgrade.mjs
set "SoulDeskVerifyExit=%ERRORLEVEL%"
pause
exit /b %SoulDeskVerifyExit%
