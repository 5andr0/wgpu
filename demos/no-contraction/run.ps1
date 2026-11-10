param(
    [ValidateSet('Verify', 'Cpu', 'GpuPlan', 'GpuRun')]
    [string]$Mode = 'Cpu',
    [ValidateSet('lost', 'preserved')]
    [string]$Expectation = 'lost',
    [string]$OutputDirectory = ''
)
$ErrorActionPreference = 'Stop'
$fixtureRoot = Join-Path $PSScriptRoot 'fixtures'
$manifest = Get-Content -Raw -LiteralPath (Join-Path $fixtureRoot 'manifest.json') | ConvertFrom-Json
if ($manifest.schema -ne 'no-contraction-precompiled-v1') { throw 'Unknown fixture manifest schema' }
foreach ($file in $manifest.files) {
    $path = Join-Path $fixtureRoot $file.name
    if ((Get-Item -LiteralPath $path).Length -ne $file.bytes -or
        (Get-FileHash -Algorithm SHA256 -LiteralPath $path).Hash.ToLowerInvariant() -ne $file.sha256) {
        throw "Fixture does not match manifest: $($file.name)"
    }
}
if ($Mode -eq 'Verify') {
    Write-Output 'Verified six precompiled shaders and the 16-byte input; no GPU or compiler used.'
    return
}
if (-not $OutputDirectory) {
    $OutputDirectory = Join-Path $PSScriptRoot ('results-' + [DateTime]::UtcNow.ToString('yyyyMMddTHHmmssfffffffZ'))
}
$OutputDirectory = [IO.Path]::GetFullPath($OutputDirectory)
if (Test-Path -LiteralPath $OutputDirectory) { throw 'Output directory must not exist' }
if ($Mode -eq 'Cpu') {
    $arguments = @('run', '--locked', '--manifest-path', (Join-Path $PSScriptRoot 'Cargo.toml'), '--',
        (Join-Path $fixtureRoot 'original-controlled.spv'),
        (Join-Path $fixtureRoot 'original-unmarked.spv'), $OutputDirectory, $Expectation)
} else {
    $operation = if ($Mode -eq 'GpuRun') { 'gpu-run' } else { 'gpu-plan' }
    $arguments = @('run', '--locked', '--manifest-path', (Join-Path $PSScriptRoot 'native/Cargo.toml'), '--',
        $operation, (Join-Path $fixtureRoot 'input.bin'))
    foreach ($name in @('original-controlled', 'original-unmarked', 'old-controlled', 'old-unmarked', 'fixed-controlled', 'fixed-unmarked')) {
        $arguments += Join-Path $fixtureRoot ($name + '.spv')
    }
    $arguments += $OutputDirectory
}
& cargo @arguments
if ($LASTEXITCODE -ne 0) { throw "Demo exited with status $LASTEXITCODE" }
