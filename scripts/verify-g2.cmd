@echo off
setlocal
cd /d "%~dp0.."
node scripts\verify-installed.mjs "%LOCALAPPDATA%\SoulDesk\souldesk.exe" --g2
set "SoulDeskVerifyExit=%ERRORLEVEL%"
pause
exit /b %SoulDeskVerifyExit%
