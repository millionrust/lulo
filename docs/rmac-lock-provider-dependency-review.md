# rmac lock-provider dependency review

Date: 2026-07-12

This review covers only dependencies needed to turn the ADR 0005 state machine
into a Linux `ext-session-lock-v1` provider. It does not authorize replacing
swaylock; that requires the runtime evidence in `docs/secure-lock.md`.

## Accepted lines

### Wayland protocol client

Use the already-resolved Smithay stack when the adapter is implemented:

- `wayland-client` 0.31.14;
- `wayland-protocols` 0.32.13 with only `client` and `staging` features.

The latter supplies the generated official
`ext::session_lock::v1::client` interfaces. Both versions already exist in the
workspace lockfile through GPUI, are MIT licensed, and avoid adding a second
client implementation. The adapter must preserve registry removal, output
hotplug, exact configure/ack/commit ordering, dispatch failure, and an explicit
display roundtrip after `unlock_and_destroy`.

They are now direct, Linux-only dependencies of `rmac-lock-provider-linux`.
Its first Wayland API is a non-mutating registry preflight: it confirms protocol
version 1, `wl_compositor` version 4, `wl_shm` version 1, at least one output,
and `wl_seat` version 4, then drops the connection without binding a global or
requesting a lock. A second safe API binds those authorities, every output, and
every supported seat. Three setup roundtrips receive output scale, seat
capabilities, and the keyboard keymap before readiness; live output, seat,
focus, modifier, and keyboard events are then tracked. Removal of a required
singleton is terminal. An internal, crate-only typestate now combines those
objects with lock roles, buffers, and fail-closed event handling. It is not an
externally callable or installed acquisition API.

### Keyboard decoding

Use exact `xkbcommon` 0.8.0 with default features disabled and only its Wayland
file-mapping feature. It is MIT licensed, already existed in the workspace
lockfile through GPUI, and calls the distribution `libxkbcommon` rather than
shipping another keyboard engine. The admitted path is deliberately narrow:
context construction, private read-only `xkb_v1` keymap mapping, state creation,
serialized modifier updates, one-keysym lookup, UTF-8 lookup, and locale compose
state. X11 support is not enabled. Official protocol rules require adding eight
to Wayland keycodes and updating all depressed, latched, locked, and group masks;
the adapter follows both rules.

Keymap sizes are checked in `2..=16 MiB` before the wrapper's unsafe private
mapping call, and unsupported formats and states fail preparation. Keymap
installation, decoding, modifier updates, and compose reset are contained by a
panic boundary. Only semantic actions or a 64-byte bounded UTF-8 fragment leave
the decoder; control characters and overlong fragments are erased and rejected,
and diagnostics expose neither text nor raw keycodes.
The fragment is erased on drop. The wrapper and the system library still need
native malformed-keymap and compose evidence. Bounded per-seat client repeat is
implemented without another dependency, but its live compositor behavior still
needs Linux evidence. Input-method/IME support and accessibility behavior also
remain open gates.

