; SPDX-License-Identifier: GPL-3.0-or-later
;
; The Nectarlink installer for Windows (built by scripts/package.ps1).
;
; Installs for every user of the PC (one UAC prompt), so it can allow the
; app through Windows Firewall: otherwise Windows asks on first use, and
; people often say no. User data stays in each user's
; %LOCALAPPDATA%\Nectarlink, and is kept when uninstalling.
;
; Defines passed in: VERSION (numbers only, e.g. 0.0.1), DISPLAY_VERSION
; (e.g. 0.0.1-beta.2), ARCH (x64 or arm64), STAGE (the
; folder with the app and its Qt files), ICON, OUTFILE.

Unicode true
ManifestDPIAware true
SetCompressor /SOLID lzma

!include "MUI2.nsh"
!include "x64.nsh"
!include "LogicLib.nsh"
!include "FileFunc.nsh"

!define APP "Nectarlink"
!define EXE "nectarlink-desktop.exe"
!define UNINSTALL_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\${APP}"
!define FIREWALL_RULE "Nectarlink"

Name "${APP}"
OutFile "${OUTFILE}"
RequestExecutionLevel admin
InstallDir "$PROGRAMFILES64\${APP}"
InstallDirRegKey HKLM "Software\${APP}" "InstallDir"
BrandingText "${APP} ${DISPLAY_VERSION}"

VIProductVersion "${VERSION}.0"
VIAddVersionKey "ProductName" "${APP}"
VIAddVersionKey "FileDescription" "${APP} Setup"
VIAddVersionKey "CompanyName" "The Nectarlink contributors"
VIAddVersionKey "LegalCopyright" "GPL-3.0-or-later"
VIAddVersionKey "FileVersion" "${VERSION}"
VIAddVersionKey "ProductVersion" "${DISPLAY_VERSION}"

!define MUI_ICON "${ICON}"
!define MUI_UNICON "${ICON}"
!define MUI_ABORTWARNING
!define MUI_FINISHPAGE_RUN
!define MUI_FINISHPAGE_RUN_TEXT "Open ${APP}"
!define MUI_FINISHPAGE_RUN_FUNCTION OpenApp

!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_LICENSE "${STAGE}\LICENSE.txt"
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH

!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES

!insertmacro MUI_LANGUAGE "English"

Function .onInit
  ${If} "${ARCH}" == "x64"
    ${IfNot} ${IsNativeAMD64}
      MessageBox MB_ICONSTOP "This copy of ${APP} is for x64 PCs. Download the ARM64 version for this PC." /SD IDOK
      Abort
    ${EndIf}
  ${ElseIf} "${ARCH}" == "arm64"
    ${IfNot} ${IsNativeARM64}
      MessageBox MB_ICONSTOP "This copy of ${APP} is for ARM64 PCs. Download the x64 version for this PC." /SD IDOK
      Abort
    ${EndIf}
  ${EndIf}
  SetRegView 64
  SetShellVarContext all
FunctionEnd

Function un.onInit
  SetRegView 64
  SetShellVarContext all
FunctionEnd

; Asks a running copy to quit (it closes its connections cleanly), and waits
; for it, so its files can be replaced or removed.
!macro QuitRunningApp
  ${If} ${FileExists} "$INSTDIR\${EXE}"
    ExecWait '"$INSTDIR\${EXE}" --quit'
  ${EndIf}
  StrCpy $R0 0
  ${Do}
    nsExec::Exec 'cmd /c tasklist /FI "IMAGENAME eq ${EXE}" /NH | find /I "${EXE}"'
    Pop $R1
    ${If} $R1 != 0
      ${Break}
    ${EndIf}
    ${If} $R0 >= 25
      ; Other users' copies, or one that's stuck.
      nsExec::Exec 'taskkill /F /IM ${EXE}'
      Sleep 500
      ${Break}
    ${EndIf}
    Sleep 200
    IntOp $R0 $R0 + 1
  ${Loop}
!macroend

