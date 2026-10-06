# Tall menu-bar menus

Run `python3 scripts/behavior/run_menu_scroll.py --bin-dir DIR` with `DIR`
holding this branch's `top-bar` (`wallpaper` and `rmac-files` fall back to
`/usr/libexec/rmac` and `/usr/bin`). Each case starts its own private nested
niri on a headless Sway, with temporary XDG directories, and sends input
only to that Sway.

- **fits**: a 1920x1080 output at scale 1.25 (the reference laptop, logical
  1536x864). Files' File menu opens whole: every row is inside the panel,
  the panel stays 5 pt above the screen bottom, nothing is drawn under the
  panel (the old material backdrop showed a light slab there while the rows
  were cut off at logical y 540), and in light appearance the material is
  bright.
- **scrolls**: a 1280x720 output at scale 1.25 (logical 1024x576). The File
  menu is taller than the room below the bar, so the panel stops 5 pt above
  the bottom with rows hidden below it. Up with nothing highlighted
  highlights the last row and scrolls it into view; the mouse wheel scrolls
  back; resting the pointer on the top scroll arrow scrolls to the top.

`--shots DIR` keeps screenshots of each step locally (never commit them).
`--dark-too` adds the fitting case in dark appearance for comparison.
