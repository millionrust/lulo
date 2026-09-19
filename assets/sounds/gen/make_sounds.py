#!/usr/bin/env python3
"""Generate rmac's original system sound set (FEEL_SPEC.md §D.1).

Every cue is synthesised from scratch here, so the set is provably original:
no Apple sample, no third-party recording, no external asset. Pure standard
library — no numpy, no scipy, no ffmpeg required.

Usage:
    python3 assets/sounds/gen/make_sounds.py [output_dir]

Default output: assets/sounds/ (16-bit PCM WAV, 48 kHz, mono).
Package them to /usr/share/rmac/sounds/. Convert to OGG at build time if you
want smaller files:
    for f in assets/sounds/*.wav; do oggenc -q4 "$f"; done

Design rules baked in:
  * Everything is short. The longest non-login cue is 220 ms.
  * Everything ends in silence (no click at the tail): each cue is faded out
    over its final 8 ms.
  * Peak level is normalised per cue to the dBFS in SOUNDS, so no cue is
    startling next to another.
  * Nothing is a melody. These are confirmations, not notifications from a toy.
"""

from __future__ import annotations

import math
import random
import struct
import sys
import wave
from pathlib import Path

RATE = 48_000
FADE_MS = 8.0


# --- tiny signal helpers -------------------------------------------------

def silence(ms: float) -> list[float]:
    return [0.0] * int(RATE * ms / 1000.0)


def sine(freq: float, ms: float, decay: float | None = None,
         attack_ms: float = 2.0) -> list[float]:
    """Sine partial with an exponential decay (tau in ms) and a soft attack."""
    n = int(RATE * ms / 1000.0)
    attack = max(1, int(RATE * attack_ms / 1000.0))
    out = []
    for i in range(n):
        t = i / RATE
        env = 1.0
        if decay is not None:
            env = math.exp(-t / (decay / 1000.0))
        if i < attack:
            env *= i / attack
        out.append(math.sin(2.0 * math.pi * freq * t) * env)
    return out


def sweep(f0: float, f1: float, ms: float, decay: float | None = None) -> list[float]:
    """Linear frequency sweep — used for the mount/unmount and unlock cues."""
    n = int(RATE * ms / 1000.0)
    out = []
    phase = 0.0
    attack = max(1, int(RATE * 0.004))
    for i in range(n):
        frac = i / max(1, n - 1)
        freq = f0 + (f1 - f0) * frac
        phase += 2.0 * math.pi * freq / RATE
        env = 1.0
        if decay is not None:
            env = math.exp(-(i / RATE) / (decay / 1000.0))
        if i < attack:
            env *= i / attack
        out.append(math.sin(phase) * env)
    return out


def noise(ms: float, low: float, high: float, decay: float,
          seed: int = 7) -> list[float]:
    """Band-passed white noise (two-pole state-variable filter)."""
    rng = random.Random(seed)
    n = int(RATE * ms / 1000.0)
    f = 2.0 * math.sin(math.pi * math.sqrt(low * high) / RATE)
    q = 1.0 / max(0.5, math.sqrt(high / low))
    low_s = band = 0.0
    out = []
    for i in range(n):
        x = rng.uniform(-1.0, 1.0)
        high_s = x - low_s - q * band
        band += f * high_s
        low_s += f * band
        env = math.exp(-(i / RATE) / (decay / 1000.0))
        out.append(band * env)
    return out


def mix(*layers: list[float]) -> list[float]:
    n = max(len(l) for l in layers)
    out = [0.0] * n
    for layer in layers:
        for i, v in enumerate(layer):
            out[i] += v
    return out


def seq(*parts: list[float]) -> list[float]:
    out: list[float] = []
    for p in parts:
        out.extend(p)
    return out


