#!/usr/bin/env python3
"""Compare the Luau weapon patches with the 1:1 GDScript references (weapons_ref.py).

For each case, renders the reference at 44.1 kHz and the patch with
`warp-audio render` at 48 kHz, over several seeds, and compares:
  duration   last sample above -60 dBFS (mean over seeds)
  peak       dBFS (mean over seeds)
  envelope   RMS in 10 ms windows: seed-averaged power, in dB. Reported as the
             mean |difference|, the mean signed difference (patch - reference,
             "bias") and the max |difference| over windows within 40 dB of the loudest.
  centroid   spectral centroid (0-20 kHz) of the seed-averaged power spectrum
             every 10 ms (40 ms Hann frames), over windows within 30 dB of the
             loudest; power-weighted mean |difference| in percent. Also the
             whole-sound centroid (0-20 kHz).
`spread` is the reference's own seed-to-seed standard deviation of the
per-window RMS (dB), for scale: an envelope gap below it is sampling noise.

The patch is rendered on the stereo layout (each channel at -3 dB) and scaled
back, which keeps it under the render mixer's -1 dBFS peak limiter, then
resampled to 44.1 kHz so both sides are measured in the same 0-22 kHz band.

`floor` columns: the same metrics for the reference against itself with a
different set of seeds. Every voice draws its own jitter, so this is how far
two honest renders of the same recipe differ with this many seeds; a patch gap
at or below the floor is not a real difference.

usage: compare_weapons.py [--seeds N] [--only NAME] [--markdown OUT.md] [--warp-audio PATH]
"""
import argparse
import math
import os
import subprocess
import sys
import tempfile
import wave

import numpy as np
from scipy.signal import resample_poly

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import weapons_ref  # noqa: E402

HERE = os.path.dirname(os.path.abspath(__file__))
SOUNDS = os.path.dirname(HERE)
REPO = os.path.abspath(os.path.join(SOUNDS, "..", "..", ".."))

# (label, patch file, patch params, reference function, reference kwargs)
CASES = [
    ("shot base", "shot", {"damage": 1, "accuracy": 0.87, "velocity": 1}, "shot", {"dmg_ratio": 1, "accuracy": 0.87, "velocity": 1}),
    ("shot heavy (dmg 3, acc 1)", "shot", {"damage": 3, "accuracy": 1, "velocity": 1}, "shot", {"dmg_ratio": 3, "accuracy": 1, "velocity": 1}),
    ("shot fast spray (acc .3, vel 3)", "shot", {"damage": 1, "accuracy": 0.3, "velocity": 3}, "shot", {"dmg_ratio": 1, "accuracy": 0.3, "velocity": 3}),
    ("shot bazooka (dmg 6, vel .5)", "shot", {"damage": 6, "accuracy": 0.5, "velocity": 0.5}, "shot", {"dmg_ratio": 6, "accuracy": 0.5, "velocity": 0.5}),
    ("grenade_launch", "grenade_launch", {}, "grenade_launch", {}),
    ("next_round", "next_round", {}, "next_round", {}),
    ("empty_chamber", "empty_chamber", {}, "empty_chamber", {}),
    ("reload 1.2 s", "reload", {"duration": 1.2}, "reload", {"duration": 1.2}),
    ("reload 0.4 s", "reload", {"duration": 0.4}, "reload", {"duration": 0.4}),
    ("reload 3.0 s", "reload", {"duration": 3.0}, "reload", {"duration": 3.0}),
    ("melee pr .71", "melee", {"pitch_ratio": 0.71}, "melee", {"pitch_ratio": 0.71}),
    ("melee pr 1.2", "melee", {"pitch_ratio": 1.2}, "melee", {"pitch_ratio": 1.2}),
    ("melee pr .5", "melee", {"pitch_ratio": 0.5}, "melee", {"pitch_ratio": 0.5}),
    ("casing_drop", "casing_drop", {}, "casing_drop", {}),
    ("bullet_zip sf 1", "bullet_zip", {"speed_factor": 1, "scale_factor": 1}, "bullet_zip", {"speed_factor": 1, "scale_factor": 1}),
    ("bullet_zip sf .2 sc 2", "bullet_zip", {"speed_factor": 0.2, "scale_factor": 2}, "bullet_zip", {"speed_factor": 0.2, "scale_factor": 2}),
    ("bullet_zip sf 4", "bullet_zip", {"speed_factor": 4, "scale_factor": 1}, "bullet_zip", {"speed_factor": 4, "scale_factor": 1}),
    ("bullet_zip sf 5 sc 4", "bullet_zip", {"speed_factor": 5, "scale_factor": 4}, "bullet_zip", {"speed_factor": 5, "scale_factor": 4}),
    ("impact dmg .5", "impact", {"dmg_ratio": 0.5}, "impact", {"dmg_ratio": 0.5}),
    ("impact dmg 1", "impact", {"dmg_ratio": 1}, "impact", {"dmg_ratio": 1}),
    ("impact dmg 3", "impact", {"dmg_ratio": 3}, "impact", {"dmg_ratio": 3}),
    ("impact dmg 6", "impact", {"dmg_ratio": 6}, "impact", {"dmg_ratio": 6}),
    ("explosion_bang r 2", "explosion_bang", {"radius": 2}, "explosion_bang", {"radius": 2}),
    ("explosion_bang r 6", "explosion_bang", {"radius": 6}, "explosion_bang", {"radius": 6}),
    ("explosion_bang r 24", "explosion_bang", {"radius": 24}, "explosion_bang", {"radius": 24}),
    ("explosion_rumble r 2", "explosion_rumble", {"radius": 2}, "explosion_rumble", {"radius": 2}),
    ("explosion_rumble r 6", "explosion_rumble", {"radius": 6}, "explosion_rumble", {"radius": 6}),
    ("explosion_rumble r 12", "explosion_rumble", {"radius": 12}, "explosion_rumble", {"radius": 12}),
    ("explosion_rumble r 24", "explosion_rumble", {"radius": 24}, "explosion_rumble", {"radius": 24}),
    ("ion_burst r 24", "ion_burst", {"radius": 24}, "ion_burst", {"radius": 24}),
    ("ion_burst r 38", "ion_burst", {"radius": 38}, "ion_burst", {"radius": 38}),
    ("ion_burst r 48", "ion_burst", {"radius": 48}, "ion_burst", {"radius": 48}),
]

