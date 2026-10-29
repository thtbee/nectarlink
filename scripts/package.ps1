# SPDX-License-Identifier: GPL-3.0-or-later
<#
.SYNOPSIS
  Builds the Windows app and packs it into an installer.

.DESCRIPTION
  1. Builds the release app (unless -SkipBuild).
  2. Stages it with the Qt libraries, plugins, QML modules and translations it
     uses (windeployqt), the Microsoft C++ runtime, and the license notices.
  3. Packs the folder into target\package\Nectarlink-<version>-x64-setup.exe
     with NSIS (unless -StageOnly).

  Needs Qt (qmake on PATH, or QT_ROOT_DIR / -QtRoot) and, for the
  installer, NSIS (makensis on PATH or in its usual folder).

.EXAMPLE
  .\scripts\package.ps1
  .\scripts\package.ps1 -StageOnly   # just the folder, to try the app as shipped
#>
param(
    [string]$QtRoot,
    [ValidateSet('x64', 'arm64')]
    [string]$Arch = 'x64',
    [switch]$SkipBuild,
    [switch]$StageOnly
)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

function Step($name) { Write-Host "`n==> $name" -ForegroundColor Cyan }

# Runs a native command, showing its output; fails on a non-zero exit.
# (Windows PowerShell treats anything on stderr as an error otherwise.)
function Invoke-Native([string]$what, [scriptblock]$command) {
    $saved = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    try { & $command 2>&1 | ForEach-Object { Write-Host "$_" } }
    finally { $ErrorActionPreference = $saved }
    if ($LASTEXITCODE -ne 0) { throw "$what failed (exit code $LASTEXITCODE)" }
}

# ---- Version, from the workspace manifest ----
$version = (Select-String -Path Cargo.toml -Pattern '^version = "([^"]+)"' | Select-Object -First 1).Matches[0].Groups[1].Value
if (-not $version) { throw "can't read the version from Cargo.toml" }
# Windows file versions are numbers only: 0.0.1-beta.2 -> 0.0.1.
$numeric = ($version -split '[-+]')[0]

# ---- Target Architecture & Build Paths ----
$isArm64 = $Arch -eq 'arm64'
$targetDir = if ($isArm64) {
    Join-Path $root 'target\aarch64-pc-windows-msvc\release'
} else {
    Join-Path $root 'target\release'
}

# ---- Qt ----
if (-not $QtRoot) { $QtRoot = $env:QT_ROOT_DIR }
if (-not $QtRoot) {
    $qmake = Get-Command qmake6, qmake -ErrorAction SilentlyContinue | Select-Object -First 1
    if ($qmake) { $QtRoot = Split-Path -Parent (Split-Path -Parent $qmake.Source) }
}
if (-not $QtRoot) {
    $QtRoot = if ($isArm64) { 'C:\Qt\6.12.0\msvc2022_arm64' } else { 'C:\Qt\6.12.0\msvc2022_64' }
}

# Normalize Qt base folder and kits
$qtBase = if ($QtRoot -match 'msvc2022_(64|arm64)') { Split-Path -Parent $QtRoot } else { $QtRoot }
$hostKit = Join-Path $qtBase 'msvc2022_64'
$targetKit = if ($isArm64) { Join-Path $qtBase 'msvc2022_arm64' } else { $hostKit }

# windeployqt always runs as host (x64)
$windeployqt = Join-Path $hostKit 'bin\windeployqt.exe'
if (-not (Test-Path $windeployqt)) {
    # Fallback to QtRoot bin
    $windeployqt = Join-Path $QtRoot 'bin\windeployqt.exe'
}
if (-not (Test-Path $windeployqt)) { throw "windeployqt not found under $hostKit or $QtRoot; pass -QtRoot" }

