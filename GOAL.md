# rmac completion goal

This file is the durable product and execution contract for finishing rmac. It
turns the project vision into one objective, a bounded sequence of checkpoints,
proof requirements, and a verifiable stopping condition suitable for Codex
`/goal` runs.

## Start or resume the goal

Run this from the rmac project:

```text
/goal Complete rmac according to GOAL.md. Read GOAL.md completely, inspect the current dev branch and its linked project documentation, then work autonomously through the highest-priority unfinished checkpoint. Keep implementing, validating on the Ubuntu reference PC, committing, and pushing coherent batches without stopping until every Definition of Done gate in GOAL.md has verifiable evidence, or until a genuine blocker requires a product decision or authority only I can provide.
```

Use `/goal` to check status, `/goal pause` before deliberately interrupting the
run, `/goal resume` to continue the same objective, and `/goal clear` only when
this objective is complete or intentionally replaced.

## The objective

Deliver rmac as a sponsorship-ready, install-once Ubuntu 26.04 desktop product:
a user installs rmac, selects the rmac session at login, and receives a fast,
coherent, dependable desktop that recreates the current macOS experience in
layout, interaction, motion, shortcuts, daily workflows, and attention to
detail while retaining an original rmac identity and using truthful Linux
system services.

The stopping condition is not "the code exists" or "the preview looks close."
The goal is complete only when a clean supported Ubuntu machine can install a
versioned candidate, log into the complete rmac session, pass the defined daily
journeys and quality gates on real hardware, recover or uninstall safely, and
produce a credible public demonstration and sponsor package.

## Why I want this

I love using my MacBook Air M2. I want the same sense of calm, polish, speed,
consistency, and confidence when I use Linux. I should not have to launch shell
pieces one by one or excuse controls that only look real. From power-on and
login onward, the desktop should feel like one product: the wallpaper, menu
bar, windows, Dock, search, notifications, settings, system controls, built-in
apps, shortcuts, gestures, animations, sound, lock screen, updates, and
recovery should agree with each other.

I also want rmac to be a serious open-source project that can earn sponsorship.
That means the demo must be visually compelling, but the engineering claims
must also be honest, reproducible, safe, and documented.

## Product principles

1. **One complete experience.** Installing rmac provides the session and its
   integrated surfaces; it is not a collection of unrelated apps or preview
   commands.
2. **Behavior before decoration.** A visible control has a real action and real
   state. Unsupported behavior is hidden or explicitly unavailable, never
   faked.
3. **Current macOS is the interaction reference.** Before implementing a
   surface, inspect the latest stable macOS behavior available on the reference
   Mac, current Apple documentation, compatible open-source references, and the
   relevant Linux authority. Record the reference date and version because the
   target changes over time.
4. **Original rmac identity.** Match the hierarchy, proportions, interaction,
   motion, and quality of macOS without shipping Apple logos, SF fonts,
   wallpapers, sounds, application artwork, or other proprietary assets.
5. **Linux is the product.** macOS is useful for development and comparison,
   but no feature is complete until it works on the supported Ubuntu/niri
   session.
6. **Fast and native.** Core shell surfaces and first-party apps stay native,
   GPU-rendered, event-driven, responsive, and quiet while idle.
7. **Safe by default.** GNOME remains available as a recovery session. Risky
   display, package, session, and destructive file changes have confirmation,
   rollback, or recovery paths.
8. **Accessible by construction.** Keyboard navigation, focus, semantic
   accessibility, contrast, scaling, reduced motion, and reduced transparency
   are part of the component, not a final patch.
9. **Evidence beats percentages.** Never invent a completion percentage.
   Progress is the set of passed gates and demonstrated journeys at an exact
   commit.
10. **Finish the product, not the backlog.** Prioritize the smallest set of
    work that makes the complete daily experience credible. Defer attractive
    extras that do not unblock the installable session or sponsor demo.

## Source of truth

Read these before choosing work, and update them when their contract changes:

- `GOAL.md` — objective, priorities, workflow, and stopping condition.
- `README.md` — public product status and commands.
- `PLAN_V2.md` — architecture, phases, journeys, and release gates.
- `PARITY.md` — application capability and framework gaps.
- `docs/product-reference-matrix.md` — research routing per product surface.
- `docs/macos-ui-reference.md` — shell research and asset policy.
- `docs/known-limitations.md` — truthful open blockers.
- `docs/linux-reference-bringup.md` — real-hardware validation procedure.
- `docs/install.md`, `docs/update-and-remove.md`, and
  `docs/session-recovery.md` — delivery and recovery contracts.
