# Handoff: Warp engine (Sideways)

For the next agent. Read this, then `README.md` (commands, parity, renderer)
and `../AGENTS.md` (the Godot game this ports).

Repo: https://github.com/ontola/warp-game-engine, branch `main`: the engine
in `engine/`, the Godot game beside it as the reference. (It started as the
`rust-engine` branch of joepio/sideways; the game's own `main` there stays
untouched.)

## Current goals

### Delivery queue — 24 October, after live sky/menu request

**A CPU prototype is not a delivered feature.** Close an item only after it is
present in an identified playable build and its real rendered result is checked.
The user explicitly reported that crowds and road depth were being left unfinished.
Keep new requests in this queue rather than silently replacing earlier work.

| Item | Actual status | Next acceptance step |
| --- | --- | --- |
| Car headlights at night | r18 live: two measured physical lamp origins per car, grass receiver fix, eight-light/four-view real GPU fixture PASS | Moving night DLSS and four-player cost acceptance remain; no grass cast shadows or volumetric beams yet |
| Sunrise light disappears on approach | Open: low-sun near/far shadow coverage and short canopy fallback under investigation | Capture matching near/far positions, isolate map/detail/fallback contributions; do not claim fixed by the road normal correction |
| Volumetric fog and indirect lighting | User requested after first starting PR2 playtest. Current fog is analytic height fog; SH9 is global sky, not local bounced-light cache | Low-resolution per-view froxels with shadowed sun/headlamps, followed by shared world-space diffuse irradiance cache. Measure four-view total; no speed claim yet |
| Live procedural sky/time menu | Delivered in PID 269560. In-place GPU update passes; actual menu observed and sunset revision 2 applied in live F12 | Complete atmosphere switching / paused-position matrix later; no clouds/rain yet |
| Roadside crowds | User rejected prototype appearance; disabled in current playtest (`SIDEWAYS_CROWDS=0`) | Deferred by user to a future update; do not re-enable as part of graphics iteration |
| Unified rock wall | Delivered in PID 269560; separate overlays removed; matched actual #1/#4 renders reviewed | One consistent surface now, but further shape/lighting refinement remains |
| Terrain pyramid/straight crest | River-bank source correction delivered in PID 269560; matched #4 render reviewed | Left toe/right rail grid artefacts remain open; not all terrain faceting solved |
| Black stone guardrail triangles | New screenshot captured; lighting/normal diagnosis active | Prove source cause and inspect corrected same-location render |
| Road surface depth/aggregate | Existing shallow geometric ruts/ridges; detailed gravel is largely texture/normal detail | Audit and deliver visible near-road stones and convincing surface depth; not complete |
| Distant forest shadow continuity | Ellipsoid fallback still loses real gaps. Source coverage tile is CPU prototype only; source crown bounds opt-in/off | Connect source-derived visibility/depth to live rendering and compare user's near/far pair |
| Unified procedural/voxel grass and branch wind | Source/voxel optics and animation prototypes exist; not yet accepted as live grass replacement | GPU optics, motion and cost acceptance before replacing current reference grass |

The immediate build contains only features that pass their actual render checks.
No further Godot comparison is requested. The 4K-output / four-player / 120 FPS
target remains unmet/unproven by this work; prototype counts are not FPS evidence.

### Current playable build — 24 October, 11:54 Amsterdam

Owned live process **231924**, visible first frame confirmed and process responding.
Controller operation in this specific restart has not been independently tested.
Manifest `build/dev-runs/20261024T095403418Z-c35d30f1/iteration.json`;
staged executable in the same directory. SHA256
`799600998ca262645e0f4806d0b06d2c0409ae70525d431b873029722e187cb7`.
4K output / one view / DLSS Quality; 120 Hz physics and four substeps retained.
Source is the dirty checkout on merged PR2 commit
`0f4baa8652cbd081a6c6810033cc466325c6f6b3`, not an immutable source snapshot.
Final release compilation took 78 s; launch's cached build took 1.62 s and visible
startup 20.08 s. Prior PID 219656 was deliberately closed before exclusive r18
GPU validation. PR2 is merged remotely; local work was preserved through safety stash
`7fed750c0d003428ae18ee48657dd662489b18cb` and reapplied. Keep that stash.
All eight merge conflicts resolved; 149 render library tests passed, one ignored;
four car-lamp, four capture-replay and four final composed-shader checks passed.

Enabled: PR2 procedural car materials, procedural sky, unified rock, river-bank
correction, finite height fog, local terrain sun occlusion, and night headlights.
Crowds remain disabled at the user's explicit request; preserve that setting.
Actual r17 haze/mist GPU images no longer show the former horizon stripe, but
mist still has no volumetric shadowing. r18 same-location road/grass renders and
a DLSS night shot were visually reviewed; moving night acceptance remains open.
Local car material binding is 19;
car diagnostics are 20/21/22, preserving renderer diagnostics 8..14.
Reference extended grass remains; new shared voxel grass and optional source
crown bounds are **not** promoted. No new road-depth implementation in this build.
The user is actively driving now: do not run concurrent GPU tests or take over
the controller/menu. Start/Esc opens the time/atmosphere menu; F12 captures state.

PR2 integration follow-ups: optional voxel scene layout now totals 17 storage
buffers against the requested device limit of 16; fix before enabling that path.
Hot reload updates opaque/transparency but not grass/far/voxel helpers and is not
transactional across the two passes. Use a full restart after shared-lighting
shader changes. No full-Nanite, four-player performance or night-quality claim.

Road stripe correction (r18): explicit `NormalEncoding::SrgbTransferred` only
for gravel decodes normal RGB before mips; other maps remain linear. Actual
matched-camera `r17/afternoon-road/lit.png` versus `r18/user-road/lit.png` shows
the three black bands removed. Prior shadow/albedo diagnostics were uniform,
but mapped normals were striped: this was an import encoding error, not a
reason to change ambient light or roughness globally.

Headlight grass correction: all three reference-grass paths share inclined-blade
light response rather than rejecting a light below the fragment via world-UP
NdotL. Actual `r18/user-grass-night/lit.png` replays F12
`sideways-1792834206688-219656-0` with its original lamp poses held fixed. Native
diagnostic view omits cars/HUD, so it is not pixel-parity with the DLSS capture.
New paired mounts were also viewed in `r18/night-dlss.png` (held frame, 1920x1080
output / 1280x720 input). No moving/4K/four-player performance inference.

`r18/headlights-final/report.json` passes: off/blocked/open RGB 0/0/56.6,
grass above lamp 196.32, eighth light alone 4.4, and zero different pixels in
all four views against the eight-light single-view reference. This exposed and
fixed a real pass-classification bug: split view marker 1 was mistaken for a
perspective shadow pass, removing NO_SHADOW grass. Headlight shadow uses marker
2 now; main-view and culling predicates distinguish both. Production scene cost
remains unmeasured. All evidence is under `C:/dev/perf/warp-sideways-20261024`.

Sunrise follow-up (read-only source audit, still OPEN): `shadow_at` blends to
`transmittance(..., 50)` at shared-map boundaries. Its 12 expanding samples end
at 48.1683 m, and the last untrimmed integration step reaches 69.844 m. Increasing
max_t alone does not add samples. At a 2-degree sun a 20 m crown can cast a
573 m ground shadow; the canopy layout only reserves a 60 m ray guard beyond
the receiver domain. These are known approximation/coverage limits, not proof
of the user's particular near-distance switch. Both sun maps use depth range
-500..+399 m toward the sun relative to their distinct centres. Near/far map
resolution, caster LOD and normal bias also differ. The latest saved low-sun
F12s are sunset (2 degrees / azimuth 270), not sunrise (azimuth 75).
Next exclusive-GPU diagnosis: same captured camera with debug=9; debug=9 and
near_shadows=0; then also gi=0. Inspect mapped/source NdotL (13/14) if unchanged.
Compare the same world receiver from two camera distances before changing
production defaults. Do not interrupt the now-live playtest just for speculative
distance tuning, and do not report sunrise fixed by the normal-map change.
Independent bias audit: far/near texels are 4.6875/1.5625 cm and receiver normal
offsets 7.03125/2.34375 cm. On a horizontal plane with 2-degree, azimuth270 sun,
receiver plus slope-scale raster bias corresponds to approximately 4.746/1.582 m
along the light ray (excluding constant depth bias). This can erase distant
contact shadows and restore them near, but is not yet a measured diagnosis of
the user's location. Saved low-sun camera targets are within far receiver
coverage, so the edge fallback alone does not explain those exact target points.
Include separate bias and caster-depth ablations in the exclusive-GPU matrix.

