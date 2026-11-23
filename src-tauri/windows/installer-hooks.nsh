; 登记通知显示身份和行情通知点击入口，保留用户的通知偏好。
!macro NSIS_HOOK_POSTINSTALL
  WriteRegStr HKCU "Software\Classes\AppUserModelId\${BUNDLEID}" "DisplayName" "${PRODUCTNAME}"
  WriteRegStr HKCU "Software\Classes\AppUserModelId\${BUNDLEID}" "IconUri" "$INSTDIR\notification-icon.png"
  WriteRegStr HKCU "Software\Classes\souldesk-market-alert" "" "URL:SoulDesk 行情警报"
  WriteRegStr HKCU "Software\Classes\souldesk-market-alert" "URL Protocol" ""
  WriteRegStr HKCU "Software\Classes\souldesk-market-alert\shell\open\command" "" "$\"$INSTDIR\${MAINBINARYNAME}.exe$\" $\"%1$\""
!macroend

; 清理当前安装的通知身份和点击入口。
!macro NSIS_HOOK_POSTUNINSTALL
  DeleteRegValue HKCU "Software\Classes\AppUserModelId\${BUNDLEID}" "DisplayName"
  DeleteRegValue HKCU "Software\Classes\AppUserModelId\${BUNDLEID}" "IconUri"
  DeleteRegKey /ifempty HKCU "Software\Classes\AppUserModelId\${BUNDLEID}"
  ReadRegStr $0 HKCU "Software\Classes\souldesk-market-alert\shell\open\command" ""
  ${If} $0 == "$\"$INSTDIR\${MAINBINARYNAME}.exe$\" $\"%1$\""
    DeleteRegValue HKCU "Software\Classes\souldesk-market-alert\shell\open\command" ""
    DeleteRegKey /ifempty HKCU "Software\Classes\souldesk-market-alert\shell\open\command"
    DeleteRegKey /ifempty HKCU "Software\Classes\souldesk-market-alert\shell\open"
    DeleteRegKey /ifempty HKCU "Software\Classes\souldesk-market-alert\shell"
    DeleteRegValue HKCU "Software\Classes\souldesk-market-alert" "URL Protocol"
    DeleteRegValue HKCU "Software\Classes\souldesk-market-alert" ""
    DeleteRegKey /ifempty HKCU "Software\Classes\souldesk-market-alert"
  ${EndIf}
!macroend
