# ForzaInfinity updater for Windows: installs the latest release into an Azimuth folder,
# keeping its server and settings. The installer leaves ForzaInfinity-Update.cmd in the Azimuth
# folder (this script is data\bin\forzainfinity-update.ps1 there): close the game, then double
# click it.
#   update.ps1 [-Folder <folder with azimuth.exe>]
param([string]$Folder = '')
$ErrorActionPreference = 'Stop'
$Release = 'https://github.com/anarqz/ForzaInfinity/releases/latest/download'
$here = if ($PSScriptRoot) { $PSScriptRoot } else { (Get-Location).Path }
function Test-Azimuth([string]$dir) { $dir -and (Test-Path (Join-Path $dir 'azimuth.exe')) }
# the given folder, else the one this script is in (or two up: data\bin)
$azimuth = @($Folder, $here, (Split-Path (Split-Path $here -Parent) -Parent)) | Where-Object { Test-Azimuth $_ } | Select-Object -First 1
$server = ''
if ($azimuth) {
    $ini = Join-Path $azimuth 'data\bin\FH2_ExtraOptions.ini'
    if (Test-Path $ini) {
        $line = Get-Content $ini | Where-Object { $_ -match '^\s*Server\s*=' } | Select-Object -First 1
        if ($line) { $server = ($line -split '=', 2)[1].Trim() }
    }
}
$installer = Join-Path $env:TEMP 'forzainfinity-install.ps1'
Invoke-WebRequest -UseBasicParsing -Uri "$Release/install.ps1" -OutFile $installer
$arguments = @{}
if ($azimuth) { $arguments.Folder = $azimuth }
if ($server) { $arguments.Server = $server }
& $installer @arguments
exit $LASTEXITCODE
