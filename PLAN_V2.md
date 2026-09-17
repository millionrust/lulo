# rmac execution plan v2

> Status: proposed execution plan  
> Research date: 2026-07-10  
> Replaces roadmap guidance in `PLAN.md`; preserves `PLAN.md` as project history  
> Reference target: Ubuntu 26.04 LTS, Wayland, x86_64 and aarch64  
> First shell host: niri 26.04 or newer compatible release  
> Planning assumption: one full-time primary developer; estimates are person-weeks, not promises

## 1. Mission

Build a fast, coherent, accessible Linux desktop experience with macOS-like ergonomics,
without cloning Apple assets or depending on private Apple behavior.

The product is delivered in three independently useful layers:

1. **rmac apps** — polished native applications that work on ordinary Wayland desktops.
2. **rmac shell** — a dock, top bar, launcher, notifications, and workspace integration,
   initially hosted by niri.
3. **rmac session** — a curated Ubuntu session and installer/package set. This is only
   attempted after the apps and shell pass their quality gates.

Success is not “every checkbox exists.” Success is that the default user journeys are
fast, dependable, keyboard-accessible, screen-reader-usable, and honest about unsupported
capabilities.

## 2. Strategic decisions

These decisions stay fixed until a written decision record replaces them.

### 2.1 Build on a compositor; do not fork one yet

Use niri as the first integration host. Its JSON IPC event stream supplies initial state
and subsequent window/workspace updates without polling, and its compatibility rules are
documented. Use layer-shell for the top bar, dock, launcher, and overlays.

A compositor fork is permitted only after Phase 7 if all of the following are true:

- a required experience cannot be implemented through standard Wayland protocols,
  portals, layer-shell, or niri IPC;
- the missing behavior affects a top-three user journey;
- an upstream niri proposal has been attempted or formally rejected;
- the team accepts ongoing security, input, display, and driver maintenance;
- a six-month maintenance budget exists.

### 2.2 Keep GPUI, but put it through a Linux gate

The current crates.io GPUI release is pre-1.0 and upstream warns about breaking changes.
Current GPUI upstream explicitly supports Wayland and X11 through `gpui_platform`, and
upstream development includes AccessKit-based accessibility. Do not perform a blind
workspace-wide upgrade.

Phase 1 creates two spikes:

- **stable spike:** current pinned GPUI 0.2.2 on Ubuntu 26.04 Wayland;
- **upstream spike:** a small app on current GPUI main/released successor using
  `gpui_platform`, layer-shell, and accessibility.

Choose the least risky version that passes the Phase 1 gate. Record the exact git revision
or crate version and keep all upgrades in isolated pull requests.

### 2.3 Linux is the product; macOS is a development port

All domain logic and UI remain buildable on macOS where practical. Linux owns product
acceptance. A feature cannot be marked complete because its macOS implementation works.

Platform-specific code must sit behind explicit service boundaries. No Linux application
module may call macOS tools such as `mdfind`, `system_profiler`, `sips`, `open -R`, `pmset`,
or `networksetup`.

### 2.4 Use platform services, not command-output scraping

Linux backends use stable system APIs:

| Capability | Primary Linux contract |
|---|---|
| Applications | XDG desktop entry and icon theme specifications |
| File chooser/open URI/trash | XDG Desktop Portal |
| Global shortcuts | XDG Global Shortcuts portal, with capability detection |
| Appearance/contrast/reduced motion | XDG Settings portal |
| Notifications | XDG Notification portal |
| Network and Wi-Fi | NetworkManager D-Bus API |
| Bluetooth | BlueZ D-Bus API |
| Battery and power | UPower D-Bus API; power-profiles-daemon when present |
| Audio | PipeWire/WirePlumber APIs |
| Workspaces and windows | niri JSON event stream first; standard protocols when sufficient |
| Privileged changes | narrowly scoped D-Bus service with polkit authorization |

Command execution is allowed only as a documented fallback, never as the primary backend,
and must use structured arguments rather than a shell.

### 2.5 Accessibility and performance are release gates

Accessibility is not postponed to beta. Every shared component must expose semantics,
keyboard behavior, and focus behavior before applications may adopt it.

Idle applications must be event-driven. Timer-based unconditional redraw loops are removed.
Animation renders only while animation is active, and reduced-motion is honored.

