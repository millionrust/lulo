# Downloads and Trash places

`rmac-places` is the platform-neutral input for Finder sidebars and the Dock's
Downloads/Trash special items. It follows the
[XDG Base Directory specification](https://specifications.freedesktop.org/basedir/latest/)
and the
[freedesktop Trash specification](https://specifications.freedesktop.org/trash/latest/)
instead of assuming one fixed home folder or one trash directory.

## Downloads resolution

The domain accepts an absolute HOME and optional `user-dirs.dirs` contents. It
recognizes only a double-quoted `XDG_DOWNLOAD_DIR` using:

- `$HOME` or `${HOME}`, optionally followed by `/...`; or
- an absolute literal path.

It does not invoke a shell. Relative paths, other variables, nested expansion,
backticks, NUL, and unsupported escapes are rejected. Missing configuration
falls back to `HOME/Downloads`. An explicit HOME value is preserved while
marking the special directory unconfigured, matching the user-dir convention
for a disabled directory.

Filesystem existence is adapter-owned so parsing remains deterministic and
testable. A missing configured directory must appear unavailable; the Dock
must not create it merely by rendering.

## Trash state

The domain models whether the platform authority is available, whether Trash
is empty, and the authoritative item count. It does not infer state by reading
only `$XDG_DATA_HOME/Trash`: the freedesktop contract permits trash bins on
other mounted filesystems as well.

## System adapter

`rmac-places-system` reads HOME and the absolute XDG configuration root, then
loads `user-dirs.dirs`, checks the resolved place paths, and returns a snapshot
plus typed warnings. An unreadable or malformed user-dir file falls back to
`HOME/Downloads` without hiding Home or Trash. A failed Trash enumeration marks
only Trash unavailable.

On Linux, Trash enumeration and purge use the repository's existing `trash`
dependency, which implements the freedesktop home and mounted-filesystem trash
contract. The adapter does not scan only one directory. Downloads opens through
the shared desktop-portal boundary and fails clearly if its directory is absent.

Permanent purge requires an `EmptyTrashConfirmation` that can only be created
from an affirmative confirmation result. After purge, the adapter enumerates
again and returns the authoritative empty/count state; errors remain typed as
empty-versus-inspect failures. macOS remains a build host and reports Trash
enumeration unavailable rather than fabricating Linux-equivalent state.

## Live Dock projection

`rmac-dock-runtime` now treats places as an independent last-known-good
authority alongside niri, shell settings, the application catalog, and the
display Main-output authority. It does not publish the first coherent Dock
snapshot until places have either loaded or failed explicitly. Later failures
retain the last accepted place model and update health without blanking the
Dock.

The live adapter watches the XDG user-directory configuration root, Home,
the resolved Downloads parent, every currently discoverable freedesktop Trash
`files` and `info` directory, the user data Trash root, and the Linux mount
table. Every notification is only a hint: the worker performs a complete
off-thread resample, recreates the watch set, and publishes only a changed
model. A 60-second reconciliation catches mount backends that do not emit a
usable filesystem notification; identical results never request a Dock frame.

The Dock model projects a fixed Files, Downloads, Trash order after a
renderer-owned separator. These entries never enter application pin ordering.
Missing or explicitly disabled Downloads remains visible but unavailable;
Trash displays a count only when enumeration is authoritative. Private paths
are retained only in activation values and redacted from their default debug
form. Files and Downloads open through the desktop portal, and Trash opens the
standard `trash:///` desktop URI, with typed success/failure receipts that do
not mutate the model optimistically.

The layer-surface renderer, reviewed Empty Trash menu flow, and Linux/niri
interaction evidence remain before D5 special-item acceptance.
