param(
    [Parameter(Mandatory)][string] $UpstreamArchive,
    [string] $Dotnet,
    [string] $PackageCache,
    [string] $TestBundleRuntime,
    [switch] $PrepareOnly,
    [string] $BundleRuntime
)

$ErrorActionPreference = 'Stop'
$repository = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..')).Path
$archive = (Resolve-Path -LiteralPath $UpstreamArchive).Path
$revision = '4cae1d4ebb0e416af7d66a764f52f6ccaafdda3a'
$archiveHash = '479A208A9D4818206BC393082A29467D7E6674F8C40083462C6733D1303653FF'
if ((Get-FileHash -LiteralPath $archive).Hash -ne $archiveHash) {
    throw 'DeskBox 源码归档与指定提交的归档不一致。'
}
if ($BundleRuntime -and $TestBundleRuntime) {
    throw '请只指定一套安装包运行目录。'
}
if ($BundleRuntime -or $TestBundleRuntime) {
    $runtimePath = if ($BundleRuntime) { $BundleRuntime } else { $TestBundleRuntime }
    $runtime = (Resolve-Path -LiteralPath $runtimePath).Path
    $buildRoot = Split-Path -Parent $runtime
    $source = Join-Path $buildRoot "DeskBox-$revision"
    $license = Join-Path $source 'LICENSE'
    $buildPatch = Join-Path $buildRoot 'deskbox.patch'
    foreach ($file in @((Join-Path $runtime 'SoulDeskGridWinUI.exe'), (Join-Path $runtime 'SoulDeskGridWinUI.dll'), $license, $buildPatch)) {
        if (!(Test-Path -LiteralPath $file -PathType Leaf)) {
            throw "桌面格子安装包缺少资源 $file"
        }
    }
    $resources = @{
        ($runtime.Replace('\', '/') + '/') = 'desktop-grid-winui/'
        ($archive.Replace('\', '/')) = 'desktop-grid-winui/source/deskbox-upstream.zip'
        ($license.Replace('\', '/')) = 'desktop-grid-winui/source/LICENSE'
        ($buildPatch.Replace('\', '/')) = 'desktop-grid-winui/source/deskbox.patch'
        ((Join-Path $repository 'src-tauri/desktop-grid-winui/README.md').Replace('\', '/')) = 'desktop-grid-winui/source/README.md'
    }
    $config = Join-Path $buildRoot 'souldesk-grid-local-bundle.json'
    $overlay = @{
        bundle = @{
            resources = $resources
            createUpdaterArtifacts = $false
            useLocalToolsDir = $true
            windows = @{ nsis = @{ compression = 'zlib' } }
        }
    }
    if ($TestBundleRuntime) {
        $config = Join-Path $buildRoot 'souldesk-grid-test-bundle.json'
        $overlay.productName = 'SoulDesk Grid QA'
        $overlay.identifier = 'local.souldesk.grid.qa'
    }
    [IO.File]::WriteAllText($config, ($overlay | ConvertTo-Json -Depth 6))
    Write-Output $config
    return
}
if (!$PrepareOnly) {
    if (!$Dotnet -or !$PackageCache) { throw '请指定 .NET SDK 和已下载的 NuGet 包目录。' }
    $sdk = (Resolve-Path -LiteralPath $Dotnet).Path
    $packages = (Resolve-Path -LiteralPath $PackageCache).Path
}

$stage = 'artifacts/grid-winui-build-' + [Guid]::NewGuid().ToString('N').Substring(0, 12)
$output = (New-Item -ItemType Directory -Path (Join-Path $repository $stage)).FullName
Expand-Archive -LiteralPath $archive -DestinationPath $output
$sourceRelative = "$stage/DeskBox-$revision"
$source = Join-Path $repository $sourceRelative
$patch = Join-Path $output 'deskbox.patch'
Copy-Item -LiteralPath (Join-Path $repository 'src-tauri/desktop-grid-winui/deskbox.patch') -Destination $patch
& git -C $repository apply --check "--directory=$sourceRelative" --whitespace=nowarn $patch
if ($LASTEXITCODE -ne 0) { throw 'DeskBox 源码补丁校验失败。' }
& git -C $repository apply "--directory=$sourceRelative" --whitespace=nowarn $patch
if ($LASTEXITCODE -ne 0) { throw '无法应用 DeskBox 源码补丁。' }
Write-Output "源码目录 $source"
if ($PrepareOnly) { return }

$env:DOTNET_CLI_HOME = Join-Path $output 'dotnet-cli'
$env:NUGET_PACKAGES = $packages
$env:DOTNET_GENERATE_ASPNET_CERTIFICATE = 'false'
$env:DOTNET_CLI_TELEMETRY_OPTOUT = '1'
$flags = @(
    '-p:Platform=x64'
    '-p:RuntimeIdentifier=win-x64'
    '-p:DeskBoxShellThumbnailProxy=false'
    '-p:DeskBoxRustNative=false'
    '-p:PublishAot=false'
    '-p:WindowsAppSDKSelfContained=true'
    '-p:SelfContained=true'
    '-p:SoulDeskGridPreview=true'
)
$intermediate = (New-Item -ItemType Directory -Path (Join-Path $output 'obj')).FullName
$binary = (New-Item -ItemType Directory -Path (Join-Path $output 'bin')).FullName
$build = @(
    'build', 'src/DeskBox/DeskBox.csproj', '--configuration', 'Debug', '--no-restore', '--verbosity', 'minimal'
    '-p:Optimize=true', '-p:DebugType=portable', "-p:IntermediateOutputPath=$intermediate/", "-p:OutputPath=$binary/"
)
Push-Location -LiteralPath $source
try {
    & $sdk restore src/DeskBox/DeskBox.csproj --locked-mode --source $packages '-p:NuGetAudit=false' @flags
    if ($LASTEXITCODE -ne 0) { throw '无法从本地目录还原 WinUI 构建所需的锁定依赖。' }
    & $sdk @build @flags
    if ($LASTEXITCODE -ne 0) { throw 'WinUI 格子编译失败。' }
} finally {
    Pop-Location
}
Write-Output "运行目录 $binary"
