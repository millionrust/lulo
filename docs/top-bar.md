# Top bar presentation domain

`rmac-top-bar` is the framework-neutral presentation boundary for the D1/D2
menu bar. It consumes the coherent `rmac-shell-status::Snapshot` already
published by `rmac-shell-runtime`; it owns no niri socket, D-Bus connection,
filesystem watcher, GPUI entity, or frame timer.

## Surface contract

The shell-status projection exposes only enabled outputs with valid positive
logical geometry and scale. Hardware make, model, physical dimensions, and
serial values do not cross this boundary. The top-bar projection defensively
revalidates and sorts that compact list, then describes one surface per output:

- 32 logical pixels high and the full logical output width;
- a 32-pixel exclusive zone on the top edge;
- no keyboard interactivity, so the bar cannot steal application focus;
- the compositor's exact stable output identity and scale.

The future renderer maps this description to a top-layer surface anchored top,
left, and right. The domain deliberately does not claim that a normal GPUI
window satisfies this contract.

## Content contract

The leading projection uses a bounded component of the stable focused app ID,
never the potentially private window title. An optional bounded workspace label
follows the versioned shell clock policy. The center clock supports explicit
12-hour and 24-hour modes, a locale-provided hour-cycle decision, date and
seconds visibility, and a complete accessible label.

Trailing indicators are projected in stable Focus, VPN, network, Bluetooth,
sound, battery, and notification order. Every visible fallback has a human
accessible state. Strength, volume, battery percentage, and badge values are
bounded; notification badges cap visually at `99+`; singular labels are
correct; disabled, unavailable, transitioning, muted, powered-off, urgent, and
empty states are not conflated. Network names and window titles are not used.
VPN and Focus names are bounded and their presentation structures redact text
from default debug output.

## Redraw and clock behavior

`State::apply` compares only renderer-visible projection. An identical runtime
snapshot requests no frame, including when only source diagnostics changed.
The clock returns exactly one deadline at the next minute boundary, or at the
next second boundary when the user explicitly enables seconds. It never asks
for a continuous frame loop.

`rmac-top-bar-runtime` is the process-facing coordinator. It waits until the
shell stream, locale subscription, and discontinuous-clock detector are all
ready before publishing its first projection. The locale adapter reads the
system `LC_TIME` hour directive directly instead of guessing from a country or
environment-variable spelling. If that authority becomes unavailable, the
runtime preserves the last accepted cycle; only cold-start failure uses the
documented 24-hour fallback.

After startup, shell changes replace the current coherent snapshot, locale and
timedated changes resample their authorities, and the sole one-shot timer is
an absolute Linux realtime deadline that advances during suspend. It is
cancelled and rearmed whenever the clock policy changes. Clock jumps, time-zone
changes, watcher reconnection, and resume therefore recalculate the deadline.
Only a changed presentation crosses to the renderer, so health-only events and
unchanged resamples do not request a frame.

## Remaining acceptance work

The product still needs the real upstream-GPUI layer-shell executable, original
icon assets, Quick Settings and Notification Center invocation/focus routing,
and semantic toolbar/status nodes. D1/D2 stay open until the Ubuntu/niri
reference PC proves placement, exclusive zone, mixed/fractional scale, focus,
fullscreen, hotplug, Orca, clock changes, service restarts, idle behavior, and
60/120 Hz performance.
