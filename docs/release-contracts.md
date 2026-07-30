# Release contract checks

`scripts/run-release-contract-checks.py` is the fast, build-free integrity gate
for the A/H/I release machinery. It runs the focused Python fixtures plus the
A4 report, update trust, keyring packages, atomic APT publication, hardware, foundation evidence, journey, visual,
accessibility, performance, chaos/soak, security, documentation, Alpha, Beta, and 1.0 structural
validators.

```sh
python3 scripts/run-release-contract-checks.py
```

The runner:

- performs no Rust build, Docker operation, network request, package install,
  evidence mutation, or shell command construction;
- uses the current Python interpreter and argument-separated subprocesses;
- enforces the 15 GiB repository storage floor;
- gives each stage 60 seconds;
- stops on the first failure and returns its bounded final output; and
- keeps bytecode and generated evidence out of the repository.

Use `--list` to inspect the exact 17 stages. The committed manifest and runner
both pin that inventory so a contract cannot disappear through a manifest-only
edit.

This gate validates definitions and fixture behavior. It does not build the
Rust workspace, authenticate evidence, run a native app, replace H8 hardware,
or turn pending A1–A6 observations into a framework decision. CI runs it early
so structural drift fails quickly, while the larger platform and release gates
remain independently mandatory.
