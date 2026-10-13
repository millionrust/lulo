# The Windows robot

A robot tester for Lulo on Windows. It drives Lulo's apps and the Lulo layer
on a real, signed-in Windows desktop with real keyboard and mouse input
(`SendInput`), reads every window through UI Automation, and writes what
looks wrong to `findings.json`, with screenshots.

It complements the CI checks (`scripts/windows/launch_smoke.py`,
`shell_smoke.py`): those prove fixed paths on a clean runner; the robot
walks *everything* it can reach on a real PC and reports what it could not
explain. Every fix for something the robot found gets a CI check or a unit
test of its own.

## Files

| File | What it is |
| --- | --- |
| `robot.ps1` | The robot: preflight, the app walk, the Lulo layer, the on/off toggle, the report. |
| `RobotLib.cs` | Native helpers compiled at run time with `Add-Type` (C# 5): input, windows, screenshots and their analysis, process numbers, UI Automation. |
| `common.ps1` | Loads `RobotLib.cs`; shared key codes. |
| `invoke.ps1` | Runs a robot script on the signed-in desktop from an SSH session, through a one-off scheduled task with an Interactive logon. |

Windows PowerShell 5.1 and .NET Framework only: nothing to install.

## Running it

From the Mac (or any SSH client), copy this folder to the PC and run:

```
powershell -NoProfile -ExecutionPolicy Bypass -File <dir>\invoke.ps1 `
    -Script <dir>\robot.ps1 -Arguments "-Apps calculator,notes"
```

Arguments:

- `-Apps all` or a list of `calculator, notes, text-editor, preview, clock,
  weather, terminal, files, system-settings`;
- `-Shell`: the Lulo layer (bar menus, status items, Spotlight, the Dock,
  the desktop, a folder opened in Files);
- `-Toggle`: Lulo off and on again, checking the taskbar and work area;
- `-MenuMode strip`: walk the in-window menu strip instead of the Lulo bar
  (the app is started with a user name the bar does not serve);
- `-Bin`: the Lulo install (default `%LOCALAPPDATA%\Programs\Lulo`);
- `-Scratch`: the robot's only folder (default `E:\lulo\robot-scratch`):
  fixtures, the apps' private profile and `out\<timestamp>\` results;
- `-MaxMinutes` (default 10): the run stops itself after this.

Keep each run short (one or two apps, or `-Shell`, or `-Toggle`).

## What it does

For each app:

1. starts it with private `%APPDATA%`/`%LOCALAPPDATA%`/`%TEMP%` under the
   scratch folder, opening only files the robot wrote there, and times the
   first window;
2. checks the window is in front, inside the work area, not black or blank,
   and that no named UI Automation element lies outside the window;
3. opens every menu of the app (in the Lulo bar in Lulo mode, else the
   in-window strip), records its items in `menus.json`, and chooses every
   item that is not on the deny list, hovering first so a submenu is listed
   and its first harmless item chosen;
4. clicks the toolbar's buttons and presses the usual Ctrl shortcuts;
5. drags the bottom-right corner small and large, maximises, and checks
   placement and contents each time (fixed-size windows are skipped);
6. measures CPU over 6 s with no input, private bytes and GDI/USER objects;
7. closes with Ctrl+W, then WM_CLOSE, and only then ends the process.

After every action it looks at what changed: new windows (each one
checked, screenshotted and closed again, with system dialogs cancelled by
Esc), a minimised, hidden, resized or always-on-top window, changed
pixels, a menu still open (the item was disabled), a crash (the process
exited; its stderr tail is kept) or a hang (`IsHungAppWindow`). An enabled
item that did none of these is reported as "no visible effect" unless it
legitimately does nothing on an empty document (Copy, Undo…).

At the end it scans the Application event log for errors that name a
`rmac-*`/`lulo-*` exe since the run began.

## Safety

The robot is written for a PC that belongs to someone else.

- It starts only after 2 minutes without input, and stops as soon as input
  arrives that it did not send itself.
- It touches only the Lulo layer, the app windows it started itself and the
  scratch folder: a click lands only where one of those windows is on top,
  and keys go only to one of them in front; anything else is skipped and
  counted (`skipped_input`). A Lulo app the user has open is theirs and is
  never clicked, nor are the user's Desktop icons (the folder double-click
  happens in Files, in the scratch folder).
- Alt+Space may belong to another app (the ChatGPT app on the first test
  PC): the robot uses the fallback Lulo recorded (`SpotlightHotkeyNotice`)
  and closes any other app's window a hotkey opened.
- Its clean-up ends only the processes it started (`launched.txt`), never
  an app the user opened while it waited.
- Menu items, buttons and keys that look destructive or outward (delete,
  Bin, empty, quit, close, send, share, print, erase, shut down, restart,
  sleep, log out, lock, wallpaper, desktop picture, rename, move, uninstall,
  reset, Wi-Fi, Bluetooth…) are listed but never chosen, and the Help menu
  is never chosen (its items open web pages). Control Centre and the status
  menus are opened and closed, nothing inside them is clicked.
- One app instance at a time: an app that is already running is skipped.
- Every window it opens is closed; leftovers it started are ended; Lulo mode
  is left as it was found; the clipboard's text is put back (Copy/Paste are
  skipped when the clipboard holds anything but text).
- Another Lulo test running on the PC (the `LuloCheck` task, or Lulo exes
  from outside the install) stops it before it starts.

## Results

`out\<timestamp>\findings.json` holds `measures` (launch times, memory, idle
CPU, menus, Dock tiles…) and `findings`, each with a severity (`high`,
`medium`, `low`, `info`), an area (`crash`, `hang`, `focus`, `placement`,
`visual`, `overflow`, `no-effect`, `speed`, `idle-cpu`, `memory`, …), the
app, the issue and a screenshot path. Every app also gets `actions.txt` (each
action and its effect), `menus.json` and `stderr.txt`.

Results and screenshots stay on the PC and in local scratch folders: they
show someone's desktop, so they are never committed.
