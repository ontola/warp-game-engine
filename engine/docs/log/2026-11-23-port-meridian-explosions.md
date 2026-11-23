# Growing Guns explosions in Port Meridian

Port Meridian `521ed81d` adds startup preparation to game `a200153d`, which uses the Growing Guns explosion renderer through the Luau API in `21655f7f`. The red hazard crates at Exchange Avenue and the docks use the same game helper as the browser story editor's `explosion` step. The pistol resolves the nearest pedestrian, crate or obstruction. Simultaneous hits consume a crate once. After 15 seconds the crate returns when its space is clear.

The fixed simulation clock ages a bounded pool of effects, shared by all active views. On a full renderer with a granted 3D texture dimension of at least 1,536, the renderer uploads the existing 48 × 48 × 1,536 RG8 smoke animation during city loading. That texture occupies 6.75 MiB on the GPU. The procedural Growing Guns billows cover smaller devices and events created before the first render. Both modes use the same fire, embers, heat distortion and light sources.

The original bang and rumble are two Luau synth patches totalling 4,143 source bytes. A radius-14 blast mixed at four metres peaked at −3.5 dBFS in the stereo Warp mixer. The previous public engine already contained the complete smoke animation. Port Meridian adds no audio samples or texture downloads for this effect.

## First-shot latency

On the RTX 5070 Ti the initial lazy-loading build took 1,790 ms to render and finish the first native blast. Later solo frames took about 11 ms wall time. In Edge at 960 × 540, a real Space press submitted the first effects frame at 16.9 ms, but the GPU finished it at 2,008.5 ms. The separate cold city startup took 84.94 s in this test. The crate returned after 900 ticks. This identified effect preparation as work to move into city loading.

The game opts in with `fx.warm_explosions()`. Startup draws an explosion into a private 16 × 16 target and exercises both direct rendering and adaptive smoke compositing on supported devices. The renderer then restores its previous effects. This also warms drivers that defer compilation until draw submission.

In matched 1,440 × 810 native runs, each in a fresh process with an empty application pipeline cache, the complete first shot through GPU completion went from 1,308.6 ms to 11.90 ms. The CPU draw portion fell from 1,286.8 ms to 0.97 ms. The next frame took 11.86 ms. The installed Windows build matched all 54 frozen runtime files and ran four occupied seats for 120 ticks.

An isolated Vulkan test granted the minimal LDR renderer a 512-texel 3D limit. Its procedural explosion changed 7,061 pixels in a 256 × 160 frame and used zero baked volumes. GPU validation passed, and the final image matched the baseline after expiry. At a 256-texel device limit the base renderer failed to allocate its existing 288-texel canopy texture before reaching the explosion path. The 512 test ran on the RTX desktop with a restricted limit.

On the installed release, median native GPU time went from 2.40 ms on the intact street to 2.57 ms on the flash frame in one view. In two views it went from 3.70 ms to 4.07 ms. The second player could see the light flash from inside the office. These are fixed-camera measurements at 1,440 × 810 on the RTX 5070 Ti.

## Browser playtest

The frozen Edge build at 960 × 540 completed its first effects frame 29.9 ms after the Space keydown with the smoke texture already uploaded. The largest animation-frame gap during that shot was 8.5 ms. The earlier lazy build took 2,008.5 ms to finish the first effects frame on the same machine.

Public shelf `521ed81d-20261123114412` matched all 87 release hashes over HTTPS. The three new game files total 5,953 source bytes. Including HTTP headers, the cold browser transfer grew from 19,679,632 to 19,692,655 bytes, a difference of 13,023 bytes. An Edge profile primed against the previous public release downloaded 17,151,770 bytes to upgrade, including 17,054,496 bytes for the changed runtime. Reopening the new shelf transferred 190 bytes and zero engine bytes.

The public build completed its first explosion GPU frame 30.5 ms after the shot and submitted it at 13.4 ms. The crate disappeared on the next tick and returned after 900 ticks. The captured frame showed the pavement and wall lighting up, and the run produced no WebGPU or Luau errors.