Evidence under `C:/dev/perf/warp-sideways-20261024/r16`:
- `sky-live-final`: one actual renderer updated morning/noon/sunset/night/noon;
  return-to-noon pixels equal, invalid update preserves output, four small views
  rendered. This is functional validation, not four-player performance evidence.
- `crowds-gpu-final`: 20 real images across native 1920x1080 and DLSS Quality
  1280x720 input; all 1,215 fixture part poses/feet match CPU reference. Characters
  visibly have prototype faceting and separated joints; no polished-art claim.
- `replay-final-1`, `replay-final-4`, `replay-final-5`: strict matching stage seeds,
  captured camera, actual lit/albedo/direct/indirect frames. No cars/HUD; native
  diagnostic images are not route performance or DLSS parity claims.
- Live `build/screenshots/sideways-1792830832981-269560-0.json`: user already
  applied sunset, revision 2, clear, elevation 2/azimuth270, pending false,
  no error, no new race round. Paused position preservation has CPU coverage;
  a controlled before/after paused live capture pair is still pending.

Black guardrail remains open. Actual #5 replay has intact nonzero albedo on
black pixels, direct=0 on both black and control regions, but indirect=0 only
in black regions. Same owner/cluster/material; next investigate five-cone
terrain sky visibility and receiver-modulated bounce. Do not mask with an
unproven material/normal/ambient-floor change.

Black-wall follow-up: F12 `sideways-1792829525441-102808-5` is round 4,
stage noise/detail `1049722424 / -85257117`. The CPU-only
`wall_lighting_audit` reproduced that stage and found all 1,150 source wall
triangles selected at its camera; Geometry and TexturedGeometry cuts match.
Across 148 sampled black pixels, source normals are finite (length >=0.9989)
and point toward the sun (dot approximately 0.89–0.92). A wall LOD/normal
patch is therefore not justified by this evidence. The scene wall has no
SOLID contact proxy. Evidence: `C:/dev/perf/warp-sideways-20261024/r16/wall-lighting-audit.json`.
`capture_replay` now includes exact black/control pixel picks and lit, albedo,
direct and indirect views to distinguish the actual frontmost surface and
light attenuation from composition. The black-wall bug remains open; do not
hold the requested playtest for speculative material changes.

#### Road surface depth — audited, next playable iteration still pending

This is a source audit, not a new rendered feature or a performance result.
`sideways::builder::road` already supplies the driving mesh and collider from
23 across-road samples at 2 m longitudinal spacing. Its coefficients are
4.5 cm rut depth, 2.5 cm centre ridge and 4 cm windrow, modulated by wear and
wandering track centres. Sampling makes the actual relief smaller: the
windrow peak at across=0.8 lies between samples 0.72 and 0.86, so that term
reaches only 2.28 cm at the source vertices. `visuals::road_mesh` adds no
small aggregate geometry and uses the positional-only `Geometry` CLOD
policy; preservation of these shallow grooves at selected LOD is unmeasured.
The ROAD shader uses real texture normal maps but its rut mask stays at
abs(across)=0.25 instead of following the builder's wander and wear. Thus
dark painted tracks and the geometric depressions are not exactly aligned.
There is currently no individual road-stone geometry or dynamic tyre-track
accumulation in this renderer. The separate cliff work does not implement it.

The user's road proposal is design input, not verified evidence of how DiRT
or WRC implements these effects. The first bounded deliverable starts with
one typed road profile: per-row track centres/wander, wear, rut depth and
ridge/windrow parameters shared by source geometry, collision and shading.
Preserve current driving heights initially; make the shader read the same
profile rather than approximate it with an unrelated fixed dark stripe.
Actual deeper deformation is a separate intentional handling change requiring
matching collision/tests. Verify the CLOD-selected surface preserves the
important groove shape before changing tessellation or shader complexity.

Then add a seeded road-aggregate builder using a small shared library of
closed, irregular stone templates, baked into short CLOD tiles on the exact
existing road triangles. First candidate budget: 8 m tiles,
up to 48 stones/tile, 12–20 triangles/stone; approximately 72–120k source
triangles/km before the existing CLOD selection, and 125 tile instances/km.
Those are proposed limits, not measured GPU costs. Template topology is
reused during generation; transformed tile vertices consume unique storage.
Use 3–10 cm diameters, mostly embedded with <=2 cm protrusion, seeded visual
RNG, and fewer stones in the worn grooves/more in the windrows. This preserves
the existing driving mesh/collider and avoids stones floating on the analytic
height field. Do not silently enlarge physical obstacles or add large
per-stone GI proxy volumes. Ordinary raster/shadow passes supply actual
silhouette/depth; visibility of their small shadows still needs GPU proof.

Later, if these two improvements still leave a clear gap, evaluate a bounded
world-space deformation cache driven by authoritative wheel contact/load/slip
signals. Toroidal storage must track its world origin and clear newly exposed
strips on integer-cell scroll; modulo UVs alone alias old tracks into new
ground. Four separated players need coordinated windows or shared sparse
world tiles, with wheel stamps applied once per simulation update rather
than once per camera. Make eviction/loss of old visual tracks explicit.
A proposed 2048-square, 40 m window has ~1.95 cm texels and costs 8 MiB in
R16F or 16 MiB in RG16F; four independent windows cost 32/64 MiB before mips,
temporary copies or other attachments. These are storage arithmetic, not a
choice of default or evidence of runtime affordability. Keep channel meanings
explicit: displacement, raised edges and wetness cannot silently share one
quantity or refer to a nonexistent B channel in an RG texture.

Do not read GPU deformation back into the 120 Hz driving simulation. Initially
it is bounded visual detail; any later physical rut tracking needs a separate
deterministic CPU-authoritative representation and matching collision design.
For microdetail, limited near-view POM/height blending is an optional measured
experiment after the real geometry is accepted. Bound its screen coverage,
step count and distance/angle transition, with a stable normal-map fallback.
POM is not free, does not provide actual silhouette/collision, and its local
self-shadow approximation does not replace ordinary scene shadowing. Keep
height/albedo/normal/roughness sampling coherent when rotating/blending tiles.
Avoid camera-dependent high-frequency sparkle: the user already reported
noisy, shimmering ground. Wetness must respond to material and lighting rather
than forcing every track to near-mirror roughness or a world-up normal.

Check deterministic seeds, exact triangle support, tile bounds/winding and
selected-CLOD profile error on CPU. Acceptance requires close oblique and
driving-camera native/DLSS images, low-sun lighting, moving LOD continuity,
and four-view GPU cost at 4K output against the total 8.33 ms frame target.
No per-view duplicate world updates or unmeasured "cheap" claims. Do not mark
this done from CPU checks or source counts. Ship the already-ready
sky/rock/crowd work first; this road iteration must not delay that playtest.

### 24 October, current playtest and pending r16 work

- Joep authorized restarting when updates were ready. Managed PID **102808**
  (`20261024T073758926Z-bdb57264`) now runs the new cliff/road-material iteration.
  Release build took 88.656 s; visible first frame and actual window image were
  confirmed. Xbox controller detected; user was already driving at capture.
  Executable SHA256: `b157001d7e4ffdfde60ef61f53cb57e4e16686173a21fb85f7208e7cbde2e3f1`.
  One player, 3840x2160 output, DLSS Quality, clear, summer, wind 0.6,
  extended reference grass. Crowds and procedural sky remain disabled.
  New foliage optics/dispatch are compiled but not enabled in the live world.
  Prior PID 109828 was closed by the managed launcher after successful build.
  Keep this new session running during CPU work; no concurrent GPU benchmarks.
- The new **Roadside crowds** task is assigned to the freed
  `road_terrain_seam` agent (its visible chat title was changed). Two attempts
  to spawn a fresh agent were rejected by the agent-thread limit. Its scope is
  now reusable baked GPU animation and opt-in roadside spectator groups;
  it must not resume old seam/sky work. User context is the pasted text in
  attachment `c1f75c08-c425-4878-9075-603a2a6d8e8a`.
