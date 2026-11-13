param(
    [Parameter(Mandatory=$true)][string]$ReportPath,
    [Parameter(Mandatory=$true)][guid]$ExpectedRunId,
    [Parameter(Mandatory=$true)][int]$ExpectedPid,
    [Parameter(Mandatory=$true)][string]$ExpectedSha256
)
$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = [Text.UTF8Encoding]::new($false)
# 仅处理当前项目、本次唯一标识的隔离测试进程。
$projectRoot = Split-Path $PSScriptRoot -Parent
$expectedPath = (Resolve-Path -LiteralPath (Join-Path $projectRoot 'artifacts\souldesk-soak.exe')).Path
$expectedReport = Join-Path $projectRoot ('artifacts\g3-soak-' + $ExpectedRunId.ToString() + '.json')
if ([IO.Path]::GetFullPath($ReportPath) -ine [IO.Path]::GetFullPath($expectedReport)) { throw '报告不属于当前项目本次运行' }
$report = Get-Content -LiteralPath $ReportPath -Raw -Encoding UTF8 | ConvertFrom-Json
if ($report.runId -ne $ExpectedRunId.ToString() -or $report.latestSample.process.pid -ne $ExpectedPid -or $report.executableSha256 -ine $ExpectedSha256) { throw '报告中的进程身份不符' }
if ($report.elapsedSeconds -lt 1800 -and ([DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds() -lt ([long]$report.soakStarted + 1830000))) { throw '尚未达到用户要求的30分钟边界' }
$probeProcess = Get-CimInstance Win32_Process -Filter ('ProcessId=' + $ExpectedPid)
if (-not $probeProcess) { throw '本次测试在授权收束前已退出，不能视为正常停止' }
if ($probeProcess.ExecutablePath -ine $expectedPath -or $probeProcess.CommandLine.IndexOf($ExpectedRunId.ToString(), [StringComparison]::OrdinalIgnoreCase) -lt 0) { throw '实际程序路径或命令行运行标识不符，拒绝结束进程' }
$sha = [Security.Cryptography.SHA256]::Create()
$binary = [IO.File]::OpenRead($expectedPath)
try { $actualHash = [BitConverter]::ToString($sha.ComputeHash($binary)).Replace('-', '') }
finally { $binary.Dispose(); $sha.Dispose() }
if ($actualHash -ine $ExpectedSha256) { throw '实际程序文件散列不符' }
$start = [DateTimeOffset](Get-Process -Id $ExpectedPid).StartTime.ToUniversalTime()
if ([Math]::Abs($start.ToUnixTimeMilliseconds() - [long]$report.started) -gt 10000) { throw '进程启动时间不符，拒绝处理可能重用的标识' }
Stop-Process -Id $ExpectedPid -Force
Get-Process -Id $ExpectedPid -ErrorAction SilentlyContinue | Wait-Process -Timeout 5
Write-Output '已按用户调整结束唯一测试进程；原始24小时报告保留'
