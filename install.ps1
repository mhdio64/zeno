# zeno — installer for Windows (PowerShell)
# Repository: https://github.com/mhdio64/zeno
# Usage: irm https://raw.githubusercontent.com/mhdio64/zeno/main/install.ps1 | iex

$ErrorActionPreference = "Stop"

$Repo = "mhdio64/zeno"
$Asset = "zeno-windows-x86_64.zip"
$Url = "https://github.com/$Repo/releases/latest/download/$Asset"

Write-Host @"
    ███████╗███████╗███╗   ██╗ ██████╗ 
    ╚══███╔╝██╔════╝████╗  ██║██╔═══██╗
      ███╔╝ █████╗  ██╔██╗ ██║██║   ██║
     ███╔╝  ██╔══╝  ██║╚██╗██║██║   ██║
    ███████╗███████╗██║ ╚████║╚██████╔╝
    ╚══════╝╚══════╝╚═╝  ╚═══╝ ╚═════╝ 
"@ -ForegroundColor Cyan

Write-Host "`nFast, non-modal terminal text editor" -ForegroundColor White
Write-Host "Downloading latest zeno release for Windows..." -ForegroundColor Yellow

$InstallDir = "$env:LOCALAPPDATA\Programs\zeno"
if (-not (Test-Path -Path $InstallDir)) {
    New-Item -ItemType Directory -Path $InstallDir -Force | Out-Null
}

$TempZip = Join-Path $env:TEMP $Asset
$TempExtract = Join-Path $env:TEMP "zeno-extract-$(Get-Random)"

try {
    [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
    Invoke-WebRequest -Uri $Url -OutFile $TempZip -UseBasicParsing

    Write-Host "Extracting archive..." -ForegroundColor Yellow
    Expand-Archive -Path $TempZip -DestinationPath $TempExtract -Force

    $SourceExe = Join-Path $TempExtract "zeno.exe"
    if (-not (Test-Path -Path $SourceExe)) {
        # Check subdirectories if zipped with root folder
        $Found = Get-ChildItem -Path $TempExtract -Filter "zeno.exe" -Recurse | Select-Object -First 1
        if ($Found) { $SourceExe = $Found.FullName }
    }

    Copy-Item -Path $SourceExe -Destination (Join-Path $InstallDir "zeno.exe") -Force

    # Ensure $InstallDir is in User PATH
    $UserPath = [Environment]::GetEnvironmentVariable("Path", [EnvironmentVariableTarget]::User)
    $Paths = $UserPath -split ";"
    if ($Paths -notcontains $InstallDir) {
        $NewPath = "$UserPath;$InstallDir"
        [Environment]::SetEnvironmentVariable("Path", $NewPath, [EnvironmentVariableTarget]::User)
        $env:Path = "$env:Path;$InstallDir"
        Write-Host "Added $InstallDir to user PATH." -ForegroundColor Gray
    }

    Write-Host "`n✔ zeno successfully installed to $InstallDir\zeno.exe" -ForegroundColor Green
    Write-Host "Restart your terminal or run directly with: zeno <filename>`n" -ForegroundColor White
}
finally {
    if (Test-Path $TempZip) { Remove-Item $TempZip -Force -ErrorAction SilentlyContinue }
    if (Test-Path $TempExtract) { Remove-Item $TempExtract -Recurse -Force -ErrorAction SilentlyContinue }
}
