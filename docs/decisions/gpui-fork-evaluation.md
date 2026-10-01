# GPUI fork evaluation

- Date: 2026-10-01
- Status: evaluated; keep the current product dependency through Beta
- Baseline: Zed GPUI `0.2.2` at `76c93968`, patched `shell/compat/gpui_linux`, vendored gpui-component `0.5.2`
- Candidates: [gpui-fast `1b381adb`](https://github.com/longbridge/gpui-fast/commit/1b381adb68a6e5a4550f73b0cd29a231ce6c7281) and [gpui-ce `ec5e7808`](https://github.com/gpui-ce/gpui-ce/commit/ec5e7808d2cc6f5473b4827e463dbeb5e39bcfbf)

## Recommendation

**Do not switch to either fork before Beta.** Keep ADR 0013's pinned Zed backend. With accessibility active, gpui-fast reduced Files list-scroll process CPU by only 3.8% in the nested software-rendered comparison, while search-typing CPU rose 7.3% and private memory rose slightly. Both variants parked at effectively zero CPU during 60-second idle periods. That trade does not justify porting and maintaining Lulo's Linux touch, accessibility and scroll patches before Beta. Reconsider gpui-fast after ADR 0015's gpui-kit migration with a real-GPU frame test. gpui-ce needs a separate framework migration plan rather than a `[patch]` swap. This evaluation does not change the product pin.

The primary expected gpui-fast benefit is selective redraw and relayout. Its [retained-mode guide](https://github.com/longbridge/gpui-fast/blob/1b381adb68a6e5a4550f73b0cd29a231ce6c7281/docs/retained-mode.md) says retained subtree reuse is disabled while accessibility is active; the code checks `self.a11y.is_active()` in `fast/retained.rs`. Lulo intentionally registers its app windows with AT-SPI as soon as accessibility is enabled (ADR 0013). All measurements below activated and read the app's AT-SPI tree before timing idle or input. The modest measured change is consistent with this source-level limitation, although the benchmark does not isolate retained reuse as the cause.

## Source and integration audit

| Topic | gpui-fast | gpui-ce |
|---|---|---|
| Purpose and activity | Experimental retained mode and window composition; latest pinned commit 2026-09-30 merged [PR 23](https://github.com/longbridge/gpui-fast/commit/1b381adb68a6e5a4550f73b0cd29a231ce6c7281), adding a `gpui-pre` compatibility package. It tracks Zed `7960b2a7` per `UPSTREAM`. | Independently developed community edition; latest pinned commit 2026-09-30. [README](https://github.com/gpui-ce/gpui-ce/blob/ec5e7808d2cc6f5473b4827e463dbeb5e39bcfbf/README.md) says it is mostly API compatible today but changing. |
| License | [Apache-2.0](https://github.com/longbridge/gpui-fast/blob/1b381adb68a6e5a4550f73b0cd29a231ce6c7281/LICENSE-APACHE). | [Apache-2.0](https://github.com/gpui-ce/gpui-ce/blob/ec5e7808d2cc6f5473b4827e463dbeb5e39bcfbf/LICENSE.md). |
| Linux / Wayland | Own `gpui_linux` and `gpui_platform`; Wayland, x11, layer shell and WGPU source present. `gpui` remains package `0.2.2`. | Own `gpui_ce_linux` and `gpui_ce_platform`; Wayland, x11, layer shell and WGPU source present. Core is package `gpui-ce 0.2.2`, library name `gpui`. |
| Cargo compatibility | The old Zed source can be `[patch]`ed with fork paths for `gpui`, `gpui_platform`, and `gpui_linux`. PR 23 also provides `gpui-pre 0.3.7`, but ADR 0015's gpui-kit `0.6.6` pins `gpui-pre =0.3.6`; that later route needs an aligned kit bump. | The package names differ. Root and vendored gpui-component manifests must alias `gpui-ce`, `gpui_ce_platform`, and `gpui_ce_macros` before `[patch.crates-io]` can point at the checkout. This is already a migration, not a source substitution. |
| Retained mode | Views and layout nodes are retained; source documents explicit invalidation for external `Rc<RefCell<_>>` or time reads and fallback when accessibility is active. The [README's performance figures](https://github.com/longbridge/gpui-fast#retained-mode) are upstream's workload, not Lulo or this laptop. | README describes mixed intermediate/retained design. No Lulo or same-workload measurements available. |
| Fork cadence risk | A small experimental fork, designed to rebase on Zed; APIs promised stable but internals explicitly experimental. Our patch stack must be rebased onto each import. | Independently versioned packages and an explicitly evolving API. It can drift from both Zed and gpui-kit/gpui-component. |

The table verifies each fork's core license; a full transitive license review for a shipping build remains separate. CE's build also fetched a git-pinned `wgsl-rs` dependency, increasing source and release coupling.

### Linux patch disposition

Source locations below refer to both pinned fork checkouts under `crates/gpui_linux/src/linux/wayland/` and the current Lulo backend under `shell/compat/gpui_linux/src/linux/wayland/`.

| Lulo behavior | gpui-fast | gpui-ce | Required spike work |
|---|---|---|---|
| Idle frame parking | `window.rs` has `FrameLoop::Parked` and frame waker. | Same. | Verify 60-second idle CPU with an accessible window; do not transplant the old frame loop mechanically. |
| Window geometry inside client frame | Both initial configure and resize use `inset_by_tiling` before `set_window_geometry`. | Same. | Verify Calculator and Files visible bounds on niri; no initial source transplant indicated. |
| Touchpad momentum on `AxisStop` | `client.rs` sends scroll events but has no `AxisStop` arm. | `client.rs` handles `AxisStop` through `kinetic_scroll`, with tests in `wayland/scroll.rs`. | Port the current or CE's equivalent kinetic scroll to fast; compare decay and cancellation with Lulo's behavior test. |
| Physical touchscreen | Neither backend binds `wl_touch`; pointer pinch events are separate. | Same. | Port Lulo's `wl_touch` contact state, tap/hold/scroll/drag classification, momentum and cancellation in `client.rs`/`window.rs`. |
| Title-bar drag threshold and touch serial | Lulo's threshold is in `rmac-ui`; fast's `start_window_move` uses a stored mouse-press serial. | Same. | Keep the app threshold; validate touch-down serial and edge resize on nested niri after touch port. |
| Accessibility proxy/text/toolkit patches | Fork Wayland window builds an AccessKit tree itself; Lulo's `collapse_text_proxies` and toolkit label are absent. | Same. | Port and re-test AT-SPI text, focus and application metadata. |
| Cross-process drag source | `start_external_drag` exists in the Wayland backend. | Same. | A fork switch alone would not complete the Files-to-Dock/Trash journey; product code must use the source API and pass a niri behavior check. |

The current patch stack changed five backend files since the untouched vendor import: `client.rs` (+796/-26 lines), `window.rs` (+232/-46), `a11y.rs` (+213), `x11/window.rs` (+5/-2), and `Cargo.toml` (+7/-6). That is **1,253 added lines to review**, not 1,253 lines to copy. For **fast**, minimum runtime parity looks like **3–5 backend files plus 2–3 manifests** after its two-call `rmac-ui` API port; plan on roughly **one to two weeks of engineering work** for the forward port and the touch, scroll, AT-SPI, geometry and window/input regression passes. That time is a planning estimate, not measured spike time. For **CE**, compilation already exposes at least 54 gpui-component files requiring adaptation before any app code is checked; this is a separate framework migration with no credible small-patch estimate. The source comparison alone cannot certify equivalent niri behavior.

## Spike setup and build results

`scripts/gpui-fork-eval/prepare.py` creates a disposable worktree `[patch]` overlay from the pinned fork checkout. It preserves the product manifest in this branch. On the laptop use one package-scoped Cargo pipeline at a time with `CARGO_TARGET_DIR=$HOME/rmac-wt/target` and `/tmp/lulo-cargo.lock`; check `df -h /home` immediately before every fork build and stop below 25 GiB. `scripts/gpui-fork-eval/measure.py` uses private nested niri inside a headless Sway parent, private XDG/D-Bus directories, one app process and the journey lock.

| Variant | Calculator build | Files build | Build wall | Disk evidence | Result |
|---|---:|---:|---:|---|---|
| Current pinned GPUI | Pass | Pass | 193.06 s initial build | Shared target already 82 GiB before fork work | Builds both binaries |
| gpui-fast `1b381adb` | Pass | Pass | 374.34 s first attempt + 148.22 s after two-call port = 522.56 s | Source checkout 23 MiB; shared target grew from 82 to 84 GiB; fork-crate cleanup after clippy/tests removed 3.2 GiB | Old vendored component compiled; `rmac-ui` needed two `Window::blur(cx)` calls |
| gpui-ce `ec5e7808` | Fail | Fail | 642.41 s to compiler stop | Source checkout 22 MiB; 766.3 MiB of CE framework artifacts removed by package clean; transitive dependencies remain shared | `gpui-component` had **412 errors across 54 files**, before either app compiled |

The CE errors are compiler diagnostics from the pinned source, not hypothetical API guesses: E0277 171, E0599 72, E0609 62, E0560 49, E0308 21, E0063 15, E0616 13, E0034 8 and E0119 1. They include `Hsla` moving to palette's alpha/color representation, `BoxShadow.color` taking `Background`, a conflicting `From<ThemeToken> for Fill`, renamed deferred priority, callback mutability changes, and schema traits. The 54-file spread rules out a minimum patch to the Linux backend as a route to two Lulo binaries. The offline metadata refresh initially lacked cached `palette` (and fast lacked `accesskit_windows`); the online build resolved these in the disposable laptop lockfile. No dependency manifest or lockfile change is proposed for the product branch.

Build times and disk figures reflect sequential work against the **shared warmed target**, with package-scoped commands and no concurrent Cargo pipeline. They include dependency graph changes and downloads and are not clean-room build-time benchmarks. The CE disk figure is the amount removed by cleaning five heavy CE packages, not the whole added graph. Fast's 3.2 GiB cleanup includes its post-build clippy/test artifacts.
Both fork package cleanups happened after their builds in the original spike; this runtime continuation rebuilt only the pinned product packages. The fork source checkout directory is empty and the disposable runtime worktree was removed after validation.

### Laptop measurements

Reference host: i5-5300U, Intel HD 5500. Each row is the median of **three fresh app processes**, each in a separate private nested niri/Sway session. Files runs used the same 5,000-file list and virtual input. Every CPU run waited 60 seconds at idle; Files then scrolled, typed in Search and hovered for 10 seconds each. Process CPU is a percentage of one core, so values above 100% reflect multiple busy threads. The runner uses pixman/Lavapipe through its headless parent, so CPU numbers compare the two variants in that software-rendered setup; **Intel GPU presentation time is not captured**. `start_to_window_ms` measures a cold process start to first compositor window with the page cache as found, not a power-cycle cold start. Private memory is `Pss_Anon + SwapPss` from `/proc/<pid>/smaps_rollup`; total PSS is given separately. Binary size is the unstripped `iterate` build. Raw runs are in `gpui-fork-evaluation-runs.json` beside this document.

| Variant / app | Idle CPU, 60 s | Files list scroll CPU | Files search typing CPU | Hover CPU | Private / total PSS | Binary size | Start to window | GPU frame time |
|---|---:|---:|---:|---:|---:|---:|---:|---|
| Current / Calculator | 0.000% | — | — | — | 59,132 / 136,724 KiB | 43,677,456 B | 1,505.6 ms | Not captured |
| Current / Files | 0.017% | 251.307% | 40.140% | 2.484% | 73,008 / 162,215 KiB | 60,380,360 B | 1,930.5 ms | Not captured |
| Fast / Calculator | 0.000% | — | — | — | 60,264 / 138,328 KiB | 42,294,064 B | 1,468.6 ms | Not captured |
| Fast / Files | 0.000% | 241.723% | 43.053% | 2.284% | 73,600 / 174,404 KiB | 57,900,544 B | 1,927.3 ms | Not captured |
| CE / Calculator | Compile blocked | — | — | — | — | — | — | Not captured |
| CE / Files | Compile blocked | — | — | — | — | — | — | Not captured |

For frame behavior, a **separate** three-run Files probe enabled `WAYLAND_DEBUG=client` and timed each `wl_surface.frame` request to its matching `wl_callback.done` during the same 10-second actions. This is callback scheduling latency under software rendering, **not** GPU frame presentation time or application render duration. The tracing overhead means these runs are excluded from the CPU table. Values are medians of each run's p50 and p95; sample counts are median counts per run.

| Files action | Current callbacks | Current p50 / p95 | Fast callbacks | Fast p50 / p95 |
|---|---:|---:|---:|---:|
| Long-list scroll | 169 | 17.69 / 79.68 ms | 125 | 57.53 / 69.60 ms |
| Search typing | 21 | 59.56 / 94.18 ms | 22 | 57.43 / 65.03 ms |
| Hover | 2 | 15.17 / 15.17 ms | 1 | 70.10 / 70.10 ms |

The scroll callback distribution does not show a clear frame-latency win for fast: its p95 is lower, but it delivered fewer callbacks and had a much higher median request-to-callback delay. The hover sample is too small to compare. A real-GPU presentation trace remains necessary before claiming a smoothness improvement.

Do not compare the upstream README's 144 Hz showcase numbers to the laptop's nested-runner numbers: the GPU, renderer, app structure and accessibility state differ. A real-GPU nested niri run with frame instrumentation is a remaining gate for any later migration decision.

The earlier journey-lock deadlock was resolved before these measurements. The runner itself takes `/tmp/lulo-journey.lock`; it was never wrapped by another lock holder. An initial private D-Bus policy rejected the AT-SPI activation call, so the runner now uses `dbus-run-session`'s standard private policy. A short smoke run then confirmed a populated AT-SPI tree, and all 12 full runs completed without a runner failure. No input was sent to the owner's live session.

### Validation

- Fast Calculator and Files package build: 2/2 binaries. The first compile stopped at two `Window::blur` calls; the small overlay port then built both.
- Fast package-scoped `cargo clippy --profile iterate -p rmac-calculator -p rmac-finder -- -D warnings`: pass, zero diagnostic errors. Laptop `cargo fmt --all -- --check`: pass.
- Fast `cargo test --profile iterate -p rmac-calculator --lib`: **98 passed, 0 failed, 0 ignored**.
- Current pinned GPUI, after the runtime run: laptop `cargo build --profile iterate -p rmac-calculator -p rmac-finder` built **2/2** binaries; package-scoped clippy with `-D warnings` passed; Calculator unit tests passed **98/98**, Files binary unit tests passed **233/233**, and `cargo fmt --all -- --check` passed. These ran sequentially under `/tmp/lulo-cargo.lock` using the shared target, with at least 34 GiB free during the validation.
- 12/12 fresh-process CPU/memory/start/size runs completed (3 per app and variant); 6/6 traced Files frame-callback runs completed (3 per variant). All 18 activated the AT-SPI tree in a private session. The separate parser's synthetic callback cases passed 2/2 (`@` and `#` Wayland log syntax).
- Local `scripts/check-gpui-component-imports.sh`: 47 files checked, none growing; `scripts/check-design-tokens.sh`: 161 values across 34 files, none growing. Both spike Python scripts passed `py_compile`; the isolated evaluation commit passed `git show --check`. The checkout has a separate, pre-existing unresolved merge in `docs/parity.md`, so a whole-tree `git diff --check` reports that conflict marker.
- CE framework crates compiled through its Wayland backend, but the first Lulo dependency (`gpui-component`) failed with the 412 errors above; no CE app binary, clippy or runtime test exists.

## Gates before reconsidering a switch

1. Resolve the framework and vendored component API breakage and produce Calculator plus Files binaries from one GPUI identity. Build a Settings or Text Editor window too, because text input is a distinct risk.
2. Port the missing Linux touch and accessibility patches, and kinetic scrolling for fast. Run the existing Calculator, Files and window-move nested behavior journeys and a physical touchscreen/touchpad check on niri.
3. The accessibility-active, software-rendered CPU and memory comparison above is complete. Before reconsidering, capture main-thread render and Intel HD 5500 presentation times under a real-GPU nested session, including missed frames and visual output. Repeat with accessibility inactive only to quantify the retained-mode ceiling, not as a substitute for the accessible path.
4. Keep the same no-polling and no-UI-thread-blocking rules, and demand a clear improvement large enough to justify the recurring backend and gpui-kit rebase cost.
