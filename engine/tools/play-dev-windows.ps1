[CmdletBinding()]
param(
    [switch]$BuildOnly,
    [switch]$Restart,
    [ValidateSet('release', 'dev')][string]$Profile = 'release',
    [string]$TargetDirectory,
    [ValidateRange(1, 4)][int]$Views = 1,
    [ValidatePattern('^\d+x\d+$')][string]$Size = '3840x2160',
    [ValidateRange(2, 8)][double]$Km = 3,
    [UInt64]$Seed = 1,
    [ValidateSet('clear', 'overcast', 'mist', 'wet')][string]$Weather = 'clear',
    [ValidateSet('spring', 'summer', 'autumn', 'winter')][string]$Season = 'summer',
    [ValidateRange(0, 1)][double]$Growth = 1,
    [ValidateRange(0, 1)][double]$Wind = 0.3,
    [Nullable[double]]$WindTime,
    [Nullable[UInt64]]$VegetationSeed,
    [switch]$Autoplay,
    [switch]$Windowed,
    [ValidateRange(10, 600)][int]$StartupTimeoutSeconds = 180
)

# Explicit invocation only. A build failure never stops a playtest.
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'play-dev-support.ps1')
$engineRoot = [IO.Path]::GetFullPath((Split-Path -Parent $PSScriptRoot))
$runRoot = Join-Path $engineRoot 'build\dev-runs'
$activePath = Join-Path $runRoot 'active.json'
$guard = $null
$manifest = $null
$manifestPath = $null

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
        throw 'This helper is for standalone play. Launch managed sessions through GameNight; no host credentials are recorded.'
    }
    $cargo = (Get-Command cargo -CommandType Application -ErrorAction Stop).Source
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
    $arguments = @('play', '--views', "$Views", '--size', $Size, '--seed', "$Seed", '--km', $Km.ToString([Globalization.CultureInfo]::InvariantCulture), '--weather', $Weather,
        '--season', $Season, '--growth', $Growth.ToString([Globalization.CultureInfo]::InvariantCulture),
        '--wind', $Wind.ToString([Globalization.CultureInfo]::InvariantCulture),
        '--state-out', (Join-Path $directory 'final-state.json'))
    if ($null -ne $WindTime) { $arguments += @('--wind-time', $WindTime.ToString([Globalization.CultureInfo]::InvariantCulture)) }
    if ($null -ne $VegetationSeed) { $arguments += @('--vegetation-seed', "$VegetationSeed") }
    if (-not $Windowed) { $arguments += '--fullscreen' }
    if ($Autoplay) { $arguments += '--autoplay' }
    $sourceRevision = $null
    $sourceDirty = $null
    if (Get-Command git -CommandType Application -ErrorAction SilentlyContinue) {
        $revision = & git -C $engineRoot rev-parse HEAD 2>$null
        if ($LASTEXITCODE -eq 0) { $sourceRevision = [string]$revision }
        $dirty = & git -C $engineRoot status --porcelain 2>$null
        if ($LASTEXITCODE -eq 0) { $sourceDirty = [bool]$dirty }
    }
    $graphics = @{}
    foreach ($key in @('KERN_ADAPTER', 'KERN_CLUSTER_BUCKETS', 'KERN_EARLY_GRASS_CULL', 'SIDEWAYS_LIVE_GRASS', 'SIDEWAYS_FAR_GRASS', 'SIDEWAYS_VOXEL_TREE_PATCH', 'SIDEWAYS_PROCEDURAL_ROCKS', 'SIDEWAYS_GENERIC_VOXEL_TREES')) {
        $value = [Environment]::GetEnvironmentVariable($key)
        if ($null -ne $value) { $graphics[$key] = $value }
    }
    $manifest = [ordered]@{ status = 'building'; started_utc = [DateTime]::UtcNow.ToString('o'); engine_root = $engineRoot;
        profile = $Profile; source_revision = $sourceRevision; source_dirty = $sourceDirty;
        source_note = 'Working checkout build, not an immutable source snapshot.';
        arguments = $arguments; graphics_environment = $graphics; executable = $stagedExecutable;
        build_stdout = $buildOut; build_stderr = $buildError; play_stdout = $playOut; play_stderr = $playError }
    Write-DevJson $manifestPath $manifest
    $buildArguments = @('build', '--locked', '-p', 'cli', '--bin', 'sideways', '--profile', $Profile, '--message-format=json-render-diagnostics')
    if ($TargetDirectory) { $buildArguments += @('--target-dir', $TargetDirectory) }
    Write-Host "Building $Profile; existing playtest stays running. Logs: $directory"
    $clock = [Diagnostics.Stopwatch]::StartNew()
    $build = Start-Process -FilePath $cargo -ArgumentList (($buildArguments | ForEach-Object { ConvertTo-NativeArgument $_ }) -join ' ') -WorkingDirectory $engineRoot -WindowStyle Hidden -RedirectStandardOutput $buildOut -RedirectStandardError $buildError -PassThru
    $build.WaitForExit()
    $manifest.build_seconds = $clock.Elapsed.TotalSeconds
    $manifest.build_exit_code = $build.ExitCode
    if ($build.ExitCode -ne 0) {
        Get-Content -LiteralPath $buildError -Tail 25 | Write-Host
        throw "Cargo failed with exit code $($build.ExitCode). Existing playtest was not stopped. See $buildError"
    }
    $builtExecutable = Get-SidewaysBuildArtifact @(Get-Content -LiteralPath $buildOut)
    if (-not (Test-Path -LiteralPath $builtExecutable -PathType Leaf)) { throw "Cargo artifact is missing: $builtExecutable" }
    Copy-Item -LiteralPath $builtExecutable -Destination $stagedExecutable
    $manifest.sha256 = (Get-FileHash -LiteralPath $stagedExecutable -Algorithm SHA256).Hash.ToLowerInvariant()
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
    $game = Start-Process -FilePath $stagedExecutable -ArgumentList (($arguments | ForEach-Object { ConvertTo-NativeArgument $_ }) -join ' ') -WorkingDirectory $engineRoot -WindowStyle Hidden -RedirectStandardOutput $playOut -RedirectStandardError $playError -PassThru
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
            Write-DevJson $manifestPath $manifest
            Write-Host "Playtest ready: PID $($game.Id), first visible frame confirmed. F12 saves PNG + metadata and copies the image."
            Write-Host "Evidence: $manifestPath"
            return
        }
        Start-Sleep -Milliseconds 250
    }
    $manifest.status = 'startup_unconfirmed'
    throw "No visible first-frame proof after $StartupTimeoutSeconds seconds. PID $($game.Id) is still running; inspect $playOut and $playError. No success claimed and no timeout kill performed."
} catch {
    if ($manifest) {
        if ($manifest.status -ne 'startup_unconfirmed') { $manifest.status = 'failed' }
        $manifest.error = $_.Exception.Message
        Write-DevJson $manifestPath $manifest
    }
    Write-Error $_
    exit 1
} finally {
    if ($guard) { $guard.Dispose() }
}