- Root added `Scene.voxel_active_tiles` (default **false**). Tile binning
  appends nonempty tiles to a GPU worklist; indirect dispatch visits their
  pixels with the unchanged integration function. Empty pixels are cleared by
  a lightweight pass included in the tile GPU timestamp. Argument bytes are
  copied into a separate indirect buffer to avoid storage/indirect aliasing.
  Dispatch supports more than 65535 tile jobs via bounded two-dimensional
  workgroups. Nine relevant CPU/WGSL tests pass, including exact pixel coverage
  for sparse, odd-sized and multirow views. Log:
  `C:/dev/perf/warp-sideways-20261024/r16/voxel-active-tiles-tests.log`.
  **No GPU image equivalence or speedup has been measured yet.**
  The one-leaf fixture accepts `--active-tiles true|false` and saves raw
  little-endian RGBA16F volume layers for a later bit-exact comparison.

Joep explicitly wants both a better engine and its application in Sideways.
Improve APIs, performance and iteration time while using the game to expose
real needs. Current work shares typed volume-view bindings between voxel
trees and distant grass, and adds a staged Windows build/restart workflow.
F12 developer captures include same-frame scene GPU metrics. Do not claim
token savings or runtime speed gains without measurements. Shader hot reload
exists for the shading chain only (shade, car_material, transparency,
wind_common: `KERN_SHADER_DIR`, `shot --watch`, live in `play`); the other
shaders and broader frame-resource reuse are still future work.

Car materials (branch `car-materials`, README "Car materials"): procedural
paint (solid, metallic, pearl, candy), wear and grime in
`shaders/car_material.wgsl`, parameters in `data/car_materials.toml`,
reloaded live. Tuned only on lavapipe in the cloud container: check flake
sparkle, swirl arcs and the paint's brightness under TAA on the real GPU.

Joep also wants future GPU wind animation at every foliage distance, and
procedural trees parameterized by growth and season. Preserve one source of
species, seed, branch structure, rest pose and seasonal leaf coverage for near
geometry and distant representations. A shared world wind field and bounded
GPU branch deformation should animate both. Wind phase must be separate from
structural changes to a tree; do not rebuild every cached canopy volume for
every moving leaf. Evaluate conservative motion bounds, sampling a deformed
rest-density field and infrequent shadow updates against actual error/cost.
The r14 first implementation below now supplies shared GPU whole-plant wind
and stage-authored growth/season profiles. Branch structure, biological growth,
seasonal terrain, animated density and per-leaf motion remain future work.

During the 24 October playtest Joep explicitly reaffirmed one common foliage
mechanism for trees, grass and shrubs: near source geometry, derived voxel
groups selected by projected error, shared GPU wind phase/transforms, and
preserved density/color/normal distributions and sun transmittance. Current
four-bin mean grass columns are an intermediate approximation, not that final
system. Avoid building incompatible separate grass and tree animation paths.
Do not reuse the obsolete subagent task name `godot_comparison_baseline`;
its visible completion messages confused Joep despite its actual grass work.
No further Godot comparisons are wanted or being performed.

Joep explicitly stopped further investment in extending the old grass-card
rendering path on 24 October. Its current extended-distance development
profile is a reference only. Continue with shared procedural opaque plant
parts, derived voxel levels, coherent normals/materials and GPU branch poses.
Source trees remain the normal forest; do not claim the bounded voxel fixture
already replaces the whole live forest or grass field.

Current focus and missing systems are now stated in both READMEs: procedural
nature and sky, vehicle physics, AI workflows, compilation/iteration speed;
no human editor, indoor/building/people/water system or general animation
system. Plant-wind work is distinct from general animation. This direction
can change. No new Godot comparison is wanted.

## In progress: shared assemblies, fog and procedural sky (r16)

- The user's far-mountain screenshot is
  `build/screenshots/sideways-1792824184064-268264-0.png`. Its fully fogged
  mountain is flat RGB 145,148,151 while the background varies with direction.
  Materials used a constant horizon average, but the background used the HDRI.
  Shared background/fog helpers now make the full-fog endpoint identical;
  this is coherent HDRI distance fog, not physical aerial perspective.
- Camera-only culling of fully obscured bounds is being validated. Preserve
  shadow/GI/reflection casters and independent split-screen visibility. This
  reduces retained geometry, not necessarily the number of indirect draw calls.
- `foliage_assembly`, `part_wind` and `procedural_foliage_gpu` are the bounded
  next-path implementation: actual blade/needle geometry, branch-local voxels
  from that exact source, common GPU poses and previous/current history.
  Procedural materials share thin-leaf reflection/transmission, source normals
  and voxel normal distributions. A covered-area leaf fraction keeps bark from
  automatically becoming translucent after aggregation.
- Limits remain explicit: 256 volume owners, six normal lobes, approximate
  Beer medium opacity, no whole-world streaming/merged assembly hierarchy,
  no automatic source/voxel handoff, and no DLSS motion integration for these
  animated parts. Old grass cards are not being further optimized.
- Agent `sky_and_weather` owns the opt-in procedural atmosphere foundation;
  see `ATMOSPHERE.md`. Bounded CPU transmittance/sky LUTs supply shared
  direct sun, background and SH9, with cache reuse across stage rebuilds.
  `SIDEWAYS_SKY=procedural`, `SIDEWAYS_SUN_ELEVATION` and
  `SIDEWAYS_SUN_AZIMUTH` opt into shot/play. Defaults remain HDRI. Clouds,
  multiple scattering, aerial perspective and live day/night progression
  remain future work. Do not call this a complete weather system.
  Complete render CPU/WGSL validation: 111 passed, one GPU-only test ignored;
  two procedural-sky CLI/metadata tests passed. Logs are `sky-render-tests.txt`
  and `sky-cli-tests.txt` under the r16 evidence directory. No GPU captures
  have been run for the atmosphere while Joep is playing.
  The release `atmosphere_gpu` example compiles successfully (41.39 seconds;
  `sky-example-build.txt`). Run its executable with `--out FRESH_DIRECTORY`
  only during a free GPU window, then inspect the four actual images.
- Evidence directory: `C:/dev/perf/warp-sideways-20261024/r16`.
  `iteration-summary.json` links the actual GPU reports. The 38-frame native
  960x540 assembly fixture (14 plants, 70 nodes) passed GPU source/volume pose,
  inverse, bounds and previous-pose checks (max pose error 2.69e-7), with zero
  traversal overflows. Both zero-light images are exactly black. This is
  mechanical correctness, NOT visual acceptance: close voxels are blurred,
  coverage is inflated and integration alone costs 6.95-11.00 ms here. Keep
  the prototype out of the live grass/forest replacement.
- Follow-up findings: the heavy integrator dispatches over the whole image
  with ~2.25 KiB dynamically indexed arrays per invocation. Naga initializes
  function locals; driver spilling is only a hypothesis, not measured. Test
  an active-tile worklist before changing optical fidelity. Finest 2.5 cm
  cells are also too coarse for 4.5 mm needles, and the existing surface baker
  counts both sides of closed leaves. A one-leaf projected-coverage oracle
  is needed before changing extinction; do not hide this with tint/density
  boosts. Updated fixture JSON now includes coarse-mip diagnostics, but the
  first report predates those extra fields.
- `fog-a/report.json`: fully fogged geometry and background were pixel-exact
  (0 different pixels), partial fog remained visible, culling removed camera
  triangles/vertices while retaining all six shadow triangles; independent
  camera distances passed. This GPU run preceded the subsequent bilinear
  sky-lookup refinement, whose full GPU regression is still pending.
- Live playtest restored and first visible frame confirmed: PID109828,
  `build/dev-runs/20261024T070554235Z-edce00cd/iteration.json`, exe SHA256
  `50be5339b3c8d779931628b34f40bd23b7d7e81c43c77c875c0c73cedfb122cc`.
  One player, 3840x2160 output, DLSS Quality, wind0.6, summer, extended reference
  grass profile. Fog fix is live; procedural sky/assembly replacement is not.
  Joep is playtesting now: leave this session running while CPU work continues.
  The explicitly named `sky_and_weather` agent owns the atmosphere follow-up;
  the old `road_terrain_seam` name was confusing and is no longer active.

