# Warp Humans stadium review

Play a full desktop Growing Guns match with one local player, three bots and the procedural spectators:

```sh
cargo run --release -p growing-guns-app --bin growing-guns -- play --warp-humans --skip-menu --seed 52
```

In this mode each player slot and bot gets a Warp Human from the arena seed. The game fixes standing height at 1.80 m and assigns the slot's shirt colour. Faces, hair, skin and body build vary. The match binds the fitted body and clothing to the player skeleton at load time, so ordinary movement, hand sockets and ragdolls use the same model across respawns.

Add `--start-in-stands` to begin the match on a tread facing a neighbour at eye level. A standard player is 1.80 m tall with the camera at 1.60 m. Seeded spectators range from 1.55 to 1.95 m. The game grounds them on the same stepped geometry used for walking.

Warp Humans matches default to packed seating with eight radial aisles. The generator spreads up to 15,000 seats over the whole bowl. Use `--crowd-density standard` for the earlier spacing. The stadium viewer and headless review accept both explicit settings and default to standard when launched without `--warp-humans`.

The match supplies the seat identities, live positions, crowd reactions and per-player camera.

Shots use the simulated bullet path up to its first gameplay obstruction. The GPU tests a body-sized cylinder at the spectator's animated position. One bullet id can remove one living spectator across its segments. Nearby survivors have individual escape and return trajectories. Repeat gunfire extends an active escape or starts from the person's current displacement during the return. An explosion removes everyone in its lethal radius and scatters the surrounding survivors. Their hips, shoulders and head turn briefly away from the blast position, with a smaller turn near the edge of the scare radius. They then face the arena again. Another explosion redirects the turn from the current angle. Death persists across cameras and detail levels until the crowd is reset.

Confirmed contacts return asynchronously for directional blood spray, GPU debris and mist. A shared atlas supplies 16 blood footprints. A rough surface finish keeps the red visible when a stadium lamp reflects toward the camera. The fan at the hit and the falling droplets sample actual steps and walls. Selected rigid gibs stamp their first static contact. All decals reuse the stadium and arena mark pools. The game's ordinary frame loop never waits for these notifications. The CPU's reported number alive can lag the visible deaths. Run the in-stands scenario with `script games/growing-guns/scripts/crowd-reactions.json --out build/crowd-reactions --start-in-stands --warp-humans --seed 52 --graphics full --sky noon --players 1`.
A single spectator hit emits twelve tumbling pieces from the GPU debris pool. Their combined mesh volume scales with the victim's body dimensions, using 75 litres for a reference adult. Large blasts split a budget of 192 pieces among their victims. Three irregular shapes share a dark, fibrous cut-surface texture. Half the pieces have one outer skin face selected from eight tones using the spectator's pigmentation.

The people render into the arena's HDR colour and depth buffers before transparency and post-processing.
Each visible spectator receives the scene's sun shadow, floodlights and local lights at torso height during GPU detail selection. All detail levels share that sample; surface normals shape the response on faces, hair and clothing. Each visible surface samples the stadium's distance fog using its animated world position and the current camera. Shirts, faces and distant impostors take on the same atmospheric colour as adjacent masonry, including shadowed mist. The crowd does not cast individual shadows.
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
height. The close character uses a fitted shirt, jeans, shoes, alpha-cutout hair and brows on the MakeHuman hm08 body. Independent nose, lip, brow and adult face morphs share the same template. Hip, knee, shoulder and elbow motion uses the body's transferred skin weights. Each visible spectator gets a seeded sequence of quiet shifts, glances and cheering bouts. Either arm can lead. The collarbone follows a raised arm, the spine leans and twists around the waist, and the head turns and nods about the neck. When someone moves along a row, each foot alternates between a fixed landing point and a swinging phase driven by signed distance from the seat. A two-segment leg solve bends the hip and knee and rolls the hip toward the target. The ankle counter-rotates to keep the shoe level. Fear lowers the pelvis while limb lengths stay fixed. A nearby escape or a rise in stadium panic makes spectators throw up one or both hands. Short reaches vary in strength and timing per arm and per spectator. Their phase follows the presentation clock through repeated scares and detail changes. The same upper-body angles and lower-body pose drive all detail levels. Live match excitement and fear come from the crowd's existing pose uniform. Attribution and the
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
