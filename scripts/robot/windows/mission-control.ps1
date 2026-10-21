# Mission Control and App Exposé on a real PC: opens two Lulo apps of its
# own (private profile, scratch files only), shows Mission Control (Ctrl+Up)
# and App Exposé (Ctrl+Down), takes a screenshot of each, checks that each
# app window got a picture, and closes everything again. Same safety rules
# as robot.ps1. Run it through invoke.ps1.
param(
    [string]$Bin = "$env:LOCALAPPDATA\Programs\Lulo",
    [string]$Scratch = 'E:\lulo\robot-scratch'
)
$ErrorActionPreference = 'Continue'
. (Join-Path (Split-Path -Parent $MyInvocation.MyCommand.Path) 'common.ps1')
$out = Join-Path $Scratch ("mission-control\" + (Get-Date).ToString('yyyyMMdd-HHmmss'))
$prof = Join-Path $out 'profile'
New-Item -ItemType Directory -Force $out, "$prof\Roaming", "$prof\Local", "$prof\Temp" | Out-Null
$env:APPDATA = "$prof\Roaming"; $env:LOCALAPPDATA = "$prof\Local"; $env:TEMP = "$prof\Temp"; $env:TMP = "$prof\Temp"
$note = Join-Path $out 'mission control note.txt'
Set-Content -Encoding UTF8 $note 'Mission Control check.'
$result = [ordered]@{}

while ($R::IdleMs() -lt 120000) { Start-Sleep -Seconds 20 }
$started = @()
try {
    $started += Start-Process (Join-Path $Bin 'rmac-calculator.exe') -PassThru
    $started += Start-Process (Join-Path $Bin 'rmac-text-editor.exe') -ArgumentList "`"$note`"" -PassThru
    Start-Sleep -Seconds 4
    $shell = (Get-Process lulo-shell | Select-Object -First 1).Id
    $front = $R::PidOf($R::Foreground())
    $result['front'] = $R::ProcessName($front)
    if (-not ($started.Id -contains $front)) { throw 'not one of our apps in front; not sending keys' }
    foreach ($step in @(@('mission-control', @($VK.Ctrl, $VK.Up)), @('app-expose', @($VK.Ctrl, $VK.Down)))) {
        $front = $R::PidOf($R::Foreground())
        if (-not ($started.Id -contains $front -or $front -eq $shell)) { throw 'someone else is in front' }
        $R::Chord($step[1]); Start-Sleep -Milliseconds 900
        $R::Shot((Join-Path $out "$($step[0]).jpg"))
        $tree = @(foreach ($w in $R::WindowsOf($shell, $false)) { $R::Tree($w, 6, 200) })
        $result[$step[0]] = (@($tree | Where-Object { $_.Type -eq 'Button' -and $_.Name } | ForEach-Object { $_.Name }) -join ' | ')
        $R::Chord(@($VK.Esc)); Start-Sleep -Milliseconds 900
    }
} catch { $result['stopped'] = "$_" }
finally {
    foreach ($p in $started) { if (-not $p.HasExited) { $p.CloseMainWindow() | Out-Null; Start-Sleep -Milliseconds 800; if (-not $p.HasExited) { $p.Kill() } } }
}
$result | ConvertTo-Json | Tee-Object (Join-Path $out 'result.json')
