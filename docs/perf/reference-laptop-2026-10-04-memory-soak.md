# Reference laptop memory soak -- 2026-10-04T06:52:05Z

> **8-hour soak, release binaries from 2f3ea7a4, reference laptop (HP EliteBook, 6.7 GB RAM)**

Source samples: `~/rmac-coord/soak-2026-10-04/samples.jsonl`. Per-app budget: 128.0 MiB idle RSS, 16.0 MiB growth over 8h. Combined shell budget: 256.0 MiB RSS, 24.0 MiB growth over 8h. A sustained slope over 5.0%/hour (R^2 >= 0.5) is also flagged as a suspected leak -- growth only, never a falling trace. Where the samples have it, growth is also evaluated on the swap-aware private-footprint metric (Pss_Anon + SwapPss), the process's real private memory, which does not fall just because the kernel reclaimed idle pages under memory pressure.

## Applications (RSS)

| App | Samples | Start RSS | End RSS | Peak RSS | Growth/8h | Slope %/h | Leak? |
|---|---:|---:|---:|---:|---:|---:|---|
| calculator | 97 | 148.4 MiB | 6.3 MiB | 148.4 MiB | -142.0 MiB | -15.98 | no |
| clock | 97 | 185.5 MiB | 17.9 MiB | 185.5 MiB | -167.6 MiB | -15.06 | no |
| files | 97 | 190.5 MiB | 92.8 MiB | 259.6 MiB | -97.7 MiB | -11.66 | YES |
| notes | 97 | 183.2 MiB | 25.3 MiB | 183.2 MiB | -157.9 MiB | -14.35 | no |
| preview | 97 | 181.5 MiB | 27.9 MiB | 181.5 MiB | -153.6 MiB | -14.2 | no |
| system-monitor | 97 | 182.5 MiB | 20.8 MiB | 182.5 MiB | -161.7 MiB | -14.86 | no |
| system-settings | 97 | 216.5 MiB | 58.9 MiB | 216.9 MiB | -157.6 MiB | -12.38 | no |
| terminal | 97 | 160.5 MiB | 52.8 MiB | 168.8 MiB | -107.7 MiB | -12.21 | no |
| text-editor | 97 | 165.4 MiB | 37.5 MiB | 168.4 MiB | -127.9 MiB | -13.07 | no |
| weather | 97 | 175.3 MiB | 18.2 MiB | 175.3 MiB | -157.1 MiB | -14.99 | no |

## Applications (private footprint: Pss_Anon + SwapPss)

| App | Samples | Start | End | Peak | Growth/8h | Slope %/h | Leak? |
|---|---:|---:|---:|---:|---:|---:|---|
| calculator | 97 | 43.9 MiB | 44.0 MiB | 44.0 MiB | 0.0 MiB | 0.0 | no |
| clock | 97 | 47.6 MiB | 47.6 MiB | 47.6 MiB | 0.0 MiB | 0.0 | no |
| files | 97 | 53.2 MiB | 73.8 MiB | 73.8 MiB | 20.6 MiB | 2.99 | YES |
| notes | 97 | 46.1 MiB | 46.1 MiB | 46.1 MiB | 0.0 MiB | 0.0 | no |
| preview | 97 | 44.6 MiB | 44.6 MiB | 44.6 MiB | 0.0 MiB | 0.0 | no |
| system-monitor | 97 | 54.5 MiB | 54.5 MiB | 54.5 MiB | 0.0 MiB | 0.0 | no |
| system-settings | 97 | 52.2 MiB | 53.6 MiB | 53.6 MiB | 1.4 MiB | 0.32 | no |
| terminal | 97 | 45.3 MiB | 47.3 MiB | 47.3 MiB | 2.0 MiB | 0.04 | no |
| text-editor | 97 | 46.2 MiB | 48.1 MiB | 48.1 MiB | 1.9 MiB | 0.51 | no |
| weather | 97 | 45.0 MiB | 45.0 MiB | 45.0 MiB | 0.0 MiB | 0.0 | no |

## Shell pieces combined

Start 615.0 MiB, end 156.3 MiB, peak 631.5 MiB, growth/8h -458.6 MiB, slope -12.68%/h -- within budget.

## Leaks detected

- files
