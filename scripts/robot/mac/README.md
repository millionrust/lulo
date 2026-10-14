# Mac-vs-Lulo robot

Walks the real macOS 26 apps and shell, walks the same states on Lulo, and
reports every difference. Three commands, each re-runnable:

```sh
# 1. Mac side, on the reference Mac, only when the owner says it is idle.
python3 scripts/robot/mac/mac_walk.py --apps all --shell

# 2. Lulo side, on the laptop, in a private nested session (never the live one).
python3 scripts/robot/mac/lulo_walk.py --bin-dir ~/rmac-macrobot-bins --out ~/rmac-macrobot-out
rsync -a jacob@192.168.18.52:rmac-macrobot-out/ /Users/jake/Projects/rmac/.claude/mac-captures/lulo/

# 3. Diff, on the Mac.
python3 scripts/robot/mac/diff.py --top 30
```

`--bin-dir` is the CI `lulo-binaries` artifact from dev
(`gh run download <runtime run> -n lulo-binaries -D ~/rmac-macrobot-bins`).

## What each side records

A state is `(app, path)`: `window/main`, `menu/<menu>[/<submenu>]` (`@app` is
the application menu), `item/<menu>/…/<item>` (the sheet, panel or window a
menu item opens), `settings[/<tab>]`, `tab/<label>`, `toolbar/<label>`,
`context/<surface>`, `pane/<label>[/<subpage>]` (System Settings) and
`shell/…` (Apple menu, menu-bar extras, Spotlight, ⌘Tab, Dock menus, desktop
menu). Each state stores an accessibility summary (role, label, enabled,
frame relative to the window), the window size and a 1x JPEG.

- `macax.swift` is the Mac helper (AX tree, CG windows, input, image score);
  `mac_walk.py` builds it into `.claude/mac-captures/bin/`.
- `lulo_walk.py` takes the state list from `tests/robot/mac/<app>.json` and
  uses AT-SPI on the private bus, AppMenu2 for menus (synthesizing the
  Application/Window/Help menus as the top bar does) and `grim` for shots.
  Lulo menus draw in the top bar, which the nested run does not start, so
  menu states have no Lulo screenshot. The shell is not walked on Lulo yet.

## Safety (Mac)

It stops at once on HID input it did not make (HIDIdleTime), starts only
after 60 s idle, never confirms anything (Escape, Cancel or the close
button), activates only ellipsis items and known panel openers that pass the
deny list in `common.py`, presses toolbar buttons only when they open a menu
or popover, presses only tab selectors (never radio buttons that are
settings), selects System Settings panes without touching controls, quits
only the apps it launched (without saving), and stops when another process
shows a prompt.

## Storage

Screenshots, full trees and side-by-sides live in
`/Users/jake/Projects/rmac/.claude/mac-captures/` and are never committed
(`scripts/check-no-mac-captures.sh`). Only the scrubbed, image-free
summaries in `tests/robot/mac/<app>.json` are committed: content labels of
personal apps (files, notes, mail, rows) and the owner's names are removed.
`tests/robot/mac/allowlist.json` lists Mac-only states Lulo leaves out on
purpose, each with a reason; `tests/inventory/allowlist.json` labels apply
too.
