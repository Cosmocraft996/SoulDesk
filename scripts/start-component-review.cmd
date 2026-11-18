@echo off
setlocal
chcp 65001 >nul
cd /d "%~dp0.."
powershell.exe -NoProfile -NonInteractive -Command "$ErrorActionPreference='Stop'; $reviewUrl='http://127.0.0.1:43187'; $reviewReady=$false; try { $reviewHealth=Invoke-RestMethod -Uri ($reviewUrl+'/api/health') -TimeoutSec 2; $reviewReady=$reviewHealth.service -eq 'souldesk-component-review' } catch {}; if (-not $reviewReady) { $reviewNode=(Get-Command node -ErrorAction Stop).Source; $reviewScript=(Resolve-Path -LiteralPath 'scripts/component-review.mjs').Path; $reviewLogs=Join-Path (Get-Location) 'artifacts/component-review'; New-Item -ItemType Directory -Path $reviewLogs -Force | Out-Null; $reviewQuote=[char]34; Start-Process -FilePath $reviewNode -ArgumentList ($reviewQuote+$reviewScript+$reviewQuote) -WorkingDirectory (Get-Location).Path -WindowStyle Hidden -RedirectStandardOutput (Join-Path $reviewLogs 'server.stdout.log') -RedirectStandardError (Join-Path $reviewLogs 'server.stderr.log'); for ($reviewTry=0; $reviewTry -lt 30; $reviewTry++) { Start-Sleep -Milliseconds 200; try { $reviewHealth=Invoke-RestMethod -Uri ($reviewUrl+'/api/health') -TimeoutSec 1; if ($reviewHealth.service -eq 'souldesk-component-review') { $reviewReady=$true; break } } catch {} } }; if (-not $reviewReady) { throw '本机网页未启动，请检查端口 43187 及 artifacts/component-review/server.stderr.log' }; Start-Process ($reviewUrl+'/')"
if errorlevel 1 (
  echo 组件验收网页启动失败，请查看上面的错误信息。
  pause
  exit /b 1
)
