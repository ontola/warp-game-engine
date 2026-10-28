<#
.SYNOPSIS
Compare two existing Growing Guns executables; no checkout or build.
.EXAMPLE
pwsh -File games/growing-guns/tools/compare-cpu-spikes.ps1 -Baseline C:\bench\before.exe -Candidate C:\bench\after.exe -Out C:\bench\fresh-ab
python games/growing-guns/tools/analyze-cpu-spikes.py --manifest C:\bench\fresh-ab\manifest.json --require-complete
.DESCRIPTION
Copies and hashes both executables into a NEW evidence directory, then runs
bullets/explosions at 1080p/1 view and 4K/4 views. Three alternating A/B repeats
by default. Screenshots are taken after measurement in the first repeat.
Known concurrent games block the run; they are never stopped. Only processes
started by this runner are terminated on timeout/error. Guard checks are
snapshots, not proof that the host was otherwise idle throughout a run.
Run only after arranging exclusive GPU use. Existing evidence is never reused.
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory=$true)][string]$Baseline,
    [Parameter(Mandatory=$true)][string]$Candidate,
    [Parameter(Mandatory=$true)][string]$Out,
    [string]$RepoRoot = (Join-Path $PSScriptRoot '..\..\..'),
    [ValidateRange(1,10)][int]$Repeats = 3,
    [ValidateRange(30,3600)][int]$TimeoutSeconds = 180,
    [ValidateRange(1,60)][int]$PollSeconds = 30
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
if ($env:OS -ne 'Windows_NT') { throw 'This runner requires Windows.' }

$resolvedRepo = (Resolve-Path -LiteralPath $RepoRoot).ProviderPath
if (-not (Test-Path -LiteralPath (Join-Path $resolvedRepo 'games/growing-guns/app/Cargo.toml') -PathType Leaf)) {
    throw "RepoRoot is not the Warp checkout: $resolvedRepo"
}
$baselineSource = (Resolve-Path -LiteralPath $Baseline).ProviderPath
$candidateSource = (Resolve-Path -LiteralPath $Candidate).ProviderPath
foreach ($source in @($baselineSource, $candidateSource)) {
    if (-not (Test-Path -LiteralPath $source -PathType Leaf) -or [IO.Path]::GetExtension($source) -ne '.exe') {
        throw "Expected an existing executable: $source"
    }
}
$resolvedOut = $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($Out)
if (Test-Path -LiteralPath $resolvedOut) { throw "Output exists; preserve evidence and choose a new -Out: $resolvedOut" }
$copiedBaseline = Join-Path $resolvedOut 'baseline.exe'
$copiedCandidate = Join-Path $resolvedOut 'candidate.exe'
$guardPaths = @($baselineSource, $candidateSource, $copiedBaseline, $copiedCandidate)
$guardNames = @($guardPaths | ForEach-Object { [IO.Path]::GetFileName($_) })

function Get-ConcurrentGames {
    # Match common original games, other benchmark snapshots, and both supplied
    # executable names/paths even when the caller used a custom filename.
    $pattern = '^(sideways(?:[-_].*)?|growing-guns(?:[-_].*)?|morerounds|godot(?:[-_].*)?|baseline(?:[-_].*)?|candidate(?:[-_].*)?|playtest(?:[-_].*)?)\.exe$'
    $gameProcesses = @(Get-CimInstance Win32_Process | Where-Object {
        $_.Name -match $pattern -or $_.Name -in $guardNames -or ($_.ExecutablePath -and $_.ExecutablePath -in $guardPaths)
    } | Select-Object Name,ProcessId,ExecutablePath,CreationDate)
    return [ordered]@{ checked_utc=[DateTime]::UtcNow.ToString('o'); games=$gameProcesses }
}

function Assert-NoGames($guard) {
    if (@($guard.games).Count -gt 0) {
        throw ('Concurrent game/fixture detected; stop it before benchmarking: ' + (($guard.games | ForEach-Object { "$($_.Name) PID $($_.ProcessId)" }) -join ', '))
    }
}

function Get-CpuContext {
    try { return Get-CimInstance Win32_PerfFormattedData_PerfOS_Processor -Filter "Name='_Total'" | Select-Object -ExpandProperty PercentProcessorTime }
    catch { return $null }
}

