# Pure helpers shared with the no-process contract checks.
Set-StrictMode -Version Latest

function ConvertTo-NativeArgument {
    param([AllowEmptyString()][string]$Value)
    # Windows CRT quoting. Never use this as cmd.exe/PowerShell shell code.
    '"' + [regex]::Replace([regex]::Replace($Value, '(\\*)"', '$1$1\"'), '(\\+)$', '$1$1') + '"'
}

function Get-SidewaysBuildArtifact {
    param([string[]]$Lines)
    $executable = $null
    $completed = $false
    foreach ($line in $Lines) {
        if ([string]::IsNullOrWhiteSpace($line)) { continue }
        $record = $line | ConvertFrom-Json -ErrorAction Stop
        if ($record.reason -eq 'compiler-artifact' -and $record.target.name -eq 'sideways' -and $record.executable) {
            $executable = [string]$record.executable
        }
        if ($record.reason -eq 'build-finished') { $completed = [bool]$record.success }
    }
    if (-not $completed -or -not $executable) { throw 'Cargo did not report a successful sideways executable artifact.' }
    return $executable
}

function Test-DevProcessIdentity {
    param($Record, [int]$ProcessId, [string]$Executable, [long]$StartTicks, [string]$RunRoot)
    try {
        $root = [IO.Path]::GetFullPath($RunRoot).TrimEnd('\', '/')
        $saved = [IO.Path]::GetFullPath([string]$Record.executable)
        $actual = [IO.Path]::GetFullPath($Executable)
        $run = [IO.Path]::GetDirectoryName($saved)
        # Only a direct run subdirectory and this exact executable can be owned.
        return [IO.Path]::GetDirectoryName($run).Equals($root, [StringComparison]::OrdinalIgnoreCase) -and
            [IO.Path]::GetFileName($saved).Equals('sideways.exe', [StringComparison]::OrdinalIgnoreCase) -and
            $saved.Equals($actual, [StringComparison]::OrdinalIgnoreCase) -and
            [int]$Record.pid -eq $ProcessId -and [long]$Record.start_utc_ticks -eq $StartTicks
    } catch { return $false }
}

function Test-VisibleFirstFrame {
    param([string]$Text)
    return $Text -match '(?m)^play: first visible frame complete\r?$' -or
        $Text -match '(?m)^play: first frame complete \(presented=[1-9][0-9]*\)\r?$'
}

function Write-DevJson {
    param([string]$Path, $Value)
    $temporary = $Path + '.tmp'
    [IO.File]::WriteAllText($temporary, ($Value | ConvertTo-Json -Depth 8), [Text.UTF8Encoding]::new($false))
    Move-Item -LiteralPath $temporary -Destination $Path -Force
}

function Get-DlssBuildSetup {
    param([string]$Upscale, [string]$Sdk = $env:NVIDIA_DLSS_SDK,
        [string]$VulkanHeaders = $env:VULKAN_HEADERS, [string]$Runtime = $env:KERN_DLSS_RUNTIME_DIR)
    if ($Upscale -eq 'native') { return [ordered]@{ features = @(); dlss = $null } }
    if ($Upscale -notin @('dlss-quality', 'dlss-balanced', 'dlss-performance')) { throw "Unknown upscaling mode: $Upscale" }
    $paths = [ordered]@{ NVIDIA_DLSS_SDK = $Sdk; VULKAN_HEADERS = $VulkanHeaders; KERN_DLSS_RUNTIME_DIR = $Runtime }
    foreach ($key in $paths.Keys) {
        if ([string]::IsNullOrWhiteSpace($paths[$key]) -or -not (Test-Path -LiteralPath $paths[$key] -PathType Container)) {
            throw "DLSS requested: set $key to its installed directory before building. Native builds do not require the optional SDK/runtime."
        }
    }
    $files = [ordered]@{
        sdk_header = Join-Path $Sdk 'include\nvsdk_ngx_vk.h'
        vulkan_header = Join-Path $VulkanHeaders 'include\vulkan\vulkan.h'
        runtime_library = Join-Path $Runtime 'nvngx_dlss.dll'
    }
    $evidence = [ordered]@{ sdk_root = [IO.Path]::GetFullPath($Sdk); vulkan_headers_root = [IO.Path]::GetFullPath($VulkanHeaders) }
    foreach ($key in $files.Keys) {
        $path = [IO.Path]::GetFullPath($files[$key])
        if (-not (Test-Path -LiteralPath $path -PathType Leaf)) { throw "DLSS prerequisite missing: $path" }
        $evidence[$key] = $path
        $evidence[$key + '_sha256'] = (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant()
    }
    return [ordered]@{ features = @('render/dlss'); dlss = $evidence }
}
