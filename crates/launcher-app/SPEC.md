# Launcher product contract

## Purpose

Launcher is the session-owned, on-demand search surface for opening installed
applications, settings, files, and calculator results. It should feel as direct
as macOS Spotlight while remaining a truthful client of Linux authorities. It
is not an application catalog, filesystem index, settings store, clipboard
owner, or second compositor.

## Primary journeys

1. Invoke Launcher and focus the query before any provider work can delay
   typing. Repeating the shortcut dismisses the same overlay.
2. With an empty query, browse the application grid before Suggestions. As
   progressive providers finish, keep a still-live user selection stable while
   arrow movement continues in visual reading order.
3. Type a query and receive bounded, ranked application, setting, calculator,
   file, and admitted extension results grouped by contiguous category.
4. Keep query focus while Up/Down wraps the active result. Return executes its
   exact primary action; Ctrl/Command-Return executes only a declared alternate
   action and never falls back to the primary action.
5. Preserve useful results while providers remain pending or degraded. Show
   loading, empty, unavailable, activating, and failed phases truthfully.
6. Close with Escape or outside focus loss, cancel stale provider generations,
   and restore only the still-live seat-bound compositor target captured at
   invocation.

## Authorities and privacy

- `rmac-launcher` owns query generations, ranking, category/result limits,
  stable selection, action privacy admission, and cancellation.
- `rmac-launcher-runtime` owns the immutable provider registry, concurrent
  dispatch, live catalog/settings resampling, keyboard effects, activation
  identity, renderer snapshot, and accessibility projection.
- `rmac-launcher-system` alone executes admitted launch, reveal, Settings,
  file, and clipboard actions. Renderers never reconstruct an action from a
  title, category, path, or result index.
- The runtime snapshot carries display text and stable controller identity but
  never an action payload. Query, result IDs, titles, subtitles, icon paths,
  announcements, and surface errors are redacted from its custom diagnostics.
- File providers run only after explicit private-content admission; network
  providers require their separate policy. A rejected or stale provider batch
  cannot mutate visible results or execute an action.

## Accessibility and interaction contract

- A public renderer-neutral accessibility boundary consumes only the exact
  runtime `Snapshot` plus surface settings health. The query is one named
  combobox/search field controlling the result collection; it owns initial and
  continuing focus while its active-descendant ID follows the selected result.
- Empty-query semantics expose Applications grid tiles followed by Suggestions
  list rows. Nonempty-query semantics reproduce every contiguous visual
  category section. Results expose name, optional description, category,
  selected state, global and section positions, layout, and exact primary and
  optional alternate action labels.
- During activation, query and result actions are disabled and only the exact
  primary or alternate action is busy. Escape remains authoritative. Loading,
  selection, empty, and opening feedback is polite; unavailable, degraded,
  activation-failed, and settings failures are assertive.
- The projection accepts at most 40 results, 64 aggregate pending/failed
  providers, 64 KiB query text, 4 KiB titles/action labels, 16 KiB subtitles or
  status text, and 1 MiB aggregate semantic text. Closed, contradictory,
  duplicate, misordered, malformed, or oversized snapshots fail closed.
- Shared constants keep the visible placeholder, sections, result collection,
  scope description, phase labels, and keyboard help equal to their semantic
  equivalents. Pinned GPUI cannot yet publish this defined tree to AT-SPI.

## Visual contract

- A centered translucent surface uses one 76-pixel search header, scrollable
  results region, and compact status/help footer with shared semantic theme
  tokens, live appearance, text scale, contrast, and reduced motion.
- Empty-query applications use original catalog icons or category fallbacks in
  a compact grid; Suggestions and search results use readable list rows with
  title, optional subtitle, category, selection fill, and alternate affordance.
- No provider failure may replace retained results with a fabricated empty
  state. Activation failure keeps the selection and query available for retry.

## Remaining release evidence

- Replace the temporary popup with the A5-approved layer-shell host, route the
  exact surface plan/session, and execute seat-aware restoration without a
  global-focus fallback.
- Export the defined semantics through A5/A6 and prove query focus,
  active-descendant movement, categories, action names, busy/error states, and
  announcements with Orca.
- Prove real portal shortcut consent/restart/fallback, provider cancellation,
  private-scope changes, clipboard and Settings routing, 100/125/150/200%
  scaling, 60/120 Hz behavior, launch latency, and idle cost on Ubuntu/niri.