$arm64ToolDir = Join-Path $env:LOCALAPPDATA "Nectarlink\dev\qmake-arm64"
if ($isArm64) {
    & (Join-Path $PSScriptRoot "qmake-arm64.ps1") -QtRoot $qtBase -OutDir $arm64ToolDir | Out-Null
    $qtpaths = Join-Path $arm64ToolDir 'qtpaths.exe'
    if (-not (Test-Path $qtpaths)) {
        $qtpaths = Join-Path $arm64ToolDir 'qtpaths6.exe'
    }
} else {
    $qtpaths = Join-Path $targetKit 'bin\qtpaths.exe'
    if (-not (Test-Path $qtpaths)) {
        $qtpaths = Join-Path $targetKit 'bin\qtpaths6.exe'
    }
}

# ---- Build ----
if (-not $SkipBuild) {
    Step "Build ($Arch)"
    if ($isArm64) {
        $env:QMAKE = Join-Path $arm64ToolDir "qmake6.exe"
        if (-not (Test-Path $env:QMAKE)) { $env:QMAKE = Join-Path $arm64ToolDir "qmake.exe" }
        Invoke-Native "the build" { cargo build --locked --release --target aarch64-pc-windows-msvc -p nectarlink-desktop -p nectarlink-vcam }
    } else {
        $env:PATH = "$(Join-Path $targetKit 'bin');$env:PATH"
        Invoke-Native "the build" { cargo build --locked --release -p nectarlink-desktop -p nectarlink-vcam }
    }
}
$exe = Join-Path $targetDir 'nectarlink-desktop.exe'
if (-not (Test-Path $exe)) { throw "no release build at $exe" }
$vcamDll = Join-Path $targetDir 'nectarlink_vcam.dll'
if (-not (Test-Path $vcamDll)) { throw "no release build at $vcamDll" }

# ---- Stage ----
Step "Stage"
$package = Join-Path $root 'target\package'
$stage = Join-Path $package 'Nectarlink'
if (Test-Path $stage) {
    $stagedVcam = Join-Path $stage 'nectarlink_vcam.dll'
    if (Test-Path $stagedVcam) {
        & regsvr32.exe /s /u $stagedVcam 2>$null | Out-Null
        try {
            Remove-Item $stagedVcam -Force -ErrorAction Stop
        } catch {
            # Windows Frame Server (svchost) may keep a mapped handle to the DLL;
            # NTFS allows moving a mapped file on the same volume so staging stays clean.
            $trashDir = Join-Path $root 'target\package-locked'
            New-Item -ItemType Directory -Force $trashDir | Out-Null
            Move-Item $stagedVcam (Join-Path $trashDir "nectarlink_vcam.$([DateTime]::UtcNow.Ticks).dll") -Force -ErrorAction SilentlyContinue
        }
    }
    Remove-Item $stage -Recurse -Force
}
New-Item -ItemType Directory -Force $stage | Out-Null
Copy-Item $exe $stage
Copy-Item $vcamDll $stage

Invoke-Native "windeployqt" {
    $deployArgs = @(
        '--verbose', '0',
        '--release',
        '--qmldir', (Join-Path $root 'desktop\app\qml'),
        '--no-system-d3d-compiler',
        '--no-opengl-sw',
        '--no-compiler-runtime',
        '--dir', $stage
    )
    if ($isArm64) {
        if (Test-Path $qtpaths) {
            $deployArgs = @('--qtpaths', $qtpaths) + $deployArgs
        }
        $env:PATH = "$arm64ToolDir;$(Join-Path $hostKit 'bin');$env:PATH"
    }
    & $windeployqt @deployArgs (Join-Path $stage 'nectarlink-desktop.exe')
}

# Translations. windeployqt stages Qt's own .qm files (in translations/, so its
# file dialogs and standard buttons follow the system language). NectarLink's
# are compiled from the .ts files the repository keeps; they land beside the
# executable, which is where the app looks for them.
# Only the languages Settings offers (i18n::LANGUAGES) ship: a catalogue still being
# translated stays out, so "System" never switches to a half-translated language.
$i18nSource = Get-Content (Join-Path $root 'desktop\app\src\i18n.rs') -Raw
$offeredLanguages = @([regex]::Matches(($i18nSource -split 'pub const LANGUAGES')[1].Split(';')[0], '\("([A-Za-z_]+)"') |
    ForEach-Object { $_.Groups[1].Value })
