$ErrorActionPreference = 'Stop'
# 只读查询自身包身份，避免把桌面宿主的 MSIX 环境当成普通安装环境。
Add-Type -TypeDefinition @'
using System;
using System.ComponentModel;
using System.Runtime.InteropServices;
using System.Text;
public static class SoulDeskPackageIdentity {
  [DllImport("kernel32.dll", CharSet=CharSet.Unicode)]
  static extern int GetCurrentPackageFamilyName(ref uint length, StringBuilder name);
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
[pscustomobject]@{packageFamily=[SoulDeskPackageIdentity]::Read()} | ConvertTo-Json -Compress
