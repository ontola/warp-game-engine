# Growing Guns CPU spike comparison — 2026-10-28

The measured candidate substantially reduced explosion reset-frame CPU wall time. It did **not** remove every hitch: bullet p99 timings increased, and explosion nonreset p99 at 4K increased slightly. GPU timings were broadly unchanged. Workload counters matched exactly; screenshots were **not** pixel-exact.

## Measured binaries and method

Baseline was built from `d221665`; candidate was built from `35497cb`, whose changes were subsequently rebased as `09639b6` (`perf: retain static colliders and reuse explosion render work`). The candidate retains static arena colliders, reuses arena/lava work and FX render resources, and adds optional timing diagnostics. The newer grass snapshot merge and subsequent main branch were **not measured** by this comparison. The candidate filename contains `d221665`; use the recorded hash and build provenance, not that filename, to identify it.

| Binary | SHA-256, verified against manifest and file |
| --- | --- |
| Baseline | `752a27615a5618dbbadd3c223681d24306995b51fe3a9833b367d909e91d603d` |
| Candidate | `4b5ea910d26461e41409d7688a4b006590beab7f27a35716c035660ea3f75df1` |

Evidence: `C:/dev/warp-bench/cpu-spikes-20261028-175917/ab/manifest.json`, collected 17:33:34–17:39:15 UTC. Hardware recorded: NVIDIA GeForce RTX 5070 Ti, driver 617.14. Three repeats alternated A/B, B/A, A/B for each case: bullets and explosions at 1920×1080/one view and 3840×2160/four views. Each run used seed 721077, full lighting, original character assets, 180 warmup ticks and 900 measured ticks at a fixed 60 Hz simulation rate. Execution was unpaced with a GPU completion wait per frame; audio and HUD were excluded. Screenshots were captured after measurement, in repeat one only. The earlier Sideways-contaminated pilot is excluded from every result.

All 24 runs exited successfully; all 12 adjacent alternating pairs passed validation. GPU timestamps covered **21,600/21,600 measured frames**, with no dropped samples or errors; per-pass sums matched the instrumented totals. Primary runs had thread CPU profiling disabled. All 18 workload/occupancy columns matched for every paired frame, including particles, debris, lights and alive chunks.

| Per-run work, identical in all repeats and both resolutions | Bullets | Explosions |
| --- | ---: | ---: |
| Requested / processed events | 3,150 / 3,150 shots | 600 / 600 explosions |
| Impacts | 3,150 | 0 |
| Chunks removed / worn, summed across cycles | 0 / 0 | 7,860 / 19,615 |
| Debris spawned | 9,450 | 590,800 |
| Reset / nonreset frames | 0 / 900 | 15 / 885 |
| Kills / exhausted-target fallbacks | 0 / 0 | 0 / 0 |

All four players remained alive in every measured frame. Reset-frame costs remain included in the all-frame results below; strata use entire frames without subtracting reset work.

## CPU wall-time results

Each cell is **median / p95 / p99 in milliseconds**, calculated per run and then taking the median of the three run statistics. Frames are never pooled across repeats. Percentiles use the benchmark's nearest-index convention. Reset p99 represents the maximum of only 15 reset frames per run, so its tail estimate is coarse.

| Case / stratum | Baseline CPU total | Candidate CPU total |
| --- | ---: | ---: |
| Bullets 1080p, all = nonreset | 1.748 / 3.946 / 7.130 | 1.731 / 3.962 / 7.731 |
| Bullets 4K, all = nonreset | 3.137 / 7.691 / 10.748 | 3.100 / 7.447 / 11.881 |
| Explosions 1080p, all | 2.325 / 6.622 / 22.674 | 2.076 / 5.640 / 13.462 |
| Explosions 1080p, reset | 25.699 / 36.552 / 45.279 | 13.666 / 23.567 / 28.019 |
| Explosions 1080p, nonreset | 2.304 / 5.934 / 8.100 | 2.049 / 4.956 / 7.693 |
| Explosions 4K, all | 4.064 / 10.379 / 22.444 | 3.728 / 9.965 / 15.464 |
| Explosions 4K, reset | 23.272 / 38.436 / 42.151 | 15.304 / 22.946 / 24.328 |
| Explosions 4K, nonreset | 4.016 / 9.593 / 12.671 | 3.698 / 9.210 / 12.919 |

Across paired repeats, the median percentage reduction in explosion reset-frame median CPU total was **46.8% at 1080p and 35.0% at 4K**; all-frame p99 reductions were 42.5% and 32.4%. These are medians of three within-pair percentage changes, so they need not equal the percentage calculated from the displayed baseline/candidate medians.

The largest measured stage improvement was reset-frame scene update: **12.184 → 3.355 ms** median at 1080p and **10.736 → 3.481 ms** at 4K. The reset operation itself fell **10.671 → 8.470 ms** and **10.297 → 8.109 ms**. Nonreset scene medians also fell **1.149 → 0.990 ms** and **1.162 → 1.038 ms**. Candidate-only absorb/pose instrumentation puts reset scene absorb medians at 3.323/3.455 ms and pose at 0.026/0.028 ms for 1080p/4K; baseline has no split columns, so this split is not an A/B comparison.

Bullet p99 was higher in two of three pairs at each resolution. The median paired p99 change was +16.7% at 1080p and +13.1% at 4K. Candidate worst individual CPU frames still reached 34.551 ms for explosions at 1080p and 39.262 ms for bullets at 4K. The data supports reduced synthetic arena rebuild/reset costs, not a general claim that gameplay hitches are solved.

