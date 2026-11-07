# SPDX-License-Identifier: GPL-3.0-or-later
<#
.SYNOPSIS
  Makes sure NSIS is installed, for building the Windows installer.

.DESCRIPTION
  Installs it with Chocolatey if it's missing. Chocolatey's feed fails on CI
  runners now and then, sometimes for a while, and choco can report that
  without failing. So this tries a couple of times, then installs the official
  download instead, checked against a pinned hash since it goes into releases.
#>
param(
    [int]$Attempts = 2,
    [int]$WaitSeconds = 15,
    [string]$InstallerUrl = 'https://downloads.sourceforge.net/project/nsis/NSIS%203/3.13/nsis-3.13-setup.exe'
)

$ErrorActionPreference = 'Stop'

# The same hash Chocolatey's nsis.install 3.13.0 lists.
$installerSha256 = 'DB9A1038BBC0E53FEF24F423FD609979BD578E942F1BE1198DB186D1C42652A3'

function Test-Nsis {
    (Get-Command makensis -ErrorAction SilentlyContinue) -or
        (Test-Path (Join-Path ${env:ProgramFiles(x86)} 'NSIS\makensis.exe'))
}

if (Test-Nsis) {
    Write-Host 'NSIS is already installed.'
    exit 0
}

for ($attempt = 1; $attempt -le $Attempts; $attempt++) {
    if ($attempt -gt 1) {
        Write-Host "Trying Chocolatey again in $WaitSeconds s (attempt $attempt of $Attempts)."
        Start-Sleep -Seconds $WaitSeconds
    }
    choco install nsis --yes --no-progress
    if (Test-Nsis) {
        Write-Host 'NSIS is installed.'
        exit 0
    }
}

Write-Host 'Chocolatey did not install NSIS. Using the official download.'
$installer = Join-Path ([IO.Path]::GetTempPath()) 'nsis-setup.exe'
# curl, not Invoke-WebRequest: SourceForge answers PowerShell with a web page.
curl.exe -fsSL --retry 3 --retry-delay 10 -o $installer $InstallerUrl
if ($LASTEXITCODE -ne 0) { throw "The NSIS download failed (curl exit $LASTEXITCODE)." }
$hash = (Get-FileHash $installer -Algorithm SHA256).Hash
if ($hash -ne $installerSha256) {
    Remove-Item $installer -Force
    throw "The NSIS download has the wrong hash: $hash"
}
Start-Process $installer -ArgumentList '/S' -Wait
Remove-Item $installer -Force
if (-not (Test-Nsis)) {
    throw "NSIS couldn't be installed."
}
Write-Host 'NSIS is installed.'
