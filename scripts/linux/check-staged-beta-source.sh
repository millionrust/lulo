#!/usr/bin/env bash
# Run scoped native checks against a verified, isolated source snapshot.
set -euo pipefail

source_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)
stage_dir=$(dirname -- "$source_dir")
test -f "$stage_dir/manifest.json"
python3 "$source_dir/scripts/stage-beta-source.py" --verify "$stage_dir"
# The shared Wayland iterate cache may be in the worktree target even when
# the primary checkout has no shell target directory. Select it before the
# existence checks so that fallback is usable in that layout.
root_target="$HOME/rmac/target"
shell_target="$HOME/rmac/shell/target"
if test ! -d "$shell_target/iterate" && test -d "$HOME/rmac-wt/shell/target/iterate"; then
    shell_target="$HOME/rmac-wt/shell/target"
fi
test -d "$root_target"
test -d "$shell_target"
export PATH="$HOME/.cargo/bin:$PATH"
command -v cargo >/dev/null
# Point the snapshot's normal target paths at existing artifacts. These links
# live only inside the disposable staging directory.
if test ! -e "$source_dir/target"; then
    ln -s "$root_target" "$source_dir/target"
fi
if test ! -e "$source_dir/shell/target"; then
    ln -s "$shell_target" "$source_dir/shell/target"
fi

log="$stage_dir/native-check-results.txt"
: > "$log"
exec 8>/tmp/lulo-cargo.lock
flock 8
run_check() {
    for target_dir in "$root_target" "$shell_target"; do
        available_kib=$(df -Pk "$target_dir" | awk 'NR == 2 { print $4 }')
        if test -z "$available_kib" || test "$available_kib" -lt 26214400; then
            echo "Native checks stopped: less than 25 GiB free on the filesystem containing $target_dir." | tee -a "$log" >&2
            return 1
        fi
    done
    printf '\n== %s ==\n' "$*" | tee -a "$log"
    "$@" 2>&1 | tee -a "$log"
}

cd "$source_dir"
run_check /usr/bin/python3 -c 'import gi, sys; gi.require_version("PackageKitGlib", "1.0"); from gi.repository import PackageKitGlib as Pk; sys.exit(0 if hasattr(Pk, "offline_cancel_with_flags") and hasattr(Pk.OfflineFlags, "NONE") else 1)'
run_check cargo check --locked --profile iterate -p rmac-finder -p rmac-ui -p rmac-app-menu -p rmac-shortcuts -p rmac-activity-monitor -p rmac-system-settings -p rmac-archive -p rmac-clock -p rmac-notes
run_check cargo check --locked --profile iterate --tests -p rmac-finder -p rmac-ui -p rmac-app-menu -p rmac-shortcuts -p rmac-activity-monitor -p rmac-system-settings -p rmac-clock -p rmac-notes
run_check cargo test --locked --profile iterate -p rmac-archive
run_check cargo test --locked --profile iterate -p rmac-app-menu --lib
run_check cargo test --locked --profile iterate -p rmac-ui --lib popup_button_activation_accepts_enter_and_space_only
run_check cargo test --locked --profile iterate -p rmac-finder --bin rmac-files app_menu_tests
run_check cargo test --locked --profile iterate -p rmac-clock --bin rmac-clock ticker_tests
run_check cargo test --locked --profile iterate -p rmac-activity-monitor --bin rmac-system-monitor
cd "$source_dir/shell"
run_check cargo check --locked --profile iterate --features wayland -p rmac-shell-app-switcher -p rmac-shell-menubar
run_check cargo test --locked --profile iterate --features wayland -p rmac-shell-app-switcher
run_check cargo test --locked --profile iterate --features wayland -p rmac-shell-menubar power_failure_tests
printf '\nAll scoped native checks passed. Log: %s\n' "$log" | tee -a "$log"
