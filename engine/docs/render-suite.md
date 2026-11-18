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
change, but not always of the code under test: streaming runs on wall time, and
on `polder` the forest cut rests in one of several states (8.09M or 8.22M
triangles, up to 1.1% of pixels apart), chosen by timing, even between repeats of
one build. The
`polder` and `flight` pictures are therefore reported but never fail a diff. If
only trees differ on another view, compare its triangle count and repeat the run
before blaming the change.
A passing diff does not replace looking at the pictures: a faster frame
with a missing effect is not a win.

Times from different machines or backends are not comparable; shadow passes that
cost 4.7 ms on an M5 Pro take well under 1 ms on the RTX 5070 Ti.

## Look suite

`engine/tools/look-suite.json` is a second suite for how things look rather than
what they cost: the places playtests found problems, a few angles each, named
`scenario.angle`. Each view can carry its own `"world"` (lowland or alpine, with
grass).

| Scenario | What to look at |
| --- | --- |
| `tree` | One tree from 10 to 200 m: mesh and voxel brightness must agree |
| `ring` | 150 m up: the edge of the grass disc |
| `shadow` | Sharp shadow under the camera at 8, 16 and 30 m up |
| `boulder` | Walking at a rock: height must not pop |
| `stripes` | Grass bands along an alpine slope |
| `circle` | Dark spots on the 1.5 m grass grid |
| `edge` | Shadow map handover on the ground |
| `grassedge` | Voxel grass against shaded ground |
| `far` | Slopes 300 to 2000 m away under a low sun |
| `load` | Bushes 60 and 120 frames after arriving |
| `fly` | In motion, at 40 m/s |

```sh
python3 engine/tools/render-suite.py run --suite engine/tools/look-suite.json --label after --arg=--voxel-raster-profile
python3 engine/tools/look-report.py after --base before --out build/look-reports/after
```

`look-report.py` prints per view the mean linear crown colour inside the view's
`crown` box with its ratio to the nearest tree view, the strength of the grass
grid (autocorrelation of dark pixels near the view's `grid_px`, about 1.5 m on
screen) and the largest voxel cell. With `--out` it writes `report.json`, with
GPU time per pass group and counters, and a 960 px JPEG per view and per base
view, for a page to show side by side. Run `look-clips.py LABEL --exe EXE` first to
add the suite's `clips`: terrain_fly `--dump` with `--dump-warm`, `--dump-every` and
`--dump-forward`, encoded with ffmpeg.

## Finding pop-in

Pop-in is a one-frame change that camera motion does not explain. Two tools
find it in a clip and name its cause:

- `terrain_fly --dump N DIR --dump-cut DIR/cut.jsonl` writes, per frame, what
  every GPU forest tree draws for the camera and the sun (a read-back of the
  residency slots and root histories, `foliage_runtime::Runtime::cut_snapshot`)
  and how each changed since the previous frame (`CutSnapshot::change`:
  appear or disappear, fallback, whole tree to branch cut or back, whole-tree
  mip, branch refine or coarsen, voxels to needles, voxel mip). Changes of trees whose
  root is on screen carry their screen position and distance. Each line also
  has the error-cut scales, the refine state's used totals, refusals and fills,
  the sun clipmap levels (`Renderer::sun_clipmap_log`: deferred, drawn,
  full redraw, scroll) and the voxel page cache counters. Diagnostic
  readbacks: use it for clips, not timing.
- `cargo run --release -p render --example pop_meter -- DIR` reads the PNGs.
  Per 16 px block it takes the smallest mean luma difference to the previous
  frame over shifts up to `--radius` px, and counts a pop where that residual
  is a spike over the frames before and after (shimmer and motion are not).
  It writes `DIR/pops.json` (per frame score, the 20 largest regions with
  their luma before and after) and, with `--crops OUT`, before|after crops.
  Disocclusion behind a near trunk or branch also scores, so pick paths
  without anything within a few metres of the camera.