### 2.6 Global menus are optional research, not a foundation

For rmac-owned apps, define an internal action/menu model that the top bar can consume.
Third-party global menu support is experimental because Linux applications expose menus
through inconsistent toolkit-specific mechanisms. Support GIO exported menu models and
compatible D-Bus menu implementations when detected. Never claim universal coverage.

## 3. Non-goals through 1.0

- Writing a compositor from scratch.
- Pixel-for-pixel copying of Apple applications or use of Apple fonts/assets.
- Replacing every Ubuntu settings backend.
- Universal global menus for GTK, Qt, Electron, XWayland, and games.
- A package manager or app store.
- Cloud sync, accounts, or telemetry before a local-first privacy design exists.
- Rich-text authoring unless GPUI gains an accessible editable rich-text primitive.
- Supporting every Linux distribution before Ubuntu 26.04 LTS is solid.

## 4. Product journeys

The following journeys determine priority. Each needs an automated or scripted acceptance
test plus a manual accessibility check.

1. Log in, launch an app from the dock/launcher, switch apps, and close it.
2. Find a file, preview it, copy/move/rename/trash it, and undo a destructive operation.
3. Open Terminal, run a command, scroll, select, copy/paste, and manage tabs.
4. Create, search, edit, and recover a note after a crash.
5. Open/edit/save a text file through the portal without losing content.
6. Inspect resource usage and safely stop a process with confirmation.
7. Join Wi-Fi, connect Bluetooth, change audio output, and inspect battery state.
8. Complete journeys 1–7 with keyboard-only navigation.
9. Complete core parts of journeys 1–7 with Orca and 200% scaling.

## 5. Target architecture

### 5.1 Workspace layout

Move toward this shape incrementally; do not perform a big-bang rewrite.

```text
crates/
  rmac-core/              errors, IDs, async task policy, app events
  rmac-ui/                tokens and accessible reusable GPUI components
  rmac-editor/            editor domain model and GPUI adapter
  rmac-storage/           atomic persistence, migrations, recovery
  rmac-portals/           typed XDG portal client
  rmac-apps/              installed-app index, icons, MIME and launch
  rmac-system/            system service interfaces and domain types
  rmac-system-linux/      NetworkManager, BlueZ, UPower, PipeWire backends
  rmac-system-macos/      optional macOS development backends
  rmac-compositor/        compositor-neutral window/workspace model
  rmac-compositor-niri/   tolerant niri IPC client
  rmac-test-support/      fake services, temp stores, fixtures, UI harness
  activity-monitor/
  app-drawer/
  finder/
  notes/
  system-settings/
  terminal/
  text-editor/
  dock/
  top-bar/
  launcher/
```

### 5.2 Dependency direction

```text
apps and shell surfaces
        |
        v
domain interfaces + rmac-ui
        |
        v
Linux/macOS adapters + storage + portals
        |
        v
OS, D-Bus, Wayland, filesystem
```

Domain crates never import GPUI, Wayland, D-Bus, or platform FFI. UI code consumes domain
snapshots and commands. Backends emit events instead of being polled by render code.

### 5.3 Required service boundaries

- `AppCatalog`: enumerate, search, categorize, resolve icons, launch, reveal.
- `FileOperations`: copy, move, trash, restore where possible, conflict policy, progress.
- `SystemMonitor`: processes, CPU, memory, disks, networks, termination requests.
- `NetworkService`, `BluetoothService`, `PowerService`, `AudioService`.
- `PortalService`: file chooser, open URI, notifications, shortcuts, settings.
- `CompositorService`: outputs, workspaces, windows, focus, launch/activation actions.
- `SettingsStore`: versioned load/save/migrate with atomic replacement.

Every interface gets an in-memory fake. Application tests use fakes, not live system D-Bus.

### 5.4 State and error policy

- Use XDG data/config/cache/state directories on Linux.
- Persist serde-backed versioned formats; no hand-written JSON parser.
- Write to a temporary sibling, flush, then atomically rename.
- Keep the last known-good config and recover from corrupt files.
- User-visible operations return typed errors with recovery actions.
- Log structured diagnostics with secrets and document contents redacted.
- Never discard a destructive-operation error with `let _ = ...`.

