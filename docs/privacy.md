# Privacy

Lulo OS is local-first and does not include a Lulo OS cloud account, advertising
identifier, analytics service, or telemetry uploader. Linux services remain
the authority for portals, networking, Bluetooth, audio, power, packages,
login, and authorization.

## Documents and search

Files, Notes, and Text Editor operate on local locations selected by the user.
Portal-based open/save flows grant the chosen document scope. Spotlight searches
applications and Settings by default; private files and removable mounts are
included only by the configured policy, with explicit exclusions. Search
results and diagnostics must not broaden that authority.

## Permissions

Privacy & Security can display bounded camera and microphone decisions stored
by the XDG PermissionStore. Reset removes that stored decision so a future
portal request may ask again. It does not prove active capture stopped, revoke
an unsandboxed application's access, or replace the Linux permission model.

## Notifications and Focus

Notification history and Focus configuration are private local state.
Notification senders remain responsible for their content. Lock-screen
presentation, history size, action targets, and diagnostics are bounded;
private notification text must not enter release evidence.

## Credentials and authorization

Lulo OS does not collect administrator passwords. Privileged changes are
authorized by the system polkit agent. Network and VPN secrets remain with
NetworkManager and its secret/authentication agents. Lock authentication
remains with PAM and the installed provider; password text and raw keycodes
must never be logged.

## Diagnostics

Status reports intentionally exclude usernames, hostnames, machine IDs,
serials, addresses, private paths, document text, clipboard contents,
credentials, tokens, D-Bus peer identities, and raw helper errors. Journals and
raw traces may still contain sensitive data from the operating system or an
application. Inspect and redact them locally before sharing.

Use a separate synthetic test account for evidence. Keep raw evidence in the
ignored `target/linux-evidence` tree and commit only canonical reviewed
summaries. See [Troubleshooting](troubleshooting.md) for safe collection and
[the security release review](security-release-review.md) for the full threat
boundary.
