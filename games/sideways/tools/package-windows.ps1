param([Parameter(Mandatory=$true)][string]$Executable,[Parameter(Mandatory=$true)][string]$OutDirectory)
$ErrorActionPreference='Stop'
$gameRoot=Split-Path -Parent $PSScriptRoot
$repoRoot=Split-Path -Parent (Split-Path -Parent $gameRoot)
$sourceExe=(Resolve-Path -LiteralPath $Executable).Path
$destination=[IO.Path]::GetFullPath($OutDirectory)
if(Test-Path -LiteralPath $destination){throw 'Package output must be a new directory'}
New-Item -ItemType Directory -Path $destination | Out-Null
Copy-Item -LiteralPath $sourceExe -Destination (Join-Path $destination 'Sideways.exe')
foreach($name in @('assets','data','CREDITS.md','gamenight.json')) {
    Copy-Item -LiteralPath (Join-Path $gameRoot $name) -Destination $destination -Recurse
}
$engineAssets=Join-Path $destination 'engine/assets'
New-Item -ItemType Directory -Path $engineAssets -Force | Out-Null
Copy-Item -LiteralPath (Join-Path $repoRoot 'engine/assets/sound') -Destination $engineAssets -Recurse
Copy-Item -LiteralPath (Join-Path $repoRoot 'engine/assets/fonts/OFL-BarlowCondensed.txt') -Destination $destination
$manifest=@{executable_sha256=(Get-FileHash -LiteralPath $sourceExe -Algorithm SHA256).Hash.ToLowerInvariant();
    scope='Native game bundle. DLSS SDK runtime is optional and must be supplied separately for DLSS-enabled builds. Packaging is not proof of GPU acceptance.'}
$manifest | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $destination 'package.json') -Encoding utf8
Write-Output $destination
