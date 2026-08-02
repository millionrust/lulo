# Terminal application specification

This file is the product and authority contract for `rmac-terminal`. Terminal
is a trusted native application because an interactive shell necessarily has
the user's ordinary account access. It must behave like a careful desktop
terminal without pretending to provide shell state, accessibility, or process
control that the platform cannot prove. Shell state is shown only when a
bounded protocol report supplies it; ordinary output is never scraped.

## Core journeys

1. Open a window into the configured login shell, type Unicode and control
   input, run interactive and full-screen programs, and see exact terminal
   output without an idle redraw loop.
2. Resize or scale the window and propagate the exact bounded row/column
   geometry to both the emulator and PTY.
3. Scroll through bounded history; select forward or backward across hard and
   soft-wrapped lines; copy, paste, clear, and search without altering output.
4. Create, select, reorder later, and close independent tabs. Each tab owns one
   PTY, parser, child lifecycle, scrollback, title, reported working directory,
   selection, and close state. A new tab inherits a still-live local reported
   directory, never a remote or stale path.
5. Observe successful, failed, and signalled shell exit truthfully. Never leave
   an exited tab looking like a live prompt.
6. Close an idle shell directly. When the PTY proves another foreground process
   group is active, review an exact tab/window close warning and explicitly
   cancel or terminate it. Unknown process state fails closed.
7. Choose a color profile and persist its stable name through private,
   atomic storage without making terminal output part of preferences or logs.

Multiplexing/server mode, remote-session management, and proprietary shell
services are not required for 1.0.

## Current implementation

The running app has a real dynamically resized PTY, Alacritty/VTE grid,
scrollback, forward/backward mouse selection, soft-wrap-aware copy, bounded
paste, search highlighting, zoom, clear, independent shell tabs, nine profiles,
and event-coalesced rendering. The shell child is now retained and reaped by one
blocking waiter per session; output and exit share the bounded wake channel, so
quiet sessions do not poll. Startup, wait, write, and termination failures are
private-safe typed states. An exited/unavailable tab is labeled, its live cursor
is hidden, input is refused, and its existing output stays readable.

