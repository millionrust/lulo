# Lulo against Windows' own shell, measured on a real PC (the owner's goal:
# Lulo mode should feel faster than Windows). Run it through invoke.ps1:
#
#   invoke.ps1 -Script <dir>\speed.ps1 [-Arguments '-Bin <dir>']
#
# What it measures, each a median of three:
# - the shells' standing cost: private memory, working set and CPU over
#   30 s with no input, for Lulo (lulo-shell, lulo-session) and for
#   Explorer's shell processes (explorer, Start, Search, the shell
#   experience host);
# - Spotlight: the hotkey to its panel on screen;
# - Control Centre: a click on its bar item to its panel on screen;
# - Mission Control: Ctrl+Up to its overlay on screen;
# - a folder window: Lulo Files and File Explorer opening the same scratch
#   folder, launch to window on screen.
#
# Windows' Start menu and search are never opened: they show the PC
# owner's own apps and files. The same safety rules as robot.ps1 apply: it
# waits for 2 minutes without input, stops on real input, sends keys only
# while Lulo is in front, clicks only Lulo, opens only its scratch folder,
# and closes every window it opened.
param(
    [string]$Bin = "$env:LOCALAPPDATA\Programs\Lulo",
    [string]$Scratch = 'E:\lulo\robot-scratch',
    [int]$Runs = 3,
    [int]$IdleWaitMinutes = 10
)
$ErrorActionPreference = 'Continue'
. (Join-Path (Split-Path -Parent $MyInvocation.MyCommand.Path) 'common.ps1')

$stamp = (Get-Date).ToString('yyyyMMdd-HHmmss')
$out = Join-Path $Scratch "speed\$stamp"
$folder = Join-Path $Scratch 'speed-folder'
New-Item -ItemType Directory -Force $out, $folder | Out-Null
Set-Content -Encoding UTF8 (Join-Path $folder 'speed note.txt') 'Lulo speed check.'
$result = [ordered]@{ started = (Get-Date).ToString('s'); bin = $Bin }

$script:lastOwnInput = Get-Date
function Mark { $script:lastOwnInput = Get-Date }
function Guard {
    $ownAgo = ((Get-Date) - $script:lastOwnInput).TotalMilliseconds
    if ($R::IdleMs() + 1500 -lt $ownAgo) { throw 'speed: real input detected, stopping' }
}
function Shell-Pid { $p = Get-Process lulo-shell -ErrorAction SilentlyContinue | Select-Object -First 1; if ($p) { $p.Id } else { 0 } }
function Median([double[]]$values) {
    $v = @($values | Where-Object { $_ -ge 0 } | Sort-Object)
    if (-not $v.Count) { return $null }
    return $v[[int][math]::Floor(($v.Count - 1) / 2)]
}
# Visible windows of the shell, by handle.
function Shell-Windows { $id = Shell-Pid; if (-not $id) { return @() }; return @($R::WindowsOf($id, $false)) }
# Milliseconds from `action` until a new visible shell window passes `test`
# (-1 after 3 s).
function Time-NewShellWindow([scriptblock]$action, [scriptblock]$test) {
    $before = @(Shell-Windows)
    $watch = [Diagnostics.Stopwatch]::StartNew()
    & $action
    while ($watch.ElapsedMilliseconds -lt 3000) {
        foreach ($w in Shell-Windows) {
            if ($before -notcontains $w -and (& $test $w)) { return [double]$watch.ElapsedMilliseconds }
        }
        Start-Sleep -Milliseconds 5
    }
    return -1
}
function Escape { [void](Keys-ToLulo @($VK.Esc)); Start-Sleep -Milliseconds 400 }
# Put Lulo in front by clicking bare desktop (Lulo's own desktop window).
function Front-Lulo {
    Guard
    $id = Shell-Pid
    $screen = $R::Screen()
    foreach ($p in @(@(($screen.Width / 2), ($screen.Height / 2)), @(40, ($screen.Height / 2)), @(($screen.Width - 40), ($screen.Height / 2)))) {
        if ($R::PidAt([int]$p[0], [int]$p[1]) -eq $id) { $R::Click([int]$p[0], [int]$p[1]); Mark; Start-Sleep -Milliseconds 300; return $true }
    }
    return $false
}
function Keys-ToLulo([int[]]$keys) {
    Guard
    if ($R::PidOf($R::Foreground()) -ne (Shell-Pid)) { if (-not (Front-Lulo)) { return $false } }
    if ($R::PidOf($R::Foreground()) -ne (Shell-Pid)) { return $false }
    $R::Chord($keys); Mark
    return $true
}

# ---- Standing cost ----------------------------------------------------------

