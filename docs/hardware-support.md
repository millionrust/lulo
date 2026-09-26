# Hardware support

No hardware configuration is certified for a public rmac release yet. The
table below is the required validation matrix, not a compatibility claim.

| Station | Intended gate | Required coverage |
| --- | --- | --- |
| amd64 Intel laptop | Alpha | Mesa, internal display, 100/200%, lid, suspend, touchpad, USB and Bluetooth |
| amd64 AMD desktop | Alpha | Mesa, multiple displays, 100/125/150/200%, USB/Bluetooth/audio hotplug |
| amd64 NVIDIA desktop | Beta | NVIDIA driver, multiple displays, all supported scales and hotplug |
| amd64 AMD laptop | 1.0 | Mesa, mixed fractional displays, lid and suspend |
| arm64 reference | 1.0 | Hardware Vulkan, 100/200%, suspend and common peripherals |

The exact station and release-tier authority is
[the H8 matrix](hardware-matrix.md).

## Baseline requirements

- Ubuntu 26.04 and a separate stock Ubuntu/GNOME Wayland recovery session
- niri and working Wayland portals
- Vulkan rendering supported by the installed driver
- a display/output combination that supports the scale and refresh-rate gate
- working systemd user services, D-Bus, PipeWire/WirePlumber, NetworkManager,
  BlueZ where applicable, UPower, logind, polkit, and PackageKit
- at least 25 GiB free before large builds and never less than 15 GiB free

## Not yet claimed

The project does not currently claim support for an untested distribution,
compositor, proprietary GPU/driver combination, architecture, touch device,
screen reader configuration, unusual filesystem, VPN plugin, printer, smart
card, or suspend firmware. Unsupported controls remain visibly unavailable;
rmac must not pretend a missing Linux authority works.

Fractional scaling, mixed displays, 60/120 Hz motion, hotplug, dock/lid events,
Bluetooth and USB devices, suspend/resume, lock recovery, and accessibility
must be proven on the named stations before the corresponding release tier.

## Touchpad recovery after resume

Some Intel/amd64 laptops (the reference station included) wire their
touchpad as a Synaptics RMI4 device reached over SMBus, with a legacy PS/2
`psmouse` serio passthrough used only to negotiate the SMBus handover at
boot. A suspend/resume on that stack can leave the PS/2 side unable to hand
control back: the kernel logs
`psmouse serioN: Failed to deactivate mouse on isa0060/serioN: -5` followed
by `rmi4_smbus`/`rmi4_f01` resume failures, and only a non-gesture
"PS/2 Generic Mouse" (or no pointer device at all) comes back — everywhere,
including the GDM greeter, until reboot.

The rmac-session package installs
`/usr/lib/systemd/system-sleep/rmac-input-resume`
(`packaging/rmac-session/system-sleep/rmac-input-resume`). systemd-logind
runs it after every resume; when the `rmi4_smbus`/`psmouse` stack is present,
it always unloads `rmi_smbus` and `psmouse`, reloads `psmouse`, and ensures
`rmi_smbus` is loaded. The input device list can retain a stale touchpad
entry after a failed resume, so it is not used to decide whether recovery is
needed. Each module command has a short timeout and logs its result under the
`rmac-input-resume` journal tag. It is a no-op on any machine that does not
run this exact driver stack, and it never touches suspend/hibernate
firmware settings. See [Troubleshooting](troubleshooting.md#common-failures).

## Reporting hardware results

Use synthetic data and report only the public station class, architecture, GPU
vendor/driver family, display count/scales, session, portal versions, and the
failing journey. Do not publish serial numbers, machine IDs, network
addresses, usernames, private paths, device addresses, or complete logs.
