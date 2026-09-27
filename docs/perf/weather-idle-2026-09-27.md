# Weather idle probe — 2026-09-27

Two bounded Lulo probes used the same `rmac-weather` candidate binary
(SHA-256 `8c7332622f3a06cf424bd9bd1a10bb5be0fb6cc86cba2ecf880f802417b22ea6`),
each with a 10-second settle period and a 60-second idle sample. The benchmark
gave each launch private HOME and XDG directories and injected no live input.

The empty first-run state measured **0.70% CPU**, 6.417 wake-ups/s, and 50.4
MiB PSS. It has no saved cities and requests focus for the city search field.
The one-city state used a valid forecast cached with a current `fetched_at`
timestamp; it measured **0.033% CPU**, 0.333 wake-ups/s, and 51.9 MiB PSS.
Warm launch was 430.9 ms and 369.5 ms, respectively. The complete metrics and
setup are in [weather-idle-2026-09-27.json](weather-idle-2026-09-27.json).

The result points to first-run search focus as the source of the higher idle
cost. Weather's search field uses `gpui-component`'s `InputState`, which blinks
its caret every 500 ms while focused. The saved-city result stays below the
0.3% app budget despite the minute-aligned local-time notification, so the
minute tick does not explain the 0.70% empty-state result. Each state was
sampled once, so the pair is directional evidence. No UI change was made:
first-run autofocus is intentional and removing it would change the keyboard
flow.
