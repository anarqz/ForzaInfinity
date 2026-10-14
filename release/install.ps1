# ForzaInfinity installer for Windows: puts the ForzaInfinity azimuth-tls.dll into Azimuth
# (keeping the original as azimuth-tls.dll.stock) and points it at a server.
#   Install.cmd                  double click (asks for the folder and the server)
#   install.ps1 -Folder <dir> -Server host:port -Quiet
#   irm https://github.com/anarqz/ForzaInfinity/releases/latest/download/install.ps1 | iex
param([string]$Folder = '', [string]$Server = '', [switch]$Quiet)
$ErrorActionPreference = 'Stop'
$DefaultServer = '__DEFAULT_SERVER__'
$Release = 'https://github.com/anarqz/ForzaInfinity/releases/latest/download'
# the dialogs are loaded only when shown, so -Quiet runs with no UI at all
function Use-Dialogs { Add-Type -AssemblyName System.Windows.Forms; Add-Type -AssemblyName Microsoft.VisualBasic }
$here = if ($PSScriptRoot) { $PSScriptRoot } else { (Get-Location).Path }

function Say([string]$text, [string]$icon = 'Information') {
    if ($Quiet) { Write-Host $text } else {
        Use-Dialogs
        [void][System.Windows.Forms.MessageBox]::Show($text, 'ForzaInfinity', 'OK', $icon)
    }
}
function Fail([string]$text) { Say $text 'Error'; exit 1 }

# 1. the Azimuth folder: given, next to this script, or found in the usual places
function Get-Bin([string]$dir) { Join-Path (Join-Path $dir 'data') 'bin' }
function Test-Azimuth([string]$dir) { $dir -and (Test-Path (Join-Path $dir 'azimuth.exe')) -and (Test-Path (Get-Bin $dir)) }
$azimuth = $null
foreach ($dir in @($Folder, $here, (Split-Path $here -Parent))) { if (Test-Azimuth $dir) { $azimuth = $dir; break } }
if (-not $azimuth) {
    $roots = @('Downloads', 'Desktop', 'Documents', 'Games' | ForEach-Object { if ($env:USERPROFILE) { Join-Path $env:USERPROFILE $_ } }) + @('C:\Games', 'D:\Games') |
        Where-Object { Test-Path $_ }
    $found = foreach ($root in $roots) {
        Get-ChildItem -Path $root -Filter 'azimuth.exe' -File -Recurse -Depth 5 -ErrorAction SilentlyContinue |
            ForEach-Object { $_.DirectoryName } | Where-Object { Test-Azimuth $_ }
    }
    $azimuth = $found | Select-Object -First 1
}
if (-not $azimuth -and -not $Quiet) {
    Use-Dialogs
    $dialog = New-Object System.Windows.Forms.FolderBrowserDialog
    $dialog.Description = 'Choose the Azimuth folder (the one with azimuth.exe)'
    if ($dialog.ShowDialog() -eq 'OK' -and (Test-Azimuth $dialog.SelectedPath)) { $azimuth = $dialog.SelectedPath }
}
if (-not $azimuth) { Fail 'Azimuth was not found. Run Install.cmd from the Azimuth folder, or pick the folder that has azimuth.exe.' }
$bin = Get-Bin $azimuth

if (Get-Process -Name 'azimuth' -ErrorAction SilentlyContinue) { Fail 'Close Forza Horizon 2 (Azimuth) first, then run the installer again.' }

# 2. the server
if (-not $Server) {
    $default = if ($DefaultServer -notlike '__*') { $DefaultServer } else { '' }
    $Server = if ($Quiet) { $default } else {
        Use-Dialogs
        [Microsoft.VisualBasic.Interaction]::InputBox('ForzaInfinity server address (host:port), from whoever runs your server:', 'ForzaInfinity', $default)
    }
}
$Server = $Server.Trim()
if ($Server -notmatch '^[A-Za-z0-9.\-]+:[0-9]{1,5}$') { Fail "The server must look like host:port (for example 192.168.0.10:8081), not '$Server'." }

# 3. the DLL (next to this script, else the latest release)
$dll = Join-Path $here 'azimuth-tls.dll'
if (-not (Test-Path $dll)) {
    $dll = Join-Path $env:TEMP 'forzainfinity-azimuth-tls.dll'
    Invoke-WebRequest -UseBasicParsing -Uri "$Release/azimuth-tls.dll" -OutFile $dll
}
$target = Join-Path $bin 'azimuth-tls.dll'
$stock = Join-Path $bin 'azimuth-tls.dll.stock'
if ((Test-Path $target) -and -not (Test-Path $stock)) { Copy-Item $target $stock }
Copy-Item $dll $target -Force

# 4. the settings: [Network] Mode=Proxy and Server=..., keeping everything else
$ini = Join-Path $bin 'FH2_ExtraOptions.ini'
if (Test-Path $ini) {
    $lines = [System.Collections.Generic.List[string]]([IO.File]::ReadAllLines($ini))
    $start = $lines.FindIndex({ param($l) $l.Trim() -eq '[Network]' })
    if ($start -lt 0) {
        $lines.Add(''); $lines.Add('[Network]'); $lines.Add('Mode=Proxy'); $lines.Add("Server=$Server")
    } else {
        $end = $start + 1
        while ($end -lt $lines.Count -and -not $lines[$end].Trim().StartsWith('[')) { $end++ }
        $mode = $false; $srv = $false
        for ($i = $start + 1; $i -lt $end; $i++) {
            if ($lines[$i] -match '^\s*Mode\s*=') { $lines[$i] = 'Mode=Proxy'; $mode = $true }
            elseif ($lines[$i] -match '^\s*Server\s*=') { $lines[$i] = "Server=$Server"; $srv = $true }
        }
        if (-not $srv) { $lines.Insert($end, "Server=$Server") }
        if (-not $mode) { $lines.Insert($start + 1, 'Mode=Proxy') }
    }
    [IO.File]::WriteAllText($ini, (($lines -join "`r`n") + "`r`n"), (New-Object System.Text.ASCIIEncoding))
} else {
    $template = Join-Path $here 'FH2_ExtraOptions.ini'
    $text = if (Test-Path $template) { [IO.File]::ReadAllText($template) } else {
        (Invoke-WebRequest -UseBasicParsing -Uri "$Release/FH2_ExtraOptions.ini").Content
    }
    [IO.File]::WriteAllText($ini, $text.Replace('__SERVER__', $Server), (New-Object System.Text.ASCIIEncoding))
}

Say "ForzaInfinity is installed in $azimuth.`n`nServer: $Server`n`nStart the game, then open Settings > INFINITY to create your profile."
