#!/usr/bin/env bash
# Build and sample the idle-CPU candidate on the reference laptop.
set -euo pipefail

output_dir=${1:?pass an output directory}
repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
mkdir -p "$output_dir"
output_dir=$(cd "$output_dir" && pwd)

exec 8>/tmp/lulo-cargo.lock
flock 8
export PATH="$HOME/.cargo/bin:$PATH"
export CARGO_TARGET_DIR="$HOME/rmac-wt/target"

python3 - <<'PY'
import shutil
from pathlib import Path
free_bytes = shutil.disk_usage(Path.home()).free
if free_bytes < 25 * 1024**3:
    raise SystemExit('less than 25 GiB free; refusing to build')
PY

cd "$repo_root"
if [[ ${IDLE_CPU_RESUME:-0} != 1 ]]; then
touch \
    crates/activity-monitor/src/metrics.rs \
    crates/activity-monitor/src/process_table.rs \
    crates/activity-monitor/src/sampling.rs \
    crates/activity-monitor/src/view.rs \
    crates/activity-monitor/src/view_filter.rs \
    crates/clock/src/view.rs \
    crates/weather/src/store.rs \
    crates/weather/src/view.rs \
    vendor/gpui-component/crates/ui/src/input/blink_cursor.rs \
    vendor/gpui-component/crates/ui/src/input/state.rs

cargo metadata --offline --format-version 1 > /dev/null
(cd shell && cargo metadata --offline --format-version 1 > /dev/null)
cargo fmt --all -- --check > "$output_dir/root-fmt.log" 2>&1
(cd shell && cargo fmt --all -- --check > "$output_dir/shell-fmt.log" 2>&1)
rustup run 1.95.0 rustfmt --edition 2024 --check \
    vendor/gpui-component/crates/ui/src/input/blink_cursor.rs \
    vendor/gpui-component/crates/ui/src/input/state.rs \
    > "$output_dir/vendor-fmt.log" 2>&1

echo 'Running focused package tests'
cargo test --offline --profile iterate \
    -p rmac-activity-monitor -p rmac-clock -p rmac-weather \
    -p rmac-text-editor -p rmac-finder \
    > "$output_dir/root-tests.log" 2>&1
echo 'Running focused package Clippy'
cargo clippy --offline --profile iterate \
    -p rmac-activity-monitor -p rmac-clock -p rmac-weather \
    -p rmac-text-editor -p rmac-finder -- -D warnings \
    > "$output_dir/root-clippy.log" 2>&1
fi

if [[ ${IDLE_CPU_RELEASE_RESUME:-0} != 1 ]]; then
echo 'Running vendored input component tests and Clippy'
cargo test --profile iterate \
    --manifest-path vendor/gpui-component/Cargo.toml \
    -p gpui-component -p gpui-component-assets -p gpui-component-macros --lib \
    > "$output_dir/vendor-tests.log" 2>&1
cargo clippy --profile iterate \
    --manifest-path vendor/gpui-component/Cargo.toml \
    -p gpui-component -p gpui-component-assets -p gpui-component-macros \
    --lib -- -D warnings \
    > "$output_dir/vendor-clippy.log" 2>&1
fi

echo 'Building optimized release binaries'
cargo build --offline --release -j 1 \
    -p rmac-activity-monitor -p rmac-clock -p rmac-weather \
    -p rmac-text-editor -p rmac-finder \
    > "$output_dir/release-build.log" 2>&1

exec 9>/tmp/lulo-journey.lock
flock 9
python3 - <<'PY'
from pathlib import Path
names = {'rmac-text-editor', 'rmac-clock', 'rmac-system-monitor', 'rmac-files', 'rmac-weather'}
running = []
for proc in Path('/proc').iterdir():
    if not proc.name.isdigit():
        continue
    try:
        name = (proc / 'exe').resolve().name
    except OSError:
        continue
    if name in names:
        running.append((proc.name, name))
if running:
    raise SystemExit(f'refusing duplicate app instances: {running}')
PY
echo 'Measuring five live apps, one at a time'
python3 scripts/linux/measure-budgets.py \
    --skip-surfaces --app rmac-text-editor --app rmac-clock \
    --app rmac-system-monitor --app rmac-files --app rmac-weather \
    --warmups 0 --repetitions 1 --settle-seconds 3 --idle-seconds 60 \
    --binary-dir "$CARGO_TARGET_DIR/release" \
    --json-output "$output_dir/after-live.json" \
    > "$output_dir/after-live.log" 2>&1
echo 'Measuring shell services together'
python3 scripts/linux/sample-shell-idle.py \
    --seconds 60 --output "$output_dir/shell-after.json" \
    > "$output_dir/shell-after.log" 2>&1

echo 'Running the nested Text Editor idle-typing scenario'
python3 scripts/behavior/run_lulo.py text-editor/find \
    --bin-dir "$CARGO_TARGET_DIR/release" \
    --output "$output_dir/text-editor-find.json" \
    > "$output_dir/text-editor-find.log" 2>&1
echo 'IDLE_CPU_VALIDATED'
