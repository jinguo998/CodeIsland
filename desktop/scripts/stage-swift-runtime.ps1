# The redistributed Swift runtime includes Foundation's ICU data and networking
# dependencies. Keep these next to the core, independent of the user's PATH.
param(
    [Parameter(Mandatory)][string]$Library,
    [Parameter(Mandatory)][string]$Destination
)
$ErrorActionPreference = "Stop"
$runtime = $env:Path.Split(';') | Where-Object {
    $_ -and (Test-Path -LiteralPath (Join-Path $_ 'swiftCore.dll') -PathType Leaf)
} | Select-Object -First 1
if (-not $runtime) { throw "Swift's redistributable runtime (swiftCore.dll) is missing from PATH." }
Get-ChildItem -LiteralPath $runtime -File | Where-Object {
    $_.Extension -in '.dll', '.dat'
} | Copy-Item -Destination $Destination -Force
if (-not (Test-Path -LiteralPath (Join-Path $Destination (Split-Path -Leaf $Library)))) {
    throw "Swift core library was not staged."
}
