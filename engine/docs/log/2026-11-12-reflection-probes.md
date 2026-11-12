# Live reflection probes, 12 November

Machine: mancave, RTX 5070 Ti. Scene: `interior_rooms`, one box per room.
Guide: [dynamic-gi.md](../dynamic-gi.md#reflection-probes).

## Built

One cube face (128x128) a frame, round robin, drawn after the players'
views with view 0's lights and shadow maps, its own cull/raster targets,
uniforms and shade scratch; copied into a raw cube array, box-filtered
mips, GGX prefilter (64 samples, filtered importance sampling) into a
second array that shade reads (binding 27). Box projection; outside every
box a blend of the boxes within 0.5 m that the reflected ray heads into.

## Faults found on the way

- Walls lie exactly on their room's box: the first version counted them
  outside, so no wall reflected a probe. Within 2 cm counts as inside.
- The first blend outside the boxes weighted by distance alone: a shut
  door, in the wall between rooms 3 and 4, reflected lit room 3 on its
  room-4 face (a bright door at EV -3 in `leak_door`). A box now counts
  only where the reflected ray heads into it.
- The diagnostic itself misled once: its magenta "no box" marker at 6000
  nits set the auto exposure, so the probes looked black.

## Measurements

- GPU timestamps, 4K, r2_box view: probe cull 0.002 ms, raster 0.006,
  shade 0.002, mips and prefilter 0.015 a frame. The shade of 16k pixels is
  one wave on this GPU, so its timestamp is its latency, not a share of
  the main shade's throughput.
- Main shade with the probes read: 11.0 ms against 10.7 with
  `KERN_REFLECTION_PROBES=0` (same view, 300 frames). Whole-frame times in
  `--bench` (19 to 25 ms over 600 frames, probes on or off) were too noisy
  to show the capture.
- `--pan 240` at look_down: mean frame-to-frame change 0.013 (without the
  probes' build 0.012).
- Updates: lamps in room 3 switched off, the next spot's reflection of
  room 2 and 3 changes within the spot; the swinging pendant shows at two
  phases in a mirror-like floor (faces captured frames apart).