## 6. Quality contract

### 6.1 Required checks on every pull request

```text
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo deny check
```

CI matrix:

- Ubuntu 26.04 x86_64: build, unit, integration, portal/D-Bus contract tests;
- macOS arm64: build and non-Linux unit tests;
- Linux aarch64: cross-build initially, native smoke test before beta;
- MSRV job once the Linux dependency set is pinned;
- scheduled dependency/security audit;
- release build and package smoke test.

No phase may begin with a red main branch.

### 6.2 Test pyramid

| Level | Purpose | Minimum expectation |
|---|---|---|
| Unit | parsers, sorting, selection, persistence, command building | domain behavior and edge cases |
| Component | GPUI actions, focus, semantics, state transitions | every shared interactive component |
| Contract | fake and live D-Bus/portal/niri fixtures | every platform adapter |
| Integration | application journey with temp data and fake services | every product journey |
| Visual | reference screenshots at supported scales | critical screens, reviewed diffs |
| System | real Wayland session and hardware | release candidate matrix |
| Fuzz | desktop entries, config, RTF, terminal input, IPC JSON | continuous bounded runs |

Every bug fix adds a regression test unless the issue is purely visual; visual fixes add a
golden or a documented screenshot comparison.

### 6.3 Accessibility gates

Each interactive element must provide:

- stable accessible identity, role, name, state, value, and available action;
- correct tab order and visible focus ring;
- keyboard operation without pointer-only affordances;
- announcements for asynchronous status and errors;
- 200% text/UI scaling without clipped controls;
- usable high-contrast colors;
- reduced-motion behavior sourced from the Settings portal;
- Orca verification on Ubuntu for each release journey.

Automate semantic-tree assertions where GPUI permits. Run manual Orca, keyboard-only,
contrast, and scaling checks at each milestone gate.

### 6.4 Initial performance budgets

Measure before optimizing. Phase 0 records baselines and may adjust these numbers once,
with a decision record.

| Metric | Initial budget |
|---|---|
| Warm launch to interactive | p95 <= 500 ms for simple apps; <= 900 ms for Finder/Terminal |
| Idle CPU | <= 0.3% per normal app; <= 1% combined shell surfaces on reference hardware |
| Idle wakeups | no periodic UI redraw when state is unchanged |
| Interaction latency | p95 <= 50 ms from input to visible response |
| 60 Hz animation | >= 99% frames within 16.67 ms during standard transitions |
| 120 Hz animation | >= 95% frames within 8.33 ms on capable reference hardware |
| Memory | baseline per app, then a documented budget with leak-free 8-hour soak |

Reference hardware must include Intel or AMD graphics and one supported NVIDIA system.
Test 100%, 125%, 150%, and 200% scaling, multiple monitors, suspend/resume, and hotplug.

## 7. Execution phases

### Phase 0 — Make the repository trustworthy (1–2 weeks)

Deliverables:

- add `README.md`, `CONTRIBUTING.md`, `ARCHITECTURE.md`, and decision-record template;
- pin Rust and all tool versions;
- format the workspace and resolve strict Clippy findings;
- add CI, dependency policy, license inventory, and security audit;
- document commands for every binary;
- capture startup, idle, memory, and frame-time baselines;
- inventory every macOS command/FFI call and every ignored I/O error;
- convert roadmap claims into verifiable issue links.

Exit gate:

- clean checkout passes all required checks;
- each binary builds from documented instructions;
- no unowned “complete” claim remains without an acceptance test or issue;
- baseline report is committed.

### Phase 1 — Prove GPUI on the actual target (2–3 weeks)

Build a small `platform-lab` binary containing text input, menus, dialogs, scrolling, image
loading, clipboard, drag/drop receiving, file portal, accessibility semantics, fractional
scaling, and a layer-shell surface.

Test on Ubuntu 26.04 under niri 26.04, GNOME Wayland, and one nested CI compositor.
Test Intel/AMD first and NVIDIA before exit.

Decisions produced:

- pinned GPUI version/revision and upgrade policy;
- whether GPUI layer-shell is sufficient or needs a narrowly maintained adapter;
- accessibility support status on Linux and the exact missing upstream work;
- supported renderer/GPU matrix;
- supported input methods and clipboard paths.

