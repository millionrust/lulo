#!/usr/bin/env bash
# Capture every rmac shell surface on the Ubuntu reference PC and pair each one
# with its macOS reference capture.
#
# Why this exists: as of 2026-09-19 the repository contained 24 screenshots of
# macOS and ZERO screenshots of rmac. Tasks were being ticked from log lines and
# unit tests. You cannot build a look you never look at. Run this after every
# batch of visual work, open the composed pairs, and fix what you see.
#
# Usage (on the reference PC, inside the rmac session):
#   bash scripts/linux/capture-shell-evidence.sh [output_dir]
#
# Requires: grim (screenshots), niri (running), the rmac session active.
# Optional: slurp for region captures.

set -uo pipefail

OUT="${1:-target/evidence/rmac-$(date +%Y-%m-%d)}"
REF="target/evidence/reference-mac"
mkdir -p "$OUT"

have() { command -v "$1" >/dev/null 2>&1; }
shot() { # shot <name> — full output capture
  sleep "${SETTLE:-1.0}"
  grim "$OUT/$1.png" 2>/dev/null && echo "  captured $1.png" || echo "  FAILED $1"
}
dispatch() { /usr/libexec/rmac/rmac-shortcut-dispatch "$1" 2>/dev/null || \
             "$HOME/.local/libexec/rmac/rmac-shortcut-dispatch" "$1" 2>/dev/null; }

if ! have grim; then echo "install grim first: sudo apt install grim"; exit 1; fi

echo "1. Desktop at rest (wallpaper + menu bar + Dock, nothing else)"
niri msg action focus-workspace-down >/dev/null 2>&1
shot desktop

echo "2. Menu bar with a menu open — open the app menu of the focused app by hand"
echo "   (press Ctrl+F2 then Down, or click the app name), then press Enter here"
read -r _
shot menu-open

echo "3. Search (Spotlight equivalent)"
dispatch launcher; SETTLE=1.2 shot search-empty
if have wtype; then wtype "term"; else echo "   type 'term' now, then press Enter"; read -r _; fi
shot search-typing
dispatch launcher

echo "4. Control Center"
dispatch quick-settings 2>/dev/null || echo "   open Control Center by hand, then press Enter"; read -r _
shot control-center
dispatch quick-settings 2>/dev/null

echo "5. Notification Center + a banner"
notify-send "Build finished" "3 warnings, 0 errors" 2>/dev/null
SETTLE=0.6 shot banner
dispatch notification-center 2>/dev/null || echo "   open Notification Center by hand, Enter"; read -r _
shot notification-center
dispatch notification-center 2>/dev/null

echo "6. Apps"
dispatch app-drawer 2>/dev/null || dispatch apps 2>/dev/null
shot apps
dispatch app-drawer 2>/dev/null || dispatch apps 2>/dev/null

echo "7. Files in all four views — open Files, then press Enter after each view"
(rmac-finder >/dev/null 2>&1 &) ; sleep 2
for v in icons list columns gallery; do
  echo "   switch Files to $v view, then press Enter"; read -r _
  shot "files-$v"
done

echo "8. Overlays: OSD, Mission Control, Dock context menu"
/usr/libexec/rmac/rmac-osd volume-up 2>/dev/null || dispatch osd 2>/dev/null
SETTLE=0.4 shot osd
niri msg action toggle-overview >/dev/null 2>&1; shot mission-control
niri msg action toggle-overview >/dev/null 2>&1
echo "   right-click a Dock tile, then press Enter"; read -r _
shot dock-menu

echo
echo "Captured to $OUT"

# --- pair each capture with its macOS reference --------------------------
declare -A PAIRS=(
  [desktop]=desktop-light
  [menu-open]=menu-file-open-dark
  [search-typing]=spotlight-typing-dark
  [control-center]=control-center-dark
  [notification-center]=notification-center-dark
  [apps]=apps-grid-dark
  [files-columns]=finder-view3-dark
  [files-list]=finder-view2-dark
  [files-icons]=finder-view1-dark
  [files-gallery]=finder-view4-dark
  [mission-control]=mission-control-dark
  [dock-menu]=dock-context-menu-dark
)

if [ -d "$REF" ] && [ -f scripts/compose-evidence.py ]; then
  mkdir -p "$OUT/pairs"
  for mine in "${!PAIRS[@]}"; do
    theirs="${PAIRS[$mine]}"
    [ -f "$OUT/$mine.png" ] || continue
    [ -f "$REF/$theirs.png" ] || continue
    python3 scripts/compose-evidence.py "$OUT/$mine.png" "$REF/$theirs.png" \
      "$OUT/pairs/$mine-vs-mac.png" >/dev/null 2>&1 \
      && echo "  paired $mine ↔ $theirs"
  done
  echo
  echo "Side-by-side pairs: $OUT/pairs/"
else
  echo "No reference set or compose script; pairs skipped."
fi

cat <<'NOTE'

Now do the part that actually matters:

  1. Open every file in .../pairs/ at 100% zoom, rmac on the left, macOS on the right.
  2. For each pair, write down every difference you can see, in pixels:
     heights, insets, gaps, radii, font weight, colour, shadow, alignment.
  3. Fix the top five. Re-run this script. Repeat until you cannot tell which
     half is which at a glance.

A pair you did not open is not evidence. A task ticked without a pair in this
folder is not done.
NOTE
