# Direct niri adapter

`rmac-compositor-niri` translates niri IPC into the compositor-neutral model
owned by `rmac-compositor`. It does not render UI.

## Connection lifecycle

`watch` reads the Unix-domain socket path from `$NIRI_SOCKET` and publishes
connection state through the same domain event channel as compositor state:

1. publish `Connecting` on the first attempt or `Reconnecting` thereafter;
2. open the event-stream socket and complete its `Handled` handshake;
3. while that stream buffers events, query outputs and layer surfaces on two
   independent request sockets;
4. reduce incoming events privately until the authoritative
   `WorkspacesChanged`, `WindowsChanged`, and `OverviewOpenedOrClosed` initial
   events have arrived;
5. publish one coherent `Snapshot`, then `Connected`, then any unknown events
   observed during initialization;
6. publish translated incremental events until the socket closes;
7. publish `Disconnected` and retry with exponential backoff from 100 ms to a
   five-second maximum.

Every reconnect starts from a new empty reducer and a new complete event
stream. Consumers never need to merge a replacement compositor instance into
stale state. An event stream and request/command traffic must not share one
socket because event-stream mode changes the socket into a continuous feed.

## Typed actions

`action_capabilities` advertises the operations implemented by this adapter:
focus a window/workspace/output, close a window, move a window to a
workspace/output, and explicitly open or close overview. `execute` uses
`$NIRI_SOCKET`; `execute_at` accepts an explicit socket path for services and
deterministic tests.

Every action opens its own request socket and includes stable window,
workspace, or output identity. This avoids the documented race where separate
“focus window” and “act on focused window” requests can affect a different
window. A `Handled` reply reports acceptance; the live event stream confirms
the resulting desktop state. Niri error replies remain typed `Rejected`
results, while missing socket environment, connection I/O, and malformed
protocol responses stay distinguishable.

## Compatibility and failures

The private wire model targets the documented niri 26.4 protocol. The official
`niri-ipc` Rust crate is GPL-3.0-or-later, so the MIT workspace does not link it
or weaken its dependency policy. The adapter instead defines the small set of
serde wire messages it consumes. Protocol upgrades require a reviewed
compatibility slice.

Incoming lines are first decoded as single-key JSON envelopes. Known niri
events are then decoded into the reviewed wire types; malformed known events are
protocol failures and trigger a clean reconnect. Unknown top-level event kinds
retain their kind and payload as `rmac_compositor::Event::Unknown`, including
when they arrive before the initial snapshot. This makes newer niri additions
non-fatal without silently pretending to understand their semantics.

Overview state uses niri's typed initial/live `OverviewOpenedOrClosed` event.
The documented 26.4 `Window` record does not expose real fullscreen state, so
the adapter does not guess it from tile dimensions or focus. Fullscreen Dock
behavior remains governed by niri's layer-shell stacking and must be proven on
the reference PC unless a future reviewed IPC field provides authority.

The domain deliberately allows temporary dangling window/workspace/output
references because niri documents non-atomic ordering across collections.
Replacement events remain authoritative only for their own collection.

## Verification boundary

Unit tests use a real temporary Unix-domain listener rather than a mocked
function call. The fixture verifies the event-stream, outputs, and layers
requests use separate connections; the overview-inclusive initial snapshot
precedes connection readiness; a future event survives initialization; and a
subsequent urgency event is delivered incrementally. Additional tests cover
typed overview translation, malformed known JSON, and bounded reconnect delay.
Action contract tests assert exact wire JSON and distinguish handled,
compositor-rejected, and transport-failure results.

This is deterministic protocol evidence on the development host, not Linux
hardware evidence. The Linux reference-PC gate must still verify a real niri
restart, the installed socket environment, representative output topology, and
logs before release claims are made.

## Upstream contract

- <https://docs.rs/niri-ipc/26.4.0/niri_ipc/>
- <https://docs.rs/niri-ipc/26.4.0/niri_ipc/enum.Event.html>
- <https://docs.rs/niri-ipc/26.4.0/niri_ipc/enum.Request.html>
- <https://docs.rs/niri-ipc/26.4.0/niri_ipc/enum.Response.html>
