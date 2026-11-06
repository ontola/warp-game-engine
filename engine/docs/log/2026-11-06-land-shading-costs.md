# Land shading costs, 6 November 2026

Branch `claude/project-thread-lbfdp3` (PR #10). Mac: Apple M5 Pro,
`terrain_fly --town`, 2296x1440, still camera, median of the last 60 of 120
frames, a throwaway run after every rebuild (the light-less shade pipelines
compile on a worker and a cold Metal compile outlasts a short wall run).
Views: wall (the "Wall, close" camera of PR #8) and hill
(`--at -1160,-22,356,0.6,-0.55`).

## Method

Local experiment builds (never committed) added `override STUB` and
`override FSTUB` bit masks to shade.wgsl, set with a temporary
`KERN_SHADE_OVERRIDES=NAME=value` hook in frame.rs. Each bit made one term
return a constant. Savings are against the same build with the mask 0.
Single savings do not add up: on the hill the `shade terrain` savings of the
lighting terms sum to 5.2 ms but all together save 4.1 ms (register
allocation).

## Lighting terms (saving in ms when stubbed)

| Term | Wall `shade terrain` | Wall `shade` | Hill `shade terrain` | Image |
|---|---|---|---|---|
| irradiance_sample | 0.09 | 0.24 | 0.37 | unchanged (no volume) |
| screen_space_ao | 0.07 | 0.20 | 0.15 | changes the wall |
| contact_visibility | 0 | 0.02 | 0.07 | unchanged |
| sky_visibility | 0.04 | 0.32 | 0.23 | changes the hill |
| environment | 0 | 0.58 | 0.07 | changes both |
| field_surface (whole) | 1.34 | 0 | 2.92 | |
| shadow_at_surface | 0.41 | 0.84 | 1.35 | |

Inside shadow_at_surface on the hill: distant_sun_visibility 0.42 ms;
shadow_filter_plane changes no pixel at either view.

## field_surface terms (hill `shade terrain`, ms)

Layer textures 0.71, far_forest 0.57, rock 0.49, field_ground 0.46, road
0.25, field_detail_slope 0.23, broad and mottle noise 0.21, meadow_canopy
0.19, ditch, water, snow under 0.1. field_ground (region colours), road,
ditch and snow change no pixel in this world.

## Changes and results

| Commit | Change | Result |
|---|---|---|
| 7d1b965 | `FEATURE_IRRADIANCE`: no irradiance volume, no lookup code | hill -0.35, wall -0.30 (both passes) |
| ed9c6e0 | `FEATURE_FIELD_REGIONS`, `FEATURE_FIELD_ROADS` from the style blocks | hill -0.42, wall -0.26 |
| 7e17eef | far_forest taps stop early; detail slope skips large pixels | hill -0.16, wall 0 |

Images: noise against the previous commit on the Mac; bit-identical against
`KERN_SHADE_FEATURES=all` and against the previous commit headless on
lavapipe (town, default, hill).

## Timing fixes found on the way

- covered_ms read ~413 million ms, then grew by one frame per frame. Pairs
  reserved for encoder-level timestamps were never written on Apple (no
  `TIMESTAMP_QUERY_INSIDE_ENCODERS`): the GPU forest selection timers and
  `voxel splats`, which read one stale first-frame value (0.7-1.3 ms of
  phantom time in the pass sum). Fixed in 6ee7439, ed9c6e0 and 1255068;
  covered_ms is now about 16.4 ms at the wall and 56 ms at the hill.
- Per-pass numbers from before 1255068 are not comparable with later ones at
  the hill: `shade terrain` read 6.32 at ed9c6e0 and 6.74 at 1255068, which
  only changed timing.

## Open

- Layer textures, far_forest and rock do real work at the hill.
- distant_sun_visibility (0.42 ms) past the 180 m map.