$translationSources = @()
if (Test-Path (Join-Path $root 'desktop\app\i18n')) {
    $translationSources = @(Get-ChildItem (Join-Path $root 'desktop\app\i18n') -Filter '*.ts' |
        Where-Object { $offeredLanguages -contains ($_.BaseName -replace '^nectarlink_', '') })
}
if ($translationSources.Count -gt 0) {
    # A host tool: the x64 kit's, as an ARM64 kit's own can't run here.
    $lrelease = Join-Path $hostKit 'bin\lrelease.exe'
    if (-not (Test-Path $lrelease)) { $lrelease = Join-Path $QtRoot 'bin\lrelease.exe' }
    if (-not (Test-Path $lrelease)) { throw "lrelease not found under $hostKit or $QtRoot; pass -QtRoot" }
    foreach ($source in $translationSources) {
        $ts = $source.FullName
        $qm = Join-Path $stage "$($source.BaseName).qm"
        Invoke-Native "lrelease $($source.Name)" { & $lrelease $ts -qm $qm }
    }
}

# QtQuick.Dialogs brings every Controls style along. The file picker is
# Windows' own dialog; only Basic, Windows and FluentWinUI3 (the default
# style on Windows) can ever be loaded, so the rest is left out.
foreach ($style in 'Fusion', 'Imagine', 'Material', 'Universal') {
    Remove-Item (Join-Path $stage "Qt6QuickControls2$style*.dll") -Force
    Remove-Item (Join-Path $stage "qml\QtQuick\Controls\$style") -Recurse -Force -ErrorAction SilentlyContinue
}

# The C++ runtime, next to the app (Microsoft allows this app-local copy),
# so it runs on PCs without the Visual C++ Redistributable.
$vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
$vs = if (Test-Path $vswhere) {
    $component = if ($isArm64) { 'Microsoft.VisualStudio.Component.VC.Tools.ARM64' } else { 'Microsoft.VisualStudio.Component.VC.Tools.x86.x64' }
    & $vswhere -latest -products * -requires $component -property installationPath
} else { $null }
$crtArch = if ($isArm64) { 'arm64' } else { 'x64' }
$redistDirs = @()
if ($vs) { $redistDirs += (Join-Path $vs 'VC\Redist\MSVC') }
if ($env:VCToolsRedistDir -and (Test-Path $env:VCToolsRedistDir)) { $redistDirs += $env:VCToolsRedistDir }
$crt = $null
foreach ($dir in $redistDirs) {
    if (Test-Path $dir) {
        $crt = Get-ChildItem $dir -Directory -ErrorAction SilentlyContinue |
            Where-Object { $_.Name -match '^\d' } | Sort-Object { [version]$_.Name } -Descending |
            ForEach-Object { Get-ChildItem (Join-Path $_.FullName $crtArch) -Directory -Filter 'Microsoft.VC*.CRT' -ErrorAction SilentlyContinue } |
            Select-Object -First 1
        if (-not $crt) {
            $crt = Get-ChildItem (Join-Path $dir $crtArch) -Directory -Filter 'Microsoft.VC*.CRT' -ErrorAction SilentlyContinue | Select-Object -First 1
        }
        if ($crt) { break }
    }
}
if (-not $crt) { throw "can't find the Visual C++ runtime ($crtArch) to ship" }
Copy-Item (Join-Path $crt.FullName '*.dll') $stage

# Licenses.
Copy-Item (Join-Path $root 'LICENSE') (Join-Path $stage 'LICENSE.txt')
Invoke-Native "writing the notices" { cargo xtask notices (Join-Path $stage 'THIRD-PARTY-NOTICES.txt') }

# The icon build.rs drew for the .exe, and Sparse Package assets for Windows Share sheet.
$icon = Get-ChildItem (Join-Path $targetDir 'build') -Recurse -Filter 'nectarlink.ico' |
    Sort-Object LastWriteTime -Descending | Select-Object -First 1
