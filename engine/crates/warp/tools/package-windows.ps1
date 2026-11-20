# Package a Luau game for Windows: warp.exe with the game folder next to it
# in `game/`. Started without arguments, warp.exe runs that folder, which is
# how a Splitscreen Club host launches it (the package's entrypoint is warp.exe).
#   ./engine/crates/warp/tools/package-windows.ps1 -Game games/hello-warp -Executable target/release/warp.exe -OutDirectory build/hello-warp
param([Parameter(Mandatory=$true)][string]$Game,[Parameter(Mandatory=$true)][string]$Executable,[Parameter(Mandatory=$true)][string]$OutDirectory)
$ErrorActionPreference='Stop'
$gameRoot=(Resolve-Path -LiteralPath $Game).Path
# A game folder holds main.luau, or exactly one other .luau file (a one-file game).
$scriptsAtTop=@(Get-ChildItem -LiteralPath $gameRoot -File -Filter '*.luau' | Where-Object { $_.Name -notlike '*.d.luau' })
if(-not (Test-Path -LiteralPath (Join-Path $gameRoot 'main.luau')) -and $scriptsAtTop.Count -ne 1){throw "$Game has no main.luau (or one .luau file)"}
$destination=[IO.Path]::GetFullPath($OutDirectory)
if(Test-Path -LiteralPath $destination){throw 'Package output must be a new directory'}
New-Item -ItemType Directory -Path $destination | Out-Null
Copy-Item -LiteralPath (Resolve-Path -LiteralPath $Executable).Path -Destination (Join-Path $destination 'warp.exe')
Copy-Item -LiteralPath $gameRoot -Destination (Join-Path $destination 'game') -Recurse
# Headless playtests stay in the repository; the package is what players run.
$scripts=Join-Path $destination 'game/scripts'
if(Test-Path -LiteralPath $scripts){Remove-Item -LiteralPath $scripts -Recurse}
# Recordings a samples.lock lists travel inside the package (fetched and
# hash-checked here), so players never download them.
$packaged=Join-Path $destination 'game'
if(Test-Path -LiteralPath (Join-Path $packaged 'samples.lock')){
  & (Join-Path $destination 'warp.exe') samples $packaged --bundle
  if($LASTEXITCODE -ne 0){throw 'warp samples --bundle failed'}
}
Write-Output $destination
