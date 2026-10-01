#!/usr/bin/env bash
# Download an unsigned candidate for the checked-out commit on the laptop.
set -euo pipefail
script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
exec python3 "$script_dir/fetch_candidate.py" "$@"
