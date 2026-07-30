# Contributing

## Before changing code

Read `PLAN_V2.md` and `ARCHITECTURE.md`. Work from a user outcome and keep the
change inside the active roadmap phase. New platform behavior belongs behind a
service boundary rather than inside a GPUI render implementation.

The worktree may contain another contributor's changes. Do not discard or
rewrite unrelated modifications.

## Validation while working

Use the narrowest package, crate, script, or fixture that exercises the change.
Reuse the normal `target` directory and do not compile multiple feature graphs
in parallel. Before a build likely to exceed 1 GiB, check free space and do not
start below 25 GiB; stop before the data volume reaches 15 GiB free.

Examples:

```sh
cargo test --locked -p rmac-theme
python3 -m unittest scripts.test_documentation
```

Do not use Docker validation, duplicate the repository, create a second Cargo
target directory, or run workspace-wide `--all-features`/`--all-targets`
commands unless that exact release/CI validation was requested.

The clean release candidate and CI run the full formatting, lint, test,
dependency, packaging, journey, and platform gates. A release is not complete
while any required job or evidence gate is red.

Install the dependency-policy tool with
`cargo install --locked cargo-deny --version 0.19.8`. Advisory exceptions and
license additions require a written scope and removal/review condition in
`docs/dependency-policy.md`.

## Code rules

- Keep domain logic independent of GPUI, D-Bus, Wayland, and platform FFI.
- Keep blocking filesystem and subprocess work off the UI thread.
- Prefer events and subscriptions to polling or unconditional redraw timers.
- Use typed errors; do not ignore failures from destructive operations.
- Use `&Path` for borrowed filesystem paths and `PathBuf` for owned paths.
- Keep platform commands fully argument-separated; never invoke a shell for
  user-controlled input.
- Add a fake implementation for every new platform service.
- Preserve unknown fields and variants when decoding forward-compatible IPC.
- Use XDG base directories on Linux and version persisted formats.
- Follow `docs/decisions/0002-gpui-version-policy.md` for every GPUI-family
  dependency change. Framework upgrades must be isolated from feature work.

## Tests

- Unit-test parsers, sorting, state transitions, persistence, and command
  construction.
- Add a regression test with every behavior bug fix.
- Test success, cancellation, permission denial, missing services, corrupt data,
  and interrupted operations where applicable.
- UI work must document keyboard and accessibility behavior.
- Visual fixes require a reproducible screenshot comparison until automated
  golden testing is available.

## Pull-request description

Include:

1. the user outcome;
2. scope and explicit non-goals;
3. failure and recovery behavior;
4. keyboard and accessibility behavior;
5. performance impact;
6. tests run;
7. screenshots or recordings for visual changes;
8. platforms tested;
9. documentation changes.

## Definition of done

A feature is done only when its acceptance criteria and failure states work,
the required checks pass, accessibility behavior is verified, errors are
actionable, documentation is current, and a reviewer can reproduce the result
from a clean checkout.
