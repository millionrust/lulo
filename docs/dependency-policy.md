# Dependency and license policy

> Adopted: 2026-07-10
>
> Scope: the product Cargo workspace and the separately locked Linux shell graph

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
product graph resolves crates.io plus an explicit `[sources] allow-git` list:
`zed-industries/zed.git` (GPUI itself), `longbridge/gpui-component.git`, and
the `zed-industries` forks (`font-kit`, `scap`, `reqwest`) GPUI depends on.
Any other registry or Git source is denied. Local rmac crates use exact
versions as well as paths, and dependencies shared by multiple members must be
declared in the root `[workspace.dependencies]` table.

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

## Separately locked Linux shell graph

`shell` remains outside the product workspace because
the four packaged layer-shell hosts require a newer GPUI API than the ordinary
application graph. It is shipping input for `rmac-wallpaper`, `rmac-top-bar`,
`rmac-dock`, and `rmac-osd`, not a disposable preview.

The graph has its own lockfile and `deny.toml`. CI checks its advisories,
licenses, bans, and sources independently. Every Git dependency has an exact
revision and version, and only the four reviewed upstream repositories are
allowed. GPL-only Zed tracing is replaced by the local MIT compatibility crate
at `compat/ztracing`, which only re-exports the permissively licensed `tracing`
macros required by `sum_tree`. GPL is not added to the accepted license list.

The shell policy covers the two supported GNU/Linux architectures. The root
policy continues to cover the application workspace on macOS and Linux. A
revision or source-policy change still requires a dedicated dependency review
and the Ubuntu runtime gates in ADR 0001.
