# Stop the installed Lulo layer and run a test build (-Bin) with the shell
# trace on and a fresh private profile under -Root, so nothing the robot
# does writes outside E:\lulo: the profile's own Lulo settings send
# screenshots (XDG_DESKTOP_DIR) and Lulo's desktop (RMAC_DESKTOP_DIR) to
# -Root\desktop. restore-mine.ps1 (lulo-session --stop, then the
# installed one) puts the installed layer back. Run it through invoke.ps1.
param(
    [string]$Bin = 'E:\lulo\winpar',
    [string]$Root = 'E:\lulo\winpar'
)
$installed = "$env:LOCALAPPDATA\Programs\Lulo"
& "$installed\lulo-session.exe" --stop 2>&1 | Out-Null
Start-Sleep 4
$prof = "$Root\profile"
if (Test-Path $prof) { Remove-Item -Recurse -Force $prof }
$desk = "$Root\desktop"
New-Item -ItemType Directory -Force "$prof\Roaming\Lulo\Config", "$prof\Local", "$prof\Temp", $desk | Out-Null
# Lulo's own folder settings, as on Lulo OS (`user-dirs.dirs`).
$forward = $desk -replace '\\', '/'
Set-Content -Encoding ASCII "$prof\Roaming\Lulo\Config\user-dirs.dirs" "XDG_DESKTOP_DIR=`"$forward`""
$env:APPDATA = "$prof\Roaming"; $env:LOCALAPPDATA = "$prof\Local"; $env:TEMP = "$prof\Temp"; $env:TMP = "$prof\Temp"
# Lulo's desktop shows the same scratch folder (the drag-out check's files).
$env:RMAC_DESKTOP_DIR = $desk
$env:LULO_SHELL_TRACE = '1'
Start-Process "$Bin\lulo-session.exe" -RedirectStandardError "$Root\trace.txt"
