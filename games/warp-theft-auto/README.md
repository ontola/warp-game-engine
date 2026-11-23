# Warp Theft Auto

Port Meridian is a 36-block coastal city you can explore on foot, by car or by helicopter. One to four people share the streets, traffic, time of day and dispatch board. Each local player has a camera and a north-up map. A Splitscreen Club party can change seats during play.

## Play

[Play Warp Theft Auto on Splitscreen Club](https://splitscreen.club/playtests/warp-theft-auto/v1.1/?game=warp-theft-auto&players=1)

The browser playtest supports one keyboard player and additional seats on controllers. On the solo game link, click the city for mouse look and sound, then use WASD or F. A controller can take the first seat with A. In the Game Room, Enter joins the keyboard and A joins a controller. The host requires WebGPU. On first launch, the **Preparing graphics** screen stays up while the GPU compiles the city’s shaders. Browser interiors sample bounced window and fixture light as you approach. Devices with at least 17 sampled textures per shader stage can also draw the glass. To build the same single-game shelf locally, run `python3 engine/tools/build-web.py --warp --games warp-theft-auto --luau-only --serve`, then open `http://localhost:8790/?game=warp-theft-auto&players=1`.

From the Warp repository root, run the game with Cargo. Use the second command for four local players. The keyboard controls the first seat and controllers drive the other seats.

```sh
cargo run --locked --release -p warp -- run games/warp-theft-auto
cargo run --locked --release -p warp -- run games/warp-theft-auto --players 4
```

A new round generates a new city seed. Pass `--seed 1948` to replay a layout. The city starts at 16:00. A full day takes eight minutes. The sun moves between simulation ticks, the sky changes continuously and the street lighting fades through twilight.

| Action | Keyboard and mouse | Controller |
| --- | --- | --- |
| Walk and look | WASD, mouse | Left and right sticks |
| Sprint | Shift | RB or LB |
| Stories | R, arrows and Space | X, D-pad and A |
| Edit a Luau story in the browser | F2 | |
| Aim and fire the arcade pistol on foot | Hold right mouse to aim, left-click or tap Space to fire | LT and RT |
| Take a vehicle or get out | F | Y / North |
| Accelerate or brake and reverse | W / S | RT / LT or left stick |
| Steer and handbrake | A / D, Space | Left stick, A |
| Fly forward, back or turn | W / S, A / D | Left stick |
| Climb and descend | Space / Shift | A / RB |

Walk through the entrance under the lit strip to enter any office, hotel or dock warehouse. The ground floors have reception desks, conference areas, hotel suites or freight racks. You can walk back out through the same doorway.

Take a parked car or wait alongside a traffic car at a stop. Slow down to get out. A helicopter must land first. On the map, white marks your position and heading, coral marks other players, teal marks available helicopters, and gold marks the current job.

Every on-foot player carries a pistol. Click the city to capture the mouse. Left-click, tap Space or press RT to fire. Hold the right mouse button or LT for the shoulder view. The free-roam pistol reaches 75 metres, far enough to shoot across a city block. The crosshair turns coral over a person in range. A brief streak shows the shot, and a cross at the impact point confirms a hit. You can shoot passing pedestrians, people inside buildings, and story characters. Shooting someone needed by a delivery story can fail that mission.

Rook and Vex patrol across the starting intersection under red chevrons. They take two hits and return after 12 seconds. Players in different seats can hit the same person or complete a contract together.

Gold diamonds follow the current person or car in the world. At a destination, a gold ring shows where to stop or land. Race checkpoints add four tall posts around the crossing area. Every seat sees the same objective, including when a custom Luau story moves its target.

The first **Harbor Transfer** takes a car to the Ferry Terminal, a helicopter to West Field, and a player on foot to Lido Gardens. Any crew member can handle the next leg. Stop at a driving destination and set the skids down at a landing point. A handoff pays the crew $750. Later contracts mark a specific car for recovery wherever the last driver left it. Free roam stays available throughout.

## Write and play a story

Press **R** or controller **X** to choose a shared mission. **Last Ferry** has Mara walk to the curb, ride in your car and walk to the ferry after you drop her off. **Exchange Street Sprint** seats you in a designated pearl sedan and times five checkpoint crossings. **The Silver Jack Contract** gives the mission owner a named target to track and shoot. Choose **Harbor dispatch** to return to the original delivery jobs.

In the browser, click **Luau stories** or press **F2**. Pick a template, edit its Luau source and choose **Run story**. The city and the other players keep their positions. A successful edit begins the new story at its first step. Each template stores its draft in this browser. **Save .luau** downloads a script you can share, and **Open file** loads one from your device. **Restore template** brings back the shipped source.

A story is a Luau program that returns a table. Give the story an ID and title, define a cast of named people and cars, then return its ordered steps. You can generate coordinates and dialogue using locals, loops and functions. For example, these steps make a courier follow a route you generate:

```lua
local route = {}
for i = 1, 4 do
    table.insert(route, vector.create(8, 0, 18 + i * 3))
end
return {
    id = "quayside", title = "Quayside Courier",
    cast = {
        {kind="person", key="courier", name="Kit", look=23, pos=vector.create(8,0,18)},
    },
    steps = {
        {op="walk", person="courier", points=route, speed=1.5, text="Walk with Kit to the curb."},
        {op="pickup", person="courier", radius=8, text="Stop beside Kit."},
        {op="dropoff", person="courier", pos=vector.create(109,0,60), radius=7, text="Drop Kit off at the harbor."},
    },
}
```

Start with the [passenger script](story_passenger.module.luau), [checkpoint race](story_race.module.luau) or [contract](story_contract.module.luau) for more actions. Positions are in metres. `look` selects a persistent Warp Human appearance. Cars have their own key, name, position, heading and paint. A story supports eight named people and four named cars. The action list supports walking and driving routes, boarding or leaving a named car, a player entering a car, passenger pickups and drop-offs, ordered race gates and an arcade contract target. To ship another story template, add its module to `stories.json`. Native builds can evaluate a single-line program with the `story` console command or call `apply_story(definition)` through Warp's live Luau console.

## How the city works

The seed controls lot dimensions, building heights, facade palettes, rooflines and dock stacks. A small grid indexes solid footprints for landing and exit queries. Shared shaders paint lanes, crossings, shopfronts and apartment windows. All views share the city, traffic simulation and eight-minute daylight clock. The city seed sets the cover and wind for a layer of maritime cumulus 1.4–2.4 km above the harbour. Watch the shadows move across the parks or take a helicopter up for a clear view of the cloud banks. [Warp atmosphere](../../engine/docs/atmosphere.md)

- **Interiors.** From the street, each of the 52 ground floors shows a GPU-rendered view through its doors and windows. The storefront glass has a light smoke tint. In daylight it reflects the sky, paving and a coarse view of the surrounding blocks. Reflections strengthen at grazing angles. The open entrance shows the room directly, and lit interiors remain visible through glass after dusk. Look sideways to see past the reception desk or warehouse racks. The preview follows the building footprint and its seeded furniture orientation and finishes. Walking within 4.5 m loads the furnished room, its occupants, collision and lighting. The real room gradually takes over by 1.25 m, behind two-sided glass at the windows. Backing away reverses the transition. The detailed room unloads once the last visitor is more than 7 m from an opening. Each split-screen player sees the transition from their own position. [Room lighting](../../engine/docs/interior-light.md#streamed-ground-floors-in-luau)
- **Vehicles.** The city places 31 parked sedans along its curbs and 12 more on moving routes. Slow vehicles separate walkers from the chassis. Faster impacts use Warp's jointed ragdolls, and people stand up when the road clears. Sideways and this game use the same four-wheel Rapier solver at 120 Hz, including suspension, tyre slip, automatic gears and drivetrain substeps. The sedans also use an adapted Sideways car asset. [Vehicle physics](../../engine/docs/physics.md) · [Model credits](assets/cars/CREDITS.md)
- **Sound.** Solver RPM, load, shift cuts, road speed and tyre slip feed the same exhaust and tyre synthesis used by Sideways. Car and rotor voices move with their vehicles and mix against the active cameras. [Engine audio](../../engine/docs/audio.md)
- **People and trees.** The city clock advances 696 pedestrians and neighbours along sidewalks and around parks. Companions walk abreast and fewer commuters are out after dark. Desks, chairs, sofas and loading aisles determine where workers and guests spend their time. You can see them from the street as you approach a window. Hotels host eight people, offices ten and warehouses nine. All views share up to 40 fitted residents and four reserved player avatars. Further down the street, the same residents walk as instanced, low-detail Warp Humans. The 72 resident appearances combine seeded builds, skin and hair with nine outfit and groom combinations, 24 shirt and jacket colors, eight trouser colors and six shoe colors. A person’s seed links height, frame, shoulder breadth, waist and hips across the mesh and animation rig. Two procedurally grown pine shapes are reused across parks and street plantings with instance scale and rotation. Lido Gardens and Market Green use Warp’s GPU meadow grass. Each camera sees individual blades up close and mown stripes in the distance. The park paths and tree grates cut through the turf. [Game kit](../../engine/docs/kit.md)

## Reproduce the playtests

The harbor scenario exercises boarding, braking, flying, acceleration audio and the switch to night. It stages the walker at the heliport between the ground and air checks. The relay scenario stages two players near successive stops, then uses normal controls to complete and credit the job. The four-seat scenario benchmarks a warmed, fixed scene with two walkers, a driver and a pilot. The interior tour walks through the three building types and photographs a warehouse at night. Each scenario runs headlessly.

```sh
cargo test --locked -p warp --test theft_auto --test theft_auto_interiors --test theft_auto_population --test theft_auto_objectives --test theft_auto_combat
warp script games/warp-theft-auto games/warp-theft-auto/scripts/harbor-run.json --out build/theft-harbor
warp script games/warp-theft-auto games/warp-theft-auto/scripts/crew-relay.json --players 2 --out build/theft-relay
warp script games/warp-theft-auto games/warp-theft-auto/scripts/four-seats.json --players 4 --out build/theft-four
warp script games/warp-theft-auto games/warp-theft-auto/scripts/interior-tour.json --out build/theft-interiors
warp script games/warp-theft-auto games/warp-theft-auto/scripts/four-interiors.json --players 4 --seed 1948 --out build/theft-four-rooms
warp script games/warp-theft-auto games/warp-theft-auto/scripts/window-views.json --players 2 --seed 1948 --out build/theft-windows
warp script games/warp-theft-auto games/warp-theft-auto/scripts/park-grass.json --players 4 --seed 1948 --out build/theft-grass
```
