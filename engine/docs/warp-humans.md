# Warp Humans stadium review

Play a full desktop Growing Guns match with one local player, three bots and the procedural spectators:

```sh
cargo run --release -p growing-guns-app --bin growing-guns -- play --warp-humans --skip-menu --seed 52
```

The match supplies the seat identities, live positions, crowd reactions and per-player camera.
The people render into the arena's HDR colour and depth buffers before transparency and post-processing.
Sun and sky illuminate them. Local floodlights and cast shadows on the figures remain open.
The normal crowd is the default. The minimum graphics profile omits the stadium.
Use the same flag with `shot` or `bench` for a repeatable match comparison.

Launch the interactive desktop stadium viewer with:

```sh
cargo run --release -p growing-guns-app --bin growing-guns -- human-play --seed 52
```

Scroll to zoom, drag with the left mouse button to orbit, and use Left/Right
to select a neighbour. Home frames the stadium, End frames the face, Space
pauses the motion and Esc closes the playtest.

Run the repeatable headless review from the workspace root on a GPU with compute and indirect drawing.

```sh
cargo run --release -p growing-guns-app --bin growing-guns -- human-review --seed 52 --far cells --size 960x540 --frames 121 --out build/humans/cells
```

Repeat with `--far mesh` and `--far impostor` and separate output directories.
Each output frame contains the travelling camera on the left and the stationary
stadium camera on the right. Both sample the same presentation time and immutable
person table. Four frames and a JSON report are saved. Add `--sequence` to save
every frame for video. Use separate runs for video and timings.

Growing Guns supplies its bowl and occupied seats. A person's seed selects
proportions, pigmentation, clothing and movement phase. Engine callers can fix
height and shirt colour with `render::humans::Constraints`. Seats use the game's
height. The close character uses a fitted shirt, jeans, shoes, alpha-cutout hair and brows on the MakeHuman hm08 body. Independent nose, lip, brow and adult face morphs share the same template. Hip, knee, shoulder and elbow motion uses the body's transferred skin weights. Attribution and the
binary layout are in `engine/assets/humans/README.md`.

The compute pass culls and classifies by projected height separately per
view. Below 32 pixels the test draws articulated boxes, low-sided meshes, or
camera-facing elliptical impostors. Up to 170 pixels it uses shared rounded
parts. The medium band starts background decoding of the anatomical mesh and appearance atlas. Beyond 170 pixels the GPU receives the shared detail page; medium
geometry covers any frames spent waiting for that page. The upload happens after both
views have rendered. Both cameras can then use the same resident mesh.
The embedded CPU reference stays available for the lifetime of the executable.
This experiment has one detail page and no eviction.

The report includes the selected instance counts, per-view GPU selection and
raster time when timestamp queries are supported, allocated geometry and work
buffer bytes, and request-to-first-detailed-frame latency. Atlas bytes and colour and depth
texture bytes are listed separately. Driver allocations are outside these totals.
Readback serializes views for reproducibility. PNG encoding and readback can
increase cold wall time. The fixture uses simple directional lighting and the
stadium geometry; gameplay, shadowing and the normal frame compositor are
outside its timing scope.

For the plant analogue, see [vegetation selection and paging](vegetation-architecture.md).
The body, morphs, clothing, skin maps and hair are attributed in [the asset notes](../assets/humans/README.md).
For comparison with production crowd animation and perceived repetition, see
[NVIDIA's animated crowd rendering chapter](https://developer.nvidia.com/gpugems/gpugems3/part-i-geometry/chapter-2-animated-crowd-rendering)
and [McDonnell et al., Clone Attack!](https://www.scss.tcd.ie/Rachel.McDonnell/papers/oldSiggraph08.pdf).

Visual references for the updated figures include the [NPHM head scans](https://simongiebenhain.github.io/NPHM/)
and [Muybridge's locomotion sequence](https://commons.wikimedia.org/wiki/File:Muybridge_human_male_walking_animated.gif).
Compare the brow ridge, nose projection and distance from the lips to the chin
at equal camera angle. The current close mesh has one neutral expression;
eye movement, blinking and expression-driven wrinkles remain open.