References: the official
[`wl_keyboard` protocol](https://wayland.freedesktop.org/docs/html/apa.html#protocol-spec-wl_keyboard),
[`xkb_state` API](https://xkbcommon.org/doc/current/group__state.html), and
[`xkb_v1` keymap format](https://xkbcommon.org/doc/current/keymap-text-format-v1-v2.html).

### Secret erasure

Use exact `zeroize` 1.9.0, already resolved in the workspace and dual
MIT/Apache-2.0 licensed. It uses volatile writes plus fences so erasure cannot
be optimized away and zeroizes a `Vec`'s complete capacity. The new
`rmac-lock-provider-linux::SecretInput` allocates the Linux-PAM response maximum
once, never clones or reallocates it, erases removed characters immediately,
and transfers the allocation to a redacted response wrapper without copying.

This mitigates ordinary allocator residue. It cannot promise that compiler
temporaries, swapped pages, hardware side channels, a malicious PAM module, or
the kernel never copy a secret; product documentation and threat claims must
remain narrower than that.

## PAM dependency decision

### `pam` 0.8.0

Advantages: small API, custom conversation trait, Debian/Fedora packaging, and
MIT/Apache-2.0 licensing. It is not accepted because upstream describes the
out-of-box path as basic username/password only, warns that environment support
is broken, and retains a TODO to verify that its conversation does not leak
memory. That is insufficient for a lock provider required to support arbitrary
configured MFA conversations and strict secret lifetime.

### `pam-client2` 0.5.5

Advantages: authentication, account management, custom multi-message
conversations, and RAII `pam_end` coverage. It is not accepted yet because it is
a recent fork, is MPL-2.0 (requiring a new explicit product-policy exception),
and its published support statement names a much older tested Rust range. The
exact 0.5.5 artifact has now been rejected: its callback lacks a catch-unwind
boundary, response allocation/cleanup does not meet the secret lifetime rule,
and `pam_start` error paths lose the boxed handler. `pam` 0.8.0 and `nonstick`
0.1.2 also fail the exact-source gate. See
`docs/rmac-pam-wrapper-audit.md` for hashes and findings.

### Handwritten PAM FFI

Rejected as the default path. Exact-source review has now shown that none of the
three high-level candidates satisfies the gate, so the reconsideration
condition is met. Raw `pam-sys2` 1.0.2 is now a Linux-only development
dependency with default pre-generated bindings and a small rmac-owned wrapper.
The wrapper validates callback memory and limits, contains panics, wipes partial
responses, supports every Linux conversation style, calls authentication plus
account policy, and propagates normal-path `pam_end` failure. Injected raw APIs
cover stage ordering and exactly-one end behavior without a real PAM service.
It is not approved as the installed provider until the remaining Ubuntu matrix
in `docs/rmac-pam-wrapper-audit.md` passes.

## Rendering evidence remains open

The lock client should start with a CPU shared-memory path so authentication and
recovery do not depend on Vulkan availability. Exact `rustix` 1.1.4 is now a
direct Linux-only dependency for its safe `memfd_create`, resize, and sealing
APIs; it was already resolved, is MIT/Apache-2.0 licensed, and introduces no
second syscall wrapper. Each frame is painted sequentially through a fixed
16 KiB chunk, then sealed against write, grow, shrink, and further seal changes.
The memfd also requests the Linux no-exec seal at creation.

This removes the need to promote `tiny-skia` for the first opaque frame
(`memmap2` is now present only through the XKB file-mapping feature). Those
remain possible later renderer candidates, not product dependencies. The
internal wire creates one role per output, waits for configure, acknowledges
only the newest serial, creates and immediately destroys each `wl_shm_pool`,
attaches/damages an ARGB8888 buffer at exact scale, and retains both the buffer
proxy and backing frame until `wl_buffer.release`. Hotplug destroys the role and
surface but not an unreleased buffer. Unlock consumes the core state machine's
move-only authentication token, destroys roles, and waits for a display-sync
barrier. These paths still require Ubuntu/niri runtime evidence; GPUI windows
cannot replace the privileged lock-surface role.

## Development process boundary

The uninstalled process is gated by the opt-in `development-provider` feature.
It uses the workspace's existing `zbus` 5.16.0 line only on Linux to resolve the
exact `XDG_SESSION_ID`, read the session UID/name, and call `SetLockedHint` on
that resolved object. It rejects an effective-UID mismatch before connecting to
Wayland or PAM. Readiness uses the already-required absolute
`/usr/bin/systemd-notify` executable with fixed arguments and no shell; this
matches the accepted swaylock supervisor boundary without adding libsystemd.
The feature and binary are absent from the session installer, and no custom
provider unit is installed while recovery evidence remains open.

## Production acceptance gate for the PAM selection

Before replacing swaylock with the development PAM path:

1. archive exact source/checksum/license and ownership evidence;
2. review every unsafe block and C allocation/free path;
3. prove arbitrary prompt/info/error sequences and cancellation;
4. prove password, Unicode, empty, maximum, and over-maximum responses;
5. call authentication plus configured account policy and pair every successful
   `pam_start` with exactly one `pam_end`;
6. prevent unwind across the C callback and keep PAM off the render loop;
7. test wrong password, locked account, expired credential, unavailable module,
   MFA success/failure, provider cancellation, and worker crash on Ubuntu;
8. pass `cargo deny`, license notices, x86_64/aarch64 builds, and the reference
   PC suspend/hotplug/recovery matrix.

Until all eight pass, swaylock remains the only installed authentication
provider.
