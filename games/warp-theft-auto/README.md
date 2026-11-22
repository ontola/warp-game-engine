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

Walk through the double doors under the lit strip to enter any office, hotel or dock warehouse. The ground floors have reception desks, conference areas, hotel suites or freight racks. You can walk back out through the same doorway.

Take a parked car or wait alongside a traffic car at a stop. Slow down to get out. A helicopter must land first. On the map, white marks your position and heading, coral marks other players, teal marks available helicopters, and gold marks the current job.

The first **Harbor Transfer** takes a car to the Ferry Terminal, a helicopter to West Field, and a player on foot to Lido Gardens. Any crew member can handle the next leg. Stop at a driving destination and set the skids down at a landing point. A handoff pays the crew $750. Later contracts mark a specific car for recovery wherever the last driver left it. Free roam stays available throughout.

## How the city works

The seed controls lot dimensions, building heights, facade palettes, rooflines and dock stacks. A small grid indexes solid footprints for landing and exit queries. Shared shaders paint lanes, crossings, shopfronts and apartment windows. All views share the city, traffic simulation and eight-minute daylight clock. The city seed sets the cover and wind for a layer of maritime cumulus 1.4–2.4 km above the harbour. Watch the shadows move across the parks or take a helicopter up for a clear view of the cloud banks. [Warp atmosphere](../../engine/docs/atmosphere.md)

- **Interiors.** The 52 buildings keep a seed and a solid exterior at a distance. An on-foot player within 4.5 m of a door or window loads the furnished ground floor and its occupants. It stays while anyone is inside or within 7 m of an opening. The two-sided glass transmits the room light and reflects the street or room behind the viewer. Re-entry regenerates the same layout. All players share the same physical doors, furniture and lights. [Room lighting](../../engine/docs/interior-light.md#streamed-ground-floors-in-luau)
- **Vehicles.** Sideways and this game use the same four-wheel Rapier solver at 120 Hz, including suspension, tyre slip, automatic gears and drivetrain substeps. The sedans also use an adapted Sideways car asset. [Vehicle physics](../../engine/docs/physics.md) · [Model credits](assets/cars/CREDITS.md)
- **Sound.** Solver RPM, load, shift cuts, road speed and tyre slip feed the same exhaust and tyre synthesis used by Sideways. Car and rotor voices move with their vehicles and mix against the active cameras. [Engine audio](../../engine/docs/audio.md)
- **People and trees.** The city clock advances 696 pedestrians and neighbours along sidewalks and around parks. Companions walk abreast and fewer commuters are out after dark. Desks, chairs, sofas and loading aisles determine where workers and guests spend their time. You can see them from the street as you approach a window. Hotels host eight people, offices ten and warehouses nine. All views share up to 40 nearby residents, with four additional Warp Human slots reserved for the players. A person’s seed links height, frame, shoulder breadth, waist and hips across the mesh and animation rig. Two procedurally grown pine shapes are reused across parks and street plantings with instance scale and rotation. Lido Gardens and Market Green use Warp’s GPU meadow grass. Each camera sees individual blades up close and mown stripes in the distance. The park paths and tree grates cut through the turf. [Game kit](../../engine/docs/kit.md)

## Reproduce the playtests

The harbor scenario exercises boarding, braking, flying, acceleration audio and the switch to night. It stages the walker at the heliport between the ground and air checks. The relay scenario stages two players near successive stops, then uses normal controls to complete and credit the job. The four-seat scenario benchmarks a warmed, fixed scene with two walkers, a driver and a pilot. The interior tour walks through the three building types and photographs a warehouse at night. Each scenario runs headlessly.

```sh
cargo test --locked -p warp --test theft_auto --test theft_auto_interiors --test theft_auto_population
warp script games/warp-theft-auto games/warp-theft-auto/scripts/harbor-run.json --out build/theft-harbor
warp script games/warp-theft-auto games/warp-theft-auto/scripts/crew-relay.json --players 2 --out build/theft-relay
warp script games/warp-theft-auto games/warp-theft-auto/scripts/four-seats.json --players 4 --out build/theft-four
warp script games/warp-theft-auto games/warp-theft-auto/scripts/interior-tour.json --out build/theft-interiors
warp script games/warp-theft-auto games/warp-theft-auto/scripts/four-interiors.json --players 4 --seed 1948 --out build/theft-four-rooms
warp script games/warp-theft-auto games/warp-theft-auto/scripts/window-views.json --players 2 --seed 1948 --out build/theft-windows
warp script games/warp-theft-auto games/warp-theft-auto/scripts/park-grass.json --players 4 --seed 1948 --out build/theft-grass
```