function Get-GpuContext([string]$Fields) {
    $smi = Get-Command nvidia-smi -ErrorAction SilentlyContinue
    if (-not $smi) { return 'nvidia-smi unavailable' }
    try {
        $result = & $smi.Source "--query-gpu=$Fields" '--format=csv,noheader' 2>&1
        if ($LASTEXITCODE -ne 0) { return 'nvidia-smi query failed' }
        return @($result | ForEach-Object { "$_" })
    } catch { return 'nvidia-smi query failed' }
}

Assert-NoGames (Get-ConcurrentGames)
$sourceHashes = @{
    baseline=(Get-FileHash -LiteralPath $baselineSource -Algorithm SHA256).Hash
    candidate=(Get-FileHash -LiteralPath $candidateSource -Algorithm SHA256).Hash
}
New-Item -ItemType Directory -Path $resolvedOut | Out-Null
Copy-Item -LiteralPath $baselineSource -Destination $copiedBaseline
Copy-Item -LiteralPath $candidateSource -Destination $copiedCandidate
foreach ($variant in @('baseline','candidate')) {
    $copyPath = if ($variant -eq 'baseline') { $copiedBaseline } else { $copiedCandidate }
    if ((Get-FileHash -LiteralPath $copyPath -Algorithm SHA256).Hash -ne $sourceHashes[$variant]) {
        throw "Executable changed while copying $variant; evidence preserved, choose a new -Out."
    }
    (Get-Item -LiteralPath $copyPath).IsReadOnly = $true
}
$manifestPath = Join-Path $resolvedOut 'manifest.json'
$manifest = [ordered]@{
    schema=2; runner='compare-cpu-spikes.ps1'; start_utc=[DateTime]::UtcNow.ToString('o')
    repo_root=$resolvedRepo; baseline=$copiedBaseline; candidate=$copiedCandidate
    baseline_source=$baselineSource; candidate_source=$candidateSource
    baseline_sha256=$sourceHashes.baseline; candidate_sha256=$sourceHashes.candidate
    repeats=$Repeats; timeout_seconds=$TimeoutSeconds; poll_seconds=$PollSeconds
    cpu=(Get-CimInstance Win32_Processor | Select-Object Name,NumberOfLogicalProcessors)
    gpu=(Get-GpuContext 'name,driver_version'); runs=@()
    notes=@('Copied executable hashes checked before and after each run; source builds cannot replace the measured copies.',
        'Same checkout assets for both binaries; no checkout/build is performed.',
        'Known-game guards and utilization are snapshots, not continuous proof of an idle host.',
        'Unprofiled primary A/B runs only; --cpu-thread diagnostics must be analyzed separately.',
        'Pilot files are not inputs; prior Sideways-contaminated pilot is excluded.')
}
function Save-Manifest { $manifest | ConvertTo-Json -Depth 12 | Set-Content -LiteralPath $manifestPath -Encoding UTF8 }
Save-Manifest
$cases = @(
    @{scenario='bullets';views=1;size='1920x1080'}, @{scenario='explosions';views=1;size='1920x1080'},
    @{scenario='bullets';views=4;size='3840x2160'}, @{scenario='explosions';views=4;size='3840x2160'}
)

