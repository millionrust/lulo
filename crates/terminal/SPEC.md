# Terminal application specification

This file is the product and authority contract for `rmac-terminal`. Terminal
is a trusted native application because an interactive shell necessarily has
the user's ordinary account access. It must behave like a careful desktop
terminal without pretending to provide shell state, accessibility, or process
control that the platform cannot prove.

## Core journeys

1. Open a window into the configured login shell, type Unicode and control
   input, run interactive and full-screen programs, and see exact terminal
   output without an idle redraw loop.
2. Resize or scale the window and propagate the exact bounded row/column
   geometry to both the emulator and PTY.
3. Scroll through bounded history; select forward or backward across hard and
   soft-wrapped lines; copy, paste, clear, and search without altering output.
4. Create, select, reorder later, and close independent tabs. Each tab owns one
   PTY, parser, child lifecycle, scrollback, title, selection, and close state.
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
only the current visual selection. Enhanced Kitty keyboard mode remains
disabled at the emulator authority, so the incomplete key-down-only path is
not partially advertised or activated.
PTY output now crosses a streaming OSC boundary before VTE: an allowed title/
palette/control payload is capped at 1 KiB and buffered only until BEL or
`ESC \`, then delivered byte-for-byte. Split sequences remain exact. Overlong,
malformed, and unterminated OSC is discarded as a unit, including the
`ESC <C0> ]` form that otherwise retains parser Escape state. OSC 8 hyperlink
metadata is also discarded until Terminal has a reviewed visual/activation
policy, preventing invisible per-cell link allocation. The reusable filtered
chunk is capped by the 8 KiB PTY read plus one accepted OSC.
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

The complete application claim remains blocked on IME preedit/commit,
enhanced Kitty keyboard press/repeat/release reporting, numeric-keypad identity,
terminal mouse reporting, hyperlink policy, shell integration, per-tab
selection/search/title state, accessible terminal text semantics, Linux
interaction/visual evidence, and measured Unicode/resident/idle/active
performance.

## Platform authorities

- `portable-pty` owns PTY creation, the configured shell child, master resize,
  readable output, writable input, child wait, and termination.
- On Unix, the PTY's kernel-backed foreground process-group identity is compared
  with the retained shell PID only to decide whether close needs review. Missing
  or inconsistent identity is treated as potentially active, never as safe.
- `alacritty_terminal` and `vte` own escape parsing, screen/scrollback state,
  cell flags, cursor position, and terminal modes.
- GPUI owns window geometry, focus, keyboard/IME delivery, clipboard exchange,
  pointer selection, and rendering.
- `rmac-storage` owns the atomic local profile preference under
  `$XDG_CONFIG_HOME/rmac-terminal` or the documented platform fallback.

Terminal does not scrape shell output to infer commands, working directories,
or job names. Rich tab titles and command-aware close text require a reviewed
shell-integration protocol before they may be shown.

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
- Clipboard paste is capped at 1 MiB, follows the active bracketed-paste mode,
  cannot embed its termination marker, requires review before unprotected
  multiline Return input, and never logs or renders pasted content in the
  review.
- PTY output, selection, search queries, clipboard text, process IDs, working
  directories, commands, and environment values stay out of default logs,
  panic text, evidence bundles, and persistence.
- Reader/parser work stays off the GPUI thread. Output bursts coalesce into one
  pending repaint; a quiet terminal has no timer-driven CPU or frame activity.
- Primary/alternate base grid storage has a conservative 512 MiB per-window
  ceiling across every supported tab count. A cell retains at most 16 combining
  marks. OSC ingress is capped at 1 KiB, and unreviewed hyperlink metadata never
  reaches the grid. Two 512 KiB-stack workers per tab, VTE's sub-2 MiB
  synchronized-update buffer, fixed parser arrays, and Alacritty's 4,096-entry
  title stack have explicit tested contracts.

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
mode and encode Shift/Alt/Control modifiers. Enhanced Kitty keyboard reporting,
numeric-keypad identity, and complete IME preedit/commit behavior remain Linux
framework acceptance gates.

## Visual and accessibility states

- Toolbar, tabs, active/inactive/hover/focus states, profile picker, context
  menu, search, selection, cursor, exited session, unavailable session, and
  close confirmation use the shared rmac visual language.
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
guarded close decisions, bracketed/unbracketed paste construction and review,
embedded-marker/control rejection, persistence failures, and redraw coalescing.
Keyboard mode transitions, navigation/function modifier sequences, control/
Unicode/Meta input, platform-shortcut suppression, and enhanced-mode
non-advertisement are checked directly. Aggregate grid-budget tests cover every
supported tab count, verify monotonic history reduction, preserve useful crowded
history, and prove one additional line would cross the ceiling at the maximum.
Streaming-output tests cover ordinary split Unicode/CSI, split BEL/ST-terminated
OSC at the exact cap, complete overlong/malformed discard, C0 Escape-state
bypass prevention, recovery to normal output, and OSC 8 refusal.
Combining-allocation tests cover multiple independently styled cells, exact
retention, wide-character targeting, a UTF-8 sequence split between reads, and
ASCII CSI REP amplification of a prior combining scalar.
Resource-contract tests cover the exact worker count/stack budget, VTE
synchronized-update cutoff, and Alacritty title-stack eviction depth.
Transport-state tests prove rejected resize transactions retain the last
accepted geometry and a writer failure permanently disables live input while
keeping existing output available.
The transport follows the official xterm
[keyboard](https://www.invisible-island.net/xterm/ctlseqs/ctlseqs.html#h2-Special-Keyboard-Keys)
and
[bracketed-paste](https://www.invisible-island.net/xterm/ctlseqs/ctlseqs.html#h2-Bracketed-Paste-Mode)
contracts.
Native
Ubuntu/niri evidence must additionally cover Bash and another supported shell,
`vim`/`less`/`top`, Unicode and IME, rapid output, scrollback, large paste,
process exit/signals, idle and foreground close, multiple tabs, resize at 100%/
150%/200%, clipboard, keyboard-only use, Orca, idle CPU, active frame pacing,
memory bounds, and abrupt app/compositor/session shutdown.