- Follow-up rock/road work is source-ready, **not yet visually accepted or
  present in PID109828**. F12 captures `sideways-1792825863196-109828-0` and
  `sideways-1792826096534-109828-1` show broad flat native cut faces alongside
  the twelve localized detailed exposures, plus a bright road/soil cut.
  Native cliff triangles now receive welded adaptive source refinement only
  at steep slopes and 5-35 m beyond the road edge. Continuous world-space
  fractured relief and curved surface normals are bounded to 0.30 m from the
  unchanged collision triangles; the first 5 m beyond the road edge stays
  exact. Rolling terrain keeps its original topology and fast construction
  path. Existing cluster LOD handles distance, with no extra patch instances.
  Rock mapping now uses arbitrary rotations and fixed 3.7/6.1 m scales,
  and irregular strata replace the periodic vertical sine bands. Road and
  terrain converge to the same world-mapped soil colour/normal at the join;
  road-edge normals now match the native terrain. Handling/colliders unchanged.
  Six geometry tests pass, including bit-exact height/normal agreement and
  identical boundary segments across two neighbouring steep-cliff chunks.
  Existing rock tests 5/5, road geometry/collision tests 3/3 and composed
  shader validation 4/4 also passed. The cliff-heavy CPU fixture increases
  2,048 native triangles to 12,880 source triangles (6.29x in that fixture,
  not a GPU cost claim); scene construction logs native/source/root counts.
  Actual visual quality and GPU/CLOD cost still need a free-GPU capture.
  Evidence: `r16/rock-terrain-geometry-tests.txt` and `r16/rock-terrain.json`.

- **Unified-rock correction supersedes the preceding rock candidate.** New
  F12s `sideways-1792827681185-102808-0`, `1792827777800-102808-1` and
  `1792827988108-102808-4` expose alternating detailed/flat spans and large
  cut-toe forms. The twelve exposure overlays and their runtime `add` API
  are removed. They sampled the old native ground while the rendered ground
  had already changed. The useful fracture field now supplies only the one
  native terrain surface. A second real discontinuity was the old rock
  mapping: variant B used 6.1 m, but became A at 3.7 m when its integer selector
  changed. Rock now has one fixed 3.7 m world projection, restrained continuous
  warp with matching analytic normal/texture gradients, and mild weathering.
  Terrain uv.y stores `1 + macro normal.y` (zero is legacy fallback), preserved
  by terrain CLOD and distant terrain, so small upward-facing rock details do
  not turn into artificial soil/grass patches.
  Relief is expressed as up to 0.24 m normal to the face, converted to a
  heightfield Y offset; rounding stays bounded to 0.16 m Y and the total Y
  deviation is capped at 1.10 m. This replaces the overly shallow 0.30 m Y
  bound. The road guard is narrowed to 2 m beyond the road edge, with 21,894
  actual rendered shoulder contacts verified across three generated stages
  and 12 refined chunks. Eight visuals tests, two shared-geology tests and
  four composed shader validations pass. Exact X/Z chunk boundaries and a
  single terrain material with preserved macro-up through CLOD pass too.
  The cliff-heavy fixture is 2,048 -> 13,109 source triangles. Actual GPU cost
  and matched-capture visual acceptance are still pending. Large cut-toe
  pyramids can also originate in the broad Stage height/ledge envelope; do
  not claim this bounded renderer refinement fully fixes that source shape.
  Evidence: `r16/unified-rock.json` and `r16/unified-rock-*-tests.txt`.

- **Authoritative macroterrain follow-up, same F12 #4.** The broad clipped
  crests were not just low-poly ledges: `Stage::carve` imposed a river-bank
  plane up to 40 m from a river, including its endpoint, then used a hard
  minimum against the cliff. At row 1142 / edge 11 m this cut a 15.58 m
  crest down to 6.80 m; at row 1174 / edge 12 m it cut 22.62 m to 17.98 m.
  The river is respectively about 17 m and 31 m away. No decorative noise
  was added. Dry bank influence now fades continuously between river half
  width + 2 m and + 12 m, with a bounded 0.1875 m soft intersection. The
  wet channel and water levels remain exact. Road + 2 m is protected with
  an additional native-grid diagonal halo, preserving triangles as well as
  point samples. This changes the authoritative off-road height and hence
  collision together; it is not another visual overlay. RNG, road layout,
  river paths and native grid resolution are unchanged.
  Nine Stage tests and three existing road-seam tests pass, including
  78,550 bit-identical road/shoulder heights, 12,568 unchanged native grid
  corners and 1,195 unchanged wet-channel/bank samples. Continuity at the
  protected-bank edge, outer influence and old 40 m boundary is tested.
  On the captured upper-crest camera ray, native-grid error against the
  continuous field drops from 1.250 m to 0.109 m; across 8,262 nearby
  section samples p95 only changes from 0.491 m to 0.483 m, so this is not
  a general grid-resolution fix. The leftmost toe (0.303 m) and right rail
  (0.210 m) ray errors remain. Source frozen; matched GPU review is pending.
  Evidence: `r16/macroterrain.json`, `macroterrain-before/after.json`, and
  `macroterrain-*-tests.txt`.

## Nanite foliage research applied to our next steps, 24 October

Primary sources checked at Joep's request:

