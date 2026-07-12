# rmac PAM wrapper exact-source audit

Date: 2026-07-12

Scope: published crates that could back the custom lock provider's Linux-PAM
application boundary. This is an implementation security review, not evidence
that authentication works on the Ubuntu reference PC.

## Artifacts reviewed

| Crate | Published artifact SHA-256 | VCS revision | License |
|---|---|---|---|
| `pam` 0.8.0 | `8ab553c52103edb295d8f7d6a3b593dc22a30b1fb99643c777a8f36915e285ba` | `85d2683af51255397413de544e3e53397366f069` | MIT OR Apache-2.0 |
| `pam-client2` 0.5.5 | `407daa00f98b05147dbbaa6d2f6bce574e76794fa4952a5b38764681a23dde5f` | `7551e27418d935a25facbf1854400b2e9a28f2c7` | MPL-2.0 |
| `nonstick` 0.1.2 | `de28bc5222f0f8495fdf5134dd91b7dc880bfdc1fa0646499d80c13d7ebbbd46` | not present in artifact | MIT |
| `pam-sys2` 1.0.2 | `63e07ea89c210813e1a48fc32cb358ba693aae8ca70163461e22d11f1884bf1d` | `a01492a3543d41b7749aff4a4ab74c7c510dec9d` | MIT OR Apache-2.0 |

The hashes were calculated over the exact `.crate` downloads. Cargo manifests,
all shipped Rust source, build scripts, licenses, and pre-generated Linux-PAM
bindings were read from those archives. Upstream contracts remain Linux-PAM's
application headers and documentation.

## Rejected safe-wrapper candidates

### `pam` 0.8.0

Reject. Its conversation callback is marked with a FIXME asking for
verification, performs no catch-unwind boundary, does not validate all incoming
pointers/counts before dereference, and duplicates responses with `strdup`.
On a partial conversation error it frees the response array but not strings
already placed inside it. Upstream also labels environment support probably
broken and retains a TODO to verify conversation leaks. These are disqualifying
for a long-lived lock authority.

### `pam-client2` 0.5.5

Reject despite better API coverage and correct normal-path `pam_end` RAII.

- `pam_converse` calls arbitrary Rust conversation methods from `extern "C"`
  without `catch_unwind`.
- Text responses are duplicated with unchecked `strdup`; allocation failure can
  become a successful null response.
- Error cleanup frees response strings without overwriting them.
- A successful callback transfers another unzeroized secret copy to PAM.
- `Context::from_boxed_conv` loses the boxed conversation handler when
  `pam_start` fails, and also on the success-with-null-handle defense path.
- `Context::drop` uses panicking conversation extraction before `pam_end`.
- MPL-2.0 would require a product-policy exception, but licensing is not the
  reason for rejection.

### `nonstick` 0.1.2

Reject despite its complete typed conversation model and intent to zero text.

- The application conversation callback has no catch-unwind boundary and casts
  a negative message count to `usize` before a documented validation layer.
- Its C-heap `calloc` helper constructs `NonNull` with `new_unchecked` without
  checking allocation failure, making OOM undefined behavior.
- Text answers are cast from a zeroing string owner into a generic C-heap box;
  normal `Answers` drop then frees them without calling the zeroing destructor.
- Service names containing NUL panic, and `pam_end` return values are ignored.

These findings also reject `nonstick2` as an unreviewed substitution; a name or
fork change is not evidence that the exact paths above were corrected.

## Accepted low-level candidate

`pam-sys2` 1.0.2 is accepted only as a candidate raw ABI source, not as an
authentication implementation. It ships pre-generated Linux-PAM declarations
and compile-time layout assertions, uses no bindgen or network access under its
default features, and links the system `pam` and `pam_misc` libraries. The
optional source-generation feature must stay disabled. Its raw declarations
are necessarily unsafe but add no callback, allocation, or transaction policy.

The rmac wrapper must therefore own a deliberately small reviewed unsafe module
on top of `pam-sys2`, rather than adopting any rejected high-level wrapper. This
is the fallback allowed by the dependency review after no existing wrapper met
the gate.

## Implemented local-wrapper properties

`pam-sys2` is now admitted as a Linux-only development dependency beneath a
small rmac wrapper. The implementation provides the following reviewed
properties:

1. validate `num_msg` in `1..=PAM_MAX_NUM_MSG`, every outer pointer, every
   message pointer, every style, and bounded NUL termination before dereference;
2. wrap the entire Rust callback body in `catch_unwind` and return
   `PAM_CONV_ERR` after zeroing partial responses on panic or error;
