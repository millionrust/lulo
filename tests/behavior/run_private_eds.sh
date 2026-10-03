#!/usr/bin/env bash
# Reference laptop only. Starts a disposable EDS instance on a private session bus.
set -euo pipefail
repo_root=$(cd "$(dirname "$0")/../.." && pwd)
private_root=$(mktemp -d /tmp/rmac-eds-private-XXXXXXXX)
trap 'rm -rf "$private_root"' EXIT
mkdir -p "$private_root/config/evolution/sources" "$private_root/data" "$private_root/cache" "$private_root/state" "$private_root/runtime" "$private_root/home"
chmod 700 "$private_root/runtime"
cat > "$private_root/config/evolution/sources/cal2-local.source" <<'SOURCE'
[Data Source]
DisplayName=CAL-2 Private Local
Enabled=true
[Calendar]
BackendName=local
SOURCE
export XDG_CONFIG_HOME="$private_root/config" XDG_DATA_HOME="$private_root/data"
export XDG_CACHE_HOME="$private_root/cache" XDG_STATE_HOME="$private_root/state"
export XDG_RUNTIME_DIR="$private_root/runtime"
export CARGO_HOME="$HOME/.cargo" RUSTUP_HOME="$HOME/.rustup"
export HOME="$private_root/home"
export RMAC_EDS_PRIVATE_BUS=1
export CARGO_TARGET_DIR="$(dirname "$repo_root")/rmac-wt/target"
cd "$repo_root"
dbus-run-session -- bash -c '
  set -euo pipefail
  /usr/libexec/evolution-source-registry & registry_pid=$!
  /usr/libexec/evolution-calendar-factory & factory_pid=$!
  trap "kill $registry_pid $factory_pid 2>/dev/null || true" EXIT
  timeout 120s cargo test -p rmac-calendar-eds --profile iterate --test private_eds -- --ignored --nocapture
'
