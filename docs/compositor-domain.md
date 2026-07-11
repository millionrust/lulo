# Compositor domain

`rmac-compositor` is the GPUI-free source of truth for compositor state. Shell
surfaces consume its snapshots and events; compositor adapters translate their
wire protocols into it. The crate does not connect to sockets, render UI, or
expose niri/Wayland types.

## Model

- `OutputId` is a stable compositor output name, never a presentation index.
- `WorkspaceId` and `WindowId` are stable identities. Workspace `index` is only
  its current position on one output.
- logical positions and sizes preserve fractional values; physical output modes
  and underlying window sizes remain integer-valued.
- workspaces can temporarily reference absent outputs, and windows can
  temporarily reference absent workspaces. This is valid event ordering, not
  corruption.
- layer surfaces have adapter-assigned identities because the source protocol
  may expose only namespace/output/layer/interactivity.
- focus is explicit across output, workspace, window, and layer-surface targets.
  Activation has its own request identity and pending/confirmed/rejected state.
- urgency is independently addressable for workspaces and windows.
- `Event::Unknown` retains a source kind and JSON payload, increments a bounded
  diagnostic counter, and never causes visible state churn.

The reducer reports whether an event affects visible state, topology, focus, or
urgency. Repeated urgency/focus values are idempotent. Replacement events are
authoritative for their own collection but do not guess about another
collection that may be updated by the next event.

## Source contract

The model was checked against the official niri IPC 26.4 documentation:

- <https://docs.rs/niri-ipc/latest/niri_ipc/>
- <https://docs.rs/niri-ipc/latest/niri_ipc/enum.Event.html>
- <https://docs.rs/niri-ipc/latest/niri_ipc/struct.Output.html>
- <https://docs.rs/niri-ipc/latest/niri_ipc/struct.Workspace.html>
- <https://docs.rs/niri-ipc/latest/niri_ipc/struct.WindowLayout.html>
- <https://docs.rs/niri-ipc/latest/niri_ipc/struct.LayerSurface.html>

The official contract says the event stream provides complete initial
workspace/window state, but related replacement events are not always atomic;
for example, a workspace may disappear before the corresponding window update.
It also warns that patch versions may add Rust fields and enum variants. C2 must
therefore decode JSON defensively, use separate event/action sockets, reconnect
from a new complete stream, and translate unknown input into the domain’s
forward-compatible path.

## Invariants

`State::validate` reports multiple focused windows/workspaces, invalid output
scale/size, and invalid current-mode indices. It deliberately does not reject
dangling cross-object references permitted by event ordering. Tests cover focus
canonicalization, replacement ordering, safe window removal, idempotent urgency,
unknown events, and malformed output values.
