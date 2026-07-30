# Supported hardware matrix

H8 uses five bounded stations instead of making unsupported claims from one
developer machine. The authoritative inventory is
`packaging/hardware-matrix.json`.

| Station | Release gate | Coverage |
| --- | --- | --- |
| amd64 Intel laptop | Alpha | Mesa, internal display, 100/200%, lid and suspend, touchpad, USB and Bluetooth |
| amd64 AMD desktop | Alpha | Mesa, multiple displays, all four scales, USB/Bluetooth/audio hotplug |
| amd64 AMD laptop | 1.0 | Mesa laptop, mixed fractional multi-display, lid and suspend |
| amd64 NVIDIA desktop | Beta | NVIDIA driver, multiple displays, all four scales and hotplug |
| arm64 reference | 1.0 | Hardware Vulkan, 100/200%, suspend and common peripherals |

The combined matrix covers amd64 and arm64, Intel/AMD/NVIDIA, single and
multiple displays, 100/125/150/200% scales, internal/USB/Bluetooth audio,
wired/USB/Bluetooth input, touchpad, display/device hotplug, lid behavior and
suspend/resume.

Validate the matrix before preparing a station:

```sh
python3 scripts/linux/verify-hardware-matrix.py
```

## Running a station

Use Ubuntu 26.04 and the exact revision under test. Keep stock GNOME Wayland
installed. Start with `scripts/linux/run-reference-gates.sh`, then complete the
station's checks returned by:

```sh
python3 - <<'PY'
import importlib.util
from pathlib import Path

path = Path("scripts/linux/verify-hardware-matrix.py")
spec = importlib.util.spec_from_file_location("matrix", path)
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)
matrix = module.load_matrix()
station = next(item for item in matrix["stations"] if item["id"] == "amd64-intel-laptop")
print(*module.required_checks(station), sep="\n")
PY
```

Each named scale is a separate run. Multi-monitor stations must include
connect/disconnect, primary/output routing, mixed scales, lid/dock where
applicable, and suspend/resume. Device checks cover arrival, removal, service
restart, authoritative state recovery and the corresponding Settings/shell
surface. Performance and accessibility use the existing project gates rather
than a visual assertion.

Raw logs, device model names, serials, addresses, machine IDs, usernames,
session identifiers and paths remain in the ignored evidence directory. The
committed result contains only the station ID, exact 40-hex revision and the
canonical check/pass list.

Create `<station>.json` for every station required by the release tier:

```json
{
  "format": 1,
  "results": [
    {
      "check": "applications",
      "status": "pass"
    }
  ],
  "revision": "0000000000000000000000000000000000000000",
  "station": "amd64-intel-laptop"
}
```

The example is intentionally incomplete. Generate the full sorted result list
from `required_checks`; do not turn a blocked, skipped or inferred observation
into `pass`. Then verify the exact directory:

```sh
python3 scripts/linux/verify-hardware-matrix.py \
  --evidence-dir /absolute/path/to/reviewed-results \
  --tier alpha \
  --revision "$(git rev-parse HEAD)"
```

Alpha requires the Intel laptop and AMD desktop. Beta adds NVIDIA. The 1.0 gate
requires all five stations, including arm64 and the second AMD form factor.
Passing a tier does not broaden support to an untested GPU driver, architecture
or device class.
