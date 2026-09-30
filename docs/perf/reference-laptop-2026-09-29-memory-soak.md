# Reference laptop memory soak -- 2026-09-30T01:26:26Z

> **Inconclusive for leak detection: the reference laptop was compiling all night during this run, so the kernel reclaimed idle pages under memory pressure and RSS/PSS fell sharply for every process. No crashes occurred (13/13 processes alive for all 97 samples) and no growth was observed, but these samples predate swap-aware sampling (VmSwap, Pss_Anon, SwapPss) and so cannot rule out a leak masked by swap-out. Rerun with swap-aware sampling on an idle machine.**

Source samples: `/home/jacob/rmac-coord/soak-2026-09-29/samples.jsonl`. Per-app budget: 128.0 MiB idle RSS, 16.0 MiB growth over 8h. Combined shell budget: 256.0 MiB RSS, 24.0 MiB growth over 8h. A sustained slope over 5.0%/hour (R^2 >= 0.5) is also flagged as a suspected leak -- growth only, never a falling trace. Where the samples have it, growth is also evaluated on the swap-aware private-footprint metric (Pss_Anon + SwapPss), the process's real private memory, which does not fall just because the kernel reclaimed idle pages under memory pressure.

## Applications (RSS)

| App | Samples | Start RSS | End RSS | Peak RSS | Growth/8h | Slope %/h | Leak? |
|---|---:|---:|---:|---:|---:|---:|---|
| calculator | 97 | 148.5 MiB | 6.5 MiB | 148.5 MiB | -142.0 MiB | -9.14 | YES |
| clock | 97 | 180.3 MiB | 5.9 MiB | 180.3 MiB | -174.4 MiB | -8.95 | YES |
| files | 97 | 187.1 MiB | 52.8 MiB | 191.2 MiB | -134.3 MiB | -8.55 | YES |
| notes | 97 | 176.7 MiB | 6.1 MiB | 176.7 MiB | -170.6 MiB | -9.04 | YES |
| preview | 97 | 176.3 MiB | 6.8 MiB | 176.3 MiB | -169.5 MiB | -8.86 | YES |
| system-monitor | 97 | 177.9 MiB | 5.9 MiB | 177.9 MiB | -172.0 MiB | -8.88 | YES |
| system-settings | 97 | 203.0 MiB | 53.0 MiB | 203.0 MiB | -150.0 MiB | -5.52 | YES |
| terminal | 97 | 161.7 MiB | 9.1 MiB | 161.8 MiB | -152.5 MiB | -9.2 | YES |
| text-editor | 97 | 162.6 MiB | 35.3 MiB | 162.6 MiB | -127.2 MiB | -7.74 | YES |
| weather | 97 | 169.2 MiB | 5.9 MiB | 169.2 MiB | -163.3 MiB | -9.15 | YES |

## Applications (private footprint: Pss_Anon + SwapPss)

No sample in this run recorded Pss_Anon/SwapPss (captured before run_memory_soak.py recorded them); growth is reported on RSS/PSS only above, which is not reliable evidence of no leak under memory pressure.

## Shell pieces combined

Start 602.8 MiB, end 134.0 MiB, peak 620.0 MiB, growth/8h -468.9 MiB, slope -6.81%/h -- LEAK SUSPECTED.

## Leaks detected

- calculator
- clock
- files
- notes
- preview
- system-monitor
- system-settings
- terminal
- text-editor
- weather
- dock
- top-bar
- wallpaper
- shell_combined
