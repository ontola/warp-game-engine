param([string]$TargetDirectory)

$ErrorActionPreference = 'Stop'
$engineRoot = Split-Path -Parent $PSScriptRoot
$buildArguments = @('build', '--locked', '--release', '-p', 'cli')
if ($TargetDirectory) { $buildArguments += @('--target-dir', $TargetDirectory) }
Push-Location $engineRoot
try {
    & cargo @buildArguments
    if ($LASTEXITCODE -ne 0) { throw "Cargo build failed with exit code $LASTEXITCODE" }
    $metadataText = & cargo metadata --format-version 1 --no-deps
    if ($LASTEXITCODE -ne 0) { throw 'Cannot read Cargo target directory' }
    $cargoTarget = if ($TargetDirectory) { $TargetDirectory } else { ($metadataText | ConvertFrom-Json).target_directory }
    $builtExecutable = Join-Path $cargoTarget 'release\sideways.exe'
    $previewDirectory = Join-Path $engineRoot 'build'
    New-Item -ItemType Directory -Path $previewDirectory -Force | Out-Null
    Copy-Item -LiteralPath $builtExecutable -Destination (Join-Path $previewDirectory 'sideways.exe')
    Get-FileHash -LiteralPath (Join-Path $previewDirectory 'sideways.exe') -Algorithm SHA256
} finally {
    Pop-Location
}
