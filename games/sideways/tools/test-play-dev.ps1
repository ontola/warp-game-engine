# Contract checks only: no Cargo, child processes, game window, or GPU.
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'play-dev-support.ps1')
function Assert-That([bool]$Condition, [string]$Message) { if (-not $Condition) { throw $Message } }

$nativeSetup = Get-DlssBuildSetup -Upscale native -Sdk '' -VulkanHeaders '' -Runtime ''
Assert-That ($nativeSetup.features.Count -eq 0 -and $null -eq $nativeSetup.dlss) 'Native builds must not depend on optional NVIDIA files.'
$missingRejected = $false
try { Get-DlssBuildSetup -Upscale dlss-quality -Sdk '' -VulkanHeaders '' -Runtime '' | Out-Null }
catch { $missingRejected = $_.Exception.Message -match 'NVIDIA_DLSS_SDK' }
Assert-That $missingRejected 'Explicit DLSS must fail before building if its SDK is missing.'

$record = [pscustomobject]@{ pid = 123; executable = 'C:\repo\games\sideways\build\dev-runs\run-a\sideways.exe'; start_utc_ticks = 456L }
$root = 'C:\repo\games\sideways\build\dev-runs'
Assert-That (Test-DevProcessIdentity $record 123 $record.executable 456 $root) 'Owned process should match.'
Assert-That (-not (Test-DevProcessIdentity $record 123 $record.executable 457 $root)) 'PID reuse must not authorize stopping.'
Assert-That (-not (Test-DevProcessIdentity $record 124 $record.executable 456 $root)) 'A different PID must not match.'
Assert-That (-not (Test-DevProcessIdentity $record 123 'C:\other\sideways.exe' 456 $root)) 'Same filename elsewhere is not owned.'
$outside = [pscustomobject]@{ pid = 123; executable = 'C:\repo\games\sideways\build\dev-runs-old\run-a\sideways.exe'; start_utc_ticks = 456L }
Assert-That (-not (Test-DevProcessIdentity $outside 123 $outside.executable 456 $root)) 'A prefix-sibling directory is not owned.'
Assert-That (-not (Test-VisibleFirstFrame 'play: first frame entered (invisible=false)')) 'Entering a frame is not presentation proof.'
Assert-That (-not (Test-VisibleFirstFrame 'play: first frame complete (presented=0)')) 'Hidden rendering is not visible presentation.'
Assert-That (Test-VisibleFirstFrame "adapter log`r`nplay: first frame complete (presented=1)`r`n") 'Visible completion must be recognized.'
Assert-That (Test-VisibleFirstFrame "play: first frame complete (presented=0)`r`nplay: first visible frame complete`r`n") 'A later first presentation after surface recovery must be recognized.'
$startupRows = @(Get-DevStartupProfiles @('ordinary log', '{"kind":"renderer_startup","total_cpu_ms":12}', '{"kind":"unrelated","elapsed_ms":999}', '{"kind":"cli_startup","event":"first_scene_frame","partition_end_ms":40}', '{"kind":"renderer_startup"'))
Assert-That ($startupRows.Count -eq 2) 'Only complete structured startup records are evidence; partial/unrelated log lines are not timing.'
Assert-That ($startupRows[1].partition_end_ms -eq 40) 'CLI elapsed partition must remain distinct from renderer constructor CPU time.'

