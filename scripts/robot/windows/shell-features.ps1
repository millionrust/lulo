# The Lulo layer's Lulo OS features on a real PC, with one Lulo app of its
# own open (private profile): the minimise effect into a Dock tile with the
# window's picture, Force Quit Applications (Ctrl+Alt+Esc), and the
# keyboard focus shortcuts (Ctrl+F2, Ctrl+F3). Screenshots and a JSON
# result go to the scratch folder. Same safety rules as robot.ps1: keys go
# only to Lulo, every window it opens is closed. Run it through invoke.ps1.
param(
    [string]$Bin = "$env:LOCALAPPDATA\Programs\Lulo",
    [string]$Scratch = 'E:\lulo\robot-scratch'
)
$ErrorActionPreference = 'Continue'
. (Join-Path (Split-Path -Parent $MyInvocation.MyCommand.Path) 'common.ps1')
$out = Join-Path $Scratch ("features\" + (Get-Date).ToString('yyyyMMdd-HHmmss'))
$prof = Join-Path $out 'profile'
New-Item -ItemType Directory -Force $out, "$prof\Roaming", "$prof\Local", "$prof\Temp" | Out-Null
$env:APPDATA = "$prof\Roaming"; $env:LOCALAPPDATA = "$prof\Local"; $env:TEMP = "$prof\Temp"; $env:TMP = "$prof\Temp"
$result = [ordered]@{}
function Shell-Pid { (Get-Process lulo-shell -ErrorAction SilentlyContinue | Select-Object -First 1).Id }
function Lulo-Front { $id = $R::PidOf($R::Foreground()); $id -eq (Shell-Pid) -or $script:app.Id -eq $id }
function Dock-Tiles {
    foreach ($w in $R::WindowsOf((Shell-Pid), $false)) {
        $tree = $R::Tree($w, 3, 80)
        if ($tree | Where-Object { $_.Name -eq 'Dock' }) { return @($tree | Where-Object { $_.Type -eq 'Button' } | ForEach-Object { $_.Name }) }
    }
    return @()
}

while ($R::IdleMs() -lt 120000) { Start-Sleep -Seconds 20 }
$script:app = Start-Process (Join-Path $Bin 'rmac-calculator.exe') -PassThru
try {
    Start-Sleep -Seconds 4
    if (-not (Lulo-Front)) { throw 'not Lulo in front; not sending keys' }
    # Minimise with Ctrl+M (the Mac's ⌘M): the window pours into its tile.
    $R::Chord(@($VK.Ctrl, 0x4D))
    Start-Sleep -Milliseconds 200
    $R::Shot((Join-Path $out 'minimising.jpg'))
    Start-Sleep -Milliseconds 900
    $R::Shot((Join-Path $out 'minimised.jpg'))
    $result['dock_after_minimise'] = (Dock-Tiles) -join ' | '
    # lulo-shell's runtime folder (%TEMP%\Lulo\runtime of the signed-in user).
    $runtime = Join-Path $env:USERPROFILE 'AppData\Local\Temp\Lulo'
    $thumbs = Get-ChildItem -Recurse -Filter '*.png' -Path $runtime -ErrorAction SilentlyContinue | Where-Object { $_.FullName -match 'thumbnails' -and $_.LastWriteTime -gt (Get-Date).AddMinutes(-1) }
    $result['thumbnails'] = @($thumbs | ForEach-Object { $_.Name }) -join ', '
    # Force Quit Applications.
    $before = @($R::ZOrder())
    if (-not (Lulo-Front) -and $R::PidAt(40, 120) -eq (Shell-Pid)) { $R::Click(40, 120); Start-Sleep -Milliseconds 300 }
    if (Lulo-Front) { $R::Chord(@($VK.Ctrl, $VK.Alt, $VK.Esc)) } else { $result['force_quit_window'] = 'skipped: Lulo not in front' }
    Start-Sleep -Milliseconds 1200
    $new = @($R::ZOrder() | Where-Object { $before -notcontains $_ -and $R::PidOf($_) -eq (Shell-Pid) -and $R::Title($_) })
    $result['force_quit_window'] = ($new | ForEach-Object { $R::Title($_) }) -join ', '
    $R::Shot((Join-Path $out 'force-quit.jpg'))
    foreach ($w in $new) { $R::Close($w) }
    Start-Sleep -Milliseconds 500
} catch { $result['stopped'] = "$_" }
finally {
    if (-not $script:app.HasExited) { $script:app.Kill() }
}
$result | ConvertTo-Json | Tee-Object (Join-Path $out 'result.json')
