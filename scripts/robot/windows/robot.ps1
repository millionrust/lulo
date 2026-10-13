# The Lulo robot for Windows: drives Lulo's apps and the Lulo layer with
# real keyboard and mouse input on the signed-in desktop, and writes what
# looks wrong to findings.json with screenshots. See README.md.
#
# Run it through invoke.ps1 (it must run on the user's desktop, not in an
# SSH session):
#
#   invoke.ps1 -Script <dir>\robot.ps1 -Arguments '-Apps calculator,notes'
#   invoke.ps1 -Script <dir>\robot.ps1 -Arguments '-Shell'
#   invoke.ps1 -Script <dir>\robot.ps1 -Arguments '-Toggle'
#
# Safety (the PC may belong to someone else):
# - it waits for 2 minutes without input before it starts and stops if
#   real input arrives while it runs;
# - apps run with private %APPDATA%/%LOCALAPPDATA% under the scratch folder
#   and open only files it writes there;
# - menu items, buttons and keys whose names look destructive or outward
#   (delete, empty, quit, send, shut down, wallpaper, ...) are listed but
#   never chosen; the Lulo layer's toggles (Wi-Fi, Bluetooth, ...) are
#   never clicked;
# - every window it opens is closed again, one app runs at a time, and
#   Lulo mode is left as it was found.
param(
    [string]$Apps = '',
    [switch]$Shell,
    [switch]$Toggle,
    [ValidateSet('auto', 'strip')][string]$MenuMode = 'auto',
    [string]$Bin = "$env:LOCALAPPDATA\Programs\Lulo",
    [string]$Scratch = 'E:\lulo\robot-scratch',
    [int]$MaxMinutes = 10,
    [int]$IdleWaitMinutes = 10
)
$ErrorActionPreference = 'Continue'
. (Join-Path (Split-Path -Parent $MyInvocation.MyCommand.Path) 'common.ps1')

$started = Get-Date
$deadline = $started.AddMinutes($MaxMinutes)
$stamp = $started.ToString('yyyyMMdd-HHmmss')
$out = Join-Path $Scratch "out\$stamp"
New-Item -ItemType Directory -Force $out | Out-Null
$findings = New-Object System.Collections.ArrayList
$measures = [ordered]@{}
$shotIndex = 0

# ---- The apps ---------------------------------------------------------------

