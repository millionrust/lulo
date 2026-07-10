# Dependency and license policy

> Adopted: 2026-07-10
>
> Scope: the product Cargo workspace and its four supported target triples

## Enforcement

The repository uses `cargo-deny` 0.19.8. CI runs the matching official
`cargo-deny-action` release `v2.0.20` with the committed lockfile and checks:

- RustSec vulnerabilities, unsoundness, unmaintained direct dependencies, and
  yanked versions;
- license expressions against an explicit allowlist;
- wildcard and duplicate workspace dependency declarations;
- registry and Git sources against an explicit source policy.

Install and run the same version locally:

```sh
cargo install --locked cargo-deny --version 0.19.8
cargo deny --locked --log-level error check
```

The graph includes all features for arm64/x86_64 macOS and GNU/Linux. The
product graph currently resolves only from crates.io; unknown registries and
all Git dependency sources are denied. Local rmac crates use exact versions as
well as paths, and dependencies shared by multiple members must be declared in
the root `[workspace.dependencies]` table.

## License decision

The globally accepted SPDX licenses are:

- permissive: `0BSD`, `Apache-2.0`, `Apache-2.0 WITH LLVM-exception`,
  `BSD-2-Clause`, `BSD-3-Clause`, `BSL-1.0`, `ISC`, `MIT`, `MIT-0`, `NCSA`,
  `Unicode-3.0`, `Unlicense`, and `Zlib`;
- public-domain dedication: `CC0-1.0`.

`MPL-2.0` is not globally accepted. It is allowed only for `cbindgen` 0.28.0
and `option-ext` 0.2.0, whose file-level copyleft terms are compatible with
their current build/runtime use. Adding another MPL dependency requires an
explicit reviewed exception. No GPL or LGPL-only dependency is accepted by the
product policy; alternative-license expressions pass only when an allowed
choice is available.

This automated metadata check is a guardrail, not legal advice. Release work
must still preserve required copyright and license notices and review assets,
fonts, bundled binaries, and source files that Cargo metadata cannot describe.

## Advisory exceptions

The initial audit upgraded `crossbeam-epoch` from vulnerable 0.9.18 to 0.9.20.
Two advisories remain as exact, reasoned exceptions:

| Advisory | Dependency path | Current exposure | Removal trigger |
|---|---|---|---|
| `RUSTSEC-2026-0194` | `wayland-scanner 0.31.10 -> quick-xml 0.39.4` | Build-time parsing of trusted, dependency-supplied Wayland protocol XML; no product XML input reaches it | Upgrade when `wayland-scanner` supports `quick-xml >= 0.41` |
| `RUSTSEC-2026-0195` | `wayland-scanner 0.31.10 -> quick-xml 0.39.4` | Same bounded build-time path; the vulnerable namespace reader is not exposed to application input | Upgrade when `wayland-scanner` supports `quick-xml >= 0.41` |

These exceptions expire for review on 2026-08-07 and must also be reviewed at
every GPUI dependency review. An exception may not be widened to another crate,
version, runtime input path, or advisory without a new written assessment.

## Duplicate versions

Third-party duplicate versions are warnings rather than errors for now. The
initial graph contains 42 warnings, predominantly inside the pinned GPUI 0.2.2
stack. Resolving them independently would require framework substitutions that
conflict with the GPUI upgrade gate. They remain visible with:

```sh
cargo deny --locked --log-level warn check bans --hide-inclusion-graph
```

Duplicate direct workspace declarations are denied. This prevents new rmac
manifest drift while leaving existing third-party convergence to normal,
reviewed upgrades.

## Isolated upstream experiment

`experiments/gpui-upstream-lab` is deliberately outside the product workspace,
uses exact Git revisions, and is not evaluated by the root `deny.toml`. It is
non-shipping evidence. Before any dependency from that graph can be promoted,
ADR 0002 requires a dedicated resolved-dependency and license review in
addition to the Linux runtime gates.
