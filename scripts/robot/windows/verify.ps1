# Every Lulo OS shell behaviour docs/parity.md's "Lulo OS → Windows" table
# marks as working, checked live on a real PC against a running Lulo layer
# started with LULO_SHELL_TRACE=1 (its trace file is -Trace) and the same
# private profile (-Profile): the bar's menus and status menus (the
# battery's mark and rows), the Dock's tiles, menus and recent apps, the
# desktop's menu and click-to-reveal, Mission Control, App Exposé, Move &
# Resize, the switcher, Spotlight, Apps, Control Centre, Notification
# Centre and Force Quit. One JSON result line per row, with screenshots,
# goes to the scratch folder. Same safety rules as robot.ps1: it waits for
# the PC to be idle, sends keys only while Lulo is in front, opens only
# Lulo apps of its own and closes every window it opens. Run it through
# invoke.ps1.
param(
    [string]$Bin = 'E:\lulo\winpar',
    [string]$Trace = 'E:\lulo\winpar\trace.txt',
    [string]$Profile = 'E:\lulo\winpar\profile',
    [string]$Scratch = 'E:\lulo\robot-scratch'
)
$ErrorActionPreference = 'Continue'
. (Join-Path (Split-Path -Parent $MyInvocation.MyCommand.Path) 'common.ps1')
$out = Join-Path $Scratch ("verify\" + (Get-Date).ToString('yyyyMMdd-HHmmss'))
New-Item -ItemType Directory -Force $out | Out-Null
$env:APPDATA = "$Profile\Roaming"; $env:LOCALAPPDATA = "$Profile\Local"; $env:TEMP = "$Profile\Temp"; $env:TMP = "$Profile\Temp"
$result = [ordered]@{}
$started = New-Object System.Collections.ArrayList

function Shell-Pid { (Get-Process lulo-shell -ErrorAction SilentlyContinue | Select-Object -First 1).Id }
function Ours($id) { $id -eq (Shell-Pid) -or (@($started | ForEach-Object { $_.Id }) -contains $id) }
function Lulo-Front { Ours ($R::PidOf($R::Foreground())) }
function Esc { if (Lulo-Front) { $R::Chord(@($VK.Esc)) }; Start-Sleep -Milliseconds 400 }
function Keys([int[]]$keys) { if (Lulo-Front) { $R::Chord($keys); return $true }; return $false }
function Shot($name) { $R::Shot((Join-Path $out "$name.jpg")) }
function Note($row, $ok, $detail) { $result[$row] = "$(if ($ok) { 'ok' } else { 'FAIL' }): $detail" }

# The shell's trace: lines after `$from`, without the prefix.
function Trace-Lines([int]$from = 0) {
    $lines = @(Get-Content -LiteralPath $Trace -ErrorAction SilentlyContinue | Where-Object { $_ -like 'lulo-shell: *' } | ForEach-Object { $_.Substring(12) })
    if ($from -ge $lines.Count) { return @() }
    return $lines[$from..($lines.Count - 1)]
}
function Mark { @(Trace-Lines).Count }
function Wait-Trace([string]$pattern, [int]$from = 0, [double]$seconds = 5) {
    $until = (Get-Date).AddSeconds($seconds)
    do {
        $hit = @(Trace-Lines $from) | Where-Object { $_ -match $pattern } | Select-Object -Last 1
        if ($hit) { return $hit }
        Start-Sleep -Milliseconds 150
    } while ((Get-Date) -lt $until)
    return $null
}
function Last-Trace([string]$pattern) { @(Trace-Lines) | Where-Object { $_ -match $pattern } | Select-Object -Last 1 }
function Point-Of([string]$line) { if ($line -match ' at (-?\d+),(-?\d+)') { return @([int]$Matches[1], [int]$Matches[2]) }; return $null }
function Click-Trace([string]$pattern) {
    $p = Point-Of (Last-Trace $pattern)
    if ($p -and (Ours ($R::PidAt($p[0], $p[1])))) { $R::Click($p[0], $p[1]); return $true }
    return $false
}
# The accessible names in lulo-shell's windows.
function Shell-Names {
    $names = @()
    foreach ($w in $R::WindowsOf((Shell-Pid), $false)) {
        if (-not $R::Visible($w)) { continue }
        $names += @($R::Tree($w, 14, 600) | Where-Object { $_.Name } | ForEach-Object { $_.Name })
    }
    return $names
}
function Shell-Node([string]$pattern) {
    foreach ($w in $R::WindowsOf((Shell-Pid), $false)) {
        if (-not $R::Visible($w)) { continue }
        $hit = $R::Tree($w, 14, 600) | Where-Object { $_.Name -match $pattern -and -not $_.Bounds.IsEmpty } | Select-Object -First 1
        if ($hit) { return $hit }
    }
    return $null
}
function Click-Node($node) {
    if (-not $node) { return $false }
    $x = [int]($node.Bounds.X + $node.Bounds.Width / 2); $y = [int]($node.Bounds.Y + $node.Bounds.Height / 2)
    if (-not (Ours ($R::PidAt($x, $y)))) { return $false }
    $R::Click($x, $y); return $true
}
function Launch($exe) {
    $p = Start-Process (Join-Path $Bin $exe) -PassThru
    [void]$started.Add($p)
    $deadline = (Get-Date).AddSeconds(15)
    while ((Get-Date) -lt $deadline -and -not @($R::WindowsOf($p.Id, $true))) { Start-Sleep -Milliseconds 200 }
    Start-Sleep -Milliseconds 800
    return $p
}
function Close-App($p) {
    if ($p -and -not $p.HasExited) { $p.CloseMainWindow() | Out-Null; Start-Sleep -Milliseconds 800; if (-not $p.HasExited) { $p.Kill() } }
}
# Calculator (the robot's own app) to the front.
function Front-Calc {
    [void]$R::Activate((@($R::WindowsOf($calc.Id, $true)) | Select-Object -First 1))
    Start-Sleep -Milliseconds 500
}
function Desktop-Point {
    $screen = $R::Screen()
    foreach ($y in 300, 450, 120) { foreach ($x in 200, 40, ($screen.Width / 2), ($screen.Width - 40)) {
        if ($R::PidAt([int]$x, [int]$y) -eq (Shell-Pid)) { return @([int]$x, [int]$y) }
    } }
    return $null
}
function Check($row, [scriptblock]$body) {
    try { & $body } catch { Note $row $false "stopped: $_" }
    Esc
}

if (-not (Shell-Pid)) { Write-Output '{"stopped": "no lulo-shell running"}'; exit 1 }
while ($R::IdleMs() -lt 120000) { Start-Sleep -Seconds 20 }
try {
    $calc = Launch 'rmac-calculator.exe'
    if (-not (Lulo-Front)) { throw 'Lulo is not in front; not sending keys' }

    # Menu bar ------------------------------------------------------------
    Check 'menu bar: Lulo menu' {
        $m = Mark
        $ok = Click-Trace '^bar title 0 logo at'
        Start-Sleep -Milliseconds 600
        $rows = @(Trace-Lines $m | Where-Object { $_ -match '^menu 0 row (.+) at ' } | ForEach-Object { ($_ -replace '^menu 0 row (.+) at .*$', '$1') })
        Shot 'lulo-menu'
        $want = 'About This Lulo OS', 'System Settings…', 'Force Quit…', 'Sleep', 'Restart…', 'Shut Down…', 'Lock Screen'
        $missing = @($want | Where-Object { $rows -notcontains $_ })
        Note 'menu bar: Lulo menu' ($ok -and -not $missing) "rows: $($rows -join ', '); missing: $($missing -join ', ')"
    }
    Check "menu bar: front app's menus" {
        Front-Calc
        $title = Wait-Trace '^bar title 1 Calculator ' 0 3
        $m = Mark
        $ok = Click-Trace '^bar title 2 '
        Start-Sleep -Milliseconds 600
        $rows = @(Trace-Lines $m | Where-Object { $_ -match '^menu 2 row (.+) at ' } | ForEach-Object { ($_ -replace '^menu 2 row (.+) at .*$', '$1') })
        Shot 'app-menu'
        Note "menu bar: front app's menus" ($ok -and $title -match 'Calculator' -and $rows.Count -gt 2) "$title; menu 2: $($rows -join ', ')"
    }
    foreach ($item in 'Wi-Fi', 'Battery', 'Sound', 'Bluetooth', 'Focus') {
        Check "menu bar: $item menu" {
            $node = Shell-Node "^$item"
            if (-not $node) {
                # Sound, Bluetooth and Focus are not in the bar by default,
                # as on the Mac; Control Centre has them (checked below).
                $result["menu bar: $item menu"] = 'n/a: not in the bar by default'
                return
            }
            $m = Mark
            $ok = Click-Node $node
            $first = Wait-Trace '^status menu ' $m 4
            # The full reading arrives after the menu opens.
            Start-Sleep -Seconds 3
            $menu = Last-Trace '^status menu '
            if ($item -eq 'Battery') { $result['battery menu at once'] = "$first" }
            Shot "status-$item"
            if ($item -eq 'Battery') {
                $result['battery reading'] = "$(Last-Trace '^battery item ')"
                $result['battery menu'] = "$menu"
            }
            Note "menu bar: $item menu" ($ok -and $menu) "item: $($node.Name); $menu"
        }
    }
    Check 'menu bar: battery mark and rows' {
        $status = [System.Windows.Forms.SystemInformation]::PowerStatus
        $item = Last-Trace '^battery item '
        $menu = "$($result['battery menu'])"
        $plugged = "$($status.PowerLineStatus)" -eq 'Online'
        $mark = if ($item -match 'mark Some\((\w+)\)') { $Matches[1] } else { 'None' }
        $wantMark = if (-not $plugged) { 'None' } elseif ("$($status.BatteryChargeStatus)" -match 'Charging') { 'Charging' } else { 'PluggedIn' }
        $hasLine = $menu -match 'Power Source: (Power Adapter|Battery)' -and ($menu -match 'Fully Charged|Charging|Battery Is Not Charging|remaining' -or -not $plugged)
        Note 'menu bar: battery mark and rows' ($mark -eq $wantMark -and $hasLine -and $menu -match 'Using Significant Energy') "Windows: $($status.PowerLineStatus), $($status.BatteryChargeStatus), $([int]($status.BatteryLifePercent * 100))%; bar: $item; menu: $menu"
    }
    Check 'menu bar: Ctrl+F2 menu focus' {
        Front-Calc
        $m = Mark
        # ⌃F2 puts the keyboard on the bar's first menu title, as on the
        # Mac; ↓ then opens that menu.
        $sent = Keys @($VK.Ctrl, 0x71)
        Start-Sleep -Milliseconds 600
        $front = $R::ProcessName($R::PidOf($R::Foreground()))
        [void](Keys @($VK.Down))
        $open = Wait-Trace '^menu \d+ row ' $m 3
        Shot 'ctrl-f2'
        Note 'menu bar: Ctrl+F2 menu focus' ($sent -and $open) "front after Ctrl+F2: $front; $open"
    }

    # Dock ----------------------------------------------------------------
    Check 'dock: tiles and running dot' {
        $tiles = @(Trace-Lines | Where-Object { $_ -match '^dock tile ' })
        $calcTile = $tiles | Where-Object { $_ -match '^dock tile org\.rmac\.Calculator' } | Select-Object -Last 1
        Note 'dock: tiles and running dot' ($calcTile -match 'running') "$calcTile"
    }
    Check 'dock: tile menu' {
        $p = Point-Of (Last-Trace '^dock tile org\.rmac\.Calculator')
        $m = Mark
        if ($p -and (Ours ($R::PidAt($p[0], $p[1])))) { $R::RightClick($p[0], $p[1]) }
        $menu = Wait-Trace '^dock menu: ' $m 3
        Start-Sleep -Milliseconds 400
        Shot 'dock-menu'
        $rows = if ($menu) { @($menu.Substring(11) -split ' \| ') } else { @() }
        $want = 'Show All Windows', 'Hide', 'Quit'
        $missing = @($want | Where-Object { $rows -notcontains $_ })
        Note 'dock: tile menu' ($menu -and -not $missing) "$menu; missing: $($missing -join ', ')"
    }
    Check 'dock: recent apps' {
        # Three Lulo apps a fresh profile's Dock does not keep, opened and
        # quit in turn: the recent apps are those three, oldest first.
        $kept = 'files', 'notes', 'text-editor', 'terminal', 'system-settings', 'weather', 'calculator', 'mail', 'app-drawer'
        $ids = @{ 'preview' = 'Preview'; 'clock' = 'Clock'; 'system-monitor' = 'SystemMonitor'; 'calendar' = 'Calendar' }
        $apps = @(Get-ChildItem $Bin -Filter 'rmac-*.exe' | ForEach-Object { $_.BaseName.Substring(5) } | Where-Object { $kept -notcontains $_ -and $ids.ContainsKey($_) }) | Select-Object -First 3
        foreach ($app in $apps) { $p = Launch "rmac-$app.exe"; Close-App $p; Start-Sleep -Milliseconds 600 }
        Start-Sleep -Seconds 2
        $state = @(Get-Content (Join-Path $Profile 'Local\Lulo\State\rmac\dock-recents') -ErrorAction SilentlyContinue)
        Shot 'dock-recents'
        $want = @($apps | ForEach-Object { "org.rmac.$($ids[$_]).desktop" })
        Note 'dock: recent apps' ($want.Count -eq 3 -and ($state -join ',') -eq ($want -join ',')) "opened: $($apps -join ', '); state: $($state -join ', ')"
        $edge = Test-Path (Join-Path $Profile 'Roaming\Lulo\Data\applications\msedge.exe.desktop')
        Note 'dock: Edge has an entry (recents for apps listed by AUMID)' $edge "msedge.exe.desktop: $edge"
    }
    Check 'dock: minimise into a tile with its picture' {
        [void]$R::Activate((@($R::WindowsOf($calc.Id, $true)) | Select-Object -First 1))
        Start-Sleep -Milliseconds 1200
        $m = Mark
        [void](Keys @($VK.Ctrl, 0x4D))
        $stored = Wait-Trace '^thumbnail stored ' $m 4
        Start-Sleep -Milliseconds 800
        Shot 'minimised'
        $tile = Last-Trace '^dock tile .*minimi'
        Note 'dock: minimise into a tile with its picture' ($stored) "$stored"
        $w = @($R::WindowsOf($calc.Id, $true)) | Select-Object -First 1
        if ($w) { $R::Show($w, 9) }
        Start-Sleep -Milliseconds 800
    }

    # Desktop -------------------------------------------------------------
    Check 'desktop: right-click menu' {
        $p = Desktop-Point
        $m = Mark
        if ($p) { $R::RightClick($p[0], $p[1]) }
        $menu = Wait-Trace '^desktop menu' $m 3
        Start-Sleep -Milliseconds 400
        Shot 'desktop-menu'
        $names = Shell-Names
        Note 'desktop: right-click menu' ($names -contains 'New Folder') "trace: $menu"
    }
    Check 'desktop: click wallpaper to reveal' {
        $w = @($R::WindowsOf($calc.Id, $true)) | Select-Object -First 1
        $before = $R::Frame($w)
        $p = Desktop-Point
        if ($p) { $R::Click($p[0], $p[1]) }
        Start-Sleep -Milliseconds 900
        $aside = $R::Frame($w)
        $p = Desktop-Point
        if ($p) { $R::Click($p[0], $p[1]) }
        Start-Sleep -Milliseconds 900
        $back = $R::Frame($w)
        Note 'desktop: click wallpaper to reveal' ($aside.Y -ne $before.Y -and $back.Y -eq $before.Y -and $back.X -eq $before.X) "before $before, aside $aside, back $back"
    }

    # Windows -------------------------------------------------------------
    foreach ($pair in @(@('windows: Mission Control (Ctrl+Up)', 0x26, 'mission'), @('windows: App Exposé (Ctrl+Down)', 0x28, 'mission|expos'))) {
        Check $pair[0] {
            [void]$R::Activate((@($R::WindowsOf($calc.Id, $true)) | Select-Object -First 1)); Start-Sleep -Milliseconds 500
            $m = Mark
            $sent = Keys @($VK.Ctrl, $pair[1])
            $shown = Wait-Trace "^(layer|open) .*($($pair[2]))" $m 3
            Start-Sleep -Milliseconds 700
            Shot ($pair[0] -replace '[^a-z]', '')
            Note $pair[0] ($sent -and $shown) "$shown"
        }
    }
    Check 'windows: Move & Resize from the Window menu' {
        [void]$R::Activate((@($R::WindowsOf($calc.Id, $true)) | Select-Object -First 1)); Start-Sleep -Milliseconds 500
        $titles = @(Trace-Lines | Where-Object { $_ -match '^bar title \d+ Window at ' }) | Select-Object -Last 1
        $m = Mark
        $ok = Click-Trace '^bar title \d+ Window at '
        Start-Sleep -Milliseconds 600
        $rows = @(Trace-Lines $m | Where-Object { $_ -match '^menu \d+ row ' })
        Note 'windows: Move & Resize from the Window menu' ($ok -and ($rows -match 'Move & Resize|Fill|Centre|Center')) "$($rows -join ' / ')"
    }

    # Switcher, Spotlight, Apps, Control Centre, Notification Centre -----
    Check 'switcher: Alt+Tab' {
        Front-Calc
        $editor = Launch 'rmac-text-editor.exe'
        $m = Mark
        if (Lulo-Front) { $R::KeyDown($VK.Alt); $R::Chord(@($VK.Tab)); Start-Sleep -Milliseconds 700; Shot 'switcher'; $R::KeyUp($VK.Alt) }
        $shown = Wait-Trace '^(switcher|layer rmac-app-switcher)' $m 2
        Start-Sleep -Milliseconds 600
        $front = $R::PidOf($R::Foreground())
        $title = $R::Title($R::Foreground())
        Note 'switcher: Alt+Tab' ($shown -and $front -eq $calc.Id) "$shown; front after: $($R::ProcessName($front)) '$title'"
        Close-App $editor
    }
    Check 'spotlight: apps and answers' {
        [void]$R::Activate((@($R::WindowsOf($calc.Id, $true)) | Select-Object -First 1)); Start-Sleep -Milliseconds 400
        $m = Mark
        $sent = Click-Trace '^bar spotlight at \d+,1\d$'
        Start-Sleep -Milliseconds 900
        if ($sent -and (Lulo-Front)) { $R::Type('100/4') }
        $rows = Wait-Trace '^spotlight "100/4": [1-9]' $m 4
        Start-Sleep -Milliseconds 500
        $answer = @(Shell-Names) -match '\b25\b'
        Shot 'spotlight'
        Esc
        $m = Mark
        $sent = Click-Trace '^bar spotlight at \d+,1\d$'
        Start-Sleep -Milliseconds 900
        if ($sent -and (Lulo-Front)) { $R::Type('calcul') }
        $apps = Wait-Trace '^spotlight "calcul": [1-9]' $m 4
        $app = @(Wait-Trace '^spotlight "calcul": [1-9]' $m 4)
        Note 'spotlight: apps and answers' ($rows -and $answer -and $apps -and $app) "$rows; answer: $($answer -join ' / '); $apps; app: $($app -join ' / ')"
    }
    Check 'apps: grid' {
        $m = Mark
        $ok = Click-Trace '^dock tile org\.rmac\.AppDrawer'
        $shown = Wait-Trace '^(layer|open) .*app-drawer' $m 3
        Start-Sleep -Milliseconds 800
        $names = Shell-Names
        Shot 'apps'
        Note 'apps: grid' ($shown -and ($names -contains 'Calculator')) "$shown"
    }
    Check 'control centre: modules' {
        $m = Mark
        $ok = Click-Trace '^bar control centre at'
        $shown = Wait-Trace '^(layer|open) .*quick-settings' $m 3
        Start-Sleep -Milliseconds 900
        $names = Shell-Names
        Shot 'control-centre'
        $want = 'Wi-Fi', 'Bluetooth', 'Focus'
        $missing = @($want | Where-Object { -not ($names -match "^$_") })
        Note 'control centre: modules' ($shown -and -not $missing) "$shown; missing: $($missing -join ', ')"
    }
    Check 'notification centre: opens from the clock' {
        $node = Shell-Node '\d{1,2}:\d{2}'
        $m = Mark
        $ok = Click-Node $node
        $shown = Wait-Trace '^(layer|open) .*notification' $m 3
        Start-Sleep -Milliseconds 900
        Shot 'notification-centre'
        Note 'notification centre: opens from the clock' ($shown) "clock: $($node.Name); $shown"
    }
    Check 'force quit: Ctrl+Alt+Esc' {
        [void]$R::Activate((@($R::WindowsOf($calc.Id, $true)) | Select-Object -First 1)); Start-Sleep -Milliseconds 400
        $before = @($R::ZOrder())
        $sent = Keys @($VK.Ctrl, $VK.Alt, $VK.Esc)
        Start-Sleep -Milliseconds 1200
        $new = @($R::ZOrder() | Where-Object { $before -notcontains $_ -and $R::PidOf($_) -eq (Shell-Pid) -and $R::Title($_) })
        Shot 'force-quit'
        Note 'force quit: Ctrl+Alt+Esc' ($new.Count -gt 0) "$(($new | ForEach-Object { $R::Title($_) }) -join ', ')"
        foreach ($w in $new) { $R::Close($w) }
    }
} catch { $result['stopped'] = "$_" }
finally {
    Esc
    foreach ($p in $started) { Close-App $p }
}
$result | ConvertTo-Json | Tee-Object (Join-Path $out 'result.json')
