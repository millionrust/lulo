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

## App suite (target)

All six are **buildable & testable on macOS now** (unlike the Dock/panels, which need
Wayland layer-shell). Build the shared foundations once; apps compose them.

### Shared crates (build first, reused everywhere)

| Crate | Responsibility | Consumers |
|---|---|---|
| `rmac-ui` | Design system: theme tokens, fonts (Inter / JetBrains Mono), traffic-light `TitleBar` + window chrome, common widgets, window-position persistence | **every** app |
| `rmac-editor` | Text editing core: rope buffer (gpui-component `input`/Rope), undo/redo (`history`), syntax highlight (`highlighter`) | Text Editor, Notes |
| `rmac-apps` | Installed-app enumeration abstraction — `.desktop` (Linux) vs `.app` (macOS), icons, launch | App Drawer, future Spotlight + Dock |
| `rmac-sys` | System data + control: `sysinfo` wrappers, per-OS settings backends | Activity Monitor, System Settings |

### The apps → Rust approach

| # | App | Core crates / approach | Prior art | macOS-now? |
|---|---|---|---|---|
| 1 | **Terminal** | `alacritty_terminal` (VTE/grid engine) + `portable-pty`, GPUI grid render | **Zed's terminal** (same stack) | ✅ |
| 2 | **Notes** | `rmac-editor` + storage (`rusqlite` or files) + full-text search | Apple Notes | ✅ |
| 3 | **Finder** | `std::fs` + `notify` (watch) + async I/O (yazi arch); `tree`/`sidebar`/`list` | yazi, cosmic-files | ✅ |
| 4 | **System Settings** | `sidebar` + panes shell; backends per-OS (Linux: gsettings/dconf/compositor) | — | shell ✅, Linux backends later |
| 5 | **App Drawer** | Launchpad/App-Library grid + fuzzy search; `rmac-apps` enumeration | macOS Launchpad/App Library | ✅ |
| 6 | **Text Editor** | `rmac-editor` + `highlighter` (syntax) | macOS TextEdit | ✅ |

## Roadmap

- **Phase 0 — Foundation** ✅ workspace + toolkit decision (GPUI).
- **Phase 1 — Activity Monitor** ✅ DONE — `crates/activity-monitor` builds & runs on macOS. Live process table + summary, 2s auto-refresh. Proves the GPUI + gpui-component + sysinfo stack.
- **Phase 2 — `rmac-ui` foundation** ✅ DONE — traffic-light titlebar, theme, fonts, `boot()`. Monitor wired onto it.
- **Phase 3 — App suite** — build order: Text Editor ✅ → Notes ✅ (both share `rmac-editor` ✅) → Terminal ⏭️ NEXT → Finder → App Drawer → System Settings.
  - **Text Editor** ✅ — rope editor + native Open/Save.
  - **Notes** ✅ — two-pane sidebar/editor, `.md` files in `~/Documents/rmac-notes`, 1.5s auto-save.
  - **Terminal** — next: `alacritty_terminal` + `portable-pty`, GPUI grid render (Zed's stack).
- **Phase 4 — Dock & shell** (Linux) — layer-shell dock (hover magnification, spring physics), top bar, Spotlight.
- **Phase 5 — The *feel*** — inertial gestures (in compositor), global menu bar, Mission-Control overview, packaging as Ubuntu remix.

## Build setup notes

- **macOS dev requires the Metal Toolchain** (Xcode 26 split it out):
  `xcodebuild -downloadComponent MetalToolchain` (one-time, multi-GB).
- Run the monitor: `cargo run -p rmac-activity-monitor` (use `--release` for real perf numbers).