## GPU and screenshot checks

| All-frame GPU union, median / p95 / p99 ms | Baseline | Candidate |
| --- | ---: | ---: |
| Bullets 1080p | 1.634 / 1.928 / 2.038 | 1.634 / 1.927 / 2.033 |
| Explosions 1080p | 1.830 / 2.194 / 2.350 | 1.833 / 2.189 / 2.347 |
| Bullets 4K | 5.503 / 5.872 / 5.994 | 5.453 / 5.876 / 6.054 |
| Explosions 4K | 6.387 / 6.844 / 7.012 | 6.372 / 6.860 / 7.026 |

GPU pass and CPU stage medians/p95/p99, including reset strata, are in the [compact JSON](cpu-spikes-20261028.json). These instrumented GPU scopes and CPU submission measurements are not presented frame rate.

| First-repeat screenshot | Changed pixels | Maximum channel delta | Mean absolute RGBA channel delta |
| --- | ---: | ---: | ---: |
| Bullets 1080p | 99,802 / 2,073,600 (4.813%) | 15 | 0.0204 |
| Explosions 1080p | 279,303 / 2,073,600 (13.469%) | 20 | 0.0568 |
| Bullets 4K | 508,673 / 8,294,400 (6.133%) | 17 | 0.0251 |
| Explosions 4K | 1,377,659 / 8,294,400 (16.610%) | 30 | 0.0696 |

Images were read with Pillow and compared as decoded RGBA8 pixels, with no tolerance and no generated images. Dimensions matched. Both file hashes and decoded pixels differed for every pair. The average differences are small on the 0–255 channel scale.

Separate follow-up runs used the same pinned binaries and unchanged assets to test repeatability. Each had matching workload/occupancy counters and metadata against its first-repeat run. Their timing samples are excluded from the primary A/B results.

| Image repeat control | Changed pixels | Mean absolute RGBA channel delta |
| --- | ---: | ---: |
| Bullets 1080p, baseline vs itself | 4.633% | 0.0192 |
| Bullets 1080p, candidate vs itself | 5.685% | 0.0234 |
| Bullets 1080p, follow-up baseline vs candidate | 5.418% | 0.0220 |
| Explosions 4K, baseline vs itself | 16.603% | 0.0696 |
| Explosions 4K, candidate vs itself | 16.629% | 0.0696 |
| Explosions 4K, follow-up baseline vs candidate | 16.623% | 0.0697 |

These controls demonstrate image variation within each unchanged binary, comparable to the cross-binary differences in the two checked cases. They do not isolate its cause or establish exact pixel parity or visual equivalence.

## Separate thread CPU diagnostic

One candidate-only explosions/1080p run enabled `--cpu-thread`, using the same pinned candidate and unchanged assets. It passed workload/GPU validation and is excluded from primary A/B timings. All 900 frames had counter values. Windows thread CPU time advanced in **15.625 ms quanta**, so individual stage/frame zeros and percentiles cannot establish how much work occurred in those intervals.

| Whole-frame stratum | Frames | Summed CPU wall time | Summed calling-thread CPU time |
| --- | ---: | ---: | ---: |
| All | 900 | 2,360.323 ms | 1,171.875 ms |
| Reset | 15 | 200.075 ms | 140.625 ms |
| Nonreset | 885 | 2,160.247 ms | 1,031.250 ms |

Across all frames, recorded thread stage totals were 640.625 ms in render submission and 421.875 ms in scene absorb; the remaining stages together recorded 109.375 ms. Stage attribution remains coarse: simulation and pose recorded zero even over the aggregate, which is not evidence that they were free. This measurement covers only the calling thread's user/kernel CPU time. Quantization and unmeasured worker threads prevent assigning the elapsed/thread-time gap or attributing individual wall-time spikes to scheduling. No baseline thread diagnostic was collected, so it cannot establish an active-CPU speedup.

## Limits and reproduction

CPU columns measure elapsed wall time, including blocking and descheduling. Host CPU snapshots ranged from 30% to 74%; they cannot attribute individual spikes to active game CPU work or scheduling. The original runner checked known game processes before runs, but did not record before/after guard results; the newer reusable runner does. An entirely idle host cannot be established retrospectively.

This is a three-repeat synthetic stress comparison with pinned players, periodic/depletion arena resets and fixed camera views. It is not a live-play timing capture, a statistical significance claim, or a benchmark of the later grass/main build.

Recompute without modifying input evidence:

```powershell
python games/growing-guns/tools/analyze-cpu-spikes.py --manifest C:/dev/warp-bench/cpu-spikes-20261028-175917/ab/manifest.json --diagnostic C:/dev/warp-bench/cpu-spikes-20261028-175917/thread-diagnostic --require-complete
```

The complete structured analysis is saved beside the evidence at `C:/dev/warp-bench/cpu-spikes-20261028-175917/ab-analysis.json`. The committed compact JSON retains the verified binary hashes, exact workload totals, paired stage/pass statistics, per-run CPU totals, screenshot controls and separate thread aggregates. Follow-up image controls were calculated with the analyzer's read-only `image_difference` helper; the CLI command above recomputes primary timings and the thread diagnostic. For a new comparison, use [compare-cpu-spikes.ps1](compare-cpu-spikes.ps1) with explicit `-Baseline`, `-Candidate` and a new `-Out` after arranging exclusive GPU use.
