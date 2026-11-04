# SPDX-License-Identifier: GPL-3.0-or-later
<#
.SYNOPSIS
  Runs the desktop app's QML tests (desktop/app/tests/qml/tst_*.qml).

.DESCRIPTION
  Puts the app's shared components and its Theme, Tokens and Icons
  singletons in a QML module named app.nectarlink under target/qml-tests,
  with the stubs in desktop/app/tests/qml/stubs standing in for the Rust
  singletons they read, and runs qmltestrunner over the tests, off screen.
  Needs Qt (qmake on PATH, or QT_ROOT_DIR / -QtRoot).
#>
param(
    [string]$QtRoot
)

$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot

if (-not $QtRoot) { $QtRoot = $env:QT_ROOT_DIR }
if (-not $QtRoot) {
    $qmake = Get-Command qmake6, qmake -ErrorAction SilentlyContinue | Select-Object -First 1
    if ($qmake) { $QtRoot = Split-Path -Parent (Split-Path -Parent $qmake.Source) }
}
if (-not $QtRoot) { $QtRoot = 'C:\Qt\6.12.0\msvc2022_64' }
$runner = Join-Path $QtRoot 'bin\qmltestrunner.exe'
if (-not (Test-Path $runner)) { throw "qmltestrunner not found under $QtRoot; pass -QtRoot" }

$qml = Join-Path $repo 'desktop\app\qml'
$tests = Join-Path $repo 'desktop\app\tests\qml'
$imports = Join-Path $repo 'target\qml-tests'
$module = Join-Path $imports 'app\nectarlink'
if (Test-Path $imports) { Remove-Item -Recurse -Force $imports }
New-Item -ItemType Directory -Force $module | Out-Null

$singletons = @('Theme', 'Tokens', 'Icons')
foreach ($name in $singletons) { Copy-Item (Join-Path $qml "$name.qml") $module }
$components = Get-ChildItem (Join-Path $qml 'components') -Filter *.qml
$components | Copy-Item -Destination $module
Copy-Item (Join-Path $qml 'IslandWindow.qml') $module
$stubs = Get-ChildItem (Join-Path $tests 'stubs') -Filter *.qml
$stubs | Copy-Item -Destination $module

$lines = @('module app.nectarlink')
foreach ($name in $singletons + @($stubs | ForEach-Object BaseName)) {
    $lines += "singleton $name 1.0 $name.qml"
}
foreach ($component in $components) {
    $lines += "$($component.BaseName) 1.0 $($component.Name)"
}
$lines += 'IslandWindow 1.0 IslandWindow.qml'
[System.IO.File]::WriteAllLines((Join-Path $module 'qmldir'), $lines)

$env:PATH = "$(Join-Path $QtRoot 'bin');$env:PATH"
& $runner -platform offscreen -import $imports -input $tests -o "-,txt"
if ($LASTEXITCODE -ne 0) { throw "QML tests failed" }
