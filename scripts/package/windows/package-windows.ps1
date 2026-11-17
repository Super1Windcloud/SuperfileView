param (
    [string]$Target = "x86_64-pc-windows-msvc",
    [Parameter(Mandatory = $true)]
    [string]$Tag
)

$ErrorActionPreference = "Stop"

$repoRoot = Resolve-Path (Join-Path $PSScriptRoot "..\..\..")
$versionScript = Join-Path $repoRoot 'scripts\package\release.py'
$cleanVersion = & python $versionScript version $Tag --field version
if ($LASTEXITCODE -ne 0) { throw 'Invalid release tag' }
if ($Target -ne 'x86_64-pc-windows-msvc') { throw 'Only Windows x86_64 packaging is supported' }
$distDir = if ($env:SUPERFILE_DIST_DIR) { $env:SUPERFILE_DIST_DIR } else { Join-Path $repoRoot 'package-dist' }
$sourceExe = if ($env:SUPERFILE_PACKAGE_BINARY) { $env:SUPERFILE_PACKAGE_BINARY } else { Join-Path $repoRoot "target\$Target\release\superfile.exe" }
if (-not (Test-Path $sourceExe -PathType Leaf)) { throw "Missing release binary: $sourceExe" }

New-Item -ItemType Directory -Force -Path $distDir | Out-Null
$stagingDir = Join-Path $repoRoot "target\package-staging\windows-$([guid]::NewGuid().ToString('N'))"
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
& python $versionScript verify $Tag --platform windows-x86_64 --directory $distDir
if ($LASTEXITCODE -ne 0) { throw 'Windows package validation failed' }
# This directory was created by this invocation; no existing staging is removed.
Remove-Item -LiteralPath $stagingDir -Recurse -Force
