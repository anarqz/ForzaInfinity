# Keep Windows awake (system + display) while this runs. Started detached by long unattended
# sessions: powershell -WindowStyle Hidden -File tools\keepawake.ps1 [hours]. Ends by itself.
param([double]$Hours = 12)
Add-Type -Namespace W -Name P -MemberDefinition '[DllImport("kernel32.dll")] public static extern uint SetThreadExecutionState(uint f);'
$deadline = (Get-Date).AddHours($Hours)
while ((Get-Date) -lt $deadline) {
    [W.P]::SetThreadExecutionState([uint32]"0x80000003") | Out-Null
    Start-Sleep -Seconds 60
}
[W.P]::SetThreadExecutionState([uint32]"0x80000000") | Out-Null
