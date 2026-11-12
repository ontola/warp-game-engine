<#
.SYNOPSIS
Register an already packaged Sideways build with an explicit developer preview profile.
.DESCRIPTION
Copies the reference playtest's graphics environment into this game's launch entry.
It never builds, launches, stops, or contacts GameNight. Preview flags remain scoped
to this registration; engine defaults and the calling process environment are unchanged.
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory=$true)][string]$PackageDirectory,
    [Parameter(Mandatory=$true)][string]$ReferencePlaytestManifest,
    [string]$DataDirectory = (Join-Path $env:LOCALAPPDATA 'GameNight'),
    [ValidateSet('dlss-performance', 'dlss-balanced', 'dlss-quality')][string]$Upscale = 'dlss-performance',
    [switch]$DebugUi,
    [switch]$ValidateOnly
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

function Read-RegistrationProperty($Object, [string]$Name) {
    if ($null -eq $Object) { return $null }
    $property = $Object.PSObject.Properties[$Name]
    if ($null -ne $property) { return $property.Value }
    return $null
}

$packageRoot = (Resolve-Path -LiteralPath $PackageDirectory).ProviderPath
$referencePath = (Resolve-Path -LiteralPath $ReferencePlaytestManifest).ProviderPath
$dataRoot = [IO.Path]::GetFullPath($DataDirectory)
if (-not (Test-Path -LiteralPath $packageRoot -PathType Container)) { throw 'PackageDirectory must be an existing directory.' }
if (-not (Test-Path -LiteralPath $referencePath -PathType Leaf)) { throw 'ReferencePlaytestManifest must be an existing file.' }
foreach ($relative in @('assets', 'data', 'engine/assets/sound/ir')) {
    if (-not (Test-Path -LiteralPath (Join-Path $packageRoot $relative) -PathType Container)) {
        throw "Incomplete package: missing directory $relative."
    }
}
foreach ($relative in @('Sideways.exe', 'package.json', 'gamenight.json', 'CREDITS.md',
    'OFL-BarlowCondensed.txt', 'nvngx_dlss.dll', 'data/render-baseline.toml', 'data/cars.luau',
    'data/car_materials.toml', 'data/sounds.toml', 'assets/meshes/fir_a_near.kmesh',
    'assets/meshes/fir_b_near.kmesh', 'assets/meshes/fir_c_near.kmesh',
    'assets/trees/authored_pine_albedo.png', 'assets/grass/grass_medium_01_alpha_2k.png',
    'assets/sky/sky_2k.hdr', 'engine/assets/sound/ir/mild_exhaust.wav')) {
    if (-not (Test-Path -LiteralPath (Join-Path $packageRoot $relative) -PathType Leaf)) {
        throw "Incomplete package: missing file $relative."
    }
}
$executable = Join-Path $packageRoot 'Sideways.exe'
$package = [IO.File]::ReadAllText((Join-Path $packageRoot 'package.json')) | ConvertFrom-Json
$expectedHash = [string](Read-RegistrationProperty $package 'executable_sha256')
if ($expectedHash -notmatch '^[0-9a-fA-F]{64}$') { throw 'package.json must contain executable_sha256.' }
$actualHash = (Get-FileHash -LiteralPath $executable -Algorithm SHA256).Hash.ToLowerInvariant()
if ($actualHash -ne $expectedHash.ToLowerInvariant()) { throw 'Packaged Sideways.exe SHA256 does not match package.json.' }
$catalog = [IO.File]::ReadAllText((Join-Path $packageRoot 'gamenight.json')) | ConvertFrom-Json
if ((Read-RegistrationProperty $catalog 'id') -ne 'sideways') { throw 'Packaged gamenight.json must identify sideways.' }

$reference = [IO.File]::ReadAllText($referencePath) | ConvertFrom-Json
if ((Read-RegistrationProperty $reference 'status') -ne 'visible_first_frame') {
    throw 'ReferencePlaytestManifest must record a visible_first_frame developer playtest.'
}
$graphics = Read-RegistrationProperty $reference 'graphics_environment'
if ($null -eq $graphics -or $graphics -isnot [pscustomobject]) { throw 'Reference playtest must contain a graphics_environment object.' }
if ((Read-RegistrationProperty $graphics 'SIDEWAYS_PROCEDURAL_GRASS') -ne 'field') {
    throw 'Reference preview profile must explicitly select SIDEWAYS_PROCEDURAL_GRASS=field; otherwise photographed grass returns.'
}
if ((Read-RegistrationProperty $graphics 'KERN_MEADOW_AGGREGATE') -ne '1') {
    throw 'Reference preview profile must explicitly enable KERN_MEADOW_AGGREGATE=1.'
}
$launchEnvironment = [ordered]@{}
foreach ($property in $graphics.PSObject.Properties) {
    # Reject a contaminated profile instead of ever copying host tokens or arbitrary
    # process settings. Do not print its values, even in validation errors.
    if ($property.Name -notmatch '^(KERN_|SIDEWAYS_)|^WARP_AUDIO_IR_DIR$' -or
        $property.Name -match '(TOKEN|SECRET|PASSWORD|CREDENTIAL|API_KEY)' -or
        $property.Value -isnot [string]) {
        throw 'Reference graphics_environment contains an unsupported name or non-string value; only graphics settings may be copied.'
    }
    $launchEnvironment[$property.Name] = $property.Value
}
# References from before r59 have no AO key. Keep its default-off behavior
# explicit so an inherited daemon environment cannot silently enable the experiment.
if (-not $launchEnvironment.Contains('KERN_MEADOW_CONTACT_AO')) { $launchEnvironment['KERN_MEADOW_CONTACT_AO'] = '0' }
$launchEnvironment['SIDEWAYS_GAME_ROOT'] = $packageRoot
$launchEnvironment['WARP_AUDIO_IR_DIR'] = Join-Path $packageRoot 'engine/assets/sound/ir'
$launchEnvironment['KERN_DLSS_RUNTIME_DIR'] = $packageRoot
$launchArguments = @('play', '--fullscreen', '--size', '3840x2160', '--upscale', $Upscale, '--wind', '0.6')
if ($DebugUi) { $launchArguments += '--debug-ui' }

