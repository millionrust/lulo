# GPUI 0.2.2 stable platform spike

> Status: implementation and macOS launch complete; interaction and Ubuntu evidence pending  
> Binary: `rmac-platform-lab`

## Purpose

The platform lab isolates GPUI and desktop integration risk from the product
applications. It is a disposable diagnostic, not a new rmac application.

Run it with:

```sh
cargo run -p rmac-platform-lab
```

## Automated checks

The lab tests ensure its capability IDs remain unique, all core desktop probes
remain represented, and the stable-API blockers are not accidentally marked as
working without evidence.

```sh
cargo test -p rmac-platform-lab
```

## Manual protocol

Run the complete checklist on macOS and on Ubuntu 26.04 under niri and GNOME
Wayland. Record OS, compositor, GPU, scale, and input method.

1. Resize, minimize, maximize, and move the window between displays.
2. Enter English, Hindi, CJK, emoji, compose-key, and dead-key text.
3. Copy the built-in Unicode probe and read it back.
4. Copy text from another application and read it in the lab.
5. Open the file chooser, select a file, then repeat and cancel.
6. Drop one and several files from the system file manager.
7. Scroll slowly and quickly with mouse and touchpad.
8. Repeat at 100%, 125%, 150%, and 200% scaling.
9. Suspend/resume and repeat clipboard and input checks.
10. Leave the lab idle for ten minutes and measure CPU/wakeups.

The lab compares the built-in clipboard probe without retaining clipboard
content, reduces other clipboard reads to a content-hidden status, and retains
only whether a chooser succeeded or how many paths were dropped. File names and
paths are never rendered into screenshots or ordinary diagnostics. Use
disposable probe files anyway; the platform chooser and drag source remain
outside the lab's authority.

## Stable API findings

Source inspection of the published GPUI 0.2.2 crate found normal Wayland/X11
window backends but no exposed programmatic-accessibility tree API and no
layer-shell API. Therefore:

- GPUI 0.2.2 remains useful as the behavior and performance baseline;
- it cannot pass the rmac accessibility or shell-surface gates;
- current upstream GPUI or a released successor must be tested separately;
- product crates remain pinned until the upstream spike passes and migration
  cost is measured.

This is a source/API conclusion, not a claim that Linux runtime behavior works.
Runtime cells stay pending until tested on Ubuntu.

## Evidence table

| Probe | macOS | Ubuntu 26.04 + niri | Ubuntu 26.04 + GNOME |
|---|---|---|---|
| Build and launch | Passed on arm64, 2026-07-10 | Pending | Pending |
| GPU/window lifecycle | Pending | Pending | Pending |
| Text and IME | Pending | Pending | Pending |
| Clipboard | Pending | Pending | Pending |
| File chooser | Pending | Pending | Pending |
| External file drop | Pending | Pending | Pending |
| Scroll and scaling | Pending | Pending | Pending |
| Idle behavior | Pending | Pending | Pending |
| Accessibility tree | API missing | API missing | API missing |
| Layer-shell | Not applicable | API missing | API missing |

Update this table only with a reproducible run and attach logs/screenshots to
the corresponding issue or decision record.
