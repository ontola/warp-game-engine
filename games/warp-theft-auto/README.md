# Warp Theft Auto

Port Meridian is a 36-block coastal city you can explore on foot, by car or by helicopter. One to four people share the streets, traffic, time of day and dispatch board. Each local player has a camera and a north-up map. A Splitscreen Club party can change seats during play.

## Play

From the Warp repository root, run the game with Cargo. Use the second command for four local players. The keyboard controls the first seat and controllers drive the other seats.

```sh
cargo run --locked --release -p warp -- run games/warp-theft-auto
cargo run --locked --release -p warp -- run games/warp-theft-auto --players 4
```

A new round generates a new city seed. Pass `--seed 1948` to replay a layout. The city starts at 16:00. A full day takes eight minutes.

| Action | Keyboard and mouse | Controller |
| --- | --- | --- |
| Walk and look | WASD, mouse | Left and right sticks |
| Sprint | Shift | RB or LB |
| Take a vehicle or get out | F | Y / North |
| Accelerate or brake and reverse | W / S | RT / LT or left stick |
| Steer and handbrake | A / D, Space | Left stick, A |
| Fly forward, back or turn | W / S, A / D | Left stick |
| Climb and descend | Space / Shift | A / RB |

Take a parked car or wait alongside a traffic car at a stop. Slow down to get out. A helicopter must land first. On the map, white marks your position and heading, coral marks other players, teal marks available helicopters, and gold marks the current job.

The first **Harbor Transfer** takes a car to the Ferry Terminal, a helicopter to West Field, and a player on foot to Lido Gardens. Any crew member can handle the next leg. Stop at a driving destination and set the skids down at a landing point. A handoff pays the crew $750. Later contracts mark a specific car for recovery wherever the last driver left it. Free roam stays available throughout.

## How the city works

The seed controls lot dimensions, building heights, facade palettes, rooflines and dock stacks. A small grid indexes solid footprints for landing and exit queries. Shared shaders paint lanes, crossings, shopfronts and apartment windows. All views use the same city, traffic simulation and daylight clock.

- **Vehicles.** Sideways and this game use the same four-wheel Rapier solver at 120 Hz, including suspension, tyre slip, automatic gears and drivetrain substeps. The sedans also use an adapted Sideways car asset. [Vehicle physics](../../engine/docs/physics.md) · [Model credits](assets/cars/CREDITS.md)
- **Sound.** Solver RPM, load, shift cuts, road speed and tyre slip feed the same exhaust and tyre synthesis used by Sideways. Car and rotor voices move with their vehicles and mix against the active cameras. [Engine audio](../../engine/docs/audio.md)
- **People and trees.** Seventy-two resident routes advance on the city clock. Eight ambient Warp Humans slots are shared across the views, plus a dedicated slot for each player. Two procedurally grown pine shapes are reused across parks and street plantings with instance scale and rotation. [Game kit](../../engine/docs/kit.md)

## Reproduce the playtests

The harbor scenario exercises boarding, braking, flying, acceleration audio and the switch to night. It stages the walker at the heliport between the ground and air checks. The relay scenario stages two players near successive stops, then uses normal controls to complete and credit the job. The four-seat scenario benchmarks a warmed, fixed scene with two walkers, a driver and a pilot. All three run without opening a game window.

```sh
cargo test --locked -p warp --test theft_auto
warp script games/warp-theft-auto games/warp-theft-auto/scripts/harbor-run.json --out build/theft-harbor
warp script games/warp-theft-auto games/warp-theft-auto/scripts/crew-relay.json --players 2 --out build/theft-relay
warp script games/warp-theft-auto games/warp-theft-auto/scripts/four-seats.json --players 4 --out build/theft-four
```
