@echo off
setlocal
cd /d "%~dp0.."
node scripts\verify-upgrade.mjs --resume-only --data-only
set "SoulDeskVerifyExit=%ERRORLEVEL%"
pause
exit /b %SoulDeskVerifyExit%
