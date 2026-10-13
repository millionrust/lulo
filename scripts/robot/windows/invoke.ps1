# Run a robot script on the signed-in user's desktop, from an SSH session.
#
#   powershell -File invoke.ps1 -Script E:\lulo\robot-scratch\robot\robot.ps1 [-Arguments '-Apps calculator'] [-TimeoutSeconds 660]
#
# A process started over SSH runs in session 0, where nothing is visible
# and SendInput reaches nobody. This registers a one-off scheduled task
# with an Interactive logon for the signed-in user (no password stored),
# starts it, waits for it to end (or kills it after the timeout), prints
# its log and removes the task again.
param(
    [Parameter(Mandatory = $true)][string]$Script,
    [string]$Arguments = '',
    [int]$TimeoutSeconds = 660,
    [string]$TaskName = 'LuloRobot'
)
$ErrorActionPreference = 'Stop'
$user = (Get-CimInstance Win32_ComputerSystem).UserName
if (-not $user) { throw 'nobody is signed in: the robot needs a desktop' }
$log = [IO.Path]::ChangeExtension($Script, '.log')
Remove-Item $log -ErrorAction SilentlyContinue
$argLine = "-NoProfile -ExecutionPolicy Bypass -WindowStyle Hidden -Command `"& '$Script' $Arguments *>&1 | Out-File -Encoding utf8 '$log'`""
$action = New-ScheduledTaskAction -Execute 'powershell.exe' -Argument $argLine
$principal = New-ScheduledTaskPrincipal -UserId $user -LogonType Interactive -RunLevel Limited
$settings = New-ScheduledTaskSettingsSet -ExecutionTimeLimit (New-TimeSpan -Seconds ($TimeoutSeconds + 60)) -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries
Register-ScheduledTask -TaskName $TaskName -Action $action -Principal $principal -Settings $settings -Force | Out-Null
try {
    Start-ScheduledTask -TaskName $TaskName
    $deadline = (Get-Date).AddSeconds($TimeoutSeconds)
    Start-Sleep -Seconds 2
    while ((Get-ScheduledTask -TaskName $TaskName).State -eq 'Running') {
        if ((Get-Date) -gt $deadline) {
            Stop-ScheduledTask -TaskName $TaskName
            Write-Output "robot: timed out after $TimeoutSeconds s"
            break
        }
        Start-Sleep -Seconds 2
    }
} finally {
    Unregister-ScheduledTask -TaskName $TaskName -Confirm:$false
}
if (Test-Path $log) { Get-Content $log }
