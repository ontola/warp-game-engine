# Warp Human fall response and Port Meridian vehicle contact

Port Meridian uses Warp Human joint controls introduced in `a395dda5`, moving vehicle contact skins from `68fa2187`, the sedan silhouette from `cdb16059` and adaptive contact steps from `fb379eb3`. The baseline used game `c0682158` on host `521ed81d`. Both were run on the Mancave Windows desktop with an RTX 5070 Ti.

## Protective flexion

A fitted, 1.78 m Warp Human from pool seed 45 started in the same idle pose and received the same Luau ragdoll call on tick 10. The Windows hosts sampled both arms and legs at 60 simulation ticks per second. At 0.30 s, mean knee flexion increased from 19° to 50° and elbow flexion from 57° to 72°. Mean hand distance to the chest decreased from 46 cm to 38 cm. At 0.45 s those distances were 45 cm and 30 cm. Skin joint samples from the final installed candidate matched the measured candidate.

The old rigs had angular limits and weak passive motors. A temporary, torque-limited spring in each joint now provides the additional flexion during a fall. It ramps in for 0.19 s, holds for 0.28 s, and fades over 0.78 s. A free two-part rig with gravity disabled returned to its passive neutral angle and kept its centre within 3 mm of the starting point. A regression test checks both arms and knees on a real fitted rig facing two directions.

## Road contact

The previous playtest passed car velocity and an upward kick to the walker, then used a roof-height box for limb contact. Mirroring the dynamics chassis improved one run but still left a 0.94 m vertical face across the front. In the slower central impact the feet moved only 0.14 m farther forward than the torso. The game now describes the existing sedan with 16 cross-sections and shares one contact surface across its paint variants. Its lower nose, rising bonnet, windscreen and roof move with the native car's pitch and roll. Car and pedestrian start with their own velocities. Nearby live ragdolls receive impulses from contact with the moving surface. Coupling is one-way. The road solver advances the vehicle independently.

In the initial two-seat native city fixture at `cdb16059` the driver reached the second player at 10.65 m/s. The feet moved 0.51 m farther forward than the torso. At ticks 5–7 the pelvis advanced over the bonnet while the torso tilted back. The vertical separation between chest and pelvis went from 17 cm to 2 cm during those samples. By tick 12 the torso was moving up the windscreen. Other fixtures cover reversing into a walking story character, passing a pedestrian alongside the car, creeping contact, braking, standing back up and prioritising nearby vehicles with 68 cars registered. A rig can stay on a moving vehicle beyond four seconds and continue falling when that support moves away.

At 57.5 km/h the sedan travelled about 27 cm per simulation tick, enough for a 7.5 cm shin collider to pass through the sloping contact skin. Video and joint samples showed the first solution lifting the character's hips to 3.38 m in the browser. Smaller steps advance the moving surface and the limbs together, up to four times per tick. With the same input and resident, the hip peak fell to 1.85 m and the head peak to 1.89 m. The native city regression covers 37.4 and 57.5 km/h through braking and requires the torso to tip onto the bonnet.

## Runtime and packaging

The fall response and vehicle contact reuse the existing human and sedan assets.

On the initial `cdb16059` host, three matched 75-tick intervals in the seeded four-seat downtown scene gave 225 simulation samples per workload. Median tick time was 3.25 ms with the seats standing and 3.43 ms with all four falling. The 95th percentile was 15.26 ms and 15.25 ms respectively. These measurements cover the CPU simulation on this machine.

On the adaptive solver, six matched runs per condition measured 360 CPU ticks in each window before and after contact. With one pedestrian the median went from 2.30 to 2.77 ms. With four simultaneous vehicle impacts it went from 2.57 to 4.94 ms. The matched drive with no contact went from 2.53 to 2.58 ms. During impact, the 95th percentiles for those three runs were 12.53, 17.05 and 14.03 ms. City streaming ran during the samples.

An eight-second hardware WebGPU playtest with the production candidate followed the character from the initial front impact at 57.5 km/h through ten samples of bonnet contact, across the roof and into recovery after the driver stopped. When a character comes to rest on the roof, recovery places them on free pavement alongside the parked car.
