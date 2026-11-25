# Wind gusts and branch resonance in the 100,000-tree forest

The playtest reported that maximum wind looked like a light breeze. The old public forest was package `e2c974345130bb6a`, museum commit `0ac7d964`. At 100% it applied at most 0.055 m of horizontal lean per vertical metre. A shared front already travelled between trees, but its dominant crest passed at a regular interval and distant crowns barely moved on screen.

## Wind through the planting

A storm samples one deterministic weather field at the tree's position. Within a 32-minute interval there are 128 fronts of different strengths and durations. Some have two crests. Wind pressure changes across each front. A weaker flow continues between them. The forest preset reaches the renderer's existing maximum shear of 0.32 and fades to rest at a horizontal distance of 1,500 m. Trees use the same field for their geometry, shadows and branch animation. The browser pack reuses the planted roots, foliage and bark of the previous release.

For one 18 m tree rooted at world X/Z (32, 0), the peak whole-crown displacement over the full cycle went from 0.98 to 5.74 m. The new median was 1.34 m and the 95th percentile was 4.68 m. The root remained fixed. A tree 24 m directly downwind sampled the same front two seconds later. The correlation between the two wind traces at roots four metres apart across the wind was 0.984. These values describe whole-crown motion. Articulated branch rotations are measured relative to their moving parents.

Epic's Dynamic Wind derives bone-level parameters from vegetation geometry and separates spring-like sway from finer flutter. Warp uses the stiffness from the grown joints and evaluates a replayable response to its spatial wind field. [Epic's Dynamic Wind for Nanite Foliage](https://dev.epicgames.com/documentation/unreal-engine/dynamic-wind-for-nanite-foliage-in-unreal-engine)

## Visible canopy motion

The native comparison used the same planting of 100,000 Norway spruces, a fixed camera at 60 m altitude, one 1440 by 900 view, one raster sample and native temporal AA on an RTX 5070 Ti. Over 11 seconds we measured the horizontal travel of the centroid of visible pixels in the top third of each resolved mature crown. Trees clipped by the frame and crowns with fewer than 12 pixels in any sample were excluded. The distance is measured from the camera.

| Distance | Median crown travel, old Storm | Median crown travel, new Storm |
| --- | ---: | ---: |
| 120–250 m | 2.93 px | 10.13 px |
| 250–400 m | 2.18 px | 5.77 px |
| 400–650 m | 1.67 px | 4.44 px |

The synchronized eight-second flight comparison is stored at `C:/dev/perf/2026-11-25-forest-storm/storm-in-100000-fijnsparren.mp4`. The two panels use the same crop and wind clock. In two 120-frame GPU measurements with this camera, the new storm took 4.83 and 5.67 ms at the median and 8.38 and 8.57 ms at p95. At 86 m altitude the medians were 5.55 and 5.51 ms, with p95 of 7.35 and 6.89 ms. The 60 m view exceeds the 8 ms target at p95.

## Springs on grown branches

The shared renderer is on [Warp main](https://github.com/ontola/warp-game-engine/commit/fe835d59) and in museum revision `237730d3`. Each active near-tree rig samples four seconds of wind pressure at its root. Each flexible joint evaluates a damped spring against that trace and bends about the pivot on its moving parent. Rooted stand batches sample the travelling pressure directly. Their whole-crown motion can be seen hundreds of metres away.

Both mature specimens provided 63 flexible joints and one fixed root. In each specimen the 63 flexible joints were sorted by their grown stiffness into groups of 21. Frequency was measured from the response to an identical pressure pulse. The bend and angular speed in the table are parent-relative measurements from a separate, continuous 32-second run with the **browser strength slider at 8%**.

| Mature Norway spruce | Resonant frequency | Local bend, p95 | Angular speed, p95 |
| --- | ---: | ---: | ---: |
| Thick boughs | 0.91 Hz | 0.44° | 0.61°/s |
| Intermediate branches | 1.50 Hz | 1.09° | 3.71°/s |
| Thin branches | 1.81 Hz | 1.91° | 6.55°/s |

Maritime pine measured 0.90, 1.51 and 1.75 Hz for the same three groups. The spruce root stayed fixed and the largest measured error in any attachment was under 0.011 mm. The p95 angular difference between the thick spruce boughs and their instantaneous wind-driven equilibrium increased from 0.003° to 0.162°.

The close-up test animated 32 real tree rigs at the same resolution on the same RTX 5070 Ti. The CPU preparation for a storm frame took 0.57–0.65 ms at the median over two runs, up from 0.27–0.32 ms. GPU frame time was 5.72–5.88 ms at the median and 8.28–8.62 ms at p95, compared with 5.88–6.21 ms and 7.23–7.59 ms in the old version. At 8% wind the new GPU median was 4.73 ms and p95 was 8.26 ms. These runs used a single 1440 by 900 view and measured 120 GPU frames each.

## Browser replay

The packaged game was built from museum revision `a524da25` and retains the maritime pine and Norway spruce packs from `e2c974345130bb6a`. In hardware Edge WebGPU, the same fixed 1440 by 900 overview across 23 samples from 3.08 to 14.08 seconds showed a luminance change greater than 8/255 on 21.29% of the green pine pixels in the estimated 300–450 m band (old Storm 3.29%) and 19.85% of the corresponding spruce pixels (old Storm 1.42%). Pixel changes include lighting and occlusion as branches move.

With wind off the new and old rendered frames matched at both species. The four previously reported dark crowns at about 100 m also matched the old fixed frames pixel for pixel. To isolate the spring from the stronger whole-crown sway, we replayed the identical gust with both branch solvers in a pine close-up. The average difference across 27,959 sampled edge pixels was 5.64/255. The spruce crown-tip view shows the shared whole-crown motion particularly clearly. [Pine branches in a storm](https://splitscreen.club/playtests/tree-museum/7cf61f408e1ab816/?species=pine&view=needles&strength=100&reach=1500) · [Spruce tips in a breeze](https://splitscreen.club/playtests/tree-museum/7cf61f408e1ab816/?species=spruce&view=crowntip&strength=25&reach=90).

In the [spruce branch close-up at 25%](https://splitscreen.club/playtests/tree-museum/7cf61f408e1ab816/?species=spruce&view=closeup&strength=25&reach=90), the branch tip bends against the adjacent woody support at 4.08 s.

The stable forest route resolved to `7cf61f408e1ab816` for both species in hardware Edge WebGPU. Each flight loaded its own 100,000-tree pack, switched between Breeze and Storm, set the reach to 1,800 m and moved more than eight metres under pointer lock. The console reported no page errors. All 11 remote files matched the local release hashes. The local five-second spruce close-up is `C:/dev/perf/2026-11-25-forest-storm/veerbeweging-fijnspar.mp4` (25% wind, real time, 24 fps).

All twelve forest flight tests passed. The shared renderer passed 29 focused wind tests and the headless GPU joint-pose parity test. The six affine tree and shadow comparisons with the CPU reference passed. No Mac frame times were measured.

The diagnostic shadow-buffer restore assertion also failed on the old render library. Before this change 1,394 of 66,049 pixels differed on that debug pass, compared with 15 on the new one. Three split-view debug panels differed by at most 1/255 from the single view. Wind-off browser checks passed.