WIN = 0.010


def q16(x):
    """Godot clamps and writes 16-bit WAV; so does warp-audio."""
    return np.round(np.clip(x, -1, 1) * 32767) / 32767


def read_wav(path):
    with wave.open(path, "rb") as w:
        sr, ch, n = w.getframerate(), w.getnchannels(), w.getnframes()
        x = np.frombuffer(w.readframes(n), dtype="<i2").astype(np.float64) / 32767
    return x.reshape(-1, ch)[:, 0] * (math.sqrt(2) if ch == 2 else 1.0), sr


def render_patch(exe, name, params, seed, tmp):
    out = os.path.join(tmp, f"{name}-{seed}.wav")
    cmd = [exe, "render", os.path.join(SOUNDS, f"{name}.luau"), "--seed", str(seed), "--layout", "stereo", "--out", out]
    for k, v in params.items():
        cmd += ["--set", f"{k}={v}"]
    subprocess.run(cmd, check=True, stdout=subprocess.DEVNULL)
    return read_wav(out)


def to_44k(x, sr):
    """Resample a 48 kHz render to 44.1 kHz (drops 22-24 kHz) to measure it in Godot's band."""
    return (resample_poly(x, 147, 160), 44100) if sr == 48000 else (x, sr)


def duration(x, sr):
    idx = np.nonzero(np.abs(x) > 1e-3)[0]
    return (idx[-1] + 1) / sr if len(idx) else 0.0


def frames(x, sr, nwin):
    """RMS power per 10 ms window, and a power spectrum per 10 ms hop from a
    40 ms Hann frame centred on that window (10 ms frames cannot resolve the
    sub-bass sounds), nwin windows (zero padded)."""
    w = int(round(WIN * sr))
    pad = np.zeros(nwin * w)
    pad[: min(len(x), nwin * w)] = x[: nwin * w]
    rms_pow = np.mean(pad.reshape(nwin, w) ** 2, axis=1)
    big = np.concatenate([np.zeros(int(1.5 * w)), pad, np.zeros(int(1.5 * w) + w)])
    fl = 4 * w
    f = np.stack([big[k * w: k * w + fl] for k in range(nwin)])
    spec = np.abs(np.fft.rfft(f * np.hanning(fl), axis=1)) ** 2
    freqs = np.fft.rfftfreq(fl, 1 / sr)
    return rms_pow, spec, freqs


BAND = 20000.0  # centroids over 0-20 kHz: the patch runs at 48 kHz, Godot at 44.1


def centroid(spec, freqs):
    keep = freqs <= BAND
    spec, freqs = spec[..., keep], freqs[keep]
    den = spec.sum(axis=-1)
    return np.where(den > 0, (spec * freqs).sum(axis=-1) / np.maximum(den, 1e-30), 0.0)


