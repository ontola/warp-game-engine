[CmdletBinding()]
param(
    [switch]$BuildOnly,
    [switch]$Restart,
    [ValidateSet('release', 'dev')][string]$Profile = 'release',
    [string]$TargetDirectory,
    # Launch a validated artifact while source work continues; never infer it
    # from the current checkout or silently reuse an earlier failed build.
    [string]$PreparedExecutable,
    [ValidatePattern('^[0-9a-fA-F]{64}$')][string]$PreparedSha256,
    [ValidateRange(1, 4)][int]$Views = 1,
    [ValidatePattern('^\d+x\d+$')][string]$Size = '3840x2160',
    # Four-player developer sessions resolve to DLSS Performance below unless
    # explicitly overridden. Other view counts retain this native fallback.
    [ValidateSet('native', 'dlss-quality', 'dlss-balanced', 'dlss-performance')][string]$Upscale = 'native',
    [ValidateRange(2, 8)][double]$Km = 3,
    [UInt64]$Seed = 1,
    [ValidateSet('clear', 'overcast', 'mist', 'wet')][string]$Weather = 'clear',
    [ValidateSet('spring', 'summer', 'autumn', 'winter')][string]$Season = 'summer',
    [ValidateRange(0, 1)][double]$Growth = 1,
    [ValidateRange(0, 1)][double]$Wind = 0.3,
    [ValidateRange(1.0, 1.35)][double]$TreeFullness = 1.25,
    [switch]$TreeOcclusion = $true,
    [switch]$MeadowAggregate = $true,
    [Nullable[double]]$WindTime,
    [Nullable[UInt64]]$VegetationSeed,
    [switch]$Autoplay,
    # Explicit developer opt-in to the measured r47/r48 paths; not a quality preset.
    [switch]$WorkReduction,
    [switch]$FrameProfile,
    [switch]$DebugUi,
    [string]$DebugUiScript,
    [switch]$Windowed,
    [ValidateRange(10, 600)][int]$StartupTimeoutSeconds = 180
)

# Explicit invocation only. A build failure never stops a playtest.
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'play-dev-support.ps1')
$gameRoot = [IO.Path]::GetFullPath((Split-Path -Parent $PSScriptRoot))
$workspaceRoot = [IO.Path]::GetFullPath((Join-Path $gameRoot '../..'))
$runRoot = Join-Path $gameRoot 'build\dev-runs'
$activePath = Join-Path $runRoot 'active.json'
$guard = $null
$manifest = $null
$manifestPath = $null
$startupProfileBefore = [Environment]::GetEnvironmentVariable('KERN_STARTUP_PROFILE')
$treeFullnessBefore = [Environment]::GetEnvironmentVariable('SIDEWAYS_TREE_FULLNESS')
$treeOcclusionBefore = [Environment]::GetEnvironmentVariable('KERN_TREE_OCCLUSION')
$meadowAggregateBefore = [Environment]::GetEnvironmentVariable('KERN_MEADOW_AGGREGATE')
$workReductionKeys = @('KERN_CLUSTER_BUCKETS', 'KERN_MEADOW_INDEXED', 'KERN_MEADOW_VISIBILITY', 'KERN_MEADOW_SKIP_HIDDEN_SHADE', 'KERN_HEADLIGHT_INSTANCE_GROUPS')
$workReductionBefore = @{}
foreach ($key in $workReductionKeys) { $workReductionBefore[$key] = [Environment]::GetEnvironmentVariable($key) }

