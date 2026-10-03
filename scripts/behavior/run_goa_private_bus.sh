#!/usr/bin/env bash
set -euo pipefail

# The private bus and XDG directories protect the owner's GOA/EDS state.
tmpdir=$(mktemp -d)
trap 'rm -rf "$tmpdir"' EXIT
mkdir -p "$tmpdir/config" "$tmpdir/data" "$tmpdir/state" "$tmpdir/cache"
export XDG_CONFIG_HOME="$tmpdir/config"
export XDG_DATA_HOME="$tmpdir/data"
export XDG_STATE_HOME="$tmpdir/state"
export XDG_CACHE_HOME="$tmpdir/cache"
export RMAC_TEST_PRIVATE_BUS=1
export CARGO_TARGET_DIR="$HOME/rmac-wt/target"
export PATH="$HOME/.cargo/bin:$PATH"
exec 8>/tmp/lulo-cargo.lock
flock 8

dbus-run-session -- cargo test -p rmac-accounts-linux --profile iterate \
  --test private_goa -- --ignored --exact adapter_uses_goa_wire_contract_on_private_bus
