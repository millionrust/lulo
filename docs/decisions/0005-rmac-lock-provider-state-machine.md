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
4. Only matching authentication success constructs a move-only unlock token.
   Failure increments a saturating public attempt count; cancellation does not.
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
disabled. The admitted Linux-only development wrapper validates callback input,
contains unwind, checks and wipes owned C allocations, preserves arbitrary
bounded conversation ordering, and owns the complete start/auth/account/end
transaction described in `docs/rmac-pam-wrapper-audit.md`. It remains excluded
from the installed provider until its compiled fault tests and real PAM matrix
execute on Ubuntu.

The conversation transport is an rmac-owned, capacity-one worker/UI broker.
Each prompt owns bounded, drop-zeroized content and a redacted monotonic ID; its
single-use response capability prevents stale replies. UI loss, prompt drop,
explicit cancellation, and response-style mismatch all wake the PAM worker and
fail closed. Secret responses move through this channel without cloning. This
transport does not grant lock authority and remains disconnected until secure
Wayland input and the rest of the production evidence exist.

`rmac-lock-provider-linux` is the start of that adapter boundary. Its first
accepted primitive is fixed-capacity credential input, bounded to Linux-PAM's
512-byte response limit and zeroized on editing, clear, transfer, and drop.
This alone does not make the crate an installed authentication provider. The
dependency decisions and remaining production acceptance gate are recorded in
`docs/rmac-lock-provider-dependency-review.md`.

The same adapter owns a bounded lock-surface lifecycle. It coalesces unhandled
configures, permits only a paint for the newest configure and scale, carries the
one serial that must be acknowledged before commit, rejects oversized layouts,
limits buffers awaiting compositor release, and accounts for hotplug removal.
This keeps renderer allocation and generated Wayland objects outside the core
security state machine while making their ordering independently testable.

The Linux Wayland boundary may preflight or prepare the connection without
locking. Preparation binds the compositor, shared-memory, session-lock manager,
outputs, and version-4 keyboard seats. It completes bounded roundtrips for
output scale, seat capabilities, and an `xkb_v1` keymap before readiness, then
tracks hotplug, focus, modifiers, layout group, repeat metadata, and bounded
semantic input. Keymap mapping is capped at 16 MiB; decoded text is capped at 64
bytes, redacted, and erased on drop. Loss of a required singleton or malformed
input state is terminal. Per-seat client repeat clamps hostile compositor
settings, emits no catch-up burst, re-decodes with current modifiers, and is
cancelled on release, focus loss, keymap replacement, or compositor repeat.
Optional per-seat pointers retain only focus, surface-local position, and one
redacted pressed target. A left-button release emits a semantic action only when
it matches the pressed target; focus/capability/output/seat loss cancels it.
Submit and radio selection share the keyboard editor path, while generic binary
prompts expose no clickable reply. Caps Lock is derived from xkb locked-modifier
state and aggregated across focused seats. Only the semantic boolean leaves the
adapter; it repaints a noninteractive warning for text/password prompts without
entering the credential queue.

The crate-only lock typestate now issues acquisition, immediately creates one
role per output, renders only after configure, performs exact ack/scale/attach/
damage/commit ordering, and retains every sealed frame until buffer release.
Hotplug creates or destroys roles while release accounting survives output
removal. The wire requires the core's move-only authentication token before it
may send `unlock_and_destroy`, destroy every role, and wait for a display-sync
barrier before emitting the flush event. Client commit is tracked as commit—not
presentation—and never replaces the compositor's authoritative `locked` event.
This typestate remains unavailable outside the crate until the complete runtime
and recovery path exist.

The platform-neutral credential editor already joins semantic input to one
single-use broker prompt. Echo-off and echo-on buffers are bounded and erased,
multi-scalar insertion is atomic, and submit/cancel move the one response
capability exactly once. Notice and radio styles have generic keyboard behavior;
binary MFA remains unavailable until its module-specific presentation exists.
This editor still has no lock authority; no shipped presentation/event loop
drives it yet.

The platform-neutral runtime coordinator joins these boundaries without doing
I/O. It starts authentication only after the compositor's `locked` event,
permits one worker, bounds pre-prompt input, drains cancellation before worker
reuse, rejects stale completion, and moves the core unlock token into one wire
action. Worker panic and invalid ordering fail closed. A crate-internal Linux
pump multiplexes Wayland with worker/prompt polling and exposes only readiness,
prompt-change, failure, and an authenticated/denied/failed-locked exit reason.
An opt-in, uninstalled process resolves the exact logind session, verifies its
UID against the process, derives its PAM user name, sets the advisory locked
hint before systemd readiness, and clears the hint only after authenticated
unlock plus the display-sync barrier. Pure lifecycle tests reject impossible
ordering and preserve the hint on post-lock failure. The process has no shipped
unit and does not yet have Linux-validated localized prompt shaping or recovery.

Recovery development uses two non-enabled evidence-only units, never the
shipping unit name. The custom unit has a bounded restart burst; a distinct
fallback unit runs the accepted swaylock supervisor against the same private
nested-compositor environment. The interactive harness must observe a ready
custom instance, stop its event loop, observe a watchdog-driven ready
replacement, kill that replacement, observe a second new ready PID and restart
count, stop it without unlocking, then complete authentication through
swaylock. The watchdog is armed only after compositor-confirmed readiness and
uses the manager-provided interval, so startup work cannot counterfeit liveness
and late checks cannot emit a catch-up burst. This proves
the intended recovery sequence only when run on Linux; committed unit/script
tests prove configuration separation but are not runtime evidence.

The initial renderer is an opaque CPU composition written in bounded chunks.
Its Linux backing is a no-exec anonymous file, immutable after
painting through kernel seals, and owned with its redacted buffer token. The
wire preserves that ownership until the compositor's release event; rendering
completion alone never authorizes readiness or unlock. A copyable state snapshot
contains prompt category, capped indicator count or selection, and failure
state. A separate drop-zeroized label admits at most 256 bytes of normalized
valid UTF-8 from the bounded PAM prompt, strips bidi controls, and falls back to
fixed text when it cannot safely present the source. Credential responses never
cross this boundary. Linux shapes the label with the existing exact
`cosmic-text` line and installed fonts, then blends a bounded alpha raster into
the sealed frame. Prompt identity avoids work on password edits; glyph state
resets per prompt and at most eight 2 MiB layout masks are retained. Diagnostics
redact label, identity, state values, and pixels. Raster failure is fail-closed.

Swaylock remains the installed/default provider until the adapter passes the
Linux PAM, wrong-password, cancel, MFA, output hotplug, scale/rotation,
suspend/resume, renderer failure, provider crash, accessibility, and emergency
recovery matrix. The new provider must use a separate unit so fallback is a
packaging/session choice, never an automatic fail-open downgrade during an
active lock.

## Consequences

- Provider behavior and the small isolated FFI boundary can be reviewed and
  fault-tested independently of the presentation and service process.
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
