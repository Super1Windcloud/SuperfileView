; NSIS installer for the Windows x86_64 Superfile build.
; Invoked by package-windows.ps1. Per-user install, no elevation.

Unicode true
ManifestDPIAware true
SetCompressor /SOLID lzma

!ifndef APP_VERSION
  !define APP_VERSION "0.0.2"
!endif
!ifndef APP_VERSION_QUAD
  !define APP_VERSION_QUAD "0.0.2.0"
!endif
!ifndef SOURCE_EXE
  !define SOURCE_EXE "..\..\..\target\x86_64-pc-windows-msvc\release\superfile.exe"
!endif
!ifndef SOURCE_README
  !define SOURCE_README "..\..\..\README.md"
!endif
!ifndef SOURCE_LICENSE
  !define SOURCE_LICENSE "..\..\..\LICENSE"
!endif
!ifndef SOURCE_ICON
  !define SOURCE_ICON "..\..\..\assets\windows\Superfile.ico"
!endif
!ifndef OUTPUT_DIR
  !define OUTPUT_DIR "..\..\..\package-dist"
!endif
!ifndef OUTPUT_FILENAME
  !define OUTPUT_FILENAME "superfile-windows-x86_64-setup"
!endif

; Same product id previously used by the Inno Setup script.
!define UNINSTALL_KEY "{9F8F8256-42E2-4C5D-876B-C877DDF7106D}"
!define UNINSTALL_REG "Software\Microsoft\Windows\CurrentVersion\Uninstall\${UNINSTALL_KEY}"

!include "MUI2.nsh"
!include "x64.nsh"
!include "FileFunc.nsh"
!include "LogicLib.nsh"

Name "Superfile"
OutFile "${OUTPUT_DIR}\${OUTPUT_FILENAME}.exe"
InstallDir "$LOCALAPPDATA\Programs\Superfile"
InstallDirRegKey HKCU "Software\Superfile" "InstallDir"
RequestExecutionLevel user
ShowInstDetails show
ShowUninstDetails show
BrandingText "Superfile ${APP_VERSION}"

VIProductVersion "${APP_VERSION_QUAD}"
VIAddVersionKey /LANG=1033 "ProductName" "Superfile"
VIAddVersionKey /LANG=1033 "CompanyName" "Superfile Team"
VIAddVersionKey /LANG=1033 "FileDescription" "Superfile Setup"
VIAddVersionKey /LANG=1033 "FileVersion" "${APP_VERSION}"
VIAddVersionKey /LANG=1033 "ProductVersion" "${APP_VERSION}"
VIAddVersionKey /LANG=1033 "LegalCopyright" "Superfile Team"
VIAddVersionKey /LANG=2052 "ProductName" "Superfile"
VIAddVersionKey /LANG=2052 "CompanyName" "Superfile Team"
VIAddVersionKey /LANG=2052 "FileDescription" "Superfile 安装程序"
VIAddVersionKey /LANG=2052 "FileVersion" "${APP_VERSION}"
VIAddVersionKey /LANG=2052 "ProductVersion" "${APP_VERSION}"
VIAddVersionKey /LANG=2052 "LegalCopyright" "Superfile Team"

!define MUI_ABORTWARNING
!define MUI_ICON "${SOURCE_ICON}"
!define MUI_UNICON "${SOURCE_ICON}"
!define MUI_FINISHPAGE_RUN "$INSTDIR\superfile.exe"

!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_COMPONENTS
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH

!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES

!insertmacro MUI_LANGUAGE "English"
!insertmacro MUI_LANGUAGE "SimpChinese"

LangString DESC_Main ${LANG_ENGLISH} "Install Superfile for the current user."
LangString DESC_Main ${LANG_SIMPCHINESE} "为当前用户安装 Superfile。"
LangString NAME_Desktop ${LANG_ENGLISH} "Desktop shortcut"
LangString NAME_Desktop ${LANG_SIMPCHINESE} "桌面快捷方式"
LangString DESC_Desktop ${LANG_ENGLISH} "Create a desktop shortcut."
LangString DESC_Desktop ${LANG_SIMPCHINESE} "创建桌面快捷方式。"

Function .onInit
  ${IfNot} ${RunningX64}
    MessageBox MB_ICONSTOP "Superfile requires 64-bit Windows."
    Abort
  ${EndIf}
FunctionEnd

!include "extensions.nsh"
!define CAPABILITIES "Software\Superfile\Capabilities"
!define CLASSES "Software\Classes"

!macro RegisterExtension ext
  WriteRegStr HKCU "${CAPABILITIES}\FileAssociations" "${ext}" "Superfile.Document"
  WriteRegNone HKCU "${CLASSES}\${ext}\OpenWithProgids" "Superfile.Document"
!macroend

!macro UnregisterExtension ext
  DeleteRegValue HKCU "${CLASSES}\${ext}\OpenWithProgids" "Superfile.Document"
  DeleteRegKey /ifempty HKCU "${CLASSES}\${ext}\OpenWithProgids"
  DeleteRegKey /ifempty HKCU "${CLASSES}\${ext}"
!macroend

