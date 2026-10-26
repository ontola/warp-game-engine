# Persistent tyre material marks

The visual-only `sideways::track_marks::Emitter` receives the same real wheel
contacts as debris. It emits continuous world-space segments, with stronger
soil compaction under slip and rubber only while skidding on asphalt. It does
not alter handling, contact forces or the simulation RNG. Default tyre width
is 24 cm and can be set per wheel. Airborne/water/missing contacts, resets,
teleports and tick discontinuities break the trail. Paused ticks do not emit.

`Renderer::update_tracks` commits at most 8192 stamps per call. The CLI retains
uncommitted segments across skipped render frames. All views sample the same
atlas; no view-specific aging or camera-centred clearing occurs. New stages
clear identities through `clear_tracks` or renderer replacement.

The atlas stores 512 sparse world tiles, each 128² texels at 8 cm (10.24 m per
tile). GPU storage is 32 MiB RGBA8 plus a 1 MiB identity lookup. CPU tile payload
grows only for occupied tiles, up to another 32 MiB. New/reused tiles upload a
cleared page; subsequent writes upload dirty rectangles. Least-recently-stamped
tiles are evicted at the explicit budget, and hash tombstones preserve collided
identities without exposing stale page contents. Stats report resident/evicted
tiles, stamps, rejected inputs, texel writes and last upload bytes.

Compaction/rubber modify terrain/road albedo and roughness **before lighting**.
Existing normals, textures, shadows and direct/indirect illumination remain
active. There is no black overlay, floating ribbon, emission or geometry
deformation. Coverage edges are filtered; subpixel marks fade rather than
aliasing into sharp tread noise. Ground-cover plants are not flattened.

Each texel retains its contact height relative to a tile base. Four filtered
taps independently reject surfaces more than 16 cm from that height; a bridge
above a marked road therefore receives no mark. This is a single height layer
per texel: a later crossing at a different height replaces that texel's earlier
mark. This limitation and tile eviction are bounded-memory trade-offs, not
claims of unlimited stage history or persistent terrain deformation.

CPU tests cover contact discontinuities, four-player emission bounds, asphalt
slip, negative coordinates, tile boundaries, height rejection, sloped contact
planes, duplicate-stamp stability, eviction and collided lookup identities.
`cargo run -p render --example tracks_gpu -- --out FRESH_DIRECTORY` is an
exclusive-GPU fixture for material lighting, bridge rejection, four shared
views, dark-scene behaviour and stage clearing. It records warm shade timings
with/without marks. Root ran it successfully on 24 October 2026; report:
`C:/dev/perf/warp-sideways-20261024/r22/track-gpu/report.json`.
Actual matched gameplay on/off images show subtle rolling paths, with zero
queue loss; native 4K/four-player also renders. See `wheel-effects.md` for
the actual image references, capacities and bounded timing observations.
This document does not assert a 120 FPS result.
