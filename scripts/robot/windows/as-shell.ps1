# Lulo as the shell, tested inside a signed-in session without signing out
# (ADR 0023, "Lulo as the shell"): stop the installed Lulo layer and
# Explorer, start a build's `lulo-session --as-shell` (startup apps not
# started again: LULO_NO_STARTUP_APPS=1), check it (no Explorer, Lulo owns
# Shell_TrayWnd, the work area, other apps' tray icons in the bar, memory
# and idle CPU), then test the watchdog: end lulo-shell twice and expect
# Explorer back on its own. Finally the installed Lulo layer is started
# again; Explorer is always restarted if anything went wrong.
#
# Run it through invoke.ps1. It waits for 2 minutes without input.
param(
    [string]$Build = 'E:\lulo\winpar',
    [string]$Installed = "$env:LOCALAPPDATA\Programs\Lulo",
    [string]$Scratch = 'E:\lulo\robot-scratch'
)
$ErrorActionPreference = 'Continue'
. (Join-Path (Split-Path -Parent $MyInvocation.MyCommand.Path) 'common.ps1')
$out = Join-Path $Scratch ("as-shell\" + (Get-Date).ToString('yyyyMMdd-HHmmss'))
New-Item -ItemType Directory -Force $out | Out-Null
$result = [ordered]@{}
function Note($key, $value) { $result[$key] = $value; "$key = $value" | Add-Content (Join-Path $out 'log.txt') }
function Running($name) { @(Get-Process $name -ErrorAction SilentlyContinue) }
function Private($names) { [math]::Round((@($names | ForEach-Object { Running $_ } | Measure-Object PrivateMemorySize64 -Sum).Sum) / 1MB, 1) }

while ($R::IdleMs() -lt 120000) { Start-Sleep -Seconds 20 }
Note 'explorer_shell_private_mb_before' (Private @('explorer', 'StartMenuExperienceHost', 'SearchHost', 'ShellExperienceHost'))
try {
    & (Join-Path $Installed 'lulo-session.exe') --stop 2>&1 | Out-Null
    Start-Sleep -Seconds 4
    # Explorer's own clean exit (what Ctrl+Shift+right-click ▸ Exit Explorer
    # sends its taskbar): Windows does not start it again, as it does after
    # Explorer is ended.
    [LuloRobot.Native]::PostMessage($R::Taskbar(), 0x5B4, [IntPtr]::Zero, [IntPtr]::Zero) | Out-Null
    $wait = [Diagnostics.Stopwatch]::StartNew()
    while ($wait.ElapsedMilliseconds -lt 10000 -and (Running 'explorer').Count) { Start-Sleep -Milliseconds 200 }
    Note 'explorer_after_stop' ((Running 'explorer').Count)
    if ((Running 'explorer').Count) { throw 'Explorer did not exit; not testing' }
    $env:LULO_SHELL_TRACE = '1'
    $env:LULO_NO_STARTUP_APPS = '1'
    $watch = [Diagnostics.Stopwatch]::StartNew()
    $session = Start-Process (Join-Path $Build 'lulo-session.exe') -ArgumentList '--as-shell' -PassThru -RedirectStandardError (Join-Path $out 'trace.txt')
    $ready = $false
    while ($watch.ElapsedMilliseconds -lt 30000 -and -not $ready) {
        Start-Sleep -Milliseconds 100
        $ready = (Get-Content (Join-Path $out 'trace.txt') -ErrorAction SilentlyContinue) -match 'ready at'
    }
    Note 'ready_ms_from_session_start' $watch.ElapsedMilliseconds
    Start-Sleep -Seconds 8
    $tray = $R::Taskbar()
    $shell = Running 'lulo-shell' | Select-Object -First 1
    Note 'tray_owner' ($(if ($tray -ne [IntPtr]::Zero) { $R::ProcessName($R::PidOf($tray)) } else { 'none' }))
    Note 'explorer_running' ((Running 'explorer').Count)
    Note 'work_area' ("$($R::WorkArea())")
    Note 'screen' ("$($R::Screen())")
    $bar = [IntPtr]::Zero
    if ($shell) {
        foreach ($w in $R::WindowsOf($shell.Id, $false)) { if ($R::Tree($w, 1, 4) | Where-Object { $_.Name -eq 'top bar' }) { $bar = $w; break } }
    }
    $buttons = if ($bar -ne [IntPtr]::Zero) { @($R::Tree($bar, 3, 120) | Where-Object { $_.Type -eq 'Button' } | ForEach-Object { $_.Name }) } else { @() }
    Note 'bar_buttons' ($buttons -join ' | ')
    $R::Shot((Join-Path $out 'as-shell.jpg'))
    $cpu0 = if ($shell) { $shell.TotalProcessorTime.TotalMilliseconds } else { 0 }
    Start-Sleep -Seconds 30
    if ($shell) { $shell.Refresh(); Note 'lulo_shell_cpu_ms_30s' ([math]::Round($shell.TotalProcessorTime.TotalMilliseconds - $cpu0)) }
    Note 'lulo_private_mb' (Private @('lulo-shell', 'lulo-session'))
    Note 'shell_hosts_private_mb_as_shell' (Private @('explorer', 'StartMenuExperienceHost', 'SearchHost', 'ShellExperienceHost'))

    # The watchdog: two failures within two minutes give the desktop to Explorer.
    Stop-Process -Name lulo-shell -Force -ErrorAction SilentlyContinue
    Start-Sleep -Seconds 8
    Note 'restarted_after_first_failure' ((Running 'lulo-shell').Count)
    Stop-Process -Name lulo-shell -Force -ErrorAction SilentlyContinue
    $watch = [Diagnostics.Stopwatch]::StartNew()
    while ($watch.ElapsedMilliseconds -lt 20000 -and -not (Running 'explorer').Count) { Start-Sleep -Milliseconds 200 }
    Note 'explorer_back_ms' ($(if ((Running 'explorer').Count) { $watch.ElapsedMilliseconds } else { -1 }))
    Start-Sleep -Seconds 3
    Note 'session_exited' ($session.HasExited)
    Note 'taskbar_visible' ($R::Visible($R::Taskbar()))
} catch {
    Note 'stopped' "$_"
} finally {
    if (-not (Running 'explorer').Count) { Start-Process explorer.exe; Note 'explorer_started_by_cleanup' $true }
    Start-Sleep -Seconds 6
    Get-Process lulo-shell, lulo-session -ErrorAction SilentlyContinue | Where-Object { $_.Path -like "$Build*" } | Stop-Process -Force
    Start-Process (Join-Path $Installed 'lulo-session.exe')
}
$result | ConvertTo-Json | Set-Content (Join-Path $out 'result.json')
$result | ConvertTo-Json
