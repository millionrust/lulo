# Click wallpaper to reveal the desktop, on a real PC: opens two Lulo apps
# of its own (private profile, scratch files only), clicks bare Lulo
# desktop, checks the windows slid to the screen edges (and a frame from
# the middle of the slide), clicks the desktop again and checks they came
# back; then pushes them aside once more and brings them back by clicking
# a window's sliver. Same safety rules as robot.ps1. Run it through
# invoke.ps1.
param(
    [string]$Bin = "$env:LOCALAPPDATA\Programs\Lulo",
    [string]$Scratch = 'E:\lulo\robot-scratch'
)
$ErrorActionPreference = 'Continue'
. (Join-Path (Split-Path -Parent $MyInvocation.MyCommand.Path) 'common.ps1')
$out = Join-Path $Scratch ("reveal\" + (Get-Date).ToString('yyyyMMdd-HHmmss'))
$prof = Join-Path $out 'profile'
New-Item -ItemType Directory -Force $out, "$prof\Roaming", "$prof\Local", "$prof\Temp" | Out-Null
$env:APPDATA = "$prof\Roaming"; $env:LOCALAPPDATA = "$prof\Local"; $env:TEMP = "$prof\Temp"; $env:TMP = "$prof\Temp"
$note = Join-Path $out 'reveal note.txt'
Set-Content -Encoding UTF8 $note 'Click wallpaper to reveal the desktop.'
$result = [ordered]@{}
function Frames($procs) { @($procs | ForEach-Object { $w = @($R::WindowsOf($_.Id, $true)) | Select-Object -First 1; if ($w) { "$($R::Frame($w))" } }) -join ' ; ' }
function Shell-Pid { (Get-Process lulo-shell -ErrorAction SilentlyContinue | Select-Object -First 1).Id }
# A point of bare Lulo desktop: a screen spot whose window is lulo-shell's.
function Desktop-Point {
    $screen = $R::Screen()
    foreach ($y in 120, 300, 450) { foreach ($x in 40, 200, ($screen.Width - 40), ($screen.Width / 2)) {
        if ($R::PidAt([int]$x, [int]$y) -eq (Shell-Pid)) { return @([int]$x, [int]$y) }
    } }
    return $null
}

while ($R::IdleMs() -lt 120000) { Start-Sleep -Seconds 20 }
$started = @()
try {
    $started += Start-Process (Join-Path $Bin 'rmac-calculator.exe') -PassThru
    $started += Start-Process (Join-Path $Bin 'rmac-text-editor.exe') -ArgumentList "`"$note`"" -PassThru
    Start-Sleep -Seconds 4
    $result['before'] = Frames $started
    $point = Desktop-Point
    if (-not $point) { throw 'no bare desktop to click' }
    $result['click_at'] = "$($point[0]),$($point[1])"
    $R::Click($point[0], $point[1])
    Start-Sleep -Milliseconds 120
    $R::Shot((Join-Path $out 'sliding.jpg'))
    $result['mid_slide'] = Frames $started
    Start-Sleep -Milliseconds 700
    $R::Shot((Join-Path $out 'revealed.jpg'))
    $result['revealed'] = Frames $started
    # The wallpaper again brings them back.
    $point = Desktop-Point
    if ($point) { $R::Click($point[0], $point[1]) }
    Start-Sleep -Milliseconds 800
    $R::Shot((Join-Path $out 'back.jpg'))
    $result['back'] = Frames $started
    # Aside once more, then back by clicking a window's sliver.
    $point = Desktop-Point
    if ($point) { $R::Click($point[0], $point[1]) }
    Start-Sleep -Milliseconds 800
    $w = @($R::WindowsOf($started[0].Id, $true)) | Select-Object -First 1
    $f = $R::Frame($w)
    $screen = $R::Screen()
    $x = [math]::Min([math]::Max($f.X + $f.Width / 2, 2), $screen.Width - 3)
    $x = if ($f.X -lt 0) { $f.X + $f.Width - 4 } elseif ($f.X + $f.Width -gt $screen.Width) { $f.X + 4 } else { $x }
    $y = [math]::Min([math]::Max($f.Y + 20, 40), $screen.Height - 3)
    if ($started.Id -contains $R::PidAt([int]$x, [int]$y)) { $R::Click([int]$x, [int]$y); $result['sliver_click'] = "$([int]$x),$([int]$y)" }
    Start-Sleep -Milliseconds 800
    $result['after_sliver'] = Frames $started
} catch { $result['stopped'] = "$_" }
finally {
    foreach ($p in $started) { if (-not $p.HasExited) { $p.CloseMainWindow() | Out-Null; Start-Sleep -Milliseconds 800; if (-not $p.HasExited) { $p.Kill() } } }
}
$result | ConvertTo-Json | Tee-Object (Join-Path $out 'result.json')
