# Foliage card coverage across tree species

On 24 November 2026 the baked maritime pines in Warp Theft Auto lost groups of needles when seen edge-on. We compared the same game cameras with photographs of an open-grown maritime pine in southwest France and a forest stand in Portugal. The exposed lower trunk and openings between branches belong in the reference. The needle clusters at the branch tips should still read as volumes. [French pine, fmunoz](https://www.inaturalist.org/photos/634624424) · [Portuguese stand, Duarte Frade](https://www.inaturalist.org/photos/536650123) · [European Atlas of Forest Tree Species](https://forest.jrc.ec.europa.eu/media/atlas/Pinus_pinaster.pdf)

## The shared bake

The default card cooker at `e0a7c26d` estimates the principal axes of each unique part from its area-weighted triangle surface. A flat or slightly cupped leaf uses one silhouette in its fitted plane. A volumetric foliage cluster gets a second, perpendicular silhouette when its RMS depth exceeds one third of its smaller in-plane RMS extent. Both silhouettes come from the same source geometry. Small woody parts use one projection. The source-code fingerprint in the card cache updates old bakes. [Plant card cooker](../../crates/render/src/plant_mesh_lod.rs)

## Species checks

Seed 18, 12 azimuths at each of two elevations. The CPU probe rasterized just the foliage using the full source and the card mesh with its 2048-square alpha atlas. Coverage is occupied pixels divided by the number of occupied source pixels, summed across all 24 views. Intersection over union measures how well those pixels line up. An exact match is 100% on both measures.

| Crown | Old coverage | New coverage | Old overlap | New overlap | Old / new foliage quads |
|---|---:|---:|---:|---:|---:|
| Maritime pine | 74.6% | 99.7% | 52.3% | 61.8% | 188 / 376 |
| Norway spruce | 88.5% | 106.7% | 69.8% | 75.8% | 540 / 1,080 |
| Birch | 99.0% | 98.9% | 74.3% | 74.3% | 2,880 / 2,880 |
| Broadleaf recipe | 98.5% | 98.5% | 71.8% | 71.9% | 3,136 / 3,136 |

Seed 1948 again gave about 100% coverage for pine, 106% for spruce and 99% for the two broadleaf crowns. A single tilted birch leaflet improved from 53.3% to 97.1% overlap with one quad. A single maritime pine spray had slightly worse mask overlap, 34.2% to 31.0%, despite the whole crown improving. The two-view approximation can leave a spruce somewhat too dense.

## In the game

Mancave, RTX 5070 Ti, native 1280 × 720, two side-by-side views and the 124 pine replacements along the West Cape rally trail. The same recipe, seed, transforms, game tick and cameras were used for both bakes. Against a registered sky background from a run with the trees hidden, the original procedure occupied 56,982 pixels. The old bake occupied 38,869 and the adaptive bake 48,928. The new bake retained 85.9% of the source coverage, up from 68.2%. In a larger solo view the corresponding figures were 76.7% and 61.6%. The corrected split-screen crop masks still left 80.5% of the original interior gap pixels open.

The shared, 1024-atlas WTA specimen uses 5,192 source triangles including wood, up from 4,816. Its complete asset takes 139,787 gzip bytes. It is shared across placements. The test establishes canopy coverage and geometry cost. A moving-camera GPU timing on an integrated laptop remains open. Retain the existing shipping trees until that frame budget has been verified.

Source data, captures, crops and photo credits are under `C:/dev/perf/2026-11-24-wta-tree-eval` on Mancave. Both reference photos are CC BY 4.0. The local contact sheet scales the photographs for comparison. [License](https://creativecommons.org/licenses/by/4.0/)