- [Epic Nanite Foliage](https://dev.epicgames.com/documentation/en-us/unreal-engine/nanite-foliage):
  the system combines assemblies, aggregate voxels and skeletal wind. Voxel
  substitution follows representation error; its shading retains a distribution
  of surface normals. Animation also needs a screen-size policy because animated
  shadows remain costly. Our fixed-distance mean grass field is not this system.
- [Epic Nanite Assemblies](https://dev.epicgames.com/documentation/unreal-engine/nanite-assemblies):
  reusable parts avoid duplicating geometry. The hierarchy merges distant parts
  so every twig does not remain a separate render instance. Current skeletal
  assembly parts follow bone pivots rigidly; individual part vertices are not
  independently skinned. Procedural variation should preserve part reuse.
- [Heitz et al., SGGX](https://research.nvidia.com/labs/rtr/publication/heitz2015sggx/):
  projected area parameterizes an anisotropic normal distribution, supporting
  interpolation and prefiltering. One averaged normal cannot preserve the same
  directional appearance. This is a useful shading model, not an automatic cure
  for correlated surface coverage becoming a homogeneous medium.
- [Epic's analytic shading status](https://forums.unrealengine.com/t/nanite-foliage-and-twosidedsign/2835970):
  the Witcher demo used analytic SGGX, while that path was not fully integrated
  into the UE 5.8 release. Stochastic normals and two-sided orientation have
  different limitations. Do not equate the released feature and demo or add
  random temporal noise to hide our existing grass transition.

Our implementation decisions, rather than claims about Epic's implementation:

1. Preserve the actual clump silhouette, holes, directional coverage, colour and
   normal distribution across representation changes. The CPU source-sheet
   prototype has better measured opacity fidelity than our Beer medium, but has
   no runtime GPU path yet. Extending source cards is only a controlled reference
   and temporary fidelity experiment, not a finished voxel solution.
2. Shared foliage assets need reusable parts plus a hierarchy that can collapse
   groups. Do not allocate a permanent scene instance per twig throughout the
   entire forest. Select representations using projected error in each actual
   split viewport; share poses and asset residency across views.
3. For visible branch wind, derive a small authored rig from the existing crown
   bundles. A bounded first experiment can use rigid bundle groups, a stiff trunk
   and parent branch pivots. Source assets have no complete branch skeleton;
   this is not a biological growth simulation. Stronger whole-tree shear would
   not meet Joep's request.
4. Before enabling branch animation, retain stable source/bone IDs through CLOD,
   current and previous poses, transformed normals, shared shadow deformation
   and conservative bounds. The current whole-plant inverse-shear motion pass
   cannot reconstruct independently animated branches. Later voxels must inherit
   the same source groups and motion rather than becoming a second tree model.
5. Compare frozen/moving near and far views, sun-facing/backlit views and four
   separated cameras. Measure visibility, animation, shadows and shading as well
   as total scene GPU time. Published demo bone timings do not establish our
   four-player 8.33 ms frame budget.

The current live renderer still uses whole-plant GPU shear and static far-grass
statistics. The CPU shared foliage index and voxel baker are foundations, not
evidence that production Nanite-equivalent foliage or branch wind is complete.

## Original goal

Joep asked for an engine designed around fast AI iteration loops, with the
Sideways rally game (Godot 4.5) as the benchmark: port it, and beat Godot
on graphics, physics and developer velocity. Modern rendering from the
start: Nanite-style cluster LOD, visibility buffer, HDR in physical units,
GI. Keep the loops short and measured (README, "The iteration loops").

## Rules from Joep

- Talk to him in Dutch, without em-dashes. Code, comments and docs in English.
- Open a visible game only for an explicitly requested playtest/demo. Joep
  now requests visible autonomous driving for remote watching. Otherwise
  render offscreen; stop the demo before isolated GPU measurements.
- Commit to `main` of ontola/warp-game-engine and push. Attribute only the
  actual contributors/session; do not copy another agent's attribution.
- When mentioning PRs, link them with a short description.
- Show actual pictures and honest limitations. Compare current Kern variants;
  Joep explicitly ended Godot comparisons on 23 October.

## In-progress Windows r15, 2026-10-24

Latest 24 October steering, road/cliff changes now in running r15e:

- One road type for now. `Stage::make_road_surfaces` now fills every row with
  Gravel; random Dirt/Mud/Asphalt bands were removed. The same data drives
  visuals, tyre grip and AI. A generated-stage regression across three seeds
  and 0.8/3/8 km verifies row/AI/wheel agreement. Loose shoulders, offroad grass
  and physical river water remain their own ground responses. Route-generation
  RNG is unchanged because the removed bands used a separate private RNG.
- Future road types need a geographical reason: mud from wet soil in rainy
  lowlands; paved/unpaved transitions at a real branch/junction, with a blocked
  unused branch retaining one race route. Road width can change with road type.
  Do not bring back arbitrary distance bands or implement junctions prematurely.
- F12 `sideways-1792820558094-57532-0` shows uniform far grass. CPU terrain rays
  locate sampled flat areas at 73-111 m and near/far transition at 53-57 m
  (`r15/grass-r15d-capture-probe.json`; ignores foreground objects/render CLOD).
  The live global four-bin mean field still loses individual clump structure.
- `r15/grass-clump-voxel-cpu-b` compares real projected sizes: the area-density
  Beer medium retains only 81-87% source opacity. An exact-source-sheet
  coverage-mip candidate gives 100.5-101.5% at the screenshot's 73-111 m Quality
  views, but has residual small-viewport error up to 7%. Fourteen CPU tests
  pass. No runtime switch: a generic API must distinguish surface coverage
  transport from a homogeneous medium; conserved area is not conserved opacity.
- F12 `...57532-1` and `...57532-2` show repetitive cliff-toe pyramids. The 2 m
  native grid undersamples a 9-22 m cliff rising in 4.6 m, before render LOD.
  The revised toe/crown varies coherently over roughly 6-9 m; ledge strength
  now interpolates continuously between road rows. Full cliff height and steep
  faces remain. Screenshot #1 source-grid error p95 fell 0.969 to 0.467 m,
  maximum 1.545 to 0.747 m. This reduces, not eliminates, the coarse-grid error.
  Screenshot #2 also contains a sharp river carve, which remains unresolved.
  All 24 Sideways unit tests pass, including bitexact drivable foundation,
  shoulder/collider, original main-road hash and the new road-surface checks.
  Evidence: `r15/cliff-sampling-source.json`, `cliff-sampling-profile-a.json`
  and `cliff-sampling-profile-a-dry.json`. No new GPU acceptance claim.

Current r15e manual playtest reached its first visible frame after a managed
restart: `build/dev-runs/20261024T060113217Z-6d2a4397/iteration.json`,
PID 194472 at launch. One player, 3840x2160 output, DLSS Quality, wind 0.6,
summer. Archived executable `C:/dev/perf/warp-sideways-20261024/r15/sideways-r15e.exe`,
SHA256 `0086bcc162b7f2fcf22bb32e6ed220d03dd8c30f176478544631d50e8dfd0496`.
Build 92.49 s; startup 20.76 s. The road is now continuous gravel and the cliff
toe/profile is revised. Grass rendering remains the r15d mean-field path.
First-frame confirmation is execution evidence, not visual acceptance.

The new CPU-only `render::foliage_groups` index is source-complete and its six
tests pass (`r15/tests-foliage-groups.log`). Shared bounds, stable identity and
generation, inverse poses, conservative wind/filter bounds, explicit memory
limits and half-open DDA segments are representation-independent. Asset,
instance, cell and owner-reference records are 64/144/32/4 bytes. It has no
runtime/GPU integration yet. Its final preallocation-budget correction happened
during the r15e release build and is not claimed to be present in that executable;
the module is unused by r15e rendering either way. No further source changes
occurred after its final targeted tests.

Previous r15d manual playtest reached its first visible frame after a managed
restart (user explicitly requested the latest version when ready):
`build/dev-runs/20261024T054027200Z-7193dab5/iteration.json`, PID 57532 at launch.
One player, 3840x2160 output, DLSS Quality, wind 0.6, summer. Archived executable
`C:/dev/perf/warp-sideways-20261024/r15/sideways-r15d.exe`, SHA256
`5849dbbe00da53d267289a62dc425b1b560cbac859d53baa3286ca7e128a572a`.
First-frame startup confirms execution, not visual/performance acceptance.
No isolated GPU benchmarks ran concurrently with the manual playtest.

Changes in r15d:

- Trees share three source archetypes (canopy/conifer/stout) and deterministic
  juvenile scale. Missing C-trunk UVs use cylinder unwrap and tileable bark.
  Textured wood LOD retains normals/UVs and bounds newly reversed faces below
  two projected pixels. This retains more near wood triangles; cost unmeasured.
- Terrain has shared world-halo normals and attribute-aware LOD, including a
  source-triangle lock within 5 m of the road. The outside 80 cm road shoulder
  follows actual terrain triangles with a 3 mm overlap; main lane unchanged.
  CPU ray/geometry checks pass; see `r15/road-seam-cpu.json`.
- Terrain texture variants keep physical scale and blend world-anchored rotated
  offsets instead of fading to a 4.3x enlarged texture with camera distance.
  Cliff relief uses the same terrain material and normals blend at the actual
  terrain intersection. Fully rocky pixels skip zero-weight soil samples.
  Painted green road-center streaks were removed. GPU visual review pending.
- Grass clump data is now derived from the exact shared near source through
  `render::foliage_voxel`, preserving local coverage/color and stable grouped
  placement. CPU fixture `r15/grass-clump-voxel-cpu-a` has one 365,768-byte
  shared asset, seven mips and 145 exact live plants. Twelve tests pass.
  Finest 2 cm cells cover only 82-84% of the source silhouette in three
  directions; fix projected-opacity fidelity before runtime acceptance.
  This is CPU/offline work; live far grass remains the mean field.
- `sideways::wheel_fx` is a tested material/slip/load-dependent spawn API at
  120 Hz, bounded to 16 wheels and 96 births/tick. It is not yet connected to a
  GPU particle pool or live play. Do not claim visible debris/dust is done.
- `chase_camera.rs` holds <=1.5 cm/0.03 degree body rest noise only below
  0.03 m/s and releases on driving/turn/teleport. Unified raw right-stick
  input uses 0.24/0.16 radial hysteresis and wrapped yaw; four CPU regressions
  pass. Vehicle physics and driving inputs are unchanged. This addresses the
  measured neutral drift, not proof that every reported wiggle is resolved.

Joep is actively playtesting. Keep the current session running and do not run
competing GPU benchmarks. The previous managed session
`build/dev-runs/20261024T050859254Z-cb791f7d/iteration.json` reached its first
visible frame at 3840x2160, one player, DLSS Quality, wind 0.6 (PID 171044 at
launch) was gracefully stopped for r15d. It ran the r15c executable below with
all three grass constructor flags enabled, as r15d does. Its controller
is connected through XInput as `Xbox Controller`. Revalidate the PID/path/start
identity in `build/dev-runs/active.json` before any later restart.

The current r15 work is uncommitted on top of r14. Earlier tested executable:
`C:/dev/perf/warp-sideways-20261024/r15/sideways-r15c.exe`, SHA256
`4bf8ff48eaf151a6626b64b6f10c335631b0bded4c2db1d1c62d3b5e701fd265`.
141 CPU tests passed, with one unrelated ignored benchmark (`tests-c.log`).
All 22 actual GPU motion cases passed (`motion-c/report.json`). The c build
adds alpha-aware linear cutout RGB filtering, a shared inclined-leaf grass
BRDF, and bounded local grass sun transmittance. Tree archetype runtime
changes are being prepared separately and are NOT in this executable.

One short, same-binary 4K one-view DLSS Quality grass A/B (`grass-old-route`
versus `grass-new-route`) measured 25 frames after ten warmup frames, with
identical car/camera traces. Instrumented GPU sum increased from 15.465924
to 16.445108 ms (+0.979184 ms). All three grass flags changed together, so
this is neither an isolated feature cost nor a robust performance result.
The new image was reviewed before manual launch. HUD, simulation,
presentation and uninstrumented work are excluded; no gameplay FPS claim.

The 60-image `grass-c` fixture completed, but its default 2x1 constant sky
exposed a separate SH integration bug: fixed step-4 sampling aliases it to
one horizon direction. The predicted inclined-leaf/UP irradiance ratio
5.05157 matches measured far indirect HDR 5.05337. The game's 2K HDRI does
not have this specific undersampling problem. The fallback now integrates
small sky texels analytically (CPU constant/hemisphere tests pass); rerun
the fixture before using its lighting measurements as acceptance evidence.
The albedo-only mismatch narrowed, but this does not prove all grass pop-in
or temporal artifacts are solved. No additional GPU runs during playtesting.

Earlier r15b executable SHA256 was
`cc8f8060f8b9e334c7b9db54806597a2604bcb0f00067b0c51853d2b40568fcd`.
Real NGX DLSS Quality completed 30 moving frames at 853x480 -> 1280x720 in
`first-dlss-b.png` / `first-dlss-b-state.json`; there was no fallback. The
visible 4K session has also submitted frames successfully. This is functional
evidence, not a four-player performance or image-quality acceptance result.
The earlier r15a failed before evaluation because wgpu 30 forbids mixing raw
and tracked commands in one encoder. r15b uses separate ordered command buffers
in one queue submission. Preserve failed and successful evidence separately.

DLSS setup: `NVIDIA_DLSS_SDK=C:/dev/tools/nvidia-dlss-sdk`,
`VULKAN_HEADERS=C:/dev/tools/vulkan-headers-dlss`,
`KERN_DLSS_RUNTIME_DIR=C:/dev/tools/nvidia-dlss-sdk/lib/Windows_x86_64/rel`.
Build with `--features render/dlss`, target `C:/dev/warp-target-20261023`.
Native remains the default. CLI/helper/GameNight accept `native`,
`dlss-quality`, `dlss-balanced`, `dlss-performance`. The helper records the
official runtime hash; see `crates/render/native/DLSS.md` for SDK/license and
integration details. Experimental voxel crowns are rejected with DLSS until
they have suitable motion data. HUD remains at output resolution.

`r15/tests-b.log`: 132 CPU/shader tests pass (52 CLI, 70 render, 10 game).
`r15/motion-a/report.json`: 20 GPU motion cases pass, max error 0.003795 pixels.
The rebuilt motion probe adds two cases and corrects merged depth on reset;
the final 22-case run passed in `motion-c`. `render/examples/dlss_probe.rs`
is a lifecycle/raw-output fixture that still needs execution.
Remaining DLSS validation: lifecycle, missing-runtime
native/error paths, real 4K/four-view modes and same-route native/DLSS timings.
Do not claim a speedup, full visual acceptance or the 120 FPS target yet.

Latest user grass report: darkening/pop-in around 50 m. The normal single-view
handoff is 40-60 m, split-screen 20-38 m. Far grass is already a terrain-following
density volume. `r15/grass-a` contains 60 real GPU captures of the same resident
photo-grass patch, five distances, near/far/blended, and four lighting components.
At 50 m near coverage is 0.98873 and far alpha is 0.99820, but their colors differ
substantially. Investigation is focused on texture filtering and shading, not
adding density or arbitrarily darkening the far field. This sunny fixed-patch
fixture does not yet certify forest shadow parity. Production grass has no
full visual acceptance yet; r15c contains the first correction below while
the statistical near/far handoff itself remains in place.
`Lighting.grass_reference=1/2` forces near/far only for diagnostics; use 0 for
normal play and performance measurements. Finish validation, document results,
then commit/push main. Joep has authorized a managed restart when the next
coherent iteration is ready; avoid concurrent GPU tests during manual play.

The grass RGB filtering candidate is now in `render/src/cutout_mips.rs` plus
the small `texture_array` hook. Five CPU oracles pass, evidence
`r15/grass-a/cpu-candidate.json`. Existing alpha/coverage is preserved byte for
byte; only actual sRGB cutouts use alpha-weighted linear RGB from the original
image, with color extended into transparent texels. This is in r15c; the
combined runtime comparison and remaining fixture limitation are above.

Actual user F12 references, both from the running r15b DLSS Quality session:
`build/screenshots/sideways-1792817001611-266592-0.png` and
`build/screenshots/sideways-1792817202919-266592-1.png`, with JSON sidecars.
The first looks across a wooded slope; the second clearly shows nearly-white
grass facing the sun. Both near/far lighting currently use one UP normal and
coherent GGX, and grass is excluded from shadow casting. The mean density field
only attenuates viewing rays, not sun rays. The RGB filtering fix alone cannot
resolve these lighting errors. A shared grass normal-distribution and local
density sun-transmittance candidate is being developed separately; its GPU
cost and visual quality are unvalidated. Avoid simply darkening grass or
promising that a particle system will fix transparency/lighting costs.
The first capture measured 1.3905 ms in the dedicated near-grass pass and
0.1791 ms far volume at 2560x1440 -> 3840x2160. Main raster/shade also contain
grass work, so these are NOT the complete cost of grass or a native/4P result.

The source lighting correction is in r15c and has run on the GPU, with the
limited comparison above. `grass_lighting.rs` and `grass_lighting.wgsl` share one
projected-area weighted, two-sided blade-normal response between opaque,
transition and far grass. Source color, F0 and roughness are retained. Four
quadrature directions follow view azimuth rather than fixed world axes;
this is a macroblade approximation, not a measured leaf-normal distribution.
Near/far grass and terrain/road also query sunlight extinction through the
existing four-height-bin grass field, with the native terrain diagonal and
road clearance. The trace is explicitly bounded to eight cell/tile visits
and 16 m; empty tiles are skipped and omitted space remains unoccluded.
No individual grass shadow geometry or full-resolution target was added.
This does not animate the far density or replace the statistical field with
source-derived voxels. Sky visibility still uses one shared canopy query.

Constructor-time ablations: `KERN_GRASS_LEAF_BRDF=0` restores the single-UP
normal response; `KERN_GRASS_SUN_OCCLUSION=0` disables the new medium shadow.
Both default on unless their value is exactly `0`. Shadow strength zero also
disables the medium shadow. `KERN_CUTOUT_LINEAR_RGB=0` independently restores
the old texture RGB filtering. Use the same executable/camera/source patch
for baseline (all three zero), mip-only, leaf-response-only, medium-shadow-only
and combined images/timings. The eight-visit per-pixel trace is not assumed
free; measure main shade, transition grass and far-volume costs separately
before accepting the candidate. The 24 October user subsequently authorized
launching the updated playtest when the coherent candidate is checked.

## Windows continuation r14, 2026-10-24

Current graphics executable: `C:/dev/perf/warp-sideways-20261024/r14/sideways-r14a.exe`,
SHA256 `2f9f4ef830df9b8d630e90da7c943000bb2a321f0b88f28731140c7e58ddf7a3`.
The previous r13 binaries/evidence remain intact. User asked to optimize the
budget and work on wind, growth and seasons. No Godot comparison is required.

Geometry work is now bucketed into 8/32/64/128-triangle indirect draws. Original
visibility IDs and LOD policy are retained; live grass rejected by the existing
opaque fade is culled before drawing. Per-camera cluster buffers persist.
`KERN_CLUSTER_BUCKETS=0` and `KERN_EARLY_GRASS_CULL=0` are constructor-time A/B
switches. Per-frame metrics distinguish actual submitted vertices from retained
shadow geometry. Buckets require up to 16 MiB additional mapping storage per
view/map (144 MiB upper bound for four main + four near + one shared sun map).

Native 4K/four-player ABBA, 80 measured route frames per run after 20 warmup:
GPU scene-pass mean 39.4352 -> 34.4169 ms (12.73% less), raster 19.5981 -> 13.3308 ms.
Sorting raises cull cost; the net improvement includes it. Exact car/camera
traces match. Final captures differ in one pixel by at most 5 RGB8 levels, also
seen between repeated baseline runs. This is bounded route evidence, not full
gameplay FPS or proof over all routes. 120 FPS remains unmet.

`render::wind::WindState` + `Renderer::set_wind` drive a single compute field
shared across source geometry, every camera/LOD and depth shadows. Roots stay
fixed; normals and CLOD bounds account for the shear. This is whole-plant sway,
not skeletal branch animation. Canopy/far-grass densities remain mean rest-pose
fields and experimental packed voxel-tree owners stay static. Wind does not
invalidate the canopy; source sun shadows update with changed wind. The GPU
differential fixture matches CPU-baked geometry and direct shadows exactly.
Separate four-view wind ABBA: 34.4684 -> 34.6230 ms at strength 0.3, +0.1546 ms;
the shared compute itself is 0.0238 ms. Not an isolated total GPU-clock claim.

`--season spring|summer|autumn|winter --growth 0..1 --vegetation-seed N`
select a stage phenotype. Shared native tree Spots keep colliders aligned;
near/far grass use the same stature and tint. Pines stay evergreen. Growth is
authored juvenile-to-mature scale, not procedural branch generation; terrain
does not yet recolor with seasons, and there is no snow. Summer/growth 1 keeps
the prior geometry. CLI play/shot now default to gentle `--wind 0.3`; engine
and existing gpu-run jobs default to zero. `--wind-time SECONDS` freezes phase.
`--gpu-profile NEW.jsonl --gpu-warmup N` records per-frame route/GPU evidence;
it synchronizes for readback and excludes HUD/presentation/CPU from pass sums.

Use `tools/play-dev-windows.ps1 -Restart -Autoplay` for the visible remote demo.
It accepts `-Season`, `-Growth`, `-Wind`, `-WindTime` and `-VegetationSeed`, and
records the options in its staged run manifest. Keep far grass enabled and
experimental voxel crowns disabled. Stop the owned demo before GPU tests.
Full evidence and remaining limitations are in WINDOWS_VALIDATION.md.

## Previous Windows continuation r13, 2026-10-24

Latest r13c build SHA256:
`c8092e3def8f464949cc6fc10738e1ae9828604a4f1a46bd388aa96212851565`.
The r13 graphics iteration adds persistent per-camera canopy clipmaps for
distant sun/sky occlusion, plus source-color/relative-height moments for far
grass. One shared canopy was rejected because separated players changed
lighting. Each view now has its own 1.5/6/24 m field levels, 27 MiB per view,
108 MiB for four. Metadata reports per-view pitches and dirty masks.
The canopy is a static ellipsoid-density approximation, not source-leaf
voxel geometry, animated trees or a spatial radiance cache.

The corrected 55-frame GPU fixture passes, including high forests, distant
sunlight, split camera invariance and the former spread-threshold jump.
12 grass and 11 tree-volume optical cases pass. The r13b graphics executable
ran native 4K one/four-view 100-frame tests with matching r12b player states.
Final scene GPU pass sums were 20.240/38.883 ms versus 17.977/36.954 ms before;
this quality improvement costs GPU time. These are captured-frame timings,
not gameplay FPS. The 120 FPS target remains unmet. r13c only raises the
validated tree-count capacity to accommodate the existing 8 km stage range;
the 8 km scene itself has not been measured. Read WINDOWS_VALIDATION.md.

Keep `SIDEWAYS_FAR_GRASS=1` and `SIDEWAYS_VOXEL_TREE_PATCH=0` for the requested
visible autoplay. Restart through `tools/play-dev-windows.ps1`; its active
manifest owns the current PID/path. Stop it before isolated GPU measurements.
Motion shimmer, approximate far silhouettes and broader route/weather
coverage remain open. Future GPU wind/growth/season requirements are above.

## Previous Windows continuation, 2026-10-23

Previous r12b validated candidate SHA256:
`05ec994693442cc72ec7d2cb10641666a616fa0d13db25138a860e75c479097e`.
The visible demo is now managed by `tools/play-dev-windows.ps1`; use its
`-Restart -Autoplay` workflow and inspect `build/dev-runs/active.json`.
It runs with `SIDEWAYS_FAR_GRASS=1`, `SIDEWAYS_VOXEL_TREE_PATCH=0` and NVIDIA.
The new far grass fills the former missing representation past 60 m; it
remains opt-in because open slopes look too smooth and the field has a fixed
allocation bound. 101 CPU/shader/oracle checks and 12 far-grass / 11 tree
GPU optical cases passed. Paired 100-frame 4K one/four-view smoke runs passed;
final-frame far-grass overflow was zero in all views. This is not full-route
temporal acceptance. Same-frame far pass samples were 0.190 ms single,
0.768 ms combined four, and 1.190 ms on a later open-slope live capture.
The four-view scene sample total was 36.954 ms excluding HUD/presentation/CPU;
120 FPS is not achieved. Read the r12 section of WINDOWS_VALIDATION.md.

Read `WINDOWS_VALIDATION.md` for measured results, evidence and limitations.
The `play` command now provides a Windows driving loop, native XInput,
GameNight lifecycle/controller transport and one to four rendered views.
Only explicit `play` opens a window. Managed preparation stays invisible;
automated GPU validation uses `play --smoke-render N` and reports zero
visible presented frames. It is not a substitute for a visible playtest.

Windows uses gilrs's XInput backend. The attached 8BitDo Ultimate is named
"Xbox Controller" by that backend. A physical input probe recorded both
sticks, full throttle/brake, handbrake, recovery, camera and Start.
The installed GameNight Preview daemon speaks protocol 1 / SDK 5, so
SDK 6-only actions and room logs must be capability-gated.

The photographed grass uses shared world cells and terrain-triangle roots.
Frozen shots have a 60,000-instance cap; live play streams cached candidates
into a fixed 70,000-slot pool for one view or 115,000 shared slots for multiple
views. Existing plants keep their poses/sizes; only changed slots upload.
The r9 grass uses static habitat density, retaining every tuft and all three
cards. Near grass stays opaque; a nearest-layer opacity pass fades from
40-60 m in one view or 20-38 m in split screen. CPU residency extends beyond
that fade. This is not a distant geometry LOD yet. Deeper partial grass
layers are omitted; glass ordering remains approximate and live TAA stays off.
The private HDR/depth targets require about 95 MiB at combined native 4K;
measure the extra GPU passes before making performance claims.
`SIDEWAYS_LIVE_GRASS=0` disables it for comparisons. Live TAA is still off
until per-instance motion vectors exist. The native-resolution HUD now uses
glyphon and shared cached Barlow text, with timer, instruments, pace notes,
progress, recovery, countdown and results. Audio, an in-game settings menu,
particles and the remaining ground cover are still missing.

One 4K output has four 1916x1076 views with an eight-pixel gap. The 120 FPS
target has not been met. Frozen GPU pass timings are not live-game FPS.
Keep the Godot source beside `engine/` as the unchanged reference.

The Windows lighting update fixes indirect visibility around cars,
direction-aware glossy reflections, locally conditioned ground bounce,
absolute shadow-texel snapping, filtered world-aligned canopy density and
reversed stone-wall normals. Read the lighting section of WINDOWS_VALIDATION
before claiming full shadow stability or 120 FPS. The subsequent r5 playtest
adds forward glTF glass transparency, smooth terrain visibility normals and
a bounded distant forest on rolling background hills. The r7 default reuses
the three actual near-tree meshes and their cluster LOD to retain species
identity; the original exposed-face voxel pines now require
`SIDEWAYS_GENERIC_VOXEL_TREES=1`. The hills have higher warped skyline ridges.
The following density iteration measures actual native trees per eligible
horizontal terrain area and matches that density on the hills, bounded to
200,000 background instances with any shortfall explicitly logged. This uses
the current cluster renderer, not Unreal Nanite.
The separate sparse density-voxel bake is still experimental and inactive in
play. Its second CPU comparison reduces oblique coverage error using 13
directional projected-area samples, but coarse levels remain soft and this
has not been accepted as a near-tree replacement.
Joep explicitly prioritizes runtime source-derived voxel trees next. Preserve
the current near crowns and species identity; procedural plant generation is
later work. Do not call the existing alpha-card cluster renderer voxel foliage.
As of the late 23 October session, Joep no longer wants Godot comparisons:
evaluate current Kern mesh and voxel versions against each other. He is done
playtesting tonight but subsequently asked to watch gameplay remotely. An
explicit `play --autoplay` uses the existing race AI and keeps driving without
focus; it does not override managed GameNight inputs. Show occasional actual
images in chat. Stop the visible demo before GPU measurements and restart it
afterwards; do not mix its GPU load into comparison timings.
Joep requested quick build/restart rounds with his live visual feedback. Do
not run competing GPU benchmarks during his playtest. Build/shader checks
and first-frame confirmation remain appropriate; do not imply a full GPU
acceptance suite or a 120 FPS result for these rapid iterations.

F12 in live play writes the current final 4K (or chosen window size) composite,
including HUD, to `engine/build/screenshots/sideways-<timestamp>-<pid>-<serial>.png`.
A JSON sidecar records the camera, cars, stage, settings and adapter. Windows
also copies the image to the clipboard for the user to paste into the chat.
This does not transmit anything automatically or advance the simulation.

## Earlier renderer baseline

- Simulation: stage, terrain, road, barriers, cars (Rapier), AI and race
  loop ported. Stage layout matches Godot bit for bit; handling bench is
  within about 1%. See README "Parity".
- Renderer (`crates/render`), all headless wgpu 30:
  - cluster LOD DAG (meshopt clusterlod) and GPU cull;
  - visibility buffer and compute shading;
  - sun shadow map (cached while nothing moves) and canopy-volume GI;
  - TAA plus CAS sharpening;
  - textures: 2K albedo and 1K normal-map arrays, sampled with
    ray-differential gradients and 16x anisotropic filtering;
  - materials: GGX, sky reflections, clear-coat paint, Godot's foliage look.
- Joep's last verdict before the latest fixes: "very pixelated, especially
  the trees, ground and wall very blurred". That is fixed now: TAA,
  anisotropic gradients, normal maps.
- The cloud container only had lavapipe (CPU), about 40 s for a 16-frame
  TAA shot. On Windows you have a real GPU: use it, the loop should be
  milliseconds.

## Setup on Windows

    winget install Rustlang.Rustup LLVM.LLVM Git.Git
    rustup default stable
    git clone https://github.com/ontola/warp-game-engine C:\dev\warp-game-engine
    cd C:\dev\warp-game-engine\engine
    cargo test -p render          # shader validation, no GPU
    cargo run --release -p cli -- shot out.png --seed 1 --km 3 --at 300 --weather clear --size 1920x1080

`engine/.cargo/config.toml` sets clang + lld for Linux. If linking fails on
Windows, scope that config to the Linux target (or use `rust-lld`). Do not
just delete it: the Linux container needs it.

`KERN_ADAPTER` picks a GPU adapter by name substring. The shot prints the
adapter and per-pass GPU ms from timestamp queries. `gpu-run` (see
GPU_RUNNER.md) runs `gpu-jobs.toml`. The polling runner described there is
not needed now that you run on the GPU machine itself.

Godot reference frame: `tools/shot.sh out.png --frames 30 --seed 1 --at 300
--cam chase` from the repo root (needs Godot 4.5 in PATH). It renders
offscreen.

## Ground-cover reference and remaining work

This is the biggest visible gap with Godot: the verge is bare, with a
mown-lawn tiling, where Godot has grass tufts and berry shrubs. Port
`src/grass_field.gd` and its shaders (`grass_cards.gdshader`,
`grass_field.gdshader`, `vegetation_density.gdshaderinc`,
`grass_coverage.gdshaderinc`), plus `src/ground_cover.gd` (ferns, mossy
rocks, stumps; the chunks already carry `ferns`, `rocks`, `stumps` and
`grass` spots in `builder.rs`).

What Godot does:

- **Carpets.** Rings of tufts around each camera, built on a world-cell
  grid with `[kind, cell, inner, radius, grow]`:
  - high detail: grass at (0.3 m, 0-20 m), (0.7 m, 18-60 m) and
    (1.6 m, 52-110 m); shrubs at (0.55 m, 0-30 m);
  - medium and low presets for 2 and 3-4 views.
- **Placement per cell id.**
  - jitter: `(hash(id), hash(id+17)) - 0.5) * 1.6` cells;
  - size: `grow * mix(0.7, 1.3, hash(id+9))`;
  - angle: `hash(id+3) * 2π`;
  - hash: `fract(sin(dot(p, (127.1, 311.7))) * 43758.5453)`.
- **Density, from the ground map** (height, metres from road edge, normal Y):
  - grass = `max(verge, wood) * (1 - shade * 0.45) * level`, where:
    - verge = `smoothstep(0.5, 1.4, d) * (1 - smoothstep(10, 22, d))`;
    - wood = `smoothstep(0.3, 0.6, noise(xz/7)) * 0.85 * smoothstep(1, 3, d)`;
    - level = `smoothstep(0.7, 0.82, up)`.
  - shrubs: patch noise, times `smoothstep(2.5, 6, d) * (0.7 + shade * 0.5)`.
  - `shade` is Godot's canopy map (a soft shadow per tree). Approximate it
    from tree spots, or from our canopy volume.
- **Coverage.** Each tuft is dissolved by a fixed per-tuft hash pattern
  (`grass_coverage_sample`), with smooth ring crossfades.
- **Grass mesh.** 3 crossing cards, 0.50 m high and
  `0.36*(257/135)` m wide. The UV is an atlas sub-rect of
  `assets/grass/grass_medium_01_diff_2k.jpg`, with the alpha in
  `grass_medium_01_alpha_2k.png` (red channel); combine them into one RGBA
  layer. Atlas min `(580.5, 889.5)/1024`, max `(836.5, 1023.5)/1024`,
  v flipped.
  - albedo: `tex * mix(0.8, 1, h) * 1.25`, normal straight up;
  - AO: `mix(0.45, 1, h) * (1 - canopy * 0.5)`;
  - translucency: `(0.12, 0.16, 0.05) * max(-n·l, 0)`.
- **Shrub mesh.** Procedural, `_shrub()`: 4 twigs × 6 leaves, RNG seed 11.
  - colour: `mix(green, dry, smoothstep(0.5, 0.75, tone))`, with green
    (0.2, 0.32, 0.1), dry (0.52, 0.15, 0.07) and
    `tone = noise(xz/5)*0.6 + hash(id+2)*0.4`;
  - Godot colours are sRGB: convert them to linear.
- **Wind.** Tips sway; there is no time in shots yet, so it can wait.

Original design, first step now implemented in `cli/src/ground_cover.rs`:

- **First step: build the tufts as instances on the CPU around the
  cameras, in `cli/src/visuals.rs`.** Make the decision for a fractional
  coverage per tuft: keep a tuft when `hash < coverage`.
  - Each tuft is one tiny cluster, so ~60k instances is fine for the cull.
  - This needs a heightfield with road distance and normal Y: extend
    `heightfield()` (the chunks have `heights` and `road_d`).
  - Add material flags for grass cards and shrubs. In `shade.wgsl`, derive
    tuft height from the instance origin (`inst.r1.w`) and `inst.scale`,
    and per-tuft tone from the instance origin.
- **Future design: GPU-driven.** A compute pass can spawn visible tufts into
  an instance list to reduce CPU streaming work. Split screen still has
  per-view culling, raster and shading costs; measure them independently.

## After that

- **Per-instance motion vectors for TAA.** Only camera motion is
  reprojected now, so moving cars would smear. Shade knows the instance;
  keep a previous-transform buffer and write velocity.
- **Complete the playable loop.** Moving instances, shadow invalidation,
  native input, GameNight and explicit window presentation now exist. Next
  are audio, settings, additional small vegetation and scene performance work.
- **Visual differences with Godot that remain:**
  - distant forest denser and darker than Godot's (Godot uses mid meshes
    and impostors beyond 150 m, `src/forest.gd`);
  - canopy GI darkens crowns a lot (try `--set gi=0`);
  - the rock wall's vertical streaks come from a faithful port of Godot's
    terrain shader; full triplanar would fix them in both.
- **Also from the game:** car lights, dust and gravel particles, wet roads,
  tyre marks (`track_map.gd`).
- **Performance on the real GPU:** measure with `gpu-run`. Shadow raster
  was the heaviest pass on lavapipe. Sim cost: Rapier step 175 µs and car
  forces 120 µs per tick.

## Handy

- `shot --pick x,y` says which instance, mesh, cluster and triangle covers
  a pixel.
- `--set key=value` retunes `Lighting` without rebuilding:
  - light and look: ev100, sun_lux, sky_lux, sun_elevation, fog_*, gi,
    debug=1 (sky visibility), debug=2 (albedo);
  - TAA: taa, taa_blend, taa_gamma, sharpen;
  - foliage: alpha_cut.
- `--taa 1` gives one frame without TAA.
- Every new WGSL file goes in the `shaders_validate` test in `render/src/lib.rs`.
- Traps already hit are listed in README. Two more:
  - In the view uniform, `params.z` is the ortho flag for culling; the alpha
    threshold lives in `extra.x`.
  - `shot` takes the output path positionally; it has no `--out`.
