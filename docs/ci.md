# Continuous integration

> Adopted: 2026-09-24
>
> Scope: `.github/workflows/ci.yml` and `.github/workflows/ci-quality.yml`

Two workflow files. `ci.yml` is the per-PR merge gate; `ci-quality.yml` adds
MSRV drift and an aarch64 cross-build check to every PR, and moves the
heavier dependency/security audit and the fuzz smoke runs to a weekly
schedule so per-PR CI stays disk- and time-bounded (see `AGENTS.md`).
Neither workflow runs a workspace-wide `--all-features`/`--all-targets`
command outside its own scoped job; see each job below for its exact
command.

Related docs: `docs/dependency-policy.md` (the `cargo-deny` policy in
detail), `docs/journey-suite.md` (the package-scoped fixture runner).

## `ci.yml` — every push and pull request

| Job | What it checks | Run it locally |
|---|---|---|
| `release-contracts` | `scripts/run-release-contract-checks.py` — the committed release-contract suite | `python3 scripts/run-release-contract-checks.py` |
| `dependency-policy` | `cargo deny check` (advisories, licenses, bans, sources) against the root `deny.toml` | `cargo install --locked cargo-deny --version 0.19.8 && cargo deny --locked --log-level error check --hide-inclusion-graph` |
| `linux` | Formatting, the shared-control/gpui_component/no-mac-captures/design-token boundary scripts, wording, Clippy (`-D warnings`), `cargo test --workspace --all-features`, and the full `scripts/test_*.py` suite | See below |
| `linux-2604` | `linux`'s formatting/Clippy/tests/Python-suite steps (not its repo-specific boundary scripts) on `ubuntu-26.04` instead of `ubuntu-24.04`. **Non-blocking** (`continue-on-error: true`) — see [Ubuntu 26.04](#ubuntu-2604) | Same `cargo`/`python3` commands as `linux`, below |
| `macos` | Clippy, tests, and the Python suite on `macos-15` | Same commands as `linux`, minus the Linux-only boundary scripts |
| `upstream-gpui-linux` | The separately locked `shell/` workspace: formatting, its own `cargo deny`, `cargo test --lib`, Clippy on the `wayland` feature, and a nested-Wayland smoke check | See `shell/` steps below |

To run what `linux`/`macos` run, from the repo root:

```sh
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
cargo test --locked --workspace --all-features
python3 -m unittest discover -s scripts -p 'test_*.py'
```

`python3` must be 3.11+ (the CI jobs pin 3.12 via `actions/setup-python`):
two of the `scripts/test_*.py` files use `tomllib` (3.11+) and
`zip(..., strict=True)` (3.10+). The system `python3` on this repo's
reference macOS machines is an old Xcode 3.9 stub that lacks both — install
a newer Python (`brew install python@3.12`, or `pyenv`) before running the
suite locally, or aim `python3 -m unittest` at that interpreter directly.
`scripts/test_build_cursors.py` also needs Pillow and
`scripts/test_release_workflows.py` needs PyYAML; the jobs install
`Pillow==12.1.1 PyYAML==6.0.3` after setting up Python.

Two of the source gates carry a checked-in exemption that may only shrink:

- `scripts/check-design-tokens.sh` compares per-file counts of hard-coded
  colors and radii against `scripts/design-token-baseline.txt`. A new file or
  a growing count fails; after tokenizing values, run
  `bash scripts/check-design-tokens.sh --update` and commit the smaller
  baseline (`--report` lists every remaining value).
- `scripts/check-gpui-component-imports.sh` does the same for
  `gpui_component` uses against `scripts/gpui-component-baseline.txt`.

`scripts/check-wording.py` skips a Rust line that ends in
`// wording: internal` — use it only for literals the user never sees, such as
a program name to match or a word used to screen error text.

For `shell/`:

```sh
cd shell
cargo fmt --all -- --check
cargo test --locked --lib
cargo clippy --locked --bins --features wayland -- -D warnings
dbus-run-session -- bash scripts/nested-wayland-smoke.sh
```

## `ci-quality.yml`

