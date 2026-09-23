# Journey 7 trace: Wi-Fi, Bluetooth, audio output, battery

Journey 7 (`todo.md` › Product journeys): "Join Wi-Fi, connect Bluetooth,
change audio output and check battery." This traces each step from the UI
surfaces that can reach it (top-bar status icons in `shell/bins/rmac-menubar`,
the Control Centre popover in `crates/rmac-quick-settings` /
`crates/rmac-quick-settings-system` / `crates/quick-settings-app`, and the
full panes in `crates/system-settings`) down to the backend crates
(`crates/rmac-network`, `crates/rmac-bluetooth`, `crates/rmac-audio`,
`crates/rmac-power`). It was built by reading the code; nothing here was run,
since this task had no cargo and only read-only laptop access.

## Architecture (already in place)

Three UI surfaces share the four backend crates, matching a real macOS split:

- **Top-bar status icons** (`shell/bins/rmac-menubar/src/main.rs`): clicking
  Wi-Fi or Battery opens a dedicated dropdown (`StatusMenuKind::Wifi` /
  `::Battery`, `menu_model.rs:236-239`). Clicking Bluetooth or Sound opens
  Control Centre instead (`main.rs:2019-2025`, comment: "Wi-Fi and Battery
  open their own menus under the icon; the rest open Control Center").
- **Control Centre** (`crates/quick-settings-app`, reducer in
  `crates/rmac-quick-settings`, backend adapter in
  `crates/rmac-quick-settings-system`): toggle-only by design
  (`crates/quick-settings-app/SPEC.md:19`: "Toggle Wi-Fi, Bluetooth, mute, or
  Focus; select only a power profile the host supports"). No network list,
  device list, or output picker here — that is intentional scope, not a bug.
- **System Settings** (`crates/system-settings/src/controller/{wifi,
  bluetooth,sound,power}`): the full flows — network list with password
  sheets, Bluetooth pairing with a real agent, output/input device switching,
  and the battery detail pane.

## Step-by-step trace

### List networks → pick → enter password → connected

- Backend: `crates/rmac-network/src/linux.rs` talks to NetworkManager only
  over D-Bus (`zbus`, `org.freedesktop.NetworkManager*`, `wc -l` 1639 lines,
  no `Command::new`/CLI parsing in this file). `WifiService` trait
  (`lib.rs:41-57`) has `snapshot`, `connect` (known/open networks),
  `connect_with_password` (new WPA/SAE networks), `connect_enterprise`
  (PEAP/MSCHAPv2), `forget`. Documented in detail in `docs/wifi.md`.
- Menubar quick list (`menu_model.rs:413-576`, `wifi_menu_rows`): known and
  open/Enhanced-Open networks get `StatusAction::Join` (direct
  `rmac_network::connect`, no password needed — the secret is already saved).
  An unknown secured network routes to `OpenSettings("wifi")`
  (`menu_model.rs:445-460`) instead of prompting inline.
- Full flow (`crates/system-settings/src/controller/wifi/*`): selecting a new
  secured network opens a password sheet
  (`controller/wifi/render/dialogs.rs:75-100`, masked `TextField`), shows
  "Connecting securely…" / "Verifying and connecting…" progress
  (`dialogs.rs:85,209`), and surfaces failures via `self.wifi_error`
  (`controller/wifi/connection.rs:26,57,85,93`; e.g. "The saved Wi-Fi network
  is no longer available."). A one-shot NM `SecretAgent` delivers the
  password (`secret_agent.rs`) and the profile is never stored by rmac.
- **Works:** full list→password→connected flow, with visible "Connecting…"
  and error text, exists end to end in System Settings. Known/open quick-join
  from the menubar works.
- **Fixed in this pass (small, per the brief's own example):** the menubar's
  quick-join path previously had no visible connecting or failure state —
  `run_status_action`'s `StatusAction::ToggleWifi` and `StatusAction::Join`
  handlers only `eprintln!`'d on failure, and `Join` closes the menu
  immediately (`StatusAction::closes_menu()`), so a failed quick join or
  radio toggle produced no user-visible signal at all. Now:
  - `WifiMenuInput` gained `joining: Option<&WifiNetworkId>` and
    `error: Option<&str>` (`menu_model.rs:418-428`); `TopBar` gained matching
    `wifi_joining`/`wifi_error` state (`main.rs:465-471`).
  - While a join is in flight, its row shows a "Connecting…" detail line and
    cannot be re-triggered (`network_row`'s `joining` guard,
    `menu_model.rs:454-476`; `wifi_menu_rows`, `menu_model.rs:501-605`).
  - `ToggleWifi`/`Join`/`ToggleLowPower` failures now surface a dismissible
    warning row above the switch (`StatusAction::DismissWifiError`/
    `DismissBatteryError`, `main.rs:873-880`) instead of only `eprintln!`ing;
    the `eprintln!` calls stay as a debug trail alongside the new UI state,
    not in place of it (`main.rs:756-871`).
  - Reopening the Wi-Fi or Battery menu after a background failure still
    shows the banner, since the state lives on `TopBar`, not the transient
    row list.
  - Not done: a known network whose *saved* secret has gone stale still has
    no inline "enter a new password" affordance — the banner is dismissible
    and explains the failure, but recovery still requires the existing
    "Wi-Fi Settings…" item at the bottom of the same menu. A full inline
    password sheet inside the native status-menu row model would be a
    materially bigger change (new row type, masked text-entry state) than
    the brief's "small" scope, and this task had no way to compile-check a
    change to GPUI window/focus code.
- **Not faked:** no stub/mock data found in any of the four backend crates or
  their UI wiring (grepped for `TODO|FIXME|unimplemented!|todo!|stub|fake|
  mock|placeholder|hardcoded`; the only hit was an unrelated Now Playing
  artwork placeholder in `quick-settings-app/src/render.rs:69`).

### List Bluetooth devices → connect

- Backend: `crates/rmac-bluetooth/src/linux.rs` is pure BlueZ D-Bus
  (`org.bluez.Adapter1`/`Device1`/`AgentManager1`, `zbus`), no CLI. `pair()`
  registers a real `org.bluez.Agent1` implementation
  (`pairing_agent.rs:437-530`) that handles PIN/passkey/confirm/authorize
  callbacks with a 60 s timeout, zeroizes secrets on drop, and re-validates
  device identity before every mutation (`device_context`,
  `linux.rs:267-331`, guards against stale/duplicate-name rows). Documented
  in `docs/bluetooth.md`.
- Control Centre: toggle-only (`Command::SetBluetoothPowered`,
  `rmac-quick-settings/src/model.rs:39`) — no device list, matching the
  documented SPEC.md scope.
- Menubar: no `StatusMenuKind::Bluetooth` exists at all (only `Wifi` and
  `Battery`, `menu_model.rs:236-239`); the Bluetooth icon opens Control
  Centre, which is toggle-only.
- Full flow: `crates/system-settings/src/controller/bluetooth.rs` +
  `controller/bluetooth/` has the real device list, pair, forget, and detail
  UI, backed by the same agent above.
- **Works:** pairing, connect/disconnect, forget, live BlueZ signal watching
  (`watch.rs`), all D-Bus, well tested (`pairing_agent.rs` has 6 unit tests
  for prompt/cancel/timeout/reject state machines; `tests.rs`).
- **Missing:** no quick way to pick a *specific* device from the top bar or
  Control Centre — only through System Settings. This is a documented,
  intentional scope decision (SPEC.md), not a bug; adding a device list to
  Control Centre is an M-sized feature, out of scope for "small" fixes here.
  Also: `cancel_bluetooth_pairing` in
  `crates/system-settings/src/controller/bluetooth/pairing.rs` drops the
  result of `rmac_bluetooth::cancel_pairing(&device_id)` with `let _ =`
  (the sheet already resets its local state regardless, and BlueZ's own
  authoritative-snapshot design means a stale cancel self-corrects on the
  next signal, so this is lower stakes than a connect/pair failure). Left
  unfixed: this controller has no existing logging convention to route a
  background-task error through (no `eprintln!`/`tracing` calls anywhere in
  `controller/bluetooth/`), and reaching back into the live `Settings` entity
  from a detached `cx.background_executor()` task to show a banner needs the
  same `cx.spawn(async move |this, cx| …)` + weak-entity pattern the menubar
  fix above uses — a change to unfamiliar GPUI controller code this task
  could not compile-check.
- **Not faked.**

### List outputs → switch

- Backend: `crates/rmac-audio/src/linux.rs` is **not** D-Bus or a PipeWire
  client library — it still shells out — but as of this pass every **read**
  comes from `pw-dump --no-colors` (JSON, parsed with `serde_json`), and
  every **mutation** still calls `wpctl`/`pw-cli`, checking only the exit
  status:
  - `system_snapshot`, `machine_devices`, `system_default_device`: one
    `pw-dump --no-colors` call each, parsed into a `GraphMetadata` by
    `parse_pw_dump_metadata` (`linux.rs:848-992`). Device lists come from
    `graph_devices` (`linux.rs:816-845`); default-device marking comes from
    the PipeWire `default` metadata object's `default.audio.sink` /
    `default.audio.source` keys (`GraphMetadata::default_sink` /
    `::default_source`, resolved by node name, not node id, since PipeWire
    only exposes the default by name). Volume/mute come from each node's
    `Props.channelVolumes`/`mute` (`parse_node_level`, `linux.rs:1252-1272`):
    `channelVolumes` is linear, so the value is averaged across channels and
    cube-rooted to match what `wpctl`/pavucontrol display — verified against
    a real capture from the reference laptop (raw `8e-6` ↔ `wpctl`-displayed
    `0.02`, i.e. `0.02³ == 8e-6`). This also fixed a latent bug: real
    ALSA-backed nodes have a *second* `Props` array entry (an ALSA-route
    entry with no `channelVolumes`), which the old single-entry assumption
    in balance parsing silently rejected; `volume_props`
    (`linux.rs:1220-1244`) now finds the one entry that actually advertises
    `channelVolumes` instead.
  - `system_set_volume`/`system_set_muted`/`system_set_default_device`/
    `system_set_profile`/`system_set_route`/`system_set_balance`: all call
    `wpctl set-volume|set-mute|set-default|set-profile|set-route` or
    `pw-cli set-param`, through a shared `pipewire_mutation`/`wpctl_mutation`
    helper (`linux.rs:314-345`) that checks only the exit status (never
    stdout text) and appends a recovery hint ("check that PipeWire and
    WirePlumber are running…") to the error detail on failure.
  - The live-update watcher now spawns `pw-dump --monitor --no-colors`
    (JSON stream) instead of `pw-mon --color=never` (`linux.rs:40-52`),
    per the brief's preference for the machine-readable monitor. It still
    only treats **any** stdout byte as a "something changed, re-read the
    authoritative state" trigger and never parses the JSON — the same
    debounce (`WATCH_QUIET_PERIOD`/`WATCH_MAX_COALESCE`) is unchanged. Two
    read-only captures on the reference laptop (5 s and 10 s, idle) showed
    `pw-dump --monitor` emit only the initial dump and nothing further,
    unlike the "~1100 events/s" `pw-mon` storm logged in
    `COMPLETION_SPEC.md` §8.6 from an earlier investigation — but that
    number could not be reproduced from `pw-mon` either in this session, so
    treat the storm as environment/history-dependent, not something this
    change is verified to fix. `crates/rmac-shell-status-linux/src/watch.rs`
    runs a **second**, independent `pw-mon` watcher for the top-bar/menubar
    and was intentionally left untouched (out of this task's scope, which
    was `rmac-audio/src/linux.rs` only) — `COMPLETION_SPEC.md` already notes
    the two watchers should eventually be consolidated.
  - `wpctl status`/`wpctl inspect` text parsing (`parse_wpctl_list`,
    `parse_wpctl_default_inspect`, `parse_wpctl_level`) is gone entirely.
    This was, for Linux, the one clear violation of the todo's "Use platform
    services, not command output… Never parse human-readable CLI output on
    Linux" rule; it is now resolved for every read path in this crate.
- **Why not a full native PipeWire client:** the brief for *this* pass was
  explicit — make the existing shell-out design JSON-only, without adding a
  new FFI dependency. `pipewire`/libpipewire FFI (a real fix that also drops
  the remaining subprocess-per-read cost) is still a multi-week rewrite of
  the async main-loop integration, device graph model and every mutation
  path against a different threading model, and remains future work, not
  done here.
- Output *switching* specifically: `system_set_default_device` exists and is
  wired to System Settings' Sound pane (`crates/system-settings/src/sound.rs`,
  `controller/sound.rs`), which is the only place a user can actually pick a
  different output device today — Control Centre only has volume/mute
  (`rmac-quick-settings/src/model.rs:38-44`, matching SPEC.md's documented
  scope) and the menubar Sound icon just opens Control Centre.
- **Parsing correctness:** the `pw-dump` parsers are well covered
  (`rmac-audio/src/tests.rs`: cubic volume-scale conversion, default-metadata
  resolution, multi-entry `Props` array handling, JSON graph correlation,
  balance clamping, and a real (sanitized) `pw-dump` capture from the
  reference laptop as a fixture — `rmac-audio/src/fixtures/pw-dump-laptop.json`
  — 25+ tests total).
- **Not faked.** Reads are now single atomic `pw-dump` snapshots (one
  subprocess call instead of the previous `wpctl list` + `wpctl inspect` +
  `pw-dump` combination), which also removes a race that used to exist
  between separately-timed reads disagreeing about device identity.
- **Still to verify on the laptop** (no cargo/build access in this pass):
  a full `cargo build -p rmac-audio` and `cargo test -p rmac-audio`; that
  Quick Settings/menubar/Sound pane volume sliders and mute toggles show the
  same values `wpctl status` reports after this change; that switching
  default device/profile/route/balance still works end-to-end; and ideally
  a longer, real idle-vs-active `pw-dump --monitor` event-rate measurement,
  since the two read-only captures here (5 s and 10 s) could not reproduce
  the historical `pw-mon` storm either way.

### Battery percentage and time remaining

- Backend: `crates/rmac-power/src/linux.rs` is fully D-Bus (`zbus`,
  `org.freedesktop.UPower`), no CLI. Reads `TimeToFull`/`TimeToEmpty` into
  `Battery.seconds_remaining: Option<u64>` (`linux.rs:75-79`,
  `model.rs:36`).
- Menubar Battery menu (`menu_model.rs:585-625`, `battery_menu_rows`): shows
  percentage and power source (Battery/Power Adapter) and, when supported, a
  Low Power Mode toggle. **Does not show `seconds_remaining`** — there is no
  time-remaining row in this dropdown.
- System Settings Battery pane
  (`crates/system-settings/src/controller/power/render.rs:114-125`): shows
  "Time to Full" or "Time Remaining", formatted by
  `system-settings/src/power.rs:29-36` (`format_duration`), alongside energy
  rate, battery health, and cycle count.
- **Works:** percentage everywhere; time remaining is read correctly by the
  backend and displayed correctly in System Settings' Battery pane.
- **Missing, deliberately not added:** the quick menubar Battery dropdown
  omits time remaining. This was measured, not guessed: the reference
  capture `target/evidence/mac-2026-09-23/ax/status-Battery.png` (real
  Tahoe menu bar, 38% on battery, Discharging) shows only "Battery 38%",
  "Power Source: Battery", the Energy Mode section, and "Using Significant
  Energy" — no time-remaining line anywhere. A draft of this fix (now
  reverted, `menu_model.rs` history) added a "Time Remaining"/"Time to Full
  Charge" row whenever `seconds_remaining` was `Some`; it was removed because
  it contradicts that capture and the project's "measure, never invent
  numbers" rule. `battery_menu_rows` now has a comment
  (`menu_model.rs:634-641`) pointing at the same evidence so the next agent
  doesn't reintroduce it without a charging-state capture to justify it.
  The reference capture also shows a "Using Significant Energy" per-app
  section rmac does not have at all; that's a separate, larger feature
  (per-process energy accounting), out of scope here.
- **Not faked.**

## Fixes made in this pass

- `shell/bins/rmac-menubar/src/menu_model.rs` and `src/main.rs`: the
  menubar's quick Wi-Fi join and the Wi-Fi/Low Power toggles now carry
  visible busy ("Connecting…") and dismissible-error state instead of only
  `eprintln!`ing on failure. See the Wi-Fi trace section above for the exact
  mechanism. Added unit tests `a_network_being_joined_shows_connecting_and_
  cannot_be_reactivated`, `a_join_or_radio_failure_shows_a_dismissible_
  banner`, and `a_battery_mutation_failure_shows_a_dismissible_banner` in
  `menu_model.rs`.
- `crates/system-settings/src/power.rs`: added
  `duration_formats_hours_minutes_and_bare_minutes`, a direct unit test for
  the battery "Time Remaining"/"Time to Full" formatter (`format_duration`),
  which had no direct test even though it's exercised through `render.rs`.
- Considered and reverted: a "Time Remaining" row in the menubar's
  `battery_menu_rows` (see the battery trace section above for why).
- `crates/rmac-audio/src/linux.rs`: moved every audio **read**
  (`system_snapshot`, `machine_devices`, `system_default_device`) from
  `wpctl list`/`wpctl inspect`/`wpctl get-volume` text parsing to
  `pw-dump --no-colors` JSON only, resolving the audio violation noted in
  the "List outputs → switch" section above (see that section for the full
  detail, including the cubic volume-scale fix and the multi-entry `Props`
  array bug this also fixed). Mutations still call `wpctl`/`pw-cli`,
  checking only exit status. The live-update watcher now runs
  `pw-dump --monitor` instead of `pw-mon`. Added
  `crates/rmac-audio/src/fixtures/pw-dump-laptop.json`, a sanitized real
  capture from the reference laptop, plus new/updated unit tests in
  `rmac-audio/src/tests.rs`.

## What still needs the reference laptop or the Mac to verify

- Real NetworkManager/BlueZ/UPower/PipeWire interaction: `docs/wifi.md` and
  `docs/bluetooth.md` both list their own explicit open evidence gates (F1,
  F2) — restart/permission/cancellation/timeout behavior against live
  services, which this task could not exercise (no cargo, no service
  mutation on the reference laptop per the brief).
- The new Wi-Fi "Connecting…"/error-banner code in `rmac-menubar` could only
  be unit tested at the pure-function level (`menu_model.rs`); it was never
  compiled, since `rmac-menubar` only builds under
  `cfg(all(target_os="linux", feature="wayland"))` and this task had no
  cargo. It needs a Linux build plus a real failing join (e.g. a saved
  network whose password recently changed) to confirm end to end.
- Whether macOS Tahoe's Battery dropdown shows a time estimate while
  actively *charging* (the only capture on hand is discharging at 38%) — a
  charging-state capture would confirm or rule out adding that case only.
- The `rmac-audio` JSON-only rewrite (see "Fixes made in this pass" and the
  "List outputs → switch" section) needs `cargo build -p rmac-audio` and
  `cargo test -p rmac-audio` on Linux — this pass had no cargo access, so it
  was verified only by `rustfmt` and manual review, plus real (read-only)
  `pw-dump`/`wpctl status` captures from the reference laptop used to build
  and cross-check the fixture and the cubic volume-scale math. After a
  build, check: Quick Settings/menubar/Sound-pane volume and mute match
  `wpctl status`; default-device/profile/route/balance switching still
  works; and, if practical, a longer idle-vs-active `pw-dump --monitor`
  event-rate measurement (the two short captures taken here did not
  reproduce the `pw-mon` event storm logged in `COMPLETION_SPEC.md` §8.6,
  in either direction).
