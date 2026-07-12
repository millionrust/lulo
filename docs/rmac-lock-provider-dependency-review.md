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
client implementation. The adapter must still implement registry removal,
output hotplug, exact configure/ack/commit ordering, dispatch failure, and an
explicit display roundtrip after `unlock_and_destroy`.

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

## PAM candidates not yet accepted

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
depends on the separate `pam-sys2` FFI/bindgen layer, and its published support
statement names a much older tested Rust range. Acceptance requires reviewing
the exact crates.io tarballs—not only repository HEAD—for unsafe blocks,
conversation allocation/free rules, unwind behavior across FFI, `pam_end`
status propagation, generated ABI bindings, build scripts, licenses, owners,
and advisories, followed by Ubuntu x86_64/aarch64 compilation and real PAM tests.

### Handwritten PAM FFI

Rejected as the default path. A small local wrapper would remove a registry
dependency but would make rmac directly responsible for C ABI layout,
conversation ownership, allocation on every error path, unwind exclusion, and
future Linux-PAM changes. Reconsider only if no reviewed wrapper can satisfy the
gate, and then require a separately reviewed C/Rust shim with sanitizers and
fault injection.

## Rendering remains open

The lock client should start with a CPU shared-memory path so authentication and
recovery do not depend on Vulkan availability. `tiny-skia` 0.11.4 and `memmap2`
0.9.11 are already resolved candidates, but they are not promoted to direct
product dependencies until a prototype proves exact buffer lifetime, scale,
format, damage, release, resize, and memory bounds. GPUI windows cannot replace
the privileged lock-surface role.

## Acceptance gate for the PAM selection

Before adding a PAM crate to `Cargo.toml`:

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
