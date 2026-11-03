# SPDX-License-Identifier: GPL-3.0-or-later
<#
.SYNOPSIS
  The update every user takes: installs the last published release from
  GitHub (checked against its SHA-256 sums), runs it once on a data folder,
  then installs this build over it silently, as the update task does, and
  checks the new version runs on the same data and quits cleanly. For CI
  (needs admin; changes this PC), not for everyday use.
#>
param(
    [string]$Setup,
    # The release to update from; the latest published one when left out.
    [string]$From
)

$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

function Check($what, [bool]$ok) {
    if (-not $ok) { throw "FAILED: $what" }
    Write-Host "ok: $what"
}

if (-not $Setup) {
    $Setup = (Get-ChildItem (Join-Path $PSScriptRoot '..\target\package\*-x64-setup.exe') | Select-Object -First 1).FullName
}
if (-not $Setup -or -not (Test-Path $Setup)) { throw "no x64 setup installer to test under target\package" }
if ($Setup -notmatch 'Nectarlink-(.+)-x64-setup\.exe$') { throw "can't read the version from $Setup" }
$newVersion = $Matches[1]

$api = 'https://api.github.com/repos/thtbee/nectarlink/releases'
$headers = @{ 'User-Agent' = 'nectarlink-ci' }
if ($env:GH_TOKEN) { $headers['Authorization'] = "Bearer $env:GH_TOKEN" }
$release = if ($From) { Invoke-RestMethod "$api/tags/$From" -Headers $headers } else { Invoke-RestMethod "$api/latest" -Headers $headers }
$oldVersion = $release.tag_name.TrimStart('v')
if ($oldVersion -eq $newVersion) {
    Write-Host "This build is $newVersion, the latest release itself: nothing to update from."
    exit 0
}
$oldName = "Nectarlink-$oldVersion-x64-setup.exe"
$work = Join-Path ($(if ($env:RUNNER_TEMP) { $env:RUNNER_TEMP } else { $env:TEMP })) 'nectarlink-upgrade'
Remove-Item -Recurse -Force $work -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force $work | Out-Null
$oldSetup = Join-Path $work $oldName
$sums = Join-Path $work 'SHA256SUMS.txt'
$asset = { param($name) ($release.assets | Where-Object name -eq $name).browser_download_url }
Invoke-WebRequest (& $asset $oldName) -OutFile $oldSetup -Headers @{ 'User-Agent' = 'nectarlink-ci' }
Invoke-WebRequest (& $asset 'SHA256SUMS.txt') -OutFile $sums -Headers @{ 'User-Agent' = 'nectarlink-ci' }
$listed = (Get-Content $sums | Where-Object { $_ -match "^([0-9a-f]{64})\s+\*?$([regex]::Escape($oldName))$" } | ForEach-Object { $Matches[1] }) | Select-Object -First 1
Check "$oldName is the published one (SHA-256)" ($listed -and (Get-FileHash $oldSetup -Algorithm SHA256).Hash.ToLower() -eq $listed)

$dir = Join-Path $env:ProgramFiles 'Nectarlink'
$exe = Join-Path $dir 'nectarlink-desktop.exe'
$uninstallKey = 'HKLM:\Software\Microsoft\Windows\CurrentVersion\Uninstall\Nectarlink'
$data = Join-Path $work 'data'
function DisplayVersion { (Get-ItemProperty $uninstallKey -ErrorAction SilentlyContinue).DisplayVersion }
function RunOnce([string]$what) {
    $app = Start-Process $exe -ArgumentList '--minimized', '--data-dir', $data -PassThru
    Start-Sleep 8
    Check "$what keeps running" (-not $app.HasExited)
    Start-Process $exe -ArgumentList '--quit', '--data-dir', $data -Wait
    Check "$what quits when asked" ($app.WaitForExit(15000))
    Check "$what shut down cleanly" ($app.ExitCode -eq 0)
}

Write-Host "Installing $oldVersion"
Start-Process $oldSetup -ArgumentList '/S' -Wait
Check "$oldVersion is installed" ((DisplayVersion) -eq $oldVersion)
RunOnce $oldVersion
$identity = Join-Path $data 'identity.key'
Check "$oldVersion made its data" (Test-Path $identity)
$identityHash = (Get-FileHash $identity -Algorithm SHA256).Hash

Write-Host "Updating to $newVersion"
Start-Process $Setup -ArgumentList '/S' -Wait
Check "$newVersion replaced it" ((DisplayVersion) -eq $newVersion)
$scheduler = New-Object -ComObject Schedule.Service
$scheduler.Connect()
$task = try { $scheduler.GetFolder('\').GetTask('Nectarlink Update') } catch { $null }
Check "the update task is there after the update" ($null -ne $task -and $task.Definition.Actions.Item(1).Path.Trim('"') -eq $exe)
# Windows' camera service can hold the webcam DLL open; the update must still
# put the new one in place rather than skip it.
$staged = Join-Path (Split-Path $Setup) 'Nectarlink'
if (Test-Path $staged) {
    $stale = @(Get-ChildItem $staged -Recurse -File | Where-Object {
        $installed = Join-Path $dir $_.FullName.Substring($staged.Length + 1)
        -not (Test-Path $installed) -or (Get-FileHash $installed).Hash -ne (Get-FileHash $_.FullName).Hash
    } | ForEach-Object { $_.Name })
    Check "the update put every file in place$(if ($stale) { " (not: $($stale -join ', '))" })" ($stale.Count -eq 0)
}
RunOnce $newVersion
Check "the update kept this PC's identity" ((Get-FileHash $identity -Algorithm SHA256).Hash -eq $identityHash)

Write-Host "Uninstalling"
Start-Process (Join-Path $dir 'uninstall.exe') -ArgumentList '/S' -Wait
$deadline = (Get-Date).AddSeconds(60)
while ((Test-Path $dir) -and (Get-Date) -lt $deadline) { Start-Sleep 1 }
# What the camera service still has loaded is deleted at the next restart.
$pending = @((Get-ItemProperty 'HKLM:\SYSTEM\CurrentControlSet\Control\Session Manager' -ErrorAction SilentlyContinue).PendingFileRenameOperations |
    Where-Object { $_ } | ForEach-Object { $_ -replace '^(\*\d+)?\\\?\?\\', '' })
$left = @(Get-ChildItem $dir -Recurse -File -Force -ErrorAction SilentlyContinue | Where-Object { $pending -notcontains $_.FullName } | ForEach-Object { $_.Name })
Check "it uninstalls after the update$(if ($left) { " (left: $($left -join ', '))" })" ($left.Count -eq 0)
Remove-Item -Recurse -Force $work -ErrorAction SilentlyContinue
Write-Host "Upgrade from $oldVersion passed."
exit 0