- `docs/release-notes.md`, `docs/alpha-contributor-build.md`,
  `docs/beta-cohort.md`, and `docs/one-dot-zero-candidate.md` — promotion
  boundaries.

Git history, tests, runtime logs, screenshots, and evidence bundles decide what
is actually complete when prose conflicts with reality. Repair stale prose as
part of the same batch.

## Supported baseline

- Primary target: Ubuntu Desktop 26.04 LTS on Wayland.
- First compositor/session host: niri 26.04 or a documented compatible release.
- Mandatory recovery path: untouched Ubuntu/GNOME Wayland remains selectable.
- First real reference machine: x86_64 HP workstation-class laptop with Intel
  HD Graphics 5500 using the hardware Vulkan driver.
- Required later matrix: current Intel and AMD graphics, then one supported
  NVIDIA configuration; x86_64 first and aarch64 before 1.0.
- Required scales: 100%, 125%, 150%, and 200% where the hardware supports them.
- Core apps may remain buildable on macOS, but Linux runtime evidence is the
  acceptance authority.

Do not commit personal SSH addresses, private keys, tokens, machine IDs, serial
numbers, user data, or unreviewed logs. Local automation may use the configured
reference-PC SSH connection through environment variables such as
`RMAC_REFERENCE_HOST` and `RMAC_REFERENCE_KEY`.

## Current baseline at the creation of this goal

The repository already contains substantial domain logic, first-party apps,
Linux service adapters, session supervision, packaging contracts, recovery
documentation, and targeted validation tooling. The upstream GPUI Linux lab
can present the wallpaper, menu bar, and Dock on the Ubuntu/niri reference PC.
The Dock has stable groups and indicators, and the menu bar has been aligned
toward the real macOS hierarchy.

This is meaningful progress, but it is not yet a release. The preview shell is
not yet the single packaged daily session; stable and upstream GPUI paths still
need a deliberate product decision; native accessibility evidence is open;
several shell/runtime integrations need real-hardware completion; and signed
install, update, rollback, uninstall, performance, soak, security, and full
hardware evidence remain release blockers. Re-audit this paragraph against the
current commit instead of treating it as permanent status.

## Execution strategy

### Work in complete vertical checkpoints

Do not bounce between dozens of unrelated crates. Select one user journey or
closely connected surface and finish a coherent batch of roughly three to five
behaviors across its model, backend, UI, recovery, and evidence. A good batch
can be reviewed in one screenshot or interaction recording and one focused
test/build report.

The loop for every checkpoint is:

1. Inspect the current branch, dirty files, relevant contracts, and existing
   implementation. Preserve all unrelated user work.
2. Research the whole target surface before changing it: current Apple
   behavior, one compatible open-source implementation, and the Linux service
   or protocol that owns the real state.
3. Write or refine the acceptance checklist for the batch. State what the user
   can do afterward and which command or artifact will prove it.
4. Implement the connected behavior end to end. Prefer shared primitives and
   event-driven services over per-app copies.
5. Run proportional source checks locally. Do not spend time on workspace-wide
   validation for a narrow change.
6. Push the coherent batch to `dev`, pull it on the Ubuntu reference PC, reuse
   existing build artifacts, and build or run only the affected packages or
   binaries.
7. Inspect Linux logs directly and collect one focused screenshot or journey
   recording when the change is visual or interactive. Compare against the
   current Mac reference at the same logical scale.
8. Fix failures in the same checkpoint. Add a regression test for behavior
   bugs or documented visual evidence for purely visual corrections.
9. Commit with the existing `Jacob Samas <samasjacob@icloud.com>` identity,
   without a coauthor line, push `dev`, update the progress log, and immediately
   continue to the next highest-impact unblocked checkpoint.

Ask the owner only for a product choice that would materially change scope, a
permission or credential not already authorized, or subjective visual judgment
that cannot be established from the supplied Mac references. A normal compiler
error, runtime failure, or difficult implementation is not a reason to stop.

### Optimize for throughput without lowering the bar

