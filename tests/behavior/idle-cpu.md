# Idle CPU behavior scenario

Run on the reference laptop with optimized release binaries and no concurrent
build. Keep one application instance at a time, give each a temporary HOME and
XDG config/data/state/cache tree, and never inject input into the live session.
The application is open and untouched for three seconds before a 60-second
`/proc` CPU and context-switch sample. Use `scripts/linux/measure-budgets.py`
for the five apps and `scripts/linux/sample-shell-idle.py` for the complete
shell over one simultaneous 60-second window. Save the raw JSON.

| Surface | Idle behavior to verify |
|---|---|
| Text Editor | The focused insertion point parks visibly after two seconds without editing; typing after four seconds still reaches the document (`text-editor/find.json` exercises this in the nested suite). Deactivating the window cancels its blink task. |
| Clock | World Clock has minute-resolution hands and labels and schedules only the next minute change. Static tabs wait on the saved-state watcher; an inactive window has no redraw timer. A running stopwatch or countdown uses its displayed precision. |
| System Monitor | Process and graph samples occur at the five-second interval only while its window is active. |
| Files | Folder and mount changes arrive through inotify and kernel mount notifications; there is no directory scan timer. |
| Settings | The Search field starts keyboard-focused, then the cursor parks without keeping the whole window busy. Measure with that initial focus intact. |
| Weather | The empty first-run window leaves Search unfocused and has no refresh or minute timer. Saved cities refresh automatically no more often than hourly, and their displayed local times change at minute boundaries. |
| Shell | Sample top bar, Dock, wallpaper, notification center, and all other running shell units together. |

Acceptance is at most 0.3% of one CPU core per normal app, 2.5% for System
Monitor, and 0.5% for the shell combined. Context switches are a wake-up
proxy; inspect timers or syscalls when they indicate unexplained activity.

Focused Search regression for Settings, using the private nested compositor and
the normal-app 0.3% limit (BUG-01):

```sh
python3 scripts/behavior/monkey.py --bin-dir ~/lulo-monkey-bins \
  --niri /usr/bin/niri --app settings --idle-only \
  --idle-seconds 60 --max-idle-cpu 0.3
```

The shared input component also powers Files Search and Text Editor Find.
Focus those fields through their normal shortcut, then sample the same idle
window in the private compositor:

```sh
printf '%s\n' '[{"index":0,"kind":"shortcut","params":{"chord":"⌘F"},"note":"focus search"}]' \
  > /tmp/lulo-focused-search-replay.json
for app in files text-editor; do
  python3 scripts/behavior/monkey.py --bin-dir ~/lulo-monkey-bins \
    --niri /usr/bin/niri --app "$app" \
    --replay /tmp/lulo-focused-search-replay.json --check-idle-cpu \
    --idle-seconds 60 --max-idle-cpu 0.3
done
```
