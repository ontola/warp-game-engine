# Gust recovery and upper shoots in the 100,000-tree forest

Playtesters reported a pause at the top of a storm gust, little independent motion in the leader, and too much damping on fine twigs relative to large boughs. The baseline was browser release `7cf61f408e1ab816`. The camera and rig selection are from museum revision `ce68f36b` and the final source cook is `adf5de36`. The shared engine changes are [gust recovery](https://github.com/ontola/warp-game-engine/commit/fe8efe90), [growth joint allocation](https://github.com/ontola/warp-game-engine/commit/e08de4af) and [stable shoot selection](https://github.com/ontola/warp-game-engine/commit/80cb8bbb).

## What happened after a gust

At full storm, the old front function could hold its maximum. The new front briefly applies pressure and the crown swings back, with an along-wind and a smaller crosswind component. The wind strength, direction and timing change across the sequence. The local springs receive the pressure pulse while inheriting the motion of their parents. A branch can continue ringing after the air load subsides.

For ten minutes at one fixed world origin, 60 samples per second, the old storm remained nearly still at high deflection for up to 88 frames (1.47 s). The new storm spent zero frames below the same speed threshold while strongly deflected. High deflection means downwind displacement above 0.60 of full scale. The speed threshold was 0.08 of full scale per second. The crown passed 0.10 of full scale back against the wind on 72 occasions. These are measurements of the field, before camera projection.

With an identical short release applied to both joints, fitting the peaks of the local angle gave an amplitude half-life of 0.228 s for the thick bough and 0.552 s for the thin twig. Two bough peaks and nine twig peaks remained above 5% of the first measured peak. Across 32,768 samples of the canopy and pressure fields covering 1.8 km by 1.8 km and a full 32-minute cycle, CPU and GPU vector results differed by at most 0.000297 in full-scale units.

## Growth and camera selection

The adult spruce previously attached all 139 needle faces measured in the top 15% of its exported crown to the rigid root. All eight faces in its top 5% had that assignment. The corresponding top 5% of the maritime pine had 107 of 107 faces assigned to the root. Root attachment supported the whole-tree lean but precluded local movement of those shoots.

The growth cooker reserves joints on the narrowing central leader and ranks forks by the highest live foliage they carry. The vertical bands narrow towards the tip. Lateral clusters share the motion of their parent while keeping their own pivots. The adult source growth samples have six distinct joints supporting living tufts in the top 5% of Norway spruce, twelve in maritime pine and five in Jeffrey pine. The near-tree budget remains 64 joints including the planted root.

Measured on the final exported adult assets, the number of independent joints carrying needle triangles in the top 5% was six for Norway spruce, nine for maritime pine and five for Jeffrey pine. All three exports assigned every triangle in this zone to a moving joint. Geometry simplification can merge shoots from the source growth sample.

At mature planting 3081 and 25% wind, a 32-second trace at 30 Hz measured the highest needle relative to its immediate parent. The spruce tip reached a p95 excursion of 30.6 cm and the maritime pine tip 28.6 cm. Both had zero independent motion at this point on the former rig. Smaller lateral shoots in the same top 5% also flexed about their own attachments. The maximum numerical drift at a branch pivot was below 0.14 mm.

The nearby forest still leases 32 rigs within 48 m. Selection tests four measured crown-height bands against the viewport, including its aspect ratio and vertical field of view. In the fixed backlit crown-tip view at 1440 by 900, two previously unassigned mature trees with visible tips (planting IDs 54743 and 54694) received 97% and 81% local bend weight on the baseline spruce pack. The new leader camera isolates mature planting 3081 at the forest edge. It receives full bend weight. The camera frames the upper trunk and shoots against the sky.

## Playtest

The fixed leader camera and the forest overview are available in the browser. Each opens in a storm with a reach of 1,500 m. [Watch the spruce leader](https://splitscreen.club/playtests/tree-museum/forest/?species=spruce&view=leader&strength=100&reach=1500) · [Fly over the pines](https://splitscreen.club/playtests/tree-museum/forest/?species=pine&view=canopy60&strength=100&reach=1500).


### Native GPU timings

One fixed 1440 by 900 view, native temporal antialiasing, one raster sample, RTX 5070 Ti on driver 621.24. Each run used the final Norway spruce pack with 32 near rigs, 32 warmup frames and 120 GPU-timestamped frames. CPU preparation covers the forest. GPU timing covers the rendered frame.

| Pose | Wind strength | CPU median / p95 | GPU median / p95 |
| --- | ---: | ---: | ---: |
| 60 m above the canopy | 100% | 0.543 / 0.673 ms | 4.647 / 7.489 ms |
| Crown tip at the forest edge | 45% | 0.582 / 0.740 ms | 2.595 / 5.126 ms |

At 60 m the p95 was below the 8 ms target in this run. The slowest frame took 9.733 ms. At the leader the slowest was 5.568 ms. Both runs finished without dropped frames or errors. Mac GPU times remain unmeasured.

### Build checks

The forest flight passed 13 tests. The shared wind code passed 32 unit tests and the separate GPU pose parity fixture. The fallback and direct shoot representations use the same grown branch hierarchy. The first browser candidate used a maritime pine pack with SHA-256 `e4e62ba3a89234c7758e4cb92ad28c0f4288e9710f452b2a19a865b3c377b2c6`, and a Norway spruce pack with `864b9b7327e71af783a425381ac99b35f1252b9b9b1c3c8f4682b3c04d832bcc`.

### Browser isolation and acceptance check

In the first candidate, the 35-degree fixed camera faced planting 3081 in hardware Edge WebGPU. For the local spring control, only the joint stiffness values in a copy of the same pack were set to one. The two rest frames matched pixel for pixel. At full storm, the 5th-to-95th percentile of the signed screen angle between the spruce leader and its supporting shoot spanned 0.91 degrees with locked joints and 1.69 degrees with the local springs enabled. The longest interval with less than 0.08 degrees of absolute tip rotation per recorded frame fell from 0.73 s to 0.17 s. The tracker accepted 85% and 80% of 599 consecutive frame pairs respectively. Frames were sampled at 30 Hz from a deterministic 120 Hz presentation clock. Screen-space angles include perspective. The parent-relative three-dimensional measurements above isolate the displacement at the joint.

The first candidate was published at a separate [immutable spruce playtest](https://splitscreen.club/playtests/tree-museum/8f5ec6f47594467e/?species=spruce&view=leader&strength=100&reach=1500). Its maritime pine lost needles on a projecting shoot visible on the right of the leader view. In the wind-off comparison, the same tip crop fell from 28.66% green coverage to 6.51%. This held up the switch of the stable forest route. Both species passed the separate 100 m dark-patch check.

### Keeping needle coverage independent of the wind rig

The full-resolution growth sample still contained the missing live shoot. Changing the wind joints had inadvertently changed the set and scale of needle tufts feeding the static export. A second reduction of that source mesh almost erased the tuft. The live rig's 12-bit limit on members per joint is needed when drawing individual members. A baked mesh only needs the tree's geometry budget. Its source now chooses tufts in tree space with a stable seed, giving living axis ends priority when thinning is necessary. Joints are assigned afterwards. The same rule is used for the growth profiles.

One 80 by 71 pixel crop isolates the terminal shoot of mature maritime pine 3099. The old export covered 1,200 pixels with needles. The first candidate covered three. The corrected export covered 1,188 at the same projected distance, 9.13 m. The wider 112 by 71 crop measured 1,994, 494 and 1,970 pixels respectively. At four azimuths the upper 15% of the corrected mature pine retained at least the old projected coverage within rounding. The final export used the existing triangle budget and material atlas. Its pack hashes are `927af1a1e63385400c9beb7fa7eb985dcc7436d54fdc79ae8f95ba312b253313` for maritime pine and `170e7ace013223f29acc42b13610ea75b2d7c52a011ebf58a4c28c9a157a8cad` for Norway spruce.

In the fixed 960 by 600 browser view, the old shoot contained 28.66% green pixels within the measured region. The rejected candidate had 6.51% and the corrected release measured 28.46%. Across the full frame the corresponding values were 59.97%, 53.00% and 59.78%.

The final spruce leader moved through a 2.12-degree range relative to its supporting shoot in the central 90% of the optical samples. A control on the same final geometry with its local joints locked measured 1.04 degrees. The tracker accepted 73% and 89% of frame pairs respectively, and their rest frames matched exactly. All eleven museum growth and static-cook tests passed, including bit-identical spray placement with a one-joint or a 64-joint wind rig. The [fixed final build](https://splitscreen.club/playtests/tree-museum/652ba83d697d40cd/?species=spruce&view=leader&strength=100&reach=1500) uses the corrected assets.

All nine baked source GLBs were reproduced byte for byte from the final code. The mature maritime pine, Norway spruce and Jeffrey pine exports contain 16,126, 14,854 and 16,132 triangles respectively.

The stable forest route serves this release and preserves the selected species, view and wind controls in the redirect.
