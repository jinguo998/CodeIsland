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
    $buildArgs = @('build', '-p', 'codeisland-hook', '-p', 'codeisland-cli', '--message-format=json-render-diagnostics')
    if ($ReleaseFlag) { $buildArgs += $ReleaseFlag }
    $records = & cargo @buildArgs
    if ($LASTEXITCODE -ne 0) { throw "cargo build failed ($LASTEXITCODE)" }
    $swiftBuild = $records | ForEach-Object { $_ | ConvertFrom-Json } | Where-Object {
        $_.reason -eq 'build-script-executed' -and $_.package_id -match 'codeisland-swift'
    } | Select-Object -First 1
    if (-not $swiftBuild) { throw "Cargo did not report the Swift core build directory." }

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
    $swiftRuntime = Join-Path $swiftBuild.out_dir "swift-core"
    $localRuntime = Join-Path $outputDir "swift-core"
    New-Item -ItemType Directory -Path $localRuntime -Force | Out-Null
    Get-ChildItem -LiteralPath $swiftRuntime -File | Copy-Item -Destination $localRuntime -Force
    $bundledRuntime = Join-Path $binaries "swift-core"
    New-Item -ItemType Directory -Path $bundledRuntime -Force | Out-Null
    Get-ChildItem -LiteralPath $swiftRuntime -File | Copy-Item -Destination $bundledRuntime -Force
    Write-Host "    staged sidecars in $binaries" -ForegroundColor Green
}
finally {
    Pop-Location
}