- Research and design a surface once, then implement it in a batch.
- Reuse dependencies and the normal Cargo target directories.
- Use source inspection and small checks on the space-constrained Mac.
- Run heavyweight Linux builds sequentially on the reference PC.
- Do not ask the user to copy compiler output when SSH access can retrieve it.
- Do not push a new build for every one-line guess. Resolve locally visible
  type and API issues first, then perform one remote compile/fix loop.
- Prefer fixing blockers on the critical path over adding more settings rows,
  speculative abstractions, or decorative polish.
- Keep worktree state explicit before and after every commit. Stage only files
  belonging to the active checkpoint.
- Do not run repetitive or unnecessary tests. Validation must be proportional
  to regression risk and must prove the changed path.

### Storage and machine safety

- Never allow either development machine to fall below 15 GiB free.
- Before any build likely to consume more than 1 GiB, require at least 25 GiB
  free on the relevant data volume.
- Stop, report, and clean only safe generated artifacts if the floor is near.
- Never create Docker Cargo caches, duplicate repositories, alternate Cargo
  targets, or parallel Cargo pipelines for routine validation.
- Never remove GNOME, rewrite firmware boot entries, erase user data, or make a
  destructive system change as part of ordinary development.

## Priority roadmap

Each checkpoint ends with an exact commit, focused validation, and an updated
status. The order may change only when evidence shows a dependency or blocker.

### Checkpoint 0 — Freeze the foundation decision

1. Complete the stable-versus-current GPUI Linux gate on the reference PC.
2. Prove the required layer-shell, focus, clipboard, input/IME, scaling, and
   accessibility path or document a bounded implementation plan for each gap.
3. Select and pin the product framework revision through a decision record.
4. Remove the split between a convincing experiment and the actual product by
   promoting the chosen shell primitives into maintained crates.

Exit proof: the product packages use the selected framework path, a fresh
targeted build succeeds on Ubuntu, and the decision record contains the exact
revision, known gaps, upgrade policy, and rollback.

### Checkpoint 1 — Make rmac one login session

1. Start wallpaper, menu bar, Dock, launcher, notifications, Quick Settings,
   lock coordination, shortcuts, and required portals from one supervised rmac
   session entry.
2. Ensure every surface has exactly one owner and no duplicate third-party bar,
   Dock, notification daemon, or shortcut handler appears.
3. Add bounded startup readiness, crash restart, safe mode, and last-known-good
   configuration behavior.
4. Preserve a selectable, working GNOME recovery session.

Exit proof: after a normal reboot and GDM login, the complete rmac desktop
appears without terminal commands; a shell component crash recovers; repeated
crashes enter safe mode; GNOME still logs in.

### Checkpoint 2 — Finish the macOS-feel shell

#### Desktop and windows

- Adaptive original wallpaper with per-output fit, light/dark behavior, and
  reduced-transparency fallback.
- Borderless, resizable windows with coherent original traffic lights,
  shadows, focus, maximize/fullscreen, tiling, minimize, restore, and move
  behavior.
- Predictable workspaces, touchpad navigation, overview/Mission Control-style
  window discovery, multi-monitor placement, hotplug, and suspend/resume.
- No compositor debug borders, stray panels, stale surfaces, or focus stealing.

#### Menu bar

- Leading active-app identity and real exported rmac app menus.
- No fake global menus for third-party apps that do not export commands.
- Trailing privacy state, status menus, Spotlight, Control Center, and date/time
  using original icons and authoritative live data.
- Correct per-output/fullscreen policy, keyboard access, popover anchoring,
  click-away/Escape behavior, and compact current-macOS proportions.

#### Dock

- Stable kept-app group, unpinned-running/recent group, minimized/place group,
  and Trash with semantic separators.
- Real application artwork where licensing allows, original first-party
  artwork, running indicators, urgent badges, stable order, launch, focus,
  minimize/restore, quit/context actions, drag reorder, folder stacks, and
  Trash behavior.
- Size, placement, auto-hide, recent apps, running indicators, launch animation,
  minimize policy, and optional magnification backed by Settings.
- The default reference profile follows the owner’s current Mac configuration;
  hover tooltips and optional stable-center magnification behave like macOS.

#### System overlays

- Spotlight, App Drawer, Control Center, Notification Center, banners, Focus,
  privacy indicators, volume/brightness/media OSDs, lock screen, logout,
  restart, suspend, and shutdown form one visual and interaction system.
