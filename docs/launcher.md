# Launcher and Spotlight domain

`rmac-launcher` is the framework-neutral contract for D7/D8. It is shared by
the centered `rmac-launcher-app` overlay and provider adapters; providers supply
typed candidates but cannot decide privacy admission, cross-category order,
selection, or activation fallback.

## Provider privacy

Every provider declares whether it needs private content and/or network access.
The versioned shell-settings policy is checked before a provider receives a
query. Missing policy uses the privacy-first defaults: local public providers
such as applications/settings/calculator run, while file-name/content and
network providers require explicit permission. Disabled providers never enter
the request.

Duplicate provider IDs are admitted once. A returned result must use its
requested provider ID, declared category, a nonempty local ID, and actions
allowed for that category. File-path actions additionally require a provider
that declared private content before admission. A batch that spoofs identity,
category, or action is rejected and exposed as a provider error, so a public
provider cannot bypass privacy policy or distort ranking.

## Query lifecycle and cancellation

Beginning a query cancels the previous `Cancellation` token, increments a
generation, clears old batches, and records the exact admitted providers. File
adapters can pass the token's atomic flag directly to `rmac-search::Options`.
Results from an older generation, an unrequested provider, or a cancelled
request are ignored.

Providers return one bounded batch or a typed error. One failure does not erase
other categories. Pending/error state remains visible for loading and honest
partial-result UI. Escape closes the overlay and cancels every outstanding
provider; there is no background indexing loop in this domain.

## Live overlay runtime

`rmac-launcher-runtime` captures each provider descriptor once and rejects
duplicate or empty identities. For every query it schedules only the exact
admitted provider instances, concurrently, on the blocking pool. Each worker
rechecks admission and cancellation; closing the result receiver cancels the
shared request. New query or privacy-policy generations reject late batches.

Opening returns query-focus intent alongside the empty-query request. The UI
must apply focus synchronously before dispatching that request, so recent-file
work can never delay the first typed character. The coordinator publishes one
overlay snapshot with closed, loading, progressive results, empty,
unavailable, activating, and activation-failed phases. Useful rows remain
visible when another provider fails or is still searching.

Arrow keys wrap stable selection, Return emits the exact primary action,
alternate Return emits only a declared alternate, and Escape closes and
invalidates late activation completion. Only one activation may be in flight.
Live announcements identify the selected title, category, and result position;
provider details and private action payloads never enter the snapshot.

The runtime consumes the typed `launcher` activation already owned by
`rmac-shortcuts`. A fresh timestamp toggles the overlay. Duplicate or older
activations, deactivation signals, and other shortcut IDs do nothing, avoiding
double-open behavior after portal replay or key repeat.

Installed applications remain live without polling. The runtime establishes
the `rmac-apps` directory watcher before discovery, coalesces changes, and
atomically replaces the shared provider catalog. Only a changed, newer catalog
revision restarts an open query; identical scans and stale events do nothing.
Watcher/discovery failure retains the last-known-good application results,
publishes a generic degraded state, and retries after a bounded delay. Detailed
filesystem errors remain diagnostics-only and never enter overlay snapshots.

## Ranking and keyboard behavior

Ranking normalizes whitespace/case and scores exact, prefix, word-prefix,
substring, then subsequence matches. Title matches outrank subtitle matches.
Small category and provider-normalized recency weights break otherwise useful
ties; result ID/title supply deterministic final ordering. Recency is a bounded
0–100 signal, never raw wall time.

A total result limit and per-category cap prevent one broad provider from
crowding out applications, settings, calculator, or files. Selection retains
its stable result ID as later providers complete, falls back to the first
result only when needed, and wraps in both directions.

Primary activation is exact. Alternate activation (for example Reveal for a
file) returns only a declared alternate and never silently falls back to the
primary action. Actions carry parsed shell-free application launch specs,
application-source reveal paths, setting pane IDs, private file paths, or
calculator text; default logs must not print action payloads.

## Local providers

`rmac-launcher-providers` implements four bounded, background-safe adapters:

- Applications retain the exact parsed desktop-entry launch specification; no
  result reconstructs a command from display text. Launch is primary; Show
  Application is a distinct alternate that reveals the desktop-entry or bundle
  source through the file-manager portal.
- Settings match titles, subtitles, and synonyms but return stable pane IDs,
  with duplicate and empty IDs excluded. The built-in catalog covers all 24
  top-level Settings destinations and includes Linux-relevant terms such as
  WLAN, DNS, touchpad, firewall, dark mode, and screen reader without changing
  the stable pane identity.