function Resolve-DevTreeSettings {
    param(
        [double]$ParameterFullness, [bool]$ParameterOcclusion,
        [bool]$FullnessBound, [bool]$OcclusionBound,
        [AllowNull()][object]$EnvironmentFullness, [AllowNull()][object]$EnvironmentOcclusion
    )
    # Explicit parameters override inherited process values. An unset value
    # uses the tested launcher default; invalid inherited values fail early.
    $fullness = $ParameterFullness
    $fullnessSource = if ($FullnessBound) { 'parameter' } else { 'launcher_default' }
    if (-not $FullnessBound -and $null -ne $EnvironmentFullness) {
        $fullnessSource = 'environment'
        if (-not [double]::TryParse([string]$EnvironmentFullness, [Globalization.NumberStyles]::Float,
                [Globalization.CultureInfo]::InvariantCulture, [ref]$fullness)) {
            throw 'SIDEWAYS_TREE_FULLNESS must be a finite number in 1..1.35.'
        }
    }
    if ([double]::IsNaN($fullness) -or [double]::IsInfinity($fullness) -or $fullness -lt 1 -or $fullness -gt 1.35) {
        throw 'TreeFullness / SIDEWAYS_TREE_FULLNESS must be a finite number in 1..1.35.'
    }
    $occlusion = $ParameterOcclusion
    $occlusionSource = if ($OcclusionBound) { 'parameter' } else { 'launcher_default' }
    if (-not $OcclusionBound -and $null -ne $EnvironmentOcclusion) {
        $occlusionSource = 'environment'
        if ([string]$EnvironmentOcclusion -notin @('0', '1')) {
            throw 'KERN_TREE_OCCLUSION must be 0 or 1.'
        }
        $occlusion = [string]$EnvironmentOcclusion -eq '1'
    }
    return [pscustomobject]@{
        fullness = $fullness; occlusion_requested = $occlusion
        fullness_source = $fullnessSource; occlusion_source = $occlusionSource
        precedence = 'Explicit parameter > inherited process environment > launcher default.'
        scope = 'Resolved launch request. Actual constructed fullness and enabled renderer culling are reported by game capture metadata; culling requires cluster buckets.'
    }
}

function Save-DevStartupEvidence {
    if (-not $manifest -or -not (Test-Path -LiteralPath $playError -PathType Leaf)) { return }
    $records = @(Get-DevStartupProfiles @(Get-Content -LiteralPath $playError))
    $manifest.startup_profiles = $records
    $manifest.startup_profile_file = Join-Path $directory 'startup.json'
    $manifest.startup_profile_scope = 'Structured CLI/renderer CPU timing available at first-visible proof or launch failure. GPU completion and later rebuilds are separate; full stderr remains available.'
    Write-DevJson $manifest.startup_profile_file ([ordered]@{ records = $records; scope = $manifest.startup_profile_scope })
}

function Get-OwnedDevProcess {
    if (-not (Test-Path -LiteralPath $activePath)) { return $null }
    $record = Get-Content -LiteralPath $activePath -Raw | ConvertFrom-Json
    $candidate = Get-Process -Id ([int]$record.pid) -ErrorAction SilentlyContinue
    if (-not $candidate) { return $null }
    if (-not (Test-DevProcessIdentity $record $candidate.Id $candidate.Path $candidate.StartTime.ToUniversalTime().Ticks $runRoot)) {
        throw "Saved playtest identity does not match PID $($record.pid). No process was stopped; inspect $activePath."
    }
    return $candidate
}

