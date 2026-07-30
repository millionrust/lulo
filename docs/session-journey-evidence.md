# GDM and recovery journey evidence

This journey proves real GDM session selection, bounded component crash-loop
recovery, a persistent safe-mode login, same-user TTY repair, a recovered rmac
login, clean logout, and the independent Ubuntu/GNOME session. It is destructive
to one fixed synthetic account. Never run it as a daily user.

## Prepare the disposable account

Install the exact candidate `rmac-apps` and `rmac-session` packages on a clean
Ubuntu 26.04 VM. Keep at least 15 GiB free and confirm the separate Ubuntu
Wayland session is visible in GDM. Then, as an administrator:

```sh
sudo useradd --create-home --shell /bin/bash rmac-journey
printf 'rmac-session-journey-v1\n' | \
  sudo tee /run/rmac-session-journey-v1 >/dev/null
sudo chmod 0644 /run/rmac-session-journey-v1
```

Make the checked-out source tree readable by `rmac-journey`; the runner itself
does not need write access to the repository. The marker grants only this fixed
account permission to corrupt its synthetic shell settings and kill the
allowlisted Dock service. The runner refuses root, SSH/remote sessions, other
users, other Ubuntu versions, reused graphical sessions, unsafe evidence paths,
and low disk.

Set `RMAC_SOURCE` below to the absolute checkout path. Run each command from a
terminal belonging to the session named in that step.

## Five real login phases

1. In GDM choose **rmac**, sign in as `rmac-journey`, open Terminal, and run:

   ```sh
   python3 "$RMAC_SOURCE/scripts/linux/run-session-journey.py" \
     --phase normal-crash-loop
   ```

   This verifies the installed package/GNOME inventory, GDM/logind Wayland
   identity, all normal user units, an add/remove round trip through the XDG
   Notification portal, exact privacy-safe diagnostics, and a valid last-good
   settings candidate. It then corrupts only the synthetic primary settings
   file and repeatedly sends `SIGKILL` to the supervised Dock until the real
   restart budget enters safe mode.

2. Sign out through the desktop, choose **rmac** in GDM again, and run:

   ```sh
   python3 "$RMAC_SOURCE/scripts/linux/run-session-journey.py" \
     --phase safe-login
   ```

   The login must have a new logind session ID, niri rather than rmac portal
   identity, only the safe/security services active, the persistent crash
   trigger, an available last-good restore, and a working portal frontend that
   does not activate the rmac backend.

3. Press `Ctrl`+`Alt`+`F3`, sign in as the same user on the local TTY, and run:

   ```sh
   python3 "$RMAC_SOURCE/scripts/linux/run-session-journey.py" \
     --phase tty-restore
   ```

   The runner proves the safe graphical session still exists, captures the
   redacted diagnostic schema, restores validated settings, preserves the
   rejected private bytes, clears safe mode, and verifies current settings.

4. Return to the graphical VT, sign out, choose **rmac** in GDM, sign in again,
   and run:

   ```sh
   python3 "$RMAC_SOURCE/scripts/linux/run-session-journey.py" \
     --phase recovered-rmac
   ```

   This must be another distinct GDM session with the complete normal target
   healthy, a successful rmac portal round trip, and no safe-mode marker.

5. Sign out, choose the stock **Ubuntu** session in GDM, sign in, and run:

   ```sh
   python3 "$RMAC_SOURCE/scripts/linux/run-session-journey.py" \
     --phase gnome-recovery
   ```

   The final phase proves the GNOME Wayland identity, a working non-rmac portal
   route, and that every rmac target, optional shell unit, supervisor, idle
   locker, and lock coordinator stopped on rmac logout. It deletes the private
   intermediate session-ID state and publishes only:

   ```text
   /home/rmac-journey/rmac-session-evidence/session-journey.json
   ```

The report contains platform identity and pass/fail facts only—no journal
bodies, user paths, session IDs, PIDs, environment, settings, or notification
content. Copy the final report to the candidate evidence bundle, then remove
the synthetic account explicitly:

```sh
sudo userdel --remove rmac-journey
```

Do not treat the build-free `--check-contract` result as runtime evidence. H4
and H6 pass only when all five phases complete on the required clean VMs and
reference PC.
