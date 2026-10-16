# ForzaInfinity uninstaller for Windows: puts back Azimuth's original azimuth-tls.dll and removes
# the free roam relay (dedicated_server.exe; its server.cfg is the player's and stays).
param([string]$Folder = '', [switch]$Quiet)
$ErrorActionPreference = 'Stop'
$here = if ($PSScriptRoot) { $PSScriptRoot } else { (Get-Location).Path }
function Say([string]$text, [string]$icon = 'Information') {
    if ($Quiet) { Write-Host $text; return }
    Add-Type -AssemblyName System.Windows.Forms
    [void][System.Windows.Forms.MessageBox]::Show($text, 'ForzaInfinity', 'OK', $icon)
}
function Get-Bin([string]$dir) { Join-Path (Join-Path $dir 'data') 'bin' }

$azimuth = @($Folder, $here, (Split-Path $here -Parent)) | Where-Object { $_ -and (Test-Path (Join-Path (Get-Bin $_) 'azimuth-tls.dll.stock')) } | Select-Object -First 1
if (-not $azimuth -and -not $Quiet) {
    Add-Type -AssemblyName System.Windows.Forms
    $dialog = New-Object System.Windows.Forms.FolderBrowserDialog
    $dialog.Description = 'Choose the Azimuth folder (the one with azimuth.exe)'
    if ($dialog.ShowDialog() -eq 'OK') { $azimuth = $dialog.SelectedPath }
}
$stock = if ($azimuth) { Join-Path (Get-Bin $azimuth) 'azimuth-tls.dll.stock' } else { '' }
if (-not $stock -or -not (Test-Path $stock)) { Say 'No backup (azimuth-tls.dll.stock) found in that folder.' 'Error'; exit 1 }
if (Get-Process -Name 'azimuth' -ErrorAction SilentlyContinue) { Say 'Close Forza Horizon 2 (Azimuth) first.' 'Error'; exit 1 }
Copy-Item $stock (Join-Path (Get-Bin $azimuth) 'azimuth-tls.dll') -Force
Remove-Item (Join-Path $azimuth 'ForzaInfinity-Update.cmd'), (Join-Path (Get-Bin $azimuth) 'forzainfinity-update.ps1'), (Join-Path $azimuth 'ForzaInfinity-Report.cmd'), (Join-Path (Get-Bin $azimuth) 'forzainfinity-report.ps1') -ErrorAction SilentlyContinue
$relayFiles = 'dedicated_server.exe', 'dedicated_server.exe.previous', 'dedicated_server.exe.old', 'dedicated_server.net', 'dedicated_server.log'
Remove-Item ($relayFiles | ForEach-Object { Join-Path (Get-Bin $azimuth) $_ }) -ErrorAction SilentlyContinue
Say "Azimuth's original azimuth-tls.dll is back. Your saves, FH2_ExtraOptions.ini and server.cfg were left as they are."
