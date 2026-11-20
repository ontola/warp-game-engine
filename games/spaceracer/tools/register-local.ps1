[CmdletBinding()]
param(
    [string]$BuildDirectory = (Join-Path $PSScriptRoot '../../../build/spaceracer-native'),
    [string]$DataDirectory = (Join-Path $env:LOCALAPPDATA 'Splitscreen Club')
)
$ErrorActionPreference = 'Stop'
$build = [IO.Path]::GetFullPath($BuildDirectory)
$exe = Join-Path $build 'spaceracer.exe'
if (-not (Test-Path -LiteralPath $exe -PathType Leaf)) { throw "Build SpaceRacer first: $exe" }
if (-not (Test-Path -LiteralPath (Join-Path $build 'spaceracer/rules/party.luau'))) { throw 'Build is missing Splitscreen Club settings' }
New-Item -ItemType Directory -Force -Path $DataDirectory | Out-Null
$stamp = Get-Date -Format 'yyyyMMdd-HHmmss-fff'
foreach ($name in @('local-games.json', 'shelf.json')) {
    $path = Join-Path $DataDirectory $name
    if ((-not (Test-Path -LiteralPath $path)) -and $name -eq 'shelf.json') { continue }
    $games = if (Test-Path -LiteralPath $path) { @(Get-Content -LiteralPath $path -Raw | ConvertFrom-Json) } else { @() }
    $entry = $games | Where-Object { $_.id -eq 'spaceracer' } | Select-Object -First 1
    if (-not $entry) {
        if ($name -eq 'shelf.json') { continue }
        $entry = [pscustomobject]@{ id='spaceracer'; title='SpaceRacer (Local)'; players='1-4 players' }
        $games += $entry
    }
    $entry | Add-Member -Force -NotePropertyName min_players -NotePropertyValue 1
    $entry | Add-Member -Force -NotePropertyName max_players -NotePropertyValue 4
    $entry | Add-Member -Force -NotePropertyName launch -NotePropertyValue ([pscustomobject]@{
        command=$exe; cwd=$build; args=@('--fullscreen')
    })
    if (Test-Path -LiteralPath $path) { Copy-Item -LiteralPath $path -Destination "$path.before-warp-$stamp.bak" }
    $json = ConvertTo-Json -InputObject @($games) -Depth 100
    [IO.File]::WriteAllText("$path.tmp", $json, [Text.UTF8Encoding]::new($false))
    Move-Item -LiteralPath "$path.tmp" -Destination $path -Force
}
Write-Output "SpaceRacer local launch now points to $exe"
Write-Output 'Restart Splitscreen Club to reload its library. Existing processes were not interrupted.'