def finish(samples: list[float], peak_dbfs: float) -> list[float]:
    """Fade the tail, then normalise to the requested peak."""
    fade = int(RATE * FADE_MS / 1000.0)
    n = len(samples)
    for i in range(max(0, n - fade), n):
        samples[i] *= (n - i) / fade
    peak = max(abs(v) for v in samples) or 1.0
    target = 10.0 ** (peak_dbfs / 20.0)
    gain = target / peak
    return [v * gain for v in samples]


def write_wav(path: Path, samples: list[float]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    frames = b"".join(
        struct.pack("<h", max(-32768, min(32767, int(v * 32767.0))))
        for v in samples
    )
    with wave.open(str(path), "wb") as handle:
        handle.setnchannels(1)
        handle.setsampwidth(2)
        handle.setframerate(RATE)
        handle.writeframes(frames)


# --- the cues ------------------------------------------------------------
# (name, peak dBFS, builder) — recipes come from FEEL_SPEC.md §D.1.

def alert() -> list[float]:
    return mix(sine(880, 90, decay=45), [v * 0.45 for v in sine(1320, 90, decay=35)])


def error() -> list[float]:
    return seq(sine(660, 70, decay=40), silence(20), sine(495, 90, decay=50))


def trash() -> list[float]:
    return mix(noise(140, 2000, 6000, decay=55),
               [v * 0.3 for v in noise(140, 700, 1500, decay=40, seed=11)])


def empty_trash() -> list[float]:
    return mix(noise(220, 1800, 6500, decay=90),
               [v * 0.5 for v in sine(120, 200, decay=70, attack_ms=1.0)])


def screenshot() -> list[float]:
    return seq(noise(14, 1500, 8000, decay=5), silence(12),
               noise(34, 1200, 7000, decay=12, seed=23))


def volume_tick() -> list[float]:
    return sine(1000, 18, decay=9, attack_ms=1.0)


def mount() -> list[float]:
    return sweep(523, 784, 110, decay=70)


def unmount() -> list[float]:
    return sweep(784, 523, 110, decay=70)


def power_plug() -> list[float]:
    return seq(sine(392, 70, decay=60, attack_ms=6.0),
               sine(587, 110, decay=70, attack_ms=6.0))


def lock() -> list[float]:
    return mix(sine(200, 40, decay=18, attack_ms=1.0),
               [v * 0.4 for v in noise(40, 300, 1200, decay=14)])


def unlock() -> list[float]:
    return sweep(300, 600, 180, decay=110)


def notification() -> list[float]:
    return mix(sine(1046, 220, decay=110, attack_ms=4.0),
               [v * 0.35 for v in sine(1568, 200, decay=80, attack_ms=4.0)])


def drag_drop() -> list[float]:
    return sine(1400, 25, decay=12, attack_ms=1.0)


def login() -> list[float]:
    return mix(sine(220, 900, decay=520, attack_ms=120.0),
               [v * 0.55 for v in sine(330, 900, decay=470, attack_ms=140.0)],
               [v * 0.35 for v in sine(440, 900, decay=420, attack_ms=160.0)])


SOUNDS = [
    ("alert", -12.0, alert),
    ("error", -12.0, error),
    ("trash", -18.0, trash),
    ("empty-trash", -16.0, empty_trash),
    ("screenshot", -14.0, screenshot),
    ("volume-tick", -20.0, volume_tick),
    ("mount", -16.0, mount),
    ("unmount", -16.0, unmount),
    ("power-plug", -16.0, power_plug),
    ("lock", -18.0, lock),
    ("unlock", -18.0, unlock),
    ("notification", -15.0, notification),
    ("drag-drop", -20.0, drag_drop),
    ("login", -16.0, login),
]


def main() -> int:
    out_dir = Path(sys.argv[1]) if len(sys.argv) > 1 else Path("assets/sounds")
    for name, peak, builder in SOUNDS:
        samples = finish(builder(), peak)
        path = out_dir / f"{name}.wav"
        write_wav(path, samples)
        print(f"{path}  {len(samples) / RATE * 1000:6.1f} ms  peak {peak:+.1f} dBFS")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
