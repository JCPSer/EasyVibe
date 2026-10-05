$ErrorActionPreference = 'Stop'
$repoRoot = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..')).Path
$tauriRoot = Join-Path $repoRoot 'easyvibe-desktop\src-tauri'

function Invoke-Build([string] $directory, [string] $command, [string[]] $arguments) {
    Push-Location -LiteralPath $directory
    try {
        & $command @arguments
        if ($LASTEXITCODE -ne 0) { throw "$command failed with exit code $LASTEXITCODE" }
    } finally {
        Pop-Location
    }
}

Invoke-Build (Join-Path $repoRoot 'easyvibe-renderer') 'npm.cmd' @('run', 'build')
Invoke-Build (Join-Path $repoRoot 'easyvibe-backend') 'cargo' @('build', '-p', 'easyvibe-app', '--release', '--locked')

$target = (& rustc -vV | Select-String '^host: (.+)$').Matches.Groups[1].Value
if ($LASTEXITCODE -ne 0 -or -not $target) { throw 'Cannot determine the Rust host target' }
$resources = Join-Path $tauriRoot 'resources'
$dist = Join-Path $resources 'dist'
if (Test-Path -LiteralPath $dist) {
    $resolvedDist = (Resolve-Path -LiteralPath $dist).Path
    $expectedDist = [IO.Path]::GetFullPath((Join-Path $repoRoot 'easyvibe-desktop\src-tauri\resources\dist'))
    if ($resolvedDist -ne $expectedDist -or (Get-Item -LiteralPath $dist).Attributes -band [IO.FileAttributes]::ReparsePoint) {
        throw "Refusing to replace unexpected resources directory: $resolvedDist"
    }
    Remove-Item -LiteralPath $resolvedDist -Recurse -Force
}
New-Item -ItemType Directory -Path $resources -Force | Out-Null
Copy-Item -LiteralPath (Join-Path $repoRoot 'easyvibe-renderer\dist') -Destination $dist -Recurse
$prompts = Join-Path $resources 'prompts'
New-Item -ItemType Directory -Path $prompts -Force | Out-Null
foreach ($name in @('easyvibe-map-prompt-v2.2.md', 'easyvibe-map-patrol-prompt.md', 'easyvibe-map-schema-v1.json', 'easyvibe-module-submap-prompt.md')) {
    Copy-Item -LiteralPath (Join-Path $repoRoot $name) -Destination (Join-Path $prompts $name) -Force
}
$binaries = Join-Path $tauriRoot 'binaries'
New-Item -ItemType Directory -Path $binaries -Force | Out-Null
Copy-Item -LiteralPath (Join-Path $repoRoot 'easyvibe-backend\target\release\easyvibe-backend.exe') -Destination (Join-Path $binaries "easyvibe-backend-$target.exe") -Force
Write-Host "Windows desktop resources ready ($target)"
