# Stadium broadcast cameras

Mancave, 11 November 2026, RTX 5070 Ti, Vulkan, release build of 67e0fa4b. Growing Guns ran the headless bullets stress fixture with full graphics, seed 1 and 1920×1080 combined native output. Four pinned fighters continuously fired rifles. Each run yielded GPU timestamps for all 900 measured frames after 180 warm-up ticks. The four-view run was repeated.

| Extra camera | Resolution and rate | Median GPU ms per refresh, one view | Median GPU ms per refresh, four views |
| --- | --- | ---: | ---: |
| Crowd section | 192×108 at 12 Hz | 0.038 | 0.038–0.039 |
| Crowd close-up | 384×216 at 12 Hz | 0.211 | 0.212–0.515 |
| Battle from a seat | 320×180 at 10 Hz | 0.121 | 0.423–0.486 |

Each camera renders once per scheduled refresh for the four stadium TVs together. Dividing the extra camera timestamps by all measured frames gave 0.019 ms per frame for one view and 0.081–0.090 ms for split screen. Four-view battle refreshes reached 4.5–4.8 ms at p95 and 5.72 ms maximum. The single-view wide shot had a 5.44 ms outlier. These are GPU pass durations. The pre-existing TV blit, CPU work and presentation run outside those intervals.

The stadium-tv match script checked the return to the primary feed after the battle overview, wide audience shot and close-up. A separate capture of the same spectators at the same tick and camera confirmed that resting hands hang closer to their thighs. The close and distant poses attenuate the inward shoulder adjustment as the arm rises.
