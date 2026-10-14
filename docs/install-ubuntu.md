# Install Lulo on Ubuntu

Lulo OS 0.9.0 Beta 2. It installs next to Ubuntu and adds a **Lulo OS**
choice to the login screen. Ubuntu itself is not changed, so you can always
go back to it.

## What you need

- A 64-bit PC (Intel or AMD processor) with **Ubuntu 26.04** installed.
- An internet connection and your Ubuntu password.
- A spare or test PC is best: this is a beta.

## Install

1. Open **Terminal** (press Ctrl+Alt+T).
2. Copy this line, paste it into Terminal (Ctrl+Shift+V) and press Enter:

   ```sh
   sudo apt install -y curl && curl -fsSL https://github.com/millionrust/lulo/releases/download/v0.9.0-beta.2/install.sh | sh
   ```

3. Type your password when asked. Nothing shows while you type; that is
   normal. The install takes a few minutes.
4. When it says "Log out and choose Lulo OS on the login screen", log out
   (top-right menu ▸ Power Off / Log Out ▸ Log Out).
5. On the login screen, click your name, then the gear button in the
   bottom-right corner, choose **Lulo OS**, and sign in.

The first time, a short Setup Assistant greets you.

## Go back to Ubuntu

Log out, choose **Ubuntu** with the same gear button, and sign in. Lulo
stays installed for next time.

## Update

This beta does not update itself. When a new release comes out, run its
install line (from that release's notes) the same way.

## Uninstall

1. Log out of Lulo OS and sign in to **Ubuntu**.
2. Open Terminal and run:

   ```sh
   curl -fsSL https://github.com/millionrust/lulo/releases/download/v0.9.0-beta.2/uninstall.sh | sh
   ```

This removes Lulo's own packages. Your files and Ubuntu are not touched.
Lulo's window manager packages (`niri` and `xwayland-satellite`) stay
installed; remove them with `sudo apt remove niri xwayland-satellite` if you
want.

## Send feedback

Open an issue at <https://github.com/millionrust/lulo/issues/new> (a free
GitHub account is needed). Please say:

- that you use **Ubuntu**, and your PC model;
- what you did, what you expected, and what happened;
- a screenshot if you can (Lulo saves them in Pictures ▸ Screenshots).

## If something goes wrong

- **The login screen has no Lulo OS choice:** run the install line again and
  read its last lines for an error.
- **Lulo does not start, or keeps failing:** restart the PC and choose
  **Ubuntu** on the login screen. If one part of Lulo keeps quitting, Lulo
  starts once in a safe mode and tells you which part it was; please
  include that name in your report.

More detail is in [Troubleshooting](troubleshooting.md) and
[Known limitations](known-limitations.md).