Exit gate:

- no crash in a four-hour interaction soak;
- input, IME, clipboard, scaling, file portal, and basic Orca output work;
- layer-shell top bar and launcher focus behavior work over fullscreen windows as designed;
- any failed criterion has a bounded workaround and owner, otherwise stop and reconsider GPUI.

### Phase 2 — Establish the platform foundation (3–5 weeks)

Create `rmac-core`, `rmac-storage`, `rmac-portals`, `rmac-apps`, and test support.

Deliverables:

- XDG directory handling and atomic versioned persistence;
- typed portal client with runtime capability discovery;
- desktop entry parser honoring `Hidden`, `NoDisplay`, `OnlyShowIn`, localization,
  `TryExec`, actions, and safe `Exec` field-code expansion;
- icon lookup following the icon theme inheritance algorithm;
- MIME/open/reveal behavior through standards and portals;
- shared cancellation and background-task policy;
- structured logging and user-facing error model;
- fake implementations and fixtures.

Exit gate:

- application catalog matches a reference desktop on a representative fixture set;
- portal tests pass both with a mock bus and a real Ubuntu session;
- corrupt config and interrupted-write tests prove recovery;
- there are no unconditional redraw timers in Apps or simple apps.

### Phase 3 — Make `rmac-ui` a real accessible design system (3–5 weeks)

Deliverables:

- semantic Button, Toggle, Slider, TextField, SearchField, List, Table, Tree, Tabs,
  Dialog, Alert, ContextMenu, Tooltip, Progress, EmptyState, and Toast;
- tokenized type scale, spacing, color, radius, elevation, motion, and focus;
- light/dark/high-contrast themes and reduced-motion variants;
- focus manager and keyboard navigation conventions;
- component gallery with every state and scale;
- GPUI component tests and semantic-tree tests;
- visual goldens at 100%, 150%, and 200%.

Exit gate:

- component gallery passes keyboard-only and Orca review;
- WCAG AA contrast is met for normal text and controls, except documented non-text
  decorative elements;
- applications no longer implement private copies of dialogs, menus, buttons, or tokens.

### Phase 4 — Port three vertical slices to Linux (5–7 weeks)

Port in this order:

1. **Text Editor** — portals, atomic save, crash recovery, find/replace, semantics.
2. **Activity Monitor** — Linux process/system metrics, event-driven refresh, safe signals.
3. **Apps** — XDG catalog, icons, categories, actions, launch activation.

Each application is split into domain, services, state/update, and render modules. Keep main
files below roughly 300 lines; exceptions need a review note.

Exit gate per app:

- all relevant product journeys pass on Ubuntu and macOS development builds still compile;
- no macOS command is reachable on Linux;
- empty/error/loading/offline states exist;
- keyboard, Orca, scaling, visual, and performance checks pass;
- destructive or privileged operations have confirmation and actionable errors.

Phase exit gate: all three ship as installable native development packages. Text Editor also
ships as a development Flatpak. Activity Monitor and Apps receive written sandbox
feasibility decisions rather than misleading Flatpaks that cannot see host processes/apps.

### Phase 5 — Build Linux system services (4–7 weeks)

Create event-driven backends for NetworkManager, BlueZ, UPower, power profiles, and
PipeWire/WirePlumber. Read-only functionality comes first. Mutations are added one at a time
with explicit authorization and rollback behavior.

Deliverables:

- service availability/capability model;
- reconnect after D-Bus service restart;
- Wi-Fi scan/connect/forget and current connection state;
- Bluetooth adapter/device discovery, pair/connect/disconnect;
- battery, AC, charge state, history where available, and power profile;
- audio devices, defaults, mute, and volume;
- minimal polkit design for operations that truly require privilege;
- fake services covering absence, permission denial, timeout, and restart.

Exit gate:

- no settings backend parses human-readable CLI output on Linux;
- service restart does not require application restart;
- permission denial never corrupts displayed or persisted state;
- System Settings labels unavailable capabilities honestly.

### Phase 6 — Complete the application suite (8–12 weeks)

Port and harden in risk order:

1. **Notes** — transactional local store, attachments, indexing, crash recovery, export.
2. **Terminal** — dynamic grid, child lifecycle, resize, IME, selection, hyperlinks,
   shell integration, accessibility strategy.
