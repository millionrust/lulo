# Downloads and Trash places

`rmac-places` is the platform-neutral input for Files sidebars, configured
folder stacks, and the Dock's Trash endpoint. It follows the
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

Permanent purge first converts every platform Trash identity into a stable
path-free digest and requires that complete set to match the count shown by the
Dock. An `EmptyTrashConfirmation` can only consume that exact review after an
affirmative decision. Execution relists the authority, fails closed if a
reviewed identity disappeared, and passes only the reviewed entries to the
purge API; items added after confirmation remain in Trash. After purge, the
adapter enumerates again and returns the authoritative remaining count. Errors
remain typed as empty-versus-inspect failures. macOS remains a build host and
reports Trash enumeration unavailable rather than fabricating Linux-equivalent
state.

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

The default Dock model projects only Trash after a renderer-owned separator.
Files stays in configured application ordering and therefore cannot appear a
second time in the tail. Downloads is not forced into the default profile;
showing it as a folder stack requires a future persisted user choice. Trash
displays a count only when enumeration is authoritative and opens the standard
`trash:///` desktop URI with typed success/failure receipts that do not mutate
the model optimistically. Private paths remain redacted from Debug output.

The Dock domain now exposes Empty Trash only for an available, authoritatively
nonempty Trash and binds the menu count into the exact system review. The
layer-surface renderer must still present that review and its result, and
Linux/niri interaction evidence remains before D5 special-item acceptance.