$fixtures = Join-Path $Scratch 'fixtures'
$AppTable = [ordered]@{
    'calculator'      = @{ Exe = 'rmac-calculator.exe'; Name = 'Calculator' }
    'notes'           = @{ Exe = 'rmac-notes.exe'; Name = 'Notes' }
    'text-editor'     = @{ Exe = 'rmac-text-editor.exe'; Name = 'Text Editor'; Args = "`"$fixtures\robot note.txt`"" }
    'preview'         = @{ Exe = 'rmac-preview.exe'; Name = 'Preview'; Args = "`"$fixtures\robot picture.png`"" }
    'clock'           = @{ Exe = 'rmac-clock.exe'; Name = 'Clock' }
    'weather'         = @{ Exe = 'rmac-weather.exe'; Name = 'Weather' }
    'terminal'        = @{ Exe = 'rmac-terminal.exe'; Name = 'Terminal' }
    'files'           = @{ Exe = 'rmac-files.exe'; Name = 'Files'; Args = "`"$fixtures`"" }
    'system-settings' = @{ Exe = 'rmac-system-settings.exe'; Name = 'System Settings' }
}

# Never chosen: destructive, outward, or would change the PC's own state
# (other apps' windows, the clipboard's files, the user's wallpaper).
$Deny = '(?i)\b(quit|close|delete|bin|trash|erase|empty|log ?out|sign out|shut ?down|restart|sleep|uninstall|send|reply|forward|desktop picture|wallpaper|remove|eject|format|lock|force|reset|discard|revert|replace|hide|show all|print|share|airdrop|mail|message|report|feedback|website|online|update|install|kill|signal|clear|turn off|archive|compress|move|burn|rename|connect|sign in|account|password|bluetooth|wi-?fi|airplane|do not disturb|focus|night|volume|mute|brightness|power|battery)\b'
# Choosing these with nothing selected and an empty document legitimately shows nothing.
$QuietOk = '(?i)^(undo|redo|cut|copy|paste|paste and match style|select all|delete|find next|find previous|use selection for find|bring all to front|arrange in front|services|start dictation|emoji & symbols|spelling.*|check spelling.*|substitutions|transformations|speech|.*recent.*)$'
# Standard window controls in every window: tested by the resize and close steps instead.
$WindowControls = '(?i)^(close window|close|minimize|minimise|maximize|maximise|zoom|enter full screen|exit full screen|system|restore)$'

function Add-Finding($severity, $area, $app, $issue, $detail = '', $shot = '') {
    [void]$findings.Add([ordered]@{
        severity = $severity; area = $area; app = $app; issue = $issue; detail = $detail; screenshot = $shot
        at = [int]((Get-Date) - $started).TotalSeconds
    })
    Write-Host "FINDING [$severity] $app/${area}: $issue $detail"
}

function Shot($label) {
    $script:shotIndex++
    $name = '{0:D3}-{1}.jpg' -f $script:shotIndex, ($label -replace '[^A-Za-z0-9._-]+', '-')
    $path = Join-Path $out $name
    $R::Shot($path)
    return $path
}

# Real input from someone else ends the run: our own SendInput resets the
# idle timer too, so this compares against the time of our last input.
$script:lastOwnInput = Get-Date
function Mark { $script:lastOwnInput = Get-Date }
function Guard {
    $ownAgo = ((Get-Date) - $script:lastOwnInput).TotalMilliseconds
    if ($R::IdleMs() + 1500 -lt $ownAgo) { throw 'robot: real input detected, stopping' }
    if ((Get-Date) -gt $deadline) { throw "robot: time limit of $MaxMinutes min reached" }
}
function Keys { param([int[]]$k) Guard; $R::Chord($k); Mark }
function Click($x, $y) { Guard; $R::Click([int]$x, [int]$y); Mark }
function RClick($x, $y) { Guard; $R::RightClick([int]$x, [int]$y); Mark }
function DClick($x, $y) { Guard; $R::DoubleClick([int]$x, [int]$y); Mark }
function Center($node) { return @(($node.Bounds.X + $node.Bounds.Width / 2), ($node.Bounds.Y + $node.Bounds.Height / 2)) }

# ---- Preflight ------------------------------------------------------------

function Wait-ForIdle {
    $until = (Get-Date).AddMinutes($IdleWaitMinutes)
    while ($R::IdleMs() -lt 120000) {
        if ((Get-Date) -gt $until) { throw "robot: the PC was in use for $IdleWaitMinutes min, not starting" }
        Write-Host "robot: input $([int]($R::IdleMs()/1000)) s ago, waiting for 2 min idle"
        Start-Sleep -Seconds 20
    }
}

function Other-TestRunning {
    if ((Get-ScheduledTask -TaskName LuloCheck -ErrorAction SilentlyContinue).State -eq 'Running') { return 'LuloCheck task' }
    $foreign = Get-CimInstance Win32_Process | Where-Object {
        $_.Name -match '^(lulo|rmac)-' -and $_.ExecutablePath -and
        -not $_.ExecutablePath.StartsWith($Bin, [StringComparison]::OrdinalIgnoreCase)
    }
    if ($foreign) { return ($foreign | ForEach-Object { $_.ExecutablePath }) -join ', ' }
    return $null
}

function Lulo-State {
    $session = Get-Process lulo-session -ErrorAction SilentlyContinue
    $tb = $R::Taskbar()
    return [ordered]@{
        on = [bool]$session; workArea = "$($R::WorkArea())"; taskbarVisible = $R::Visible($tb)
    }
}

function Write-Fixtures {
    New-Item -ItemType Directory -Force $fixtures, "$fixtures\Robot Folder", "$fixtures\Robot Folder\Inner" | Out-Null
    Set-Content -Encoding UTF8 "$fixtures\robot note.txt" "Lulo robot note.`r`nThe quick brown fox jumps over the lazy dog.`r`n"
    Set-Content -Encoding UTF8 "$fixtures\Robot Folder\inside.txt" 'inside'
    Add-Type -AssemblyName System.Drawing
    $bmp = New-Object System.Drawing.Bitmap 320, 200
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.Clear([System.Drawing.Color]::FromArgb(255, 30, 120, 220))
    $g.FillEllipse([System.Drawing.Brushes]::Orange, 60, 30, 200, 140)
    $g.Dispose(); $bmp.Save("$fixtures\robot picture.png"); $bmp.Dispose()
}

function Saved-Clipboard {
    Add-Type -AssemblyName System.Windows.Forms
    $data = [System.Windows.Forms.Clipboard]::GetDataObject()
    if (-not $data) { return @{ textOnly = $true; text = $null } }
    $formats = @($data.GetFormats())
    $textOnly = -not ($formats | Where-Object { $_ -match 'FileDrop|Bitmap|DeviceIndependentBitmap|PNG|HTML Format|Rich Text' })
    $text = $null
    if ($textOnly -and [System.Windows.Forms.Clipboard]::ContainsText()) { $text = [System.Windows.Forms.Clipboard]::GetText() }
    return @{ textOnly = $textOnly; text = $text; formats = $formats -join ',' }
}

function Restore-Clipboard($saved) {
    if (-not $saved.textOnly) { return }
    try {
        if ($null -ne $saved.text -and $saved.text.Length -gt 0) { [System.Windows.Forms.Clipboard]::SetText($saved.text) }
        else { [System.Windows.Forms.Clipboard]::Clear() }
    } catch { Write-Host "robot: could not restore the clipboard: $_" }
}

# ---- The Lulo bar ---------------------------------------------------------

function Shell-Pid { $p = Get-Process lulo-shell -ErrorAction SilentlyContinue | Select-Object -First 1; if ($p) { $p.Id } else { 0 } }

function Bar-Window {
    $pidShell = Shell-Pid
    if (-not $pidShell) { return [IntPtr]::Zero }
    foreach ($w in $R::WindowsOf($pidShell, $false)) {
        if ($R::Tree($w, 1, 4) | Where-Object { $_.Name -eq 'top bar' }) { return $w }
    }
    return [IntPtr]::Zero
}

function Bar-Buttons {
    $bar = Bar-Window
    if ($bar -eq [IntPtr]::Zero) { return @() }
    return @($R::Tree($bar, 3, 80) | Where-Object { $_.Type -eq 'Button' })
}

# The open menu (and submenu) panels the shell shows, with their items.
function Shell-Menus {
    $pidShell = Shell-Pid
    if (-not $pidShell) { return @() }
    $menus = @()
    foreach ($w in $R::WindowsOf($pidShell, $false)) {
        $tree = $R::Tree($w, 8, 400)
        $menus += @($tree | Where-Object { $_.Type -eq 'Menu' })
    }
    return $menus
}

function Items-Of($menuNode) {
    $items = @()
    try {
        $walker = [System.Windows.Automation.TreeWalker]::RawViewWalker
        $c = $walker.GetFirstChild($menuNode.Element)
        while ($c) {
            $cur = $c.Current
            if ($cur.ControlType.ProgrammaticName -eq 'ControlType.MenuItem') {
                $r = $cur.BoundingRectangle
                $items += [pscustomobject]@{ Name = $cur.Name; Enabled = $cur.IsEnabled; X = $r.X + $r.Width / 2; Y = $r.Y + $r.Height / 2; Element = $c }
            }
            $c = $walker.GetNextSibling($c)
        }
    } catch { }
    return $items
}

function Close-ShellMenus {
    for ($i = 0; $i -lt 3 -and (Shell-Menus).Count -gt 0; $i++) { Keys $VK.Esc; Start-Sleep -Milliseconds 300 }
}

# ---- The in-window strip (no Lulo bar) ------------------------------------

function Strip-Menus($hwnd) {
    return @($R::Tree($hwnd, 6, 300) | Where-Object { $_.Type -eq 'Menu' })
}

# ---- One app's session ----------------------------------------------------

function New-Run($key, $name) {
    $dir = Join-Path $out $key
    New-Item -ItemType Directory -Force $dir | Out-Null
    return [pscustomobject]@{
        Key = $key; Name = $name; Proc = $null; Main = [IntPtr]::Zero
        Home = [System.Drawing.Rectangle]::Empty; Dir = $dir; Err = (Join-Path $dir 'stderr.txt')
    }
}

function Profile-Env($run) {
    $prof = Join-Path $Scratch 'profile'
    New-Item -ItemType Directory -Force "$prof\Roaming", "$prof\Local", "$prof\Temp" | Out-Null
    $envs = @('APPDATA', "$prof\Roaming", 'LOCALAPPDATA', "$prof\Local", 'TEMP', "$prof\Temp", 'TMP', "$prof\Temp", 'RMAC_MENU_STRIP_TRACE', '1')
    if ($MenuMode -eq 'strip') { $envs += @('USERNAME', 'lulo-robot') }
    return [string[]]$envs
}

function Start-App($key) {
    $spec = $AppTable[$key]
    $run = New-Run $key $spec.Name
    if (Get-Process ($spec.Exe -replace '\.exe$', '') -ErrorAction SilentlyContinue) {
        Add-Finding 'info' 'launch' $run.Name 'an instance was already running; skipped (one instance at a time)'
        return $null
    }
    $t0 = Get-Date
    $run.Proc = $R::Launch((Join-Path $Bin $spec.Exe), $spec.Args, $fixtures, (Profile-Env $run), $run.Err)
    Mark
    while (((Get-Date) - $t0).TotalSeconds -lt 30) {
        $w = @($R::WindowsOf($run.Proc.Id, $true))
        if ($w.Count) { $run.Main = $w[0]; break }
        if ($run.Proc.HasExited) { break }
        Start-Sleep -Milliseconds 15
    }
    $ms = [int]((Get-Date) - $t0).TotalMilliseconds
    $measures["$key.launch_ms"] = $ms
    if ($run.Main -eq [IntPtr]::Zero) {
        $shot = Shot "$key-no-window"
        Add-Finding 'high' 'launch' $run.Name "no window within 30 s (exit code $(if ($run.Proc.HasExited) { $run.Proc.ExitCode } else { 'still running' }))" (Tail $run.Err) $shot
        if (-not $run.Proc.HasExited) { $run.Proc.Kill() }
        return $null
    }
    Write-Host "robot: $($run.Name) window after $ms ms"
    if ($ms -gt 1500) { Add-Finding 'medium' 'speed' $run.Name "first window after $ms ms (target about 300 ms)" }
    Start-Sleep -Milliseconds 1500
    $run.Home = $R::Frame($run.Main)
    $fg = $R::Foreground()
    if ($R::PidOf($fg) -ne $run.Proc.Id) {
        Add-Finding 'medium' 'focus' $run.Name "the new window opened behind another window (foreground: '$($R::Title($fg))' [$($R::ClassOf($fg))])" '' (Shot "$key-launch-focus")
    }
    Check-Placement $run $run.Main 'launch'
    Check-Visual $run $run.Main 'launch'
    Check-Bounds $run $run.Main 'launch'
    $measures["$key.private_mb.launch"] = [math]::Round($R::PrivateBytes($run.Proc.Id) / 1MB, 1)
    [void](Shot "$key-launched")
    return $run
}

function Tail($path) {
    if (Test-Path $path) { return ((Get-Content $path -Tail 12 -ErrorAction SilentlyContinue) -join "`n") }
    return ''
}

function Check-Placement($run, $hwnd, $state) {
    $wa = $R::WorkArea(); $f = $R::Frame($hwnd)
    if ($f.Width -le 0) { return }
    $outside = ($f.Left -lt $wa.Left - 1) -or ($f.Top -lt $wa.Top - 1) -or ($f.Right -gt $wa.Right + 1) -or ($f.Bottom -gt $wa.Bottom + 1)
    if ($outside) {
        Add-Finding 'medium' 'placement' $run.Name "window outside the work area after $state (frame $f, work area $wa)" '' (Shot "$($run.Key)-$state-outside")
    }
}

# Black, blank or washed-out window contents.
function Check-Visual($run, $hwnd, $state) {
    $f = $R::Frame($hwnd)
    if ($f.Width -le 8 -or $f.Height -le 8) { return }
    $b = $R::Grab($f)
    $a = $R::Analyze($b)
    if ($b) { $b.Dispose() }
    if (-not $a.Count) { return }
    $measures["$($run.Key).visual.$state"] = "black=$([math]::Round($a['black'],2)) dominant=$([math]::Round($a['dominant'],2)) spread=$($a['spread']) colours=$($a['colours'])"
    if ($a['black'] -gt 0.9) { Add-Finding 'high' 'visual' $run.Name "black window after $state" '' (Shot "$($run.Key)-$state-black") }
    elseif ($a['dominant'] -gt 0.985 -and $a['colours'] -lt 12) { Add-Finding 'high' 'visual' $run.Name "blank window after $state (one colour)" '' (Shot "$($run.Key)-$state-blank") }
    elseif ($a['spread'] -lt 25) { Add-Finding 'low' 'visual' $run.Name "low contrast after $state (luminance spread $($a['spread']))" '' (Shot "$($run.Key)-$state-contrast") }
}

# Named elements drawn outside their window, or with no size: overflow and clipping.
function Check-Bounds($run, $hwnd, $state) {
    $f = $R::Frame($hwnd)
    $bad = @()
    foreach ($n in $R::Tree($hwnd, 12, 600)) {
        if ($n.Depth -eq 0 -or -not $n.Name -or $n.Offscreen -or $n.Bounds.IsEmpty) { continue }
        if ($n.Type -in @('TitleBar', 'MenuBar', 'MenuItem')) { continue }
        if ($n.Bounds.Height -le 2) { continue } # DWM's own hidden caption buttons
        $b = $n.Bounds
        if ($b.Left -lt $f.Left - 2 -or $b.Right -gt $f.Right + 2 -or $b.Top -lt $f.Top - 2 -or $b.Bottom -gt $f.Bottom + 2) {
            $bad += "$($n.Type) '$($n.Name)' $b"
        }
    }
    if ($bad.Count) {
        $sev = if ($state -eq 'small') { 'low' } else { 'medium' }
        Add-Finding $sev 'overflow' $run.Name "$($bad.Count) element(s) reach outside the window after $state (frame $f)" (($bad | Select-Object -First 6) -join '; ') (Shot "$($run.Key)-$state-overflow")
    }
}

function App-Alive($run) {
    if ($run.Proc.HasExited) {
        Add-Finding 'high' 'crash' $run.Name "the app exited (code $($run.Proc.ExitCode))" (Tail $run.Err) (Shot "$($run.Key)-exited")
        return $false
    }
    return $true
}

function Focus-App($run) {
    if (-not $R::Alive($run.Main)) {
        $w = @($R::WindowsOf($run.Proc.Id, $true)); if ($w.Count) { $run.Main = $w[0] } else { return }
    }
    if ($R::Minimized($run.Main)) { $R::Show($run.Main, 9); Start-Sleep -Milliseconds 300 }
    if ($R::Foreground() -ne $run.Main) {
        [void]$R::Activate($run.Main); Start-Sleep -Milliseconds 250
    }
}

# Run one action and look at what it did: new windows, dialogs, a changed
# window, a minimised or resized window, a crash or a hang. Everything the
# action opened is closed again and the window put back where it was.
function Observe($run, [string]$label, [scriptblock]$action, [switch]$QuietExpected) {
    if (-not (App-Alive $run)) { return 'dead' }
    Focus-App $run
    $before = @($R::WindowsOf($run.Proc.Id, $false))
    $frame = $R::Frame($run.Main)
    $img = $R::Grab($frame)
    $top = $R::Topmost($run.Main)
    try { & $action } catch { if ("$_" -match '^robot:') { throw } ; Write-Host "robot: $label failed: $_" }
    Start-Sleep -Milliseconds 1100
    $effect = @()
    if ($run.Proc.HasExited) {
        Add-Finding 'high' 'crash' $run.Name "the app exited after '$label' (code $($run.Proc.ExitCode))" (Tail $run.Err) (Shot "$($run.Key)-crash")
        if ($img) { $img.Dispose() }
        return 'crash'
    }
    if ($R::IsHung($run.Main)) {
        Start-Sleep -Seconds 4
        if ($R::IsHung($run.Main)) { Add-Finding 'high' 'hang' $run.Name "the window stopped responding after '$label'" '' (Shot "$($run.Key)-hang") }
    }
    $now = @($R::WindowsOf($run.Proc.Id, $false))
    $new = @($now | Where-Object { $before -notcontains $_ })
    if (-not $new.Count -and $label -match '(…|\.\.\.)$|Ctrl\+O') {
        # An item that opens a dialog: the system's Open and Save dialogs can take
        # a few seconds to appear the first time on a slow PC.
        $waitUntil = (Get-Date).AddSeconds(5)
        while (-not $new.Count -and (Get-Date) -lt $waitUntil) {
            Start-Sleep -Milliseconds 200
            $new = @(@($R::WindowsOf($run.Proc.Id, $false)) | Where-Object { $before -notcontains $_ })
        }
        if ($new.Count) { $measures["$($run.Key).dialog_ms.$label"] = [int](5000 - ($waitUntil - (Get-Date)).TotalMilliseconds) + 1100 }
    }
    $shot = ''
    if ($new.Count) {
        $shot = Shot "$($run.Key)-$label"
        $fg = $R::Foreground()
        $descr = @()
        foreach ($w in $new) {
            $descr += "'$($R::Title($w))' [$($R::ClassOf($w))] $($R::Frame($w))"
            Check-Placement $run $w $label
            Check-Visual $run $w $label
        }
        $effect += "window: " + ($descr -join ', ')
        if (($new -notcontains $fg) -and ($R::PidOf($fg) -ne $run.Proc.Id -or $fg -eq $run.Main)) {
            # A new window that is not in front: a panel or dialog the user cannot see.
            $big = $new | Where-Object { $f = $R::Frame($_); $f.Width -gt 120 -and $f.Height -gt 80 }
            if ($big) { Add-Finding 'medium' 'focus' $run.Name "'$label' opened a window that is not in front (foreground: '$($R::Title($fg))' [$($R::ClassOf($fg))])" '' $shot }
        }
        foreach ($w in $new) {
            if (-not $R::Alive($w)) { continue }
            if ($R::ClassOf($w) -eq '#32770') { [void]$R::Activate($w); Keys $VK.Esc } else { $R::Close($w) }
        }
        Start-Sleep -Milliseconds 700
        foreach ($w in $new) {
            if ($R::Alive($w) -and $R::Visible($w)) {
                [void]$R::Activate($w); Keys $VK.Esc; Start-Sleep -Milliseconds 500
                if ($R::Alive($w) -and $R::Visible($w)) {
                    Add-Finding 'medium' 'close' $run.Name "a window opened by '$label' did not close on WM_CLOSE or Esc: '$($R::Title($w))'" '' (Shot "$($run.Key)-$label-stuck")
                }
            }
        }
    }
    $topNow = $R::Alive($run.Main) -and $R::Topmost($run.Main)
    if ($topNow -ne $top) { $effect += "always-on-top $top -> $topNow" }
    # A menu still open after a click on one of its items: the item was disabled
    # (UI Automation does not report menu items' disabled state yet).
    if ((Shell-Menus).Count) { $effect += 'menu stayed open (item disabled?)'; Close-ShellMenus }
    if ($R::Alive($run.Main) -and -not $R::Minimized($run.Main)) { Focus-App $run; Start-Sleep -Milliseconds 250 }
    if (-not $R::Alive($run.Main)) {
        $effect += 'main window closed'
        $w = @($R::WindowsOf($run.Proc.Id, $true)); if ($w.Count) { $run.Main = $w[0] }
    } elseif ($R::Minimized($run.Main) -or -not $R::Visible($run.Main)) {
        $effect += 'minimised/hidden'
        $R::Show($run.Main, 9); Start-Sleep -Milliseconds 500
    } else {
        $f2 = $R::Frame($run.Main)
        if ($f2 -ne $frame) {
            $effect += "frame $frame -> $f2"
            if (-not $shot) { $shot = Shot "$($run.Key)-$label" }
            Check-Placement $run $run.Main $label
            $wa = $R::WorkArea()
            if ($label -match 'Full Screen$') {
                # Real full screen: the app leaves it the way a user would (Walk-Menus
                # chooses the item again); moving the window here would confuse its state.
                $effect += 'full screen'
            } elseif ($f2.Width -ge $wa.Width - 4 -or $f2.Height -ge $wa.Height - 4 -or $label -match '^Window >') {
                # Zoom, Fill or full screen: put the window back. A size the app chose
                # itself (Calculator's Scientific view) stays.
                $R::Show($run.Main, 9); Start-Sleep -Milliseconds 300
                if ($R::Frame($run.Main) -ne $run.Home) { [void]$R::Move($run.Main, $run.Home.X, $run.Home.Y, $run.Home.Width, $run.Home.Height) }
            } else { $run.Home = $f2 }
        } elseif ($img) {
            $img2 = $R::Grab($frame)
            $d = $R::Diff($img, $img2)
            if ($img2) { $img2.Dispose() }
            if ($d -gt 0.004) { $effect += ('content changed {0:P1}' -f $d); if (-not $shot) { $shot = Shot "$($run.Key)-$label" } }
        }
    }
    if ($img) { $img.Dispose() }
    if (-not $effect.Count) {
        if (-not $QuietExpected) {
            Add-Finding 'low' 'no-effect' $run.Name "'$label' showed no visible effect" '' (Shot "$($run.Key)-$label-noeffect")
        }
        $effect = @('none')
    }
    $line = "  $label -> $($effect -join '; ')"
    Write-Host $line
    Add-Content (Join-Path $run.Dir 'actions.txt') $line
    return ($effect -join '; ')
}

# Every menu of the app, from the Lulo bar (Lulo mode) or the in-window strip.
function Walk-Menus($run) {
    Focus-App $run
    Start-Sleep -Milliseconds 400
    $bar = Bar-Window
    $viaBar = ($bar -ne [IntPtr]::Zero) -and ($MenuMode -ne 'strip')
    $menuButtons = @()
    if ($viaBar) {
        $menuButtons = @(Bar-Buttons | Where-Object { $_.Name -match ' menu$' -and $_.Name -ne 'Lulo menu' })
        if (-not ($menuButtons | Where-Object { $_.Name -eq "$($run.Name) menu" })) {
            Add-Finding 'medium' 'menus' $run.Name "the Lulo bar does not show the app's menus (bar: $(($menuButtons | ForEach-Object { $_.Name }) -join ', '))" '' (Shot "$($run.Key)-bar")
        }
    } else {
        # Tap Alt: the strip opens its first menu.
        Keys $VK.Alt; Start-Sleep -Milliseconds 700
        $open = Strip-Menus $run.Main
        if (-not $open.Count) {
            Add-Finding 'high' 'menus' $run.Name 'Alt did not open the in-window menu strip' '' (Shot "$($run.Key)-strip")
            return
        }
        Keys $VK.Esc
        $menuButtons = @($R::Tree($run.Main, 6, 300) | Where-Object { $_.Type -in @('Button', 'MenuItem') -and $_.Name -and $_.Bounds.Y -lt ($R::Frame($run.Main).Y + 40) -and $_.Name -notmatch $WindowControls })
    }
    $catalogue = [ordered]@{}
    foreach ($button in $menuButtons) {
        $menuName = $button.Name -replace ' menu$', ''
        $open = { param($b) $c = Center $b; Click $c[0] $c[1]; Start-Sleep -Milliseconds 600 }
        Focus-App $run
        & $open $button
        $panel = @(if ($viaBar) { Shell-Menus } else { Strip-Menus $run.Main }) | Select-Object -First 1
        if (-not $panel) {
            Add-Finding 'medium' 'menus' $run.Name "clicking '$($button.Name)' opened no menu" '' (Shot "$($run.Key)-menu-$menuName-none")
            continue
        }
        [void](Shot "$($run.Key)-menu-$menuName")
        $items = @(Items-Of $panel)
        $catalogue[$menuName] = @($items | ForEach-Object { if ($_.Enabled) { $_.Name } else { "($($_.Name))" } })
        Close-ShellMenus
        if (-not $viaBar) { Keys $VK.Esc }
        if ($menuName -eq 'Help') { continue } # Help items open web pages: listed, never chosen
        foreach ($item in $items) {
            if (-not $item.Enabled -or -not $item.Name) { continue }
            if ($item.Name -match $Deny) { Add-Content (Join-Path $run.Dir 'actions.txt') "  skipped (deny list): $menuName > $($item.Name)"; continue }
            $label = "$menuName > $($item.Name)"
            $quiet = $item.Name -match $QuietOk
            [void](Observe $run $label {
                & $open $button
                $p = @(if ($viaBar) { Shell-Menus } else { Strip-Menus $run.Main }) | Select-Object -First 1
                $target = @(Items-Of $p | Where-Object { $_.Name -eq $item.Name }) | Select-Object -First 1
                if (-not $target) { throw "item '$($item.Name)' not found again" }
                # Hover first: a submenu opens on hover, and the robot walks it once.
                Guard; $R::MoveTo([int]$target.X, [int]$target.Y); Mark; Start-Sleep -Milliseconds 700
                $menus = @(if ($viaBar) { Shell-Menus } else { Strip-Menus $run.Main })
                if ($menus.Count -gt 1) {
                    $sub = $menus | Where-Object { $_.Name -ne $p.Name } | Select-Object -First 1
                    $subItems = @(Items-Of $sub)
                    $catalogue["$menuName > $($item.Name)"] = @($subItems | ForEach-Object { $_.Name })
                    $first = $subItems | Where-Object { $_.Enabled -and $_.Name -notmatch $Deny } | Select-Object -First 1
                    if ($first) { Click $first.X $first.Y } else { Close-ShellMenus; if (-not $viaBar) { Keys $VK.Esc } }
                } else {
                    Click $target.X $target.Y
                }
            } -QuietExpected:$quiet)
            if ($run.Proc.HasExited) { return $catalogue }
            if ($item.Name -eq 'Enter Full Screen') { Leave-FullScreen $run $button $viaBar $open }
        }
    }
    $catalogue | ConvertTo-Json -Depth 4 | Set-Content -Encoding UTF8 (Join-Path $run.Dir 'menus.json')
    return $catalogue
}

# Leave full screen the way a user does, by the same menu (now "Exit Full
# Screen"), and check the window comes back to where it was.
function Leave-FullScreen($run, $button, $viaBar, $open) {
    $sc = $R::Screen()
    $f = $R::Frame($run.Main)
    if ($f.Width -lt $sc.Width -or $f.Height -lt $sc.Height) { return }
    Focus-App $run
    & $open $button
    $p = @(if ($viaBar) { Shell-Menus } else { Strip-Menus $run.Main }) | Select-Object -First 1
    $exit = @(Items-Of $p | Where-Object { $_.Name -match '^(Exit|Leave|Enter) Full Screen$' }) | Select-Object -First 1
    if (-not $exit) {
        Close-ShellMenus
        Add-Finding 'medium' 'menus' $run.Name "the menu has no 'Exit Full Screen' while the window is full screen" '' (Shot "$($run.Key)-no-exit-full-screen")
        Keys $VK.Esc
    } else {
        if ($exit.Name -ne 'Exit Full Screen') { Add-Finding 'low' 'menus' $run.Name "the full-screen item still reads '$($exit.Name)' while the window is full screen" }
        Click $exit.X $exit.Y
    }
    Start-Sleep -Milliseconds 1200
    $back = $R::Frame($run.Main)
    $line = "  leave full screen -> frame $back"
    Write-Host $line; Add-Content (Join-Path $run.Dir 'actions.txt') $line
    if ($back.Width -ge $sc.Width -and $back.Height -ge $sc.Height) {
        Add-Finding 'high' 'full-screen' $run.Name 'the window could not leave full screen from its menu' '' (Shot "$($run.Key)-stuck-full-screen")
        $R::Show($run.Main, 9)
    } elseif ([math]::Abs($back.Width - $run.Home.Width) -gt 4 -or [math]::Abs($back.Height - $run.Home.Height) -gt 4) {
        Add-Finding 'medium' 'full-screen' $run.Name "after full screen the window is $back, not $($run.Home)" '' (Shot "$($run.Key)-after-full-screen")
    }
}

function Click-Toolbar($run) {
    $f = $R::Frame($run.Main)
    $buttons = @($R::Tree($run.Main, 12, 600) | Where-Object {
        $_.Type -in @('Button', 'CheckBox', 'RadioButton', 'TabItem', 'ListItem') -and $_.Name -and -not $_.Offscreen -and
        $_.Bounds.Height -gt 4 -and $_.Name -notmatch $WindowControls -and $_.Bounds.Y -lt $f.Y + 90
    })
    $seen = @{}
    foreach ($b in $buttons | Select-Object -First 16) {
        if ($seen[$b.Name]) { continue }; $seen[$b.Name] = 1
        if ($b.Name -match $Deny) { Add-Content (Join-Path $run.Dir 'actions.txt') "  skipped (deny list): toolbar $($b.Name)"; continue }
        if (-not $b.Enabled) { continue }
        [void](Observe $run "toolbar $($b.Name)" { $c = Center $b; Click $c[0] $c[1] })
        if ($run.Proc.HasExited) { return }
        # A popover or menu the button opened.
        Keys $VK.Esc; Start-Sleep -Milliseconds 200
    }
}

$Shortcuts = @(
    @{ Label = 'Ctrl+N'; Keys = @(0x11, 0x4E) }
    @{ Label = 'Ctrl+T'; Keys = @(0x11, 0x54) }
    @{ Label = 'Ctrl+F'; Keys = @(0x11, 0x46); Quiet = $true }
    @{ Label = 'Ctrl+A'; Keys = @(0x11, 0x41); Quiet = $true }
    @{ Label = 'Ctrl+C'; Keys = @(0x11, 0x43); Quiet = $true; Clipboard = $true }
    @{ Label = 'Ctrl+Z'; Keys = @(0x11, 0x5A); Quiet = $true }
    @{ Label = 'Ctrl+Comma'; Keys = @(0x11, 0xBC) }
    @{ Label = 'Ctrl+O'; Keys = @(0x11, 0x4F) }
    @{ Label = 'Ctrl+1'; Keys = @(0x11, 0x31); Quiet = $true }
    @{ Label = 'Ctrl+2'; Keys = @(0x11, 0x32); Quiet = $true }
    @{ Label = 'Ctrl+Plus'; Keys = @(0x11, 0xBB); Quiet = $true }
    @{ Label = 'Ctrl+Minus'; Keys = @(0x11, 0xBD); Quiet = $true }
    @{ Label = 'Ctrl+0'; Keys = @(0x11, 0x30); Quiet = $true }
    @{ Label = 'Ctrl+M'; Keys = @(0x11, 0x4D) }
)

function Try-Shortcuts($run, $clip) {
    foreach ($s in $Shortcuts) {
        if ($s.Clipboard -and -not $clip.textOnly) { continue }
        [void](Observe $run $s.Label { Keys $s.Keys } -QuietExpected:([bool]$s.Quiet))
        Keys $VK.Esc; Start-Sleep -Milliseconds 200
        if ($run.Proc.HasExited) { return }
    }
}

function Resize-App($run) {
    Focus-App $run
    if (-not $R::Resizable($run.Main)) {
        $measures["$($run.Key).resizable"] = $false
        Add-Content (Join-Path $run.Dir 'actions.txt') '  fixed-size window: resize and maximise skipped'
        return
    }
    $R::Show($run.Main, 9); Start-Sleep -Milliseconds 300
    $f = $R::Frame($run.Main); $wa = $R::WorkArea()
    # Small: drag the bottom-right corner up and left as far as it goes.
    Guard; $R::Drag($f.Right + 3, $f.Bottom + 3, $f.Left + 60, $f.Top + 60); Mark
    Start-Sleep -Milliseconds 800
    $small = $R::Frame($run.Main)
    $measures["$($run.Key).min_size"] = "$($small.Width)x$($small.Height)"
    [void](Shot "$($run.Key)-small")
    if ($small -eq $f) { Add-Finding 'low' 'resize' $run.Name "dragging the bottom-right corner did not resize the window ($f)" }
    Check-Visual $run $run.Main 'small'
    Check-Bounds $run $run.Main 'small'
    # Large: drag it to the work area's bottom-right corner.
    $s = $R::Frame($run.Main)
    Guard; $R::Drag($s.Right + 3, $s.Bottom + 3, $wa.Right - 4, $wa.Bottom - 4); Mark
    Start-Sleep -Milliseconds 800
    [void](Shot "$($run.Key)-large")
    Check-Placement $run $run.Main 'large'
    Check-Visual $run $run.Main 'large'
    Check-Bounds $run $run.Main 'large'
    # Maximised: must fill the work area exactly (between the bar and the Dock in Lulo mode).
    $R::Show($run.Main, 3); Start-Sleep -Milliseconds 900
    $m = $R::Frame($run.Main)
    [void](Shot "$($run.Key)-maximised")
    if ([math]::Abs($m.Top - $wa.Top) -gt 2 -or [math]::Abs($m.Bottom - $wa.Bottom) -gt 2 -or [math]::Abs($m.Left - $wa.Left) -gt 2 -or [math]::Abs($m.Right - $wa.Right) -gt 2) {
        Add-Finding 'medium' 'placement' $run.Name "maximised frame $m does not match the work area $wa"
    }
    Check-Visual $run $run.Main 'maximised'
    $R::Show($run.Main, 9); Start-Sleep -Milliseconds 400
    [void]$R::Move($run.Main, $run.Home.X, $run.Home.Y, $run.Home.Width, $run.Home.Height)
    Start-Sleep -Milliseconds 400
}

function Idle-Numbers($run) {
    Start-Sleep -Seconds 3
    $c0 = $R::CpuMs($run.Proc.Id); Start-Sleep -Seconds 6; $c1 = $R::CpuMs($run.Proc.Id)
    $idle = [math]::Round($c1 - $c0, 1)
    $measures["$($run.Key).idle_cpu_ms_per_6s"] = $idle
    $mb = [math]::Round($R::PrivateBytes($run.Proc.Id) / 1MB, 1)
    $measures["$($run.Key).private_mb.end"] = $mb
    $measures["$($run.Key).gdi_user"] = $R::GuiObjects($run.Proc.Id)
    $limit = if ($run.Key -eq 'terminal') { 100 } else { 32 }
    if ($idle -gt $limit) { Add-Finding 'medium' 'idle-cpu' $run.Name "$idle ms CPU in 6 s with no input (about $([math]::Round($idle/15.6)) scheduler ticks)" }
    $grow = $mb - $measures["$($run.Key).private_mb.launch"]
    if ($grow -gt 60) { Add-Finding 'medium' 'memory' $run.Name "private bytes grew by $grow MB during the run ($($measures["$($run.Key).private_mb.launch"]) -> $mb MB)" }
}

function Close-App($run) {
    if ($run.Proc.HasExited) { return }
    Focus-App $run
    # Ctrl+W is the Windows spelling of Cmd+W.
    Keys @(0x11, 0x57); Start-Sleep -Seconds 1
    # A "save changes?" sheet: Esc cancels it, then WM_CLOSE / kill below.
    $left = @($R::WindowsOf($run.Proc.Id, $true))
    if ($left.Count) {
        $titles = ($left | ForEach-Object { "'$($R::Title($_))'" }) -join ', '
        Add-Content (Join-Path $run.Dir 'actions.txt') "  Ctrl+W left: $titles"
        foreach ($w in $left) { $R::Close($w) }
        Start-Sleep -Seconds 1
    }
    if (-not $run.Proc.WaitForExit(4000)) {
        # Mac apps stay running with no window; Windows users expect the app to end.
        $run.Proc.Kill(); [void]$run.Proc.WaitForExit(3000)
    }
    $err = Tail $run.Err
    if ($err -match 'panicked at') { Add-Finding 'high' 'crash' $run.Name 'a thread panicked during the run' $err }
}

function Exercise-App($key, $clip) {
    Write-Host "robot: === $key"
    $run = Start-App $key
    if (-not $run) { return }
    try {
        [void](Walk-Menus $run)
        if (App-Alive $run) { Click-Toolbar $run }
        if (App-Alive $run) { Try-Shortcuts $run $clip }
        if (App-Alive $run) { Resize-App $run }
        if (App-Alive $run) { Check-Visual $run $run.Main 'exercise'; Idle-Numbers $run }
    } finally {
        Close-App $run
        Close-ShellMenus
    }
}

# ---- The Lulo layer ---------------------------------------------------------

function Shell-Observe([string]$label, [scriptblock]$action, [string]$expect) {
    $pidShell = Shell-Pid
    $before = @($R::ZOrder())
    $img = $R::Grab($R::Screen())
    try { & $action } catch { if ("$_" -match '^robot:') { throw }; Write-Host "robot: $label failed: $_" }
    Start-Sleep -Milliseconds 1000
    $shot = Shot "shell-$label"
    $after = @($R::ZOrder())
    $new = @($after | Where-Object { $before -notcontains $_ })
    $img2 = $R::Grab($R::Screen())
    $d = $R::Diff($img, $img2)
    $img.Dispose(); $img2.Dispose()
    $menus = @(Shell-Menus)
    $items = @(); foreach ($m in $menus) { $items += @(Items-Of $m | ForEach-Object { $_.Name }) }
    $line = "  $label -> new windows: $(($new | ForEach-Object { "'$($R::Title($_))' [$($R::ClassOf($_))] pid $($R::PidOf($_))" }) -join ', '); screen changed $([math]::Round($d*100,1))%; menu items: $($items -join ' | ')"
    Write-Host $line; Add-Content (Join-Path $out 'shell-actions.txt') $line
    if (-not (Shell-Pid)) { Add-Finding 'high' 'crash' 'Lulo layer' "lulo-shell exited after '$label'" '' $shot }
    elseif ($d -lt 0.001 -and -not $new.Count) { Add-Finding 'medium' 'no-effect' 'Lulo layer' "'$label' showed nothing ($expect expected)" '' $shot }
    return @{ New = $new; Items = $items; Shot = $shot; Diff = $d }
}

function Escape-All { for ($i = 0; $i -lt 2; $i++) { Keys $VK.Esc; Start-Sleep -Milliseconds 250 } }

function Exercise-Shell {
    Write-Host "robot: === Lulo layer"
    $pidShell = Shell-Pid
    if (-not $pidShell) { Add-Finding 'info' 'shell' 'Lulo layer' 'Lulo mode is off: the layer checks were skipped'; return }
    $mb0 = [math]::Round($R::PrivateBytes($pidShell) / 1MB, 1)
    $measures['shell.private_mb.start'] = $mb0
    $buttons = @(Bar-Buttons)
    $names = $buttons | ForEach-Object { $_.Name }
    $measures['shell.bar_buttons'] = $names -join ' | '
    # The Lulo menu: listed and its harmless items opened.
    $lulo = $buttons | Where-Object { $_.Name -eq 'Lulo menu' } | Select-Object -First 1
    if ($lulo) {
        $r = Shell-Observe 'lulo-menu' { $c = Center $lulo; Click $c[0] $c[1] } 'the Lulo menu'
        $measures['shell.lulo_menu'] = $r.Items -join ' | '
        Close-ShellMenus
        foreach ($name in $r.Items) {
            if (-not $name -or $name -match $Deny -or $name -notmatch '(?i)^about') { continue }
            $res = Shell-Observe "lulo-menu-$name" {
                $c = Center $lulo; Click $c[0] $c[1]; Start-Sleep -Milliseconds 600
                $it = @(Shell-Menus | ForEach-Object { Items-Of $_ } | Where-Object { $_.Name -eq $name }) | Select-Object -First 1
                if ($it) { Click $it.X $it.Y }
            } "a window for '$name'"
            foreach ($w in $res.New) { if ($R::Title($w)) { $R::Close($w) } }
            Escape-All
        }
    } else { Add-Finding 'high' 'shell' 'Lulo layer' 'the bar has no Lulo menu' }
    # Status items: open each, look, close. Nothing inside them is clicked.
    foreach ($b in $buttons | Where-Object { $_.Name -notmatch ' menu$' }) {
        $short = ($b.Name -split '[ ,:]')[0]
        $r = Shell-Observe "status-$short" { $c = Center $b; Click $c[0] $c[1] } "a panel for '$($b.Name)'"
        foreach ($w in $r.New) { if ($R::PidOf($w) -eq $pidShell) { Check-ShellPanel $w "status-$short" } }
        Escape-All
        Start-Sleep -Milliseconds 300
        $still = @($r.New | Where-Object { $R::Alive($_) -and $R::Visible($_) -and $R::PidOf($_) -eq $pidShell })
        if ($still.Count) { Add-Finding 'medium' 'close' 'Lulo layer' "Esc did not close the panel opened by '$($b.Name)'" '' (Shot "shell-status-$short-stuck"); Click ($R::Screen().Width / 2) ($R::Screen().Height / 2 - 100); Escape-All }
    }
    # Spotlight from the keyboard.
    $r = Shell-Observe 'spotlight-alt-space' { Keys @(0x12, 0x20) } 'Spotlight'
    if (-not ($r.New | Where-Object { $R::PidOf($_) -eq $pidShell })) {
        Add-Finding 'medium' 'spotlight' 'Lulo layer' 'Alt+Space did not open a Spotlight window'
    } else {
        Guard; $R::Type('calcul'); Mark; Start-Sleep -Milliseconds 900
        [void](Shot 'shell-spotlight-typed')
        foreach ($w in $r.New) { if ($R::PidOf($w) -eq $pidShell) { Check-ShellPanel $w 'spotlight' } }
    }
    Escape-All
    # The Dock: one Lulo app opened from its tile and closed; a right-click menu looked at.
    $dock = $null
    foreach ($w in $R::WindowsOf($pidShell, $false)) { $t = $R::Tree($w, 3, 60); if ($t | Where-Object { $_.Name -eq 'Dock' }) { $dock = $t; break } }
    if ($dock) {
        $tiles = @($dock | Where-Object { $_.Type -eq 'Button' })
        $measures['shell.dock_tiles'] = ($tiles | ForEach-Object { $_.Name }) -join ' | '
        $editor = $tiles | Where-Object { $_.Name -match '^Text Editor' } | Select-Object -First 1
        if ($editor -and -not (Get-Process rmac-text-editor -ErrorAction SilentlyContinue)) {
            $t0 = Get-Date
            $c = Center $editor; Click $c[0] $c[1]
            $p = $null
            while (((Get-Date) - $t0).TotalSeconds -lt 20) {
                $p = Get-Process rmac-text-editor -ErrorAction SilentlyContinue | Select-Object -First 1
                if ($p -and @($R::WindowsOf($p.Id, $true)).Count) { break }
                Start-Sleep -Milliseconds 50
            }
            $ms = [int]((Get-Date) - $t0).TotalMilliseconds
            $measures['shell.dock_text_editor_ms'] = $ms
            Start-Sleep -Milliseconds 800
            $shot = Shot 'shell-dock-text-editor'
            if ($p) {
                $w = @($R::WindowsOf($p.Id, $true))[0]
                if ($R::PidOf($R::Foreground()) -ne $p.Id) { Add-Finding 'medium' 'focus' 'Lulo layer' 'Text Editor opened from the Dock is not in front' '' $shot }
                $R::Close($w); [void]$p.WaitForExit(4000)
                if (-not $p.HasExited) { Keys $VK.Esc; $R::Close($w); [void]$p.WaitForExit(3000) }
                if (-not $p.HasExited) { $p.Kill() }
            } else { Add-Finding 'high' 'dock' 'Lulo layer' 'clicking the Text Editor tile opened no window in 20 s' '' $shot }
        }
        if ($editor) {
            $r = Shell-Observe 'dock-right-click' { $c = Center $editor; RClick $c[0] $c[1] } 'the tile menu'
            $measures['shell.dock_tile_menu'] = $r.Items -join ' | '
            Close-ShellMenus; Escape-All
        }
    } else { Add-Finding 'high' 'dock' 'Lulo layer' 'no Dock found' }
    # The desktop: a click and a right-click on empty space (left third, below the bar).
    $wa = $R::WorkArea()
    $x = $wa.Left + [int]($wa.Width * 0.3); $y = $wa.Top + [int]($wa.Height * 0.45)
    $covered = $R::ZOrder() | Where-Object { $f = $R::Frame($_); $R::ClassOf($_) -notmatch 'Zed::Window|LuloWallpaperLayer|Progman|WorkerW|DummyDWM|CoreWindow' -and $f.Contains($x, $y) -and $f.Width -lt $R::Screen().Width }
    if ($covered) { Write-Host "robot: the desktop point is covered by another window; skipping desktop clicks" }
    else {
        [void](Shell-Observe 'desktop-click' { Click $x $y } 'nothing (selection cleared)')
        $r = Shell-Observe 'desktop-right-click' { RClick $x $y } 'the desktop menu'
        $measures['shell.desktop_menu'] = $r.Items -join ' | '
        Close-ShellMenus; Escape-All
    }
    # A folder double-clicked in Files (the scratch folder stands in for the desktop: the user's Desktop is not ours to touch).
    Exercise-FolderOpen
    $measures['shell.private_mb.end'] = [math]::Round($R::PrivateBytes($pidShell) / 1MB, 1)
    Start-Sleep -Seconds 3
    $c0 = $R::CpuMs($pidShell); Start-Sleep -Seconds 6; $c1 = $R::CpuMs($pidShell)
    $measures['shell.idle_cpu_ms_per_6s'] = [math]::Round($c1 - $c0, 1)
    if ($c1 - $c0 -gt 32) { Add-Finding 'medium' 'idle-cpu' 'Lulo layer' "lulo-shell used $([math]::Round($c1-$c0,1)) ms CPU in 6 s with no input" }
}

function Check-ShellPanel($hwnd, $label) {
    $f = $R::Frame($hwnd)
    $sc = $R::Screen()
    if ($f.Right -gt $sc.Right + 1 -or $f.Left -lt -1 -or $f.Bottom -gt $sc.Bottom + 1) {
        Add-Finding 'medium' 'placement' 'Lulo layer' "the $label panel reaches off screen ($f)"
    }
    if ($f.Width -lt $sc.Width) {
        $b = $R::Grab($f); $a = $R::Analyze($b); if ($b) { $b.Dispose() }
        if ($a.Count -and $a['black'] -gt 0.9) { Add-Finding 'high' 'visual' 'Lulo layer' "the $label panel is black" }
    }
}

function Exercise-FolderOpen {
    $exe = Join-Path $Bin 'rmac-files.exe'
    if (Get-Process rmac-files -ErrorAction SilentlyContinue) { return }
    $run = New-Run 'files-folder' 'Files'
    $run.Proc = $R::Launch($exe, "`"$fixtures`"", $fixtures, (Profile-Env $run), $run.Err)
    $t0 = Get-Date
    while (((Get-Date) - $t0).TotalSeconds -lt 20 -and -not @($R::WindowsOf($run.Proc.Id, $true)).Count) { Start-Sleep -Milliseconds 50 }
    $w = @($R::WindowsOf($run.Proc.Id, $true))
    if (-not $w.Count) { Add-Finding 'high' 'launch' 'Files' 'no window opening the scratch folder'; if (-not $run.Proc.HasExited) { $run.Proc.Kill() }; return }
    $run.Main = $w[0]; Start-Sleep -Seconds 1.5
    $run.Home = $R::Frame($run.Main)
    $folder = @($R::Tree($run.Main, 14, 800) | Where-Object { $_.Name -match '^Robot Folder' -and -not $_.Offscreen }) | Select-Object -First 1
    if (-not $folder) {
        Add-Finding 'medium' 'files' 'Files' "'Robot Folder' is not in the accessibility tree of the scratch folder's window" '' (Shot 'files-folder-missing')
    } else {
        $title0 = $R::Title($run.Main)
        [void](Observe $run 'double-click Robot Folder' { $c = Center $folder; DClick $c[0] $c[1] })
        $inner = @($R::Tree($run.Main, 14, 800) | Where-Object { $_.Name -match '^(Inner|inside\.txt)' }).Count
        if (-not $inner) { Add-Finding 'medium' 'files' 'Files' "double-clicking 'Robot Folder' did not show its contents (title '$title0' -> '$($R::Title($run.Main))')" '' (Shot 'files-folder-open') }
    }
    Close-App $run
}

# Turn Lulo off and on again, and check the desktop is given back and taken again.
function Toggle-Lulo {
    Write-Host "robot: === Lulo off and on"
    $session = Get-Process lulo-session -ErrorAction SilentlyContinue | Select-Object -First 1
    if (-not $session) { Add-Finding 'info' 'toggle' 'Lulo layer' 'Lulo mode was off; the toggle check needs it on and was skipped'; return }
    $onState = Lulo-State
    [void](Shot 'toggle-on-before')
    $exe = Join-Path $Bin 'lulo-session.exe'
    & $exe --stop 2>&1 | Out-Null
    $t0 = Get-Date
    while (((Get-Date) - $t0).TotalSeconds -lt 20 -and (Get-Process lulo-session, lulo-shell -ErrorAction SilentlyContinue)) { Start-Sleep -Milliseconds 100 }
    $measures['toggle.stop_ms'] = [int]((Get-Date) - $t0).TotalMilliseconds
    Start-Sleep -Seconds 2
    $off = Lulo-State
    $measures['toggle.off'] = $off
    $shotOff = Shot 'toggle-off'
    if (Get-Process lulo-session, lulo-shell -ErrorAction SilentlyContinue) { Add-Finding 'high' 'toggle' 'Lulo layer' 'Lulo processes still running 20 s after --stop' '' $shotOff }
    if (-not $off.taskbarVisible) { Add-Finding 'high' 'toggle' 'Lulo layer' 'the Windows taskbar is not visible after turning Lulo off' '' $shotOff }
    $sc = $R::Screen(); $wa = $R::WorkArea()
    if ($wa.Top -ne 0) { Add-Finding 'high' 'toggle' 'Lulo layer' "the work area still starts below the bar after turning Lulo off ($wa)" '' $shotOff }
    # On again, the way the user starts it (no arguments, the user's own environment).
    Start-Process -FilePath $exe -WorkingDirectory $Bin
    $t0 = Get-Date
    while (((Get-Date) - $t0).TotalSeconds -lt 30 -and -not ((Bar-Window) -ne [IntPtr]::Zero)) { Start-Sleep -Milliseconds 100 }
    $measures['toggle.start_to_bar_ms'] = [int]((Get-Date) - $t0).TotalMilliseconds
    Start-Sleep -Seconds 4
    $on = Lulo-State
    $measures['toggle.on_again'] = $on
    $shotOn = Shot 'toggle-on-again'
    if (-not $on.on) { Add-Finding 'high' 'toggle' 'Lulo layer' 'lulo-session did not start again' '' $shotOn }
    if ($on.workArea -ne $onState.workArea) { Add-Finding 'medium' 'toggle' 'Lulo layer' "work area after turning Lulo on again is $($on.workArea), was $($onState.workArea)" '' $shotOn }
    if ($on.taskbarVisible -ne $onState.taskbarVisible) { Add-Finding 'medium' 'toggle' 'Lulo layer' "taskbar visibility after turning Lulo on again is $($on.taskbarVisible), was $($onState.taskbarVisible)" '' $shotOn }
}

# Windows Error Reporting and .NET/app crash records for Lulo's exes since the run began.
function Scan-EventLog {
    $events = Get-WinEvent -FilterHashtable @{ LogName = 'Application'; StartTime = $started.AddSeconds(-5) } -ErrorAction SilentlyContinue |
        Where-Object { $_.Level -le 2 -or $_.ProviderName -in @('Windows Error Reporting', 'Application Hang', 'Application Error') } |
        Where-Object { $_.Message -match '(?i)\b(rmac|lulo)-[a-z-]+\.exe' }
    foreach ($e in $events) {
        $exe = [regex]::Match($e.Message, '(?i)\b(rmac|lulo)-[a-z-]+\.exe').Value
        $msg = ($e.Message -split "`n" | Select-Object -First 6) -join ' '
        Add-Finding 'high' 'event-log' $exe "$($e.ProviderName) event $($e.Id) at $($e.TimeCreated.ToString('HH:mm:ss'))" $msg.Substring(0, [math]::Min(400, $msg.Length))
    }
}

# ---- Main -----------------------------------------------------------------

$initial = $null; $clip = $null
try {
    Wait-ForIdle
    $other = Other-TestRunning
    if ($other) { throw "robot: another Lulo test is running ($other); not starting" }
    $initial = Lulo-State
    $measures['initial'] = $initial
    "robot: start $stamp, Lulo mode $(if ($initial.on) { 'on' } else { 'off' }), screen $($R::Screen()), work area $($R::WorkArea())"
    $clip = Saved-Clipboard
    $measures['clipboard_text_only'] = $clip.textOnly
    Write-Fixtures
    Mark
    $keys = if ($Apps -eq 'all') { @($AppTable.Keys) } elseif ($Apps) { $Apps -split '[,\s]+' | Where-Object { $_ } } else { @() }
    foreach ($key in $keys) {
        if (-not $AppTable.Contains($key)) { "robot: unknown app '$key'"; continue }
        Exercise-App $key $clip
    }
    if ($Shell) { Exercise-Shell }
    if ($Toggle) { Toggle-Lulo }
} catch {
    "robot: stopped: $_"
    Add-Finding 'info' 'robot' 'robot' "run stopped early: $_"
} finally {
    # Leave the desktop as it was: our apps closed, menus closed, Lulo mode as found, the clipboard back.
    foreach ($spec in $AppTable.Values) {
        Get-Process ($spec.Exe -replace '\.exe$', '') -ErrorAction SilentlyContinue | Where-Object { $_.Path -and $_.Path.StartsWith($Bin, [StringComparison]::OrdinalIgnoreCase) -and $_.StartTime -gt $started } | ForEach-Object { "robot: closing leftover $($_.Name) $($_.Id)"; $_.Kill() }
    }
    try { Close-ShellMenus } catch { }
    if ($initial -and $initial.on -and -not (Get-Process lulo-session -ErrorAction SilentlyContinue)) {
        "robot: Lulo mode was on; starting it again"
        Start-Process -FilePath (Join-Path $Bin 'lulo-session.exe') -WorkingDirectory $Bin
    }
    if ($initial -and -not $initial.on -and (Get-Process lulo-session -ErrorAction SilentlyContinue | Where-Object { $_.StartTime -gt $started })) {
        "robot: Lulo mode was off; turning it off again"
        & (Join-Path $Bin 'lulo-session.exe') --stop 2>&1 | Out-Null
    }
    if ($clip) { Restore-Clipboard $clip }
    Scan-EventLog
    $report = [ordered]@{ started = $started.ToString('s'); seconds = [int]((Get-Date) - $started).TotalSeconds; bin = $Bin; menuMode = $MenuMode; measures = $measures; findings = $findings }
    $report | ConvertTo-Json -Depth 6 | Set-Content -Encoding UTF8 (Join-Path $out 'findings.json')
    "robot: done in $($report.seconds) s, $($findings.Count) finding(s), results in $out"
}
