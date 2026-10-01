# Reference laptop idle memory, 2026-10-01

This is a short idle footprint comparison, **not** an eight-hour leak test or
a release-budget pass. The 2026-09-29 soak ran during a compile and its
reclaimed RSS cannot establish a leak trend. This measurement reads
`/proc/<pid>/smaps_rollup` five seconds after launch, with one application at
a time. Values are MiB. `Pss` divides shared pages among their users;
`Pss_Anon` and `Private_Dirty` show the private resident portion more clearly
than RSS. Thread counts come from `/proc/<pid>/task`.

The private test uses headless Sway with Pixman, nested niri, a private D-Bus
session, and disposable HOME/XDG directories. The *client* Vulkan ICD is
either Intel `intel_hasvk` (verified as Intel HD Graphics 5500) or llvmpipe;
the nested compositor itself uses software rendering. The Intel baseline and
after run used the same existing app and shell binaries except the rebuilt
`wallpaper` and `osd` binaries. Some unchanged service binaries came from the
installed `/usr/libexec/rmac` tree because the shared `iterate` directory did
not contain them. The worktree baseline was `71516817`; the installed
service binaries' exact revision was not recorded. The changed after binaries
were built from `b1d6202a` on `cx/memory-diet`. The script is
[`scripts/behavior/measure_memory.py`](../../scripts/behavior/measure_memory.py).

## Changed processes and combined shell

| Renderer | Process | Before RSS | After RSS | Before PSS | After PSS | Before Pss_Anon | After Pss_Anon | Before Private_Dirty | After Private_Dirty |
|---|---|---:|---:|---:|---:|---:|---:|---:|---:|
| Intel | Wallpaper | 153.89 | 147.48 | 100.69 | 97.93 | 64.68 | 56.66 | 64.69 | 56.66 |
| Intel | OSD before first use | 106.57 | 17.47 | 52.57 | 14.40 | 25.41 | 2.79 | 25.42 | 2.79 |
| Intel | 12 sampled shell processes | 637.49 | 538.53 | 399.77 | 361.98 | 171.89 | 140.72 | 171.91 | 140.73 |
| Software | Wallpaper | 250.60 | 244.88 | 183.24 | 180.62 | 86.43 | 78.49 | 138.83 | 130.88 |
| Software | OSD before first use | 118.52 | 17.31 | 64.71 | 14.23 | 37.92 | 2.85 | 38.45 | 2.85 |
| Software | 12 sampled shell processes | 855.96 | 744.99 | 577.16 | 525.91 | 268.09 | 221.36 | 328.95 | 281.68 |

After the first synthetic OSD presentation, the Intel OSD was 102.87 MiB RSS,
51.97 MiB PSS and visibly painted in the private compositor. The software OSD
was 136.41 MiB RSS and 76.75 MiB PSS after first use. The deferred surface
allocation saves memory until OSD is used; it does not promise that warmed
OSD memory is returned after the popup hides. Both runs recorded zero SwapPss.

Wallpaper drops its decoded RGBA cache after the renderer owns the surfaces.
This saved about 8 MiB of private dirty memory in either renderer, consistent
with one full-size pixel copy. OSD's hidden surfaces were the larger idle
consumer. The Intel versus software samples also change `Pss_Anon` by tens
of MiB for GPUI apps, consistent with renderer and buffer allocations. The
sampler does not attribute those pages to a specific glyph atlas or GPU
allocation. Threads ranged from 25 to 45 per Intel app; thread count alone
does not establish committed stack size as a large contributor.

## Applications after the change

These are Intel client Vulkan samples. Preview opened a one-page PDF; Player
opened a 0.1-second silent WAV and was sampled after playback. Neither had a
second instance running. App Drawer is a resident shell service in this
measurement, not an open application window.

| App | RSS | PSS | Pss_Anon | Pss_File | Private_Dirty | Threads | 128 MiB RSS |
|---|---:|---:|---:|---:|---:|---:|---|
| Calculator | 108.45 | 52.13 | 22.52 | 29.34 | 22.52 | 25 | Pass |
| Clock | 111.28 | 56.74 | 25.16 | 31.31 | 25.17 | 26 | Pass |
| Files | 116.55 | 61.75 | 24.75 | 36.73 | 24.76 | 28 | Pass |
| Notes | 113.14 | 58.14 | 23.82 | 34.05 | 23.82 | 33 | Pass |
| Player | 160.17 | 96.79 | 36.08 | 60.45 | 36.09 | 39 | **Fail** |
| Preview | 122.82 | 67.98 | 34.69 | 33.03 | 34.69 | 25 | Pass |
| System Monitor | 118.03 | 63.46 | 30.97 | 32.22 | 30.97 | 29 | Pass |
| System Settings | 131.30 | 72.71 | 28.46 | 43.99 | 28.46 | 45 | **Fail** |
| Terminal | 112.14 | 57.96 | 23.41 | 34.29 | 23.41 | 27 | Pass |
| Text Editor | 112.18 | 57.42 | 23.12 | 34.03 | 23.13 | 26 | Pass |
| Weather | 108.50 | 53.83 | 22.76 | 30.80 | 22.77 | 25 | Pass |

Thus 9/11 opened app windows meet the idle RSS budget on this Intel path.
With llvmpipe, all 11 opened apps exceed 128 MiB RSS (151.18–207.70 MiB).
The combined shell still exceeds 256 MiB RSS under either renderer, even
though its Intel PSS is lower than summed RSS. The release memory gate remains
**failed**. A quiescent eight-hour swap-aware soak and a full installed
release-profile rerun are still needed.

## Read-only view of the owner's installed session

An independent `/proc` snapshot included all 23 processes in the current
`rmac-*.service` cgroups, including top bar, dock, wallpaper, mission control,
notification centre and panel, quick settings, OSD, launcher, app drawer,
file chooser, focus, screenshot, shortcut broker, clipboard, session
supervisor, idle locker and lock coordinator, plus their observed children.
It read no input or control socket and sent no signal. Together they had
150.97 MiB resident RSS, 76.40 MiB resident PSS, 42.03 MiB Pss_Anon,
34.31 MiB Pss_File, 41.53 MiB Private_Dirty, and **202.27 MiB SwapPss**.
Heavy swapping makes the small resident RSS unsuitable as proof of a budget
pass; PSS plus SwapPss was 278.67 MiB. The modified binaries were not
installed in this live session.

## Validation

The affected wallpaper crates passed 37/37 unit tests, and the two changed
shell binaries built under the shared Cargo target. Package-scoped root and
shell clippy each passed with `-D warnings`; root and shell
`cargo fmt --all -- --check` passed. The complete app behavior suite passed
111/111 scenarios. Nested niri window movement passed 19/19 checks, frame
and wallpaper repaint passed 15/15, power dialogs passed 51/51, and shutdown
completion passed 38/38, with the latter two using fake `systemctl` only.
The synthetic first-use OSD screenshot changed in both Intel and software
runs. Idle CPU and an eight-hour swap-aware soak were not rerun.
