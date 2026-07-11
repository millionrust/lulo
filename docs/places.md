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

The Linux adapter and destructive confirmation flow remain the next slice.
Until they land, the Dock must not show a functioning Empty Trash action or
claim D5 special-item completion.
