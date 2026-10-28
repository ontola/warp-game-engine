#!/usr/bin/env python3
"""Compare the Luau strike, movement and UI patches with the 1:1 GDScript references (ui_ref.py).

Same method and metrics as compare_weapons.py (whose analysis functions this
imports): per case, the reference renders at 44.1 kHz and the patch with
`warp-audio render` at 48 kHz, over many seeds; duration, peak, the seed-averaged
RMS envelope in 10 ms windows, the spectral centroid over time and the
whole-sound centroid, each against a floor (the reference against itself with a
second set of seeds).

Extra columns:
  wave err   for deterministic recipes (no random draws in Godot): the RMS of
             (patch resampled to 44.1 kHz - reference) relative to the reference
             RMS, in dB. Below about -40 dB the two are the same waveform.
  bands      for the lava sizzle (--bands, on by default for it): the RMS envelope
             of the low band (< 400 Hz: the bubble bed) and the high band
             (> 2 kHz: the hiss) separately, and the depth of the ~30 Hz bubble
             amplitude modulation (power of the low-band envelope at 20-40 Hz
             over its mean), seed-averaged.

usage: compare_ui.py [--seeds N] [--only NAME] [--markdown OUT.md] [--warp-audio PATH]
"""
import argparse
import math
import os
import subprocess
import sys
import tempfile

import numpy as np

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import ui_ref  # noqa: E402
from compare_weapons import analyse, compare, q16, read_wav, to_44k  # noqa: E402

SOUNDS = os.path.dirname(HERE)
REPO = os.path.abspath(os.path.join(SOUNDS, "..", "..", ".."))

# (label, patch file, sound name or None, patch params, reference function, reference kwargs)
CASES = [
    ("air_strike_inbound 2.25 s", "air_strike_inbound", None, {"travel_seconds": 2.25}, "air_strike_inbound", {"travel_seconds": 2.25}),
    ("air_strike_inbound 1.75 s", "air_strike_inbound", None, {"travel_seconds": 1.75}, "air_strike_inbound", {"travel_seconds": 1.75}),
    ("air_strike_inbound 6.5 s", "air_strike_inbound", None, {"travel_seconds": 6.5}, "air_strike_inbound", {"travel_seconds": 6.5}),
    ("air_strike_inbound distant", "air_strike_inbound", None, {"travel_seconds": 2.25, "distant": 1}, "air_strike_inbound", {"travel_seconds": 2.25, "distant": True}),
    ("ion_cannon_charge 3.8 s", "ion_cannon_charge", None, {"charge_seconds": 3.8}, "ion_cannon_charge", {"charge_seconds": 3.8}),
    ("ion_cannon_charge 2.5 s", "ion_cannon_charge", None, {"charge_seconds": 2.5}, "ion_cannon_charge", {"charge_seconds": 2.5}),
    ("ion_cannon_charge 6.5 s", "ion_cannon_charge", None, {"charge_seconds": 6.5}, "ion_cannon_charge", {"charge_seconds": 6.5}),
    ("rocket_descent", "rocket_descent", None, {}, "rocket_descent", {}),
    ("mine_plant", "mine_plant", None, {}, "mine_plant", {}),
    ("dash", "dash", None, {}, "dash", {}),
    ("lava_sizzle contact", "lava_sizzle", None, {"fall_death": 0}, "lava_sizzle", {"fall_death": False}),
    ("lava_sizzle fall", "lava_sizzle", None, {"fall_death": 1}, "lava_sizzle", {"fall_death": True}),
    ("hit_received", "hit_received", None, {}, "hit_received", {}),
    ("kill_confirm", "kill_confirm", None, {}, "kill_confirm", {}),
    ("hitmarker body r 1", "hitmarker", None, {"dmg_ratio": 1}, "hitmarker", {"kind": "body", "dmg_ratio": 1.0}),
    ("hitmarker body r 2", "hitmarker", None, {"dmg_ratio": 2}, "hitmarker", {"kind": "body", "dmg_ratio": 2.0}),
    ("hitmarker body r 5", "hitmarker", None, {"dmg_ratio": 5}, "hitmarker", {"kind": "body", "dmg_ratio": 5.0}),
    ("hitmarker head r 1", "hitmarker", "hitmarker_head", {"dmg_ratio": 1}, "hitmarker", {"kind": "head", "dmg_ratio": 1.0}),
    ("hitmarker head r 2", "hitmarker", "hitmarker_head", {"dmg_ratio": 2}, "hitmarker", {"kind": "head", "dmg_ratio": 2.0}),
    ("hitmarker head r .5", "hitmarker", "hitmarker_head", {"dmg_ratio": 0.5}, "hitmarker", {"kind": "head", "dmg_ratio": 0.5}),
    ("hitmarker kill r 1", "hitmarker", "hitmarker_kill", {"dmg_ratio": 1}, "hitmarker", {"kind": "kill", "dmg_ratio": 1.0}),
    ("pick beep 3 s left", "pick_countdown_beep", None, {"seconds_left": 3}, "pick_beep", {"seconds_left": 3}),
    ("pick beep 2 s left", "pick_countdown_beep", None, {"seconds_left": 2}, "pick_beep", {"seconds_left": 2}),
    ("pick beep 1 s left", "pick_countdown_beep", None, {"seconds_left": 1}, "pick_beep", {"seconds_left": 1}),
    ("pling 1.15", "pling", None, {"pitch_ratio": 1.15}, "pling", {"pitch_ratio": 1.15}),
    ("pling 1", "pling", None, {"pitch_ratio": 1}, "pling", {"pitch_ratio": 1.0}),
    ("card_flip", "card_flip", None, {}, "card_flip", {}),
    ("pickup_drop", "pickup_drop", None, {}, "pickup_drop", {}),
    ("enemy_incoming", "enemy_incoming", None, {}, "enemy_incoming", {}),
]

