# Shell menu dismissal

Run `python3 scripts/behavior/run_menu_dismiss.py --bin-dir DIR` with `DIR`
containing symlinks to the built root and shell binaries. The runner creates
its own nested compositor, starts one instance of each shell service with
temporary XDG directories, and sends input only to that compositor.

The first open of the Lulo menu after the top bar starts must close on one
Dock click. A second click on the same menu title must keep it open; clicking
another title must switch menus. The Lulo menu, Wi-Fi, Bluetooth, Sound,
Dock context menu, Spotlight, Apps, Control Center, and Notification Center
must close on an outside click and Escape. The runner also checks clicks on
wallpaper inside and below the top bar's own layer, and on another window.
Inside Control Centre, Esc in Sound's output list must return to the grid and
a second Esc must close Control Centre.
