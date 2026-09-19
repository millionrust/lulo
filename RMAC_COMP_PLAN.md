# rmac-comp — building rmac's own compositor, the right way

> **Why this file exists:** niri can host rmac, but it caps how Mac-like the *window layer* can ever
> feel. This is the plan to remove that cap without killing the project on the way.
> **Written 2026-09-19.** Companion to `docs/decisions/0007-compositor-choice.md`,
> `AGENT_PLAYBOOK.md`, `FEEL_SPEC.md`.

## 1. What "perfect" actually means here

Only these behaviours are compositor-owned. Everything else that makes rmac feel like a Mac —
sound, cursor, colours, tinting, typography, scrolling, dialogs, wording — is rmac's own code and is
**not** blocked by niri. Be precise about the list, or you will rewrite a compositor for nothing:

| Behaviour | Why niri cannot give it |
|---|---|
| **Real minimize** | niri has no minimize concept; rmac parks windows on a hidden workspace |
| **Genie / Scale minimize effect** | requires owning the window's geometry per frame |
| **Window opens by scaling out of its Dock tile** | same |
| **rmac's own Mission Control** | niri owns the overview's look and input |
| **Per-Space wallpaper, wallpaper moving with the Space** | not supported |
| **Dock magnification interacting with window layout** (windows avoiding the grown shelf) | needs layout control |
| **Exact window shadows, corner radii and inactive states as one system** | niri rules approximate it |
| **Split View as a real window pairing** | not a niri concept |
| **Rubber-band at the edge of a Space, continuous gesture-driven Space switching** | partial |
| **Frame-perfect coordination** (menu bar hiding *with* a window entering full screen) | two processes guessing |

That list is the entire payoff. It is large enough to justify the work — after 1.0, not before.

## 2. Ground rules (non-negotiable)

1. **Licence:** rmac is MIT. **Smithay is MIT** — safe to depend on. **niri is GPL-3.0** and
   cosmic-comp is GPL-3.0 — you may read their docs and observe behaviour, but **never copy code,
   shaders or structure** from them into this tree. When in doubt, implement from the protocol spec.
2. **niri keeps working the whole time.** `rmac-comp` lands behind the `Compositor` trait
   (ADR 0007 task 2.13) as a second backend, selected by a session entry. Two session files:
   "rmac" (niri, supported) and "rmac (Preview)" (rmac-comp) until the conformance suite passes.
3. **No milestone may take the desktop backwards.** Each one ends with the full session running on
   `rmac-comp`, or it is not finished.
4. **Feel work continues in parallel and takes priority.** `AGENT_PLAYBOOK.md` tasks 1–12 ship
   first. They are ~3 weeks and worth more visible quality than 6 months of compositor work.

## 3. What Smithay gives you, and what you write