if (-not $icon) { throw "the app icon isn't built" }
$assetsDir = Join-Path $stage 'Assets'
New-Item -ItemType Directory -Force $assetsDir | Out-Null
Add-Type -AssemblyName System.Drawing
foreach ($spec in @(@{ Name = 'Square44x44Logo.png'; Size = 44 }, @{ Name = 'Square150x150Logo.png'; Size = 150 })) {
    $ico = New-Object System.Drawing.Icon($icon.FullName, $spec.Size, $spec.Size)
    $bmp = New-Object System.Drawing.Bitmap($spec.Size, $spec.Size)
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
    $g.DrawIcon($ico, (New-Object System.Drawing.Rectangle(0, 0, $spec.Size, $spec.Size)))
    $g.Dispose()
    $bmp.Save((Join-Path $assetsDir $spec.Name), [System.Drawing.Imaging.ImageFormat]::Png)
    $bmp.Dispose()
    $ico.Dispose()
}
$manifest = Get-Content (Join-Path $root 'desktop\installer\AppxManifest.xml') -Raw
# Only the package's own version: MinVersion is the oldest Windows it runs on.
$manifest = $manifest -replace '(<Identity\b[^>]*?\s)Version="[^"]*"', "`${1}Version=`"$numeric.0`""
$manifest = $manifest -replace 'ProcessorArchitecture="x64"', "ProcessorArchitecture=`"$Arch`""
Set-Content -Path (Join-Path $stage 'AppxManifest.xml') -Value $manifest -Encoding UTF8 -NoNewline

# Windows checks the manifest only when setup registers it, and setup can't
# show why it failed, so it's checked here the way makeappx packs it (#84).
$makeappx = Get-ChildItem "${env:ProgramFiles(x86)}\Windows Kits\10\bin\*\x64\makeappx.exe" -ErrorAction SilentlyContinue |
    Sort-Object { [version]$_.Directory.Parent.Name } -Descending | Select-Object -First 1
if ($makeappx) {
    $check = Join-Path ([IO.Path]::GetTempPath()) "nectarlink-appx-$PID"
    try {
        New-Item -ItemType Directory -Force (Join-Path $check 'content') | Out-Null
        Copy-Item (Join-Path $stage 'AppxManifest.xml'), $assetsDir -Destination (Join-Path $check 'content') -Recurse
        # The app is outside the package (its install folder); any file stands in for it.
        Set-Content (Join-Path $check 'content\nectarlink-desktop.exe') 'app'
        Invoke-Native "the package manifest check" {
            & $makeappx.FullName pack /o /d (Join-Path $check 'content') /p (Join-Path $check 'check.msix')
        }
    } finally {
        Remove-Item -Recurse -Force $check -ErrorAction SilentlyContinue
    }
} else {
    Write-Warning "makeappx isn't installed (Windows SDK), so the package manifest isn't checked"
}

$size = (Get-ChildItem $stage -Recurse -File | Measure-Object Length -Sum).Sum / 1MB
Write-Host ("Staged {0:N1} MB in {1}" -f $size, $stage)
if ($StageOnly) { return }

# ---- Installer ----
Step "Installer ($Arch)"
$makensis = (Get-Command makensis -ErrorAction SilentlyContinue).Source
if (-not $makensis) { $makensis = Join-Path ${env:ProgramFiles(x86)} 'NSIS\makensis.exe' }
if (-not (Test-Path $makensis)) { throw "NSIS (makensis) not found" }
$out = Join-Path $package "Nectarlink-$version-$Arch-setup.exe"
Invoke-Native "makensis" {
    & $makensis /V2 "/DVERSION=$numeric" "/DDISPLAY_VERSION=$version" "/DARCH=$Arch" "/DSTAGE=$stage" `
        "/DICON=$($icon.FullName)" "/DOUTFILE=$out" (Join-Path $root 'desktop\installer\nectarlink.nsi')
}
Write-Host ("Installer: {0} ({1:N1} MB)" -f $out, ((Get-Item $out).Length / 1MB))
