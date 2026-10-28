# Growing Guns as a folder to copy to another Windows machine:
#   growing-guns.exe, assets/characters, mods, sounds (the patches) and audio
#   (the Godot project's recordings and the crowd samples, taken from a Godot
#   checkout because they are not in this repository).
#
#   cargo build --release -p growing-guns-app
#   games/growing-guns/tools/package-windows.ps1 -Executable target/release/growing-guns.exe `
#       -GodotProject C:/dev/growing-guns -OutDirectory C:/dev/gg-package
param(
    [Parameter(Mandatory=$true)][string]$Executable,
    [Parameter(Mandatory=$true)][string]$OutDirectory,
    [string]$GodotProject = ''
)
$ErrorActionPreference = 'Stop'
$gameRoot = Split-Path -Parent $PSScriptRoot
$sourceExe = (Resolve-Path -LiteralPath $Executable).Path
$destination = [IO.Path]::GetFullPath($OutDirectory)
if (Test-Path -LiteralPath $destination) { throw 'Package output must be a new directory' }
New-Item -ItemType Directory -Path $destination | Out-Null
Copy-Item -LiteralPath $sourceExe -Destination (Join-Path $destination 'growing-guns.exe')
New-Item -ItemType Directory -Path (Join-Path $destination 'assets') | Out-Null
Copy-Item -LiteralPath (Join-Path $gameRoot 'assets/characters') -Destination (Join-Path $destination 'assets') -Recurse
Copy-Item -LiteralPath (Join-Path $gameRoot 'mods') -Destination $destination -Recurse
New-Item -ItemType Directory -Path (Join-Path $destination 'sounds') | Out-Null
Get-ChildItem -LiteralPath (Join-Path $gameRoot 'sounds') -Filter '*.luau' | Copy-Item -Destination (Join-Path $destination 'sounds')
$recordings = 'none'
if ($GodotProject -ne '') {
    $audio = Join-Path $destination 'audio'
    New-Item -ItemType Directory -Path $audio | Out-Null
    Get-ChildItem -LiteralPath (Join-Path $GodotProject 'assets/audio') -Filter '*.wav' | Copy-Item -Destination $audio
    $crowd = Join-Path $GodotProject 'addons/crowd_sound/samples'
    if (Test-Path -LiteralPath $crowd) { Copy-Item -LiteralPath $crowd -Destination (Join-Path $audio 'crowd') -Recurse }
    $recordings = (Resolve-Path -LiteralPath $GodotProject).Path
}
$manifest = @{
    executable_sha256 = (Get-FileHash -LiteralPath $sourceExe -Algorithm SHA256).Hash.ToLowerInvariant()
    recordings_from = $recordings
    scope = 'Native game bundle. Without -GodotProject the footsteps, hurts, deaths and crowd are silent; the synthesised sounds play. Packaging is not proof of GPU acceptance.'
}
$manifest | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $destination 'package.json') -Encoding utf8
Write-Output $destination
