# Test strategy: fakes and contracts for platform services

> Updated: 2026-09-24
>
> Scope: in-memory fakes for the service interfaces named in `todo.md`
> "Quality › Tests", and the shared contract tests that check a fake and a
> live backend against the same assertions.

## The pattern

Each service crate in this matrix that has a fake follows the same shape,
mirrored across `rmac-network`, `rmac-bluetooth`, `rmac-power`,
`rmac-audio`, `rmac-storage`, and `rmac-quick-settings-system`:

- `src/fake.rs` — an in-memory struct (`Fake*Service` / `FakeBackend` /
  `InMemoryBackend`) implementing the same trait as the real backend,
  holding its state in a `Mutex` (or `RefCell` for the single-threaded
  `rmac-quick-settings-system::execute` contract), with builder methods
  (`with_*`) to seed it and no I/O of any kind.
- `src/contract.rs` — one or two generic functions, each taking
  `&impl Trait`, asserting the invariants any implementation must satisfy.
  Where mutating a live backend would be disruptive on a shared machine
  (Wi-Fi, Bluetooth, audio, power), the contract is split in two:
  - a **read-only half** (`assert_*_is_observable`), safe to run against
    the real `System*Service` — wired up as an `#[ignore]`d test in
    `tests.rs`, meant to be run deliberately on the reference laptop
    (`cargo test -p <crate> -- --ignored`);
  - a **full mutation half** (`assert_*_contract`), which additionally
    toggles/connects/switches state, and only ever runs against the fake.
  For `rmac-storage`, mutating the backend never touches anything
  durable (it's a scratch tempdir or purely in-memory), so
  `assert_backend_contract` runs unconditionally against both
  `InMemoryBackend` and the real `FileSystem` — no `#[ignore]` needed.
- Both modules live behind `#[cfg(any(test, feature = "test-support"))]`,
  with a new `test-support` Cargo feature declared in the crate's
  `[features]` table. A crate that wants another crate's fake in its own
  `#[cfg(test)]` code adds it as a `[dev-dependencies]` entry with
  `features = ["test-support"]` (see `rmac-shell-settings/Cargo.toml`).

This keeps every fake reachable from *outside* its crate — the gap the
inventory below found repeated across the codebase: crates already had a
private `Fake*` struct defined inline inside their own `#[cfg(test)] mod
tests`, invisible to any other crate's tests.

## Matrix

| Interface | Trait | Fake | Contract (fake) | Contract (live/host) | Notes |
|---|---|---|---|---|---|
| Wi-Fi (`rmac-network`) | `WifiService` (pre-existing) | `fake::FakeWifiService` | `contract::assert_wifi_service_contract` | `#[ignore]` `system_wifi_service_snapshot_is_well_formed`, read-only | Mutating live Wi-Fi on the shared reference laptop would disconnect other agents' SSH sessions, so the live contract is snapshot-only. |
| Bluetooth (`rmac-bluetooth`) | `BluetoothService` (pre-existing) | `fake::FakeBluetoothService` | `contract::assert_bluetooth_service_contract` | `#[ignore]`, read-only | Same reasoning as Wi-Fi: pairing/connect against real hardware never runs unattended. |
| Power (`rmac-power`) | `fake::PowerService` (new; scoped to `snapshot`/`set_profile`, the surface `rmac-quick-settings-system::Backend` already uses) | `fake::FakePowerService` | `contract::assert_power_service_contract` | `#[ignore]`, read-only | Charge-threshold control (`set_charge_threshold`) is not in the trait yet — its `ChargeThresholdIdentity` model ties a threshold to one specific hardware battery instance, which needs more design than an additive pass should attempt. `SystemPowerService` is a pure delegate to the existing `snapshot`/`set_profile` free functions; nothing changed behaviorally. |
| Audio (`rmac-audio`) | `fake::AudioService` (new; scoped to `snapshot`/`set_volume`/`set_muted`/`set_default_device`) | `fake::FakeAudioService` | `contract::assert_audio_service_contract` | `#[ignore]`, read-only | Profile/route/balance control stay out of the trait for the same reason as power. The crate already had strong fixture coverage independent of this trait: `tests.rs` parses a real, sanitized `pw-dump` capture (`fixtures/pw-dump-laptop.json`) and asserts the exact defaults/volumes/profile it decodes to — that is itself a fake-vs-live-shaped contract test at the parsing layer, just not through a trait. |
| Compositor domain (`rmac-compositor`) | n/a — a compositor-agnostic pure state/event crate (`WorkspaceId`, `WindowId`, `OutputId`, `Event`, `State`) | n/a — the crate itself *is* the fake surface | `State`/`actions` unit tests in `tests.rs` | n/a | No live backend to run against; this crate never touches niri, D-Bus, or Wayland by design (see its module doc comment). App/view-model tests can build a `State` and feed it `Event`s directly without depending on `rmac-compositor-niri` at all. |
| niri IPC adapter (`rmac-compositor-niri`) | n/a — free functions (`decode_event`, `translate`) | n/a (see below) | `tests.rs` decodes literal niri wire-format JSON strings (e.g. `r#"{"OverviewOpenedOrClosed":{"is_open":true}}"#`) through `decode_event`/`translate` and asserts the resulting `domain::Event`s | not run in CI | This already exercises the exact seam an app would use, at unit-test granularity, for the known niri IPC event types plus the `Unknown`-event fallback and malformed-payload rejection. It's inline literals rather than checked-in fixture files (unlike `rmac-audio`'s `pw-dump-laptop.json`); recording real fixture files from the reference laptop's `niri msg -j event-stream` would strengthen this further but wasn't started here — flagged as a follow-up. |
| Settings store (`rmac-shell-settings`) | `ShellSettingsStore<B: rmac_storage::Backend>` (pre-existing; already generic) | `rmac_storage::fake::InMemoryBackend` (new, in `rmac-storage`, not `rmac-shell-settings` itself — see below) | `settings_round_trip_through_an_in_memory_backend` in `rmac-shell-settings/src/tests.rs` | every other existing `rmac-shell-settings` test, which uses the real `FileSystem` in a tempdir | `ShellSettingsStore` already had a `with_backend(path, backend)` constructor; the gap was that `rmac-storage::Backend` (the trait it's generic over) had only the real host `FileSystem`, so every test — in this crate and in every other store built the same way — touched real disk. |
| Durable storage (`rmac-storage`) | `Backend` (pre-existing, already an injectable trait with `Unsupported`-by-default methods) | `fake::InMemoryBackend` (new) | `contract::assert_backend_contract`, run against **both** `InMemoryBackend` and the real `FileSystem` (in a scratch tempdir) in the same non-`#[ignore]` test | see above — the contract *is* the live test here | This is the highest-leverage fix in this pass: every durable store in the product (`rmac-shell-settings`, and by the same pattern `rmac-notes-storage`, `rmac-focus-store`, `rmac-theme`'s store, `rmac-wallpaper-*`, `rmac-recent-documents`, etc., wherever they're generic over `Backend`) can now be tested without touching disk, for free, without changing those crates. Only `rmac-shell-settings` was actually wired up to prove it in this pass (see above); wiring the others is unstarted. |
| Quick-settings backend (`rmac-quick-settings-system`) | `Backend` (pre-existing; unifies wifi/bluetooth/audio/power/focus for the quick-settings panel) | `fake::FakeBackend` (promoted from a private struct inside `tests.rs` to a public module) | this crate's own `tests.rs`, now importing `fake::FakeBackend` instead of redefining it | none (would just re-exercise the per-service live contracts above) | **Known remaining gap:** `crates/quick-settings-app/src/view.rs:449` constructs `rmac_quick_settings_system::SystemBackend` directly inline, with no injection seam and no `tests.rs` in `quick-settings-app` at all. `FakeBackend` is now reusable, but nothing in `quick-settings-app` uses it yet — wiring the view to accept `&impl Backend` (or a boxed backend) and adding app-level tests is the natural next step, left out of this pass to keep the change additive rather than touching the view's construction path. |
| App catalog (`rmac-apps`) | none | none | none | none | `catalog::discover()` walks real XDG desktop-entry directories and `catalog::launch()` spawns real processes; both are free functions with no seam. This is the largest remaining gap against the todo list (`AppCatalog` is named explicitly). A scoped fix would mirror the audio/power pattern: a small trait covering `discover`/`launch`/`file_association`, a `FakeAppCatalog` seeded with a `Vec<Application>`, leaving `desktop_group_named`'s parsing (already unit-tested) untouched. Not started in this pass. |
| File operations (`finder`) | `file_ops.rs::FileSystem` (pre-existing) | `file_ops.rs::FakeFileSystem` (pre-existing, private to `finder`) | `finder`'s own tests | none | Already follows the fake pattern in spirit; the only gap versus this matrix's convention is that it isn't exported behind `test-support` for reuse outside `finder` (nothing outside `finder` needs file operations today, so this is low priority). |
| Portals (`rmac-portal`) | none | none | none | none | Calls `ashpd` (async D-Bus portal bindings) directly for file choosers and open-URI requests, which pop real user-facing dialogs. `ashpd` has no fake backend of its own to build against; faking this meaningfully would mean wrapping `ashpd`'s async request types behind a trait, which is a larger design task than an additive pass should take on. Left as a documented gap. |
| Theme/appearance (`rmac-theme`) | `ThemeStore<B: rmac_storage::Backend>` (pre-existing, generic like `ShellSettingsStore`) | `rmac_storage::fake::InMemoryBackend` (reusable, not yet used here) | none yet | `rmac-theme`'s existing tests (real `FileSystem`/tempdir) | Confirmed: `crates/rmac-theme/src/store.rs:85` is `impl<B: Backend> ThemeStore<B>`, so it gets `InMemoryBackend` for free the same way `rmac-shell-settings` did (add `rmac-storage` as a `test-support` `dev-dependency` and a round-trip test through `with_backend`) — not wired up in this pass. Wallpaper D-Bus/portal calls in `wallpaper.rs` are a separate, unaudited surface. |
| Activity monitor sampling (`activity-monitor`) | none | none | none | none | `sampling.rs` reads `sysinfo::Networks`, `/proc/stat` CPU ticks, and host stats directly; fully live, no seam. Not started in this pass. |
| Compositor actions/state (other consumers: `rmac-dock-system`, `rmac-launcher-system`, `rmac-places-system`, `rmac-locale`, `rmac-system-info`, `rmac-time`, `rmac-session`, `rmac-updates`, `rmac-shell-runtime`, `rmac-launcher-providers`) | various, each pre-existing | each has a private `Fake*` inline in its own `#[cfg(test)] mod tests` | each crate's own tests | none | Same shape as the gap this pass closed for `rmac-quick-settings-system`: the fake already exists but is invisible outside its crate. Promoting each to a public `fake` module behind `test-support` is mechanical, low-risk, and unstarted here — a good next batch, one crate at a time, following the exact diff shape of the `rmac-quick-settings-system` commit in this branch's history. |

## Running the live contract tests

The mutating fakes above (`WifiService`, `BluetoothService`, `PowerService`,
`AudioService`) split their contract into a safe, read-only half that can
run against the real backend, and a mutating half that only ever runs
against the fake. The read-only half is wired up as an `#[ignore]`d test
per crate; run it deliberately, one crate at a time, on the reference
laptop over SSH (see `AGENTS.md`/the shared agent brief for the exact SSH
and environment invocation):

```sh
cargo test -p rmac-network  -- --ignored system_wifi_service_snapshot_is_well_formed
cargo test -p rmac-bluetooth -- --ignored system_bluetooth_service_snapshot_is_well_formed
cargo test -p rmac-power    -- --ignored system_power_service_snapshot_is_well_formed
cargo test -p rmac-audio    -- --ignored system_audio_service_snapshot_is_well_formed
```

Never run the mutating contract (`assert_*_service_contract`, as opposed
to `assert_*_service_is_observable`) against a `System*Service` on a
shared machine: it toggles Wi-Fi/Bluetooth power, joins/forgets networks,
pairs/removes devices, and switches the audio/power profile, any of which
would disrupt other agents' concurrent SSH sessions or work on the same
laptop. The fake-only tests already run under the crate's normal
`cargo test -p <crate>` (no `--ignored` needed).

`rmac-storage`'s `assert_backend_contract` is the one contract in this
matrix that's unconditionally safe live: it runs identically against
`InMemoryBackend` and the real `FileSystem` (in a fresh tempdir it creates
and removes itself), inside the crate's normal, non-ignored test suite.

## What this pass did not do

- Did not wire any *app* crate (`quick-settings-app`, `system-settings`,
  `player`, `activity-monitor`, `finder`, launcher/dock crates) to
  actually construct a fake in its own tests — it made the fakes reusable
  (`pub mod fake` + `test-support` feature) and, for the settings store,
  proved the seam works end to end. Most app crates still have no
  `tests.rs` at all against their service dependencies; that's a larger,
  per-app follow-up.
- Did not touch `crates/system-settings`, `crates/text-editor`,
  `shell/bins/rmac-menubar`, `crates/rmac-app-launch`, or
  `crates/rmac-app-menu` — other agents were editing them concurrently.
- Did not add an `AppCatalog` fake, a `FileOperations` export outside
  `finder`, or anything for portals, theme, or activity-monitor sampling.
  Each is described above with a concrete, scoped next step.
- Did not restructure any crate's module layout beyond adding
  `fake.rs`/`contract.rs` and one `mod`/`pub mod` declaration each.

## Compile risk

None of this was built with `cargo` (see `AGENTS.md`: no Docker-based or
workspace-wide Cargo validation on this machine). Every new/changed file
was checked with `rustfmt --edition 2021 --check` only; visibility
(`pub`, `pub(crate)`, `pub(super)`) and trait-bound reasoning were checked
by hand against the existing source, not compiled. The coordinator should
run, at minimum:

```sh
cargo test -p rmac-network -p rmac-bluetooth -p rmac-power -p rmac-audio \
  -p rmac-storage -p rmac-shell-settings -p rmac-quick-settings-system
```

on the laptop before trusting this branch. The most likely failure modes,
in rough order of likelihood: a missed `pub(super)`/`pub(crate)`
visibility edge in one of the six new `fake.rs`/`contract.rs` pairs; a
borrow-checker conflict in `rmac-network::fake::activate` (mutating
`state.saved` while holding a borrow into `state.networks`); or a stale
doc-link (`[\`crate::...\`]`) that `cargo doc`/`rustdoc` would flag but
`rustfmt` does not check.
