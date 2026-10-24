"""Compare engine sound against the engine-sim reference: rpm-locked order
spectrum, 1/3-octave balance, harmonic-to-noise ratio, cycle envelope.

usage: analyze.py ref.wav track.json [ours.wav ...]
"""
import json, sys
import numpy as np
from scipy.io import wavfile

def load(p):
    sr, x = wavfile.read(p)
    x = x.astype(np.float64)
    if x.ndim > 1:
        x = x.mean(axis=1)
    return sr, x / (np.abs(x).max() + 1e-12)

def refine_rpm(x, sr, track):
    """Gauge rpm refined by the audio: the cycle (720 deg) harmonics."""
    out = []
    win = int(sr * 0.5)
    for t, r, thr in track:
        if r is None or r < 400:
            out.append((t, None, thr)); continue
        i = int(t * sr)
        seg = x[max(0, i - win // 2): i + win // 2]
        if len(seg) < win // 2:
            out.append((t, None, thr)); continue
        seg = seg * np.hanning(len(seg))
        spec = np.abs(np.fft.rfft(seg, 1 << 17))
        df = sr / (1 << 17)
        best, br = -1, r
        for cand in np.linspace(r * 0.88, r * 1.12, 121):
            f0 = cand / 120.0
            ks = np.arange(1, 31)
            s = np.log(spec[np.clip((ks * f0 / df).astype(int), 0, len(spec) - 1)] + 1e-9).sum()
            if s > best:
                best, br = s, cand
        out.append((t, br, thr))
    return out

def segment(x, sr, t0, t1):
    return x[int(t0 * sr): int(t1 * sr)]

def order_spectrum(x, sr, rpm, orders=np.arange(0.5, 12.01, 0.5)):
    seg = x * np.hanning(len(x))
    n = 1 << int(np.ceil(np.log2(len(seg) * 4)))
    spec = np.abs(np.fft.rfft(seg, n)); df = sr / n
    crank = rpm / 60.0
    amps = np.array([spec[int(round(o * crank / df)) - 2: int(round(o * crank / df)) + 3].max() for o in orders])
    return orders, 20 * np.log10(amps / amps.max() + 1e-9)

def third_octaves(x, sr):
    centres = 1000 * 2.0 ** (np.arange(-17, 13) / 3.0)  # 20 Hz .. 16 kHz
    spec = np.abs(np.fft.rfft(x * np.hanning(len(x)))) ** 2
    f = np.fft.rfftfreq(len(x), 1 / sr)
    e = np.array([spec[(f >= c / 2 ** (1 / 6)) & (f < c * 2 ** (1 / 6))].sum() for c in centres])
    db = 10 * np.log10(e / e.sum() + 1e-12)
    return centres, db

def hnr(x, sr, rpm):
    """Energy at cycle harmonics (+-2 bins) over the rest, below 3 kHz."""
    seg = x * np.hanning(len(x))
    n = 1 << int(np.ceil(np.log2(len(seg) * 2)))
    p = np.abs(np.fft.rfft(seg, n)) ** 2; df = sr / n
    f0 = rpm / 120.0
    mask = np.zeros(len(p), bool)
    k = 1
    while k * f0 < 3000:
        c = int(round(k * f0 / df)); mask[max(0, c - 2): c + 3] = True; k += 1
    band = np.arange(len(p)) * df < 3000
    return 10 * np.log10(p[mask & band].sum() / p[~mask & band].sum())

def report(name, x, sr, segs):
    print(f"== {name}")
    for label, t0, t1, rpm in segs:
        s = segment(x, sr, t0, t1)
        o, od = order_spectrum(s, sr, rpm)
        c, td = third_octaves(s, sr)
        print(f"  {label:10s} {rpm:5.0f} rpm  HNR {hnr(s, sr, rpm):5.1f} dB  rms {np.sqrt((s**2).mean()):.3f}  crest {np.abs(s).max()/np.sqrt((s**2).mean()):.1f}")
        print("    orders " + " ".join(f"{a:g}:{b:.0f}" for a, b in zip(o, od)))
        print("    1/3oct " + " ".join(f"{int(a)}:{b:.0f}" for a, b in zip(c, td) if 40 <= a <= 12500))

if __name__ == "__main__":
    ref, track = sys.argv[1], json.load(open(sys.argv[2]))
    sr, x = load(ref)
    fine = refine_rpm(x, sr, track)
    json.dump(fine, open("track_fine.json", "w"))
    # Steady-ish segments: idle and full throttle at three speeds.
    def mean_rpm(t0, t1):
        v = [r for t, r, _ in fine if r and t0 <= t <= t1]
        return float(np.mean(v)) if v else 0
    segs = [("idle", 4.0, 7.5, mean_rpm(4.0, 7.5)), ("wot 4.4k", 33.0, 35.0, mean_rpm(33, 35)), ("wot 5.4k", 41.5, 43.5, mean_rpm(41.5, 43.5)), ("wot 6.1k", 51.0, 53.0, mean_rpm(51, 53)), ("wot 4.2k", 58.0, 62.0, mean_rpm(58, 62))]
    json.dump(segs, open("segments.json", "w"))
    report("reference", x, sr, segs)
    for p in sys.argv[3:]:
        sr2, y = load(p)
        report(p, y, sr2, segs)
