[CmdletBinding()]
param(
    [ValidateSet('bullets','explosions')][string[]]$Scenarios = @('bullets','explosions'),
    [ValidateRange(1,20)][int]$Repeats = 3,
    [ValidateRange(15,120)][int]$TimeoutSeconds = 120,
    [string]$GameExe = 'C:\dev\growing-guns\MoreRounds.exe',
    [string]$OutputRoot = '',
    [int]$Seed = 721077,
    [string]$WindowPosition = '-4000,0',
    [switch]$CaptureOnly
)
$ErrorActionPreference = 'Stop'
$probe = Join-Path $PSScriptRoot 'stress-probe.gd'
$gameDir = Split-Path -Parent $GameExe
$overridePath = Join-Path $gameDir 'override.cfg'
$pck = Join-Path $gameDir 'MoreRounds.pck'
foreach ($required in @($GameExe, $pck, $probe)) {
    if (-not (Test-Path -LiteralPath $required -PathType Leaf)) { throw "Missing required file: $required" }
}
if ($WindowPosition -notmatch '^-?\d+,-?\d+$') { throw 'WindowPosition must be two comma-separated integers.' }
if (Test-Path -LiteralPath $overridePath) { throw "Existing override.cfg: refusing to overwrite $overridePath" }
$resolvedExe = (Resolve-Path -LiteralPath $GameExe).Path
$alreadyRunning = @(Get-CimInstance Win32_Process | Where-Object { $_.ExecutablePath -and $_.ExecutablePath -eq $resolvedExe })
if ($alreadyRunning.Count -gt 0) { throw 'This exact game executable is already running. No process was changed.' }
if (-not $OutputRoot) { $OutputRoot = Join-Path 'C:\dev\warp-bench' ('godot-stress-' + (Get-Date -Format 'yyyyMMdd-HHmmss')) }
if (Test-Path -LiteralPath $OutputRoot) { throw "Output directory already exists: $OutputRoot" }
$null = New-Item -ItemType Directory -Path $OutputRoot
$OutputRoot = (Resolve-Path -LiteralPath $OutputRoot).Path
$utf8 = New-Object System.Text.UTF8Encoding($false)
$manifest = [ordered]@{
    created_utc = [DateTime]::UtcNow.ToString('o'); game_executable = $resolvedExe
    executable_sha256 = (Get-FileHash -LiteralPath $GameExe -Algorithm SHA256).Hash
    pck_sha256 = (Get-FileHash -LiteralPath $pck -Algorithm SHA256).Hash
    probe_sha256 = (Get-FileHash -LiteralPath $probe -Algorithm SHA256).Hash
    harness_sha256 = (Get-FileHash -LiteralPath $PSCommandPath -Algorithm SHA256).Hash
    seed = $Seed; scenarios = $Scenarios; repeats = $Repeats; capture_only = [bool]$CaptureOnly
    timeout_seconds = $TimeoutSeconds; runs = @()
    notes = @('Original exported executable and PCK; no source or PCK changes.', 'External probe freezes governor at quality 1 and cancels screen ownership. Existing scene workload and VFX caps retained.', 'Offscreen rendered stress test, not onscreen presentation or cross-engine comparison.', 'Engine startup adapter line and probe adapter_name are authoritative; Windows adapter inventory is not used.')
}
function Save-Manifest { [IO.File]::WriteAllText((Join-Path $OutputRoot 'manifest.json'), ($manifest | ConvertTo-Json -Depth 12), $utf8) }
Save-Manifest
$index = 0
foreach ($repeat in 1..$Repeats) {
    foreach ($scenario in $Scenarios) {
        $index++
        $runDir = Join-Path $OutputRoot ('{0:D2}-{1}' -f $index,$scenario)
        $null = New-Item -ItemType Directory -Path $runDir
        $scene = if ($scenario -eq 'bullets') { 'res://scenes/remote_minigun_benchmark.tscn' } else { 'res://scenes/perf_benchmark.tscn' }
        $userArg = if ($scenario -eq 'bullets') { 'build=minigun,minigun,minigun,shotgun,shotgun,shotgun' } else { 'mode=wall_spray' }
        $probeForward = $probe.Replace('\','/')
        $config = "[application]`nrun/main_scene=`"$scene`"`nrun/max_fps=0`n`n[display]`nwindow/size/viewport_width=1920`nwindow/size/viewport_height=1080`nwindow/size/window_width_override=1920`nwindow/size/window_height_override=1080`nwindow/size/mode=0`nwindow/vsync/vsync_mode=0`n`n[autoload]`nStressProbe=`"*$probeForward`"`n"
        if (Test-Path -LiteralPath $overridePath) { throw 'override.cfg appeared between runs; refusing to overwrite it.' }
        [IO.File]::WriteAllText((Join-Path $runDir 'owned-override.cfg.txt'), $config, $utf8)
        $record = [ordered]@{ index=$index; repeat=$repeat; scenario=$scenario; scene=$scene; user_argument=$userArg; directory=$runDir; started_utc=[DateTime]::UtcNow.ToString('o'); status='starting'; pid=$null; exit_code=$null; timeout=$false; override_removed=$false }
        $process = $null
        try {
            [IO.File]::WriteAllText($overridePath, $config, $utf8)
            $engineLog = Join-Path $runDir 'engine.log'
            $argLine = '--resolution 1920x1080 --position ' + $WindowPosition + ' --max-fps 0 --print-fps --log-file "' + $engineLog + '" -- ' + $userArg
            $record.arguments = $argLine
            $envValues = @{ GG_NO_TRACE='1'; GG_STRESS_OUT=$runDir; GG_STRESS_SCENARIO=$scenario; GG_STRESS_SEED=[string]$Seed; GG_STRESS_POSITION=$WindowPosition; GG_STRESS_CAPTURE_ONLY=$(if($CaptureOnly){'1'}else{'0'}) }
            $savedEnv = @{}
            try {
                foreach ($key in $envValues.Keys) { $savedEnv[$key]=[Environment]::GetEnvironmentVariable($key,'Process'); [Environment]::SetEnvironmentVariable($key,$envValues[$key],'Process') }
                $process = Start-Process -FilePath $resolvedExe -WorkingDirectory $gameDir -ArgumentList $argLine -PassThru -WindowStyle Hidden -RedirectStandardOutput (Join-Path $runDir 'stdout.log') -RedirectStandardError (Join-Path $runDir 'stderr.log')
            } finally {
                foreach ($key in $savedEnv.Keys) { [Environment]::SetEnvironmentVariable($key,$savedEnv[$key],'Process') }
            }
            $record.pid = $process.Id
            # Pin the native handle before exit; Windows PowerShell otherwise can
            # expose a null ExitCode for a Start-Process Process wrapper.
            $ownedProcessHandle = $process.Handle
            $record.process_started_utc = $process.StartTime.ToUniversalTime().ToString('o')
            Write-Host ('[{0}] {1} repeat {2}: PID {3}' -f $index,$scenario,$repeat,$process.Id)
            if (-not $process.WaitForExit($TimeoutSeconds * 1000)) {
                $record.timeout = $true
                # Kill this owned process object/handle only, never by name or process tree.
                $process.Kill()
                $null = $process.WaitForExit(5000)
            }
            $process.Refresh()
            $record.exit_code = $process.ExitCode
            $summaryPath = Join-Path $runDir 'summary.json'
            $record.status = if ($record.timeout) { 'timeout' } elseif ($null -eq $record.exit_code) { 'exit_code_unavailable' } elseif ($record.exit_code -ne 0) { 'process_failed' } elseif (-not (Test-Path -LiteralPath $summaryPath)) { 'missing_probe_summary' } else { 'completed' }
            if (Test-Path -LiteralPath $summaryPath) {
                $summary = Get-Content -LiteralPath $summaryPath -Raw | ConvertFrom-Json
                $record.adapter_name = $summary.adapter_name
                $record.probe_reason = $summary.reason
                $record.probe_samples = $summary.raw_process_loop_interval_us.count
                if (-not $CaptureOnly -and (-not $summary.scene_completed -or $summary.raw_process_loop_interval_us.count -lt 1)) { $record.status = 'incomplete_measurement' }
                if ($CaptureOnly -and -not (Test-Path -LiteralPath (Join-Path $runDir 'visual.jpg'))) { $record.status = 'missing_visual' }
            }
            $adapterLines = @()
            if (Test-Path -LiteralPath $engineLog) { $adapterLines = @(Select-String -LiteralPath $engineLog -Pattern 'Using Device|Vulkan|D3D12|OpenGL|\[stress-probe\] loaded' | ForEach-Object { $_.Line }) }
            $record.renderer_startup_lines = $adapterLines
            $record.adapter_log_present = @($adapterLines | Where-Object { $_ -match 'Using Device|Vulkan|D3D12|OpenGL' }).Count -gt 0
        } catch {
            $record.status = 'harness_error'
            $record.error = $_.Exception.Message
            if ($process -and -not $process.HasExited) { $process.Kill(); $null = $process.WaitForExit(5000) }
        } finally {
            if (Test-Path -LiteralPath $overridePath) {
                if ([string]::Equals([IO.File]::ReadAllText($overridePath),$config,[StringComparison]::Ordinal)) {
                    Remove-Item -LiteralPath $overridePath
                    $record.override_removed = $true
                } else { $record.cleanup_warning = 'override.cfg changed; left untouched.' }
            }
            $record.ended_utc = [DateTime]::UtcNow.ToString('o')
            $manifest.runs += $record
            Save-Manifest
        }
        Write-Host ('[{0}] {1}: {2}, exit {3}, samples {4}' -f $index,$scenario,$record.status,$record.exit_code,$record.probe_samples)
        if ($record.status -ne 'completed') { throw "Run failed ($($record.status)); retained logs at $runDir" }
        if (-not $record.override_removed) { throw 'Could not confirm owned override.cfg cleanup; stopping.' }
    }
}
Write-Host "Completed. Results: $OutputRoot"
