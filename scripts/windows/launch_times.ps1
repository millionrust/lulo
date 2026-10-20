# Launch times of the Lulo apps on a real Windows PC (WIN-OS-99), for the
# release builds CI makes (windows-preview.yml's zip, unpacked). CI's own
# budget runs on debug builds and a software renderer
# (launch_smoke.py --launch-budget-ms); this is the PC's number.
#
#   powershell -ExecutionPolicy Bypass -File launch_times.ps1 -Src <unpacked zip folder> -Out <folder>
#
# Run it on the desktop (over SSH, through a scheduled task started with
# /IT): a process started in session 0 has no visible
# window. Each round copies the build to a new folder, so Microsoft Defender
# has cached none of the exes yet: "cold" is that first launch, "warm" the
# launch after it. Each app runs with a private %APPDATA%/%LOCALAPPDATA%.
#
# Columns: before_ms, Start-Process to the process existing (what Windows,
# Defender's scan of an exe it has not cached, holds CreateProcess for);
# frame_ms, process creation to the first frame on screen
# (`first_frame_shown` in RMAC_GPUI_STARTUP_TRACE, `first_present` on older
# builds); total_ms, both; shown_ms, PowerShell's own view of a main window,
# polled every 5 ms. Each app's start-up trace is kept beside summary.csv.
param(
    [Parameter(Mandatory = $true)][string]$Src,
    [Parameter(Mandatory = $true)][string]$Out,
    [int]$Rounds = 2,
    [string[]]$Apps = @('rmac-calculator', 'rmac-notes', 'rmac-text-editor', 'rmac-files',
        'rmac-system-settings', 'rmac-weather', 'rmac-clock', 'rmac-terminal', 'rmac-preview',
        'rmac-mail', 'rmac-calendar', 'rmac-system-monitor'),
    # Preview opens this instead of its Open panel.
    [string]$PreviewFile = ''
)
New-Item -ItemType Directory -Force $Out | Out-Null
$sum = Join-Path $Out 'summary.csv'
'round,kind,app,before_ms,frame_ms,total_ms,shown_ms' | Set-Content $sum
$appProfile = Join-Path $Out 'profile'
New-Item -ItemType Directory -Force (Join-Path $appProfile 'Roaming'), (Join-Path $appProfile 'Local') | Out-Null
$env:APPDATA = Join-Path $appProfile 'Roaming'
$env:LOCALAPPDATA = Join-Path $appProfile 'Local'
$env:RMAC_GPUI_STARTUP_TRACE = '1'

function Phase($file, $name) {
    foreach ($line in (Get-Content $file -ErrorAction SilentlyContinue)) {
        if ($line -match "^gpui_windows startup: $name at ([0-9.]+) ms") { return [double]$matches[1] }
    }
    return $null
}

for ($round = 1; $round -le $Rounds; $round++) {
    $bin = Join-Path $Out "bin$round"
    if (Test-Path $bin) { Remove-Item -Recurse -Force $bin }
    New-Item -ItemType Directory -Force $bin | Out-Null
    foreach ($app in $Apps) { Copy-Item (Join-Path $Src "$app.exe") $bin }
    if (Test-Path (Join-Path $Src 'fonts')) { Copy-Item -Recurse (Join-Path $Src 'fonts') (Join-Path $bin 'fonts') }
    Start-Sleep 5
    foreach ($app in $Apps) {
        foreach ($kind in 'cold', 'warm') {
            $err = Join-Path $Out "$app-$round-$kind.err"
            $exe = Join-Path $bin "$app.exe"
            $started = [DateTime]::Now
            $clock = [Diagnostics.Stopwatch]::StartNew()
            if ($app -eq 'rmac-preview' -and $PreviewFile) {
                $process = Start-Process $exe -ArgumentList @($PreviewFile) -PassThru -RedirectStandardError $err -RedirectStandardOutput (Join-Path $Out 'out.txt')
            } else {
                $process = Start-Process $exe -PassThru -RedirectStandardError $err -RedirectStandardOutput (Join-Path $Out 'out.txt')
            }
            while ($clock.ElapsedMilliseconds -lt 15000) {
                $process.Refresh()
                if ($process.MainWindowHandle -ne 0) { break }
                Start-Sleep -Milliseconds 5
            }
            $shown = $clock.ElapsedMilliseconds
            Start-Sleep 3
            $before = [math]::Round(($process.StartTime - $started).TotalMilliseconds)
            $frame = Phase $err 'first_frame_shown'
            if ($null -eq $frame) { $frame = Phase $err 'first_present' }
            $total = if ($null -ne $frame) { [math]::Round($before + $frame) } else { '' }
            $frameText = if ($null -ne $frame) { [math]::Round($frame) } else { '' }
            "$round,$kind,$app,$before,$frameText,$total,$shown" | Add-Content $sum
            $process.CloseMainWindow() | Out-Null
            Start-Sleep 2
            if (!$process.HasExited) { Stop-Process -Id $process.Id -Force }
            Start-Sleep 2
        }
    }
    Remove-Item -Recurse -Force $bin
}
'done' | Add-Content $sum
