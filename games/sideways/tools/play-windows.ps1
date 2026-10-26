param(
    [ValidateRange(1, 4)][int]$Views = 1,
    [ValidatePattern('^\d+x\d+$')][string]$Size = '3840x2160',
    [ValidateSet('native', 'dlss-quality', 'dlss-balanced', 'dlss-performance')][string]$Upscale = 'native',
    [switch]$Windowed
)

# Explicit user launch only. Automated checks use the invisible smoke CLI.
$ErrorActionPreference = 'Stop'
$gameRoot = Split-Path -Parent $PSScriptRoot
$gameExecutable = Join-Path $gameRoot 'build\sideways.exe'
if (-not (Test-Path -LiteralPath $gameExecutable)) {
    throw 'Build the Windows playtest first: tools\build-windows.ps1'
}
$gameArguments = @('play', '--views', "$Views", '--size', $Size, '--upscale', $Upscale)
if (-not $Windowed) { $gameArguments += '--fullscreen' }
Push-Location $gameRoot
try {
    & $gameExecutable @gameArguments
    exit $LASTEXITCODE
} finally {
    Pop-Location
}
