# ForzaInfinity report for Windows: puts what the last launch of the game left (versions,
# settings without tokens, the hook's logs, the last requests) in one file,
# ForzaInfinity-Report.txt on the Desktop, to send to whoever helps you. Nothing is sent
# anywhere. The installer leaves ForzaInfinity-Report.cmd in the Azimuth folder (this script is
# data\bin\forzainfinity-report.ps1 there): double click it.
#   report.ps1 [-Folder <folder with azimuth.exe>]
param([string]$Folder = '')
$ErrorActionPreference = 'Continue'
$here = if ($PSScriptRoot) { $PSScriptRoot } else { (Get-Location).Path }
function Test-Azimuth([string]$dir) { $dir -and (Test-Path (Join-Path $dir 'azimuth.exe')) }
$azimuth = @($Folder, $here, (Split-Path (Split-Path $here -Parent) -Parent), (Split-Path $here -Parent)) | Where-Object { Test-Azimuth $_ } | Select-Object -First 1
if (-not $azimuth) { $azimuth = if ($Folder) { $Folder } else { $here } }
$bin = Join-Path $azimuth 'data\bin'
$desktop = [Environment]::GetFolderPath('Desktop')
if (-not $desktop -or -not (Test-Path $desktop)) { $desktop = $env:USERPROFILE }
$out = Join-Path $desktop 'ForzaInfinity-Report.txt'

function Clean([string]$path, [int]$last = 0) {
    if (-not (Test-Path $path)) { return @('(missing)') }
    $lines = Get-Content $path -ErrorAction SilentlyContinue | ForEach-Object { $_ -replace '(?i)^(\s*(Token|Key|WebKey|Password)\s*=).*', '$1 (removed)' }
    if ($last -gt 0) { $lines | Select-Object -Last $last } else { $lines }
}
function Stamp([string]$path) { if (Test-Path $path) { (Get-Item $path).LastWriteTime.ToString('yyyy-MM-dd HH:mm') } else { '' } }
function AzimuthVersion([string]$exe) {
    if (-not (Test-Path $exe)) { return '' }
    $text = [Text.Encoding]::Unicode.GetString([IO.File]::ReadAllBytes($exe))
    $found = [regex]::Matches($text, '(?<![0-9.])0\.[0-9]+\.[0-9]+(?![0-9.])') | Group-Object Value | Sort-Object Count -Descending | Select-Object -First 1
    if ($found) { $found.Name } else { '' }
}

$report = New-Object System.Collections.Generic.List[string]
$report.Add('ForzaInfinity report')
$report.Add('Made:     ' + (Get-Date).ToUniversalTime().ToString('yyyy-MM-dd HH:mm:ss') + ' UTC')
$report.Add('System:   ' + [Environment]::OSVersion.VersionString + ', PowerShell ' + $PSVersionTable.PSVersion)
$report.Add('Folder:   ' + $azimuth)
if (-not (Test-Azimuth $azimuth)) { $report.Add('azimuth.exe was not found here: run this from the Azimuth folder.') }
$report.Add('Azimuth:  ' + (AzimuthVersion (Join-Path $azimuth 'azimuth.exe')))
$dll = Join-Path $bin 'azimuth-tls.dll'
if (Test-Path $dll) {
    $ascii = [Text.Encoding]::ASCII.GetString([IO.File]::ReadAllBytes($dll))
    $hook = [regex]::Match($ascii, '0\.2\.[0-9]+\+fh2online\.[0-9]+').Value
    $report.Add('Hook:     ' + $hook + ' sha256 ' + (Get-FileHash $dll -Algorithm SHA256).Hash.ToLower())
} else { $report.Add('Hook:     (no data\bin\azimuth-tls.dll)') }
$report.Add('Running:  ' + $(if (Get-Process azimuth -ErrorAction SilentlyContinue) { 'yes' } else { 'no' }))
$log = Join-Path $bin 'FH2_ExtraOptions.log'
$previous = Join-Path $bin 'FH2_ExtraOptions.previous.log'
$report.Add(''); $report.Add("===== FH2_ExtraOptions.log (last launch, $(Stamp $log)) =====")
Clean $log 3000 | ForEach-Object { $report.Add($_) }
$report.Add(''); $report.Add("===== FH2_ExtraOptions.previous.log (the launch before, $(Stamp $previous)) =====")
Clean $previous 3000 | ForEach-Object { $report.Add($_) }
$report.Add(''); $report.Add('===== FH2_ExtraOptions.ini =====')
Clean (Join-Path $bin 'FH2_ExtraOptions.ini') | ForEach-Object { $report.Add($_) }
$report.Add(''); $report.Add('===== FH2Online_Accounts.ini =====')
Clean (Join-Path $bin 'FH2Online_Accounts.ini') | ForEach-Object { $report.Add($_) }
$report.Add(''); $report.Add('===== Free roam world: dedicated_server.net (what the last hosted world found about the network) =====')
Clean (Join-Path $bin 'dedicated_server.net') | ForEach-Object { $report.Add($_) }
$report.Add(''); $report.Add('===== Free roam world: server.cfg =====')
Clean (Join-Path $bin 'server.cfg') | ForEach-Object { $report.Add($_) }
$report.Add(''); $report.Add('===== Free roam world: dedicated_server.log (last 300 lines) =====')
Clean (Join-Path $bin 'dedicated_server.log') 300 | ForEach-Object { $report.Add($_) }
$report.Add(''); $report.Add('===== Last requests (FH2_Network.jsonl: method, address, result) =====')
$network = Join-Path $bin 'FH2_Network.jsonl'
if (Test-Path $network) {
    Get-Content $network -Tail 40 -ErrorAction SilentlyContinue | ForEach-Object {
        try { $d = $_ | ConvertFrom-Json; $report.Add("$($d.id) $($d.tick_ms) $($d.method) $($d.url) $($d.status) $($d.outcome)") } catch { }
    }
} else { $report.Add('(none)') }
[IO.File]::WriteAllLines($out, $report, (New-Object System.Text.UTF8Encoding $false))
Write-Host "Report written to $out"
Write-Host 'Send that file to whoever helps you (it has no tokens or keys).'
