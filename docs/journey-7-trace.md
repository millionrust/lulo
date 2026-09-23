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
- **Not faked.**

### List outputs → switch

- Backend: `crates/rmac-audio/src/linux.rs` (1458 lines) is **not** D-Bus or
  a PipeWire client library — every read and mutation shells out to
  **`wpctl`** (WirePlumber CLI) and parses its text, plus `pw-dump --no-colors`
  (JSON, still a subprocess) for port/profile/route metadata:
  - `system_default_device`, `read_default_level`: `wpctl inspect …` /
    `wpctl get-volume …`, parsed by `parse_wpctl_default_inspect` /
    `parse_wpctl_level` (`linux.rs:252-282,780-854`).
  - `machine_devices`: `wpctl list audio sinks|sources`, parsed by
    `parse_wpctl_list` (`linux.rs:283-296,887-969`).
  - `system_set_volume`/`system_set_muted`/`system_set_default_device`/
    `system_set_profile`/`system_set_route`/`system_set_balance`: all call
    `wpctl set-volume|set-mute|set-default|set-profile|set-route` etc.
    (`linux.rs:313-680`).
  - The live-update watcher spawns `pw-mon --color=never` and treats **any**
    stdout byte as a "something changed, re-read the authoritative state"
    signal (`linux.rs:29-119`) — it does not parse `pw-mon`'s output, only
    uses it as a change tickler, but it is still a CLI subprocess kept
    running as the watch mechanism.
  - This is the one clear violation, for Linux, of the todo's "Use platform
    services, not command output… Never parse human-readable CLI output on
    Linux" rule and of the brief's "flag any backend that parses
    nmcli/bluetoothctl/pactl/upower text" instruction.
- **Why not fixed here:** the brief says to do the most-used replacement
  (Wi-Fi) and document the rest if a replacement is large. Wi-Fi already had
  no violation to fix (see above), so audio is "the rest." Unlike
  NetworkManager/BlueZ/UPower, PipeWire has **no D-Bus API**; a real fix means
  adding a `pipewire` (libpipewire FFI) crate dependency — not present in
  `Cargo.lock` today — and rewriting the async main-loop integration, device
  graph model, and every mutation path (~1450 lines) against a different
  threading model (libpipewire's own loop vs. this crate's
  `async_process`/thread pattern). That is a multi-week rewrite, not a small
  fix, and untestable here without cargo. Recommend a dedicated task with
  laptop build access.
- Output *switching* specifically: `system_set_default_device` exists and is
  wired to System Settings' Sound pane (`crates/system-settings/src/sound.rs`,
  `controller/sound.rs`), which is the only place a user can actually pick a
  different output device today — Control Centre only has volume/mute
  (`rmac-quick-settings/src/model.rs:38-44`, matching SPEC.md's documented
  scope) and the menubar Sound icon just opens Control Centre.
- **Parsing correctness:** despite the architecture issue, the existing
  `wpctl`/`pw-dump` parsers are well covered (`rmac-audio/src/tests.rs`: PSK/
  volume/mute parsing, malformed/ambiguous list rejection, JSON graph
  correlation, balance clamping — 20+ tests already present).
- **Not faked**, just built on CLI text where every other backend uses D-Bus.

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
- The `rmac-audio` CLI-parsing rewrite needs a full build/test cycle on
  Linux; it was only documented here, not attempted.