3. **Finder** — async file operations, progress/cancel, conflict UI, trash, undo,
   mounts, removable devices, MIME actions, search provider.
4. **System Settings** — consume Phase 5 services; keep unsupported pages out of navigation.

Finder safety rules:

- never block the UI thread on recursive I/O;
- never recursively follow symlinks during copy/delete;
- every overwrite has an explicit conflict policy;
- cancel leaves source data intact and destination state explainable;
- trash is preferred to deletion; permanent deletion is clearly separate;
- tests cover cross-filesystem moves, permission errors, low disk, name collisions,
  disappearing mounts, and interrupted operations.

Exit gate: journeys 2–7 pass, including recovery and failure scenarios, on the system matrix.

### Phase 7 — Build the shell on niri (5–8 weeks)

Components:

- **top bar:** focused app, clock, workspace state, system indicators, quick settings;
- **dock:** pinned/running apps, launch/focus, badges only through supported contracts;
- **launcher:** applications, files, settings, calculator, and pluggable providers;
- **notifications:** portal-compatible presentation and history where permitted;
- **session service:** starts components, monitors crashes, and restores them safely.

Integration rules:

- connect directly to `$NIRI_SOCKET`; do not spawn and scrape `niri msg`;
- consume the event stream and tolerate unknown JSON fields/variants;
- use standard workspace protocols when they cover the required behavior;
- use the Global Shortcuts portal and compositor configuration fallback;
- top bar uses the top layer; launcher uses overlay where fullscreen visibility is required;
- reserve exclusive zones deliberately and test monitor hotplug;
- components are separate crash domains.

Exit gate:

- journey 1 works through keyboard and pointer on single and multi-monitor setups;
- shell survives niri restart/logout semantics as designed;
- no state polling when event streams are available;
- combined shell meets idle and frame budgets;
- Orca announces shell UI, workspace changes, and important status.

### Phase 8 — Session integration and packaging (4–6 weeks)

Reference base: Ubuntu 26.04 LTS, whose desktop is Wayland-only.

Deliverables:

- reproducible native packages for session/shell components;
- Flatpak manifests for sandbox-appropriate standalone apps using the current Freedesktop
  runtime, with minimal permissions and portal use;
- desktop files, AppStream metadata, icons, MIME declarations, and D-Bus activation;
- systemd user units with restart limits and log collection;
- niri configuration fragments, portal backend selection, polkit policy, and session entry;
- signed repository/update design and rollback procedure;
- VM image for testing, not yet a public installer.

Exit gate:

- clean VM install, upgrade, rollback, and uninstall tests pass;
- no app requests broad filesystem/session bus access without written justification;
- license and source-offer obligations are satisfied;
- a failed shell component does not destroy the user session.

### Phase 9 — Alpha and beta hardening (6–10 weeks)

Alpha audience: contributors and developers on supported hardware.  
Beta audience: daily-driver volunteers who accept known limitations.

Required programs:

- issue templates that capture GPU, compositor, scale, portal backend, and logs;
- opt-in local diagnostic bundle with preview/redaction; no telemetry by default;
- crash triage and severity SLA;
- eight-hour and seven-day soak tests;
- suspend/resume, display hotplug, low disk, D-Bus restart, shell crash, and update chaos tests;
- translation infrastructure and pseudolocalization;
- security review of launchers, desktop entry expansion, file operations, D-Bus, and polkit;
- accessibility review with users of assistive technology.

Beta exit / 1.0 candidate gate:

- zero open data-loss, privilege-escalation, session-lockout, or critical accessibility bugs;
- no known crash in the top five journeys;
- >= 95% pass rate across the supported hardware/session matrix for two consecutive builds;
- performance budgets pass on reference systems;
- documentation covers install, recovery, safe mode, logs, and uninstall;
- all known limitations are specific, current, and user-visible.

### Phase 10 — Conditional R&D after the beta gate

Evaluate separately; none may delay 1.0:

- third-party global menu adapters;
- richer overview/Mission Control behavior;
- native drag-out support if GPUI/Wayland APIs permit it;
- cloud sync;
- compositor patches or fork;
- Ubuntu flavor/remix application and graphical installer;
- support for additional distributions and compositors.