def analyse(sigs):
    """sigs: list of (x, sr). Seed-averaged stats. Duration and peak at the native
    rate; envelope and centroids in the 0-22 kHz band."""
    nwin = max(int(math.ceil(len(x) / sr / WIN)) for x, sr in sigs) + 1
    pows, specs, cents, durs, peaks = [], [], [], [], []
    for x0, sr0 in sigs:
        durs.append(duration(x0, sr0))
        peaks.append(20 * math.log10(max(np.max(np.abs(x0)), 1e-9)))
        x, sr = to_44k(x0, sr0)
        p, s, f = frames(x, sr, nwin)
        pows.append(p)
        specs.append(s)
        full = np.abs(np.fft.rfft(x)) ** 2
        cents.append(centroid(full, np.fft.rfftfreq(len(x), 1 / sr)))
    pows = np.array(pows)
    spec = np.mean(specs, axis=0)
    return {
        "pow": pows.mean(axis=0),
        "pow_db_seeds": 10 * np.log10(np.maximum(pows, 1e-12)),
        "cent_t": centroid(spec, f),
        "cent": float(np.mean(cents)),
        "dur": float(np.mean(durs)),
        "peak": float(np.mean(peaks)),
        "nwin": nwin,
    }


def compare(ref, pat):
    n = min(len(ref["pow"]), len(pat["pow"]))
    rdb = 10 * np.log10(np.maximum(ref["pow"][:n], 1e-12))
    pdb = 10 * np.log10(np.maximum(pat["pow"][:n], 1e-12))
    loud = rdb.max()
    m40 = rdb > loud - 40
    m30 = rdb > loud - 30
    sd = (pdb - rdb)[m40]
    d = np.abs(sd)
    spread = float(np.mean(np.std(ref["pow_db_seeds"][:, :n], axis=0)[m40]))
    rc, pc = ref["cent_t"][:n][m30], pat["cent_t"][:n][m30]
    wgt = ref["pow"][:n][m30]
    cdiff = float(np.sum(wgt * np.abs(pc - rc) / np.maximum(rc, 1)) / np.sum(wgt)) * 100
    return {"env_mean": float(d.mean()), "env_bias": float(sd.mean()), "env_max": float(d.max()), "spread": spread, "cent_t": cdiff}


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--seeds", type=int, default=24)
    ap.add_argument("--only")
    ap.add_argument("--markdown")
    # The workspace builds into a shared target dir (CARGO_TARGET_DIR); pass --warp-audio or WARP_AUDIO otherwise.
    ap.add_argument("--warp-audio", default=os.environ.get("WARP_AUDIO", os.path.join(REPO, "target", "debug", "warp-audio")))
    a = ap.parse_args()
    rows = []
    with tempfile.TemporaryDirectory() as tmp:
        for label, patch, pparams, ref_name, rkw in CASES:
            if a.only and a.only not in (patch, label):
                continue
            seeds = range(1, a.seeds + 1)
            refs = [(q16(weapons_ref.SOUNDS[ref_name](weapons_ref.Rng(1000 + s), **rkw)), weapons_ref.SR) for s in seeds]
            pats = [render_patch(a.warp_audio, patch, pparams, s, tmp) for s in seeds]
            refs2 = [(q16(weapons_ref.SOUNDS[ref_name](weapons_ref.Rng(5000 + s), **rkw)), weapons_ref.SR) for s in seeds]
            r, p, r2 = analyse(refs), analyse(pats), analyse(refs2)
            c = compare(r, p)
            fl = compare(r, r2)
            fl["dur"] = abs(r2["dur"] - r["dur"])
            fl["peak"] = abs(r2["peak"] - r["peak"])
            rows.append((label, r, p, c, fl))
            print(f"{label:34s} dur {r['dur']*1000:6.1f} / {p['dur']*1000:6.1f} [{fl['dur']*1000:4.1f}] ms  peak {r['peak']:5.1f} / {p['peak']:5.1f} dB  "
                  f"env |d| {c['env_mean']:4.1f} bias {c['env_bias']:+4.1f} (max {c['env_max']:4.1f}, spread {c['spread']:4.1f}) dB  "
                  f"centroid {r['cent']:5.0f} / {p['cent']:5.0f} Hz, over time {c['cent_t']:4.1f}%  | floor env {fl['env_mean']:4.1f} cent {fl['cent_t']:4.1f}%", flush=True)
    if a.markdown:
        with open(a.markdown, "w") as f:
            f.write("| case | duration ref / patch (ms) [floor] | peak ref / patch (dBFS) [floor] | RMS env mean abs / bias / max diff (dB) [floor mean] | centroid ref / patch (Hz) | centroid over time diff [floor] |\n")
            f.write("|---|---|---|---|---|---|\n")
            for label, r, p, c, fl in rows:
                f.write(f"| {label} | {r['dur']*1000:.1f} / {p['dur']*1000:.1f} [{fl['dur']*1000:.1f}] | {r['peak']:.1f} / {p['peak']:.1f} [{fl['peak']:.1f}] | "
                        f"{c['env_mean']:.2f} / {c['env_bias']:+.2f} / {c['env_max']:.1f} [{fl['env_mean']:.2f}] | {r['cent']:.0f} / {p['cent']:.0f} | {c['cent_t']:.1f}% [{fl['cent_t']:.1f}%] |\n")


if __name__ == "__main__":
    main()
