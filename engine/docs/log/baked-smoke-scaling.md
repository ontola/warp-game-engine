# Baked smoke scalability baseline, 31 October 2026

This report describes the implementation before [adaptive smoke rendering](../adaptive-explosion-smoke.md).
Its 16-volume cap, full-resolution costs and proposed changes are historical.

The current full-quality volume smoke is inexpensive for a few small distant
blasts, but is not ready as an unrestricted effect for explosion-heavy combat.
Memory is bounded; screen coverage, overlap and the cost of almost-empty late
volumes are the problems. The existing 16-volume cap also discards old smoke
before its intended 4.6-second tail finishes at sustained rates above about
3.5 explosions per second. At 40/s, the retained history is roughly 0.4 seconds.

This report measures renderer revision `9af7ed3` on an NVIDIA GeForce RTX 5070 Ti,
Vulkan, native Windows/MSVC dev build (workspace opt-level 1, dependencies 3).
No game window was opened. GPU experiments ran serially, with no simultaneous
game or compilation. CPU figures are observations of that development build,
not release CPU acceptance. The [measurement data](../data/baked-smoke-scaling-20261031.json)
records scopes, sample counts, cameras, radii, modes and workload counters.

## Real-game sustained explosions

The existing Growing Guns stress fixture requests 40 real explosions per
simulation second, radius 6.5 m, damage 140, at live arena chunks. Both recipes
completed all 600 requests over 900 measured frames after 180 warmup frames.
Both removed 7,800 chunks, made 15 arena resets and had zero empty-target
fallbacks. All 900 frames had GPU profiles, with no dropped/error profiles.

One native 3840x2160 output, four 1920x1080 views, full graphics. This fixture
uses four identical overhead cameras; it is not a close-up worst case. Arena
resets are included. Audio, HUD and display presentation are excluded.

| GPU scope | Legacy median / p95 | Baked median / p95 |
| --- | ---: | ---: |
| Complete FX pass, summed across views | 0.717 / 0.976 ms | 2.818 / 3.087 ms |
| Covered GPU union across measured scopes | 8.017 / 8.304 ms | 10.121 / 10.452 ms |

The roughly 2.1 ms median increase is material against an 8.33 ms total budget
for 120 fps. These scopes do not establish presented FPS. The CPU FX-flush
median was 0.069 ms legacy and 0.062 ms baked; GPU drawing dominates the smoke
regression in this workload. The smoke cap means this test continually removes
older plumes and therefore does not exercise complete long tails at 40/s.

## Isolated screen coverage and overlap

The new [scaling probe](../../crates/render/examples/baked_smoke_scaling.rs) uses
the actual `Effects::explosion` recipe and its lights in a floor-only scene.
Every case uses 12 warmup frames and 24 measured GPU timestamp samples. Assets,
pipelines and targets are warmed; startup, CPU upload and image readback are
outside the measured scope. These are complete FX-pass costs, not pure-smoke
costs or full-game FPS. Both resolutions below are one output split into four
views (960x540 at 1080p; 1920x1080 at 4K), with four distinct camera directions.
No temporal reconstruction or spatial upscaler is enabled.

At explosion age 0.85 s, baked recipe median GPU milliseconds:

| Setup | 1 blast, 4K | 4 blasts, 4K | 8 blasts, 4K | 16 blasts, 4K | 16 blasts, 1080p |
| --- | ---: | ---: | ---: | ---: | ---: |
| Spread, radius 3.3 m | 0.200 | 0.416 | 0.714 | 1.345 | 0.398 |
| Overlapping, radius 3.3 m, cameras outside | 0.429 | 1.376 | 2.623 | 5.151 | 1.339 |
| Overlapping, radius 6.5 m, cameras inside volume bounds | 2.133 | 8.305 | 16.501 | 33.002 | 8.273 |

The spread layout increases the grid footprint with count, so its curve also
changes projection/visibility. Do not interpret it as a constant per-blast
cost. The overlap/inside cases deliberately stress overdraw; their cameras and
radii are explicit in the data. For 16 radius-30 m airstrike-sized volumes with
cameras inside, the complete FX pass reached 48.403 ms median at age 0.85 s.

At age 4.3 s, 16 nearly faded radius-6.5 m volumes still cost 33.000 ms at 4K
with cameras inside their bounds (8.278 ms at 1080p). The captured image has
almost no visible smoke there, but the shader still visits the empty/thin
regions. From outside, the corresponding radius-3.3 m overlap case costs
5.124 ms. Low remaining opacity is not currently a cheap case.

Requested burst counts above 16 plateau for smoke because older volumes are
removed, not because all requested smoke becomes cheap. Retained volume counts
in the probe are labeled as derived from the source cap, not GPU observations.
Other explosion effects continue to be generated above this count.

## Why, and what to improve

The animation uses one shared 6.75 MiB texture and no runtime fluid simulation.
Each covered pixel nevertheless runs up to 48 ray steps, with two 3D texture
samples per step to interpolate frames. Bounding quads and opaque depth limit
work, but separate smoke volumes do not stop each other's raymarches. Near-plane
crossing expands a volume's quad to the viewport. Thin wisps rarely reach the
opacity early-exit threshold. This pass runs at output resolution after temporal
reconstruction, so lowering an upscaler's internal resolution does not directly
lower its pixel count.

Before making this the general explosion effect, prioritize a cheaper smoke
render target with depth-aware upsampling; fewer samples or a cheaper representation
for small/distant plumes; rejection of negligible-density regions/ages; and a
screen-area/visibility budget with gradual eviction instead of the hard oldest-
volume removal. Keep detailed volumes for the blasts where their depth is
visible. Those are proposed changes, not implemented or measured savings.

The current low/minimum graphics profiles retain the older billows. Actual
large overlapping blasts on slower GPUs remain unmeasured. The 16-volume cap
is a resource limit, not a frame-time guarantee.

## Reproduce

```powershell
$env:GG_BAKED_SMOKE='0' # repeat with '1'
target/debug/growing-guns.exe bench --stress explosions --views 4 --size 3840x2160 --graphics full --cpu-thread --csv build/smoke-scaling/legacy-4k-four.csv --shot build/smoke-scaling/legacy-4k-four.png
cargo build --locked -p render --example baked_smoke_scaling
target/debug/examples/baked_smoke_scaling.exe --size 3840x2160 --views 4 --out build/smoke-scaling/probe-4k --captures
target/debug/examples/baked_smoke_scaling.exe --size 1920x1080 --views 4 --out build/smoke-scaling/probe-1080p --captures
target/debug/examples/baked_smoke_scaling.exe --size 3840x2160 --views 4 --layout inside --radius 30 --counts 0,1,4,8,16 --out build/smoke-scaling/probe-airstrike
```

Use separate filenames for each game recipe and restore the prior environment
value after comparison. `CARGO_TARGET_DIR` changes the executable paths. Full
raw game CSVs/logs and isolated per-pass reports from this run are under
`build/smoke-scaling`; the committed data preserves the FX measurements and
real-game summaries needed for the conclusions above.
