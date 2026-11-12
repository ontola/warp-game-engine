# Isolated registration checks: fake executable bytes are hashed, never executed.
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
function Assert-That([bool]$Condition, [string]$Message) { if (-not $Condition) { throw $Message } }
function Write-FixtureJson([string]$Path, $Value) {
    [IO.File]::WriteAllText($Path, (ConvertTo-Json -InputObject $Value -Depth 32), [Text.UTF8Encoding]::new($false))
}
function Assert-Rejected([scriptblock]$Action, [string]$Pattern) {
    $message = ''
    try { & $Action | Out-Null } catch { $message = $_.Exception.Message }
    Assert-That ($message -match $Pattern) "Expected rejection matching '$Pattern', received '$message'."
}
$helper = Join-Path $PSScriptRoot 'register-gamenight-windows.ps1'
$tempRoot = [IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd('\', '/')
$fixtureRoot = Join-Path $tempRoot ('sideways-register-' + [Guid]::NewGuid().ToString('N'))
try {
    $packageRoot = Join-Path $fixtureRoot 'package with spaces'
    $dataRoot = Join-Path $fixtureRoot 'isolated GameNight'
    foreach ($relative in @('assets/meshes', 'assets/trees', 'assets/grass', 'assets/sky', 'data', 'engine/assets/sound/ir')) {
        New-Item -ItemType Directory -Path (Join-Path $packageRoot $relative) -Force | Out-Null
    }
    New-Item -ItemType Directory -Path $dataRoot | Out-Null
    foreach ($relative in @('Sideways.exe', 'CREDITS.md', 'OFL-BarlowCondensed.txt', 'nvngx_dlss.dll',
        'data/render-baseline.toml', 'data/cars.luau', 'data/car_materials.toml', 'data/sounds.toml',
        'assets/meshes/fir_a_near.kmesh', 'assets/meshes/fir_b_near.kmesh', 'assets/meshes/fir_c_near.kmesh',
        'assets/trees/authored_pine_albedo.png', 'assets/grass/grass_medium_01_alpha_2k.png',
        'assets/sky/sky_2k.hdr', 'engine/assets/sound/ir/mild_exhaust.wav')) {
        [IO.File]::WriteAllText((Join-Path $packageRoot $relative), "fixture only: $relative")
    }
    $executable = Join-Path $packageRoot 'Sideways.exe'
    $executableHash = (Get-FileHash -LiteralPath $executable -Algorithm SHA256).Hash.ToLowerInvariant()
    Write-FixtureJson (Join-Path $packageRoot 'package.json') @{ executable_sha256 = $executableHash }
    Write-FixtureJson (Join-Path $packageRoot 'gamenight.json') @{ id = 'sideways'; tagline = 'Fixture rally' }
    $referencePath = Join-Path $fixtureRoot 'iteration.json'
    $reference = [ordered]@{
        status = 'visible_first_frame'
        graphics_environment = [ordered]@{
            SIDEWAYS_PROCEDURAL_GRASS = 'field'; KERN_MEADOW_AGGREGATE = '1'
            SIDEWAYS_MEADOW_SOURCE_BAND = '25,35'; SIDEWAYS_MEADOW_BLADE_BUDGET = '120'
            KERN_MEADOW_AGGREGATE_SAMPLE_PRUNE = '1'; KERN_MEADOW_AGGREGATE_HALF_RES = '0'
            SIDEWAYS_TREE_FULLNESS = '1.25'; SIDEWAYS_SKY = 'procedural'
            SIDEWAYS_GAME_ROOT = 'C:/old/source'; KERN_DLSS_RUNTIME_DIR = 'C:/old/runtime'
            WARP_AUDIO_IR_DIR = 'C:/old/ir'
        }
        # Unrelated manifest fields must never become launch credentials/arguments.
        GAMENIGHT_TOKEN = 'fixture-secret'; arguments = @('--smoke-render', '100')
    }
    Write-FixtureJson $referencePath $reference
    $libraryPath = Join-Path $dataRoot 'local-games.json'
    $unrelated = [ordered]@{ id = 'other-game'; title = 'Keep me'; launch = @{ command = 'other.exe'; env = @{ KEEP = 'value' } }; extra = @(1, 2, 3) }
    $initialEntries = @($unrelated, @{ id = 'sideways-kern'; title = 'Legacy'; cover = 'local-cover.png'; launch = @{ command = 'old.exe'; env = @{ GAMENIGHT_TOKEN = 'obsolete-fixture-token' } } })
    Write-FixtureJson $libraryPath $initialEntries
    $originalBytes = [Convert]::ToBase64String([IO.File]::ReadAllBytes($libraryPath))
    $options = @{ PackageDirectory = $packageRoot; ReferencePlaytestManifest = $referencePath; DataDirectory = $dataRoot }

    $preview = & $helper @options -ValidateOnly
    Assert-That ($preview.status -eq 'validated' -and $preview.changed) 'Validation must describe the pending change.'
    Assert-That ([Convert]::ToBase64String([IO.File]::ReadAllBytes($libraryPath)) -eq $originalBytes) 'ValidateOnly must not rewrite registration.'
    Assert-That (@(Get-ChildItem -LiteralPath $dataRoot).Count -eq 1) 'ValidateOnly must not create backup files.'
    Assert-That ($preview.entry.launch.env.SIDEWAYS_PROCEDURAL_GRASS -eq 'field') 'Required procedural meadow selection must survive.'
    Assert-That ($preview.entry.launch.env.SIDEWAYS_MEADOW_BLADE_BUDGET -eq '120') 'Source geometry profile must survive.'
    Assert-That ($preview.entry.launch.env.KERN_MEADOW_AGGREGATE_SAMPLE_PRUNE -eq '1') 'The complete reference profile must survive, including opt-in settings.'
    Assert-That ($preview.entry.launch.env.KERN_MEADOW_AGGREGATE_HALF_RES -eq '0') 'Explicitly disabled experiments must remain disabled.'
    Assert-That ($preview.entry.launch.env.KERN_MEADOW_CONTACT_AO -eq '0') 'Pre-r59 profiles must explicitly keep AO off instead of inheriting the daemon environment.'
    Assert-That ($preview.entry.launch.env.SIDEWAYS_GAME_ROOT -eq $packageRoot -and $preview.entry.launch.env.KERN_DLSS_RUNTIME_DIR -eq $packageRoot) 'Runtime assets and DLSS must resolve inside the package.'
    Assert-That ($preview.entry.launch.env.WARP_AUDIO_IR_DIR -eq (Join-Path $packageRoot 'engine/assets/sound/ir')) 'Audio must use packaged impulse responses.'
    Assert-That (-not $preview.entry.launch.env.Contains('GAMENIGHT_TOKEN')) 'Host credentials must never be copied.'
    Assert-That (($preview.entry.launch.args -join ' ') -eq 'play --fullscreen --size 3840x2160 --upscale dlss-performance --wind 0.6') 'Managed arguments must use 4K and leave seat count to the host.'
    $quality = & $helper @options -ValidateOnly -Upscale dlss-quality -DebugUi
    Assert-That ($quality.entry.launch.args -contains 'dlss-quality' -and $quality.entry.launch.args -contains '--debug-ui') 'Explicit quality/debug selection must be forwarded.'

    $registered = & $helper @options
    Assert-That ($registered.status -eq 'registered' -and (Test-Path -LiteralPath $registered.backup)) 'A replacement must retain a backup.'
    Assert-That ([Convert]::ToBase64String([IO.File]::ReadAllBytes($registered.backup)) -eq $originalBytes) 'Backup must retain the exact original bytes.'
    $saved = Get-Content -LiteralPath $libraryPath -Raw | ConvertFrom-Json
    $saved = @($saved)
    Assert-That ($saved.Count -eq 2 -and $saved[1].id -eq 'sideways') 'Legacy entry must normalize without duplicates.'
    Assert-That ($saved[1].cover -eq 'local-cover.png') 'Local artwork must survive normalization.'
    Assert-That ((ConvertTo-Json -InputObject $saved[0] -Depth 32 -Compress) -ceq (ConvertTo-Json -InputObject $unrelated -Depth 32 -Compress)) 'Unrelated entries must retain their complete contents.'
    $fileCount = @(Get-ChildItem -LiteralPath $dataRoot).Count
    $again = & $helper @options
    Assert-That ($again.status -eq 'unchanged' -and -not $again.changed -and $null -eq $again.backup) 'Identical registration must be idempotent.'
    Assert-That (@(Get-ChildItem -LiteralPath $dataRoot).Count -eq $fileCount) 'A no-op must not produce another backup.'

    $beforeFailures = (Get-FileHash -LiteralPath $libraryPath -Algorithm SHA256).Hash
    Write-FixtureJson $referencePath @{ status = 'visible_first_frame' }
    Assert-Rejected { & $helper @options } 'graphics_environment object'
    $reference.graphics_environment.Remove('SIDEWAYS_PROCEDURAL_GRASS')
    Write-FixtureJson $referencePath $reference
    Assert-Rejected { & $helper @options } 'SIDEWAYS_PROCEDURAL_GRASS=field'
    $reference.graphics_environment['SIDEWAYS_PROCEDURAL_GRASS'] = 'field'
    $reference.graphics_environment['GAMENIGHT_TOKEN'] = 'must-not-be-copied'
    Write-FixtureJson $referencePath $reference
    Assert-Rejected { & $helper @options } 'unsupported name'
    $reference.graphics_environment.Remove('GAMENIGHT_TOKEN')
    $reference.status = 'build_complete'
    Write-FixtureJson $referencePath $reference
    Assert-Rejected { & $helper @options } 'visible_first_frame'
    $reference.status = 'visible_first_frame'
    Write-FixtureJson $referencePath $reference
    [IO.File]::AppendAllText($executable, 'modified')
    Assert-Rejected { & $helper @options } 'SHA256'
    [IO.File]::WriteAllText($executable, 'fixture only: Sideways.exe')
    $missingAsset = Join-Path $packageRoot 'assets/meshes/fir_a_near.kmesh'
    Remove-Item -LiteralPath $missingAsset
    Assert-Rejected { & $helper @options } 'missing file assets/meshes/fir_a_near.kmesh'
    [IO.File]::WriteAllText($missingAsset, 'fixture only: assets/meshes/fir_a_near.kmesh')
    foreach ($relative in @('nvngx_dlss.dll', 'engine/assets/sound/ir/mild_exhaust.wav', 'data/render-baseline.toml')) {
        $missingAsset = Join-Path $packageRoot $relative
        Remove-Item -LiteralPath $missingAsset
        Assert-Rejected { & $helper @options } 'Incomplete package: missing file'
        [IO.File]::WriteAllText($missingAsset, "fixture only: $relative")
    }
    Assert-That ((Get-FileHash -LiteralPath $libraryPath -Algorithm SHA256).Hash -eq $beforeFailures) 'Failed validation must leave registration unchanged.'
    Assert-That (@(Get-ChildItem -LiteralPath $dataRoot).Count -eq $fileCount) 'Failed validation must not create backup/temp files.'

    # New installations keep an array even when Sideways is the only entry.
    $newDataRoot = Join-Path $fixtureRoot 'new GameNight'
    & $helper -PackageDirectory $packageRoot -ReferencePlaytestManifest $referencePath -DataDirectory $newDataRoot -ValidateOnly | Out-Null
    Assert-That (-not (Test-Path -LiteralPath $newDataRoot)) 'Validation must not create a missing data directory.'
    $fresh = & $helper -PackageDirectory $packageRoot -ReferencePlaytestManifest $referencePath -DataDirectory $newDataRoot
    Assert-That ($fresh.status -eq 'registered' -and $null -eq $fresh.backup) 'A new registration has no prior file to back up.'
    Assert-That ([IO.File]::ReadAllText($fresh.library).TrimStart().StartsWith('[')) 'A one-game library must serialize as an array.'
    # Prefer existing canonical metadata if both current and obsolete ids exist.
    Write-FixtureJson $fresh.library @($unrelated,
        @{ id = 'sideways-kern'; cover = 'obsolete-cover.png' },
        @{ id = 'sideways'; cover = 'canonical-cover.png' })
    & $helper -PackageDirectory $packageRoot -ReferencePlaytestManifest $referencePath -DataDirectory $newDataRoot | Out-Null
    $normalized = Get-Content -LiteralPath $fresh.library -Raw | ConvertFrom-Json
    Assert-That ($normalized.Count -eq 2 -and $normalized[1].id -eq 'sideways' -and $normalized[1].cover -eq 'canonical-cover.png') 'Duplicate obsolete entries must collapse while preserving canonical artwork.'
    [IO.File]::WriteAllText($fresh.library, '[null]')
    Assert-Rejected { & $helper -PackageDirectory $packageRoot -ReferencePlaytestManifest $referencePath -DataDirectory $newDataRoot } 'null entry|entry without an id'
    Assert-That ([IO.File]::ReadAllText($fresh.library) -eq '[null]') 'Malformed libraries must not be silently rewritten.'
    Write-Host 'PASS: preview-profile retention, packaged asset/hash checks, credential exclusion, validation-only, preservation, backup, and idempotent registration. No process launched.'
} finally {
    # Never recursively remove a computed path until its exact parent/name is checked.
    $resolvedFixture = [IO.Path]::GetFullPath($fixtureRoot)
    if ([IO.Path]::GetDirectoryName($resolvedFixture) -ne $tempRoot -or
        [IO.Path]::GetFileName($resolvedFixture) -notmatch '^sideways-register-[0-9a-f]{32}$') {
        throw 'Refusing cleanup outside the uniquely named registration fixture.'
    }
    if (Test-Path -LiteralPath $resolvedFixture) { Remove-Item -LiteralPath $resolvedFixture -Recurse -Force }
}