Function OpenApp
  ; Through Explorer, so the app runs as the user and not elevated.
  Exec '"$WINDIR\explorer.exe" "$INSTDIR\${EXE}"'
FunctionEnd

Section "Install"
  !insertmacro QuitRunningApp

  ; A clean copy: nothing left over from an older version.
  RMDir /r "$INSTDIR\qml"
  RMDir /r "$INSTDIR\platforms"
  SetOutPath "$INSTDIR"
  File /r "${STAGE}\*.*"
  WriteUninstaller "$INSTDIR\uninstall.exe"

  CreateShortCut "$SMPROGRAMS\${APP}.lnk" "$INSTDIR\${EXE}" "" "$INSTDIR\${EXE}" 0 "" "" "Your phone, on your PC"

  ; Phones reach this PC over UDP (QUIC) and find it with mDNS.
  nsExec::Exec 'netsh advfirewall firewall delete rule name="${FIREWALL_RULE}"'
  nsExec::Exec 'netsh advfirewall firewall add rule name="${FIREWALL_RULE}" dir=in action=allow program="$INSTDIR\${EXE}" enable=yes profile=private,domain description="Lets paired phones connect to Nectarlink."'

  WriteRegStr HKLM "Software\${APP}" "InstallDir" "$INSTDIR"
  WriteRegStr HKLM "${UNINSTALL_KEY}" "DisplayName" "${APP}"
  WriteRegStr HKLM "${UNINSTALL_KEY}" "DisplayVersion" "${DISPLAY_VERSION}"
  WriteRegStr HKLM "${UNINSTALL_KEY}" "Publisher" "The Nectarlink contributors"
  WriteRegStr HKLM "${UNINSTALL_KEY}" "DisplayIcon" "$INSTDIR\${EXE}"
  WriteRegStr HKLM "${UNINSTALL_KEY}" "InstallLocation" "$INSTDIR"
  WriteRegStr HKLM "${UNINSTALL_KEY}" "URLInfoAbout" "https://github.com/thtbee/nectarlink"
  WriteRegStr HKLM "${UNINSTALL_KEY}" "UninstallString" '"$INSTDIR\uninstall.exe"'
  WriteRegStr HKLM "${UNINSTALL_KEY}" "QuietUninstallString" '"$INSTDIR\uninstall.exe" /S'
  WriteRegDWORD HKLM "${UNINSTALL_KEY}" "NoModify" 1
  WriteRegDWORD HKLM "${UNINSTALL_KEY}" "NoRepair" 1
  ${GetSize} "$INSTDIR" "/S=0K" $0 $1 $2
  WriteRegDWORD HKLM "${UNINSTALL_KEY}" "EstimatedSize" $0

  ; /RUN (the app's own updater): open the new version when done.
  ${GetParameters} $R0
  ClearErrors
  ${GetOptions} $R0 "/RUN" $R1
  ${IfNot} ${Errors}
    Call OpenApp
  ${EndIf}
SectionEnd

Section "Uninstall"
  !insertmacro QuitRunningApp
  ; What the app set up for this user: Send to entries, sign-in start,
  ; notification registration. (Data stays, for a reinstall.)
  ${If} ${FileExists} "$INSTDIR\${EXE}"
    ExecWait '"$INSTDIR\${EXE}" --uninstall'
  ${EndIf}

  nsExec::Exec 'netsh advfirewall firewall delete rule name="${FIREWALL_RULE}"'
  Delete "$SMPROGRAMS\${APP}.lnk"
  ; Only ever a folder of our own (a silent install can be pointed
  ; anywhere with /D): never everything in, say, C:\.
  ${GetFileName} "$INSTDIR" $0
  ${If} $0 == "${APP}"
    RMDir /r "$INSTDIR"
  ${Else}
    Delete "$INSTDIR\${EXE}"
    Delete "$INSTDIR\uninstall.exe"
  ${EndIf}

  DeleteRegKey HKLM "${UNINSTALL_KEY}"
  DeleteRegKey HKLM "Software\${APP}"
SectionEnd
