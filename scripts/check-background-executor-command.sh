#!/usr/bin/env bash
# LINUX-HW-07 (docs/parity.md) / ARCHITECTURE.md "Safety boundaries":
# `std::process::Command` must never run directly inside
# `cx.background_executor().spawn(...)`. GPUI's background executor is a
# small fixed-size worker-thread pool; spawning (or waiting on) a child
# process from one of its threads has hung indefinitely on the reference
# laptop while the identical call returns in under 100 ms outside GPUI (see
# the Mouse-pane fix, commit 7c38a3b7). Route such work through
# `blocking::unblock`, the dedicated blocking-task pool the rest of the
# codebase already uses for this (sound.rs, spotlight.rs, wallpaper.rs,
# storage.rs, ...).
#
# This is a heuristic line-window scan, not a parser: it looks at the lines
# following `background_executor()` + `.spawn(async` for a bare
# `Command::new(` / `std::process::Command` and treats the block as safe when
# `blocking::unblock` also appears before it in the same window (the
# established idiom nests the blocking call inside the spawn). A genuine
# false positive can be silenced with a `// background-executor-allow: <why>`
# comment on the flagged line.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

# How many lines after `.spawn(async` to scan for a Command call. Generous
# enough for the multi-line spawns this codebase writes, small enough to
# avoid bleeding into the next, unrelated spawn.
WINDOW=25

status=0
files="$(grep -rl 'background_executor()' --include='*.rs' crates shell/bins shell/crates shell/compat 2>/dev/null || true)"

for file in $files; do
  awk -v file="$file" -v window="$WINDOW" '
    # Record every line so we can look back/ahead by number.
    { lines[NR] = $0 }
    /\.spawn\(async/ { spawn_lines[NR] = 1 }
    END {
      for (start in spawn_lines) {
        saw_unblock = 0
        for (i = start; i < start + window && i <= NR; i++) {
          line = lines[i]
          if (line ~ /blocking::unblock/) {
            saw_unblock = 1
          }
          if (line ~ /background-executor-allow/) {
            continue
          }
          if (line ~ /Command::new\(/ || line ~ /std::process::Command/ || line ~ /process::Command::new/) {
            if (!saw_unblock) {
              printf "%s:%d: Command spawned inside background_executor().spawn without blocking::unblock\n", file, i
              status = 1
            }
          }
        }
      }
      exit status
    }
  ' "$file" || status=1
done

if (( status == 0 )); then
  echo "background_executor()/Command boundary holds"
fi
exit "$status"
