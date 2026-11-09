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

# 2. Compile Inno Setup installer if iscc is found
$isccPath = "C:\Program Files (x86)\Inno Setup 6\ISCC.exe"
if (-not (Test-Path $isccPath)) {
    $isccPath = (Get-Command iscc.exe -ErrorAction SilentlyContinue)?.Source
}

if ($isccPath -and (Test-Path $isccPath)) {
    Write-Host "Compiling Inno Setup installer using $isccPath..."
    $issFile = Join-Path $PSScriptRoot "installer.iss"
    $setupBase = "superfile-${Tag}-windows-x86_64-setup"
    & $isccPath "/DAppVersion=$cleanVersion" "/DSourceExe=$sourceExe" "/DOutputDir=$distDir" "/DOutputFileName=$setupBase" $issFile
    if ($LASTEXITCODE -ne 0) {
        Write-Warning "Inno Setup compilation returned exit code $LASTEXITCODE"
    } else {
        Write-Host "Inno Setup installer created successfully."
    }
} else {
    Write-Host "ISCC.exe not found. Skipping .exe installer generation."
}

Write-Host "Windows packaging completed. Dist files:"
Get-ChildItem -Path $distDir
