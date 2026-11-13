@echo off
setlocal
cd /d "%~dp0.."
node scripts\verify-installed.mjs "%LOCALAPPDATA%\SoulDesk\souldesk.exe" --g3
set "SoulDeskVerifyExit=%ERRORLEVEL%"
pause
exit /b %SoulDeskVerifyExit%
