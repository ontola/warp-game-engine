# Render suite

`engine/tools/render-suite.py` measures the renderer on one fixed world from a
fixed set of cameras, keeps the pictures, and compares two runs. Use it for any
change to occlusion, terrain, shading or shadows: change code, build, run,
diff, look at the pictures, keep what wins.

The world and cameras live in `engine/tools/render-suite.json`: alpine west of
x = 0, polder east of it, a town at the origin, seed 1, 2560x1440.

| View | What it stresses |
| --- | --- |
| `near` | Trees at a few metres, the mountain behind |
| `forest` | Forest edge, trees from 5 to 40 m |
| `wall` | A building fills the screen: cost of what is hidden |
| `town` | Street level between buildings |
| `hill` | Vista over forest and polder |
| `polder` | Flat land, ditches and water |
| `far` | 900 m up: valley, town, distant forest |
| `flight` | 60 m/s into the town: streaming and hitches |

## Running

Run from the repository root, with a release build of `terrain_fly`:

```sh
cargo build --release -p app --example terrain_fly
python3 engine/tools/render-suite.py run --label main --repeat 3
python3 engine/tools/render-suite.py run --label candidate --only wall,forest
python3 engine/tools/render-suite.py run --label no-fog-shadow --env KERN_SOME_SWITCH=0
python3 engine/tools/render-suite.py diff build/render-suite/main build/render-suite/candidate
```

Results go to `build/render-suite/LABEL/`: `report.json`, per view and repeat a
PNG, the raw frame times, per-pass GPU samples and `*.stats.json`. `diff` prints
a table and writes `vs-BASE/index.html` with base, candidate and heat map side
by side. It exits with 1 if a picture changed beyond the limit.

On Mancave the GPU is on the Windows side and Windows has no Python, so run the
script from WSL in a clone under `C:\dev` and pass
`--exe target/release/examples/terrain_fly.exe`: the tool hands the executable
Windows paths and the timings are native. Build with Windows `cargo`.

Each view is its own process, so no state leaks between cameras. The first run
after a shader change compiles every pipeline, which takes minutes, so a short
warm-up run comes first. Repeats go round all views, so drift spreads evenly.

## What is measured

- GPU time per pass and for the frame: median of the last `measure_frames` of
  `frames` still frames, then the median over repeats. The spread between
  repeats is kept as the noise of each number.
- Wall time per frame. This includes pixel readback and CPU scheduling, so it
  is not frame rate; it moves with whatever else the machine does. GPU times
  hardly do.
- Camera and shadow clusters, triangles and submitted vertices, voxel splats,
  and the forest cut: tested nodes, slots, camera and shadow-only roots.
- Memory: wgpu's allocator report (allocated, reserved, largest allocations by
  label), peak GPU memory over idle from `nvidia-smi`, peak process memory.
- Machine state before and after each view: load, CI jobs, GPU use, clock and
  temperature. `--require-idle` refuses to start next to CI jobs.

## Reading a diff

A time counts as changed when it moves more than 0.1 ms and more than the
spread of either run. Pictures are compared per pixel on the largest colour
channel: `over16` is the share of pixels off by more than 16 levels. More than
0.1% above the noise floor between repeats is a failure. The noise floor is very
low (views without trees repeat bit for bit), so a changed picture is a real
change, but not always of the code under test: the forest cut keeps moving for
hundreds of frames and streaming runs on wall time, so a build with faster
frames can catch it at another moment. That is why `polder` runs 1200 frames.
If only trees differ, rerun that view with more frames before blaming the
change. The `flight` picture is reported but never fails a diff. A passing diff does not replace looking at the pictures: a faster frame
with a missing effect is not a win.

Times from different machines or backends are not comparable; shadow passes that
cost 4.7 ms on an M5 Pro take well under 1 ms on the RTX 5070 Ti.
