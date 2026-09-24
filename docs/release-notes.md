# Release notes

## Unreleased

No user-visible changes recorded yet since 0.9.0-beta.1.

## 0.9.0-beta.1 - 2026-09-24

The first public Beta: a GitHub Release with `.deb` packages and a manual
install guide. Full user-facing detail is in
[0.9.0-beta.1 release notes](release-notes/0.9.0-beta1.md); the
release-engineering detail (what each gate verified) is in
[docs/beta-checklist.md](beta-checklist.md).

- **User-visible changes:** the desktop (menu bar, Dock, Spotlight, Mission
  Control, Control Center, Notification Center, lock screen) and eleven
  first-party apps (Files, Terminal, Notes, Text Editor, System Monitor,
  System Settings, Calculator, Clock, Weather, Media Player, Preview, and
  Archive Utility), all drawn to match macOS 26 and running on real Linux
  services. The project is renamed from rmac to Lulo OS in every place a
  person sees it.
- **Fixed defects:** the first accessibility, code-rules, and CLI-text-
  parsing audits landed real fixes (shared keyboard focus ring and AT-SPI
  roles for toggles/checkboxes/radios/dialogs/menus/toasts; window traffic
  lights made keyboard-reachable; several silently-dropped save/delete
  errors now report instead of failing silently; UFW/Samba/PipeWire status
  now read structured state instead of scraped command output). See
  [docs/accessibility-audit.md](accessibility-audit.md) and
  [docs/code-rules-audit.md](code-rules-audit.md) for the itemized lists.
- **Security/privacy impact:** none negative; a permanent-delete code path
  that could bypass Trash and confirmation was closed
  (`docs/code-rules-audit.md`). No formal security review has been run yet
  (see `docs/beta-checklist.md`).
- **Persisted-data/package compatibility:** first release; there is no prior
  version to migrate from and no in-place update path yet (no signed APT
  repository -- see "What comes after this Beta" in the release notes).
- **Known limitations:** Terminal's and Notes' own content areas have no
  accessible text surface for a screen reader yet; Files exposes no
  accessible file/folder list; the reference-laptop performance budgets,
  the full hardware matrix, and a security review are still outstanding.
  Full list: [Known limitations](known-limitations.md).
- **Restart/sign-out:** installing requires no restart; using Lulo OS
  requires signing out of your current session and choosing the Lulo OS
  session at the login screen. Ubuntu/GNOME stays installed and selectable.
- **Rollback:** `sudo apt purge rmac-session rmac-apps rmac-archive-keyring`,
  or simply choose Ubuntu/GNOME at the next login -- nothing about
  installing Lulo OS changes your default session.

Future entries must state user-visible changes, fixed defects, security/privacy
impact, persisted-data or package compatibility, known limitations, required
restart/sign-out, and exact rollback steps.