- Global shortcuts work immediately after login and never depend on a terminal.

Exit proof: the complete shell passes the login/launch/switch/search/settings/
notification/lock/logout journey at 100% and 200%, keyboard-only, after a shell
restart, and in a recorded side-by-side review with the reference Mac.

### Checkpoint 3 — Finish daily first-party applications

Treat each application as a complete daily journey, not a static imitation.

1. **Files:** real mounts and portals; sidebar; tabs; icon/list/column/gallery
   views; preview and Quick Look; search; tags; Get Info; file operations with
   progress, conflicts, undo, Trash, removable media, drag/drop, and recovery.
2. **Terminal:** correct PTY lifecycle; dynamic resize; tabs/windows; profiles;
   search; selection; clipboard; links; Unicode/IME; process titles; scrollback;
   safe close; accessibility; and reliable shell integration.
3. **Notes:** local-first transactional storage; folders; tags; pinned notes;
   search; attachments; tables/lists where honestly supported; autosave;
   versioning/recovery; app Trash; import/export/print; and no implied cloud
   collaboration.
4. **Text Editor:** portal open/save; plain-text daily use; encodings and line
   endings; find/replace; wrapping; spelling; recovery; print; recent documents;
   and truthful bounded rich-text behavior.
5. **System Monitor:** truthful process/CPU/memory/energy/disk/network metrics;
   sorting/search; process inspection; graphs; adjustable refresh; safe quit and
   force quit; and no invented GPU or energy figures.
6. **System Settings:** real appearance, Desktop & Dock, displays, keyboard,
   pointer/trackpad, accessibility, network/VPN, Bluetooth, sound, battery,
   Focus, users, privacy, language/time, storage, updates, About, lock, and
   session controls where Linux has an authority. Risky changes offer Keep and
   Revert. Unsupported Apple-only services do not masquerade as working rows.
7. **App Drawer:** complete installed-app discovery, search, categories/folders,
   keyboard navigation, live refresh, launch, context actions, stable layout,
   and Dock interaction.

Add small built-in utilities only when the shell and journeys above are solid:
Calculator, Clock, Calendar, image/PDF preview, media playback, and software
management must use real implementations or established Linux apps presented
coherently. Do not delay the release to clone every bundled Apple application.

Exit proof per app: its primary journey passes on Ubuntu with real services,
keyboard-only at 100% and 200%, safe failure/recovery, reviewed visual evidence,
and bounded idle/startup measurements.

### Checkpoint 4 — Complete Linux integration

- NetworkManager Wi-Fi/Ethernet/VPN and connectivity changes.
- BlueZ discovery, pairing, connect/disconnect, trust, and error recovery.
- PipeWire/WirePlumber devices, mute, volume, routing, and live change events.
- UPower battery, charging, health where available, and power profiles.
- Displays, brightness, scaling, arrangement, mirroring where supported, and
  reversible risky changes.
- Keyboard layouts, IME, shortcuts, pointer, touchpad, and gestures.
- XDG portals for files, URI launch, notifications, settings, global shortcuts,
  screenshots/capture, wallpaper, and sandbox interactions.
- Clipboard, drag/drop, MIME/default apps, removable media, secrets, polkit,
  privacy indicators, and screen capture authority.
- Niri outputs, windows, workspaces, activation, focus, fullscreen, and hotplug.
- Third-party GTK, Qt, Electron, XWayland, game, and sandbox compatibility with
  honest limits.

AirDrop, Handoff, iCloud, Apple accounts, AppleCare, Time Machine, Find My, and
other proprietary services are not release requirements. A future compatible
Linux alternative must be separately named, secured, tested, and described
truthfully rather than pretending to be the Apple service.

Exit proof: changing a real service outside rmac updates the UI event-driven;
service restarts degrade only their own controls; secrets and private content
do not enter logs; and the seven core journeys survive suspend/resume and
hotplug.

### Checkpoint 5 — Accessibility, performance, and resilience

- Complete accessible roles, names, states, actions, focus order, live regions,
  and screen-reader operation for shared components and core journeys.
- Pass Orca checks, keyboard-only use, high contrast, reduced motion, reduced
  transparency, and 200% scale without clipped critical controls.
