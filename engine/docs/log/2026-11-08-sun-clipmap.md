# One sun clipmap, 8 November 2026

Goal (Joep, 6 November, and gap 2 of the UE5 comparison): one sun shadow
system, VSM-style, instead of the shared map, per-view near levels, the
anchored far map and the paged foliage sun. This step covers streamed
GPU-forest worlds (`terrain_fly`), behind `KERN_SUN_CLIPMAP=1`.

Machine: cloud container, lavapipe (software Vulkan, 4 cores). Its pass
times are CPU emulation: a cull dispatch costs about 200 ms whatever it
culls, so the numbers below compare variants with each other only. A real
GPU measurement (Mancave) is still to do.

## What changed

- `shadow_damage.rs`: one caster history for up to 33 cached maps (slot 0
  the shared map, then up to eight levels for each of four views). One track
  pass marks old and new bounds of every changed caster in each live map's
  64 x 64 tile mask. Scrolling works on array layers. A map can be deferred:
  it keeps collecting damage and redraws later; it is forced to redraw in
  full if it moved while damage was pending.
- Cached raster consumers derive the tile size from the map (`view.size.x`
  in `raster.wgsl`, header row 11.y in the voxel splat passes) instead of an
  override constant fixed to 4096.
- `KERN_SUN_CLIPMAP=1`: seven levels (24 to 1536 m), all cached; the shared
  map and the anchored far map are not drawn; the outermost level is bound as
  the "shared" map in shading; outer levels snap to whole tiles and redraw on
  the stagger or when they move.

## Measurements (640x360, `terrain_fly` default style, lavapipe)

Damage per frame with a still camera (`KERN_SUN_DAMAGE_REPORT=1`, bench):
the shared map had 2200 to 3000 of 4096 tiles marked every frame. Changed
casters per frame: about 130k source roots in snapshot row 4 (no bounds, so
they mark nothing), 900 to 1600 source transforms, 1000 to 2300 voxel bounds
and 8k to 14k voxel identities. The GPU forest reassigns voxel slots every
frame; each reassignment marks the old and the new place.

Sun pass time per frame, 45 frames, 30 warm-up (`WARP_BENCH_WARMUP=30`):

| Variant | Sun passes (ms) | Forest selection (ms) |
| --- | --- | --- |
| Separate maps (default) | 1077 | 298 |
| Clipmap, every level cached but redrawn every frame | 5080 | 380 |
| Clipmap, outer levels deferred and staggered | 1051 | 375 |

So with deferral the clipmap costs about what the separate maps cost while
it covers 1.5 km with one system. Forest selection grows with the number of
shadow consumers (seven levels instead of shared, four levels and far).

## Images

- Cached against full redraw (each level redrawn every frame), flying at
  6 m: SSIM 0.97 to 0.99, within the run-to-run noise of the forest's
  streaming (two identical runs: 0.91 to 0.93).
- `--shot` (deterministic) at 4 m, 40 m and 250 m, sun 25 degrees, clipmap
  against separate maps: SSIM 0.95, 0.95, 0.95. Sun visibility (`debug=8`,
  `9`), direct (`3`) and indirect (`4`) light match while flying.
- Visible change: ground shadows 15 to 30 m out are a little softer. The old
  path also took the minimum with the shared map 3 m up the sun ray (added
  when the levels' depth box was shallow); levels now span the scene's sun
  depth, so the clipmap path reads only the covering level. Separate maps
  without that minimum score 0.97 against the clipmap.

## Open

- Real GPU timings (Mancave) before making it the default.
- Voxel slot churn: tracking per tree instead of per slot would stop
  reassignments from dirtying tiles.
- Sideways (stage scenes, `far_sun` stage bake) and the paged foliage sun
  still have their own maps; non-forest scenes have no caster history yet.
- Local light shadows are a separate thread (clustered shadowed local
  lights).
