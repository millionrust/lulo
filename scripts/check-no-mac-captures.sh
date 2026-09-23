#!/usr/bin/env bash
# Screenshots of macOS are measuring references only and must never be
# published with rmac (they show Apple's copyrighted interface).
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."
if tracked="$(git ls-files | grep -E '(^design-lab/ref/|(^|/)target/|-mac\.(png|jpe?g|webp)$|-vs-mac\.)')" \
  && [[ -n "$tracked" ]]; then
  echo "screenshots of macOS or build output must not be committed:" >&2
  echo "$tracked" >&2
  exit 1
fi
echo "no macOS captures are tracked"
