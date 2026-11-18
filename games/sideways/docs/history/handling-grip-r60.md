# Earlier tyre bite and more grip — r60

The 27 October playtest feedback was that the car felt like a boat and kept
sliding too long. This first tuning trial raises available tyre grip by 12%
and brings peak lateral force to 80% of the previous slip value. The measured
flat-gravel impulse travels about 10% less distance sideways and settles 50 ms
earlier. This is a small first improvement; subjective handling acceptance
is still pending.

## Configuration and scope

The `[base]` section of [`../data/cars.toml`](../../data/cars.toml) now selects:

```toml
tyre_grip_scale = 1.12
lateral_peak_scale = 0.80
```

Both parameters accept finite values in `0.5..1.5`. Missing parameters default
to `1.0`, and explicitly selecting `1.0/1.0` restores the original tyre tuning.
Models inherit the base values unless overridden. Grip multiplies the existing
surface/weather coefficient in the combined-slip force calculation; the lateral
parameter changes its lateral slip normalization. Tyre forces remain bounded
by that surface's friction budget. Braking and acceleration can therefore change
along with cornering. The slip value used by wheel effects follows the same
normalization.

Physics remains **120 Hz with four tyre/drivetrain substeps**. Relaxation length,
steering rates/assist, inertia, suspension and body damping are unchanged.
This intentionally changes simulation trajectories; it is not an equality claim
against old traces.

The new fields require an executable containing this change. That executable
loads the game's `data/cars.toml` at startup and Splitscreen Club `Prepare`, so later
number changes need no compilation. A bundle uses its own data copy through
the normal game-root locator. New Stage reuses the loaded specifications;
editing the file does not change a running race or reload at stage transitions.

## Deterministic comparison

From the repository root, the exact CPU-only commands used were:

```powershell
$env:CARGO_TARGET_DIR = 'C:/dev/warp-target-20261023'
cargo test --locked -p sideways --lib --example handling_probe
cargo run --locked -p sideways --example handling_probe -- games/sideways/data/cars.toml > C:/dev/perf/warp-sideways-20261027/handling-grip-1.12-0.80.json
```

The probe explicitly compares original `1.0/1.0` and candidate `1.12/0.80`
for Asti and MV6. Each fixture settles the original car on flat gravel for
360 ticks, then initializes identical body, wheel and engine speeds at
60 km/h. There is no random input or renderer. The car coasts in manual third
gear with steering assist enabled and no throttle, brake or handbrake.

One maneuver adds 5 m/s body-right velocity at the centre of mass with no
initial yaw. The other steers `0.35` for 0.75 s, countersteers `-0.18` for
0.25 s, then recentres for four seconds. Recovery is the start of a continuous
0.5 s interval with body slip below 3 degrees and yaw rate below 0.1 rad/s.
Confirmation therefore occurs 0.5 s after the times below. Lateral travel
integrates absolute body-relative sideways speed, rather than net displacement.

| Measurement, original → candidate | Asti | MV6 |
| --- | ---: | ---: |
| Impulse recovery onset, s | 0.450 → 0.400 | 0.467 → 0.417 |
| Impulse lateral travel over 5 s, m | 1.362 → 1.221 | 1.433 → 1.289 |
| Speed at recovery onset, km/h | 58.38 → 58.57 | 58.16 → 58.37 |
| Steer/recentre recovery after release, s | 0.083 → 0.075 | 0.083 → 0.083 |
| Steer/recentre lateral travel after release, m | 0.0253 → 0.0184 | 0.0223 → 0.0214 |
| Peak yaw rate during steering, rad/s | 0.528 → 0.582 | 0.533 → 0.591 |
| Bench 100–0 braking distance, m | 42.3 → 37.8 | 41.6 → 37.2 |
| Bench 0–100 time, s | 3.65 → 3.57 | 3.53 → 3.41 |
| Bench skidpad lateral acceleration, g | 1.15 → 1.28 | 1.18 → 1.31 |
| Bench handbrake heading change, degrees | 147 → 143 | 147 → 142 |

All eight maneuver runs stayed finite with four contacting wheels. Their
initialized states matched exactly. The JSON at the command's output path
retains full states, recovery confirmation, speed and yaw measurements.

## Checks and limits

43 library tests and two transient comparison tests passed; two existing
geometry diagnostics were ignored. Tests cover old-file defaults, bounded
configuration, finite/contact-gated forces, retained wet-surface differences,
and at least 5% less impulse lateral travel without slower recovery. The
existing handling bench also passed for base, Asti, MV6 and Toyama in both
configurations.

Two benchmark limits explicitly account for the intended friction increase:
the 0–100 minimum time is `3.5 / tyre_grip_scale` seconds, and the skidpad band
is `[0.75, 1.25] * tyre_grip_scale` g. Original `1.0` settings retain the exact
old limits. At `1.12` these become 3.125 s and 0.84–1.40 g. The 6.5 s
acceleration maximum, 35–60 m braking, 60–180 degree handbrake turn, rest-drift
and top-speed limits remain unchanged. The unadjusted old limits would reject
MV6's faster launch and the higher skidpad grip.

These are flat-gravel, coasting comparisons. They do not establish subjective
feel, powered-slide recovery, rough-road or wet-road behavior, or input/display
latency. MV6's measured steer/recentre recovery time is unchanged. Real
Splitscreen Club driving remains the acceptance step.
