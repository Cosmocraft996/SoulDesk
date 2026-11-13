@echo off
setlocal
cd /d "%~dp0.."
node scripts\verify-installed.mjs "%LOCALAPPDATA%\SoulDesk\souldesk.exe" --g1
set "SoulDeskVerifyExit=%ERRORLEVEL%"
pause
exit /b %SoulDeskVerifyExit%
