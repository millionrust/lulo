#!/usr/bin/env bash
set -euo pipefail

lab_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
target_dir="${CARGO_TARGET_DIR:-$lab_dir/target}"
if [[ "$target_dir" != /* ]]; then
  target_dir="$lab_dir/$target_dir"
fi
runtime_root="$(mktemp -d)"
runtime_dir="$runtime_root/runtime"
wayland_socket=""
ipc_socket=""
layer_pid=""
a11y_pid=""
sway_pid=""

cleanup() {
  local status=$?
  trap - EXIT INT TERM

  for pid in "$a11y_pid" "$layer_pid" "$sway_pid"; do
    if [[ -n "$pid" ]] && kill -0 "$pid" 2>/dev/null; then
      kill "$pid" 2>/dev/null || true
      wait "$pid" 2>/dev/null || true
    fi
  done

  if [[ $status -ne 0 ]]; then
    for log in "$runtime_root"/*.log; do
      if [[ -f "$log" ]]; then
        echo "===== $(basename "$log") =====" >&2
        tail -200 "$log" >&2
      fi
    done
  fi

  rm -rf "$runtime_root"
  exit "$status"
}
trap cleanup EXIT INT TERM

wait_for_path() {
  local path=$1
  local owner_pid=$2
  local description=$3

  for _ in {1..200}; do
    if [[ -e "$path" ]]; then
      return 0
    fi
    if ! kill -0 "$owner_pid" 2>/dev/null; then
      echo "$description exited before creating $path" >&2
      return 1
    fi
    sleep 0.1
  done

  echo "timed out waiting for $description to create $path" >&2
  return 1
}

wait_for_sway_sockets() {
  for _ in {1..200}; do
    wayland_socket="$(find "$runtime_dir" -maxdepth 1 -type s -name 'wayland-*' -print -quit)"
    ipc_socket="$(find "$runtime_dir" -maxdepth 1 -type s -name 'sway-ipc.*.sock' -print -quit)"
    if [[ -n "$wayland_socket" && -n "$ipc_socket" ]]; then
      return 0
    fi
    if ! kill -0 "$sway_pid" 2>/dev/null; then
      echo "Sway exited before creating its Wayland and IPC sockets" >&2
      return 1
    fi
    sleep 0.1
  done

  echo "timed out waiting for Sway sockets" >&2
  return 1
}

mkdir -p "$runtime_dir"
chmod 700 "$runtime_dir"
mkdir -p "$runtime_root/cache" "$runtime_root/config"

export XDG_RUNTIME_DIR="$runtime_dir"
export XDG_CACHE_HOME="$runtime_root/cache"
export XDG_CONFIG_HOME="$runtime_root/config"
export XDG_SESSION_TYPE=wayland
export WLR_BACKENDS=headless
export WLR_HEADLESS_OUTPUTS=1
export WLR_LIBINPUT_NO_DEVICES=1
export WLR_RENDERER=pixman
export LIBGL_ALWAYS_SOFTWARE=1

lavapipe_icd="$(find /usr/share/vulkan/icd.d -maxdepth 1 -type f -name '*lvp*json' -print -quit 2>/dev/null || true)"
if [[ -n "$lavapipe_icd" ]]; then
  export VK_ICD_FILENAMES="$lavapipe_icd"
fi

cat >"$runtime_root/sway.conf" <<'EOF'
xwayland disable
default_border none
output HEADLESS-1 mode 1280x720
seat seat0 fallback true
EOF

cd "$lab_dir"
cargo build --locked --jobs "${CARGO_BUILD_JOBS:-2}" --features wayland --bins

sway --unsupported-gpu --config "$runtime_root/sway.conf" --debug \
  >"$runtime_root/sway.log" 2>&1 &
sway_pid=$!
wait_for_sway_sockets
export WAYLAND_DISPLAY="$(basename "$wayland_socket")"
export SWAYSOCK="$ipc_socket"

if ! wayland-info >"$runtime_root/wayland-info.log" 2>&1; then
  echo "wayland-info could not inspect the nested compositor" >&2
  exit 1
fi
if ! grep -q 'zwlr_layer_shell_v1' "$runtime_root/wayland-info.log"; then
  echo "nested compositor does not advertise zwlr_layer_shell_v1" >&2
  exit 1
fi

# A normal desktop starts the AT-SPI registry before applications. Explicitly
# activate it in this minimal D-Bus session so AccessKit can register each
# window when it starts.
gsettings set org.gnome.desktop.interface toolkit-accessibility true
gsettings set org.gnome.desktop.a11y.applications screen-reader-enabled true
/usr/bin/python3 -c 'import pyatspi; pyatspi.Registry.getDesktop(0)'

RMAC_SMOKE_READY_FILE="$runtime_root/layer-shell.ready" \
  "$target_dir/debug/layer-shell" >"$runtime_root/layer-shell.log" 2>&1 &
layer_pid=$!
wait_for_path "$runtime_root/layer-shell.ready" "$layer_pid" "layer-shell probe"
grep -qx 'layer-shell' "$runtime_root/layer-shell.ready"

tree_json="$(swaymsg -t get_tree -r)"
workspace_y="$(jq -r '[.. | objects | select(.type? == "workspace" and .name? != "__i3_scratch")][0].rect.y' <<<"$tree_json")"
if [[ "$workspace_y" != "40" ]]; then
  echo "layer-shell exclusive zone did not reserve 40 pixels; workspace y=$workspace_y" >&2
  exit 1
fi

RMAC_SMOKE_READY_FILE="$runtime_root/a11y.ready" \
  "$target_dir/debug/a11y" >"$runtime_root/a11y.log" 2>&1 &
a11y_pid=$!
wait_for_path "$runtime_root/a11y.ready" "$a11y_pid" "accessibility probe"
grep -qx 'a11y' "$runtime_root/a11y.ready"

/usr/bin/python3 scripts/assert_accessibility.py

kill -0 "$layer_pid"
kill -0 "$a11y_pid"
echo "nested Wayland frame, layer-shell exclusive-zone, and AT-SPI smoke checks passed"
