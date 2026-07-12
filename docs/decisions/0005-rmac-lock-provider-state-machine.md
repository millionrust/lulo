# ADR 0005: rmac lock-provider state machine

- Status: Accepted foundation; platform provider not selected for production
- Date: 2026-07-12

## Context

The accepted swaylock boundary provides real PAM authentication and
`ext-session-lock-v1` exclusion, but swaylock cannot render the rmac lock
presentation or the bounded notification projection. A cosmetic GPUI overlay
cannot solve this: while locked, the compositor normally renders only surfaces
owned by the privileged session-lock client, and that client is responsible for
authentication and the unlock request.

Replacing swaylock therefore combines several security-critical concerns:

- acquire and retain the compositor session-lock object;
- create and resize one exact lock surface for every current and hot-plugged
  output;
- report readiness only after the compositor's `locked` event;
- run the distribution PAM stack, including multi-message conversations;
- retain credentials only for the bounded conversation and clear them after;
- reject stale asynchronous authentication results;
- send `unlock_and_destroy` only after successful PAM authentication;
- flush the unlock request before exiting; and
- remain fail-closed on provider, renderer, PAM, or compositor failure.

No reviewed PAM binding currently exists in the product dependency graph, and
the macOS development host cannot provide Linux compositor/PAM evidence.

## Decision

`rmac-lock-provider` is the dependency-free authority for provider lifecycle.
It contains no credential, PAM, Wayland, renderer, filesystem, notification,
or D-Bus type. Its events and transitions enforce these invariants:

1. A rendered frame never means secure readiness. Only the compositor's
   `ext_session_lock_v1.locked` event emits the one systemd-ready transition.
2. Output presentation state is tracked separately. Hotplug makes presentation
   incomplete until that output's lock frame is presented, but compositor
   blanking remains secure.
3. Starting authentication produces a unique attempt token. Success, failure,
   and cancellation must carry the active token; stale results cannot mutate
   state.
4. Only matching authentication success emits the unlock transition. Failure
   increments a saturating public attempt count; cancellation does not.
5. A compositor `finished` event before readiness is a denied acquisition. The
   same event after readiness is a failed-but-still-locked terminal state. It
   never emits unlock.
6. The provider exits normally only after its unlock request has been flushed,
   or fail-closed after compositor denial/failure. Process death never requests
   unlock.
7. Output and attempt identifiers are redacted from diagnostics.

The Linux adapter must use the distribution PAM service. A reviewed binding
must correctly pair `pam_start`/`pam_end`, execute the complete configured
authentication and account policy, support arbitrary bounded conversation
messages, keep PAM work off the render/event loop, and erase response buffers.
The adapter—not the state machine—owns secrets.

Exact published-source review rejected `pam`, `pam-client2`, and `nonstick` for
callback unwind, allocation, cleanup, or transaction-lifetime defects. The only
accepted candidate dependency is raw `pam-sys2` with its optional bindgen path
disabled; rmac must supply the small checked callback/RAII wrapper described in
`docs/rmac-pam-wrapper-audit.md`. It is not admitted to the manifest until that
wrapper and fault-injection tests exist.

`rmac-lock-provider-linux` is the start of that adapter boundary. Its first
accepted primitive is fixed-capacity credential input, bounded to Linux-PAM's
512-byte response limit and zeroized on editing, clear, transfer, and drop.
This does not select a PAM binding or make the crate an authentication provider.
The dependency decisions and remaining PAM acceptance gate are recorded in
`docs/rmac-lock-provider-dependency-review.md`.

The same adapter owns a bounded lock-surface lifecycle. It coalesces unhandled
configures, permits only a paint for the newest configure and scale, carries the
one serial that must be acknowledged before commit, rejects oversized layouts,
limits buffers awaiting compositor release, and accounts for hotplug removal.
This keeps renderer allocation and generated Wayland objects outside the core
security state machine while making their ordering independently testable.

The Linux Wayland boundary may preflight or prepare the connection without
locking. Preparation binds the compositor, shared-memory, session-lock manager,
and outputs, completes initial output-scale dispatch, and then tracks hotplug.
Loss of a required singleton is terminal. Lock acquisition remains unavailable
until the prepared objects can immediately create every output role and render
through the bounded lifecycle.

The initial renderer is an opaque, dependency-free CPU composition written in
bounded chunks. Its Linux backing is a no-exec anonymous file, immutable after
painting through kernel seals, and owned with its redacted buffer token. Wire
buffer creation and release must preserve that ownership until the compositor's
release event; rendering completion alone never authorizes readiness or unlock.

Swaylock remains the installed/default provider until the adapter passes the
Linux PAM, wrong-password, cancel, MFA, output hotplug, scale/rotation,
suspend/resume, renderer failure, provider crash, accessibility, and emergency
recovery matrix. The new provider must use a separate unit so fallback is a
packaging/session choice, never an automatic fail-open downgrade during an
active lock.

## Consequences

- Provider behavior can be reviewed and tested before introducing unsafe FFI or
  another dependency source.
- Lock presentation work has an explicit interface that cannot accidentally
  transport credentials or authorize unlock from UI state.
- The current product remains less visually complete but keeps its proven PAM
  boundary while the replacement earns security evidence.
- E5 and E10 remain incomplete.

## Upstream contracts

- `ext-session-lock-v1`:
  <https://wayland.app/protocols/ext-session-lock-v1>
- Linux-PAM upstream:
  <https://github.com/linux-pam/linux-pam>
- swaylock readiness and configuration:
  <https://github.com/swaywm/swaylock/blob/master/swaylock.1.scd>