# Godot recipes without random draws: one seed is enough, and the waveform can be compared.
DETERMINISTIC = {"mine_plant", "kill_confirm", "pick_beep", "pling", "pickup_drop"}
# Short sounds are cheap, and their seed-averaged envelopes are dominated by the
# slowest-decaying draws, so they get more seeds.
SEED_MULT = {"dash": 4, "hit_received": 4, "hitmarker": 4, "card_flip": 4, "enemy_incoming": 4,
             "rocket_descent": 2, "lava_sizzle": 2}
BANDS = {"lava_sizzle"}


def render_patch(exe, name, sound, params, seed, tmp):
    out = os.path.join(tmp, f"{name}-{sound}-{seed}.wav")
    cmd = [exe, "render", os.path.join(SOUNDS, f"{name}.luau"), "--seed", str(seed), "--layout", "stereo", "--out", out]
    if sound:
        cmd += ["--sound", sound]
    for k, v in params.items():
        cmd += ["--set", f"{k}={v}"]
    subprocess.run(cmd, check=True, stdout=subprocess.DEVNULL)
    return read_wav(out)


def wave_err(ref, pat):
    """RMS(patch - ref) / RMS(ref) in dB, the patch resampled to 44.1 kHz."""
    x, _ = to_44k(*pat)
    n = max(len(ref), len(x))
    a, b = np.zeros(n), np.zeros(n)
    a[: len(ref)] = ref
    b[: len(x)] = x
    return 10 * math.log10(max(np.mean((b - a) ** 2), 1e-20) / max(np.mean(a ** 2), 1e-20))


def band_stats(sigs, nwin):
    """Seed-averaged 10 ms power envelopes of < 400 Hz and > 2 kHz, and the ~30 Hz AM depth of the low band."""
    lows, highs, am = [], [], []
    for x0, sr0 in sigs:
        x, sr = to_44k(x0, sr0)
        w = int(round(0.010 * sr))
        pad = np.zeros(nwin * w)
        pad[: min(len(x), nwin * w)] = x[: nwin * w]
        X = np.fft.rfft(pad)
        f = np.fft.rfftfreq(len(pad), 1 / sr)
        lo = np.fft.irfft(X * (f < 400), len(pad))
        hi = np.fft.irfft(X * (f > 2000), len(pad))
        lows.append(np.mean(lo.reshape(nwin, w) ** 2, axis=1))
        highs.append(np.mean(hi.reshape(nwin, w) ** 2, axis=1))
        # Bubble AM: envelope of the 60-250 Hz band (the bed, below the AM sidebands
        # of the hiss), its spectrum at 20-40 Hz relative to its mean.
        bed = np.fft.irfft(X * ((f > 60) & (f < 250)), len(pad))
        n_sig = len(x)
        e = np.abs(bed[:n_sig]) ** 2
        k = int(sr / 400)  # smooth over 2.5 ms
        e = np.convolve(e, np.ones(k) / k, mode="same")
        seg = e[int(0.05 * sr): int(0.9 * n_sig)]
        if len(seg) < sr // 10:
            am.append(0.0)
            continue
        seg = seg - np.linspace(seg[0], seg[-1], len(seg))  # remove the slow trend
        E = np.abs(np.fft.rfft(seg * np.hanning(len(seg)))) ** 2
        fe = np.fft.rfftfreq(len(seg), 1 / sr)
        am.append(float(np.sum(E[(fe > 20) & (fe < 40)]) / max(np.sum(E[(fe > 1) & (fe < 200)]), 1e-30)))
    return np.mean(lows, axis=0), np.mean(highs, axis=0), float(np.mean(am))


