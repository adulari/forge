#!/usr/bin/env python3
"""Procedurally synthesized soundtrack for the Forge promo videos.

Everything is generated from oscillators, filtered noise and envelopes —
no samples, no external audio, zero licensing risk.

Timing is derived from scripts/promo/src/timeline.json (the same file the
Remotion compositions import), so musical accents land exactly on scene cuts.

Usage:
    python3 make_audio.py            # writes normalized .m4a per cut into audio/
Requires: numpy, ffmpeg (for EBU R128 loudness normalization + AAC encode).
"""

import json
import math
import subprocess
import sys
import wave
from pathlib import Path

import numpy as np

HERE = Path(__file__).resolve().parent
TIMELINE = json.loads((HERE.parent / "src" / "timeline.json").read_text())

SR = 48000
FPS = TIMELINE["fps"]
BPM = 88.0
BEAT = 60.0 / BPM  # 0.6818 s
RNG = np.random.default_rng(4242)

# ---------------------------------------------------------------- helpers


def t_axis(dur):
    return np.arange(int(dur * SR)) / SR


def f2s(frame):
    """Composition frame -> seconds."""
    return frame / FPS


def add(buf, sig, at):
    """Mix `sig` into `buf` starting at time `at` (seconds), clipping to length."""
    i = int(at * SR)
    if i >= len(buf):
        return
    n = min(len(sig), len(buf) - i)
    buf[i : i + n] += sig[:n]


def env_exp(dur, decay):
    t = t_axis(dur)
    return np.exp(-t / decay)


