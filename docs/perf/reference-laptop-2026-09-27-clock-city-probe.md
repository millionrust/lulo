# Reference laptop Clock idle probe -- 2026-09-27

This paired probe isolates the cost of one visible World Clock city on Lulo
(i5-5300U / HD 5500, Ubuntu 26.04, niri). Both runs used the same release
binary, started in World view, and received no injected input. Each app launch
used a private temporary HOME and XDG config/data/state/cache directories. The
runs were sequential under /tmp/lulo-journey.lock; no Cargo or rustc
processes ran.

| Private saved state | Idle CPU | Wake-ups/s | PSS |
|---|---:|---:|---:|
| Missing state; Clock initialized its default local city | 0.550% | 3.117 | 58.6 MiB |
| cities: []; no city cards or second hands | 0.433% | 3.117 | 63.6 MiB |
| Budget | <=0.3% | <=0.5/s threshold | — |

The empty-city run retained Clock's World-tab ticker, which wakes once per
second and calls cx.notify() because World always needs a redraw. Removing
the one visible city reduced measured CPU by only 0.117 percentage points,
while wake-ups stayed flat. This points to work shared by the World view/tick
as a larger cost than the one city face; this probe does not profile the frame
or identify the remaining CPU's call stack. Both runs remain over the 0.3% app
budget.

This is a focused one-city versus zero-city probe, not a replacement for the
nine-app candidate release table. Its one-city measurement is close to the
2026-09-26 focused Clock result (0.467% CPU, 3.200 wake-ups/s), and differs
from the candidate table's 1.87% / 12.417/s. The source binary identity and
raw measurements are in [the JSON report](reference-laptop-2026-09-27-clock-city-probe.json).

The binary was ~/rmac-release/inputs-20260926T2339/rmac-clock,
SHA-256 2f29b3bb578f5279e1f84184bbdf39389422cced68ce6c9a3f8d54cbddec9c7a.
The run used one idle sample per state, with a 3-second settle and 60-second
idle window:

    python3 /tmp/rmac-measure-clock.py --skip-surfaces --app rmac-clock --warmups 0 --repetitions 1 --settle-seconds 3 --idle-seconds 60 --binary-dir ~/rmac-release/inputs-20260926T2339

/tmp/rmac-measure-clock.py was a temporary copy of
scripts/linux/measure-budgets.py. For the zero-city run only, its
app_environment seeded the private config file rmac/clock.json with
{"cities":[],"alarms":[],"timers":[],"next_id":0} before launching the
app. The one-city run used the unmodified default-state path.