Triggers on `pull_request`, `push` to `master`/`dev`, a weekly `schedule`
(Monday 05:17 UTC), and `workflow_dispatch`. Jobs are split by `if:` on
`github.event_name` so PR-time jobs never run on the schedule and vice
versa.

### `msrv` — every PR

Confirms `rust-version` in `Cargo.toml` and `shell/Cargo.toml` still matches
the channel pinned in `rust-toolchain.toml`/`shell/rust-toolchain.toml`
(currently `1.95.0`, matching the reference laptop). Pure TOML parsing, no
build:

```sh
python3 scripts/check-msrv.py
```

If the toolchain is ever bumped, bump `rust-version` in the same commit —
this job is what would catch forgetting to.

### `linux-aarch64` — every PR, non-blocking

Cross-builds the crates with no GPUI/Wayland/Vulkan dependency
(`rmac-storage`, `rmac-apps`, `rmac-shell-settings`, `rmac-theme`,
`rmac-compositor-niri`) for `aarch64-unknown-linux-gnu` using
[`cross`](https://github.com/cross-rs/cross) (Docker-based; that's fine in
GitHub's cloud runners even though `AGENTS.md` forbids Docker-based Rust
validation on this laptop — that rule is about the local development
machine, not GitHub's runners).

This is marked `continue-on-error: true` because it is new and the
project's first real aarch64 CI coverage. It proves the
toolchain/target/`cross` pipeline works; it does **not** yet cross-build
the GPUI-based application binaries, because `cross`'s stock Docker image
doesn't carry aarch64 Wayland/Vulkan/fontconfig headers. Extending it needs
a custom `cross` image (a `Cross.toml` pointing at a Dockerfile that
installs those `-dev` packages for `arm64`) — tracked as follow-up, not
done here. `cargo zigbuild` is a lighter-weight (no Docker) alternative
worth revisiting for that follow-up.

Run the same cross-build locally (needs Docker):

```sh
cargo install --locked --git https://github.com/cross-rs/cross \
  --rev f8151ae777290430cf2108efacf3976d9528500b cross
rustup target add aarch64-unknown-linux-gnu
cross build --locked --target aarch64-unknown-linux-gnu \
  -p rmac-storage -p rmac-apps -p rmac-shell-settings -p rmac-theme \
  -p rmac-compositor-niri
```

todo.md's "native smoke test before Beta" phase is not started. GitHub's
`ubuntu-24.04-arm`/`ubuntu-26.04-arm` hosted runners (native Arm64, no
cross-compilation) are a plausible way to do that later without `cross` at
all; not adopted yet to avoid the extra per-PR runner cost.

### `dependency-audit` — weekly + `workflow_dispatch`

`cargo deny check advisories` for both workspaces, plus
[`rustsec/audit-check`](https://github.com/rustsec/audit-check) (`cargo
audit`) for both `Cargo.lock` files. `rustsec/audit-check` opens its own
GitHub issue automatically when it finds an advisory on a scheduled run
(that's the action's built-in behavior, not something this job configures);
a separate best-effort `gh issue create` step (`|| true`, so a transient
API error doesn't leave the job red for an unrelated reason) covers the
`cargo deny` steps failing for a different reason, which the action can't
see.

Run locally:

```sh
cargo deny --locked --log-level error check advisories --hide-inclusion-graph
cargo deny --locked --log-level error --manifest-path shell/Cargo.toml \
  --config shell/deny.toml check advisories --hide-inclusion-graph
cargo install --locked cargo-audit
cargo audit --file Cargo.lock
cargo audit --file shell/Cargo.lock
```

### `fuzz` — weekly + `workflow_dispatch`

Runs each target in `fuzz/` for 60 seconds via `cargo-fuzz` on nightly (one
matrix job per target, so one hung/slow target doesn't block the others).
Crash artifacts upload on failure.

Run one target locally for 60 seconds:

```sh
rustup toolchain install nightly --profile minimal
cargo install --locked cargo-fuzz
cd fuzz
cargo +nightly fuzz run desktop_entry -- -max_total_time=60
```

Targets and what they actually exercise:

| Target | Real function under test |
|---|---|
| `desktop_entry` | `rmac_apps::desktop_group_named` — the freedesktop `.desktop` `[group]`/`key=value` tokenizer used to build the installed-application catalog |
| `shell_settings_config` | `rmac_shell_settings::ShellSettingsStore::load`, through a minimal in-memory `rmac_storage::Backend` — the real JSON version-detection and migration logic, no file I/O |
| `theme_config` | `rmac_theme::ThemeStore::load`, same `Backend` trick — the real stored-preferences JSON parser |
| `niri_ipc` | `serde_json::from_str::<rmac_compositor_niri::wire::{Event, Reply}>` — the same `Deserialize` impls the niri IPC event stream and command replies are parsed with |
| `terminal_escape` | **Not** `crates/terminal`'s own code. `crates/terminal` only has a `[[bin]]` target, so its `advance_filtered_output` wrapper (the thin combining-mark-capping layer around the parser) isn't reachable from an external crate without adding a library target — a restructuring this task intentionally didn't do. This target instead drives the same pinned `vte`/`alacritty_terminal` versions (`Processor::advance` over a `Term`) directly, which covers the escape-sequence parser itself but not rmac's wrapper around it. |

`fuzz/` is its own Cargo workspace (`[workspace]` with no `members`), kept
separate from the root and `shell/` workspaces so `cargo-fuzz` owns its own
lockfile and `target` directory and none of `fuzz/`'s dependencies (e.g.
`libfuzzer-sys`) leak into the product graph or its `cargo deny` policy.

Three crates gained a visibility change so the fuzz targets could call the
real parsers without any other restructuring:

- `crates/rmac-apps/src/platform.rs`: `desktop_group_named` is now `pub`
  (was `pub(super)`), re-exported from `lib.rs`.
- `crates/rmac-compositor-niri/src/lib.rs`: `mod wire;` is now `pub mod
  wire;` (the types inside were already `pub`, just unreachable from
  outside the crate).
- `rmac-shell-settings` and `rmac-theme` needed no change: their stores
  already take a generic `Backend` (from `rmac-storage`), so an external
  fuzz crate can supply its own in-memory implementation.

## MSRV

`rust-version = "1.95.0"` in `Cargo.toml` and `shell/Cargo.toml` records
what `rust-toolchain.toml`/`shell/rust-toolchain.toml` already pin — the
same version the reference laptop uses. There is deliberately no separate
"build with an older Rust" job: every other CI job already builds with the
exact pinned toolchain, so the pinned version *is* the MSRV. `msrv`
(above) only guards against the two files drifting apart.

## `cargo deny` sources

`deny.toml`'s `[sources]` section denies unknown registries and unknown Git
sources by default (`unknown-registry = "deny"`, `unknown-git = "deny"`).
Until this task, the root `deny.toml` had no `allow-git` list at all, which
meant *every* Git dependency — including GPUI itself
(`zed-industries/zed.git`) and `gpui-component`
(`longbridge/gpui-component.git`) — was technically "unknown" and denied.
`allow-git` now lists exactly the Git sources the resolved graph uses (see
`deny.toml` and `docs/dependency-policy.md`); anything else stays denied.

## Ubuntu 26.04

GitHub's `ubuntu-26.04` hosted runner image went GA on 2026-09-17, so
todo.md's "Move the Linux jobs from ubuntu-24.04 to Ubuntu 26.04
(self-hosted until GitHub offers the image)" no longer needs a self-hosted
runner or an `ubuntu:26.04` container fallback — the native hosted image
covers it.

Two jobs moved immediately (`release-contracts`, `dependency-policy`):
neither runs `apt-get`, so the runner OS doesn't matter to them. The two
GPUI jobs (`linux`, `upstream-gpui-linux`) install a specific list of
`apt` `-dev` packages (Wayland, Vulkan, X11, fontconfig); those package
names/versions on 26.04 are unverified from this machine (no Linux runner
available locally), so `linux-2604` runs the same checks as `linux` on
`ubuntu-26.04` but non-blocking. Once it has run clean for a while, it
should replace `linux` outright and `upstream-gpui-linux` should move the
same way.
