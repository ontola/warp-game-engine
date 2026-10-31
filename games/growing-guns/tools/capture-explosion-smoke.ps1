param(
    [string]$Executable = "",
    [string]$Out = "build/explosion-smoke",
    [ValidateSet('baked', 'legacy')][string]$Variant = 'baked',
    [ValidateRange(1,4)][int]$Views = 1,
    [ValidateRange(64,4096)][int]$Width = 960,
    [ValidateRange(64,4096)][int]$Height = 540,
    [switch]$Orbit
)
$ErrorActionPreference = 'Stop'
if ($Width % 2 -ne 0 -or $Height % 2 -ne 0) { throw 'Width and Height must be even for the video encoder.' }
$repo = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../../..'))
if (!$Executable) { $Executable = Join-Path $repo 'target/debug/growing-guns.exe' }
$Executable = [IO.Path]::GetFullPath($Executable)
if (!(Test-Path -LiteralPath $Executable)) { throw "Build growing-guns-app first, or pass -Executable." }
$destination = if ([IO.Path]::IsPathRooted($Out)) { $Out } else { Join-Path $repo $Out }
[IO.Directory]::CreateDirectory($destination) | Out-Null
$steps = @(@{cmd=@('bots pause','health 0 10000','teleport 0 7 23 12','aim 0 0 21 0');ticks=2;capture='baseline.png';state='baseline.json'}, @{cmd='boom 0 16 0 6.5 0';ticks=1})
for ($frame=0; $frame -lt 96; $frame++) {
    $commands = @()
    for ($seat=0; $seat -lt $Views; $seat++) {
        $angle = 0.528 + $seat * [Math]::PI / 2
        if ($Orbit) { $angle += $frame / 95.0 * 1.1 }
        $x = (13.9 * [Math]::Sin($angle)).ToString('F4',[Globalization.CultureInfo]::InvariantCulture)
        $z = (13.9 * [Math]::Cos($angle)).ToString('F4',[Globalization.CultureInfo]::InvariantCulture)
        $commands += "health $seat 10000", "teleport $seat $x 23 $z", "aim $seat 0 21 0"
    }
    $steps += @{cmd=$commands;ticks=2;capture=('smoke-{0:D3}.png' -f $frame)}
}
$scriptFile = Join-Path $destination 'scenario.json'
[IO.File]::WriteAllText($scriptFile, (@{size=@($Width,$Height);steps=$steps}|ConvertTo-Json -Depth 6))
$priorSmoke = $env:GG_BAKED_SMOKE
Push-Location $repo
try {
    $env:GG_BAKED_SMOKE = if ($Variant -eq 'legacy') {'0'} else {'1'}
    & $Executable script $scriptFile --out $destination --players $Views --seed 3 --graphics full --no-music > (Join-Path $destination 'capture.log')
    if ($LASTEXITCODE -ne 0) { throw "Explosion capture failed ($LASTEXITCODE)." }
    $encoder = Get-Command ffmpeg -ErrorAction SilentlyContinue
    if ($encoder) {
        & $encoder.Source -hide_banner -loglevel error -y -framerate 30 -i (Join-Path $destination 'smoke-%03d.png') -c:v libx264 -crf 18 -pix_fmt yuv420p -movflags +faststart (Join-Path $destination 'explosion.mp4')
        if ($LASTEXITCODE -ne 0) { throw 'Video encoding failed.' }
    }
} finally {
    Pop-Location
    $env:GG_BAKED_SMOKE = $priorSmoke
}
Write-Output "Preview: $destination"
