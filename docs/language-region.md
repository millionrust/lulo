# Language & Region authority

System Settings presents separate Language, Region, and Input Sources controls
while keeping one Linux authority. `rmac-locale-linux` reads and mutates
`org.freedesktop.locale1`; `rmac-locale` owns bounded decoding, semantic locale
state, validation, rollback identity, and XKB validation without depending on
D-Bus or GPUI.

The adapter follows the current upstream
[`org.freedesktop.locale1`](https://www.freedesktop.org/software/systemd/man/latest/org.freedesktop.locale1.html)
contract and its
[`SetLocale` implementation](https://github.com/systemd/systemd/blob/main/src/locale/localed.c).
Localed merges supplied assignments with current state, may derive `LANGUAGE`
from its fallback table, and removes redundant non-`LANG` assignments whose
value equals `LANG`. rmac therefore compares canonical semantic assignment
sets instead of assuming request bytes are the persisted result.

## Snapshot and previews

A complete snapshot reads `Locale`, `X11Layout`, `X11Model`, `X11Variant`,
`X11Options`, and the console keymap. Assignment names are restricted to the
locale categories accepted by localed; values are bounded and reject control
characters. Installed locales come from the standard argument-separated
`locale -a` inventory, then are syntax-validated, sorted, deduplicated, and
bounded to 4,096 values.

Date/time, number, and currency examples use independent POSIX locale objects
and fixed sample values. They never change process-global locale state. Native
preview failures are separate from the authoritative assignment snapshot.

Changing Language updates `LANG` while retaining the effective value of every
existing `LC_*` override. If a retained override becomes identical to the new
language, canonical state omits that redundant assignment exactly as localed
does; its effective behavior is unchanged.

Changing Region updates only regional format categories: numeric, time,
monetary, paper, name, address, telephone, and measurement. It does not change
`LANG`, `LC_MESSAGES`, character classification, or collation. The input locale
must be installed. A mixed pre-existing format configuration remains visible
as mixed until the user intentionally applies one Region.

## Transactions and rollback

Every locale mutation starts from a fresh snapshot, validates only new values
against the installed inventory, constructs the complete desired semantic
state, and sends explicit empty assignments for categories that must be
removed. Interactive authorization is requested through localed. rmac accepts
success only when a complete post-mutation snapshot canonically matches the
desired state.

When localed derives an unwanted `LANGUAGE` fallback, rmac rereads the exact
intermediate state before removing only that assignment in a second localed
request. Any intervening locale change produces a conflict instead of a broad
cleanup. Raw D-Bus errors are used only to classify authorization; UI errors
are bounded, control-character-free, and contain no bus or peer details.

After a successful change, the one-step rollback stores both the previous
canonical assignment set and the exact applied readback. Revert first requires
the current complete assignment set to equal that applied state. If another
tool changed locale state, rmac refuses to overwrite it and requests a refresh.

## Input sources

One to four comma-separated XKB layouts are accepted only from the bounded
installed `localectl list-x11-keymap-layouts` inventory. Variant slots must not
outnumber layouts, and multiple layouts require an explicit `grp:` switching
option. Model, layout, variant, and options receive exact post-mutation
readback. Rollback retains both previous and applied keyboard tuples and refuses
to run after an external change.

rmac passes `convert=false` to `SetX11Keyboard`: this pane owns the graphical
default and deliberately does not rewrite the console keymap as a side effect.
The console keymap remains truthful read-only state. The editor is enabled only
when `rmac-input` proves that niri follows localed. A direct or traversed
included niri XKB configuration remains the active owner and keeps localed
read-only, consistent with the
[`niri` integration contract](https://github.com/YaLTeR/niri/wiki/Integrating-niri).

## Live state and session behavior

Filtered localed property signals and well-known-name owner changes trigger
coalesced complete snapshots. Each refresh or mutation advances a generation;
an older stream read cannot replace newer mutation readback. One pending refresh
is retained through a busy transaction. Service loss has a separate live-update
error while the last known-good snapshot remains visible, and daemon
reappearance guarantees resampling.

Localed updates the system manager environment for subsequently started
services, but already-running processes do not adopt it. The pane therefore
requires sign-out and sign-in before judging the whole desktop language or
regional format.

## Linux acceptance matrix

F13 remains unchecked until the Ubuntu/niri reference PC proves:

- empty, normal, mixed, aliased, and bounded/truncated locale inventories;
- Language and Region success, no-op, authorization denial/cancellation,
  localed mismatch, derived `LANGUAGE`, exact rollback, and concurrent-edit
  refusal;
- deterministic date, number, and currency previews across representative
  UTF-8 locales, including missing/broken native locale data;
- one and multiple XKB layouts, aligned empty/nonempty variants, switching
  options, invalid/uninstalled input, exact readback, rollback conflict, and
  niri direct/included/localed ownership;
- external `localectl` changes, localed stop/restart, suspend/resume, and
  last-known-good recovery without stale stream overwrite;
- sign-out/sign-in adoption by new apps and services, with the existing session
  explicitly unchanged before sign-out;
- keyboard-only editing, focus behavior, 100–200% scaling, contrast modes,
  Orca names/state/errors, and bounded idle CPU/wakeups.

Commit only privacy-safe results. Locale values and XKB names may be recorded;
never include hostnames, users, private paths, raw D-Bus diagnostics, or polkit
conversation details.
