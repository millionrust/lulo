# Install Lulo on Windows (preview)

Lulo 0.9.0 Beta 3 for Windows is an **early preview**. It runs on top of
Windows: the Lulo apps are normal Windows programs, and the optional Lulo
layer puts Lulo's menu bar, Dock and Spotlight over the Windows desktop.
Windows itself is not replaced. Expect rough edges.

## What you need

- A 64-bit PC with **Windows 11**. (Windows 10 may work but has not been
  tested.)
- No administrator password: Lulo installs for your user only.

## Install

1. Download **`Lulo-Setup-0.9.0-beta.3-x64.msi`** from the
   [release page](https://github.com/millionrust/lulo/releases/tag/v0.9.0-beta.3).
2. Open the downloaded file.
3. **Windows warns you, because the installer is not signed.** (Code signing
   costs money; Lulo is a free project.) On "Windows protected your PC",
   click **More info**, then **Run anyway**. If your browser warns too,
   choose **Keep**.
4. The install finishes on its own in a few seconds.

Lulo's apps are now in the Start menu, in a **Lulo** folder (All apps ▸
Lulo): Calculator, Notes, Text Editor, Preview, Clock, Weather, Terminal,
Files, System Settings, System Monitor, Mail, Calendar, Player and Archive
Utility.

## Try the Lulo layer

Open **Lulo** from that folder. Lulo's menu bar appears at the top, its
Dock at the bottom, and the Windows taskbar hides while it runs.

- **Alt+Space** opens Spotlight.
- Shortcuts use **Ctrl** where a Mac uses ⌘ (Ctrl+N, Ctrl+S, Ctrl+Q).
- To get the normal Windows desktop back: Lulo menu (top-left) ▸
  **Turn Off Lulo**.

The Lulo layer never starts by itself unless you turn on **Start Lulo at
Sign-In** in the Lulo menu.

## Update

Download the next release's MSI and open it. It replaces this version and
keeps your settings.

## Uninstall

1. If the Lulo layer is running, choose Lulo menu ▸ **Turn Off Lulo**.
2. Open **Settings ▸ Apps ▸ Installed apps**, find **Lulo**, click **…** and
   choose **Uninstall**.

This removes every Lulo app, shortcut and scheduled Clock alarm. Your own
files are not touched.

## If something goes wrong

If the taskbar does not come back after Lulo stops, open Start (press the
Windows key), type `cmd`, press Enter, and run:

```bat
"%LOCALAPPDATA%\Programs\Lulo\lulo-session.exe" --restore-windows-desktop
```

## Send feedback

Open an issue at <https://github.com/millionrust/lulo/issues/new> (a free
GitHub account is needed). Please say:

- that you use **Windows**, your Windows version and PC model;
- what you did, what you expected, and what happened;
- a screenshot if you can (Windows key+Shift+S).

Known gaps are listed in [Known limitations](known-limitations.md#windows-preview).
