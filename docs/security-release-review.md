# Security and privacy release review

I6 is a threat-driven release gate, not a claim derived from a clean dependency
scan. `scripts/security-review.json` binds 80 checks across desktop-entry
execution, D-Bus/polkit, portals, file operations, the lock boundary,
notifications, search/indexing, packages, updates, and logs/diagnostics to the
reviewed sources, H8 tier, ten product journeys, and exact candidate revision.

The external authority boundaries are:

- desktop entries follow the freedesktop
  [Desktop Entry Specification](https://specifications.freedesktop.org/desktop-entry/latest/);
  rmac preserves executable/argument boundaries and never converts `Exec` text
  into a shell command;
- the session bus is not treated as a privilege boundary. System-bus callers
  are untrusted, sensitive values never enter broadcast signals, and
  authorization remains with the privileged mechanism and
  [polkit](https://polkit.pages.freedesktop.org/polkit/polkit.8.html);
- interactive polkit authorization is requested only from a direct user action,
  matching the
  [`PolkitAuthority` contract](https://polkit.pages.freedesktop.org/polkit/PolkitAuthority.html);
  applications do not collect administrator credentials;
- portal calls follow the
  [XDG Desktop Portal](https://flatpak.github.io/xdg-desktop-portal/docs/)
  request/response boundary. A file chooser grants access to the user's
  selection, while the
  [Documents portal](https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.Documents.html)
  scopes exported document access; and
- D-Bus authentication, ownership, message flags, and bus policy follow the
  [D-Bus specification](https://dbus.freedesktop.org/doc/dbus-specification.html).
  A well-known name is never accepted as permanent proof of the same owner
  across a service restart.

The current review for 0.9.0-beta.1 (a source review; the station runs are
still pending) is
[security-review-0.9.0-beta.1.md](security-review-0.9.0-beta.1.md).

Validate the committed threat inventory without a Rust build:

```sh
python3 scripts/verify-security-review.py
```

## Review procedure

Use synthetic accounts, documents, networks, notifications, searches, and
update fixtures. Review every named source, its relevant implementation and
tests, packaged artifacts, and the native Ubuntu behavior. For each check,
exercise success, refusal, malformed/excessive input, cancellation, timeout,
concurrent authority change, service loss/recovery, and diagnostics where they
apply. Repeat the security-sensitive product journeys on every H8 station
required by the release tier.

Specifically inspect desktop-entry field-code parsing and launch arguments;
D-Bus peer/owner changes and polkit denial/cancellation; portal handle
correlation and returned URI/document scope; symlink, mount, overwrite,
cancellation and destructive file paths; PAM conversation memory and
fail-closed lock recovery; untrusted notification markup/actions; search roots,
exclusions, bounds and stale cancellation; native/Flatpak contents, licences,
dependencies and uninstall; APT key scoping and trusted PackageKit operations;
and all user-visible, Debug, journal and evidence output for private values.

Generate the pending canonical summary outside the repository:

```sh
python3 scripts/verify-security-review.py \
  --print-template \
  --tier alpha \
  --revision "$(git rev-parse HEAD)" \
  > /absolute/review/path/security-review.json
```

Every discovered issue must have an external tracker entry with severity,
affected boundary, reproduction, fix revision, and regression evidence. The
release summary passes only after every check and required station is reviewed
and `open_findings` is empty:

```sh
python3 scripts/verify-security-review.py \
  --evidence /absolute/review/path/security-review.json \
  --tier alpha \
  --revision "$(git rev-parse HEAD)"
```

Missing, extra, reordered, pending, skipped, stale, source-drift, open-finding,
or incomplete-station evidence fails closed. Keep raw traces, payloads, bus
peers, paths, tokens, credentials, device/session identity and personal content
out of Git. The canonical summary cannot replace source review, adversarial
execution, packaged-artifact inspection, or native denial/recovery evidence.
