"""One number per render: mean |dB| difference to the reference over the
1/3-octave bands 80 Hz..8 kHz and the order spectrum, plus the HNR gap,
averaged over the segments."""
import json, sys
import numpy as np
sys.argv = sys.argv[:1] + sys.argv[1:]
from analyze import load, segment, third_octaves, order_spectrum, hnr
segs = json.load(open("segments.json"))
sr, ref = load("ref.wav")
def feats(x, sr):
    out = []
    for label, t0, t1, rpm in segs:
        s = segment(x, sr, t0, t1)
        c, t = third_octaves(s, sr)
        o, od = order_spectrum(s, sr, rpm)
        out.append((t[(c >= 80) & (c <= 8000)], od, hnr(s, sr, rpm)))
    return out
R = feats(ref, sr)
for p in sys.argv[1:]:
    sr2, y = load(p)
    Y = feats(y, sr2)
    band = np.mean([np.abs(a[0] - b[0]).mean() for a, b in zip(R, Y)])
    orders = np.mean([np.abs(np.maximum(a[1], -30) - np.maximum(b[1], -30)).mean() for a, b in zip(R, Y)])
    h = np.mean([abs(a[2] - b[2]) for a, b in zip(R, Y)])
    print(f"{p}: bands {band:.1f} dB  orders {orders:.1f} dB  HNR {h:.1f} dB  total {band + orders + h:.1f}")
