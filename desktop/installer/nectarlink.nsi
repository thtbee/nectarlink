; SPDX-License-Identifier: GPL-3.0-or-later
;
; The NectarLink installer for Windows (built by scripts/package.ps1).
;
; Two ways to run it:
;
; - Opened by the user: no NSIS pages. It unpacks the app to a temporary
;   folder and runs it there, as "Nectarlink Setup.exe" with --setup, which
;   draws NectarLink's own setup window (desktop/app/src/setup.rs). That
;   window runs this installer again, elevated, as below.
; - With /S (that window, or the app's updater): installs into Program Files
;   for every user of the PC, without a window. /PROGRESS=<file> writes each
;   step to the file; /RUN opens the app afterwards. Without administrator
;   rights it asks for them (UAC) and carries on elevated.
;
; Installing for everyone lets it allow the app through Windows Firewall:
; otherwise Windows asks on first use, and people often say no.
;
; The uninstaller (uninstall.exe) is always silent. Windows Settings runs
; the app's own uninstall window (--remove), which runs uninstall.exe
; elevated and then removes the user's data.
;
; Defines passed in: VERSION (numbers only, e.g. 0.0.1), DISPLAY_VERSION
; (e.g. 0.0.1-beta.2), ARCH (x64 or arm64), STAGE (the folder with the app
; and its Qt files), ICON, OUTFILE.

Unicode true
ManifestDPIAware true
SetCompressor /SOLID zlib

!include "x64.nsh"
!include "LogicLib.nsh"
!include "FileFunc.nsh"

; APP names the install folder, registry keys and the firewall rule; DISPLAY is
; the name people see.
!define APP "Nectarlink"
!define DISPLAY "NectarLink"
!define EXE "nectarlink-desktop.exe"
!define SETUP_EXE "Nectarlink Setup.exe"
!define UNINSTALL_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\${APP}"
!define FIREWALL_RULE "Nectarlink"

Name "${DISPLAY}"
OutFile "${OUTFILE}"
; Elevates itself only when it installs (see .onInit).
RequestExecutionLevel user
InstallDir "$PROGRAMFILES64\${APP}"
InstallDirRegKey HKLM "Software\${APP}" "InstallDir"
Icon "${ICON}"
UninstallIcon "${ICON}"
SilentUnInstall silent
ShowInstDetails nevershow
ShowUninstDetails nevershow

VIProductVersion "${VERSION}.0"
VIAddVersionKey "ProductName" "${DISPLAY}"
VIAddVersionKey "FileDescription" "${DISPLAY} Setup"
VIAddVersionKey "CompanyName" "The NectarLink contributors"
VIAddVersionKey "LegalCopyright" "GPL-3.0-or-later"
VIAddVersionKey "FileVersion" "${VERSION}"
VIAddVersionKey "ProductVersion" "${DISPLAY_VERSION}"

; 1: show NectarLink's own setup window. 0: install now (/S).
Var Interactive
; Where each step is written, for the setup window (/PROGRESS=<file>).
Var ProgressFile
Var AppDir

!macro Progress line
  ${If} $ProgressFile != ""
    FileOpen $9 $ProgressFile a
    FileSeek $9 0 END
    FileWrite $9 "${line}$\r$\n"
    FileClose $9
  ${EndIf}
!macroend

!macro ArchCheck
  ${If} "${ARCH}" == "x64"
    ${IfNot} ${IsNativeAMD64}
      MessageBox MB_ICONSTOP "This copy of ${DISPLAY} is for x64 PCs. Download the ARM64 version for this PC." /SD IDOK
      Abort
    ${EndIf}
  ${ElseIf} "${ARCH}" == "arm64"
    ${IfNot} ${IsNativeARM64}
      MessageBox MB_ICONSTOP "This copy of ${DISPLAY} is for ARM64 PCs. Download the x64 version for this PC." /SD IDOK
      Abort
    ${EndIf}
  ${EndIf}
!macroend

Function .onInit
  !insertmacro ArchCheck
  SetRegView 64
  SetShellVarContext all
  ${GetParameters} $R0
  ClearErrors
  ${GetOptions} $R0 "/PROGRESS=" $ProgressFile
  ${If} ${Errors}
    StrCpy $ProgressFile ""
  ${EndIf}

  ${If} ${Silent}
    StrCpy $Interactive 0
    ; Installing needs administrator rights: ask, and let the elevated copy
    ; do it. (The app's own updater runs as SYSTEM and has them already.)
    UserInfo::GetAccountType
    Pop $0
    ${If} $0 != "Admin"
      ExecShell "runas" '"$EXEPATH"' "$R0" SW_HIDE
      SetErrorLevel 0
      Quit
    ${EndIf}
  ${Else}
    StrCpy $Interactive 1
    ; No NSIS window: the window is NectarLink's own.
    SetSilent silent
  ${EndIf}
FunctionEnd

Function un.onInit
  SetRegView 64
  SetShellVarContext all
  ${un.GetParameters} $R0
  ClearErrors
  ${un.GetOptions} $R0 "/PROGRESS=" $ProgressFile
  ${If} ${Errors}
    StrCpy $ProgressFile ""
  ${EndIf}
FunctionEnd

; Asks a running copy to quit (it closes its connections cleanly), and waits
; for it, so its files can be replaced or removed. The setup and uninstall
; windows run under other names, so they're left alone.
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
  ${If} $Interactive == 1
    InitPluginsDir
    StrCpy $AppDir "$PLUGINSDIR\app"
  ${Else}
    !insertmacro Progress "step closing"
    !insertmacro QuitRunningApp
    !insertmacro Progress "step files"
    ; A clean copy: nothing left over from an older version.
    RMDir /r "$INSTDIR\qml"
    RMDir /r "$INSTDIR\platforms"
    StrCpy $AppDir "$INSTDIR"
  ${EndIf}

  ; One copy of the files, for either use.
  SetOutPath "$AppDir"
  File /r "${STAGE}\*.*"

  ${If} $Interactive == 1
    Rename "$AppDir\${EXE}" "$AppDir\${SETUP_EXE}"
    ExecWait '"$AppDir\${SETUP_EXE}" --setup --installer "$EXEPATH"' $0
    ; Out of the temporary folder, so it can be deleted.
    SetOutPath "$TEMP"
    SetErrorLevel $0
    Return
  ${EndIf}

  WriteUninstaller "$INSTDIR\uninstall.exe"

  ; Phones reach this PC over UDP (QUIC) and find it with mDNS.
  !insertmacro Progress "step firewall"
  nsExec::Exec 'netsh advfirewall firewall delete rule name="${FIREWALL_RULE}"'
  nsExec::Exec 'netsh advfirewall firewall add rule name="${FIREWALL_RULE}" dir=in action=allow program="$INSTDIR\${EXE}" enable=yes profile=private,domain description="Lets paired phones connect to NectarLink."'

  ; Updates without an administrator prompt: a task that runs this app as
  ; SYSTEM to install a newer release (desktop/app/src/updater.rs). Only
  ; SYSTEM and administrators may change it; signed-in users may read and
  ; start it. Registered with schtasks from a task definition rather than
  ; PowerShell, which security software can hold up without a word, leaving
  ; setup stuck at this step.
  !insertmacro Progress "step updates"
  InitPluginsDir
  FileOpen $0 "$PLUGINSDIR\update-task.xml" w
  FileWriteUTF16LE /BOM $0 '<?xml version="1.0" encoding="UTF-16"?>$\r$\n'
  FileWriteUTF16LE $0 '<Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">$\r$\n'
  FileWriteUTF16LE $0 '  <RegistrationInfo>$\r$\n'
  FileWriteUTF16LE $0 '    <Description>Installs new versions of NectarLink from its GitHub releases.</Description>$\r$\n'
  FileWriteUTF16LE $0 '    <SecurityDescriptor>D:(A;;FA;;;SY)(A;;FA;;;BA)(A;;GRGX;;;AU)</SecurityDescriptor>$\r$\n'
  FileWriteUTF16LE $0 '  </RegistrationInfo>$\r$\n'
  FileWriteUTF16LE $0 '  <Principals>$\r$\n'
  FileWriteUTF16LE $0 '    <Principal id="Author"><UserId>S-1-5-18</UserId><RunLevel>HighestAvailable</RunLevel></Principal>$\r$\n'
  FileWriteUTF16LE $0 '  </Principals>$\r$\n'
  FileWriteUTF16LE $0 '  <Settings>$\r$\n'
  FileWriteUTF16LE $0 '    <MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>$\r$\n'
  FileWriteUTF16LE $0 '    <DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>$\r$\n'
  FileWriteUTF16LE $0 '    <StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>$\r$\n'
  FileWriteUTF16LE $0 '    <AllowStartOnDemand>true</AllowStartOnDemand>$\r$\n'
  FileWriteUTF16LE $0 '    <Enabled>true</Enabled>$\r$\n'
  FileWriteUTF16LE $0 '    <ExecutionTimeLimit>PT30M</ExecutionTimeLimit>$\r$\n'
  FileWriteUTF16LE $0 '  </Settings>$\r$\n'
  FileWriteUTF16LE $0 '  <Actions Context="Author">$\r$\n'
  FileWriteUTF16LE $0 '    <Exec><Command>"$INSTDIR\${EXE}"</Command><Arguments>--update-service</Arguments></Exec>$\r$\n'
  FileWriteUTF16LE $0 '  </Actions>$\r$\n'
  FileWriteUTF16LE $0 '</Task>$\r$\n'
  FileClose $0
  nsExec::Exec 'schtasks /Create /TN "Nectarlink Update" /XML "$PLUGINSDIR\update-task.xml" /F'

  !insertmacro Progress "step shortcuts"
  CreateShortCut "$SMPROGRAMS\${DISPLAY}.lnk" "$INSTDIR\${EXE}" "" "$INSTDIR\${EXE}" 0 "" "" "Your phone, on your PC"
  CreateShortCut "$DESKTOP\${DISPLAY}.lnk" "$INSTDIR\${EXE}" "" "$INSTDIR\${EXE}" 0 "" "" "Your phone, on your PC"

  ; The Windows Share sheet needs the app's sparse package registered, which
  ; Windows allows for an unsigned one only with Developer Mode on. Why it
  ; didn't register goes to sparse-package.log (#84).
  Delete "$INSTDIR\sparse-package.log"
  nsExec::Exec 'powershell -NoProfile -NonInteractive -ExecutionPolicy Bypass -Command "try { Add-AppxPackage -Register \"$INSTDIR\AppxManifest.xml\" -ExternalLocation \"$INSTDIR\" -ErrorAction Stop } catch { $$_ | Out-File -Encoding utf8 \"$INSTDIR\sparse-package.log\" }"'

  WriteRegStr HKLM "Software\${APP}" "InstallDir" "$INSTDIR"
  WriteRegStr HKLM "${UNINSTALL_KEY}" "DisplayName" "${DISPLAY}"
  WriteRegStr HKLM "${UNINSTALL_KEY}" "DisplayVersion" "${DISPLAY_VERSION}"
  WriteRegStr HKLM "${UNINSTALL_KEY}" "Publisher" "The NectarLink contributors"
  WriteRegStr HKLM "${UNINSTALL_KEY}" "DisplayIcon" "$INSTDIR\${EXE}"
  WriteRegStr HKLM "${UNINSTALL_KEY}" "InstallLocation" "$INSTDIR"
  WriteRegStr HKLM "${UNINSTALL_KEY}" "URLInfoAbout" "https://github.com/thtbee/nectarlink"
  ; Windows Settings > Apps opens NectarLink's own uninstall window.
  WriteRegStr HKLM "${UNINSTALL_KEY}" "UninstallString" '"$INSTDIR\${EXE}" --remove'
  WriteRegStr HKLM "${UNINSTALL_KEY}" "QuietUninstallString" '"$INSTDIR\uninstall.exe" /S'
  WriteRegDWORD HKLM "${UNINSTALL_KEY}" "NoModify" 1
  WriteRegDWORD HKLM "${UNINSTALL_KEY}" "NoRepair" 1
  ${GetSize} "$INSTDIR" "/S=0K" $0 $1 $2
  WriteRegDWORD HKLM "${UNINSTALL_KEY}" "EstimatedSize" $0

  !insertmacro Progress "done"

  ; /RUN (the app's own updater): open the new version when done.
  ${GetParameters} $R0
  ClearErrors
  ${GetOptions} $R0 "/RUN" $R1
  ${IfNot} ${Errors}
    Call OpenApp
  ${EndIf}
SectionEnd

Section "Uninstall"
  !insertmacro Progress "step closing"
  !insertmacro QuitRunningApp
  ; What the app set up for this user: Send to entries, sign-in start,
  ; notification registration.
  ${If} ${FileExists} "$INSTDIR\${EXE}"
    ExecWait '"$INSTDIR\${EXE}" --uninstall'
  ${EndIf}

  !insertmacro Progress "step cleanup"
  nsExec::Exec 'powershell -NoProfile -NonInteractive -ExecutionPolicy Bypass -Command "try { Get-AppxPackage -Name \"Nectarlink.Desktop\" -ErrorAction SilentlyContinue | Remove-AppxPackage -ErrorAction SilentlyContinue } catch {}"'
  nsExec::Exec 'netsh advfirewall firewall delete rule name="${FIREWALL_RULE}"'
  nsExec::Exec 'schtasks /Delete /TN "Nectarlink Update" /F'
  Delete "$SMPROGRAMS\${DISPLAY}.lnk"
  Delete "$DESKTOP\${DISPLAY}.lnk"
  ; Only ever a folder of our own (a silent install can be pointed
  ; anywhere with /D): never everything in, say, C:\.
  ${GetFileName} "$INSTDIR" $0
  ${If} $0 == "${APP}"
    RMDir /r "$INSTDIR"
  ${Else}
    Delete "$INSTDIR\${EXE}"
    Delete "$INSTDIR\nectarlink_vcam.dll"
    Delete "$INSTDIR\AppxManifest.xml"
    RMDir /r "$INSTDIR\Assets"
    Delete "$INSTDIR\sparse-package.log"
    Delete "$INSTDIR\update.log"
    Delete "$INSTDIR\uninstall.exe"
  ${EndIf}

  DeleteRegKey HKLM "${UNINSTALL_KEY}"
  DeleteRegKey HKLM "Software\${APP}"
  !insertmacro Progress "done"
SectionEnd