Section "Superfile" SecMain
  SectionIn RO
  SetOutPath "$INSTDIR"
  File "/oname=superfile.exe" "${SOURCE_EXE}"
  !if /FileExists "${SOURCE_README}"
    File "/oname=README.md" "${SOURCE_README}"
  !endif
  !if /FileExists "${SOURCE_LICENSE}"
    File "/oname=LICENSE" "${SOURCE_LICENSE}"
  !endif
  WriteUninstaller "$INSTDIR\Uninstall.exe"

  CreateShortcut "$SMPROGRAMS\Superfile.lnk" "$INSTDIR\superfile.exe"

  WriteRegStr HKCU "Software\Superfile" "InstallDir" "$INSTDIR"
  SetRegView 64
  WriteRegStr HKCU "${CAPABILITIES}" "ApplicationName" "Superfile"
  WriteRegStr HKCU "${CAPABILITIES}" "ApplicationDescription" "Desktop file viewer and editor"
  WriteRegStr HKCU "${CLASSES}\Superfile.Document" "" "Superfile Document"
  ; Older binaries may have left a cached name for this install path.
  WriteRegStr HKCU "${CLASSES}\Superfile.Document\shell\open" "FriendlyAppName" "Superfile"
  WriteRegStr HKCU "${CLASSES}\Superfile.Document\shell\open\command" "" '"$INSTDIR\superfile.exe" "%1"'
  ReadRegStr $1 HKCU "${CLASSES}\Superfile.Url\shell\open\command" ""
  WriteRegStr HKCU "${CLASSES}\Superfile.Url" "" "URL:Superfile"
  WriteRegStr HKCU "${CLASSES}\Superfile.Url" "URL Protocol" ""
  WriteRegStr HKCU "${CLASSES}\Superfile.Url\shell\open\command" "" '"$INSTDIR\superfile.exe" "%1"'
  WriteRegStr HKCU "${CAPABILITIES}\URLAssociations" "superfile" "Superfile.Url"
  ReadRegStr $0 HKCU "${CLASSES}\superfile\shell\open\command" ""
  ${If} $0 == ""
  ${OrIf} $0 == $1
    WriteRegStr HKCU "${CLASSES}\superfile" "" "URL:Superfile"
    WriteRegStr HKCU "${CLASSES}\superfile" "URL Protocol" ""
    WriteRegStr HKCU "${CLASSES}\superfile\shell\open\command" "" '"$INSTDIR\superfile.exe" "%1"'
  ${EndIf}
  !insertmacro SuperfileExtensions RegisterExtension
  WriteRegStr HKCU "Software\RegisteredApplications" "Superfile" "${CAPABILITIES}"
  System::Call 'shell32::SHChangeNotify(i 0x08000000, i 0, p 0, p 0)'
  SetRegView 32

  WriteRegStr HKCU "${UNINSTALL_REG}" "DisplayName" "Superfile"
  WriteRegStr HKCU "${UNINSTALL_REG}" "DisplayVersion" "${APP_VERSION}"
  WriteRegStr HKCU "${UNINSTALL_REG}" "Publisher" "Superfile Team"
  WriteRegStr HKCU "${UNINSTALL_REG}" "DisplayIcon" "$INSTDIR\superfile.exe"
  WriteRegStr HKCU "${UNINSTALL_REG}" "UninstallString" '"$INSTDIR\Uninstall.exe"'
  WriteRegStr HKCU "${UNINSTALL_REG}" "QuietUninstallString" '"$INSTDIR\Uninstall.exe" /S'
  WriteRegStr HKCU "${UNINSTALL_REG}" "InstallLocation" "$INSTDIR"
  WriteRegStr HKCU "${UNINSTALL_REG}" "URLInfoAbout" "https://github.com/superfile"
  WriteRegDWORD HKCU "${UNINSTALL_REG}" "NoModify" 1
  WriteRegDWORD HKCU "${UNINSTALL_REG}" "NoRepair" 1
  ${GetSize} "$INSTDIR" "/S=0K" $0 $1 $2
  WriteRegDWORD HKCU "${UNINSTALL_REG}" "EstimatedSize" "$0"
SectionEnd

Section /o "$(NAME_Desktop)" SecDesktop
  CreateShortcut "$DESKTOP\Superfile.lnk" "$INSTDIR\superfile.exe"
SectionEnd

!insertmacro MUI_FUNCTION_DESCRIPTION_BEGIN
  !insertmacro MUI_DESCRIPTION_TEXT ${SecMain} $(DESC_Main)
  !insertmacro MUI_DESCRIPTION_TEXT ${SecDesktop} $(DESC_Desktop)
!insertmacro MUI_FUNCTION_DESCRIPTION_END

Section "Uninstall"
  Delete "$INSTDIR\superfile.exe"
  Delete "$INSTDIR\README.md"
  Delete "$INSTDIR\LICENSE"
  Delete "$INSTDIR\Uninstall.exe"
  Delete "$SMPROGRAMS\Superfile.lnk"
  Delete "$DESKTOP\Superfile.lnk"
  RMDir "$INSTDIR"
  SetRegView 64
  ReadRegStr $0 HKCU "${CLASSES}\Superfile.Document\shell\open\command" ""
  ${If} $0 == '"$INSTDIR\superfile.exe" "%1"'
    !insertmacro SuperfileExtensions UnregisterExtension
    DeleteRegValue HKCU "Software\RegisteredApplications" "Superfile"
    DeleteRegKey HKCU "${CAPABILITIES}"
    DeleteRegKey HKCU "${CLASSES}\Superfile.Document"
    DeleteRegKey HKCU "${CLASSES}\Superfile.Url"
    ReadRegStr $0 HKCU "${CLASSES}\superfile\shell\open\command" ""
    ${If} $0 == '"$INSTDIR\superfile.exe" "%1"'
      DeleteRegKey HKCU "${CLASSES}\superfile"
    ${EndIf}
    System::Call 'shell32::SHChangeNotify(i 0x08000000, i 0, p 0, p 0)'
  ${EndIf}
  SetRegView 32
  ReadRegStr $0 HKCU "Software\Superfile" "InstallDir"
  ${If} $0 == "$INSTDIR"
    DeleteRegKey HKCU "${UNINSTALL_REG}"
    DeleteRegValue HKCU "Software\Superfile" "InstallDir"
    DeleteRegKey /ifempty HKCU "Software\Superfile"
  ${EndIf}
SectionEnd