try {
    if ($BuildOnly -and $Restart) { throw '-BuildOnly and -Restart cannot be combined.' }
    if ([bool]$PreparedExecutable -ne [bool]$PreparedSha256) { throw 'PreparedExecutable and PreparedSha256 must be supplied together.' }
    if ($PreparedExecutable -and $BuildOnly) { throw 'BuildOnly cannot be combined with a prepared executable.' }
    $preparedArtifact = if ($PreparedExecutable) { Get-PreparedDevArtifact $PreparedExecutable $PreparedSha256 } else { $null }
    $debugScriptBytes = $null
    if ($DebugUiScript) {
        $debugScriptPath = (Resolve-Path -LiteralPath $DebugUiScript -ErrorAction Stop).Path
        if ((Get-Item -LiteralPath $debugScriptPath).Length -gt 65536) { throw 'DebugUiScript exceeds 64 KiB.' }
        $debugScriptBytes = [IO.File]::ReadAllBytes($debugScriptPath)
        $debugScriptText = [Text.Encoding]::UTF8.GetString($debugScriptBytes)
        $debugScriptRows = @($debugScriptText | ConvertFrom-Json -ErrorAction Stop)
        if (-not $debugScriptText.TrimStart().StartsWith('[') -or $debugScriptRows.Count -gt 256) { throw 'DebugUiScript must be a JSON array of at most 256 events.' }
    }
    $dimensions = $Size.Split('x') | ForEach-Object { [int]$_ }
    if ($dimensions[0] -lt 64 -or $dimensions[0] -gt 7680 -or $dimensions[1] -lt 64 -or $dimensions[1] -gt 4320) {
        throw 'Size must be within 64x64..7680x4320.'
    }
    if ([double]::IsNaN($Km) -or [double]::IsInfinity($Km)) { throw 'Km must be finite.' }
    foreach ($value in @($Growth, $Wind)) {
        if ([double]::IsNaN($value) -or [double]::IsInfinity($value)) { throw 'Growth and Wind must be finite.' }
    }
    if ($null -ne $WindTime -and ([double]::IsNaN($WindTime) -or [double]::IsInfinity($WindTime) -or $WindTime -lt 0)) {
        throw 'WindTime must be finite and nonnegative.'
    }
    if (-not $BuildOnly -and (@('GAMENIGHT', 'GAMENIGHT_ADDR', 'GAMENIGHT_GAME_ID', 'GAMENIGHT_TOKEN') | Where-Object { [Environment]::GetEnvironmentVariable($_) })) {
        throw 'This helper is for standalone play. Launch managed sessions through Splitscreen Club; no host credentials are recorded.'
    }
    $treeSettings = Resolve-DevTreeSettings -ParameterFullness $TreeFullness -ParameterOcclusion ([bool]$TreeOcclusion) `
        -FullnessBound $PSBoundParameters.ContainsKey('TreeFullness') -OcclusionBound $PSBoundParameters.ContainsKey('TreeOcclusion') `
        -EnvironmentFullness $treeFullnessBefore -EnvironmentOcclusion $treeOcclusionBefore
    $env:SIDEWAYS_TREE_FULLNESS = $treeSettings.fullness.ToString('R', [Globalization.CultureInfo]::InvariantCulture)
    $env:KERN_TREE_OCCLUSION = if ($treeSettings.occlusion_requested) { '1' } else { '0' }
    $meadowAggregateSource = if ($PSBoundParameters.ContainsKey('MeadowAggregate')) { 'parameter' } else { 'launcher_default' }
    $meadowAggregateRequested = [bool]$MeadowAggregate
    if (-not $PSBoundParameters.ContainsKey('MeadowAggregate') -and $null -ne $meadowAggregateBefore) {
        if ($meadowAggregateBefore -notin @('0', '1')) { throw 'KERN_MEADOW_AGGREGATE must be 0 or 1.' }
        $meadowAggregateRequested = $meadowAggregateBefore -eq '1'
        $meadowAggregateSource = 'environment'
    }
    $env:KERN_MEADOW_AGGREGATE = if ($meadowAggregateRequested) { '1' } else { '0' }
    if ($WorkReduction) {
        foreach ($key in $workReductionKeys) { [Environment]::SetEnvironmentVariable($key, '1', 'Process') }
        Write-Host 'Developer work reduction enabled: single grass geometry pass, hidden-ground shading skip, grouped headlight casters.'
    }
    $upscaleSource = if ($PSBoundParameters.ContainsKey('Upscale')) { 'parameter' } else { 'launcher_default' }
    if ($Views -eq 4 -and -not $PSBoundParameters.ContainsKey('Upscale')) {
        $Upscale = 'dlss-performance'
        $upscaleSource = 'four_player_default'
    }
    $upscaleBuild = Get-DlssBuildSetup -Upscale $Upscale
    Write-Host "Render mode: $Upscale ($upscaleSource), $Views views, $Size output."
    $env:KERN_STARTUP_PROFILE = '1'
    $cargo = if (-not $preparedArtifact) { (Get-Command cargo -CommandType Application -ErrorAction Stop).Source } else { $null }
    New-Item -ItemType Directory -Path $runRoot -Force | Out-Null
    try { $guard = [IO.File]::Open((Join-Path $runRoot 'iteration.lock'), 'OpenOrCreate', 'ReadWrite', 'None') }
    catch { throw 'Another developer iteration is still building or starting in this checkout.' }
    $previous = Get-OwnedDevProcess
    if ($previous -and -not $BuildOnly -and -not $Restart) {
        throw "Developer playtest PID $($previous.Id) is running. Use -Restart to replace it after a successful build, or -BuildOnly."
    }
    $runId = [DateTime]::UtcNow.ToString('yyyyMMddTHHmmssfffZ') + '-' + [Guid]::NewGuid().ToString('N').Substring(0, 8)
    $directory = Join-Path $runRoot $runId
    New-Item -ItemType Directory -Path $directory | Out-Null
    $manifestPath = Join-Path $directory 'iteration.json'
    $stagedExecutable = Join-Path $directory 'sideways.exe'
    $buildOut = Join-Path $directory 'build.jsonl'
    $buildError = Join-Path $directory 'build.stderr.log'
    $playOut = Join-Path $directory 'play.stdout.log'
    $playError = Join-Path $directory 'play.stderr.log'
    $arguments = @('play', '--views', "$Views", '--size', $Size, '--upscale', $Upscale, '--seed', "$Seed", '--km', $Km.ToString([Globalization.CultureInfo]::InvariantCulture), '--weather', $Weather,
        '--season', $Season, '--growth', $Growth.ToString([Globalization.CultureInfo]::InvariantCulture),
        '--wind', $Wind.ToString([Globalization.CultureInfo]::InvariantCulture),
        '--state-out', (Join-Path $directory 'final-state.json'))
    if ($null -ne $WindTime) { $arguments += @('--wind-time', $WindTime.ToString([Globalization.CultureInfo]::InvariantCulture)) }
    if ($null -ne $VegetationSeed) { $arguments += @('--vegetation-seed', "$VegetationSeed") }
    if (-not $Windowed) { $arguments += '--fullscreen' }
    if ($Autoplay) { $arguments += '--autoplay' }
    if ($FrameProfile) { $arguments += @('--frame-profile', (Join-Path $directory 'frame-profile.jsonl')) }
    if ($DebugUi) { $arguments += '--debug-ui' }
    $debugScriptRecord = $null
    if ($null -ne $debugScriptBytes) {
        $stagedDebugScript = Join-Path $directory 'debug-ui-script.json'
        [IO.File]::WriteAllBytes($stagedDebugScript, $debugScriptBytes)
        $arguments += @('--debug-ui-script', $stagedDebugScript)
        $debugScriptRecord = @{ path = $stagedDebugScript; sha256 = (Get-FileHash -LiteralPath $stagedDebugScript -Algorithm SHA256).Hash.ToLowerInvariant(); events = $debugScriptRows.Count }
    }
    $sourceRevision = $null
    $sourceDirty = $null
    if (Get-Command git -CommandType Application -ErrorAction SilentlyContinue) {
        $revision = & git -C $workspaceRoot rev-parse HEAD 2>$null
        if ($LASTEXITCODE -eq 0) { $sourceRevision = [string]$revision }
        $dirty = & git -C $workspaceRoot status --porcelain 2>$null
        if ($LASTEXITCODE -eq 0) { $sourceDirty = [bool]$dirty }
    }
    $graphics = @{}
    $graphics['KERN_STARTUP_PROFILE'] = '1'
    $graphics['SIDEWAYS_TREE_FULLNESS'] = $env:SIDEWAYS_TREE_FULLNESS
    $graphics['KERN_TREE_OCCLUSION'] = $env:KERN_TREE_OCCLUSION
    $graphics['KERN_MEADOW_AGGREGATE'] = $env:KERN_MEADOW_AGGREGATE
    if ($null -ne [Environment]::GetEnvironmentVariable('KERN_MEADOW_AGGREGATE_HALF_RES')) {
        $graphics['KERN_MEADOW_AGGREGATE_HALF_RES'] = $env:KERN_MEADOW_AGGREGATE_HALF_RES
    }
    if ($null -ne [Environment]::GetEnvironmentVariable('KERN_MEADOW_AGGREGATE_SAMPLE_PRUNE')) {
        $graphics['KERN_MEADOW_AGGREGATE_SAMPLE_PRUNE'] = $env:KERN_MEADOW_AGGREGATE_SAMPLE_PRUNE
    }
    if ($null -ne [Environment]::GetEnvironmentVariable('KERN_MEADOW_CONTACT_AO')) {
        $graphics['KERN_MEADOW_CONTACT_AO'] = $env:KERN_MEADOW_CONTACT_AO
    }
    foreach ($key in @('KERN_MEADOW_SUBSTRATE', 'KERN_MEADOW_INDEXED', 'KERN_MEADOW_VISIBILITY', 'KERN_MEADOW_SKIP_HIDDEN_SHADE', 'KERN_MEADOW_SPATIAL_FILTER', 'KERN_MEADOW_EDGE_SHADOW', 'KERN_MEADOW_OCCLUSION', 'KERN_MEADOW_GROUP_CACHE', 'KERN_MEADOW_GROUP_REPLACE', 'SIDEWAYS_MEADOW_SOURCE_RADIUS', 'SIDEWAYS_MEADOW_SOURCE_BAND', 'SIDEWAYS_MEADOW_BLADE_BUDGET', 'SIDEWAYS_MEADOW_BLADE_SHADING', 'SIDEWAYS_MEADOW_PROFILE', 'SIDEWAYS_MEADOW_SPECIES', 'SIDEWAYS_TREE_SPECIES', 'SIDEWAYS_MEADOW_WINDSWEPT_RADIUS', 'SIDEWAYS_RENDER_BASELINE', 'KERN_REUSE_SUN_TEXTURE', 'KERN_ZERO_SUN_FAST_PATH', 'KERN_INSTANCE_GROUPS', 'KERN_SUN_INSTANCE_GROUPS', 'KERN_HEADLIGHT_INSTANCE_GROUPS', 'KERN_CLUSTER_TRAVERSAL', 'KERN_STATIC_SUN_CACHE', 'KERN_SUN_CACHE_GUARD_METRES')) {
        $value = [Environment]::GetEnvironmentVariable($key)
        if ($null -ne $value) { $graphics[$key] = $value }
    }
    foreach ($key in @('SIDEWAYS_GAME_ROOT', 'WARP_AUDIO_IR_DIR', 'KERN_ADAPTER', 'KERN_CLUSTER_BUCKETS', 'KERN_EARLY_GRASS_CULL', 'KERN_CUTOUT_LINEAR_RGB', 'KERN_GRASS_LEAF_BRDF', 'KERN_GRASS_SUN_OCCLUSION', 'KERN_SOURCE_CROWN_BOUNDS', 'KERN_CANOPY_COMPACT_SUPPORT', 'KERN_LEGACY_LIGHT_LOOKAHEAD', 'KERN_SUN_RECEIVER_PLANE', 'KERN_SUN_SCENE_DEPTH', 'KERN_CAMERA_SUN_OCCLUSION', 'KERN_FAR_SUN_SHADOW', 'KERN_FAR_SUN_LOD', 'KERN_WORLD_TERRAIN_VISIBILITY', 'KERN_TERRAIN_NODE_BUDGET', 'KERN_SUN_TERRAIN_ERROR', 'KERN_SUN_SURFACE_SOURCE', 'KERN_ROCK_VARIATION', 'SIDEWAYS_ADAPTIVE_CLIFF', 'SIDEWAYS_LIVE_GRASS', 'SIDEWAYS_FAR_GRASS', 'SIDEWAYS_GRASS_DETAIL', 'SIDEWAYS_VOXEL_TREE_PATCH', 'SIDEWAYS_PROCEDURAL_GRASS', 'SIDEWAYS_PROCEDURAL_GRASS_CELL', 'SIDEWAYS_PROCEDURAL_ROCKS', 'SIDEWAYS_CROWDS', 'SIDEWAYS_GENERIC_VOXEL_TREES', 'SIDEWAYS_SKY', 'SIDEWAYS_SUN_ELEVATION', 'SIDEWAYS_SUN_AZIMUTH')) {
        $value = [Environment]::GetEnvironmentVariable($key)
        if ($null -ne $value) { $graphics[$key] = $value }
    }
    foreach ($key in @('KERN_PAGED_FOLIAGE_SUN', 'KERN_FOLIAGE_SUN_PAGE_METRES', 'KERN_FOLIAGE_SUN_LOD', 'KERN_TEXTURE_CACHE', 'KERN_TEXTURE_CACHE_DIR', 'KERN_PIPELINE_CACHE', 'KERN_PIPELINE_CACHE_DIR', 'KERN_GRASS_DEPTH_PREPASS', 'KERN_INSTANCE_BROADPHASE', 'KERN_RASTER_LARGE_FIRST')) {
        $value = [Environment]::GetEnvironmentVariable($key)
        if ($null -ne $value) { $graphics[$key] = $value }
    }
    $manifest = [ordered]@{ status = 'building'; started_utc = [DateTime]::UtcNow.ToString('o'); engine_root = (Join-Path $workspaceRoot 'engine'); game_root = $gameRoot; workspace_root = $workspaceRoot;
        profile = $Profile; source_revision = $sourceRevision; source_dirty = $sourceDirty;
        cargo_features = $upscaleBuild.features; dlss = $upscaleBuild.dlss; work_reduction_requested = [bool]$WorkReduction;
        upscale_requested = $Upscale; upscale_source = $upscaleSource;
        tree_settings = $treeSettings;
        debug_ui_script = $debugScriptRecord;
        meadow_settings = @{ aggregate_requested = $meadowAggregateRequested; source = $meadowAggregateSource; scope = 'Launch request; actual constructed representation and counts are in capture metadata.' };
        source_note = 'Working checkout build, not an immutable source snapshot.';
        arguments = $arguments; graphics_environment = $graphics; executable = $stagedExecutable;
        build_stdout = $buildOut; build_stderr = $buildError; play_stdout = $playOut; play_stderr = $playError }
    Write-DevJson $manifestPath $manifest
    $clock = [Diagnostics.Stopwatch]::StartNew()
    if ($preparedArtifact) {
        $builtExecutable = $preparedArtifact.path
        $manifest.prepared_artifact = $preparedArtifact
        $manifest.build_seconds = 0
        $manifest.build_exit_code = $null
        $manifest.source_note = 'Explicit SHA-pinned prebuilt artifact; current checkout revision is launch context, not its source provenance.'
        $manifest.cargo_features = $null
        Write-Host "Staging verified prebuilt artifact; no compilation. Logs: $directory"
    } else {
    $buildArguments = @('build', '--locked', '-p', 'cli', '--bin', 'sideways', '--profile', $Profile, '--message-format=json-render-diagnostics')
    if ($upscaleBuild.features.Count -gt 0) { $buildArguments += @('--features', ($upscaleBuild.features -join ',')) }
    if ($TargetDirectory) { $buildArguments += @('--target-dir', $TargetDirectory) }
    Write-Host "Building $Profile; existing playtest stays running. Logs: $directory"
    $build = Start-Process -FilePath $cargo -ArgumentList (($buildArguments | ForEach-Object { ConvertTo-NativeArgument $_ }) -join ' ') -WorkingDirectory $workspaceRoot -WindowStyle Hidden -RedirectStandardOutput $buildOut -RedirectStandardError $buildError -PassThru
    $build.WaitForExit()
    $manifest.build_seconds = $clock.Elapsed.TotalSeconds
    $manifest.build_exit_code = $build.ExitCode
    if ($build.ExitCode -ne 0) {
        Get-Content -LiteralPath $buildError -Tail 25 | Write-Host
        throw "Cargo failed with exit code $($build.ExitCode). Existing playtest was not stopped. See $buildError"
    }
    $builtExecutable = Get-SidewaysBuildArtifact @(Get-Content -LiteralPath $buildOut)
    }
    if (-not (Test-Path -LiteralPath $builtExecutable -PathType Leaf)) { throw "Cargo artifact is missing: $builtExecutable" }
    Copy-Item -LiteralPath $builtExecutable -Destination $stagedExecutable
    $manifest.sha256 = (Get-FileHash -LiteralPath $stagedExecutable -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($preparedArtifact -and $manifest.sha256 -ne $preparedArtifact.sha256) { throw 'Prepared artifact changed while staging; no process was replaced.' }
    $manifest.status = 'built'
    Write-DevJson $manifestPath $manifest
    if ($BuildOnly) {
        Write-Host "Built and staged; no game launched: $stagedExecutable"
        return
    }
    # Revalidate after compilation. Never trust a PID saved before a long build.
    $previous = Get-OwnedDevProcess
    if ($previous) {
        if (-not $Restart) { throw 'A tracked playtest appeared during the build; use -Restart explicitly.' }
        Write-Host "Replacing owned playtest PID $($previous.Id)."
        if (-not $previous.CloseMainWindow() -or -not $previous.WaitForExit(5000)) {
            $previous = Get-OwnedDevProcess
            if ($previous) {
                Stop-Process -InputObject $previous -ErrorAction Stop
                if (-not $previous.WaitForExit(10000)) { throw 'The owned playtest did not exit after termination; no replacement launched.' }
            }
        }
    }
    # Hide only the console helper; winit explicitly shows the requested game.
    $game = Start-Process -FilePath $stagedExecutable -ArgumentList (($arguments | ForEach-Object { ConvertTo-NativeArgument $_ }) -join ' ') -WorkingDirectory $gameRoot -WindowStyle Hidden -RedirectStandardOutput $playOut -RedirectStandardError $playError -PassThru
    $manifest.pid = $game.Id
    $manifest.start_utc_ticks = $game.StartTime.ToUniversalTime().Ticks
    $manifest.status = 'starting'
    Write-DevJson $manifestPath $manifest
    Write-DevJson $activePath ([ordered]@{ pid = $game.Id; executable = $stagedExecutable; start_utc_ticks = $manifest.start_utc_ticks; manifest = $manifestPath })
    $clock.Restart()
    while ($clock.Elapsed.TotalSeconds -lt $StartupTimeoutSeconds) {
        $game.Refresh()
        if ($game.HasExited) {
            Get-Content -LiteralPath $playError -Tail 25 | Write-Host
            throw "Playtest exited with code $($game.ExitCode) before a visible first frame. Logs: $directory"
        }
        $output = Get-Content -LiteralPath $playOut -Raw
        if (Test-VisibleFirstFrame $output) {
            $manifest.status = 'visible_first_frame'
            $manifest.startup_seconds = $clock.Elapsed.TotalSeconds
            Save-DevStartupEvidence
            Write-DevJson $manifestPath $manifest
            Write-Host "Playtest ready: PID $($game.Id), first visible frame confirmed. F12 saves PNG + metadata and copies the image."
            Write-Host "Evidence: $manifestPath"
            return
        }
        Start-Sleep -Milliseconds 250
    }
    $manifest.status = 'startup_unconfirmed'
    Save-DevStartupEvidence
    throw "No visible first-frame proof after $StartupTimeoutSeconds seconds. PID $($game.Id) is still running; inspect $playOut and $playError. No success claimed and no timeout kill performed."
} catch {
    if ($manifest) {
        Save-DevStartupEvidence
        if ($manifest.status -ne 'startup_unconfirmed') { $manifest.status = 'failed' }
        $manifest.error = $_.Exception.Message
        Write-DevJson $manifestPath $manifest
    }
    Write-Error $_
    exit 1
} finally {
    [Environment]::SetEnvironmentVariable('KERN_STARTUP_PROFILE', $startupProfileBefore)
    [Environment]::SetEnvironmentVariable('SIDEWAYS_TREE_FULLNESS', $treeFullnessBefore)
    [Environment]::SetEnvironmentVariable('KERN_TREE_OCCLUSION', $treeOcclusionBefore)
    [Environment]::SetEnvironmentVariable('KERN_MEADOW_AGGREGATE', $meadowAggregateBefore)
    foreach ($key in $workReductionKeys) { [Environment]::SetEnvironmentVariable($key, $workReductionBefore[$key], 'Process') }
    if ($guard) { $guard.Dispose() }
}