$names = 'lulo-shell', 'lulo-session', 'explorer', 'StartMenuExperienceHost', 'SearchHost', 'ShellExperienceHost'
function Snap {
    $h = @{}
    foreach ($p in Get-Process -ErrorAction SilentlyContinue | Where-Object { $names -contains $_.ProcessName }) {
        $k = $p.ProcessName
        if (-not $h[$k]) { $h[$k] = @{ cpu = 0.0; priv = 0; ws = 0 } }
        $h[$k].cpu += $p.TotalProcessorTime.TotalMilliseconds; $h[$k].priv += $p.PrivateMemorySize64; $h[$k].ws += $p.WorkingSet64
    }
    return $h
}
$a = Snap; Start-Sleep -Seconds 30; $b = Snap
$cost = [ordered]@{}
foreach ($k in $names) {
    if (-not $b[$k]) { continue }
    $c0 = if ($a[$k]) { $a[$k].cpu } else { 0 }
    $cost[$k] = [ordered]@{ private_mb = [math]::Round($b[$k].priv / 1MB, 1); ws_mb = [math]::Round($b[$k].ws / 1MB, 1); cpu_ms_30s = [math]::Round($b[$k].cpu - $c0) }
}
$result['standing'] = $cost
$sum = { param($keys, $field) [math]::Round((@($keys | ForEach-Object { if ($cost[$_]) { $cost[$_][$field] } else { 0 } }) | Measure-Object -Sum).Sum, 1) }
$result['lulo_private_mb'] = & $sum @('lulo-shell', 'lulo-session') 'private_mb'
$result['explorer_shell_private_mb'] = & $sum @('explorer', 'StartMenuExperienceHost', 'SearchHost', 'ShellExperienceHost') 'private_mb'
$result['lulo_ws_mb'] = & $sum @('lulo-shell', 'lulo-session') 'ws_mb'
$result['explorer_shell_ws_mb'] = & $sum @('explorer', 'StartMenuExperienceHost', 'SearchHost', 'ShellExperienceHost') 'ws_mb'

# ---- Interactive timings ----------------------------------------------------