def env_diff(rp, pp):
    rdb = 10 * np.log10(np.maximum(rp, 1e-12))
    pdb = 10 * np.log10(np.maximum(pp, 1e-12))
    m = rdb > rdb.max() - 40
    d = (pdb - rdb)[m]
    return float(np.mean(np.abs(d))), float(np.mean(d))


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--seeds", type=int, default=24)
    ap.add_argument("--only")
    ap.add_argument("--markdown")
    ap.add_argument("--warp-audio", default=os.environ.get("WARP_AUDIO", os.path.join(REPO, "target", "debug", "warp-audio")))
    a = ap.parse_args()
    rows, bands = [], []
    with tempfile.TemporaryDirectory() as tmp:
        for label, patch, sound, pparams, ref_name, rkw in CASES:
            if a.only and a.only not in (patch, label, ref_name):
                continue
            det = ref_name in DETERMINISTIC
            nseed = 2 if det else a.seeds * SEED_MULT.get(patch, 1)
            seeds = range(1, nseed + 1)
            fn = ui_ref.SOUNDS[ref_name]
            refs = [(q16(fn(ui_ref.Rng(1000 + s), **rkw)), ui_ref.SR) for s in seeds]
            pats = [render_patch(a.warp_audio, patch, sound, pparams, s, tmp) for s in seeds]
            refs2 = [(q16(fn(ui_ref.Rng(5000 + s), **rkw)), ui_ref.SR) for s in seeds]
            r, p, r2 = analyse(refs), analyse(pats), analyse(refs2)
            c = compare(r, p)
            fl = compare(r, r2)
            fl["dur"] = abs(r2["dur"] - r["dur"])
            fl["peak"] = abs(r2["peak"] - r["peak"])
            fl["seeds"] = len(seeds)
            werr = wave_err(refs[0][0], pats[0]) if det else None
            rows.append((label, r, p, c, fl, werr))
            we = f"  wave err {werr:6.1f} dB" if det else ""
            print(f"{label:28s} dur {r['dur']*1000:6.1f} / {p['dur']*1000:6.1f} [{fl['dur']*1000:4.1f}] ms  peak {r['peak']:5.1f} / {p['peak']:5.1f} [{fl['peak']:3.1f}] dB  "
                  f"env |d| {c['env_mean']:4.2f} bias {c['env_bias']:+5.2f} (max {c['env_max']:4.1f}) [{fl['env_mean']:4.2f}] dB  "
                  f"centroid {r['cent']:5.0f} / {p['cent']:5.0f} Hz, over time {c['cent_t']:4.1f}% [{fl['cent_t']:4.1f}%]{we}", flush=True)
            if patch in BANDS:
                nwin = max(r["nwin"], p["nwin"])
                rl, rh, ram = band_stats(refs, nwin)
                pl, ph, pam = band_stats(pats, nwin)
                ql, qh, qam = band_stats(refs2, nwin)
                b = (label, env_diff(rl, pl), env_diff(rl, ql), env_diff(rh, ph), env_diff(rh, qh), ram, pam, qam,
                     10 * math.log10(np.sum(pl) / np.sum(rl)), 10 * math.log10(np.sum(ph) / np.sum(rh)))
                bands.append(b)
                print(f"   bands: low <400 Hz env |d| {b[1][0]:4.2f} bias {b[1][1]:+5.2f} [{b[2][0]:4.2f}], energy {b[8]:+5.2f} dB;  "
                      f"high >2 kHz env |d| {b[3][0]:4.2f} bias {b[3][1]:+5.2f} [{b[4][0]:4.2f}], energy {b[9]:+5.2f} dB;  "
                      f"30 Hz AM depth ref {ram:.3f} / patch {pam:.3f} [ref2 {qam:.3f}]", flush=True)
    if a.markdown:
        with open(a.markdown, "w") as f:
            f.write("| case | seeds | duration ref / patch (ms) [floor] | peak ref / patch (dBFS) [floor] | RMS env mean abs / bias / max diff (dB) [floor mean] | centroid ref / patch (Hz) | centroid over time diff [floor] | wave err (dB) |\n")
            f.write("|---|---|---|---|---|---|---|---|\n")
            for label, r, p, c, fl, werr in rows:
                we = f"{werr:.1f}" if werr is not None else "-"
                f.write(f"| {label} | {fl['seeds']} | {r['dur']*1000:.1f} / {p['dur']*1000:.1f} [{fl['dur']*1000:.1f}] | {r['peak']:.1f} / {p['peak']:.1f} [{fl['peak']:.1f}] | "
                        f"{c['env_mean']:.2f} / {c['env_bias']:+.2f} / {c['env_max']:.1f} [{fl['env_mean']:.2f}] | {r['cent']:.0f} / {p['cent']:.0f} | {c['cent_t']:.1f}% [{fl['cent_t']:.1f}%] | {we} |\n")
            if bands:
                f.write("\n| lava case | low band (<400 Hz) env mean abs / bias (dB) [floor] | low energy patch - ref (dB) | high band (>2 kHz) env mean abs / bias (dB) [floor] | high energy patch - ref (dB) | 30 Hz AM depth ref / patch [ref 2nd seeds] |\n")
                f.write("|---|---|---|---|---|---|\n")
                for b in bands:
                    f.write(f"| {b[0]} | {b[1][0]:.2f} / {b[1][1]:+.2f} [{b[2][0]:.2f}] | {b[8]:+.2f} | {b[3][0]:.2f} / {b[3][1]:+.2f} [{b[4][0]:.2f}] | {b[9]:+.2f} | {b[5]:.3f} / {b[6]:.3f} [{b[7]:.3f}] |\n")


if __name__ == "__main__":
    main()
