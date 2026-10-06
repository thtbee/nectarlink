# SPDX-License-Identifier: GPL-3.0-or-later
<#
.SYNOPSIS
  Installs, starts, quits and uninstalls the packaged app silently, checking
  each step. For CI (needs admin; changes this PC), not for everyday use.
#>
param([string]$Setup = (Get-ChildItem (Join-Path $PSScriptRoot '..\target\package\*-setup.exe') | Select-Object -First 1).FullName)

$ErrorActionPreference = 'Stop'
$dir = Join-Path $env:ProgramFiles 'Nectarlink'
$exe = Join-Path $dir 'nectarlink-desktop.exe'
$uninstallKey = 'HKLM:\Software\Microsoft\Windows\CurrentVersion\Uninstall\Nectarlink'
$shortcut = Join-Path $env:ProgramData 'Microsoft\Windows\Start Menu\Programs\Nectarlink.lnk'
$data = Join-Path $env:RUNNER_TEMP 'nectarlink-smoke'
if (-not $env:RUNNER_TEMP) { $data = Join-Path $env:TEMP 'nectarlink-smoke' }

function Check($what, [bool]$ok) {
    if (-not $ok) { throw "FAILED: $what" }
    Write-Host "ok: $what"
}
function FirewallRule { (netsh advfirewall firewall show rule name=Nectarlink) -match 'Nectarlink' }

Write-Host "Installing $Setup"
Start-Process $Setup -ArgumentList '/S' -Wait
Check "the app is installed" (Test-Path $exe)
Check "an uninstaller is installed" (Test-Path (Join-Path $dir 'uninstall.exe'))
Check "it's in Apps (uninstall entry)" (Test-Path $uninstallKey)
Check "it's in the Start menu" (Test-Path $shortcut)
Check "the firewall lets phones in" ([bool](FirewallRule))
Check "license notices are included" ((Get-Item (Join-Path $dir 'THIRD-PARTY-NOTICES.txt')).Length -gt 100KB)

# The installed copy starts on its own files and quits when asked.
$app = Start-Process $exe -ArgumentList '--minimized', '--data-dir', $data -PassThru
Start-Sleep 8
Check "the installed app keeps running" (-not $app.HasExited)
Start-Process $exe -ArgumentList '--quit', '--data-dir', $data -Wait
Check "the app quits when asked" ($app.WaitForExit(15000))
Check "it shut down cleanly" ($app.ExitCode -eq 0)

Write-Host "Uninstalling"
Start-Process (Join-Path $dir 'uninstall.exe') -ArgumentList '/S' -Wait
# The uninstaller runs from a copy in %TEMP%, so it returns before it's done.
$deadline = (Get-Date).AddSeconds(60)
while ((Test-Path $dir) -and (Get-Date) -lt $deadline) { Start-Sleep 1 }
Check "the app folder is gone" (-not (Test-Path $dir))
Check "the uninstall entry is gone" (-not (Test-Path $uninstallKey))
Check "the Start menu shortcut is gone" (-not (Test-Path $shortcut))
Check "the firewall rule is gone" (-not (FirewallRule))
Write-Host "Installer smoke test passed."