- Meet or deliberately revise the documented startup, latency, animation, idle
  CPU, wakeup, and memory budgets with repeatable evidence.
- Complete crash injection, service restart, corrupt configuration, low disk,
  suspend/resume, display hotplug, long-session leak, and four/eight-hour soak
  checks as required by release stage.
- Complete threat modeling, dependency/license review, bounded-input review,
  privilege/polkit review, secret redaction, and private vulnerability process.

Exit proof: accessibility and performance reports name the exact candidate and
hardware; no critical journey has an unresolved crash, data-loss, lock-screen,
privilege, or inaccessible-operation defect.

### Checkpoint 6 — Install, update, recover, and remove

- Produce reproducible native amd64 and aarch64 packages with recorded source
  and asset provenance.
- Complete a signed APT repository and safe trust-anchor installation.
- Prove clean install, first login, same-version reinstall, upgrade, interrupted
  upgrade recovery, rollback, uninstall while retaining optional user data,
  and complete purge on clean machines.
- Keep the GNOME recovery session and documented TTY recovery available.
- Never require users to clone the repository, compile Rust, edit niri config,
  or start individual services for ordinary installation and use.

Exit proof: a tester follows the public install guide on a fresh supported
machine, reaches the complete session, upgrades to the next signed candidate,
rolls back or removes it, and retains a bootable GNOME desktop throughout.

### Checkpoint 7 — Sponsorship-ready public release

- Clear README with a short value proposition, honest status, architecture,
  hardware support, install path, screenshots, demo video, roadmap, license,
  contribution guide, security policy, and sponsor links.
- A polished two-to-five-minute demo: boot/login, shell, app launch/switch,
  Spotlight, Files, Settings/Control Center, notifications, lock/recovery, and
  the original rmac identity on real Ubuntu hardware.
- Before/after and side-by-side visual material that demonstrates inspiration
  without implying Apple affiliation or distributing Apple assets.
- Versioned release notes, known limitations, reproducible verification
  summary, acknowledgements, asset/dependency provenance, and beginner-friendly
  installation.
- A bounded post-release issue list split into defects, verified compatibility
  expansion, and optional stretch features.

Exit proof: the release candidate passes every gate below, the public materials
match the actual build, and a new user can understand, install, demonstrate,
recover, and remove rmac without developer assistance.

## Definition of Done

All of these gates must be checked against one versioned candidate and exact
commits. An unchecked gate means the durable goal continues.

- [ ] **D1 — Framework:** the maintained product uses the selected/pinned Linux
  GPUI path; required layer-shell, input, focus, scale, and accessibility gaps
  have working solutions or explicitly accepted release-safe boundaries.
- [ ] **D2 — Install:** a fresh supported Ubuntu machine installs rmac from the
  documented versioned package/repository flow without a source checkout.
- [ ] **D3 — First login:** selecting rmac in GDM starts one complete supervised
  desktop with no manual terminal step or duplicate shell component.
- [ ] **D4 — Daily journeys:** launch/switch apps; find/manage files; use
  Terminal; create/recover a note; edit/save a text file; inspect/stop a
  process; and manage Wi-Fi/Bluetooth/audio/battery all pass with real data.
- [ ] **D5 — Shell parity:** menu bar, Dock, wallpaper, windows, search,
  workspaces, Control Center, notifications, Focus, OSDs, lock, and session
  actions pass documented behavioral and visual review against the current Mac
  reference using original rmac assets.
- [ ] **D6 — Settings:** every visible control is authoritative, persistent,
  accessible, failure-aware, and reversible where risky; unsupported controls
  are hidden or explicit.
- [ ] **D7 — Accessibility:** core journeys pass keyboard-only, Orca, high
  contrast, reduced motion/transparency, and 200% scale checks.
- [ ] **D8 — Performance:** measured launch, input latency, animation, idle CPU,
  wakeups, and memory meet the accepted budgets on named hardware with no
  unresolved long-session leak.
- [ ] **D9 — Resilience:** component crashes, service restarts, corrupt config,
  low disk, suspend/resume, output hotplug, and interrupted updates recover
  without data loss or an unusable machine.
- [ ] **D10 — Security and provenance:** privilege boundaries, dependencies,
  source/assets, secrets, diagnostics, and release signing pass review; no Apple
  proprietary asset is distributed.
