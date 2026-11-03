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
; The setup window can pick another folder (#102): /D=<folder> (last, as
; NSIS wants) and /ADMINONLY=1 when only administrators can change the
; folders above it. Updates go to the folder of the last install. The app's
; update task runs it as SYSTEM, and installing runs the old copy to close
; it, so that only happens where nobody else can swap the files: Program
; Files, or a folder above which only administrators can change anything.
; Elsewhere updates ask for permission instead. Folders setup creates
; outside Program Files are locked like Program Files.
;
; Installing for everyone lets it allow the app through Windows Firewall:
; otherwise Windows asks on first use, and people often say no.
;
; The uninstaller (uninstall.exe) is always silent. Windows Settings runs
; the app's own uninstall window (--remove), which runs uninstall.exe
; elevated and then removes the user's data.
;
; Defines passed in: VERSION (numbers only, e.g. 0.0.1), BUILD (the file
; version's fourth number: a nightly's number, 999 for a release), DISPLAY_VERSION
; (e.g. 0.0.1-beta.2), ARCH (x64 or arm64), STAGE (the folder with the app
; and its Qt files), ICON, OUTFILE.

Unicode true
ManifestDPIAware true
SetCompressor /SOLID zlib

!include "x64.nsh"
!include "LogicLib.nsh"
!include "FileFunc.nsh"
!include "WordFunc.nsh"

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