**Smithay provides** ([smithay.github.io](https://smithay.github.io/), [docs.rs](https://docs.rs/smithay)):
session/seat handling (libseat), DRM/KMS output management, GBM/EGL/Vulkan buffers, a GLES renderer
with damage tracking, libinput plumbing, the core Wayland protocol implementations, many extension
protocols including the wlroots and KDE families, and XWayland management helpers. niri itself is
built on Smithay, which is the proof that a single developer can reach a usable compositor with it.

**You write:** the window model and layout policy, the animation system, effects (blur, shadows,
rounded corners, genie), the overview, gestures, workspace/Space semantics, per-output wallpaper
integration, the IPC that `rmac-compositor` speaks, and the policy glue for lock/idle/power.

## 4. Milestones

Each milestone is a shippable state with an acceptance test. Estimates assume you plus an AI agent,
working the way you have been (roughly 150–250 commits a month), **part-time alongside the rest of
rmac**.

### M0 — Boundary and conformance (2 weeks) — *do this even if you never build the compositor*
- ADR 0007 tasks 2.13–2.16: `Compositor` trait, capability flags, no `rmac_compositor_niri` outside
  the adapter, no "niri" in user-visible strings.
- **Backend conformance suite**: one test module every backend must pass — focus, close, move to
  workspace/output, fullscreen, minimize/park, output hotplug, event-stream reconnect, and a headless
  smoke test.
- **Accept:** niri backend passes the suite; `grep` gates are green in CI.

### M1 — It draws (3–4 weeks)
- New workspace member `comp/` (separate Cargo workspace, like `shell/`): winit backend for
  development, udev/DRM backend for real hardware, GLES renderer, damage tracking.
- xdg-shell: map, configure, commit, close. Floating layout only. Pointer, keyboard, xkb.
- **Accept:** launch `rmac-comp` nested in a niri session, run `foot` and Files, move and resize
  windows, type into them, close them.

### M2 — It hosts the rmac shell (3 weeks)
- `wlr-layer-shell` with all four layers, anchors, exclusive zones, keyboard interactivity modes.
- Output management, hotplug, scale, per-output logical geometry.
- **Accept:** rmac's wallpaper, menu bar, Dock and OSD run on `rmac-comp` exactly as on niri.

### M3 — It replaces niri for daily use (4–5 weeks)
- Implement the `rmac-compositor` IPC: the full `ActionKind` set plus the event stream.
- Spaces (workspaces), window→Space assignment, multi-output policy, focus rules, app switching.
- `ext-session-lock`, `ext-idle-notify`, `xdg-activation`, `wlr-screencopy`, `xdg-output`,
  primary selection, data-device drag and drop.
- XWayland via Smithay's helpers.
- **Accept:** the conformance suite passes on `rmac-comp`; `FEEL_SPEC.md` §E steps 1–20 pass; you
  daily-drive it for a week.

### M4 — The effects that justify all of this (4 weeks)
- Rounded corners, per-window shadows keyed to focus, wallpaper-tinted materials, real blur behind
  layer surfaces (your own shader; do not port anyone else's).
- **Real minimize**, with the Scale effect landing exactly on the Dock tile, and window open
  scaling out of that tile.
- Window open/close/move/resize animations driven by rmac's motion tokens, not config guesses.
- **Accept:** frame-by-frame recordings match `FEEL_SPEC.md` §D.3 within ±30 ms; ≥ 99% of frames
  inside the refresh interval on the reference PC.

### M5 — rmac's Mission Control and gestures (3 weeks)
- Your own overview: the measured layout (windows spread without overlap, app-name labels, the
  "Desktop" pill with **+**), live thumbnails, keyboard selection, drag between Spaces.
- Continuous three- and four-finger gestures that track the finger and cancel below 40%.
- **Accept:** side-by-side with the macOS capture; gestures feel continuous, never stepped.

### M6 — Hardening (3 weeks)
- Multi-GPU, suspend/resume, hotplug storms, VT switching, crash recovery (compositor restart
  without losing the session where possible), 8-hour soak, memory and FD stability.
- **Accept:** `scripts/chaos-soak.json` drills pass on `rmac-comp`.

### M7 — Promotion (1 week)
- `rmac-comp` becomes the default session; niri stays selectable as "rmac (niri)" for one release,
  then moves to a compatibility note.
- **Accept:** `FEEL_SPEC.md` §E all 35 steps pass; `docs/hardware-support.md` updated honestly.

**Total: roughly 5–6 months part-time**, on top of 1.0. M0 alone is worth doing this month.

## 5. Risks and the honest counter-arguments

| Risk | Mitigation |
|---|---|
| Six months where the product does not visibly improve | Feel work (playbook 1–12) runs first and continues; compositor work is the background track |
| Hardware bugs are brutal solo (NVIDIA, multi-GPU, suspend) | Keep the niri session installable and supported; narrow the hardware claims in `docs/hardware-support.md` |
| You need income before this finishes | 1.0 on niri is the sponsorship artifact; `rmac-comp` is what a sponsored project builds next |
| Scope creep into a general-purpose compositor | `rmac-comp` serves rmac only: no config language, no plugin API, no other shells |
| Accidental GPL contamination | Rule 1 above; implement from protocol specs; note the source of every algorithm in commit messages |

## 6. What to do this week

1. **`AGENT_PLAYBOOK.md` Task 1** — the visual loop. One hour. Nothing else matters until you can
   see rmac beside macOS.
2. **M0** — seal the boundary and write the conformance suite. Two weeks, and it is the thing that
   makes `rmac-comp` a finite job instead of a leap of faith.
3. Keep shipping playbook tasks 2–12. When 1.0 is out and the feel work is done, start M1.

Perfect is reachable. It is just ordered: **see it → feel it → own the window layer.**
