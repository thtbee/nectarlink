# SPDX-License-Identifier: GPL-3.0-or-later
<#
.SYNOPSIS
    Checks that every source file starts with an SPDX license header that
    matches the license map in LICENSE. Exits non-zero on any problem.
#>
$ErrorActionPreference = "Stop"
Push-Location (Split-Path -Parent $PSScriptRoot)

# Which license a source file must declare (mirrors LICENSE).
function Get-ExpectedLicense([string]$path) {
    if ($path -like "core/nectarlink-cli/*") { return "GPL-3.0-or-later" }
    if ($path -like "core/*") { return "MPL-2.0" }
    return "GPL-3.0-or-later"
}

try {
    $extensions = @(".rs", ".qml", ".cpp", ".h", ".hpp", ".ps1", ".sh", ".kt", ".kts")
    $files = @(git ls-files --cached --others --exclude-standard) |
        Where-Object { $extensions -contains [System.IO.Path]::GetExtension($_) } |
        Where-Object { Test-Path $_ }
    $problems = 0
    foreach ($file in $files) {
        $head = Get-Content $file -TotalCount 5 -ErrorAction SilentlyContinue
        $line = $head | Where-Object { $_ -match "SPDX-License-Identifier:\s*(\S+)" } | Select-Object -First 1
        if (-not $line) {
            Write-Host "  missing SPDX header: $file" -ForegroundColor Red
            $problems++
            continue
        }
        $null = $line -match "SPDX-License-Identifier:\s*(\S+)"
        $expected = Get-ExpectedLicense $file
        if ($Matches[1] -ne $expected) {
            Write-Host "  wrong license in $file ($($Matches[1]), expected $expected)" -ForegroundColor Red
            $problems++
        }
    }
    Write-Host "  $($files.Count) files checked, $problems problem(s)"
    exit [int]($problems -gt 0)
} finally {
    Pop-Location
}
