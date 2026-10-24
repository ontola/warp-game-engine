# Matching an engine sound to a recording

How the `ej3` entry in `data/sounds.toml` was fitted to an engine-sim
recording of its own config (`3_cylinder_boxer.mr`, "Subaru EJ 3").

1. Get the reference audio and the rpm and throttle over time. engine-sim
   videos show both: the rpm needle and the throttle bar can be read per
   frame (a gauge fit to a few readable values). `analyze.py` refines the
   rpm from the audio (cycle harmonics).

       ffmpeg -i video.mp4 -ac 1 -ar 48000 ref.wav
       python3 analyze.py ref.wav track.json      # writes track_fine.json, segments.json

   `track.json` is `[[t, rpm, throttle_bar_px], ...]`. The controls for
   Kern are `[[t, rpm, throttle 0..1], ...]`.

2. Render ours along the same trajectory, and score it:

       cargo run --release -p cli -- sound ours.wav --car ej3 --model pulse --controls controls.json
       python3 score.py ours.wav    # mean |dB| on 1/3-octave bands, orders, HNR

3. Change one number with `--set key=value` (no rebuild), render, score again.

The score compares, over idle and four full-throttle segments, the
1/3-octave balance from 80 Hz to 8 kHz, the order spectrum (half orders to
12), and the harmonic-to-noise ratio. Low is close. It does not replace
listening. For ej3 (pulse model) it went from 14.0 to 5.2. Most of that came
from fixing the derivative scale in the port of engine-sim's synthesizer,
and from the runner low-pass that stands in for its 0D runner volume.
