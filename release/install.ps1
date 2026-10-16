# ForzaInfinity installer for Windows: puts the ForzaInfinity azimuth-tls.dll into Azimuth
# (keeping the original as azimuth-tls.dll.stock) and points it at a server.
#   Install.cmd                  double click (asks for the folder and the server)
#   install.ps1 -Folder <dir> -Server host:port|https://host -Quiet
#   irm https://github.com/anarqz/ForzaInfinity/releases/latest/download/install.ps1 | iex
# It needs an Azimuth version the hook was made for ($SupportedAzimuth); -Force installs on
# another one anyway (the game then plays offline). Updating Azimuth puts its own
# azimuth-tls.dll back: run the installer again after each update.
param([string]$Folder = '', [string]$Server = '', [switch]$Quiet, [switch]$Force)
$SupportedAzimuth = @('0.9.7', '0.9.9', '0.9.10')
$SupportedText = '0.9.10 (the latest), 0.9.9 or 0.9.7'
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

# the Azimuth version: the UTF-16 "0.x.y" string in azimuth.exe
$text = [Text.Encoding]::Unicode.GetString([IO.File]::ReadAllBytes((Join-Path $azimuth 'azimuth.exe')))
$version = [regex]::Matches($text, '(?<![0-9.])0\.[0-9]+\.[0-9]+(?![0-9.])') | Group-Object Value | Sort-Object Count -Descending | Select-Object -First 1 -ExpandProperty Name
if (-not $version) { Write-Host "Could not read the Azimuth version; ForzaInfinity needs Azimuth $SupportedText." }
elseif ($SupportedAzimuth -notcontains $version -and -not $Force) {
    Fail "This is Azimuth $version. ForzaInfinity needs Azimuth ${SupportedText}: on $version the game would play offline.`n`nUpdate Azimuth to the latest version and run the installer again, or run install.ps1 -Force to install anyway."
}

# 2. the server
if (-not $Server) {
    $default = if ($DefaultServer -notlike '__*') { $DefaultServer } else { '' }
    $Server = if ($Quiet) { $default } else {
        Use-Dialogs
        [Microsoft.VisualBasic.Interaction]::InputBox('ForzaInfinity server address (host:port or https://host), from whoever runs your server:', 'ForzaInfinity', $default)
    }
}
$Server = $Server.Trim()
if ($Server -notmatch '^(https?://[A-Za-z0-9.\-]+(:[0-9]{1,5})?|[A-Za-z0-9.\-]+:[0-9]{1,5})$') { Fail "The server must look like host:port (for example 192.168.0.10:8081) or https://host, not '$Server'." }

# 3. the DLL (next to this script, else the latest release)
$dll = Join-Path $here 'azimuth-tls.dll'
if (-not (Test-Path $dll)) {
    $dll = Join-Path $env:TEMP 'forzainfinity-azimuth-tls.dll'
    Invoke-WebRequest -UseBasicParsing -Uri "$Release/azimuth-tls.dll" -OutFile $dll
}
$target = Join-Path $bin 'azimuth-tls.dll'
$stock = Join-Path $bin 'azimuth-tls.dll.stock'
# keep Azimuth's own (a few KB; ours is far bigger): the first time, and again after an Azimuth
# update put its own back
if ((Test-Path $target) -and (-not (Test-Path $stock) -or (Get-Item $target).Length -lt 65536)) { Copy-Item $target $stock -Force }
Copy-Item $dll $target -Force
# the free roam world a player hosts (Settings > INFINITY > HOST WORLD): dedicated_server.exe,
# next to this script, else the latest release's
$relay = Join-Path $here 'dedicated_server.exe'
if (-not (Test-Path $relay)) {
    $relay = Join-Path $env:TEMP 'forzainfinity-dedicated_server.exe'
    try { Invoke-WebRequest -UseBasicParsing -Uri "$Release/dedicated_server.exe" -OutFile $relay } catch { $relay = '' }
}
if ($relay) { Copy-Item $relay (Join-Path $bin 'dedicated_server.exe') -Force }