def fade_edges(sig, a=0.004, r=0.01):
    n = len(sig)
    na, nr = min(int(a * SR), n // 2), min(int(r * SR), n // 2)
    if na > 0:
        sig[:na] *= np.linspace(0, 1, na)
    if nr > 0:
        sig[-nr:] *= np.linspace(1, 0, nr)
    return sig


def onepole_lp(sig, cutoff):
    """Simple one-pole lowpass (6 dB/oct); cutoff may be scalar or array."""
    if np.isscalar(cutoff):
        a = math.exp(-2.0 * math.pi * cutoff / SR)
        out = np.empty_like(sig)
        acc = 0.0
        one_minus = 1.0 - a
        # vectorized via lfilter-style recursion in chunks (numpy scan)
        # fall back to a fast loop in C via frompyfunc is slow; use scipy-free trick:
        # y[n] = (1-a) x[n] + a y[n-1]  ->  IIR; implement with cumulative product.
        # For stability and speed, do it blockwise in float64.
        y = np.zeros_like(sig)
        prev = 0.0
        b = one_minus
        # plain python loop is too slow for minutes of audio; use the
        # exponential-decay convolution identity instead:
        # y = b * sum_{k} a^k x[n-k]  -> filter with scipy... not available.
        # Efficient alternative: recursive doubling.
        y = recursive_iir(sig * b, a)
        return y
    else:
        # time-varying cutoff: process in 256-sample blocks with per-block coeff
        out = np.empty_like(sig)
        prev = 0.0
        block = 256
        for i in range(0, len(sig), block):
            c = float(np.mean(cutoff[i : i + block]))
            a = math.exp(-2.0 * math.pi * max(20.0, c) / SR)
            b = 1.0 - a
            seg = sig[i : i + block]
            y = recursive_iir(seg * b, a, prev)
            prev = y[-1] if len(y) else prev
            out[i : i + block] = y
        return out


def recursive_iir(x, a, y0=0.0):
    """y[n] = x[n] + a*y[n-1] via log-time recursive doubling (numpy only)."""
    y = x.astype(np.float64).copy()
    if y0 != 0.0:
        y[0] += a * y0
    coeff = a
    shift = 1
    while shift < len(y):
        y[shift:] += coeff * y[:-shift]
        coeff = coeff * coeff
        shift *= 2
    return y


def highpass(sig, cutoff):
    return sig - onepole_lp(sig, cutoff)


def bandnoise(dur, lo, hi):
    n = RNG.standard_normal(int(dur * SR))
    return highpass(onepole_lp(n, hi), lo)


# ---------------------------------------------------------------- voices


def pad_chord(freqs, dur, gain=1.0, attack=1.2, release=1.6, cutoff=900.0):
    """Warm detuned pad: sine stacks + a whiff of filtered noise air."""
    t = t_axis(dur)
    sig = np.zeros_like(t)
    for f in freqs:
        for det, g in ((0.9985, 0.5), (1.0, 1.0), (1.0018, 0.5)):
            ph = RNG.uniform(0, 2 * math.pi)
            sig += g * np.sin(2 * math.pi * f * det * t + ph)
            sig += 0.22 * g * np.sin(2 * math.pi * 2 * f * det * t + ph * 1.7)
    sig /= len(freqs) * 2.2
    # slow movement
    sig *= 1.0 + 0.12 * np.sin(2 * math.pi * 0.11 * t + 0.8)
    air = 0.05 * onepole_lp(RNG.standard_normal(len(t)), 600)
    sig = onepole_lp(sig + air, cutoff)
    na, nr = int(attack * SR), int(release * SR)
    env = np.ones_like(t)
    env[: min(na, len(t))] = np.linspace(0, 1, min(na, len(t)))
    if nr < len(t):
        env[-nr:] *= np.linspace(1, 0, nr)
    return sig * env * gain


def sub_thump(f=55.0, dur=0.5, gain=1.0):
    t = t_axis(dur)
    sweep = f * (1.0 + 0.6 * np.exp(-t / 0.02))
    ph = 2 * math.pi * np.cumsum(sweep) / SR
    return fade_edges(np.sin(ph) * np.exp(-t / 0.16) * gain)


def pluck(f, dur=0.5, gain=1.0, bright=0.35):
    t = t_axis(dur)
    sig = np.sin(2 * math.pi * f * t) + bright * np.sin(2 * math.pi * 2 * f * t) + 0.12 * np.sin(2 * math.pi * 3 * f * t)
    return fade_edges(sig * np.exp(-t / 0.22) * gain)


def whoosh(dur=0.75, gain=1.0, f_start=1400.0, f_end=240.0, updown=False):
    t = t_axis(dur)
    prog = t / dur
    if updown:
        f_start, f_end = f_end, f_start
    cut = f_start * (f_end / f_start) ** prog
    n = RNG.standard_normal(len(t))
    sig = onepole_lp(n, cut) - onepole_lp(n, cut * 0.35)
    env = np.sin(np.pi * np.clip(prog, 0, 1)) ** 1.5
    return fade_edges(sig * env * gain)


def spark(gain=1.0, fs=(3050.0, 4620.0, 6200.0), decay=0.055):
    """Tiny metallic tick — inharmonic partials + a click of noise."""
    dur = 0.22
    t = t_axis(dur)
    sig = np.zeros_like(t)
    for i, f in enumerate(fs):
        sig += (0.9 ** i) * np.sin(2 * math.pi * f * t + RNG.uniform(0, 6.28)) * np.exp(-t / (decay * (1 + 0.4 * i)))
    click = highpass(RNG.standard_normal(len(t)), 2500) * np.exp(-t / 0.006) * 0.8
    return fade_edges((sig * 0.8 + click) * gain)


def buzz429(gain=1.0):
    """Denied buzz: two short dark square-ish pulses."""
    out = np.zeros(int(0.5 * SR))
    for k in range(2):
        dur = 0.11
        t = t_axis(dur)
        f = 196.0
        sig = np.tanh(2.6 * np.sin(2 * math.pi * f * t) + 0.9 * np.sin(2 * math.pi * f * 1.5 * t))
        sig = onepole_lp(sig, 1400) * np.exp(-t / 0.09)
        add(out, fade_edges(sig * gain), k * 0.16)
    return out


def bell(f, dur=0.9, gain=1.0):
    t = t_axis(dur)
    sig = np.sin(2 * math.pi * f * t) * np.exp(-t / 0.28)
    sig += 0.4 * np.sin(2 * math.pi * f * 2.0 * t) * np.exp(-t / 0.16)
    sig += 0.18 * np.sin(2 * math.pi * f * 2.99 * t) * np.exp(-t / 0.1)
    return fade_edges(sig * gain)


def shimmer(dur=1.7, gain=1.0, f0=420.0, f1=1750.0):
    """Rising airy gliss for the wordmark ignition."""
    t = t_axis(dur)
    prog = t / dur
    f = f0 * (f1 / f0) ** prog
    ph = 2 * math.pi * np.cumsum(f) / SR
    tone = np.sin(ph) * 0.5 + 0.3 * np.sin(2.01 * ph)
    n = RNG.standard_normal(len(t))
    aircut = 900 + 5200 * prog
    air = onepole_lp(n, aircut) - onepole_lp(n, aircut * 0.4)
    env = np.sin(np.pi * np.clip(prog, 0, 1)) ** 1.2 * (0.25 + 0.75 * prog)
    return fade_edges((tone * 0.55 + air * 0.5) * env * gain)


# ---------------------------------------------------------------- music bed

# A-minor forge progression, low and warm.
A1, C2, D2, E2, F2, G2 = 55.0, 65.41, 73.42, 82.41, 87.31, 98.0
A2, C3, E3, F3, G3, B2 = 110.0, 130.81, 164.81, 174.61, 196.0, 123.47
CHORDS = [
    (A1, A2, C3, E3),          # Am
    (F2, F3, A2 * 2 / 2, C3),  # Fmaj
    (G2, G3, B2, D2 * 2),      # G
    (E2, E3, G3 * 0.5 * 2, B2),# Em
]

ARP_NOTES = [220.0, 261.63, 329.63, 392.0, 329.63, 261.63, 220.0, 329.63]  # A3 C4 E4 G4 …


def intensity_curve(total, sections):
    """sections: list of (t_start, level). Piecewise-linear ramps (1.2s)."""
    n = int(total * SR)
    curve = np.zeros(n)
    ramp = int(1.2 * SR)
    for i, (ts, lvl) in enumerate(sections):
        i0 = int(ts * SR)
        i1 = int(sections[i + 1][0] * SR) if i + 1 < len(sections) else n
        i0, i1 = min(i0, n), min(i1, n)
        curve[i0:i1] = lvl
    # smooth
    k = np.ones(ramp) / ramp
    curve = np.convolve(curve, k, mode="same")
    return curve


def music_bed(total, sections, arp_start, arp_end, fade_out=2.8):
    bed = np.zeros(int(total * SR))

    # pads: one chord per 8 beats, crossfaded by their own attack/release
    chord_len = 8 * BEAT
    t = 0.0
    ci = 0
    while t < total:
        dur = min(chord_len + 1.8, total - t + 1.8)
        add(bed, pad_chord(CHORDS[ci % len(CHORDS)], dur, gain=0.4), t)
        t += chord_len
        ci += 1

    # sub pulse on beats 1+3 of each bar (slow heartbeat)
    beat_i = 0
    t = 2 * BEAT
    while t < total - 1.0:
        if beat_i % 2 == 0:
            add(bed, sub_thump(gain=0.5), t)
        beat_i += 1
        t += 2 * BEAT

    # arp: 8th-note ember pluck with a 3/16 echo
    if arp_end > arp_start:
        arp = np.zeros(int(total * SR))
        step = BEAT / 2
        t = arp_start
        k = 0
        while t < arp_end:
            f = ARP_NOTES[k % len(ARP_NOTES)] * (2.0 if (k // len(ARP_NOTES)) % 4 == 3 else 1.0)
            add(arp, pluck(f, gain=0.16), t)
            k += 1
            t += step
        echo = int(3 * step / 2 * SR)
        arp[echo:] += 0.4 * arp[:-echo]
        bed += arp

    # intensity + master fade
    curve = intensity_curve(total, sections)
    bed *= 0.35 + 0.65 * curve
    n = len(bed)
    nf = int(fade_out * SR)
    bed[-nf:] *= np.linspace(1, 0, nf) ** 1.4
    nin = int(0.8 * SR)
    bed[:nin] *= np.linspace(0, 1, nin)
    return bed


# ---------------------------------------------------------------- cuts


def build_promo():
    sc = TIMELINE["promo"]["scenes"]
    ev = TIMELINE["promo"]["events"]
    total = f2s(sc["close"][0] + sc["close"][1])
    cuts = [f2s(sc[k][0]) for k in ("mesh", "tui", "proof", "feat", "remote", "close")]
    vig = [f2s(sc["feat"][0] + ev["vignette"] * i) for i in range(1, 5)]

    sections = [
        (0.0, 0.30),
        (f2s(sc["mesh"][0]), 0.48),
        (f2s(sc["tui"][0]), 0.55),
        (f2s(sc["proof"][0]), 0.66),
        (f2s(sc["feat"][0]), 0.85),
        (f2s(sc["remote"][0]), 0.70),
        (f2s(sc["close"][0]), 0.55),
        (total - 3.0, 0.40),
    ]
    mix = music_bed(total, sections, arp_start=f2s(sc["mesh"][0]), arp_end=f2s(sc["close"][0] + 170))

    # scene-cut whooshes
    for tcut in cuts:
        add(mix, whoosh(gain=0.30), tcut - 0.28)
    # vignette cuts: whoosh lite + tick
    for tcut in vig:
        add(mix, whoosh(dur=0.5, gain=0.20, f_start=1000, f_end=300), tcut - 0.18)
        add(mix, spark(gain=0.16), tcut)

    # cold open: spark flare + wordmark ignition shimmer
    add(mix, spark(gain=0.22), f2s(ev["sparkFlare"]))
    add(mix, shimmer(gain=0.30), f2s(ev["wordmarkIgnite"]) - 0.1)

    # 429 -> failover story beats (offsets are relative to mesh scene start)
    m0 = sc["mesh"][0]
    add(mix, buzz429(gain=0.30), f2s(m0 + ev["meshFlash429"]))
    add(mix, spark(gain=0.20), f2s(m0 + ev["meshFlash429"]))
    add(mix, whoosh(dur=0.6, gain=0.26, updown=True), f2s(m0 + ev["meshReroute"]) - 0.1)
    add(mix, bell(659.26, gain=0.20), f2s(m0 + ev["meshSuccess"]))
    add(mix, bell(880.0, gain=0.14), f2s(m0 + ev["meshSuccess"]) + 0.12)

    # tui submit tick, proof cards, duel winner, remote tap
    add(mix, spark(gain=0.14), f2s(sc["tui"][0] + ev["tuiSubmit"]))
    add(mix, spark(gain=0.14), f2s(sc["proof"][0] + ev["proofCards"]))
    add(mix, bell(523.25, gain=0.16), f2s(sc["feat"][0] + ev["duelWinner"]))
    add(mix, spark(gain=0.16), f2s(sc["remote"][0] + ev["remoteTap"]))
    add(mix, bell(659.26, gain=0.13), f2s(sc["remote"][0] + ev["remoteTap"]) + 0.7)

    # close: ignition shimmer + final resolve bell
    add(mix, shimmer(gain=0.30), f2s(sc["close"][0] + ev["closeIgnite"]) - 0.15)
    add(mix, bell(440.0, dur=1.6, gain=0.16), f2s(sc["close"][0] + ev["closeIgnite"]) + 1.1)
    return mix, total


def build_teaser():
    sc = TIMELINE["teaser"]["scenes"]
    ev = TIMELINE["teaser"]["events"]
    total = f2s(sc["close"][0] + sc["close"][1])
    sections = [
        (0.0, 0.32),
        (f2s(sc["mesh"][0]), 0.52),
        (f2s(sc["duel"][0]), 0.75),
        (f2s(sc["close"][0]), 0.55),
        (total - 2.8, 0.40),
    ]
    mix = music_bed(total, sections, arp_start=f2s(sc["mesh"][0]), arp_end=f2s(sc["close"][0] + 170))
    for k in ("mesh", "duel", "close"):
        add(mix, whoosh(gain=0.30), f2s(sc[k][0]) - 0.28)
    add(mix, spark(gain=0.22), f2s(26))
    add(mix, shimmer(gain=0.30), f2s(30) - 0.1)
    m0 = sc["mesh"][0]
    add(mix, buzz429(gain=0.30), f2s(m0 + ev["meshFlash429"]))
    add(mix, spark(gain=0.20), f2s(m0 + ev["meshFlash429"]))
    add(mix, whoosh(dur=0.6, gain=0.26, updown=True), f2s(m0 + ev["meshReroute"]) - 0.1)
    add(mix, bell(659.26, gain=0.20), f2s(m0 + ev["meshSuccess"]))
    add(mix, bell(523.25, gain=0.16), f2s(sc["duel"][0] + ev["duelWinner"]))
    add(mix, shimmer(gain=0.30), f2s(sc["close"][0] + ev["closeIgnite"]) - 0.15)
    add(mix, bell(440.0, dur=1.6, gain=0.16), f2s(sc["close"][0] + ev["closeIgnite"]) + 1.1)
    return mix, total


def build_vertical():
    sc = TIMELINE["vertical"]["scenes"]
    ev = TIMELINE["vertical"]["events"]
    total = f2s(sc["close"][0] + sc["close"][1])
    sections = [
        (0.0, 0.45),  # vertical opens hot — hook must land immediately
        (f2s(sc["mesh"][0]), 0.55),
        (f2s(sc["duel"][0]), 0.75),
        (f2s(sc["proof"][0]), 0.68),
        (f2s(sc["close"][0]), 0.55),
        (total - 2.6, 0.40),
    ]
    mix = music_bed(total, sections, arp_start=f2s(sc["mesh"][0]), arp_end=f2s(sc["close"][0] + 110))
    for k in ("mesh", "duel", "auto", "proof", "close"):
        add(mix, whoosh(gain=0.30), f2s(sc[k][0]) - 0.28)
    add(mix, shimmer(gain=0.32), f2s(ev["hookIgnite"]) - 0.05)
    m0 = sc["mesh"][0]
    add(mix, buzz429(gain=0.30), f2s(m0 + ev["meshFlash429"]))
    add(mix, spark(gain=0.20), f2s(m0 + ev["meshFlash429"]))
    add(mix, whoosh(dur=0.6, gain=0.26, updown=True), f2s(m0 + ev["meshReroute"]) - 0.1)
    add(mix, bell(659.26, gain=0.20), f2s(m0 + ev["meshSuccess"]))
    add(mix, bell(523.25, gain=0.16), f2s(sc["duel"][0] + ev["duelWinner"]))
    add(mix, shimmer(gain=0.30), f2s(sc["close"][0] + ev["closeIgnite"]) - 0.15)
    add(mix, bell(440.0, dur=1.6, gain=0.16), f2s(sc["close"][0] + ev["closeIgnite"]) + 1.1)
    return mix, total


# ---------------------------------------------------------------- output


def to_stereo(mono):
    """Subtle width: pad/noise slightly decorrelated via short haas on one side."""
    delay = int(0.011 * SR)
    left = mono.copy()
    right = mono.copy()
    right[delay:] = 0.88 * right[delay:] + 0.12 * mono[:-delay]
    return np.stack([left, right], axis=1)


def write_wav(path, stereo):
    peak = np.max(np.abs(stereo))
    if peak > 0:
        stereo = stereo / peak * 0.70  # headroom pre-loudnorm
    pcm = (stereo * 32767.0).astype(np.int16)
    with wave.open(str(path), "wb") as w:
        w.setnchannels(2)
        w.setsampwidth(2)
        w.setframerate(SR)
        w.writeframes(pcm.tobytes())


def loudnorm(src, dst, target_i=-14.0, target_tp=-1.2):
    measure = subprocess.run(
        ["ffmpeg", "-hide_banner", "-i", str(src), "-af",
         f"loudnorm=I={target_i}:TP={target_tp}:LRA=11:print_format=json", "-f", "null", "-"],
        capture_output=True, text=True,
    )
    j = measure.stderr[measure.stderr.rfind("{"):]
    j = j[: j.rfind("}") + 1]
    m = json.loads(j)
    af = (
        f"loudnorm=I={target_i}:TP={target_tp}:LRA=11:"
        f"measured_I={m['input_i']}:measured_TP={m['input_tp']}:"
        f"measured_LRA={m['input_lra']}:measured_thresh={m['input_thresh']}:"
        f"offset={m['target_offset']}:linear=true,"
        # safety limiter: loudnorm's dynamic fallback + AAC encoder overshoot
        # can push true peak past target; cap sample peak at -1.5 dBFS.
        "alimiter=limit=0.84:attack=1:release=60:level=false"
    )
    subprocess.run(
        ["ffmpeg", "-hide_banner", "-loglevel", "error", "-y", "-i", str(src),
         "-af", af, "-ar", str(SR), "-c:a", "aac", "-b:a", "128k", str(dst)],
        check=True,
    )
    print(f"  {dst.name}: measured I={m['input_i']} LUFS -> normalized to {target_i}")


def main():
    HERE.mkdir(exist_ok=True)
    for name, builder in (("promo", build_promo), ("teaser", build_teaser), ("vertical", build_vertical)):
        print(f"synthesizing {name}…")
        mono, total = builder()
        stereo = to_stereo(mono)
        raw = HERE / f"{name}-raw.wav"
        write_wav(raw, stereo)
        loudnorm(raw, HERE / f"{name}.m4a")
        raw.unlink()
    print("done.")


if __name__ == "__main__":
    sys.exit(main())