3. allocate response arrays and strings with checked C allocations, cap text at
   `PAM_MAX_RESP_SIZE`, and overwrite every still-owned response before free;
4. support echo-on, echo-off, info, error, radio, and bounded binary messages,
   preserving the order of arbitrary multi-message batches;
5. move rmac secret input into the worker without cloning and keep the single
   unavoidable PAM-owned response copy inside the documented Linux-PAM trust
   boundary;
6. pair each successful `pam_start` with exactly one `pam_end`, propagate the
   end status, and call both `pam_authenticate` and `pam_acct_mgmt`;
7. never implement `Send`/`Sync` for the PAM handle; construct and consume the
   whole transaction on one bounded worker thread;
8. compile fault injection for nulls, negative/zero/33 message counts,
   allocation failure, panic, partial batches, and authentication/account/end
   outcomes before real Ubuntu PAM testing.

The platform-neutral conversation contract has executable macOS tests for
style/reply matching, response bounds, NUL rejection, binary header allowance,
and diagnostic redaction. The raw callback and transaction fault tests compile
for Linux but cannot execute on the macOS development kernel. They must run on
the Ubuntu reference PC before this evidence is considered complete.

The platform-neutral worker/UI broker has executable tests for ordered prompt
delivery, unique redacted identities, secret allocation movement without a
clone, response-style rejection on both sides, explicit and drop cancellation,
UI disconnection, binary bounds, and prompt redaction. It permits at most one
outstanding prompt and uses a unique single-use response capability, so delayed
UI state cannot answer a subsequent PAM message.

The semantic prompt editor has executable platform-neutral tests for Unicode
secret editing and backspace, echo-on submission, notice acknowledgement,
radio selection, cancellation, binary-prompt rejection, and exact-capacity
overflow without partial insertion. These tests reach through the broker to the
typed PAM conversation response; they do not replace the native PAM or keyboard
matrix.

The runtime coordinator adds executable tests for a successful secret reaching
the one-shot unlock boundary, wrong-password retry, cancellation followed by
worker drain, queued input reuse, stale completion rejection, worker panic,
compositor finish, and bounded/redacted pre-prompt input. The Linux pump compiles
against the real `Worker` and never joins it until `JoinHandle::is_finished` is
true. Presentation tests verify that the renderer receives prompt category, a
capped indicator count or selection, failure state, and a separately bounded
presentation label—not response bytes. Tests cover invalid UTF-8 fallback,
Unicode preservation, whitespace normalization, bidi-control removal,
scalar-safe truncation, alpha-mask bounds, and diagnostic redaction. A
Linux-only test requires installed fonts to shape, rasterize, cache, and redact
a mixed-script prompt. Pointer tests cover exact submit/radio hit regions,
non-finite/out-of-bounds rejection, drag-away cancellation, focus-loss
cancellation, and redacted gesture diagnostics; emitted actions reuse the same
single-use prompt editor as keyboard input. Portable process tests prove
readiness/hint ordering and
distinguish authenticated unlock from compositor denial and post-lock failure.
The feature-gated Linux runner derives its PAM name from the exact logind
session only after verifying session ownership. The native Ubuntu matrix
remains required.

`cargo deny 0.19.8 check` passed advisories, bans, licenses, and sources for the
admitted lockfile on 2026-07-12.

`pam-sys2` 1.0.2 has a cross-build defect: its build script uses the build
host's `cfg!(target_os)` rather than Cargo's target OS. Native Linux builds pick
Linux-PAM automatically. A Linux check from macOS must set
`PAM_SYS_IMPL=linuxpam`; this override is required in cross-build CI and is not
needed in the installed Ubuntu build.

Linux-PAM owns successful response allocations after the callback returns, so
rmac cannot honestly promise to overwrite that transferred copy itself. The
supported Ubuntu PAM stack must be reviewed and tested for its cleanup behavior;
all rmac-owned copies still require immediate zeroization. This limitation is a
property of the PAM conversation ownership contract, not permission to retain
extra application copies.

## Evidence still required

- archive crates.io owner data when the registry API is available (it returned
  HTTP 403 during this review);
- compare pre-generated x86_64/aarch64 layouts with Ubuntu 26.04 headers;
- compile/link against the reference image's `libpam0g-dev`, install the
  reviewed `pam/rmac-lock` common-auth/common-account policy as
  `/etc/pam.d/rmac-lock`, and package only the runtime library dependency;
- run password, wrong password, locked/expired account, cancellation, MFA,
  allocation failure, callback panic, and `pam_end` failure tests under
  sanitizers where supported.

Until that evidence exists, swaylock remains the production PAM client.
