; 只登记本应用的通知显示身份，不修改 Windows 或用户的通知偏好。
!macro NSIS_HOOK_POSTINSTALL
  WriteRegStr HKCU "Software\Classes\AppUserModelId\${BUNDLEID}" "DisplayName" "${PRODUCTNAME}"
  WriteRegStr HKCU "Software\Classes\AppUserModelId\${BUNDLEID}" "IconUri" "$INSTDIR\notification-icon.png"
!macroend

; 仅清理安装器写入的两个值；保留系统或后续版本写入的其他值。
!macro NSIS_HOOK_POSTUNINSTALL
  DeleteRegValue HKCU "Software\Classes\AppUserModelId\${BUNDLEID}" "DisplayName"
  DeleteRegValue HKCU "Software\Classes\AppUserModelId\${BUNDLEID}" "IconUri"
  DeleteRegKey /ifempty HKCU "Software\Classes\AppUserModelId\${BUNDLEID}"
!macroend