Every R&D item starts with a two-week spike and a stop/go decision.

## 8. Application-specific 1.0 scope

| App | Required for 1.0 | Explicitly later |
|---|---|---|
| Text Editor | UTF-8 text, open/save, find/replace, recovery, status, printing/export path | rich-text editing |
| Notes | folders, tags, search, attachments, pinning, import/export, recovery | cloud collaboration |
| Terminal | real PTY, dynamic resize, scrollback, selection, search, tabs, profiles | multiplexing/server mode |
| Finder | safe file operations, trash, undo, mounts, search, previews, MIME actions | universal remote filesystem support |
| Activity Monitor | process/resource views, search/sort, safe terminate, histories | unsupported per-process GPU fabrication |
| Apps | standards-compliant discovery/icons/actions/search/launch | store/install management |
| System Settings | only working Network, Bluetooth, Power, Audio, Display info, Appearance | placeholder panes |

“Required” includes loading, empty, unavailable, permission-denied, and error states.

## 9. Risk register

| Risk | Trigger | Mitigation | Stop/reconsider condition |
|---|---|---|---|
| GPUI instability | frequent breaking upgrades or Linux regressions | pin, isolate adapter, upstream minimal fixes | accessibility/input/render gate cannot pass in Phase 1 |
| Accessibility gaps | semantics do not reach Orca | AccessKit/upstream work, semantic components first | core journeys remain screen-reader inaccessible |
| Layer-shell limitations | launcher/dock focus fails around fullscreen | correct layer choice, niri tests, upstream discussion | top journey cannot be made reliable |
| Cross-GPU rendering | NVIDIA/scale crashes or artifacts | hardware CI/manual matrix, renderer fallback if available | supported matrix cannot achieve stable session |
| Unsafe file operations | data loss or partial moves | operation journal, atomic steps, extensive fault tests | any unresolved P0/P1 data-loss bug |
| Privilege boundary | settings require broad root helper | D-Bus + polkit, one narrow method at a time | helper needs arbitrary command/filesystem access |
| Scope explosion | every macOS feature becomes mandatory | fixed journeys and explicit non-goals | milestone lacks a shippable vertical slice |
| Bus/API churn | service restarts or unknown fields crash apps | capability model, tolerant decoding, reconnect tests | adapter cannot degrade safely |
| Packaging sandbox conflict | Finder/Terminal need broad access | native packages for trusted system apps; Flatpak where appropriate | security requires misleading permissions |
| Maintainer overload | shell + apps + compositor exceed capacity | no compositor fork, shared services, staged scope | recurring work exceeds feature delivery for two milestones |

Review the risk register at every phase gate.

## 10. Working method

### 10.1 Issue shape

Every implementation issue contains:

- user outcome;
- in-scope and out-of-scope behavior;
- failure and recovery behavior;
- accessibility behavior;
- performance impact/budget;
- test plan;
- screenshots or interaction recording when visual;
- platform matrix;
- documentation change.

### 10.2 Definition of done

A feature is done only when:

- behavior and failure states meet written acceptance criteria;
- unit/component/integration tests pass;
- keyboard and accessible semantics are verified;
- no unconditional polling or main-thread blocking was introduced;
- errors are visible and actionable;
- Linux works on the reference target and macOS build impact is known;
- formatting, Clippy, tests, audit, and packaging checks pass;
- docs and known limitations are updated;
- a reviewer can reproduce the result from a clean checkout.

### 10.3 Milestone discipline

- One active phase, one stabilization branch, no parallel feature flood.
- Merge small vertical slices; avoid multi-crate rewrites without a migration sequence.
- Keep a decision log for GPUI, compositor, data format, privilege, and packaging choices.
- Demo a real user journey at the end of every two-week iteration.
- Cut scope before moving a gate. Never waive data safety, accessibility, or security gates.

## 11. First 20 pull requests

This is the recommended starting queue.

