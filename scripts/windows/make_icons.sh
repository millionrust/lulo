#!/usr/bin/env bash
# Rasterise each Windows app's own rmac artwork
# (packaging/rmac-apps/icons/org.rmac.<App>.svg, the same files
# scripts/build-icons.py writes for the Linux .desktop icons -- never
# Apple's) into a multi-resolution .ico for its exe's embedded icon
# resource (ADR 0023 "Installer"). Run before `cargo build --release` so
# rmac-windows-resource-build's build.rs finds a real icon for each app;
# without this step the exe still builds, with the platform's default
# binary icon and a `cargo:warning` saying why.
#
# Needs ImageMagick ("magick"), preinstalled on the windows-latest runner
# image. Usage: make_icons.sh [output-dir]
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
out_dir=${1:-"$root/packaging/windows/icons"}
mkdir -p "$out_dir"

python3 - "$root" "$root/packaging/windows/apps.json" "$root/packaging/rmac-apps/icons" "$out_dir" <<'PY'
import json
import pathlib
import subprocess
import sys

root, apps_path, icons_dir, out_dir = (pathlib.Path(a) for a in sys.argv[1:5])
apps = json.loads(apps_path.read_text())
# Covers everything from a Start Menu tile down to a small list icon;
# ImageMagick packs them into one multi-resolution .ico.
sizes = (16, 24, 32, 48, 64, 128, 256)

for app in apps:
    # Most apps' artwork is their Linux .desktop icon
    # (packaging/rmac-apps/icons/<app_id>.svg); an entry can instead name
    # its own source (the Lulo layer's own mark is not a packaged Linux
    # app icon). An entry with neither (a helper exe with no Start Menu
    # shortcut of its own, e.g. lulo-shell) gets no icon here -- its exe
    # still builds, with the platform's default icon.
    if app.get("icon_svg"):
        svg = root / app["icon_svg"]
    elif app.get("app_id"):
        svg = icons_dir / f"{app['app_id']}.svg"
    else:
        print(f"skipping {app['id']!r}: no icon source declared")
        continue
    if not svg.is_file():
        raise SystemExit(f"missing icon artwork for {app['id']!r}: {svg}")
    pngs = []
    for size in sizes:
        png = out_dir / f"{app['id']}-{size}.png"
        subprocess.run(
            [
                "magick",
                "-background", "none",
                "-density", "384",
                str(svg),
                "-resize", f"{size}x{size}",
                str(png),
            ],
            check=True,
        )
        pngs.append(png)
    ico = out_dir / f"{app['id']}.ico"
    subprocess.run(["magick", *(str(p) for p in pngs), str(ico)], check=True)
    for png in pngs:
        png.unlink()
    print(f"wrote {ico}")
PY