$artifact = '{"reason":"compiler-artifact","target":{"name":"sideways"},"executable":"C:\\custom target\\debug\\sideways.exe"}'
$finished = '{"reason":"build-finished","success":true}'
Assert-That ((Get-SidewaysBuildArtifact @($artifact,$finished)) -eq 'C:\custom target\debug\sideways.exe') 'Use the reported artifact, independent of profile/target location.'
$rejected = $false
try { Get-SidewaysBuildArtifact @($artifact,'{"reason":"build-finished","success":false}') | Out-Null } catch { $rejected = $true }
Assert-That $rejected 'A failed build must not accept an earlier artifact.'
$preparedFixture = Join-Path ([IO.Path]::GetTempPath()) ('warp-prebuilt-' + [Guid]::NewGuid().ToString('N') + '.exe')
try {
    [IO.File]::WriteAllText($preparedFixture, 'artifact identity fixture; not executed')
    $preparedHash = (Get-FileHash -LiteralPath $preparedFixture -Algorithm SHA256).Hash
    $prepared = Get-PreparedDevArtifact $preparedFixture $preparedHash
    Assert-That ($prepared.sha256 -eq $preparedHash.ToLowerInvariant()) 'Prepared artifact must retain its explicit hash.'
    [IO.File]::AppendAllText($preparedFixture, 'modified')
    $rejected = $false
    try { Get-PreparedDevArtifact $preparedFixture $preparedHash | Out-Null } catch { $rejected = $true }
    Assert-That $rejected 'Changed prepared artifacts must fail before any process replacement.'
} finally { Remove-Item -LiteralPath $preparedFixture -ErrorAction SilentlyContinue }
Assert-That ((ConvertTo-NativeArgument 'C:\path with spaces\') -eq '"C:\path with spaces\\"') 'Trailing backslash must not escape the closing quote.'
Assert-That ((ConvertTo-NativeArgument 'say "hi"') -eq '"say \"hi\""') 'Embedded quotes must remain an argument, not shell code.'

foreach ($file in @('play-dev-support.ps1', 'play-dev-windows.ps1', 'play-windows.ps1', 'test-play-dev.ps1')) {
    $tokens = $null; $parseErrors = $null
    [Management.Automation.Language.Parser]::ParseFile((Join-Path $PSScriptRoot $file), [ref]$tokens, [ref]$parseErrors) | Out-Null
    Assert-That ($parseErrors.Count -eq 0) "PowerShell parse failure in $file : $parseErrors"
}
foreach ($file in @('play-dev-windows.ps1', 'play-windows.ps1')) {
    $tokens = $null; $parseErrors = $null
    $ast = [Management.Automation.Language.Parser]::ParseFile((Join-Path $PSScriptRoot $file), [ref]$tokens, [ref]$parseErrors)
    $parameter = @($ast.ParamBlock.Parameters | Where-Object { $_.Name.VariablePath.UserPath -eq 'Upscale' })
    Assert-That ($parameter.Count -eq 1) "$file must expose one Upscale parameter."
    Assert-That ($parameter[0].DefaultValue.SafeGetValue() -eq 'native') "$file must preserve native by default."
    $validation = @($parameter[0].Attributes | Where-Object { $_.TypeName.Name -eq 'ValidateSet' })
    $values = @($validation[0].PositionalArguments | ForEach-Object { $_.SafeGetValue() })
    Assert-That (($values -join ',') -eq 'native,dlss-quality,dlss-balanced,dlss-performance') "$file must reject unknown modes before any build or launch."
}
$tokens = $null; $parseErrors = $null
$debugAst = [Management.Automation.Language.Parser]::ParseFile((Join-Path $PSScriptRoot 'play-dev-windows.ps1'), [ref]$tokens, [ref]$parseErrors)
$debugParameter = @($debugAst.ParamBlock.Parameters | Where-Object { $_.Name.VariablePath.UserPath -eq 'DebugUi' })
Assert-That ($debugParameter.Count -eq 1 -and $debugParameter[0].StaticType -eq [Management.Automation.SwitchParameter]) 'DebugUi must be one optional switch.'
Assert-That ($null -eq $debugParameter[0].DefaultValue) 'Debug UI must remain closed by default.'
$debugForward = @($debugAst.FindAll({ param($node)
    $node -is [Management.Automation.Language.IfStatementAst] -and
        $node.Clauses.Count -eq 1 -and $node.Clauses[0].Item1.Extent.Text -eq '$DebugUi'
}, $true))
Assert-That ($debugForward.Count -eq 1) 'DebugUi must have exactly one conditional argument-forwarding branch.'
foreach ($enabled in @($false, $true)) {
    # Execute only the isolated argument branch, never the launcher/build body.
    $debugArguments = & {
        param([string]$Forward, [bool]$Enabled)
        $DebugUi = $Enabled; $arguments = @('play')
        . ([ScriptBlock]::Create($Forward))
        return ,$arguments
    } $debugForward[0].Extent.Text $enabled
    Assert-That (@($debugArguments | Where-Object { $_ -eq '--debug-ui' }).Count -eq [int]$enabled) 'DebugUi must forward --debug-ui once only when requested.'
    Assert-That ($debugArguments[0] -eq 'play' -and $debugArguments.Count -eq (1 + [int]$enabled)) 'DebugUi must preserve other arguments.'
}
Write-Host 'PASS: developer launcher identity, artifact, presentation-proof, quoting, debug-UI forwarding, and syntax contracts. No process launched.'
