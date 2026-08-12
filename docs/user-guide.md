# rmac user guide

rmac aims to give Linux the calm, consistent feeling of a Mac: one top bar,
one Dock, Spotlight-style search, focused first-party apps, coherent Settings,
predictable shortcuts, and careful system integration. It remains Linux
underneath. niri owns windows and outputs; Ubuntu services own hardware,
authorization, packages, and login; rmac presents those real capabilities.

## The desktop

- The top bar shows focused-app context and bounded system indicators.
- The Dock combines configured applications, running windows, and Trash
  without inventing state. Optional folder stacks appear only when the user
  has configured them; the default profile does not force Downloads.
- The launcher searches applications, Settings, allowed files/recents, and
  calculator results. Private file search is opt-in and exclusion-aware.
- Notification Center stores bounded local history. Focus controls delivery
  without modifying the sending application.
- Quick Settings exposes common live controls and always reads the system
  authority back after a change.

Shell surfaces remain subject to the Linux framework and hardware gates in
[Known limitations](known-limitations.md).

## First-party applications

| App | What it is for |
| --- | --- |
| Files | Browse, search, preview, copy, move, rename, trash, and manage mounted locations |
| Notes | Keep a private local library with folders, tags, pins, search, and recovery |
| Text Editor | Open and safely save text, Markdown, and supported rich-text documents |
| Terminal | Run the host shell in tabs with selection, search, profiles, and process ownership |
| System Monitor | Inspect resources and request confirmed process actions |
| Apps | Browse and launch valid installed desktop applications |
| System Settings | Configure the rmac desktop and supported Linux services |

## Everyday flow

Open the launcher with `Super`–`Space`, type an app or Settings destination,
and press Enter. Use the Dock to return to running apps. Files and document
apps keep system services and the filesystem authoritative: an external
change, disconnected mount, save conflict, or permission failure is shown
instead of being silently overwritten.

Use Notification Center for history and Focus for temporary quiet periods.
Use Quick Settings for immediate connectivity, sound, and power actions; use
System Settings for complete configuration and recovery information.

## Locking and leaving

Use `Super`–`Control`–`Q` to request the supervised lock. Authentication stays
with PAM and the installed lock provider. Sign out from the session rather than
killing niri or the user manager. If the shell is unhealthy, use safe mode or
the separate Ubuntu/GNOME session described in
[Troubleshooting](troubleshooting.md).

## Your data

Documents stay in the locations you choose. Application preferences, cache,
state, and runtime files follow XDG directories. Removing packages does not
mean consent to delete user data. Read [Privacy](privacy.md) before collecting
diagnostics or testing portal permissions.

## Learn more

- [Install rmac](install.md)
- [Keyboard shortcuts](shortcuts.md)
- [System Settings guide](settings-guide.md)
- [Privacy](privacy.md)
- [Updates, rollback, and removal](update-and-remove.md)
- [Known limitations](known-limitations.md)