- Files use recent documents for an empty query and filename search otherwise.
  They accept only an absolute root, return only absolute deduplicated paths,
  pass the query cancellation flag into `rmac-search`, and declare private
  content before the request is admitted. Open is primary and Reveal is the
  explicit alternate action. Versioned shell settings supply bounded absolute
  exclusions and an explicit removable-mount opt-in. Excluded directories are
  pruned before filesystem descent; search stays on the root device by default.
  Results are rechecked for existence, scope, exclusions, and device boundary,
  including canonical checks that prevent recent-document symlink aliases from
  escaping an exclusion. Deleted recent records are silently omitted.
- Calculator evaluates finite arithmetic with precedence, parentheses, unary
  signs, a 256-byte input bound, and no scripting or function surface. Its
  result is a typed copy-text action.

Before an adapter runs, its complete descriptor must exactly match one admitted
to the privacy-filtered request. Cancellation before or after provider work
suppresses the batch; cancellation observed during work becomes an explicit
provider error. Providers never execute their returned action.

## Activation

`rmac-launcher-system` executes the selected typed action once. It validates
nonempty application and pane IDs, absolute file paths, and nonempty copied
text before calling a backend. Application launch remains off the UI executor
and consumes the exact parsed launch specification through `rmac-app-launch`.
In the niri session, direct IPC spawn supplies XDG activation; transport loss
falls back to direct spawn, while rejection/protocol failures remain visible.
Open and Reveal use
different portal-backed operations, preserving the alternate-action contract.
Application reveal uses the same portal authority but has its own validated
action, operation label, and payload-free success outcome.

Clipboard writes and Settings navigation are surface operations: the GPUI
overlay supplies them from its live application context and installed sibling
binary instead of a shell command. Successful receipts expose only the activation ID and
outcome kind. Default error formatting does not contain a file path, copied
text, pane ID, or backend detail; UI code may deliberately inspect typed kind
and detail to produce a suitable private on-screen error.

System Settings now edits the same provider enablement, private-file admission,
removable-mount scope, and directory exclusions consumed by this domain. The
pane reports on-demand/no-background-index behavior and reads the session
shortcut broker's typed status without becoming a second shortcut authority.
For a live GlobalShortcuts v2 session it can request the portal's all-session
configuration UI over a bounded runtime-directory control socket. The broker
uses its existing session handle and returns an exact acknowledgement; Settings
never binds shortcuts or opens a competing portal session. Older portals and
the explicit niri fallback remain visible but do not expose a misleading
Configure action.

## GPUI session surface

`rmac-launcher-app` is the executable consumed by `rmac-launcher.service`. It
creates no idle hidden window: a typed launcher dispatch creates one centered,
non-resizable popup on demand, applies shared appearance/text-scale state, and
focuses the query synchronously before any provider job is scheduled. A second
fresh dispatch, Escape, outside-window deactivation, or successful activation
cancels work and removes the window.

The service uses `Type=notify` and reports readiness only after its action-
scoped Unix socket is bound. The shortcut broker starts after that handshake,
so the first consented activation cannot race an endpoint that merely has a
started process.

An empty query presents installed applications as an icon grid plus other
suggestions. Typed queries use ranked category sections. Application icons come
from the parsed desktop catalog; every other result has an original category
fallback. Arrow keys wrap selection, Return performs the exact primary action,
the semantic secondary-Return modifier and visible ellipsis perform only an
available alternate, and pointer activation first selects the stable result ID.
Loading, partial degradation, empty, unavailable, opening, and private-safe
failure states are visible. The footer mirrors the runtime's bounded live
announcement while detailed provider errors remain out of the UI.

The executable's isolated view-render boundary owns application tiles,
categorized rows, fallback icons, phase/empty/degraded presentation, the search
surface, footer, and keyboard/pointer intent wiring. Query generations,
provider dispatch, selection/activation policy, settings resampling, and
overlay/service lifetime remain outside the renderer.

Application discovery and C4 settings are watched for the lifetime of the
service. A complete settings replacement rebuilds file scope and provider
privacy, cancels the old generation, and reissues an open query once. Invalid
or temporarily unavailable settings retain the last-good registry and disable
no privacy boundary. Settings actions start the installed sibling
`rmac-system-settings --pane <stable-id>` executable; that binary maps every
provider destination to a visible production category. Calculator results use
the live GPUI clipboard, while files and application reveal use the portal.

The installer now builds and installs both the launcher and System Settings
siblings before enabling the session units. `--show` is a deliberate
development-only direct-open path; the normal session accepts only the
allowlisted launcher shortcut endpoint.

Linux/niri placement and focus evidence, the real portal consent/configuration
shortcut journey, Orca runtime evidence, context-menu polish beyond the
explicit alternate action, and performance/idle measurements remain pending.
D7/D8 therefore remain open release gates.
