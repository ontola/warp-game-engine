# Package a Luau game for Windows: warp.exe with the game folder next to it
# in `game/`. Started without arguments, warp.exe runs that folder, which is
# how a GameNight host launches it (the package's entrypoint is warp.exe).
#   ./engine/crates/warp/tools/package-windows.ps1 -Game games/hello-warp -Executable target/release/warp.exe -OutDirectory build/hello-warp
param([Parameter(Mandatory=$true)][string]$Game,[Parameter(Mandatory=$true)][string]$Executable,[Parameter(Mandatory=$true)][string]$OutDirectory)
$ErrorActionPreference='Stop'
$gameRoot=(Resolve-Path -LiteralPath $Game).Path
if(-not (Test-Path -LiteralPath (Join-Path $gameRoot 'main.luau'))){throw "$Game has no main.luau"}
$destination=[IO.Path]::GetFullPath($OutDirectory)
if(Test-Path -LiteralPath $destination){throw 'Package output must be a new directory'}
New-Item -ItemType Directory -Path $destination | Out-Null
Copy-Item -LiteralPath (Resolve-Path -LiteralPath $Executable).Path -Destination (Join-Path $destination 'warp.exe')
Copy-Item -LiteralPath $gameRoot -Destination (Join-Path $destination 'game') -Recurse
# Headless playtests stay in the repository; the package is what players run.
$scripts=Join-Path $destination 'game/scripts'
if(Test-Path -LiteralPath $scripts){Remove-Item -LiteralPath $scripts -Recurse}
Write-Output $destination
