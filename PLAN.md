# rmac — a fast, macOS-feel desktop suite in Rust for Ubuntu

> Goal: capture the macOS *ergonomics* (dock, global menu, Spotlight, smooth
> gestures, instant native apps) on Ubuntu/Wayland — without cloning Apple
> assets or chasing pixel-perfection. "Feels as good as macOS," not "is a copy."

## Strategic decisions (locked)

| Decision | Choice | Rationale |
|---|---|---|
| Build direction | **Top-down**, not bottom-up | Don't write a compositor first. Ride an existing Rust compositor; replace pieces incrementally. The trap that kills these projects is months of DRM/KMS plumbing before any visible UI. |
| UI toolkit | **GPUI** (Zed's framework) + `gpui-component` 0.5.1 | Highest ceiling for 120fps macOS-feel. Accepted tradeoffs: pre-1.0 API churn, weak docs, **zero accessibility (we build it)**. |
| Compositor (target) | **niri** or **cosmic-comp** (Smithay) | Already smooth, GPU-animated, gesture-capable, Rust. Configure/fork, don't write from scratch. |
| Text | GPUI text system (cosmic-text / HarfRust shaping) | macOS-quality typography is a solved problem in Rust now. |
| System data | `sysinfo` + `starship-battery` + `nvml-wrapper` | Backbone of every utility. |
| Fonts | **Inter** (UI) + **JetBrains Mono** | NEVER ship SF Pro/SF Mono — Apple license forbids non-Apple-OS use. |

## Prior art (the map)

- **COSMIC** (System76) — first production Rust desktop environment, stable Dec 2025.
  Five-layer arch: DRM/KMS → Smithay → wgpu → iced → libcosmic. Took ~3 years.
  Read `cosmic-comp` (compositor) and `libcosmic` (toolkit/theming/config) for patterns.
- **niri** — Smithay-based scrollable-tiling compositor; smaller, readable codebase;
  custom-shader animations + touchpad gestures. Best reference for compositor + motion.

## The hard parts (design around these)

1. **Global menu bar is broken on Wayland for GTK/Electron apps** — the #1 reason
   Linux macOS-clones "feel off." Don't make it load-bearing until solved at compositor level.
2. **App visual consistency is unsolvable by theming** (GTK3/GTK4/Qt/Electron mix).
   Our *own* apps will be consistent; third-party ones won't.
3. **Fractional scaling** still has blurry XWayland stragglers. Plasma/Wayland is best base.
4. **Don't theme GNOME** — libadwaita ignores custom themes by design.

## Performance rules (the #1 requirement)

- Native binaries, **no webview** (no Electron/Tauri for core apps).
- **GPU-render everything** (quads/glyphs/shadows via Metal/Vulkan). This enables 120fps.
- Release flags: `lto=true`, `codegen-units=1`, `panic="abort"`, `strip=true` (already in workspace Cargo.toml).
- Decouple state updates from render; sync render loop to display refresh.
- `sysinfo`: keep ONE `System` instance, refresh in place (works on diffs).

## Roadmap

- **Phase 0 — Foundation** ✅ workspace + toolkit decision (GPUI).
- **Phase 1 — Activity Monitor** 🟡 IN PROGRESS — `crates/activity-monitor`.
  Runs on macOS today (Metal); exercises GPUI + gpui-component Table + sysinfo.
  *(Dock/panels need Wayland layer-shell → Linux-only → deferred until testing on Ubuntu.)*
- **Phase 2 — Dock** (Linux) — layer-shell, hover magnification, spring physics, launch.
- **Phase 3 — Spotlight + top bar** — instant fuzzy launcher; clock/tray panel.
- **Phase 4 — More utilities** — file manager (copy yazi's async-I/O arch), settings, screenshot, clipboard.
- **Phase 5 — Notes app** (flagship) — rich text (cosmic-text), folders, fast full-text search.
- **Phase 6 — The *feel*** — inertial gestures (in compositor), global menu bar, Mission-Control overview, packaging as Ubuntu remix.

## Build setup notes

- **macOS dev requires the Metal Toolchain** (Xcode 26 split it out):
  `xcodebuild -downloadComponent MetalToolchain` (one-time, multi-GB).
- Run the monitor: `cargo run -p rmac-activity-monitor` (use `--release` for real perf numbers).
