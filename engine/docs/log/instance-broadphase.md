# Compact colour-view instance selection

`KERN_INSTANCE_BROADPHASE=1` enables an experimental, default-off GPU broadphase.
It does not change a mesh, LOD threshold, alpha test, wind deformation or shadow
selection. The measured motivation was the four-player scenario
`r26-regression/scenario-four-001`: colour culling averaged 4.24 / 4.66 ms and
visibility rasterization 14.57 / 16.19 ms in its two steady sections. Those
numbers describe the old path; no gain is claimed before an actual matched run.

The existing path launches one 64-thread workgroup for every scene instance in
every view, including empty reserved pool slots. The new first pass assigns one
thread per instance. It uses the same empty-slot, voxel-replacement, grass
opacity, frustum, wind-bound and camera-fog rules. Surviving source owner IDs
are compacted before an indirect dispatch of the unchanged cluster-selection
function. Only colour views use it initially; sun and headlight passes retain
the established scan.

The compact list temporarily occupies the existing bucket `draw_map` buffer;
the later cluster bucket scatter overwrites it after consumption. Counter 11 is
temporarily the owner count and is reset before bucket scatter. A separate
12-byte indirect buffer avoids writable-storage/indirect aliasing. No new
large storage allocation or binding is needed. If the scratch list cannot hold
every surviving owner, the indirect dispatch falls back to the full old scan
and recounts grass rejects, rather than dropping geometry. Existing cluster
capacity assertions remain in place.

GPU profiling reports `instance broadphase` separately. Compare the sum of
that pass and `cull` with old `cull`; do not present a lower second-pass time as
the full gain. The normal path performs no count readback or CPU wait.

`render/examples/instance_broadphase.rs --out FRESH_DIR` is an exclusive-GPU
acceptance fixture. It compares actual sorted `(owner, cluster)` pairs,
triangle/submitted-vertex/grass-reject/shadow counters and rendered pixels
between OFF and ON. Cases cover mixed multi-LOD/alpha geometry, wind, fog,
four distinct views, live instance updates, 70,003 surviving owners across an
indirect-dispatch row boundary, and an empty scene. The explicitly blocking
`Renderer::debug_visible_clusters` API exists for this diagnostic and is not
called during gameplay. It reads saved references to the actual active colour
targets from the last submission, with frame ID, render sequence, camera,
viewport, input size and output size. An optional expected sequence rejects
stale requests; unused pooled views are inaccessible after a 4-to-1 change.
The initial diagnostic reader mistakenly indexed the shared shadow slot; this
was isolated to that new reader, not the frame profiler, and was corrected.
The fixture also checks that readback-pair totals equal the colour-pass counter.
The twelve focused frame/layout tests and the fixture's Cargo check passed
before the identity hardening, including specialization of all cull entrypoints
and the 242/256-query worst-case timestamp budget. The strengthened API still
needs its final compile and GPU-fixture run.

The corrected GPU fixture v2 passed all five original selection cases with
exact pixels, including all 70,003 intended owners. The subsequent identity and
stale-view assertions still need their updated fixture run. The first matched
route measurement does **not** demonstrate a culling speedup: over 179 eligible
frames, OFF cull averaged 4.446 ms, while ON cull plus broadphase averaged
4.259 + 0.347 = 4.606 ms. Raster was approximately unchanged. The default stays
off; repeated whole-frame measurements must not attribute unrelated pass
variation to this candidate.