Command-W and window close now bind a stable session identity. On Unix, a
kernel-reported foreground process group different from the retained shell PID
opens a destructive review; unknown group/PID state also requires review.
Confirmation sends SIGHUP to the exact positive foreground group and then the
shell, while cancellation changes no process state. The final tab follows the
same guarded window path. Clipboard writes larger than 1 MiB are refused before
the PTY. One window is capped at 16 tabs; every emulator grid is bounded to
20–500 columns and 5–300 rows. One or two tabs can each retain the full
10,000-line history. At every higher supported tab count, Terminal derives and
applies one smaller per-tab limit before creating the next session so the
window's primary/alternate base cell grids remain within a conservative
512 MiB ceiling. The accounting uses the compiled Cell/Row sizes, maximum
geometry, two retained screen grids, worst-case amortized row capacity, and
per-row allocator size-class allowance, plus retained/rounded outer Row storage.
The tab bar reports the active history cap; closing tabs raises future capacity
but cannot restore history already trimmed. A session-state failure aborts tab
creation rather than bypassing the budget.
Paste now follows the active emulator mode: xterm private mode 2004 wraps exact
clipboard bytes with `ESC[200~` / `ESC[201~`, strips embedded Escape and ETX so
the payload cannot end the bracket early, and preserves multiline Unicode. If
the active program did not enable bracketed paste, platform line boundaries
become Return, non-text controls are refused, and multiline content pauses in a
content-redacted stable-session review that shows only checked line/byte counts.
Cancellation drops the private payload, while confirmation rechecks the bound
live session and current mode before writing.
Traditional keyboard input now reads the parsed xterm application-cursor mode:
arrows plus Home/End switch between CSI and SS3, editing keys and F1–F20 use
their conventional sequences, and Shift/Alt/Control combinations use xterm's
one-based modifier parameter. Alt prefixes ordinary/control text with Escape,
Ctrl-letter and conventional Ctrl-punctuation mappings are exact, Shift-Tab
sends backtab, platform shortcuts never leak to the PTY, and terminal-state
lock failure sends nothing. Successful input returns to live output and clears
only the current visual selection. Applications can now negotiate Kitty's
bounded progressive mode stack and query the current flags. Disambiguated,
all-key, associated-text, and truthfully available alternate-key forms use the
parsed mode; GPUI's held and key-up events provide explicit repeat/release
types. Enter, Tab, and Backspace releases remain suppressed unless all-key mode
requests them. F13–F20 use Kitty's private key codes in enhanced mode, while
ordinary mode retains terminfo-compatible sequences. GPUI does not expose
physical key location, so numeric-keypad identity remains deliberately
unadvertised instead of being guessed.
GPUI's platform text-input handler now owns both ordinary Unicode keystrokes
and IME composition. Marked text is held in a private 16 KiB UTF-8 buffer,
underlined at the live cursor, and exposed to the platform in UTF-16 scalar-safe
ranges with Unicode cell-aware candidate bounds. Preedit changes never reach
the PTY. Commit or platform unmark writes the final text exactly once to the
stable live session that began composition; cancellation writes nothing.
When all-key plus associated-text reporting is active, a multi-scalar native
IME commit uses Kitty's zero-key form with the exact non-control Unicode
codepoints; without that explicit request the committed UTF-8 remains direct.
Switching tabs leaves a composition bound to its original session, and a later
stale update or commit is visibly refused rather than injected into another
shell. Exited sessions, open confirmations, oversized input, invalid UTF-16
ranges, and control-bearing input all fail closed. The preedit is memory-only
and is not copied into errors, persistence, or logs.
PTY output now crosses a streaming OSC boundary before VTE: an allowed title/
palette/control payload is capped at 1 KiB and buffered only until BEL or
`ESC \`, then delivered byte-for-byte. Split sequences remain exact. Overlong,
malformed, and unterminated OSC is discarded as a unit, including the
`ESC <C0> ]` form that otherwise retains parser Escape state. Bounded OSC 8
hyperlinks now reach Alacritty's cell metadata and render underlined. Hover
shows only a privacy-safe scheme/host summary (or a generic email label);
Command-click on macOS and Super-click on Linux re-read the exact cell and open
only `http`, `https`, or `mailto` through the desktop boundary. Activation
rejects URI values above 768 bytes, controls, directional spoofing, missing
hosts, embedded web credentials, local files, and custom/executable schemes
with a visible content-redacted error. The reusable filtered chunk is capped by
the 8 KiB PTY read plus one accepted OSC.
After every non-ASCII parser completion, Terminal inspects the exact cell that
Alacritty can have modified—including the base of a wide-character spacer—and
retains at most 16 zero-width combining marks. Rebuilding an over-cap cell
preserves its base character, colors, flags, underline color, and reviewed
hyperlink metadata. This bounds the rare allocation immediately without a
full-grid scan and works when UTF-8 is split across PTY reads.
Session creation reserves exactly one named PTY reader and one named child
waiter before launching the shell. Each uses an explicit 512 KiB stack, so the
16-tab limit caps worker-stack reservation at 16 MiB per window. Thread
reservation is fallible and private-safe; a missing worker produces an
unavailable tab instead of panicking or launching an unsupervised child.
Dependency-contract tests pin the remaining parser assumptions: VTE's
synchronized-update heap buffer stops before 2 MiB, while its CSI parameters,
intermediates, and partial UTF-8 state are fixed arrays. Alacritty evicts the
oldest saved title at 4,096 entries, and Terminal's 1 KiB OSC ingress limit
bounds each title payload.
Resize is now an authority-first per-tab transaction: Terminal locks the model,
asks the kernel PTY to accept the bounded dimensions, and resizes the emulator
grid only after success. A rejection retains and renders the exact last accepted
rows and columns and remains visible on that tab until a later resize succeeds
or the window returns to the accepted geometry. A writer failure is terminal
for that tab's input path: subsequent key/paste writes are refused without
clearing selection or showing a live cursor, the tab is labeled unavailable,
and existing output remains readable.
Each stable session now owns its selection plus find-open/query state. Switching,
creating, or removing tabs saves and restores the active editor without leaking
another tab's query or discarding its selection. Queries remain memory-only and
are capped at 4 KiB on a valid UTF-8 boundary before matching; oversized editor
input is normalized on the next event-driven render. Generic numbered tab
labels remain the fallback. Each session now owns a memory-only title authority
fed only by its bounded parsed title events. It collapses whitespace, removes
control and Unicode directional-control characters, stops on a valid UTF-8
boundary at 256 bytes, and wakes GPUI only when the normalized value changes.
The active title appears in the centered toolbar and every titled tab; both use
visual ellipsis, while exited/unavailable state remains an explicit suffix.
While a live foreground process group differs from the retained shell PID, a
duplicated PTY descriptor lets the existing output reader refresh that exact
kernel identity without a timer. Linux resolves only `/proc/<group>/comm` and
macOS only `proc_name`; strict UTF-8 names are trimmed, control/directional
characters are refused, and visible output stops at 64 bytes. A valid group-
leader name temporarily precedes an application title, then disappears when the
shell regains the terminal. Missing descriptors, groups, leaders, names, or
platform support preserve the existing title/directory/number fallback. This
state is presentation only and never changes close or signal authority.
Each session also owns a memory-only OSC 7 directory authority. A valid,
complete, 768-byte-or-smaller `file:` URI supplies a bounded folder fallback
when no application title exists. Local and `localhost` reports retain the
decoded path only in that exact session; remote reports show only
`Remote — host` and can never become a local process directory. New Tab
rechecks that the reported local path still exists as a directory before
passing it directly (without a shell) to `portable-pty`; otherwise ordinary
startup-directory fallback applies. Invalid schemes, credentials,
query/fragment data, controls, directional spoofing, malformed UTF-8, partial
OSC, and overlong values leave the last trusted state unchanged.
Complete FinalTerm OSC 133 `A`/`B`/`C`/`D[;status]` reports also feed an exact
session-local, 32-byte command-phase authority. `C` gives a tab a `Running`
suffix; a nonzero `D` retains `Failed status` until the next prompt/input phase.
Zero or statusless completion has no failure badge. Invalid, control-bearing,
oversized, or out-of-range reports change nothing. This presentation context is
never trusted for close, signal, child-lifecycle, or command-text decisions and
adds no timer or output scraping. Every complete `A` report is routed at its
exact filtered-parser boundary and records only a retained-grid line coordinate,
never prompt or command text. Up to 512 exact-session marks provide Previous
Prompt (⌘↑) and Next Prompt (⌘↓) actions plus context-menu paths. Navigation
places the target at the viewport top where history permits. Resize, clear, and
history-budget changes discard coordinates; alternate-screen reports and a full
scrollback eviction boundary record or resolve nothing rather than targeting a
stale row. Strict ordered `A`→`B`→`C`→`D` sequences additionally retain at most
256 coordinate-only completed records. Select Command targets the `B`–`C`
cells; Select Command Output (⇧⌘A) targets `C`–`D`. The live viewport chooses
the newest complete command, while a scrolled viewport chooses the newest
record at or above its top line. Missing, repeated, reversed, incomplete,
alternate-screen, resized, cleared, re-budgeted, or eviction-ambiguous
coordinates produce no range. Neither range retains command/output text; copy
still reads only the explicitly selected live grid cells.
Terminal mouse input now follows the parsed xterm 1000/1002/1003 tracking mode
and 1005/1006 coordinate encoding. Press, balanced release, cell-deduplicated
drag/all-motion, vertical and horizontal wheel events use one-based viewport
cells; SGR retains release-button identity, while legacy and UTF-8 reports are
refused beyond their exact 223/2,015 coordinate limits instead of being clamped
to a false cell. Each wheel event emits at most 16 reports per axis and discards
overflow. A release outside the body still finishes the exact application drag.
Shift always takes the local selection/context-menu path, including while an
application has reporting enabled. Without mouse reporting, alternate-screen
mode 1007 translates vertical wheel steps to mode-correct cursor keys; otherwise
the wheel remains local scrollback. One-tab and multi-tab pointer coordinates
now share the rendered title/tab/content offsets.
Focus input now follows parsed xterm mode 1004. Operating-system activation and
deactivation send exact `CSI I` and `CSI O` reports only to the current live
session; selecting another tab transfers focus out of the old PTY and into the
new one while the window is active. Re-selecting the current tab, internal find
focus, and ordinary renders emit nothing. The three-byte static reports use the
same truthful writer-failure path as keyboard, paste, and mouse input, and their
successful path does not request a repaint.

The complete application claim remains blocked on numeric-keypad identity;
accessible terminal text semantics; Linux interaction/visual evidence
(including native IME proof); and measured Unicode/resident/idle/active
performance.

## Platform authorities

- `controller` owns GPUI window lifecycle, stable per-tab orchestration,
  selection/search state, close and paste review, platform input decisions,
  and the exact intents supplied to the renderer and session authorities. The
  binary root owns only module composition and application boot.
- `portable-pty` owns PTY creation, the configured shell child, master resize,
  readable output, writable input, child wait, and termination.
- On Unix, the PTY's kernel-backed foreground process-group identity is compared
  with the retained shell PID to decide whether close needs review. Missing or
  inconsistent identity is treated as potentially active, never as safe.
- `job` owns one close-on-exec duplicate PTY descriptor, output-event-driven
  foreground-group refresh, Linux `/proc/<group>/comm` and macOS `proc_name`
  lookup, strict 64-byte presentation normalization, and exact-session sharing.
  It names only a different live group leader and has no polling thread, command
  parser, process-control authority, or invented fallback.
- `alacritty_terminal` and `vte` own escape parsing, screen/scrollback state,
  cell flags, cursor position, and terminal modes.
- `keyboard` owns the traditional xterm/DEC and negotiated Kitty encoders, event
  forms, associated/alternate text projection, and the decision between encoded
  input and GPUI's direct-text/IME path. It deliberately does not invent keypad
  identity while GPUI omits physical key location.
- `session` serializes ordinary input and parser-generated protocol replies
  through one PTY writer and one permanent visible failure state, and routes
  parsed title/reset events to that exact session's title authority.
- `title` owns private-safe normalization, the 256-byte display bound, stable
  per-session sharing, and change detection.
- `working_directory` owns strict OSC 7 URI validation, the 768-byte bound,
  local-versus-remote classification, bounded presentation labels, exact
  session sharing, and live-directory revalidation. The session alone may pass
  a validated local path directly to the next PTY spawn.
- `shell_integration` owns the bounded OSC 133 marker grammar, exact-session
  command phase, private 512-coordinate prompt-mark history, navigation policy,
  private 256-record ordered command/output range history, range selection
  policy, and `Running`/`Failed status` presentation label. `output_filter`
  preserves every marker and parser offset from one bounded 8 KiB worker read;
  `session` alone captures the corresponding primary-grid coordinate, applies
  resolved display offsets, and projects a chosen range into the ordinary
  per-tab selection. Kernel PTY and child authorities remain the sole source of
  process-control truth.
- `hyperlink` owns the 768-byte activation bound, non-spoofing URI validation,
  web/email scheme allowlist, credential refusal, and privacy-safe destination
  preview. The pointer adapter re-reads exact Alacritty cell metadata before
  activation; `rmac-portal` owns the shell-free desktop open request.
- `mouse` owns xterm button/modifier encoding, legacy/UTF-8/SGR coordinate
  limits, drag/all-motion selection, and bounded fractional wheel conversion.
  The view retains only pointer capture, body-cell projection, and the exact
  active-session write decision.
- `ime` owns the private 16 KiB preedit buffer, scalar-safe UTF-16/UTF-8 range
  conversion, replacement, and selection validation. The view owns GPUI
  composition lifecycle, stable-session binding, modal/input eligibility, and
  final one-shot delivery.
- `paste` owns the 1 MiB ceiling, platform line counting, bracketed/unbracketed
  byte construction, unsafe-control refusal, and the content-redacted pending
  review model. The session owns current mode lookup and exact PTY delivery;
  the view owns clipboard access and confirmation lifecycle.
- `renderer` owns the complete GPUI projection: grid/style runs, cursor,
  selection/find highlighting, IME preedit, tabs, profiles, dialogs, overlays,
  status/errors, semantic action/pointer wiring, input bridge, and terminal
  profile/ANSI color mapping. The controller retains session/input policy and
  supplies only authoritative state and intents.
- `emulator` owns bounded terminal geometry, Alacritty configuration,
  per-window scrollback budgeting, filtered VTE advancement, and the exact
  per-cell combining-mark cap. Its contracts pin geometry/resource ceilings,
  parser allocation assumptions, alternate-screen history, title-stack
  eviction, and split Unicode behavior beside the authority they protect.
- `session` owns PTY/shell creation, bounded reader/waiter reservation, output
  parsing delivery, child lifecycle/reaping, foreground-process-group review,
  accepted resize geometry, permanent writer failure, paste-mode lookup and
  delivery, termination, and drop cleanup. The controller receives stable
  session identity, authoritative grid/UI state, accepted size, and typed
  operations without owning OS handles or worker lifetimes.
- `output_filter` owns the split-safe 1 KiB OSC boundary, including bounded OSC
  8 admission plus complete OSC 7 and OSC 133 extraction, before untrusted PTY
  bytes reach VTE. The reader worker owns only the reusable 8 KiB buffers,
  delivery into the emulator, and routing of extracted reports to exact-session
  authorities.
- GPUI owns window geometry, operating-system activation, internal focus,
  keyboard/IME delivery, clipboard exchange, pointer selection, and rendering.
  Only OS activation and explicit active-tab ownership reach xterm focus mode;
  movement into Terminal's own find control does not.
- `rmac-storage` owns the atomic local profile preference under
  `$XDG_CONFIG_HOME/rmac-terminal` or the documented platform fallback.
- `profiles` owns the stable built-in profile catalog, legacy numeric migration,
  persisted-name parsing, load/save boundary, and render-pass palette
  activation. The view retains only the selected profile index and picker
  interaction; it cannot invent another persistence format or palette fallback.

Terminal does not scrape shell output to infer commands, directories, or job
names. It accepts complete bounded title, OSC 7 directory, and OSC 133 phase/
status reports, plus a kernel foreground-group leader name on Linux/macOS.
Prompt navigation trusts only OSC 133 `A` positions and
stores no shell text. Command/output selection likewise trusts only complete
ordered A/B/C/D coordinates and reads text only through the existing explicit
grid selection. Persistent command text is not inferred or shown.

## Child lifecycle and close safety

- The shell child handle remains owned until one waiter observes its terminal
  status. Child exit wakes the UI through the same bounded coalescing channel as
  PTY output; there is no lifecycle polling timer.
- A tab distinguishes starting/running, successful exit, nonzero exit, signal,
  wait failure, and startup failure without retaining raw OS diagnostics in
  ordinary UI state.
- Dropping or explicitly closing a live session requests termination and closes
  its PTY handles. A reviewed window close covers every tab rather than silently
  bypassing per-tab state.
- A running shell with itself as the PTY foreground group is considered idle
  for close presentation. A different foreground group, unavailable group, or
  unavailable shell PID requires confirmation.
- Cancellation changes no process state. A failed termination remains visible
  and must not be reported as closed.

## Input, output, and privacy

- User text is written directly to the PTY writer; it is never interpolated
  into a shell command or command-line parser.
- Direct Unicode and IME commits share GPUI's platform text-input path, so text
  reaches the PTY once rather than through both key and composition events.
  Marked text stays in a private 16 KiB memory-only buffer and is sent only on
  final commit to its exact stable session; cancellation sends nothing.
- Clipboard paste is capped at 1 MiB, follows the active bracketed-paste mode,
  cannot embed its termination marker, requires review before unprotected
  multiline Return input, and never logs or renders pasted content in the
  review.
- PTY output, selection, search queries, clipboard text, process IDs, working
  directories, commands, and environment values stay out of default logs,
  panic text, evidence bundles, and persistence. Each per-tab query is limited
  to 4 KiB in memory.
- Reader/parser work stays off the GPUI thread. Output bursts coalesce into one
  pending repaint; a quiet terminal has no timer-driven CPU or frame activity.
- Primary/alternate base grid storage has a conservative 512 MiB per-window
  ceiling across every supported tab count. A cell retains at most 16 combining
  marks. OSC ingress is capped at 1 KiB, while hyperlink activation independently
  caps a validated URI at 768 bytes. One exact-session OSC 7 directory report is
  likewise capped at 768 bytes, and its OSC 133 phase marker at 32 bytes. Two
  512 KiB-stack workers per tab, VTE's sub-2 MiB
  synchronized-update buffer, fixed parser arrays, and Alacritty's 4,096-entry
  title stack have explicit tested contracts. Mouse coordinates are limited by
  their selected wire encoding, and one input event can create at most 32 wheel
  reports across both axes. Focus reports are static three-byte slices. IME
  preedit is capped at 16 KiB and every platform range is checked against exact
  UTF-16 scalar boundaries before allocation or display.

## Failure states

- PTY allocation, bounded worker reservation, shell start, reader/writer setup,
  child wait, resize, write, profile load/save, and termination each have a
  truthful unavailable or error state. Raw private paths, environment values,
  commands, and output are not exposed in those messages.
- A shell exit is not an application crash. Existing output remains readable
  and a new tab remains available.
- Writer failure permanently disables misleading live input for that tab,
  preserves readable output and local selection state, hides the live cursor,
  and exposes the tab as unavailable.
- IME commit is refused if its stable session is no longer active/live, a
  confirmation is open, its text exceeds 16 KiB, it contains control data, or
  the platform supplies a non-scalar UTF-16 range. The private preedit is then
  discarded and only a content-free explanation is shown.
- Resize failures retain and render the last kernel-accepted PTY size, expose a
  persistent per-tab explanation, and suppress repeated calls for the rejected
  geometry until the window requests a different size.
- Closing the final tab follows the same guarded window-close path.

## Keyboard map

- `Cmd-T`: new tab.
- `Cmd-W`: guarded close of the current tab, or guarded window close for the
  final tab.
- `Shift-Cmd-]` / `Shift-Cmd-[`: next / previous tab.
- `Cmd-C` / `Cmd-V`: copy selected text / paste into the active PTY.
- `Cmd-F`: show and focus search; `Escape` closes a modal or search before it is
  sent to the PTY.
- `Cmd-A`: select the complete bounded buffer.
- `Cmd-K`: clear visible output and scrollback after terminal-mode review.
- `Cmd-+`, `Cmd-=`, `Cmd--`, `Cmd-0`: zoom controls.
- `Shift-Cmd-P`: show profiles.

Terminal control sequences without the platform modifier, including arrows,
Tab, Escape, Backspace, Enter, and Ctrl-letter input, go to the PTY. Complete
traditional xterm navigation and F1–F20 sequences follow application-cursor
mode and encode Shift/Alt/Control modifiers. Ordinary and shift-modified text
uses GPUI's platform handler so direct Unicode and final IME commits have one
exact delivery path; marked text is visibly underlined at the live cursor and
is not sent early. Negotiated Kitty modes add disambiguated/all-key sequences,
associated and available shifted text, mode queries, and press/repeat/release
events. Numeric-keypad identity remains a GPUI metadata gate; native Linux IME
behavior remains an interaction-evidence gate.

## Pointer map

- Unshifted press/release and requested motion go to the active PTY only while
  its parsed 1000, 1002, or 1003 mouse mode owns them.
- Shift-drag always selects locally. Shift-right-click opens the local context
  menu even when a full-screen application has mouse reporting enabled.
- Motion is emitted only when the pointer enters another terminal cell. A
  release over or outside the body completes a successfully reported press.
- Vertical/horizontal wheel reports follow 1005/1006 when selected. In an
  alternate screen with 1007 but no mouse tracking, vertical wheel input sends
  cursor Up/Down; otherwise the wheel navigates local scrollback.

## Focus map

- Parsed mode 1004 is the sole authority for emitting focus reports.
- Window activation emits `CSI I`; deactivation emits `CSI O`.
- While the window is active, changing tabs emits focus-out to the prior live
  session and focus-in to the newly active live session. Closing an active tab
  does not write to the terminating session, but the surviving active tab gets
  focus-in.
- Find, menus, and other internal controls do not masquerade as operating-system
  window activation changes.

## Visual and accessibility states

- Toolbar, tabs, active/inactive/hover/focus states, profile picker, context
  menu, search, selection, cursor, exited session, unavailable session, and
  close confirmation use the shared rmac visual language.
- The active bounded session title is centered in the toolbar; each titled tab
  uses the same value with ellipsis and a truthful state suffix. Without a
  title, the exact session's bounded reported folder/remote-host label precedes
  the generic fallback. A valid OSC 133 phase may add `Running` or
  `Failed status`, but cannot replace lifecycle/transport failure.
- Terminal profiles are user content and may retain explicit ANSI palettes;
  application chrome follows shared light/dark/accent/contrast/motion tokens.
- At 200% scale, rows and tabs remain usable and PTY geometry matches the
  actually visible grid. Fractional scaling must not create a clipped phantom
  row or column.
- Tabs, buttons, menus, alerts, search, session state, and the terminal content
  expose meaningful roles, names, focus, state, and actions. The terminal grid
  needs a documented accessible text/caret/selection strategy compatible with
  Orca; visual cell rendering alone is not an accessibility claim.

## Acceptance evidence

Unit and contract tests cover input encoding, resize arithmetic and bounds,
selection extraction, child state transitions, foreground-job classification,
bounded/spoof-safe job-name projection, exact-session isolation, guarded close
decisions, bracketed/unbracketed paste construction and review,
embedded-marker/control rejection, redacted pending-review state, persistence
failures, and redraw coalescing. Pure paste contracts live beside that policy;
parsed bracketed-mode transitions remain an integration test.
Profile contract tests keep every stable name and legacy numeric index
loadable, reject empty/unknown/out-of-range preferences, and keep the default
fallback explicit.
Keyboard mode transitions, navigation/function modifier sequences, control/
Unicode/Meta input, platform-shortcut suppression, negotiated Kitty mode-stack
transitions, disambiguation, associated/alternate Unicode, function-key forms,
and press/repeat/release events are checked directly. A connected parser-query
test proves replies use the exact shared PTY writer; encoder-only contracts live
beside the keyboard authority while parsed mode transitions remain integration
tests.
Aggregate grid-budget tests cover every
supported tab count, verify monotonic history reduction, preserve useful crowded
history, and prove one additional line would cross the ceiling at the maximum.
Streaming-output tests cover ordinary split Unicode/CSI, split BEL/ST-terminated
OSC at the exact cap, complete overlong/malformed discard, C0 Escape-state
bypass prevention, recovery to normal output, and byte-exact bounded OSC 8
admission. A parser integration test proves link metadata ends at the closing
sequence. URI-policy tests cover private previews, scheme and credential
refusal, control/directional spoofing, missing hosts, ports, IPv6, and the exact
activation bound; portal errors prove the requested URI is never retained.
OSC 7 tests cover split completion, exact URI extraction, malformed/overlong
discard, local session isolation, remote display-only state, repeated reports,
and scheme/credential/query/control/directional refusal.
OSC 133 tests cover split extraction, ordered multi-report parser offsets,
exact-session isolation, running/failure/success projection, invalid status and
command-text refusal, bounded private prompt navigation, ordered command/output
ranges, exclusive-to-inclusive cell projection, incomplete/reversed refusal,
grid-identity clearing, eviction refusal, and lifecycle/transport label
precedence.
Combining-allocation tests cover multiple independently styled cells, exact
retention, wide-character targeting, a UTF-8 sequence split between reads, and
ASCII CSI REP amplification of a prior combining scalar.
Resource-contract tests cover the exact worker count/stack budget, VTE
synchronized-update cutoff, and Alacritty title-stack eviction depth.
Transport-state tests prove rejected resize transactions retain the last
accepted geometry and a writer failure permanently disables live input while
keeping existing output available. Worker, lifecycle, resize, foreground-job,
writer-failure, and redraw-coalescing contracts live beside the session runtime.
Per-tab state tests prove independent selection/find state and UTF-8-safe query
bounding.
Title contracts prove whitespace/control/directional normalization, exact UTF-8
bounding, clone/session isolation, change-only redraw, parsed title delivery,
and reset-to-generic fallback.
IME contract tests prove the direct-text routing split, exact UTF-16 offsets,
surrogate-boundary refusal, marked-text replacement/selection behavior, and
the inclusive 16 KiB UTF-8 ceiling beside the IME authority. Runtime routing
binds preedit and commit to
one stable live session, keeps marked text out of PTY writes, sends a final
commit once, and fails closed on stale, modal, exited, invalid, oversized, or
control-bearing input.
Mouse contract tests parse the mutually exclusive tracking/encoding modes and
prove exact SGR press/release/motion, legacy and UTF-8 boundaries, extended
buttons, drag/all-motion mode selection, fractional wheel accumulation, and the
per-event report ceiling. Runtime routing preserves Shift-local selection,
same-cell suppression, outside-release state, and input-failure
visibility without repainting ordinary application motion.
Focus contract tests parse mode 1004 directly and prove exact enable/disable and
static focus-in/focus-out bytes. Runtime routing deduplicates OS activation,
transfers active-tab ownership, refuses exited sessions, and uses the tested
permanent writer-failure state without scheduling ordinary success redraws.
The transport follows the official xterm
[keyboard](https://www.invisible-island.net/xterm/ctlseqs/ctlseqs.html#h2-Special-Keyboard-Keys)
and
[Kitty keyboard](https://sw.kovidgoyal.net/kitty/keyboard-protocol/)
and
[bracketed-paste](https://www.invisible-island.net/xterm/ctlseqs/ctlseqs.html#h2-Bracketed-Paste-Mode)
and
[mouse-tracking](https://www.invisible-island.net/xterm/ctlseqs/ctlseqs.html#h2-Mouse-Tracking)
contracts, including focus events.
Native
Ubuntu/niri evidence must additionally cover Bash and another supported shell,
`vim`/`less`/`top`, Unicode and IME, rapid output, scrollback, large paste,
process exit/signals, idle and foreground close, multiple tabs, resize at 100%/
150%/200%, clipboard, keyboard-only use, Orca, idle CPU, active frame pacing,
memory bounds, and abrupt app/compositor/session shutdown.
