$ErrorActionPreference = 'Stop'
# 只读查询自身包身份，避免把桌面宿主的 MSIX 环境当成普通安装环境。
$SoulDeskProcesses = $args -contains '-SoulDeskProcesses'
Add-Type -TypeDefinition @'
using System;
using System.ComponentModel;
using System.Runtime.InteropServices;
using System.Text;
public static class SoulDeskPackageIdentity {
  [DllImport("kernel32.dll", CharSet=CharSet.Unicode)]
  static extern int GetCurrentPackageFamilyName(ref uint length, StringBuilder name);
  [DllImport("kernel32.dll", SetLastError=true)]
  static extern IntPtr OpenProcess(uint access, bool inherit, uint processId);
  [DllImport("kernel32.dll", CharSet=CharSet.Unicode)]
  static extern int GetPackageFamilyName(IntPtr process, ref uint length, StringBuilder name);
  [DllImport("kernel32.dll")]
  static extern bool CloseHandle(IntPtr handle);
  public static string ReadProcess(uint processId) {
    var handle = OpenProcess(0x1000, false, processId);
    if (handle == IntPtr.Zero) throw new Win32Exception(Marshal.GetLastWin32Error());
    try {
      uint length = 0;
      int error = GetPackageFamilyName(handle, ref length, null);
      if (error == 15700) return null;
      if (error != 122) throw new Win32Exception(error);
      var name = new StringBuilder((int)length);
      error = GetPackageFamilyName(handle, ref length, name);
      if (error != 0) throw new Win32Exception(error);
      return name.ToString();
    } finally { CloseHandle(handle); }
  }
  public static string Read() {
    uint length = 0;
    int error = GetCurrentPackageFamilyName(ref length, null);
    if (error == 15700) return null;
    if (error != 122) throw new Win32Exception(error);
    var name = new StringBuilder((int)length);
    error = GetCurrentPackageFamilyName(ref length, name);
    if (error != 0) throw new Win32Exception(error);
    return name.ToString();
  }
}
'@
if ($SoulDeskProcesses) {
  $found = @(Get-CimInstance Win32_Process | Where-Object { $_.Name -match '^(?i)(souldesk.*|desktop-probe)\.exe$' } | ForEach-Object {
    $soulDeskProcess = $_
    try {
      $family = [SoulDeskPackageIdentity]::ReadProcess($soulDeskProcess.ProcessId)
      [pscustomobject]@{processId=$soulDeskProcess.ProcessId;executable=$soulDeskProcess.ExecutablePath;commandLine=$soulDeskProcess.CommandLine;packageFamily=$family}
    } catch {
      # 枚举与查询身份之间正常退出不算失败；仍存活进程的查询错误继续上抛。
      if (Get-Process -Id $soulDeskProcess.ProcessId -ErrorAction SilentlyContinue) { throw }
    }
  })
  ConvertTo-Json -InputObject $found -Compress
} else {
  [pscustomobject]@{packageFamily=[SoulDeskPackageIdentity]::Read()} | ConvertTo-Json -Compress
}