$until = (Get-Date).AddMinutes($IdleWaitMinutes)
while ($R::IdleMs() -lt 120000) {
    if ((Get-Date) -gt $until) { $result['interactive'] = 'skipped: the PC was in use'; break }
    Start-Sleep -Seconds 20
}
if (-not $result['interactive'] -and (Shell-Pid)) {
    try {
        Mark
        # Spotlight's hotkey: the one Lulo recorded when Alt+Space was taken.
        $notice = (Get-ItemProperty 'HKCU:\Software\Lulo\Shell' -Name SpotlightHotkeyNotice -ErrorAction SilentlyContinue).SpotlightHotkeyNotice
        $hotkey = switch ($notice) { 2 { @(0x5B, 0x20) } 3 { @(0x11, 0x12, 0x20) } 4 { @(0x12, 0x10, 0x20) } default { @(0x12, 0x20) } }
        $times = @()
        for ($i = 0; $i -lt $Runs; $i++) {
            if (-not (Front-Lulo)) { break }
            $times += Time-NewShellWindow { [void](Keys-ToLulo $hotkey) } { param($w) $R::Title($w) -eq 'Spotlight' }
            Escape
        }
        $result['spotlight_open_ms'] = Median $times
        $result['spotlight_open_runs'] = $times -join ','

        $cpu0 = $R::CpuMs((Shell-Pid))
        # A file search: typing a folder every profile has ("Documents")
        # until a row names it. Only the timing is kept.
        $times = @()
        for ($i = 0; $i -lt $Runs; $i++) {
            if (-not (Front-Lulo)) { break }
            [void](Keys-ToLulo $hotkey); Start-Sleep -Milliseconds 600
            $panel = @(Shell-Windows | Where-Object { $R::Title($_) -eq 'Spotlight' }) | Select-Object -First 1
            if (-not $panel -or $R::PidOf($R::Foreground()) -ne (Shell-Pid)) { Escape; break }
            $watch = [Diagnostics.Stopwatch]::StartNew()
            $R::Type('Documents'); Mark
            $found = -1
            while ($watch.ElapsedMilliseconds -lt 15000) {
                if (@($R::Tree($panel, 8, 300) | Where-Object { $_.Name -match '^Documents\b' -and $_.Type -ne 'Edit' }).Count) { $found = [double]$watch.ElapsedMilliseconds; break }
                Start-Sleep -Milliseconds 20
            }
            $times += $found
            Escape
        }
        $result['spotlight_file_ms'] = Median $times
        $result['spotlight_file_runs'] = $times -join ','
        $result['spotlight_file_shell_cpu_ms'] = [math]::Round($R::CpuMs((Shell-Pid)) - $cpu0)

        # Mission Control (Ctrl+Up): a new full-screen shell window.
        $screen = $R::Screen()
        $times = @()
        for ($i = 0; $i -lt $Runs; $i++) {
            if (-not (Front-Lulo)) { break }
            $times += Time-NewShellWindow { [void](Keys-ToLulo @($VK.Ctrl, $VK.Up)) } {
                param($w) $f = $R::WindowRect($w); $f.Width -ge $screen.Width -and $f.Height -ge $screen.Height
            }
            Start-Sleep -Milliseconds 600
            if ($i -eq 0) { $R::Shot((Join-Path $out "mission-control.jpg")) }
            Escape
        }
        $result['mission_control_open_ms'] = Median $times
        $result['mission_control_open_runs'] = $times -join ','

        # Control Centre from its bar item.
        $bar = [IntPtr]::Zero
        foreach ($w in Shell-Windows) { if ($R::Tree($w, 1, 4) | Where-Object { $_.Name -eq 'top bar' }) { $bar = $w; break } }
        $button = if ($bar -ne [IntPtr]::Zero) { @($R::Tree($bar, 3, 80) | Where-Object { $_.Type -eq 'Button' -and $_.Name -eq 'Control Centre' }) | Select-Object -First 1 }
        if ($button) {
            $times = @()
            for ($i = 0; $i -lt $Runs; $i++) {
                $x = [int]($button.Bounds.X + $button.Bounds.Width / 2); $y = [int]($button.Bounds.Y + $button.Bounds.Height / 2)
                Guard
                if ($R::PidAt($x, $y) -ne (Shell-Pid)) { break }
                $times += Time-NewShellWindow { $R::Click($x, $y); Mark } { param($w) $R::Title($w) -eq 'Quick Settings' }
                Escape
            }
            $result['control_centre_open_ms'] = Median $times
            $result['control_centre_open_runs'] = $times -join ','
        }

        # A folder window: Lulo Files, then File Explorer, on the same folder.
        $files = Join-Path $Bin 'rmac-files.exe'
        $times = @()
        for ($i = 0; $i -lt $Runs -and (Test-Path $files); $i++) {
            Guard
            $watch = [Diagnostics.Stopwatch]::StartNew()
            $p = Start-Process $files -ArgumentList "`"$folder`"" -PassThru
            $w = $null
            while ($watch.ElapsedMilliseconds -lt 15000 -and -not $w) { $w = @($R::WindowsOf($p.Id, $true)) | Select-Object -First 1; if (-not $w) { Start-Sleep -Milliseconds 5 } }
            $times += if ($w) { [double]$watch.ElapsedMilliseconds } else { -1 }
            Start-Sleep -Milliseconds 800
            if ($w) { $R::Close($w) }
            Start-Sleep -Milliseconds 800
            if (-not $p.HasExited) { $p.Kill() }
        }
        $result['lulo_files_window_ms'] = Median $times
        $result['lulo_files_window_runs'] = $times -join ','
        $leaf = Split-Path -Leaf $folder
        $times = @()
        for ($i = 0; $i -lt $Runs; $i++) {
            Guard
            $before = @($R::ZOrder())
            $watch = [Diagnostics.Stopwatch]::StartNew()
            Start-Process explorer.exe -ArgumentList "`"$folder`""
            $w = $null
            while ($watch.ElapsedMilliseconds -lt 15000 -and -not $w) {
                $w = @($R::ZOrder() | Where-Object { $before -notcontains $_ -and ($R::Title($_) -eq $leaf -or $R::Title($_).StartsWith("$leaf - ")) }) | Select-Object -First 1
                if (-not $w) { Start-Sleep -Milliseconds 5 }
            }
            $times += if ($w) { [double]$watch.ElapsedMilliseconds } else { -1 }
            if ($w) { $result['explorer_folder_process'] = $R::ProcessName($R::PidOf($w)) }
            Start-Sleep -Milliseconds 800
            # Every window this opened on the scratch folder, found or late.
            foreach ($o in $R::ZOrder()) { if ($before -notcontains $o -and $R::Title($o).StartsWith($leaf)) { $R::Close($o) } }
            Start-Sleep -Milliseconds 800
        }
        $result['explorer_folder_window_ms'] = Median $times
        $result['explorer_folder_window_runs'] = $times -join ','
    } catch {
        $result['stopped'] = "$_"
    }
}

$result['shell_private_mb_after'] = if (Shell-Pid) { [math]::Round($R::PrivateBytes((Shell-Pid)) / 1MB, 1) }
$json = $result | ConvertTo-Json -Depth 5
Set-Content -Encoding UTF8 (Join-Path $out 'speed.json') $json
$json
