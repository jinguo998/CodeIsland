#Requires -Version 5.1
<#
.SYNOPSIS
    Build CodeIsland Desktop release artifacts on Windows.

.DESCRIPTION
    Runs npm ci, the frontend build, builds the hook sidecar, runs the Tauri 2
    build, then copies the Windows bundles into an artifacts directory and
    writes release-manifest.json with SHA256 hashes for every file.

.PARAMETER Configuration
    Cargo configuration (default: Release).

.PARAMETER OutputDir
    Where to collect artifacts (default: <repo>/artifacts).

.EXAMPLE
    pwsh -NoProfile -File .\scripts\build-release.ps1
#>
[CmdletBinding()]
param(
    [string]$Configuration = "Release",
    [string]$OutputDir = ""
)

$ErrorActionPreference = "Stop"
$ProgressPreference = "SilentlyContinue"

$DesktopDir = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
if ([string]::IsNullOrWhiteSpace($OutputDir)) {
    $OutputDir = Join-Path $DesktopDir "artifacts"
}
$OutputDir = [System.IO.Path]::GetFullPath($OutputDir)
$ConfigurationLower = $Configuration.ToLowerInvariant()
$CargoTargetDir = if ($env:CARGO_TARGET_DIR) {
    [System.IO.Path]::GetFullPath($env:CARGO_TARGET_DIR)
} else {
    Join-Path $DesktopDir "src-tauri\target"
}

Push-Location $DesktopDir
try {
    Write-Host "==> CodeIsland Desktop release build" -ForegroundColor Cyan
    Write-Host "    workspace: $DesktopDir"
    Write-Host "    artifacts: $OutputDir"

    $package = Get-Content (Join-Path $DesktopDir "package.json") -Raw | ConvertFrom-Json
    $version = $package.version
    Write-Host "    version:   $version"

    Write-Host "==> npm ci" -ForegroundColor Cyan
    & npm ci
    if ($LASTEXITCODE -ne 0) { throw "npm ci failed ($LASTEXITCODE)" }

    Write-Host "==> npm run build" -ForegroundColor Cyan
    & npm run build
    if ($LASTEXITCODE -ne 0) { throw "frontend build failed ($LASTEXITCODE)" }

    Write-Host "==> hook sidecar" -ForegroundColor Cyan
    & powershell.exe -NoProfile -ExecutionPolicy Bypass -File (Join-Path $PSScriptRoot "prepare-sidecar.ps1") -Configuration $Configuration
    if ($LASTEXITCODE -ne 0) { throw "hook sidecar build failed ($LASTEXITCODE)" }

    # Prefer a globally installed cargo-tauri; fall back to the project's
    # @tauri-apps/cli so the script works on a clean Windows machine.
    Write-Host "==> tauri build" -ForegroundColor Cyan
    $cargoTauri = Get-Command cargo-tauri -ErrorAction SilentlyContinue
    if ($cargoTauri) {
        & cargo tauri build
    } else {
        & npm run tauri build
    }
    if ($LASTEXITCODE -ne 0) { throw "Tauri build failed ($LASTEXITCODE)" }

    $targetDir = Join-Path $CargoTargetDir $ConfigurationLower
    $bundleDir = Join-Path $targetDir "bundle"
    if (-not (Test-Path $bundleDir)) {
        throw "bundle directory not found: $bundleDir"
    }

    if (Test-Path $OutputDir) {
        Remove-Item $OutputDir -Recurse -Force
    }
    New-Item -ItemType Directory -Path $OutputDir | Out-Null

    $files = @()
    $patterns = @(
        (Join-Path $bundleDir "nsis\*.exe"),
        (Join-Path $bundleDir "msi\*.msi")
    )
    foreach ($pattern in $patterns) {
        Get-ChildItem -Path $pattern -File -ErrorAction SilentlyContinue | ForEach-Object {
            $destination = Join-Path $OutputDir $_.Name
            Copy-Item $_.FullName $destination -Force
            $files += $destination
        }
    }

    foreach ($name in @("codeisland-desktop.exe", "CodeIsland.exe")) {
        $exe = Join-Path $targetDir $name
        if (Test-Path $exe) {
            $destination = Join-Path $OutputDir "CodeIsland.exe"
            Copy-Item $exe $destination -Force
            $files += $destination
            break
        }
    }

    $hook = Join-Path $targetDir "codeisland-hook.exe"
    if (Test-Path $hook) {
        $destination = Join-Path $OutputDir "codeisland-hook.exe"
        Copy-Item $hook $destination -Force
        $files += $destination
    }

    if ($files.Count -eq 0) {
        throw "no release artifacts were produced under $bundleDir"
    }

    $swiftRuntime = Join-Path $OutputDir "swift-core"
    Copy-Item -LiteralPath (Join-Path $targetDir "swift-core") -Destination $swiftRuntime -Recurse
    $files += @(Get-ChildItem -LiteralPath $swiftRuntime -File | ForEach-Object { $_.FullName })

    $entries = foreach ($file in $files) {
        $item = Get-Item $file
        [ordered]@{
            name   = $item.FullName.Substring($OutputDir.Length + 1).Replace('\', '/')
            size   = $item.Length
            sha256 = (Get-FileHash $file -Algorithm SHA256).Hash.ToLowerInvariant()
        }
    }

    $manifest = [ordered]@{
        product       = "CodeIsland Desktop"
        version       = $version
        configuration = $Configuration
        generated_at  = (Get-Date).ToUniversalTime().ToString("o")
        files         = @($entries)
    }
    $manifestPath = Join-Path $OutputDir "release-manifest.json"
    $manifest | ConvertTo-Json -Depth 6 | Set-Content -Path $manifestPath -Encoding UTF8

    Write-Host "==> Done" -ForegroundColor Green
    Get-ChildItem $OutputDir | ForEach-Object {
        Write-Host ("    {0}  ({1:N0} bytes)" -f $_.Name, $_.Length)
    }
}
finally {
    Pop-Location
}
