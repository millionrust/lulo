# ADR 0008 — How rmac gets the best Mac feel on Ubuntu (the window-layer strategy)

- **Status:** proposed 2026-09-19. Supersedes decision 3 of ADR 0007.
- **Framing:** written as an owner's decision, not a menu. One route is chosen; the others are
  recorded with the reason they lost.

## The question

What is the fastest route to a desktop that genuinely feels like a Mac on Ubuntu, for one developer
with an AI agent, no income, and a need for a credible public release?

## What the evidence says

1. **The feel is mostly not compositor work.** Of everything a Mac user notices in the first minute
   — sound, pointer, motion continuity, scrolling physics, typography, measured colours, wallpaper
   tinting, native-looking dialogs, wording — **none** is blocked by niri. All of it is rmac's own
   code, specified in `FEEL_SPEC.md` and ordered in `AGENT_PLAYBOOK.md`. Estimated: **~3 weeks**.
2. **Exactly ten behaviours are compositor-owned** (listed in `RMAC_COMP_PLAN.md` §1). The three
   that matter daily are **real minimize**, **window animations anchored to Dock tiles**, and
   **rmac's own Mission Control**.
3. **niri will not supply them soon.** Minimize is an open discussion
   ([niri #3982](https://github.com/niri-wm/niri/discussions/3982), May 2026), unimplemented, with
   the maintainer leaning toward a "super-charged scratchpad with hooks for third-party apps"
   instead. That shape — hide/restore plus IPC hooks — is exactly what rmac needs, which makes it a
   good upstream contribution target rather than a blocker.
4. **niri is GPL-3.0; rmac is MIT; Smithay and Wayfire are MIT.** rmac talks to the compositor over
   **IPC**, so a GPL compositor never touches rmac's licence. A *fork* of niri shipped as its own
   executable is legally clean: publish the fork under GPL-3.0, keep every rmac crate MIT, and never
   copy niri code into this tree.
5. **Writing a compositor from scratch is 5–6 months** (`RMAC_COMP_PLAN.md`). Patching a compositor
   that already does 90% of the job is weeks.

## Decision: three tracks, in this order

### Track A — Feel (weeks 1–3, starts today, highest priority)
`AGENT_PLAYBOOK.md` tasks 1–12: the visual loop, measured colours, wallpaper tinting, fonts, the
sound set, the cursor, motion continuity, scrolling physics, the owner's real defaults, the three
corrected layouts, the rmac file picker and polkit dialog, the wording pass.

**This is 80% of the perceived Mac feeling and none of it needs a different compositor.**
Do not start Track B before Track A is visibly done, judged by side-by-side pairs.

### Track B — `rmac-wm`: a thin niri fork, upstream-first (weeks 4–10)
Not a new compositor. niri is already Rust, already Smithay, already does blur, layer-shell,
session-lock, screencopy, gestures and hotplug. It needs **three patches**:

| Patch | What | Upstream first? |
|---|---|---|
| **B1 — Hide/restore with IPC** | A real hidden state (not a parked workspace): `HideWindow`, `RestoreWindow`, `hidden: bool` in the window event stream, excluded from overview, switcher and workspace counts | **Yes** — matches the scratchpad direction in #3982 |
| **B2 — Animation hooks** | Let an IPC client supply a target rect and duration for open/close/hide/restore, so a window can scale out of and into its Dock tile | **Propose it**; expect a fork |
| **B3 — Overview control** | Either suppress niri's overview so rmac draws its own, or restyle it to the measured layout (spread windows, app-name labels, "Desktop" pill with +) | **Fork** — it is rmac-specific styling |

Rules: send B1 and B2 upstream as PRs before forking; if rejected, maintain `rmac-wm` as a separate
**GPL-3.0** package that tracks niri releases. Keep the patch set small enough to rebase in a day.
rmac's adapter keeps speaking the same IPC, so the shell and apps need **zero** changes.

**Outcome:** real minimize with a genie/scale effect landing on the Dock tile, tile-anchored window
animations, and rmac's own Mission Control — the three daily gaps closed for roughly **6 weeks** of
work instead of six months.

### Track C — `rmac-comp` on Smithay (after 1.0, ideally after sponsorship)
`RMAC_COMP_PLAN.md` unchanged: full ownership, per-Space wallpaper, Split View, Dock magnification
affecting layout, frame-perfect shell coordination. Start it when rmac has users, money, or both.
Track B's IPC work is not wasted — `rmac-comp` implements the same interface and passes the same
conformance suite.

## Why the alternatives lost

| Alternative | Why not |
|---|---|
| **Switch to Wayfire** (MIT, has blur/expo/scale/minimize) | Trades a Rust product for a C++ plugin tail, slower release cadence (0.10.0, Aug 2025), a different window model, and a new adapter — all to reach roughly where a three-patch niri fork lands |
| **Switch to KWin** | Most complete effects, but rmac becomes KDE-shaped, inherits heavy dependencies, and the packaging story on Ubuntu gets worse |
| **Hyprland** | No true minimize either; no advantage over niri |
| **Write a compositor now** | Five to six months with no visible product improvement, while the actual feel gaps stay unfixed. Correct destination, wrong moment |
| **Stay pure-niri forever** | Accepts a permanently fake minimize and someone else's Mission Control in the product's most-used interactions |

## What this buys, in business terms

- **Week 3:** a desktop that *sounds*, *moves* and *reads* like a Mac. This is what a demo video
  shows, and what makes a viewer say "how is this Linux?" — sponsorship is won here, not in the
  window manager.
- **Week 10:** the three daily interactions that betray the illusion are gone.
- **1.0 on Ubuntu 26.04** with an honest support statement, a signed repository, and a fork that is
  small enough for one person to maintain.
- **Post-1.0:** Track C becomes the roadmap item that a sponsored project builds — and a credible
  reason to sponsor.

## Checkpoints (stop-and-reassess, not blind execution)

1. **End of Track A:** if the side-by-side pairs are not close on the surfaces that do not depend on
   the compositor, fix that before touching Track B. Feel gaps are never solved by window managers.
2. **B1 upstream verdict:** if niri accepts hide/restore, skip the fork for that patch entirely.
3. **After B3:** if the fork's rebase cost exceeds one day per niri release, pull Track C forward.

## Licence rules (absolute)

- rmac crates stay **MIT**. `rmac-wm` is a **separate GPL-3.0 package** with its own source release.
- Never copy niri, cosmic-comp or KWin code into `crates/` or `shell/`.
- The IPC boundary is the licence boundary. Keep it clean; ADR 0007 task 2.13 enforces it.
