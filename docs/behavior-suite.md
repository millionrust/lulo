# Behaviour-parity suite

Finds places where Lulo *behaves* differently from the Mac, without anyone testing by hand. A
scenario is data. The Mac recorder plays it on the owner's Mac and saves what macOS did. The Lulo
runner plays the same scenario inside a private nested compositor and diffs the two.

| Piece | Where | Runs on |
|---|---|---|
| Scenarios | `tests/behavior/<area>/<name>.json` | — |
| Mac expectations | `tests/behavior/<area>/<name>.mac.json` (words and numbers only) | written by the recorder |
| Recorder | `scripts/behavior/record_mac.py` (+ `mac_observe.js`, `mac_click.py`) | the owner's Mac |
| Runner | `scripts/behavior/run_lulo.py` (+ `wlinput.py`) | the laptop, or CI's `behavior-parity` job |
| Comparator | `scripts/behavior/compare.py`, rules in `scripts/behavior/scenario.py` | anywhere |

## Add a scenario

1. Write `tests/behavior/<area>/<name>.json`. `area` is `files`, `text-editor`, `settings`,
   `calculator` or `desktop`.

   ```json
   {
     "title": "New Folder (⇧⌘N) leaves the new folder's whole name selected for editing",
     "app": "files",
     "setup": {"files": {"report.txt": "text", "Projects/": null}},
     "launch": {"folder": "."},
     "steps": [
       {"key": "cmd-2"},
       {"key": "cmd-shift-n"},
       {"observe": "created", "facts": ["focus", "files"]}
     ]
   }
   ```

   - **launch**: `{"folder": "."}` or `{"reveal": "report.txt"}` for Files. The other apps start
     with no arguments. Text Editor starts with one Untitled document.
   - **Steps**:
     - `key`: a chord like `cmd-shift-n` or `⇧⌘N`. ⌘ is Super on Lulo (ADR 0017), ⌥ is Alt,
       ⌃ is Control.
     - `type`: ASCII text.
     - `wait`: seconds.
     - `select` or `context`: click or right-click the item with that name.
     - `focus_desktop`.
     - `observe`: records facts.
     - Any step can take `settle` (seconds to wait after it; the default is 0.8).
     - `menu` (a menu-bar path) works on the Mac only for now: the nested runner has no top bar.
   - **facts**:
     - `focus`: the focused element's role, value, selection, selected_text and selected_all.
     - `windows`: count, front title and titles.
     - `dialog`: whether one is present, its title, texts, buttons in reading order and its default
       button.
     - `menu`: the open menu's items, with ✓ and [disabled].
     - `selection`: the selected item names.
     - `files`: the entries under the sandbox.
     - `tabs`.
     - `display`: Calculator.
   - **tolerance**: `{"<observation>.<fact>.<field>": rule}`. The rules are `exact`, `set`,
     `text` (normalizes quotes, ellipses and spacing), `role-class`, `subset`, `present`, `count`
     and `ignore`. The defaults are in `DEFAULT_RULES`. Pixel sizes are never recorded.
   - **omit**: fields the recorder must not save. Use it for anything that depends on the owner's
     machine, such as a new Finder window's home folder or the Go to Folder path. `menu_until`
     cuts a menu after an item, which drops the owner's own Services.
2. Record it on the Mac. The recorder holds `/tmp/lulo-mac-gui.lock` and uses a
   `/tmp/lulo-behavior/…/sandbox` folder:

   ```sh
   python3 scripts/behavior/record_mac.py files/new-folder --dry-run   # look first
   python3 scripts/behavior/record_mac.py files/new-folder             # writes .mac.json
   ```

3. Run it on Lulo. Do this on the laptop, under the screen lock, with binaries built from the
   branch under test. Build them with
   `cargo build --profile iterate --bins -p rmac-finder -p rmac-text-editor
   -p rmac-system-settings -p rmac-calculator -p rmac-file-chooser`, plus the shell's
   `wallpaper` binary. Don't use `--bin`: it restricts every `-p` to that one binary, which leaves
   the others stale.

   ```sh
   exec 9>/tmp/lulo-journey.lock; flock 9
   python3 scripts/behavior/run_lulo.py --bin-dir $CARGO_TARGET_DIR/iterate \
     --shell-bin-dir $CARGO_TARGET_DIR/iterate --output /tmp/behavior.json files/new-folder 9>&-
   python3 scripts/behavior/compare.py /tmp/behavior.json --emit-parity-rows
   ```

   `--explore --explore-steps N` prints the app's AT-SPI tree after the first N steps. Use it when
   a fact reads `nothing` on Lulo. `--emit-parity-rows` proposes `docs/parity.md` rows, which cite
   `behavior:<area>/<name>`. Copy the real ones into parity.md; the tool never edits it.

