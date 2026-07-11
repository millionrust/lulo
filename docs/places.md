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

The live places watcher and Dock projection remain the next slice. Until they
land, this adapter does not claim D5 special-item completion.