VIProductVersion "${VERSION}.${BUILD}"
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
; Where an earlier copy was installed, if there is one.
Var PreviousDir
; 1 when only administrators can change $INSTDIR and the folders above it.
Var AdminOnly

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
    ; Without /D (an update): where the last install went. InstallDirRegKey
    ; can't say, as it reads the 32-bit registry before SetRegView.
    ${If} $INSTDIR == "$PROGRAMFILES64\${APP}"
      ReadRegStr $0 HKLM "Software\${APP}" "InstallDir"
      ${If} $0 != ""
      ${AndIf} ${FileExists} "$0\${EXE}"
        StrCpy $INSTDIR $0
      ${EndIf}
    ${EndIf}
    ; Installing needs administrator rights: ask, and let the elevated copy
    ; do it. (The app's own updater runs as SYSTEM and has them already.)
    ; NSIS takes /D off the command line, so it's passed on again.
    UserInfo::GetAccountType
    Pop $0
    ${If} $0 != "Admin"
      ExecShell "runas" '"$EXEPATH"' "$R0 /D=$INSTDIR" SW_HIDE
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
  StrCpy $AdminOnly 0
  StrLen $0 "$PROGRAMFILES64\"
  StrCpy $1 "$INSTDIR" $0
  StrLen $0 "$PROGRAMFILES\"
  StrCpy $2 "$INSTDIR" $0
  ${If} $1 == "$PROGRAMFILES64\"
  ${OrIf} $2 == "$PROGRAMFILES\"
    StrCpy $AdminOnly 1
  ${Else}
    ReadRegStr $PreviousDir HKLM "Software\${APP}" "InstallDir"
    ${If} $INSTDIR == $PreviousDir
    ${AndIf} ${FileExists} "$INSTDIR\${EXE}"
      ReadRegDWORD $AdminOnly HKLM "Software\${APP}" "AdminOnlyFolder"
      ${If} $AdminOnly != 1
        StrCpy $AdminOnly 0
      ${EndIf}
    ${EndIf}
  ${EndIf}
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
  ; Only where nobody else can have replaced it: this runs with
  ; administrator rights.
  ${If} $AdminOnly == 1
  ${AndIf} ${FileExists} "$INSTDIR\${EXE}"
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

; Windows' camera service (Frame Server) keeps the webcam's DLL, and the
; runtime it needs, loaded from this folder for as long as it runs, so they
; can't be overwritten or deleted: a silent install would skip them and keep
; the old ones for good. A loaded DLL can still be renamed, so one that can't
; be deleted is moved aside and deleted at the next restart (under its new
; name, so a later install's file isn't); the service keeps its copy until
; then.
!macro MoveAsideInUse FILE
  Delete "$INSTDIR\${FILE}"
  ${If} ${FileExists} "$INSTDIR\${FILE}"
    StrCpy $R2 0
    ${Do}
      IntOp $R2 $R2 + 1
    ${LoopWhile} ${FileExists} "$INSTDIR\${FILE}.$R2.old"
    Rename "$INSTDIR\${FILE}" "$INSTDIR\${FILE}.$R2.old"
    Delete /REBOOTOK "$INSTDIR\${FILE}.$R2.old"
  ${EndIf}
!macroend

!macro MoveAsideCameraFiles
  ; Moved aside earlier and no longer loaded.
  Delete "$INSTDIR\*.dll.*.old"
  !insertmacro MoveAsideInUse "nectarlink_vcam.dll"
  !insertmacro MoveAsideInUse "vcruntime140.dll"
  !insertmacro MoveAsideInUse "vcruntime140_1.dll"
  !insertmacro MoveAsideInUse "msvcp140.dll"
!macroend

; Locks folder $R1 the way Program Files is: owned by administrators, which
; SYSTEM and administrators may change and everyone else only read. Anything
; in it before then could have been put there by someone else, so it must be
; empty. Sets the error flag when it can't.
Function LockFolder
  ; Never follow a junction or symbolic link (FILE_ATTRIBUTE_REPARSE_POINT).
  System::Call 'kernel32::GetFileAttributesW(w "$R1") i .r0'
  IntOp $1 $0 & 0x400
  ${If} $0 == -1
  ${OrIf} $1 != 0
    SetErrors
    Return
  ${EndIf}
  nsExec::Exec 'icacls "$R1" /setowner *S-1-5-32-544'
  Pop $0
  ${If} $0 == 0
    nsExec::Exec 'icacls "$R1" /reset'
    Pop $0
  ${EndIf}
  ${If} $0 == 0
    nsExec::Exec 'icacls "$R1" /inheritance:r /grant:r *S-1-5-18:(OI)(CI)F *S-1-5-32-544:(OI)(CI)F *S-1-5-32-545:(OI)(CI)RX *S-1-15-2-1:(OI)(CI)RX'
    Pop $0
  ${EndIf}
  StrCpy $4 0
  ${If} $0 == 0
    FindFirst $2 $3 "$R1\*.*"
    ${DoWhile} $3 != ""
      ${If} $3 != "."
      ${AndIf} $3 != ".."
        StrCpy $4 1
        ${Break}
      ${EndIf}
      FindNext $2 $3
    ${Loop}
    FindClose $2
  ${EndIf}
  ClearErrors
  ${If} $0 != 0
  ${OrIf} $4 == 1
    SetErrors
  ${EndIf}
FunctionEnd

; Creates $INSTDIR outside Program Files one folder at a time from the
; first one that exists, locking each (LockFolder) before going in, so
; nobody else can put anything on the way. Sets the error flag when it can't.
Function CreateLockedInstallDir
  StrCpy $R0 "$INSTDIR"
  StrCpy $R2 0
  ${Do}
    ${If} ${FileExists} "$R0\*.*"
      ${Break}
    ${EndIf}
    Push $R0
    IntOp $R2 $R2 + 1
    ${GetParent} "$R0" $R0
    ${If} $R0 == ""
      ${Break}
    ${EndIf}
  ${Loop}
  ; The folder itself already there (empty, setup checked): lock it too.
  ${If} $R2 == 0
    Push "$INSTDIR"
    StrCpy $R2 1
  ${EndIf}
  StrCpy $R3 0
  ${DoWhile} $R2 > 0
    Pop $R1
    IntOp $R2 $R2 - 1
    ${If} $R3 == 0
      CreateDirectory "$R1"
      Call LockFolder
      ${If} ${Errors}
        StrCpy $R3 1
      ${EndIf}
    ${EndIf}
  ${Loop}
  ClearErrors
  ${If} $R3 == 1
    SetErrors
  ${EndIf}
FunctionEnd

Function OpenApp
  ; Through Explorer, so the app runs as the user and not elevated.
  Exec '"$WINDIR\explorer.exe" "$INSTDIR\${EXE}"'
FunctionEnd

Section "Install"
  ${If} $Interactive == 1
    InitPluginsDir
    StrCpy $AppDir "$PLUGINSDIR\app"
  ${Else}
    ReadRegStr $PreviousDir HKLM "Software\${APP}" "InstallDir"
    ${If} $PreviousDir != ""
    ${AndIfNot} ${FileExists} "$PreviousDir\${EXE}"
      StrCpy $PreviousDir ""
    ${EndIf}
    ; Program Files is admin-only. Elsewhere: the setup window's check
    ; (/ADMINONLY), or, for an update, what the last install found.
    StrCpy $AdminOnly 0
    StrLen $0 "$PROGRAMFILES64\"
    StrCpy $1 "$INSTDIR" $0
    StrLen $0 "$PROGRAMFILES\"
    StrCpy $2 "$INSTDIR" $0
    ${If} $1 == "$PROGRAMFILES64\"
    ${OrIf} $2 == "$PROGRAMFILES\"
      StrCpy $AdminOnly 1
    ${Else}
      ${GetParameters} $R0
      ClearErrors
      ${GetOptions} $R0 "/ADMINONLY=" $R1
      ${If} ${Errors}
        ${If} $INSTDIR == $PreviousDir
          ReadRegDWORD $AdminOnly HKLM "Software\${APP}" "AdminOnlyFolder"
        ${EndIf}
      ${ElseIf} $R1 == 1
        StrCpy $AdminOnly 1
      ${EndIf}
      ${If} $AdminOnly != 1
        StrCpy $AdminOnly 0
      ${EndIf}
      ${If} $INSTDIR != $PreviousDir
        Call CreateLockedInstallDir
        ${If} ${Errors}
          !insertmacro Progress "error Setup couldn't make that folder safe for NectarLink. Choose another folder."
          SetErrorLevel 2
          Quit
        ${EndIf}
      ${EndIf}
    ${EndIf}
    !insertmacro Progress "step closing"
    !insertmacro QuitRunningApp
    !insertmacro Progress "step files"
    ; A clean copy: nothing left over from an older version.
    RMDir /r "$INSTDIR\qml"
    RMDir /r "$INSTDIR\platforms"
    !insertmacro MoveAsideCameraFiles
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
  ; Shared folder for update downloads and channel choice: any signed-in user
  ; on this PC may update them, and the update task checks the download's
  ; SHA-256 in SystemTemp before running it.
  ; Owned by administrators, and set on the folder alone: never /T, which
  ; would follow a junction a user put inside to wherever it points. What
  ; is created in it from now on inherits the grant.
  CreateDirectory "$APPDATA\${APP}\updates"
  nsExec::Exec 'icacls "$APPDATA\${APP}" /setowner *S-1-5-32-544'
  nsExec::Exec 'icacls "$APPDATA\${APP}" /grant *S-1-5-32-545:(OI)(CI)M'
  ${If} $AdminOnly != 1
    ; Updates ask for permission in this folder (see the top of this file).
    nsExec::Exec 'schtasks /Delete /TN "Nectarlink Update" /F'
    Goto updates_done
  ${EndIf}
  InitPluginsDir
  ${WordReplace} "$INSTDIR" "&" "&amp;" "+" $1
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
  FileWriteUTF16LE $0 '    <Exec><Command>"$1\${EXE}"</Command><Arguments>--update-service</Arguments></Exec>$\r$\n'
  FileWriteUTF16LE $0 '  </Actions>$\r$\n'
  FileWriteUTF16LE $0 '</Task>$\r$\n'
  FileClose $0
  nsExec::Exec 'schtasks /Create /TN "Nectarlink Update" /XML "$PLUGINSDIR\update-task.xml" /F'
  updates_done:

  !insertmacro Progress "step shortcuts"
  CreateShortCut "$SMPROGRAMS\${DISPLAY}.lnk" "$INSTDIR\${EXE}" "" "$INSTDIR\${EXE}" 0 "" "" "Your phone, on your PC"
  ; The desktop shortcut is a choice in the setup window (/DESKTOP=1 or 0).
  ; Without one (the app's updater), a first install adds it and an update
  ; keeps it only if it's still there, so a deleted one stays deleted (#102).
  ${GetParameters} $R0
  ClearErrors
  ${GetOptions} $R0 "/DESKTOP=" $R1
  ${If} ${Errors}
    StrCpy $R1 1
    ${If} $PreviousDir != ""
    ${AndIfNot} ${FileExists} "$DESKTOP\${DISPLAY}.lnk"
      StrCpy $R1 0
    ${EndIf}
  ${EndIf}
  ${If} $R1 == 1
    CreateShortCut "$DESKTOP\${DISPLAY}.lnk" "$INSTDIR\${EXE}" "" "$INSTDIR\${EXE}" 0 "" "" "Your phone, on your PC"
  ${Else}
    Delete "$DESKTOP\${DISPLAY}.lnk"
  ${EndIf}

  ; The Windows Share sheet needs the app's sparse package registered, which
  ; Windows allows for an unsigned one only with Developer Mode on. Why it
  ; didn't register goes to sparse-package.log (#84). The update task runs as
  ; SYSTEM, which Windows doesn't allow to register per-user packages; the
  ; app registers it when reopened after an update.
  Delete "$INSTDIR\sparse-package.log"
  System::Call 'kernel32::SetEnvironmentVariableW(w "NECTARLINK_INSTDIR", w "$INSTDIR")'
  nsExec::Exec 'powershell -NoProfile -NonInteractive -ExecutionPolicy Bypass -Command "if (-not [Security.Principal.WindowsIdentity]::GetCurrent().IsSystem) { try { Add-AppxPackage -Register \"$$env:NECTARLINK_INSTDIR\AppxManifest.xml\" -ExternalLocation $$env:NECTARLINK_INSTDIR -ErrorAction Stop } catch { $$_ | Out-File -Encoding utf8 -LiteralPath \"$$env:NECTARLINK_INSTDIR\sparse-package.log\" } }"'

  WriteRegStr HKLM "Software\${APP}" "InstallDir" "$INSTDIR"
  WriteRegDWORD HKLM "Software\${APP}" "AdminOnlyFolder" $AdminOnly
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
  ${If} $AdminOnly == 1
  ${AndIf} ${FileExists} "$INSTDIR\${EXE}"
    ExecWait '"$INSTDIR\${EXE}" --uninstall'
  ${EndIf}

  !insertmacro Progress "step cleanup"
  nsExec::Exec 'powershell -NoProfile -NonInteractive -ExecutionPolicy Bypass -Command "try { Get-AppxPackage -Name \"Nectarlink.Desktop\" -ErrorAction SilentlyContinue | Remove-AppxPackage -ErrorAction SilentlyContinue } catch {}"'
  nsExec::Exec 'netsh advfirewall firewall delete rule name="${FIREWALL_RULE}"'
  nsExec::Exec 'schtasks /Delete /TN "Nectarlink Update" /F'
  ; Users can write here, so only our own files go, and the folders only
  ; when empty: a recursive delete as administrator could follow a junction.
  Delete "$APPDATA\${APP}\updates\*.exe"
  Delete "$APPDATA\${APP}\updates\*.part"
  Delete "$APPDATA\${APP}\update-channel"
  Delete "$APPDATA\${APP}\update-feed.json"
  RMDir "$APPDATA\${APP}\updates"
  RMDir "$APPDATA\${APP}"
  Delete "$SMPROGRAMS\${DISPLAY}.lnk"
  Delete "$DESKTOP\${DISPLAY}.lnk"
  ; Only ever a folder of our own (a silent install can be pointed
  ; anywhere with /D): never everything in, say, C:\. What the camera
  ; service still has loaded goes at the next restart.
  ${GetFileName} "$INSTDIR" $0
  ${If} $0 == "${APP}"
    !insertmacro MoveAsideCameraFiles
    RMDir /r /REBOOTOK "$INSTDIR"
  ${Else}
    Delete "$INSTDIR\${EXE}"
    !insertmacro MoveAsideInUse "nectarlink_vcam.dll"
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