## Safety

**Mac**

- Before every key press, the recorder checks two things: the frontmost app is the scenario's
  app, and the focused window is one the scenario opened. Otherwise it stops.
- It never quits an app that was already running. It closes only the windows and documents it
  opened, without saving.
- TextEdit autosaves an edited Untitled document to iCloud. The recorder moves only such copies,
  created during the run, to the Bin.
- The Desktop scenario needs Finder to have no open windows. It removes its folder with
  `rmdir`, which only removes an empty folder.
- Calculator must not be running when the scenario starts. System Settings is reused if it is
  open and left open.
- Nothing destructive outside the sandbox is ever confirmed.

**Lulo**

- `run_lulo.py` re-executes itself under `dbus-run-session` with a temporary HOME,
  XDG_RUNTIME_DIR and every XDG_* directory, and `GSETTINGS_BACKEND=memory`. Each scenario gets
  a fresh HOME.
- The private bus can start only three services: the AT-SPI bus, `xdg-desktop-portal`, and the
  `rmac-file-chooser` from `--bin-dir`, which is what opens Save panels. The installed rmac
  services never start. At the end the runner stops every process still using its runtime
  directory. `text-editor/save-untitled` requires the `rmac-file-chooser` executable in one of
  the supplied `--bin-dir` or `--shell-bin-dir` directories; the runner fails before launching
  the app if it is absent. Running only a fresh `rmac-text-editor` binary leaves the private
  portal without a FileChooser backend and cannot test Save-panel behavior.
- It starts its own headless Sway and holds `wayland-0`/`wayland-1`'s lock files so its socket
  is never named `wayland-1`.
- It injects input only through `wlinput.py`. That script is a pure-Python virtual keyboard and
  pointer, and it refuses `WAYLAND_DISPLAY=wayland-1`, any `/run/user/*` runtime directory, and
  any environment without `RMAC_BEHAVIOR_NESTED=1`.
- One app instance runs at a time. It is stopped by PID and waited for.

## Why headless Sway, not nested niri

Scenarios test app behaviour: focus, selection, dialogs, files. They do not test window
management. Sway's headless backend needs no GPU or seat, offers the wlroots virtual-keyboard and
virtual-pointer protocols this suite injects through, and is what CI's nested smoke test already
uses. niri has no headless backend and no virtual-input protocols. A scenario that needs
niri-specific behaviour should say so in its title and stay a live AT-SPI journey
(`docs/journey-suite.md`).

The one exception is minimising, which only niri can do (the parking workspace, the
`WindowMinimizeRequested` event, the ⌘M bind; ADR 0021). `scripts/behavior/run_niri_minimize.py`
runs niri *nested inside* the headless Sway, with the shipped `shell.kdl` and this branch's Dock
and Mission Control service. Keys go to Sway, because a virtual keyboard on niri itself bypasses
niri's binds, and niri reads Mod as Alt when it is nested. It uses `run_lulo.py`'s isolation, and
it checks a GTK window's own minimise, the Dock tile and restore, ⌘M, and the clean-up when a
parked window closes. Pass `--calculator` to include an rmac app. Checks 1 and 2 need Lulo's
patched niri (`--niri`, 26.04+lulo1-2).

```sh
python3 scripts/behavior/run_niri_minimize.py --niri ~/rmac-niri-build/target/release/niri \
  --bin-dir $CARGO_TARGET_DIR/iterate [--calculator $CARGO_TARGET_DIR/iterate/rmac-calculator]
```

Title-bar movement runs in nested niri as well, because Sway does not exercise GPUI's
`xdg_toplevel.move` requests. The runner uses the shipped `shell.kdl`, drags Calculator and
Settings plus a GTK window with the virtual pointer, and checks niri's reported positions.
Floating edge resizing remains open in WIN-10:

```sh
python3 scripts/behavior/run_window_move.py --niri ~/rmac-niri-build/target/release/niri \
  --bin-dir $CARGO_TARGET_DIR/iterate
```
