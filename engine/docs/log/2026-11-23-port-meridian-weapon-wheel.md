# Port Meridian weapon wheel and Growing Guns loadouts

Game `c0682158` runs on the previously shipped `521ed81d` hosts. The desktop and browser games run the same Luau scripts. The six-slot wheel uses the mouse or right stick for direct selection and the arrows or D-pad to step between slots. It consumes look, aim and fire on the browsing seat. Movement, time, other seats and grenades already in flight continue to advance. A grenade follows a fixed-tick arc and checks the entire segment between successive positions for impact.

The stock Growing Guns rifle and its Shotgun, Uzi and Sniper cards are exported from the original game's scripts. Cadence, magazine, reload time, damage, spread and shot count are resolved once from those recipes. A small third-person model reads the dimensions and attachment thresholds from Growing Guns' gun design. The shot and grenade launch use the original synth patches. The F2 Custom loadout template drives the same weapon, sound and model code as selection in the wheel.

At 1,440 × 810 on the RTX 5070 Ti, a two-seat native replay opened the wheel, selected the grenade launcher, fired, observed the impact and switched through the other four guns in 151 ticks. The other seat kept its pistol. The F2 test also submitted invalid builds and verified the current loadout survived.

With four seats firing eight-pellet bursts every .06 s, a 60-round magazine and .15 s reload, the downtown test sampled 1,235 live entities. With four loaded interiors it reached 1,518 of the 1,800-entity limit. Across the two 460-tick runs each seat fired 252 shots. Four empty magazines finished reloading while holstered. In the four-room test, median simulation time went from 3.49 ms idle to 5.69 ms during this workload. The 95th percentile was 13.97 ms idle and 17.57 ms during firing.

All 44 city integration tests passed on the frozen game, including simultaneous fire, grenade trajectories, cover, weapon-specific magazines, the editor and the wheel. The Windows installation matched all 63 runtime files and its four-seat smoke test ran 120 ticks with zero game errors. The new browser package has nine additional game files totalling 24,648 source bytes. The host, shared lighting and compressed WebAssembly sidecar match the previous public release.

On frozen shelf `c0682158-20261123133916`, Edge accepted direct mouse selection, all six keyboard selections and a separate virtual Gamepad API device for the other seat. The F2 editor preserved the prior story on an invalid build. Applying a valid four-pellet, nine-round shotgun and firing once left its HUD at 8/9 while the other seat kept its rifle at 5/5.

All 98 files matched the published release over HTTPS. A cold Edge profile transferred 19,714,032 bytes including headers, 21,348 bytes more than the previous shelf. An existing player upgraded in 92,208 bytes with zero WebAssembly bytes, and returning to the updated version transferred 250 bytes.
