@echo off
setlocal
cd /d "%~dp0.."
start "" "%CD%\src-tauri\target\debug\souldesk.exe" --desktop-integration-widgetjs-manual "%CD%\artifacts\desktop-integration-widgetjs-manual" --show
