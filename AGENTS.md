# Local Agent Constraints

This machine has limited disk capacity. Keep validation scoped and reuse existing build artifacts.

- Do not run Docker-based Rust validation or create Docker Cargo registry, Git, or target volumes unless the user explicitly requests it.
- Do not copy this repository into `/private/tmp` or create a separate `CARGO_TARGET_DIR` for review or validation.
- Do not run workspace-wide `--all-features` or `--all-targets` Cargo commands unless the user explicitly requests that exact validation.
- Prefer package-, crate-, binary-, or test-specific checks that exercise the changed code.
- Before a build likely to exceed 1 GiB, check `df -h /System/Volumes/Data`. Stop if available space is below 25 GiB.
- Reuse the repository's normal `target` directory and avoid compiling the same dependency graph under multiple feature combinations.
- Do not run multiple Cargo validation pipelines concurrently.
