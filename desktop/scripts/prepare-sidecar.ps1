#Requires -Version 5.1
<#
.SYNOPSIS
    Builds the hook and CLI binaries and stages them as Tauri sidecars.

.DESCRIPTION
    Tauri's bundler expects sidecar binaries at
    src-tauri/binaries/<name>-<target-triple>.exe. This script builds
    codeisland-hook and codeisland from the workspace and copies them there.
#>
[CmdletBinding()]
param(
    [string]$Configuration = "Release"
)

$ErrorActionPreference = "Stop"
$DesktopDir = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$SrcTauri = Join-Path $DesktopDir "src-tauri"
$CargoTargetDir = if ($env:CARGO_TARGET_DIR) {
    [System.IO.Path]::GetFullPath($env:CARGO_TARGET_DIR)
} else {
    Join-Path $SrcTauri "target"
}
$Profile = if ($Configuration -ieq "Debug") { "debug" } else { "release" }
$ReleaseFlag = if ($Profile -eq "release") { "--release" } else { "" }

$triple = (& rustc -vV | Select-String -Pattern '^host: (.+)$').Matches[0].Groups[1].Value.Trim()
if ([string]::IsNullOrWhiteSpace($triple)) {
    $triple = "x86_64-pc-windows-msvc"
}

Push-Location $SrcTauri
try {
    Write-Host "==> cargo build hook + cli ($Profile, $triple)" -ForegroundColor Cyan
    if ($ReleaseFlag) {
        & cargo build -p codeisland-hook -p codeisland-cli --release
    } else {
        & cargo build -p codeisland-hook -p codeisland-cli
    }
    if ($LASTEXITCODE -ne 0) { throw "cargo build failed ($LASTEXITCODE)" }

    $binaries = Join-Path $SrcTauri "binaries"
    New-Item -ItemType Directory -Path $binaries -Force | Out-Null

    $outputDir = Join-Path $CargoTargetDir $Profile
    foreach ($name in @("codeisland-hook", "codeisland")) {
        $source = Join-Path $outputDir "$name.exe"
        if (-not (Test-Path $source)) {
            throw "missing $source (did the workspace build?)"
        }
        Copy-Item $source (Join-Path $binaries "$name-$triple.exe") -Force
    }
    Write-Host "    staged sidecars in $binaries" -ForegroundColor Green
}
finally {
    Pop-Location
}
