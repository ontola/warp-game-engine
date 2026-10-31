[CmdletBinding()]
param(
    [ValidateSet('dev','release')][string]$Profile = 'dev',
    [string]$TargetDirectory = $env:CARGO_TARGET_DIR
)
$ErrorActionPreference = 'Stop'
$workspace = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../../..'))
$game = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
if (-not $TargetDirectory) { $TargetDirectory = Join-Path $workspace 'target' }
$TargetDirectory = [IO.Path]::GetFullPath($TargetDirectory)
$prior = $env:CARGO_TARGET_DIR
Push-Location $workspace
try {
    $env:CARGO_TARGET_DIR = $TargetDirectory
    & cargo build --locked -p spaceracer-app --profile $Profile
    if ($LASTEXITCODE -ne 0) { throw 'SpaceRacer native build failed' }
    $folder = if ($Profile -eq 'dev') { 'debug' } else { 'release' }
    $output = Join-Path $workspace 'build/spaceracer-native'
    $content = Join-Path $output 'spaceracer'
    New-Item -ItemType Directory -Force -Path $content | Out-Null
    Copy-Item -LiteralPath (Join-Path $TargetDirectory "$folder/spaceracer.exe") -Destination (Join-Path $output 'spaceracer.exe') -Force
    Copy-Item -LiteralPath (Join-Path $game 'rules') -Destination $content -Recurse -Force
    Copy-Item -LiteralPath (Join-Path $game 'assets') -Destination $content -Recurse -Force
    Copy-Item -LiteralPath (Join-Path $game 'licenses') -Destination $content -Recurse -Force
    Write-Output "Built $output/spaceracer.exe (native development port; see games/spaceracer/README.md for gaps)"
} finally {
    Pop-Location
    $env:CARGO_TARGET_DIR = $prior
}
