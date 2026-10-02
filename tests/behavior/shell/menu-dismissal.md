# Shell menu dismissal

Run `python3 scripts/behavior/run_menu_dismiss.py --bin-dir ~/rmac-wt/target/iterate`
inside the nested compositor harness. The runner starts one instance of each
shell service with temporary XDG directories and sends input only to that
compositor.

The first open of the Lulo menu after the top bar starts must close on one
Dock click. A second click on the same menu title must keep it open; clicking
another title must switch menus. The Lulo menu, Wi-Fi, Bluetooth, Sound,
Dock context menu, Spotlight, Apps, Control Center, and Notification Center
must close on an outside click and Escape. The runner also checks clicks on
wallpaper inside and below the top bar's own layer, and on another window.
