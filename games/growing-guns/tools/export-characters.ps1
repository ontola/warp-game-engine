[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$SourceRoot,
    [Parameter(Mandatory = $true)][string]$Godot,
    [string]$OutputDirectory = (Join-Path $PSScriptRoot '../assets/characters'),
    [string]$StagingDirectory = (Join-Path $PSScriptRoot '../../../build/character-extract')
)
$ErrorActionPreference = 'Stop'
$sourcePath = (Resolve-Path -LiteralPath $SourceRoot).ProviderPath
$godotPath = (Resolve-Path -LiteralPath $Godot).ProviderPath
$destinationPath = [IO.Path]::GetFullPath($OutputDirectory)
$stagePath = [IO.Path]::GetFullPath($StagingDirectory)
if ($stagePath -eq $sourcePath -or $destinationPath -eq $sourcePath) {
    throw 'The isolated staging and output directories must differ from the source project.'
}
New-Item -ItemType Directory -Force $stagePath, $destinationPath,
    (Join-Path $stagePath 'assets/models'), (Join-Path $stagePath 'assets/animations') | Out-Null
Set-Content -LiteralPath (Join-Path $stagePath 'project.godot') -Value 'config_version=5' -Encoding utf8

$sourceFiles = @(
    'assets/models/knight.fbx', 'assets/models/paladin.fbx', 'assets/models/ch10.fbx',
    'assets/animations/Pistol Idle.fbx', 'assets/animations/Pistol Run.fbx', 'assets/animations/Pistol Jump.fbx'
)
foreach ($file in @($sourceFiles)) {
    $import = "$file.import"
    if (Test-Path -LiteralPath (Join-Path $sourcePath $import)) { $sourceFiles += $import }
}
foreach ($pattern in @('knight_*.png', 'paladin_*.png', 'ch10_*.png')) {
    foreach ($texture in Get-ChildItem -LiteralPath (Join-Path $sourcePath 'assets/models') -Filter $pattern) {
        $sourceFiles += 'assets/models/' + $texture.Name
    }
}
foreach ($file in $sourceFiles) {
    Copy-Item -LiteralPath (Join-Path $sourcePath $file) -Destination (Join-Path $stagePath $file) -Force
}
Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'export-characters.gd') -Destination (Join-Path $stagePath 'export-characters.gd') -Force
& $godotPath --headless --editor --path $stagePath --import
if ($LASTEXITCODE -ne 0) { throw "Godot asset import failed ($LASTEXITCODE)" }
& $godotPath --headless --path $stagePath --script res://export-characters.gd -- $destinationPath
if ($LASTEXITCODE -ne 0) { throw "Character extraction failed ($LASTEXITCODE)" }

# Hash the actual working-copy inputs: the source assets may differ from HEAD.
$sourceFiles += 'scripts/character_visual.gd', 'LICENSE'
Copy-Item -LiteralPath (Join-Path $sourcePath 'LICENSE') -Destination (Join-Path $destinationPath 'SOURCE-SOFTWARE-LICENSE.txt') -Force
$provenance = [ordered]@{
    format_version = 1
    source_project = 'Growing Guns'
    source_files = @($sourceFiles | Sort-Object | ForEach-Object {
        [ordered]@{ path = $_; sha256 = (Get-FileHash -LiteralPath (Join-Path $sourcePath $_) -Algorithm SHA256).Hash.ToLowerInvariant() }
    })
    exporter_files = @('export-characters.gd', 'export-characters.ps1') | ForEach-Object {
        [ordered]@{ path = $_; sha256 = (Get-FileHash -LiteralPath (Join-Path $PSScriptRoot $_) -Algorithm SHA256).Hash.ToLowerInvariant() }
    }
    generated_files = @(Get-ChildItem -LiteralPath $destinationPath -File | Where-Object {
        $_.Extension -eq '.bin' -or $_.Extension -eq '.png' -or $_.Name -in @('knight.json', 'paladin.json', 'ch10.json', 'SOURCE-SOFTWARE-LICENSE.txt')
    } | Sort-Object Name | ForEach-Object {
        [ordered]@{ path = $_.Name; sha256 = (Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash.ToLowerInvariant() }
    })
}
$provenance | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath (Join-Path $destinationPath 'provenance.json') -Encoding utf8
Write-Host "Exported the original knight, paladin and classic zombie to $destinationPath"