1. Add CI, pinned Rust toolchain, formatting, and strict Clippy; make main green.
2. Add README with prerequisites and commands for all binaries.
3. Add architecture and decision-record documents.
4. Add baseline benchmark harness and publish current measurements.
5. Add Linux/macOS command and FFI inventory with owners.
6. Create `platform-lab` using current GPUI 0.2.2 on Ubuntu Wayland.
7. Create upstream GPUI/accessibility/layer-shell comparison spike.
8. Record GPUI version decision and upgrade policy.
9. Create `rmac-core` typed error/event/task foundations.
10. Create `rmac-storage` atomic versioned settings with fault tests.
11. Create `rmac-portals` file/open/settings clients with mock-bus tests.
12. Create `rmac-apps` desktop-entry parser and conformance fixtures.
13. Add icon-theme resolver and cache.
14. Remove Apps polling and port it to `rmac-apps`.
15. Build accessible Button/Dialog/Menu/Search components and gallery.
16. Add focus/keyboard/semantic component tests.
17. Port Text Editor open/save/recovery to portals and storage.
18. Split Activity Monitor domain/state/render and add Linux metric tests.
19. Package the three vertical slices natively and Text Editor as a development Flatpak.
20. Run the Phase 4 acceptance review and revise later estimates from evidence.

Do not begin dock animation or global-menu work before PR 20 and the Phase 4 gate.

## 12. Research basis

Primary upstream sources used for this plan:

- [GPUI upstream README](https://github.com/zed-industries/zed/blob/main/crates/gpui/README.md) — pre-1.0 status, current platform split, Wayland/X11 features.
- [GPUI accessibility implementation](https://github.com/zed-industries/zed/blob/main/crates/gpui/src/_accessibility.rs) — current AccessKit-based direction.
- [GPUI 0.2.2 package documentation](https://docs.rs/crate/gpui/0.2.2) — current pinned release baseline.
- [niri IPC documentation](https://github.com/niri-wm/niri/wiki/IPC) — JSON socket, event stream, and compatibility rules.
- [niri layer-shell guidance](https://github.com/niri-wm/niri/wiki/Layer%E2%80%90Shell-Components) — fullscreen, focus, and layer behavior.
- [niri releases](https://github.com/niri-wm/niri/releases) — current release line, Xwayland integration, and accessibility work.
- [COSMIC desktop repository](https://github.com/pop-os/cosmic-epoch) — production Rust desktop component map and Linux dependencies.
- [XDG Desktop Portal API](https://flatpak.github.io/xdg-desktop-portal/docs/api-reference.html) — desktop integration interfaces.
- [XDG Global Shortcuts portal](https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.GlobalShortcuts.html) — user-approved cross-compositor shortcuts.
- [XDG Settings portal](https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.Settings.html) — color scheme, accent, contrast, and reduced motion.
- [XDG File Chooser portal](https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.FileChooser.html) — sandbox-safe file access.
- [XDG Notification portal](https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.Notification.html) — persistent actionable notifications.
- [Desktop Entry Specification 1.5](https://specifications.freedesktop.org/desktop-entry/latest-single/) — application discovery and safe launch rules.
- [Icon Theme Specification](https://specifications.freedesktop.org/icon-theme-spec/latest/index.html) — icon resolution and inheritance.
- [NetworkManager D-Bus API](https://www.networkmanager.dev/docs/api/latest/spec.html) — network state and configuration.
- [BlueZ D-Bus API](https://bluez.readthedocs.io/en/latest/) — Bluetooth adapter and device control.
- [UPower D-Bus API](https://upower.freedesktop.org/docs/ref-dbus.html) — battery and power information.
- [PipeWire API](https://pipewire.pages.freedesktop.org/pipewire/page_api.html) and [WirePlumber API](https://pipewire.pages.freedesktop.org/wireplumber/) — audio graph, device, route, and policy integration.
- [AT-SPI architecture](https://gnome.pages.gitlab.gnome.org/at-spi2-core/devel-docs/architecture.html) — Linux assistive technology contract.
- [Flatpak build documentation](https://docs.flatpak.org/en/latest/building.html) — manifests, runtimes, permissions, and portals.
- [Ubuntu release cycle](https://ubuntu.com/about/release-cycle) and [Ubuntu 26.04 release notes](https://documentation.ubuntu.com/release-notes/26.04/summary-for-lts-users/) — reference LTS and Wayland-only desktop direction.

## 13. Final rule

At every decision, prefer the smallest complete user journey that is safe, accessible,
measurable, and maintainable. A narrower feature that works is better than a broad mockup,
and an honest limitation is better than simulated system behavior.