$libraryPath = Join-Path $dataRoot 'local-games.json'
$originalText = $null
$entries = @()
if (Test-Path -LiteralPath $libraryPath) {
    $originalText = [IO.File]::ReadAllText($libraryPath)
    if (-not $originalText.TrimStart().StartsWith('[')) { throw 'local-games.json must be a JSON array.' }
    $parsed = $originalText | ConvertFrom-Json
    if ($null -eq $parsed -and $originalText -notmatch '^\s*\[\s*\]\s*$') {
        throw 'local-games.json contains a null entry; registration left unchanged.'
    }
    if ($null -ne $parsed) { $entries = @($parsed) }
    foreach ($entry in $entries) {
        if ($entry -isnot [pscustomobject] -or [string]::IsNullOrWhiteSpace([string](Read-RegistrationProperty $entry 'id'))) {
            throw 'local-games.json contains an entry without an id; registration left unchanged.'
        }
    }
}
$existing = @($entries | Where-Object { (Read-RegistrationProperty $_ 'id') -eq 'sideways' })
if ($existing.Count -eq 0) { $existing = @($entries | Where-Object { (Read-RegistrationProperty $_ 'id') -eq 'sideways-kern' }) }
$sidewaysEntry = [ordered]@{}
if ($existing.Count -gt 0) {
    # Preserve local artwork and other metadata; replace the entire launch block,
    # so credentials or obsolete command arguments cannot survive from an old entry.
    foreach ($property in $existing[0].PSObject.Properties) { $sidewaysEntry[$property.Name] = $property.Value }
}
$sidewaysEntry['id'] = 'sideways'
$sidewaysEntry['title'] = 'Sideways'
$sidewaysEntry['tagline'] = [string](Read-RegistrationProperty $catalog 'tagline')
$sidewaysEntry['players'] = '1-4 players'
$sidewaysEntry['min_players'] = 1
$sidewaysEntry['max_players'] = 4
if (-not $sidewaysEntry.Contains('best_players')) { $sidewaysEntry['best_players'] = 2 }
$sidewaysEntry['launch'] = [ordered]@{
    command = $executable; cwd = $packageRoot; args = $launchArguments; env = $launchEnvironment
}
$updated = [Collections.Generic.List[object]]::new()
$inserted = $false
foreach ($entry in $entries) {
    if ((Read-RegistrationProperty $entry 'id') -in @('sideways', 'sideways-kern')) {
        if (-not $inserted) { $updated.Add($sidewaysEntry); $inserted = $true }
    } else { $updated.Add($entry) }
}
if (-not $inserted) { $updated.Add($sidewaysEntry) }
$updatedJson = ConvertTo-Json -InputObject @($updated.ToArray()) -Depth 64
$previousJson = ConvertTo-Json -InputObject @($entries) -Depth 64
$changed = $null -eq $originalText -or $previousJson -cne $updatedJson
$backupPath = $null
$status = if ($ValidateOnly) { 'validated' } elseif ($changed) { 'registered' } else { 'unchanged' }
if (-not $ValidateOnly -and $changed) {
    if (-not (Test-Path -LiteralPath $dataRoot)) { New-Item -ItemType Directory -Path $dataRoot | Out-Null }
    $temporary = Join-Path $dataRoot ('local-games.' + [Guid]::NewGuid().ToString('N') + '.tmp')
    try {
        [IO.File]::WriteAllText($temporary, $updatedJson + [Environment]::NewLine, [Text.UTF8Encoding]::new($false))
        if ($null -ne $originalText) {
            if ([IO.File]::ReadAllText($libraryPath) -cne $originalText) { throw 'local-games.json changed during registration; retry against the new contents.' }
            $backupPath = $libraryPath + '.before-sideways-' + [DateTime]::UtcNow.ToString('yyyyMMddTHHmmssfffZ') + '-' + [Guid]::NewGuid().ToString('N')
            [IO.File]::Replace($temporary, $libraryPath, $backupPath)
        } else { [IO.File]::Move($temporary, $libraryPath) }
    } finally {
        if (Test-Path -LiteralPath $temporary) { Remove-Item -LiteralPath $temporary }
    }
}
[pscustomobject]@{
    status = $status; changed = $changed; library = $libraryPath; backup = $backupPath
    package_sha256 = $actualHash; reference_playtest = $referencePath
    reference_sha256 = (Get-FileHash -LiteralPath $referencePath -Algorithm SHA256).Hash.ToLowerInvariant()
    entry = [pscustomobject]$sidewaysEntry
    scope = 'Registration only. No process launched or stopped; visible rendering requires a managed GameNight launch.'
}
