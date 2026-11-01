[CmdletBinding()]
param(
    [string]$Executable = 'build/spaceracer-native/spaceracer.exe',
    [string]$OutputDirectory = 'build/spaceracer-profile',
    [int[]]$Seeds = @(35),
    [ValidateSet('city','forest','cell','desert','ocean')][string[]]$Biomes = @('city','forest','cell'),
    [ValidateRange(1,4)][int[]]$Players = @(1,2),
    [ValidateRange(0,100000)][int]$Ticks = 2400,
    [ValidateRange(30,10000)][int]$Frames = 360,
    [ValidateRange(1,10000)][int]$Warmup = 120
)
$ErrorActionPreference = 'Stop'
$workspace = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../../..'))
Push-Location $workspace
$oldSpans = $env:KERN_CPU_SPANS
try {
    $binary = (Resolve-Path -LiteralPath $Executable).Path
    $output = [IO.Path]::GetFullPath($(if ([IO.Path]::IsPathRooted($OutputDirectory)) { $OutputDirectory } else { Join-Path $workspace $OutputDirectory }))
    if (Test-Path -LiteralPath $output) { throw "Use a fresh output directory: $output" }
    New-Item -ItemType Directory -Path $output | Out-Null
    $env:KERN_CPU_SPANS = '1'
    # Record provenance once; never change clocks, background apps or quality.
    $manifest = [ordered]@{
        utc = [DateTime]::UtcNow.ToString('o')
        executable = $binary
        sha256 = (Get-FileHash -LiteralPath $binary -Algorithm SHA256).Hash
        source_revision = (& git rev-parse HEAD)
        source_status = @(& git status --short)
        cpu = @(Get-CimInstance Win32_Processor | Select-Object Name,NumberOfCores,NumberOfLogicalProcessors)
        gpu = @(Get-CimInstance Win32_VideoController | Select-Object Name,DriverVersion)
        seeds = $Seeds; biomes = $Biomes; players = $Players; ticks = $Ticks; frames = $Frames; warmup = $Warmup
        note = 'Serial headless native 3840x2160 runs. No screenshots, playtest windows or resolution adaptation. Compile first, do not build concurrently.'
    }
    $manifest | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $output 'manifest.json') -Encoding utf8
    foreach ($seed in $Seeds) { foreach ($biome in $Biomes) { foreach ($count in $Players) {
        $name = "$biome-$seed-${count}p"
        $report = Join-Path $output "$name.json"
        Write-Output "Profiling $name at native 4K"
        & $binary --demo --seed $seed --biome $biome --difficulty hard --players $count --ticks $Ticks --bench $Frames --warmup $Warmup --width 3840 --height 2160 --out $report *> (Join-Path $output "$name.log")
        if ($LASTEXITCODE -ne 0) { throw "Profile failed: $name; see its log" }
        $result = Get-Content -LiteralPath $report -Raw | ConvertFrom-Json
        if ($result.profile.gpu_samples -ne $Frames -or $result.profile.errors -gt 0 -or $result.profile.dropped -gt 0 -or $result.profile.pending -gt 0) {
            throw "Incomplete GPU timestamp coverage: $name"
        }
    } } }
    Write-Output "Reports: $output"
} finally {
    $env:KERN_CPU_SPANS = $oldSpans
    Pop-Location
}
