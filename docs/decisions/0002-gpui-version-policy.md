# ADR 0002: pin GPUI exactly and promote upgrades through the Linux gate

- Status: accepted, amended for the packaged Linux shell
- Date: 2026-07-10
- Owners: rmac maintainers
- Review cadence: every four weeks and at each phase gate

## Context

GPUI is pre-1.0 and its current development branch has material API and crate
layout differences from the published 0.2.2 release. The product also depends
on `gpui-component` and `gpui-component-assets`, which must stay compatible with
the selected GPUI version. Broad semver requirements would allow an ordinary
lockfile refresh to change this framework stack without running the required
Linux runtime evaluation.

ADR 0001 requires accessibility, input, renderer, and layer-shell evidence
before a product migration. The current-upstream experiment now compiles on
macOS and Linux, but its Ubuntu/Wayland runtime gates remain open.

## Decision

### Product dependency line

- Product crates use exact workspace requirements: GPUI `=0.2.2`,
  `gpui-component =0.5.1`, and `gpui-component-assets =0.5.1`.
- All three requirements are owned by the root workspace manifest. Member
  crates may not declare their own GPUI-family version.
- `Cargo.lock` is committed. A GPUI-family lockfile change is allowed only in a
  dedicated framework-upgrade change; it must not ride along with feature work.
- The product remains on this line until every promotion gate below passes. A
  compiling experiment alone is not approval.

### Separately locked Linux shell line

- `experiments/gpui-upstream-lab` stays outside the product workspace.
- It pins a full immutable Git revision and its matching Rust toolchain and
  commits its own lockfile.
- Its `wallpaper`, `top-bar`, `dock`, and `osd` binaries are the maintained
  layer-shell hosts packaged as the corresponding `rmac-*` session binaries.
- Each host consumes the framework-neutral rmac runtime/model crate recorded by
  `SHIPPING_SHELL_SOURCES`; application binaries remain on the root GPUI line.
- The shell graph has a dedicated `deny.toml` and CI gate. Exact reviewed Git
  sources are permitted, but GPL-only tracing is replaced by an original MIT
  compatibility shim instead of widening the product license policy.
- Moving the revision requires a dedicated change containing the upstream
  comparison, resolved dependency diff, macOS smoke result, Linux compile
  result, and updated runtime evidence.
- Product workspace crates must not import the shell host package.

### Review cadence and triggers

The maintainers review the GPUI release line and upstream experiment every four
weeks and before each phase exit. An out-of-cycle review starts when any of the
following occurs:

- a relevant security advisory or unsoundness report;
- a GPUI release or upstream change claiming Linux accessibility, Wayland,
  layer-shell, IME, scaling, renderer, or crash fixes;
- the pinned line stops building on the supported Rust toolchain or OS images;
- a confirmed product blocker has an upstream fix.

Security reviews start immediately. Other triggers are triaged within seven
days. Review does not imply upgrade; a no-change result is recorded in the
experiment evidence.

## Promotion gates

A candidate may replace the product pin only when one dedicated migration
change demonstrates all of the following:

1. formatting, strict Clippy, tests, and clean lockfile resolution pass on
   macOS and Linux;
2. the complete ADR 0001 protocol passes on Ubuntu 26.04 with GNOME Wayland and
   niri, including Orca, IME, clipboard, portals, scaling, multi-display,
   fullscreen layer-shell behavior, idle CPU, and the four-hour soak;
3. Intel/AMD evidence exists and NVIDIA is tested before the Phase 1 exit;
4. one representative application is migrated first and the adapter/API diff
   is bounded and reviewed before the remaining applications move;
5. accessibility semantics and keyboard behavior do not regress;
6. the change includes release notes, known limitations, and rollback steps.

No gate may be waived by labeling the candidate experimental. Failed criteria
need a bounded workaround with an owner; otherwise ADR 0001's reconsideration
path applies.

## Upgrade mechanics

1. Branch from a green product revision and record the old exact versions and
   lockfile checksum.
2. Update only the isolated experiment first. Run and record all compile and
   runtime gates.
3. Update the root exact requirements in one change and regenerate the root
   lockfile with targeted `cargo update -p` commands.
4. Keep persistence formats, user-facing features, and unrelated refactors out
   of the migration so the framework change can be reverted independently.
5. Land the representative application and shared adapter changes before
   mechanical migrations of the remaining applications.

## Rollback policy

The previous exact requirements and lockfile remain available in the parent
commit. Revert the migration as one unit when it introduces a crash, data-loss
risk, inaccessible core journey, broken input/IME, renderer regression, or
layer-shell focus/fullscreen failure that cannot be corrected immediately.

Because framework upgrades may not alter persisted data formats, rollback must
not require user-data conversion. Any narrowly necessary compatibility adapter
belongs in `rmac-ui` or a platform boundary, not duplicated across applications.

## Consequences

- Dependency refreshes cannot silently upgrade the UI framework family.
- Framework upgrades cost a dedicated evidence cycle, which is intentional for
  the project's highest platform risk.
- Security fixes may require an expedited evaluation, but still require the
  safety and accessibility gates proportional to the affected code path.
- The packaged shell and ordinary application graphs can evolve independently,
  while both remain lockfile-, policy-, and runtime-gated.

## Evidence

- `docs/decisions/0001-gpui-linux-gate.md`
- `docs/gpui-0.2.2-stable-spike.md`
- `docs/gpui-current-upstream-spike.md`
- `experiments/gpui-upstream-lab`
