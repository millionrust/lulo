# ADR 0011 — Clipboard history for Spotlight

Status: accepted, 2026-09-23

## Context

macOS 26 Spotlight has a Clipboard view (⌘4). On first use it asks "Allow
search results from clipboard"; nothing is recorded before the user allows
it. rmac needs a session service that watches the Wayland clipboard,
keeps recent items privately and hands them to the launcher.

## Decision

- **Watcher: wl-clipboard (`wl-paste --watch`), not a built-in Wayland
  client.** wl-clipboard 2.2 (Ubuntu 26.04) binds ext-data-control-v1 or
  wlr-data-control-unstable-v1, whichever niri offers, handles offer
  lifetimes and pipe transfers, and reports each selection's state in
  `CLIPBOARD_STATE` (`data`, `nil`, `clear`, `sensitive`). The watch command
  only drains the offer and prints that word, so clipboard content never
  passes through a shell or the watch pipe. The service then runs
  `wl-paste --list-types` and reads one whitelisted type with
  `timeout 5 wl-paste --type`. A native data-control client would save two
  short-lived processes per copy but would be a second implementation of
  protocol handling that wl-clipboard already maintains for both
  protocols. `wl-clipboard` is a dependency of the `rmac-session` package.
- **Consent first.** History is off until the user presses Allow in the
  Clipboard view. The choice lives in `$XDG_CONFIG_HOME/rmac/clipboard.json`
  (0600). Turning history off forgets every item.
- **Secrets are never read.** Offers marked `sensitive` by wl-paste, or
  carrying `x-kde-passwordManagerHint: secret`, are skipped; only the hint
  itself is read.
- **Private, bounded, cleared at logout.** Payloads and the index live in
  `$XDG_RUNTIME_DIR/rmac/clipboard` (0700 directory, 0600 files), a tmpfs
  that logind removes when the user logs out. At most 100 items, 64 MiB in
  total, 16 MiB per image, 1 MiB per text or file list, and 8 hours per item.
  Larger copies are skipped, never truncated.
- **Interface.** `org.rmac.Clipboard1` at `/org/rmac/Clipboard1`:
  `Enabled`, `SetEnabled`, `Items`, `Copy`, `Remove`, `Clear` and a
  `Changed` signal. The service runs as `rmac-clipboard.service`
  (`Type=dbus`) under `rmac-session.target`.

## Consequences

Recorded kinds are text, images (PNG/JPEG/WebP/GIF/BMP/TIFF) and local
file lists. Rich text is recorded as its plain-text alternative. The
primary selection (middle-click) is not recorded, as on the Mac.