- [ ] **D11 — Lifecycle:** signed update, rollback, uninstall, optional data
  retention, complete purge, GNOME recovery login, and TTY recovery are proven.
- [ ] **D12 — Hardware:** required Intel/AMD/NVIDIA, x86_64/aarch64, scaling, and
  multi-monitor results meet the declared support matrix or the public support
  statement is narrowed honestly before release.
- [ ] **D13 — Public release:** README, install guide, user guide, screenshots,
  demo video, release notes, known limits, security policy, contribution path,
  sponsor material, and downloadable candidate all describe the same build.

## Validation policy

Use the least expensive proof that is strong enough for the change, then run
the milestone gate once per coherent batch or candidate.

### Local Mac

- Inspect source, format only touched Rust files when possible, run focused
  non-Linux unit checks, validate Markdown/scripts, and review diffs.
- Do not build the current upstream GPUI dependency graph on the Mac while free
  space is constrained.
- Do not run workspace-wide `--all-features` or `--all-targets` unless that
  exact release gate is explicitly required and storage is safe.

### Ubuntu reference PC

- Confirm `df -h /` and stop before the 15 GiB floor.
- Pull `dev` with `git pull --ff-only`; never overwrite uncommitted remote work.
- Reuse the existing target directory and build affected packages/binaries
  sequentially with `--locked`.
- Run `scripts/linux/run-reference-gates.sh` only at the appropriate milestone;
  use its preflight and package-scoped modes for routine work.
- Inspect `systemctl --user` state and `journalctl --user` logs for the affected
  components directly, reviewing private content before committing evidence.
- Capture screenshots/recordings at controlled scale and note the exact commit,
  session, output scale, hardware, and observed pass/fail.

### Minimum batch proof

Every committed implementation batch records:

1. Exact commit and files/components changed.
2. Focused local check result.
3. Focused Ubuntu build/run result when Linux-facing.
4. Screenshot or interaction result when user-visible.
5. Known limitations or follow-up that remain.

## Git and change-control contract

- Work on `dev` unless the owner directs otherwise.
- The owner authorizes Codex to commit and push coherent in-scope batches.
- Use the existing `Jacob Samas <samasjacob@icloud.com>` identity and never add
  a coauthor line.
- Inspect `git status` before editing, staging, committing, pulling, or pushing.
- Existing dirty and untracked files belong to the owner unless explicitly
  brought into the active checkpoint. Never stage unrelated changes.
- Make focused commits with outcome-based messages. Do not mix research,
  mechanical churn, generated artifacts, and unrelated product work.
- Do not force-push, reset hard, discard user changes, rewrite published
  history, or use destructive cleanup.
- Keep generated builds, evidence, logs, captures, caches, private machine data,
  and credentials ignored. Commit only reviewed durable documentation and
  intentional product assets.
- After pushing, verify the remote branch contains the expected commit and the
  reference PC can fast-forward cleanly before using it as evidence.

## Never do these to look finished

- Do not draw fake status, fake settings, fake devices, fake search results, or
  controls whose only effect is local demo state.
- Do not copy Apple source, logos, fonts, icons, wallpapers, sounds, or private
  APIs into the product.
- Do not call a macOS implementation complete before Linux runtime proof.
- Do not call a compile, static screenshot, generated evidence directory, or
  passing mock-service test a successful real-machine journey.
- Do not remove recovery paths, bypass authentication, weaken the lock screen,
  or make privileged operations broad for convenience.
- Do not add cloud accounts, telemetry, sponsorship analytics, or remote data
  collection without a separate privacy and security decision.
- Do not optimize a tiny decorative mismatch while a login, activation,
  accessibility, recovery, installer, or data-loss blocker is open.
- Do not keep reporting vague progress or an invented percentage. Name the
  exact checkpoint, proof, remaining blocker, and next action.

## Progress log format

Keep goal updates short and decision-useful:

```text
Checkpoint: <name and current outcome>
Completed: <user-visible behavior and exact commit>
Verified: <focused commands, Ubuntu journey, screenshot/evidence>
Remaining: <next highest-impact gap>
Blocked: <none, or the precise authority/decision required>
Storage: <Mac and reference-PC free GiB when a heavy build is relevant>
Next: <the batch already being started>
```

Update durable status documents at milestone boundaries rather than appending a
large diary to this file. Continue autonomously while safe in-scope work remains.
