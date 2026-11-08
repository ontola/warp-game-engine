# Warp Humans stadium review

Run from the workspace root on a GPU with compute and indirect drawing.

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
height. The head is derived from MakeHuman's CC0 hm08 model. Attribution and the
binary layout are in `engine/assets/humans/README.md`.

The compute pass culls and classifies by projected height separately per
view. Below 32 pixels the test draws articulated boxes, low-sided meshes, or
camera-facing elliptical impostors. Up to 170 pixels it uses shared rounded
parts. Beyond that threshold it requests a shared high-detail mesh; medium
geometry covers the first requesting frame. The upload happens after both
views have rendered. Both cameras can then use the same resident mesh.
The embedded CPU reference stays available for the lifetime of the executable.
This experiment has one detail page and no eviction.

The report includes the selected instance counts, per-view GPU selection and
raster time when timestamp queries are supported, allocated geometry and work
buffer bytes, and request-to-first-detailed-frame latency. Colour and depth
texture bytes are listed separately. Driver allocations are outside these totals.
Readback serializes views for reproducibility. PNG encoding and readback can
increase cold wall time. The fixture uses simple directional lighting and the
stadium geometry; gameplay, shadowing and the normal frame compositor are
outside its timing scope.