try {
    foreach ($repeat in 1..$Repeats) {
        foreach ($case in $cases) {
            $order = if ($repeat % 2 -eq 1) { @('baseline','candidate') } else { @('candidate','baseline') }
            foreach ($variant in $order) {
                $beforeGuard = Get-ConcurrentGames
                Assert-NoGames $beforeGuard
                $exe = if ($variant -eq 'baseline') { $copiedBaseline } else { $copiedCandidate }
                $beforeHash = (Get-FileHash -LiteralPath $exe -Algorithm SHA256).Hash
                if ($beforeHash -ne $sourceHashes[$variant]) { throw "Copied $variant changed before measurement." }
                $stem = Join-Path $resolvedOut ('{0}-{1}-{2}-{3}v' -f $repeat,$variant,$case.scenario,$case.views)
                $arguments = 'bench --stress '+$case.scenario+' --views '+$case.views+' --size '+$case.size+' --seed 721077 --csv "'+$stem+'.csv"'
                if ($repeat -eq 1) { $arguments += ' --shot "'+$stem+'.png"' }
                $record = [ordered]@{
                    repeat=$repeat; variant=$variant; scenario=$case.scenario; views=$case.views; size=$case.size
                    stem=$stem; arguments=$arguments; start_utc=[DateTime]::UtcNow.ToString('o'); status='running'
                    cpu_before=(Get-CpuContext); gpu_before=(Get-GpuContext 'temperature.gpu,utilization.gpu,clocks.gr')
                    concurrency_before=$beforeGuard; concurrency_after=$null; executable_sha256_before=$beforeHash
                    timeout=$false; exit_code=$null; heartbeats=@()
                }
                $manifest.runs += $record
                Save-Manifest
                Write-Host "$repeat/$Repeats $variant $($case.scenario) $($case.views) views"
                $owned = $null
                try {
                    $owned = Start-Process -FilePath $exe -WorkingDirectory $resolvedRepo -ArgumentList $arguments -WindowStyle Hidden -PassThru -RedirectStandardOutput ($stem+'.log') -RedirectStandardError ($stem+'.err.log')
                    $ownedHandle = $owned.Handle # Keep the original process handle, not a later PID lookup.
                    $record.pid = $owned.Id
                    $record.process_started_utc = $owned.StartTime.ToUniversalTime().ToString('o')
                    $watch = [Diagnostics.Stopwatch]::StartNew()
                    while ($true) {
                        $remainingMs = [Math]::Max(0, $TimeoutSeconds * 1000 - $watch.ElapsedMilliseconds)
                        if ($remainingMs -eq 0) { $record.timeout = $true; break }
                        $waitMs = [int][Math]::Min($PollSeconds * 1000, $remainingMs)
                        if ($owned.WaitForExit($waitMs)) { break }
                        $record.heartbeats += [ordered]@{utc=[DateTime]::UtcNow.ToString('o');elapsed_seconds=$watch.Elapsed.TotalSeconds}
                        Write-Host ("  still running {0:N0}s / {1}s timeout" -f $watch.Elapsed.TotalSeconds,$TimeoutSeconds)
                    }
                    if ($record.timeout) { $owned.Kill(); $null = $owned.WaitForExit(5000) }
                    $owned.Refresh()
                    $record.exit_code = $owned.ExitCode
                    $record.concurrency_after = Get-ConcurrentGames
                    Assert-NoGames $record.concurrency_after
                    $record.executable_sha256_after = (Get-FileHash -LiteralPath $exe -Algorithm SHA256).Hash
                    if ($record.executable_sha256_after -ne $beforeHash) { throw "Copied $variant changed during measurement." }
                    if ($record.timeout -or $null -eq $record.exit_code -or $record.exit_code -ne 0) { throw "Benchmark failed: $stem" }
                    $record.status = 'completed'
                    Get-Content -LiteralPath ($stem+'.log') | Select-String 'CPU total |CPU \+ GPU wait |GPU union |GPU timestamp coverage'
                } catch {
                    $record.status = 'failed'
                    $record.failure = $_.Exception.Message
                    throw
                } finally {
                    if ($null -ne $owned) {
                        if (-not $owned.HasExited) { $owned.Kill(); $null = $owned.WaitForExit(5000) }
                        $owned.Refresh()
                        if ($owned.HasExited) { $record.exit_code = $owned.ExitCode }
                        $owned.Dispose()
                    }
                    $record.end_utc = [DateTime]::UtcNow.ToString('o')
                    $record.cpu_after = Get-CpuContext
                    $record.gpu_after = Get-GpuContext 'temperature.gpu,utilization.gpu,clocks.gr'
                    Save-Manifest
                }
            }
        }
    }
    $manifest.end_utc = [DateTime]::UtcNow.ToString('o')
    Save-Manifest
    Write-Host "Completed evidence: $manifestPath"
} catch {
    $manifest.failure = $_.Exception.Message
    $manifest.failed_utc = [DateTime]::UtcNow.ToString('o')
    Save-Manifest
    throw
}