# 4. the settings. FH2_ExtraOptions.ini is the player's: what is in it stays as it is, and only the
# options of our template it lacks are added (with their comments), at the end of their section.
# [Network] is ForzaInfinity's: Mode=Proxy and Server=<the server chosen above>. Before the first
# change, the file as it was is kept as FH2_ExtraOptions.ini.pre-infinity.
$ini = Join-Path $bin 'FH2_ExtraOptions.ini'
$templateFile = Join-Path $here 'FH2_ExtraOptions.template.ini'
$template = if (Test-Path $templateFile) { [IO.File]::ReadAllText($templateFile) } else {
    # GitHub serves release files as application/octet-stream: Content is then bytes
    $content = (Invoke-WebRequest -UseBasicParsing -Uri "$Release/FH2_ExtraOptions.template.ini").Content
    if ($content -is [byte[]]) { [Text.Encoding]::UTF8.GetString($content) } else { $content }
}
$template = $template.Replace('__SERVER__', $Server)
$utf8 = New-Object System.Text.UTF8Encoding($false)
function Strip([string]$t) { ($t -replace '[;#].*$', '').Trim() }
function Section([string]$t) { $n = $t.Substring(1, [Math]::Max(0, $t.IndexOf(']') - 1)).Trim().ToLowerInvariant(); if ($n -eq 'general') { 'global' } else { $n } }
function Key([string]$t) { $t.Substring(0, $t.IndexOf('=')).Trim().ToLowerInvariant() }
if (Test-Path $ini) {
    # the template: its sections in order, and each option with the comments right above it
    $order = New-Object System.Collections.Generic.List[string]; $headers = @{}; $keys = @{}; $blocks = @{}
    $s = ''; $notes = New-Object System.Collections.Generic.List[string]
    foreach ($raw in ($template -split "`r?`n")) {
        $line = Strip $raw
        if (-not $line) { if ($raw -match '^\s*[;#]') { $notes.Add($raw) } else { $notes.Clear() }; continue }
        if ($line.StartsWith('[')) { $s = Section $line; $order.Add($s); $headers[$s] = $line; $keys[$s] = New-Object System.Collections.Generic.List[string]; $notes.Clear(); continue }
        $k = Key $line; $keys[$s].Add($k); $blocks["$s|$k"] = @($notes) + $raw; $notes.Clear()
    }
    # the player's INI: what it has, and where each of its sections ends
    $original = [IO.File]::ReadAllBytes($ini)
    $lines = [IO.File]::ReadAllText($ini).TrimEnd("`r", "`n") -split "`r?`n"
    $seen = @{}; $last = @{}; $has = @{}; $s = ''
    for ($i = 0; $i -lt $lines.Count; $i++) {
        $line = Strip $lines[$i]
        if ($line.StartsWith('[')) { $s = Section $line; $seen[$s] = $true; $last[$s] = $i; continue }
        if (-not $line -or -not $s) { continue }
        if ($line.Contains('=')) {
            $k = Key $line; $has["$s|$k"] = $true
            if ($s -eq 'network' -and $k -eq 'mode') { $lines[$i] = 'Mode=Proxy' }
            if ($s -eq 'network' -and $k -eq 'server') { $lines[$i] = "Server=$Server" }
        }
        $last[$s] = $i
    }
    # an option with its comments; blank line before: 1 always, 0 when it has comments, -1 never
    $out = New-Object System.Collections.Generic.List[string]
    function Put($block, [int]$blank) { if ($blank -gt 0 -or ($blank -eq 0 -and $block.Count -gt 1)) { $out.Add('') }; foreach ($b in $block) { $out.Add($b) } }
    for ($i = 0; $i -lt $lines.Count; $i++) {
        $out.Add($lines[$i])
        foreach ($s in $order) {
            if (-not $seen[$s] -or $last[$s] -ne $i) { continue }
            $first = 1
            foreach ($k in $keys[$s]) { if (-not $has["$s|$k"]) { Put $blocks["$s|$k"] $first; $first = 0 } }
        }
    }
    foreach ($s in $order) {
        if ($seen[$s]) { continue }
        $out.Add(''); $out.Add($headers[$s]); $m = 0
        foreach ($k in $keys[$s]) { Put $blocks["$s|$k"] $(if ($m++ -eq 0) { -1 } else { 0 }) }
    }
    $merged = $utf8.GetBytes(($out -join "`r`n") + "`r`n")
    if ([Convert]::ToBase64String($merged) -ne [Convert]::ToBase64String($original)) {
        if (-not (Test-Path "$ini.pre-infinity")) { [IO.File]::WriteAllBytes("$ini.pre-infinity", $original) }
        [IO.File]::WriteAllBytes($ini, $merged)
    }
} else {
    [IO.File]::WriteAllText($ini, $template, $utf8)
}

# 5. the updater, left in the Azimuth folder: ForzaInfinity-Update.cmd installs the latest release
$updater = Join-Path $bin 'forzainfinity-update.ps1'
try {
    $source = Join-Path $here 'update.ps1'
    if (Test-Path $source) { Copy-Item $source $updater -Force }
    else { Invoke-WebRequest -UseBasicParsing -Uri "$Release/update.ps1" -OutFile $updater }
    $cmd = "@echo off`r`nrem ForzaInfinity: close the game, then double click to update to the latest release.`r`npowershell -NoProfile -ExecutionPolicy Bypass -File `"%~dp0data\bin\forzainfinity-update.ps1`" -Folder `"%~dp0.`"`r`nif errorlevel 1 pause`r`n"
    [IO.File]::WriteAllText((Join-Path $azimuth 'ForzaInfinity-Update.cmd'), $cmd, (New-Object System.Text.ASCIIEncoding))
} catch { Write-Host "The updater could not be set up: $_" }

# 6. the report, left there too: ForzaInfinity-Report.cmd writes the last launch to one file
$reporter = Join-Path $bin 'forzainfinity-report.ps1'
try {
    $source = Join-Path $here 'report.ps1'
    if (Test-Path $source) { Copy-Item $source $reporter -Force }
    else { Invoke-WebRequest -UseBasicParsing -Uri "$Release/report.ps1" -OutFile $reporter }
    $cmd = "@echo off`r`nrem ForzaInfinity: writes ForzaInfinity-Report.txt (the last launch) on the Desktop, to send to whoever helps you.`r`npowershell -NoProfile -ExecutionPolicy Bypass -File `"%~dp0data\bin\forzainfinity-report.ps1`" -Folder `"%~dp0.`"`r`npause`r`n"
    [IO.File]::WriteAllText((Join-Path $azimuth 'ForzaInfinity-Report.cmd'), $cmd, (New-Object System.Text.ASCIIEncoding))
} catch { Write-Host "The report could not be set up: $_" }

Say "ForzaInfinity is installed in $azimuth.`n`nServer: $Server`n`nStart the game, then open Settings > INFINITY to create your profile."
