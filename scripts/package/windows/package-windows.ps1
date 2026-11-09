param (
    [string]$Target = "x86_64-pc-windows-msvc",
    [string]$Tag = "v0.0.2"
)

$ErrorActionPreference = "Stop"

$cleanVersion = $Tag.TrimStart('v')
if ([string]::IsNullOrWhiteSpace($cleanVersion)) {
    $cleanVersion = "0.0.2"
}

$repoRoot = Resolve-Path (Join-Path $PSScriptRoot "..\..\..")
$distDir = Join-Path $repoRoot "package-dist"
$sourceCandidates = @(
    (Join-Path $repoRoot "target\$Target\release\superfile.exe"),
    (Join-Path $repoRoot "target\release\superfile.exe"),
    (Join-Path $repoRoot "target\$Target\debug\superfile.exe"),
    (Join-Path $repoRoot "target\debug\superfile.exe")
)

$sourceExe = $null
foreach ($cand in $sourceCandidates) {
    if (Test-Path $cand) {
        $sourceExe = $cand
        break
    }
}

if (-not $sourceExe) {
    Write-Error "Cannot find superfile.exe in target directories"
}

New-Item -ItemType Directory -Force -Path $distDir | Out-Null
$stagingDir = Join-Path $repoRoot "staging\superfile"
New-Item -ItemType Directory -Force -Path $stagingDir | Out-Null

Copy-Item $sourceExe (Join-Path $stagingDir "superfile.exe") -Force
$readme = Join-Path $repoRoot "README.md"
if (Test-Path $readme) { Copy-Item $readme $stagingDir -Force }
$license = Join-Path $repoRoot "LICENSE"
if (Test-Path $license) { Copy-Item $license $stagingDir -Force }

# 1. Create Portable Zip
$zipName = "superfile-${Tag}-windows-x86_64.zip"
$zipPath = Join-Path $distDir $zipName
if (Test-Path $zipPath) { Remove-Item $zipPath -Force }
Write-Host "Creating portable archive: $zipPath"
Compress-Archive -Path "$stagingDir\*" -DestinationPath $zipPath -Force

# 2. Compile the NSIS installer. Missing makensis is a failure: tag releases
# must publish superfile-<tag>-windows-x86_64-setup.exe next to the zip.
$makensisPath = $null
if ($env:NSIS_HOME) {
    $candidate = Join-Path $env:NSIS_HOME "makensis.exe"
    if (Test-Path $candidate) { $makensisPath = $candidate }
}
if (-not $makensisPath) {
    foreach ($root in @(${env:ProgramFiles}, ${env:ProgramFiles(x86)})) {
        if (-not $root) { continue }
        $candidate = Join-Path $root "NSIS\makensis.exe"
        if (Test-Path $candidate) {
            $makensisPath = $candidate
            break
        }
    }
}
if (-not $makensisPath) {
    $makensisPath = (Get-Command makensis.exe -ErrorAction SilentlyContinue)?.Source
}
if (-not $makensisPath) {
    Write-Error "makensis.exe not found. Install NSIS 3 before packaging the Windows setup executable."
}

function Get-VersionQuad([string]$version) {
    $core = ($version -split '[-+]', 2)[0]
    $nums = @()
    foreach ($part in ($core -split '\.')) {
        if ($part -match '^[0-9]+$') {
            $nums += [string]([int]$part)
        } else {
            break
        }
    }
    if ($nums.Count -eq 0) { $nums = @('0') }
    while ($nums.Count -lt 4) { $nums += '0' }
    return ($nums[0..3] -join '.')
}

$versionQuad = Get-VersionQuad $cleanVersion
$nsiFile = Join-Path $PSScriptRoot "installer.nsi"
$setupBase = "superfile-${Tag}-windows-x86_64-setup"
$setupPath = Join-Path $distDir "$setupBase.exe"
if (Test-Path $setupPath) { Remove-Item $setupPath -Force }

Write-Host "Compiling NSIS installer with $makensisPath"
& $makensisPath `
    "/WX" `
    "/INPUTCHARSET" "UTF8" `
    "/DAPP_VERSION=$cleanVersion" `
    "/DAPP_VERSION_QUAD=$versionQuad" `
    "/DSOURCE_EXE=$sourceExe" `
    "/DSOURCE_README=$readme" `
    "/DSOURCE_LICENSE=$license" `
    "/DOUTPUT_DIR=$distDir" `
    "/DOUTPUT_FILENAME=$setupBase" `
    $nsiFile
if ($LASTEXITCODE -ne 0) {
    Write-Error "NSIS compilation failed with exit code $LASTEXITCODE"
}
if (-not (Test-Path $setupPath)) {
    Write-Error "NSIS did not create $setupPath"
}
Write-Host "NSIS installer created: $setupPath"

Write-Host "Windows packaging completed. Dist files:"
Get-ChildItem -Path $distDir
