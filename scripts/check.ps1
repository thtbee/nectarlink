# SPDX-License-Identifier: GPL-3.0-or-later
<#
.SYNOPSIS
    Runs the same checks as CI: formatting, lints, tests, license headers,
    generated files and the dependency policy. Exits non-zero if any step
    fails.

.DESCRIPTION
    Every step runs even after a failure, so one run shows everything that
    needs fixing. Steps that need Qt (the desktop crates) use QMAKE or the
    qmake on PATH.

.PARAMETER Arm64
    Also cross-compiles the workspace for Windows ARM64.

.PARAMETER Android
    Also builds the Android app, runs its unit tests and lint (needs the Android
    SDK, NDK and cargo-ndk; see android/README.md).

.PARAMETER Fast
    Skips the slow steps (tests and the dependency audit).

.EXAMPLE
    ./scripts/check.ps1
    ./scripts/check.ps1 -Arm64
#>
param(
    [switch]$Arm64,
    [switch]$Android,
    [switch]$Fast
)
$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
Push-Location $root

$results = New-Object System.Collections.Generic.List[object]

function Invoke-Step {
    param([string]$Name, [scriptblock]$Body)
    Write-Host ""
    Write-Host "==> $Name" -ForegroundColor Cyan
    $timer = [System.Diagnostics.Stopwatch]::StartNew()
    $ok = $true
    $global:LASTEXITCODE = 0 # a previous step's exit code must not leak in
    # Windows PowerShell turns a native tool's stderr (cargo's progress
    # output) into errors when output is redirected; judge by exit code.
    $ErrorActionPreference = "Continue"
    try {
        & $Body
        if ($LASTEXITCODE -ne 0) { $ok = $false }
    } catch {
        Write-Host $_ -ForegroundColor Red
        $ok = $false
    }
    $results.Add([pscustomobject]@{ Step = $Name; Ok = $ok; Seconds = [math]::Round($timer.Elapsed.TotalSeconds, 1) })
}

try {
    Invoke-Step "Format" { cargo fmt --all --check }
    Invoke-Step "Clippy" { cargo clippy --workspace --all-targets --locked -- -D warnings }
    if (-not $Fast) {
        Invoke-Step "Tests" { cargo test --workspace --locked }
    }
    Invoke-Step "License headers" { & (Join-Path $PSScriptRoot "check-headers.ps1") }
    Invoke-Step "Generated files" { cargo xtask tokens --check }
    if (-not $Fast) {
        Invoke-Step "Dependency policy" {
            if (-not (Get-Command cargo-deny -ErrorAction SilentlyContinue)) {
                throw "cargo-deny is not installed: cargo install --locked cargo-deny"
            }
            $output = cargo deny --log-level error check 2>&1 | Out-String
            if ($LASTEXITCODE -ne 0 -and $output -match "unable to access|Could not resolve host|failed to fetch") {
                # Offline: check against the advisories fetched last time.
                Write-Host "  advisory database unreachable; using the cached copy" -ForegroundColor Yellow
                $output = cargo deny --offline --log-level error check 2>&1 | Out-String
            }
            Write-Host $output.Trim()
        }
    }
    if ($Arm64) {
        Invoke-Step "ARM64 build" {
            $previous = $env:QMAKE
            try {
                $env:QMAKE = & (Join-Path $PSScriptRoot "qmake-arm64.ps1")
                cargo build --workspace --release --locked --target aarch64-pc-windows-msvc
            } finally {
                $env:QMAKE = $previous
            }
        }
    }
    if ($Android) {
        Invoke-Step "Android" {
            Push-Location (Join-Path $root "android")
            try {
                if (-not $env:JAVA_HOME) { $env:JAVA_HOME = "C:\Program Files\Android\Android Studio\jbr" }
                & .\gradlew.bat --console=plain -q testDebugUnitTest lintDebug assembleDebug
            } finally {
                Pop-Location
            }
        }
    }
} finally {
    Pop-Location
}

Write-Host ""
$results | Format-Table -AutoSize @{ L = "Step"; E = { $_.Step } }, @{ L = "Result"; E = { if ($_.Ok) { "ok" } else { "FAILED" } } }, @{ L = "Time (s)"; E = { $_.Seconds } }
$failed = @($results | Where-Object { -not $_.Ok }).Count
if ($failed -gt 0) {
    Write-Host "$failed step(s) failed." -ForegroundColor Red
    exit 1
}
Write-Host "All checks passed." -ForegroundColor Green
