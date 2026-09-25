# ADR 0007 — Compositor choice: niri now, an rmac compositor later

- **Status:** proposed, 2026-09-19
- **Supersedes nothing.** Complements ADR 0001/0002 (framework) and 0004 (lock boundary).

## Question

Is niri the only way to implement rmac, and is it the right host for the macOS experience?

## What the code says today

rmac already abstracts the compositor:

- `crates/rmac-compositor` — the portable model: outputs, workspaces, windows, `ActionKind`
  (Spawn, Focus*, Close, Move*, SetOverview, plus the minimize/parking work), events, state.
  ~42 public items.
- `crates/rmac-compositor-niri` — the only backend: niri IPC transport, wire format, translation.
  ~1.2 k lines.

So a second backend is a **new adapter crate plus session packaging**, not a rewrite. The shell
binaries and the six apps do not care which compositor runs underneath — *in principle*.

**In practice the abstraction leaks**, and this must be fixed before any backend swap is realistic:

1. `rmac-shell-runtime`, `rmac-wallpaper-runtime` and `rmac-dock-system` call
   `rmac_compositor_niri::{watch,execute}` **directly** instead of going through a trait object or
   a selected backend.
2. **262 non-comment references** to niri exist outside the adapter crate, including
   **user-visible strings** ("niri compositor events are disconnected", "Choose original or local
   images for every niri display"). A user must never read the word "niri" — that breaks both the
   illusion and the wording rules in `FEEL_SPEC.md` §D.10.

## What niri gives rmac

- Wayland, actively developed, Rust, IPC that is easy to drive.
- **Blur since 26.04** via the `ext-background-effect` protocol — this is what makes rmac's glass
  materials possible at all, and it landed at the right time
  ([Phoronix](https://www.phoronix.com/news/Niri-26.04-Released),
  [release notes](https://github.com/niri-wm/niri/releases/tag/v26.04)).
- Layer-shell, session-lock, screencopy, per-window rules (radius, shadow, floating), configurable
  open/close/workspace animations, an overview, gestures, good multi-monitor handling.

## What niri costs rmac (the feel ceiling)

| macOS behaviour | niri reality | rmac's workaround today |
|---|---|---|
| Minimize to the Dock | **niri has no minimize** | windows are parked on a hidden `rmac-parking` workspace; the Dock fakes the tile |
| Genie / Scale minimize animation | not expressible — rmac does not own the window's frames | none; only a Dock-side thumbnail |
| Mission Control's exact look | niri's own overview, styled by niri config | rmac presents niri's overview and cannot restyle it into the measured "Desktop pill + spread windows" layout |
| Window open/close choreography from the Dock tile | niri animates; rmac cannot drive per-window transforms | approximate config curves |
| Spaces semantics | niri workspaces are a scrolling column model | mapped, with the parking workspace needing to stay hidden everywhere |
| Wallpaper per Space, wallpaper moving with the Space switch | not supported | omitted |

None of this blocks 1.0, but it caps how close the *window layer* can get. Notably, **most of the
missing "Mac feeling" is not compositor-bound**: sound, cursor, fonts, measured colours, wallpaper
tinting, scrolling physics, foreign dialogs and wording are all rmac's own code
(`AGENT_PLAYBOOK.md` tasks 1–12).

## The alternatives, honestly

| Option | Minimize | Restyle overview | Per-window animation control | Blur | Effort to adopt | Verdict |
|---|---|---|---|---|---|---|
| **niri** (today) | no (parking hack) | no | config-level only | yes (26.04) | zero | Ship 1.0 on it |
| **Wayfire** (wlroots, C++) | yes, with effects (expo/scale, animate plugins) | partly, via plugins | plugin-level | yes | new adapter + plugins in C++; smaller community | Possible, but a C++ plugin tail inside a Rust product |
| **KWin standalone** (C++, KDE) | yes, incl. a genie-style effect and an Overview effect | via effects/QML | effect-level | yes | new adapter + KWin scripts/effects; heavy dependency; excellent maturity | Strong on features, weak on fit: rmac becomes a KDE-shaped product |
| **Hyprland** (C++) | no true minimize (special-workspace trick) | plugin | good animation control | yes | new adapter + plugins | No better than niri for the specific gaps |
| **Mutter/GNOME** | yes | no | no | limited | heavy, GNOME-shaped | No |
| **Own compositor on Smithay** (Rust) | **yes, exactly as designed** | **yes** | **yes — genie, scale-from-tile, Dock magnification interplay, per-Space wallpaper** | yes, yours | 6–12 months for one person, then permanent ownership | **The real destination, after 1.0** |

## Decision

1. **Ship 1.0 on niri.** Everything already targets it, blur landed, and the remaining feel gaps are
   overwhelmingly rmac's own code, not the compositor's.
2. **Do the portability cleanup now, while it is cheap** (see tasks below). It costs a few days and
   it is what makes options 3 and 4 possible at all.
3. **Treat `rmac-comp` (Smithay, Rust, in this workspace) as the post-1.0 path** for the behaviours
   niri structurally cannot give: real minimize with a genie or scale effect, rmac's own Mission
   Control, window animations anchored to Dock tiles, per-Space wallpaper.
4. **Do not evaluate KWin or Wayfire further** unless a 1.0 blocker appears that only they solve.
   Adopting either trades a Rust product for a C++ plugin tail without reaching the ceiling that
   option 4 reaches.

## Tasks this creates (add to `COMPLETION_SPEC.md` Phase 2)

- [ ] **2.13 Seal the compositor boundary.** Introduce `trait Compositor` in `rmac-compositor`
  (watch + execute + capabilities), make `rmac-compositor-niri` implement it, and route
  `rmac-shell-runtime`, `rmac-wallpaper-runtime` and `rmac-dock-system` through the trait. No crate
  outside the adapter may name `rmac_compositor_niri`.
  Verify: `grep -rl rmac_compositor_niri --include=*.rs crates shell | grep -v compositor-niri` is empty.
- [ ] **2.14 Remove "niri" from every user-visible string** (errors, Settings copy, notifications).
  Say "the window server" or name nothing. Add it to the string test from `FEEL_SPEC.md` §D.10.
  Verify: `grep -rn '"[^"]*niri' --include=*.rs crates shell` returns only comments and tests.
- [ ] **2.15 Capability flags.** `Compositor::capabilities()` reports `native_minimize`,
  `custom_overview`, `window_animation_control`, `per_space_wallpaper`. The shell asks instead of
  assuming, so a second backend lights up better behaviour without touching shell code.
- [ ] **2.16 Backend conformance suite.** One test module that any backend must pass (focus, close,
  move to workspace/output, fullscreen, parking/minimize, output hotplug, event stream reconnect).
  This is what makes writing `rmac-comp` a finite, verifiable job instead of a rewrite.

## Consequences

- 1.0 keeps its schedule and its honest support statement: "rmac runs on niri".
- The Dock's minimize remains a parking hack until `rmac-comp`; document it in
  `docs/known-limitations.md` rather than hiding it. Amended by ADR 0021: Lulo's niri carries one
  patch that reports third-party minimize requests, so the parking covers every app.
- After the cleanup, adding a backend is bounded work that one person can start without stopping the
  product.
