# Live audit of the running rmac session — 2026-09-19

Connected to the reference PC (`jacob@192.168.18.52`, Ubuntu 26.04 LTS, kernel 7.0.0-31, niri 26.04),
captured the live desktop (`target/evidence/live-2026-09-19.png`) and measured it against the macOS 27
reference. This is the first audit of the **running** product rather than the source.

## What is genuinely good

- The session starts and stays up: wallpaper, top bar, Dock, notification centre, focus service, idle
  lock policy and the supervisor are all `active (running)` under systemd.
- **The menu bar is already transparent.** Sampling the strip at y = 5…20 returns wallpaper pixels
  (`#1F295D`, `#324096`, `#345CA9`) — there is no opaque panel. Fix 1 from the playbook is done.
- Fonts resolve correctly: `fc-match Inter` → Inter-Regular, `fc-match "JetBrains Mono"` →
  JetBrainsMono-Regular.
- niri 26.04 is installed, so `ext-background-effect` blur is available.
- Layout is structurally right: bar at the top, centred Dock shelf with a semantic separator.

## What is broken, in order of damage

### 1. The shell runs **debug builds** — this alone ruins the feel
```
/home/jacob/rmac/shell/target/debug/top-bar     610 MB
/home/jacob/rmac/shell/target/debug/dock        596 MB
/home/jacob/rmac/shell/target/debug/wallpaper   568 MB
```
Release binaries **already exist** (41 MB, 44 MB, 39 MB) but the systemd units carry a dev override:
```
ExecStart=
ExecStart=/home/jacob/rmac/shell/target/debug/top-bar
```
Unoptimised GPUI means slow frames, late input and janky animation. Every judgement about "it doesn't
feel like a Mac" made on this build is judging a debug binary.

### 2. The top bar burns a quarter of a CPU core, permanently
```
rmac-top-bar.service: Consumed 30min 28s CPU over 2h 1min wall clock  (≈25 % of one core)
```
Idle sampling agrees: **8.4 % CPU with nothing happening**. The budget is ≤ 0.1 %. Something redraws
continuously — a per-frame or per-second loop rather than an event-driven clock. This is why the
machine will never feel calm, and it is a battery and fan problem on a laptop.

### 3. The lock screen does not work at all
```
rmac-lock.service: failed (restart counter 5, "Start request repeated too quickly")
rmac-lock-coordinator.service: stuck in "activating"
```
There is no working lock. `FEEL_SPEC.md` §E steps 32–33 cannot pass.

### 4. Zombie processes are accumulating
Three defunct `rmac-shortcut-dispatch` processes, plus the log line
`top-bar[616601]: Could not Dispatch: Connection refused (os error 111)`. The shortcut broker socket
is refusing connections and the children are never reaped.

### 5. The Dock geometry is wrong in every dimension
Measured from `live-2026-09-19.png` against the macOS reference:

| Metric | rmac live | macOS 27 | Delta |
|---|---|---|---|
| Icon size | 67 px | 64 px | +3 |
| **Pitch (centre to centre)** | **95 px** | **76 px** | **+19** |
| **Gap between icons** | **28 px** | **12 px** | **+16** |
| **Shelf height** | **92 px** | **72 px** | **+20** |
| Bottom margin | 22 px | 18 px | +4 |
| Trash tile | **missing** — 97 px of empty shelf after the separator | present | — |
| Running indicators | **none visible** | 4 px dots | — |

The Dock is too airy and too tall, and it has a hole where the Bin belongs. This is most of why the
shelf reads as a Linux dock.

### 6. Assets are not deployed
- `/usr/share/rmac/sounds/` does not exist → **no sound anywhere**.
- `~/rmac/assets/icons/*.svg` count is **0** on the box → the generated squircle icons never arrived;
  the Dock still shows flat glyphs.
- Cursor theme is **Adwaita**; `/usr/share/icons/` has no `rmac` entry.

### 7. The reference PC is three commits behind
Box HEAD: `963e457`. Mac HEAD: `1368df7` (measured colours, wallpaper tinting). It also has a dirty
tree (Finder presentation files modified). Conclusions drawn from this machine are one build old.

### 8. Still missing from the menu bar
Only the app name renders — no File/Edit/View/Go/Window/Help. The clock shows `Sat 19 Sep 6:05 PM`
where the owner's Mac shows `Fri 18 Sep 11:30 am` (lowercase meridiem, tabular numerals).

## Fix order

1. **Point the units at release binaries** and rebuild them. One command, biggest instant change.
   ```sh
   systemctl --user edit --full rmac-top-bar.service   # and rmac-dock, rmac-wallpaper
   # ExecStart=/home/jacob/rmac/shell/target/release/top-bar
   cd ~/rmac/shell && cargo build --release --features wayland
   systemctl --user daemon-reload && systemctl --user restart rmac-top-bar rmac-dock rmac-wallpaper
   ```
2. **Find the top bar's redraw loop.** Instrument `RMAC_TOP_BAR_RENDER_COUNT_DIR` for 60 idle seconds:
   the counter must not move. Likely causes: a clock timer that ticks every second instead of aligning
   to the next minute, a status subscription that re-renders on every poll, or an animation that never
   reaches rest.
3. **Fix `rmac-lock.service`** — read the actual exit code with
   `journalctl --user -u rmac-lock.service -b --no-pager`, then repair the failing path before
   anything else in Phase 6.
4. **Fix the shortcut dispatch socket** and reap children, so the zombies stop.
5. **Dock geometry:** gap 28 → 12, shelf padding so the height lands at 72, bottom margin 18, add the
   Bin tile, add running dots.
6. **Deploy the assets:** `git pull` on the box, install `assets/sounds/*.wav` to
   `/usr/share/rmac/sounds/`, wire `rmac-sound`, install the cursor theme to `/usr/share/icons/rmac`
   and set it in the niri config, and use `assets/icons/*.svg` in the Dock.
7. **Menu bar:** real app menus, and the clock format `Fri 18 Sep  11:30 am` with tabular numerals.

## How to re-run this audit

```sh
ssh -i ~/.ssh/rmac-reference-pc jacob@192.168.18.52 \
  'export XDG_RUNTIME_DIR=/run/user/1000; WAYLAND_DISPLAY=wayland-1 grim /tmp/shot.png'
scp -i ~/.ssh/rmac-reference-pc jacob@192.168.18.52:/tmp/shot.png target/evidence/live-$(date +%F).png
python3 scripts/measure-reference.py edges-col target/evidence/live-$(date +%F).png 700 --from 930 --to 1080
systemctl --user --failed          # on the box
systemd-cgtop -1 -n1               # on the box: who is burning CPU
```
