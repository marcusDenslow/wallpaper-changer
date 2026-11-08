Unicode true
ManifestDPIAware true
SetCompressor /SOLID lzma
RequestExecutionLevel user

!define APP_NAME "Wallpaper Switcher"
!define APP_EXE "wallswitch.exe"
!define APP_ID "com.wallpaperswitcher.app"
!define APP_VERSION "2.0.0"
!define UNINSTALL_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\WallpaperSwitcher"
!define RUN_KEY "Software\Microsoft\Windows\CurrentVersion\Run"
!define APPROVED_KEY "Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run"

!ifndef BUILD_DIR
  !define BUILD_DIR "..\native\target\release"
!endif

Name "${APP_NAME}"
OutFile "Wallpaper Switcher Setup ${APP_VERSION}.exe"
InstallDir "$LOCALAPPDATA\Programs\${APP_NAME}"
InstallDirRegKey HKCU "${UNINSTALL_KEY}" "InstallLocation"
BrandingText " "

VIProductVersion "${APP_VERSION}.0"
VIAddVersionKey "ProductName" "${APP_NAME}"
VIAddVersionKey "FileDescription" "${APP_NAME} Setup"
VIAddVersionKey "FileVersion" "${APP_VERSION}"
VIAddVersionKey "ProductVersion" "${APP_VERSION}"
VIAddVersionKey "CompanyName" "${APP_NAME}"
VIAddVersionKey "LegalCopyright" "Copyright 2026 ${APP_NAME}"

!include MUI2.nsh
!include LogicLib.nsh
!include FileFunc.nsh
!include x64.nsh

!define MUI_ICON "..\native\assets\icon.ico"
!define MUI_UNICON "..\native\assets\icon.ico"
!define MUI_WELCOMEFINISHPAGE_BITMAP "welcome.bmp"
!define MUI_UNWELCOMEFINISHPAGE_BITMAP "welcome.bmp"
!define MUI_ABORTWARNING

!define MUI_WELCOMEPAGE_TITLE "${APP_NAME}"
!define MUI_WELCOMEPAGE_TEXT "Switch your wallpaper from anywhere with one shortcut.$\r$\n$\r$\nIt installs just for you, so Windows won't ask for admin rights.$\r$\n$\r$\nClick Install to continue."
!define MUI_PAGE_CUSTOMFUNCTION_SHOW RenameNextToInstall
!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_INSTFILES

!define MUI_FINISHPAGE_TITLE "You're all set"
!define MUI_FINISHPAGE_TEXT "${APP_NAME} waits in the tray near the clock.$\r$\n$\r$\nPress Ctrl + Alt + W any time to open it."
!define MUI_FINISHPAGE_RUN "$INSTDIR\${APP_EXE}"
!define MUI_FINISHPAGE_RUN_TEXT "Open ${APP_NAME} now"
!insertmacro MUI_PAGE_FINISH

!define MUI_UNCONFIRMPAGE_TEXT_TOP "${APP_NAME} will be removed from your PC. Your wallpapers stay where they are."
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES

!insertmacro MUI_LANGUAGE "English"

!macro StopRunningApp
  nsExec::Exec '"$SYSDIR\taskkill.exe" /IM ${APP_EXE} /F'
  Pop $0
  Sleep 500
!macroend

Function .onInit
  ${IfNot} ${RunningX64}
    MessageBox MB_ICONSTOP "${APP_NAME} needs 64-bit Windows 10 or 11." /SD IDOK
    Abort
  ${EndIf}
FunctionEnd

Function RenameNextToInstall
  GetDlgItem $0 $HWNDPARENT 1
  SendMessage $0 ${WM_SETTEXT} 0 "STR:Install"
FunctionEnd

Section "Install"
  !insertmacro StopRunningApp

  SetOutPath "$INSTDIR"
  File "${BUILD_DIR}\${APP_EXE}"
  Delete "$INSTDIR\WebView2Loader.dll" ; left over from 1.x (WebView2 build)
  WriteUninstaller "$INSTDIR\uninstall.exe"

  CreateShortcut "$SMPROGRAMS\${APP_NAME}.lnk" "$INSTDIR\${APP_EXE}" "" "$INSTDIR\${APP_EXE}" 0

  WriteRegStr HKCU "${UNINSTALL_KEY}" "DisplayName" "${APP_NAME}"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "DisplayVersion" "${APP_VERSION}"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "Publisher" "${APP_NAME}"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "DisplayIcon" "$INSTDIR\${APP_EXE},0"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "InstallLocation" "$INSTDIR"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "UninstallString" '"$INSTDIR\uninstall.exe"'
  WriteRegStr HKCU "${UNINSTALL_KEY}" "QuietUninstallString" '"$INSTDIR\uninstall.exe" /S'
  WriteRegDWORD HKCU "${UNINSTALL_KEY}" "NoModify" 1
  WriteRegDWORD HKCU "${UNINSTALL_KEY}" "NoRepair" 1
  ${GetSize} "$INSTDIR" "/S=0K" $0 $1 $2
  IntFmt $0 "0x%08X" $0
  WriteRegDWORD HKCU "${UNINSTALL_KEY}" "EstimatedSize" "$0"

  ${If} ${FileExists} "$LOCALAPPDATA\dev.wallswitch.app\*.*"
  ${OrIf} ${FileExists} "$APPDATA\dev.wallswitch.app\*.*"
    RMDir /r "$LOCALAPPDATA\dev.wallswitch.app"
    RMDir /r "$APPDATA\dev.wallswitch.app"
    DeleteRegValue HKCU "${RUN_KEY}" "${APP_NAME}"
    DeleteRegValue HKCU "${APPROVED_KEY}" "${APP_NAME}"
  ${EndIf}

SectionEnd

Section "Uninstall"
  !insertmacro StopRunningApp

  Delete "$INSTDIR\${APP_EXE}"
  Delete "$INSTDIR\WebView2Loader.dll"
  Delete "$INSTDIR\uninstall.exe"
  RMDir "$INSTDIR"
  Delete "$SMPROGRAMS\${APP_NAME}.lnk"

  DeleteRegValue HKCU "${RUN_KEY}" "${APP_NAME}"
  DeleteRegValue HKCU "${APPROVED_KEY}" "${APP_NAME}"
  DeleteRegKey HKCU "${UNINSTALL_KEY}"

  RMDir /r "$APPDATA\${APP_ID}"
  RMDir /r "$LOCALAPPDATA\${APP_ID}"
SectionEnd
