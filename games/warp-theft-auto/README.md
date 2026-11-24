# Warp Theft Auto

Port Meridian is a 36-block coastal city you can explore on foot, by car or by helicopter. One to four people share the streets, traffic, time of day and dispatch board. Each local player has a camera and a north-up map. A Splitscreen Club party can change seats during play.

## Play

[Play Warp Theft Auto on Splitscreen Club](https://splitscreen.club/playtests/warp-theft-auto/v1.1/?game=warp-theft-auto&players=1)

[Play with low material detail](https://splitscreen.club/playtests/warp-theft-auto/v1.1/?game=warp-theft-auto&players=1&detail=low) for simpler stone, glass and window interiors. On desktop set `WARP_SURFACE_DETAIL=low` before starting Warp. The regular profile shades brick bevels, recessed mortar and inset window frames in the moving sunlight. Street-level windows show people before the fully furnished room loads.

The browser playtest supports one keyboard player and additional seats on controllers. On the solo game link, click the city for mouse look and sound, then use WASD or F. A controller can take the first seat with A. In the Game Room, Enter joins the keyboard and A joins a controller. The host requires WebGPU. On first launch, the **Preparing graphics** screen stays up while the GPU compiles the cityâ€™s shaders. Browser interiors sample bounced window and fixture light as you approach. Devices with at least 17 sampled textures per shader stage can also draw the glass. To build the same single-game shelf locally, run `python3 engine/tools/build-web.py --warp --games warp-theft-auto --luau-only --serve`, then open `http://localhost:8790/?game=warp-theft-auto&players=1`.

From the Warp repository root, run the game with Cargo. Use the second command for four local players. The keyboard controls the first seat and controllers drive the other seats.

```sh
cargo run --locked --release -p warp -- run games/warp-theft-auto
cargo run --locked --release -p warp -- run games/warp-theft-auto --players 4
```

Follow Exchange Avenue west through Lido Gardens to reach the 1.68 km paved loop. Both branches lead to the red-roofed shelter at Cape Lookout. The ochre line on the minimap marks a 907 m gravel trail connecting the two branches through Mariner Wood. Expect hairpins and a loose surface on the climb. Press R to select Mariner Ridge Rally for a timed run.

Shoot the red crate with the yellow hazard stripe across the junction from the starting cars on Exchange Avenue. There are two more crates at the eastern docks. A shot ignites the crate, damaging exposed people and cars within 14 metres. The crate returns after 15 seconds once the site is clear. For a scripted blast, add a story step such as `{op="explosion",pos=vector.create(10.7,.625,-10.8),radius=5,text="Signal from the docks"}`. It plays once when the story reaches that step.

A new round generates a new city seed. Pass `--seed 1948` to replay a layout. The city starts at 16:00. A full day takes eight minutes. The sun moves between simulation ticks, the sky changes continuously and the street lighting fades through twilight.

| Action | Keyboard and mouse | Controller |
| --- | --- | --- |
| Walk and look | WASD, mouse | Left and right sticks |
| Sprint | Shift | RB |
| Stories | R, arrows and Space | X, D-pad and A |
| Edit a Luau story or West Cape in the browser | F2 | |
| Weapon wheel on foot | Hold Q, point with the mouse or use the left/right arrows, release Q to equip | Hold LB, point with the right stick or use the D-pad, release LB |
| Aim and fire the equipped weapon on foot | Hold right mouse to aim, left-click or tap Space to fire | LT and RT |
| Take a vehicle or get out | F | Y / North |
| Accelerate or brake and reverse | W / S | RT / LT or left stick |
| Steer and handbrake | A / D, Space | Left stick, A |
| Fly forward, back or turn | W / S, A / D | Left stick |
| Climb and descend | Space / Shift | A / RB |

Walk through the entrance under the lit strip to enter any office, hotel or dock warehouse. The ground floors have reception desks, conference areas, hotel suites or freight racks. You can walk back out through the same doorway.

Each of the twelve traffic cars has a seated Warp Human driver. You can see and hit the driver through the windows. A metal door or pillar can stop the shot. Taking a slow car makes its driver step out and run. Trapped drivers also try to get out when their escape stalls or the car catches fire.

Traffic drivers flee when a shot strikes their car or passes close to its body. They accelerate, take intersections without waiting, and leave less braking room. A driver facing the shooter may turn around into the opposite lane. After several quiet seconds the driver returns to normal speed and yields again.

Take a parked car or wait alongside a traffic car at a stop. Slow down to get out. A helicopter must land first. On the map, white marks your position and heading, coral marks other players, teal marks available helicopters, and gold marks the current job.

Every player starts with the pistol equipped. The wheel also contains a grenade launcher, the stock Growing Guns rifle, and its Shotgun, Uzi and Sniper upgrades. Hold the trigger to repeat Growing Guns shots. Their magazines refill after the last round. Aiming the sniper narrows the field of view. Each player can change weapons independently while the city keeps moving. The launcher fires a visible projectile that arcs through the scene. The fireball has a 5.5 m effect radius. Damage reaches 17 m, falls off quadratically, and stops at cover. Close hits on people produce Growing Gunsâ€™ red mist and fragments from their current pose. Nearby fallen people keep their last physical pose. A second shot can move a limb, and a close explosion can fragment the body. Wheels can hit a fallen pedestrian on a later pass. These follow-up hits give no additional defeat credit.

Car impacts push the painted mesh inward around the contact point. Repeated local damage or a severe crash can tear off a door with its window and mirror, the bonnet, the boot lid or a wheel. Loose parts carry the vehicleâ€™s velocity. Missing wheels affect acceleration and steering. At 82% damage the engine bay catches fire. You have six seconds to get clear. The paint scorches halfway through, then the car explodes and throws off its remaining panels and wheels. A direct grenade hit blows the car up immediately. The open engine bay and cabin keep burning after the blast. The flames taper over fourteen seconds and the wreck smokes for another ten. The four paint variants share one set of panel geometry. Run `python games/warp-theft-auto/scripts/split_sedan.py --check` to verify the cooked parts against the Sideways model.

Click the city to capture the mouse. Left-click, tap Space or press RT to fire. Hold the right mouse button or LT for the shoulder view. The free-roam pistol reaches 75 metres, far enough to shoot across a city block. The crosshair turns coral over a person in range. A short flash at the muzzle marks each shot. A small streak can briefly catch the light along the shot path. A cross at the impact point confirms a hit. You can shoot passing pedestrians, people inside buildings, and story characters. Shooting someone needed by a delivery story can fail that mission. Nearby civilians run when any player fires, even if the shot misses. Gunfire carries 54 metres outdoors and within the same building, or 18 metres across a building boundary. Sidewalk crowds sprint around their block. Park neighbours and indoor workers seek shelter, then walk back to their activities once the shooting stops.

Rook and Vex patrol across the starting intersection under red chevrons. They take two hits and return after 12 seconds. Players in different seats can hit the same person or complete a contract together.

Gold diamonds follow the current person or car in the world. At a destination, a gold ring shows where to stop or land. Race checkpoints add four tall posts around the crossing area. Every seat sees the same objective, including when a custom Luau story moves its target.

The first **Harbor Transfer** takes a car to the Ferry Terminal, a helicopter to West Field, and a player on foot to Lido Gardens. Any crew member can handle the next leg. Stop at a driving destination and set the skids down at a landing point. A handoff pays the crew $750. Later contracts mark a specific car for recovery wherever the last driver left it. Free roam stays available throughout.

## Edit West Cape in Luau

Press **F2**, select **West Cape layout**, then choose **Apply layout**. The template is the [authored map](west_cape_layout.module.luau). Positions are metres in the X/Z plane, with north towards negative Z. For a first edit, change the rally bend `{-738,-52,4}` to `{-746,-52,4}`. The last number attaches a checkpoint to the bend. Apply the edit and drive through the new turn. **Restore template**, then **Apply layout**, reinstates the published map. **Save .luau** exports a draft.

The definition includes coast slices, the connection to Exchange Avenue, asphalt and gravel bends and widths, hills, forest stands and named places. Moving `places.ferryTerminal` moves the moored ferry and redirects the passenger and dispatch missions. The game rebuilds the affected terrain, road surfaces, collision, tyre grip, minimap and rally positions in the running session. All local seats share the edit. Coast slices meet Port Meridian at its western edge. The grain-field pattern and woodland ground tint are painted in the terrain shader. The city's 36-block street grid is authored separately in [the city generator](world.module.luau).

On desktop, edit the same file while the game is running, or use the live Luau console:

```luau
local map = get_layout()
map.rally.bends[6][1] -= 8
return apply_layout(map)
```

Shipping builds can use a prebuilt mesh. Run `python games/warp-theft-auto/scripts/bake_west_cape.py` after settling on a design, then `--check` to verify it. The exporter runs the Luau definition and compiler and records the geometry signature alongside the mesh. Edits with a different geometry signature use the live mesh builder.

## Write and play a story

Press **R** or controller **X** to choose a shared mission. **Last Ferry** has Mara walk to the curb, ride in your car and walk to the ferry after you drop her off. **Exchange Street Sprint** seats you in a designated pearl sedan and times five checkpoint crossings. **The Silver Jack Contract** gives the mission owner a named target to track and shoot. **Mariner Ridge Rally** marks a red Kestrel at the eastern gravel trailhead and times eleven gates through the wooded hills. The approach from the city is untimed. Choose **Harbor dispatch** to return to the original delivery jobs.

In the browser, click **Luau stories** or press **F2**. Pick a template, edit its Luau source and choose **Run story**. The city and the other players keep their positions. A successful edit begins the new story at its first step. Each template stores its draft in this browser. **Save .luau** downloads a script you can share, and **Open file** loads one from your device. **Restore template** brings back the shipped source.

To try weapon editing immediately, open **F2**, select **Custom loadout**, change a value and choose **Run story**. A story can equip its owner with a Growing Guns preset and change its parameters while the city is running. This action gives the player a four-projectile shotgun.

```lua
{op="equip", weapon="shotgun", params={pellets=4, magazine=8, spread=0.05}, text="Collect the dockside special."},
```

The Growing Guns preset IDs are `rifle`, `shotgun`, `uzi` and `sniper`. Each accepts `pellets`, `spread` (radians), `interval` and `reload` (seconds), `magazine`, `damage` (in base-rifle hits), `range`, `automatic`, `headshot` (multiplier), `aimFov` (degrees) and the synthâ€™s `velocity` setting. The fitted model reads the same parameters as the firing and sound code. The weapon wheel remembers a playerâ€™s custom build. An equip step without `params` restores the preset. Use `pistol` or `launcher` to equip the city weapons. The data and synth patches come from Growing Guns. Run `python games/warp-theft-auto/scripts/sync_growing_guns.py --check` to verify them against the original game.

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

The seed controls lot dimensions, building heights, facade palettes, rooflines and dock stacks. A small grid indexes solid footprints for landing and exit queries. Shared shaders paint lanes, crossings, shopfronts and apartment windows. All views share the city, traffic simulation and eight-minute daylight clock. The city seed sets the cover and wind for a layer of maritime cumulus 1.4â€“2.4 km above the harbour. Watch the shadows move across the parks or take a helicopter up for a clear view of the cloud banks. [Warp atmosphere](../../engine/docs/atmosphere.md)

- **Interiors.** From the street, each of the 52 ground floors shows a GPU-rendered view through its doors and windows. The storefront glass has a light smoke tint. In daylight it reflects the sky, paving and a coarse view of the surrounding blocks. Reflections strengthen at grazing angles. The open entrance shows the room directly, and lit interiors remain visible through glass after dusk. Look sideways to see past the reception desk or warehouse racks. The preview follows the building footprint and its seeded furniture orientation and finishes. Walking within 4.5 m loads the furnished room, its occupants, collision and lighting. The real room gradually takes over by 1.25 m, behind two-sided glass at the windows. Backing away reverses the transition. The detailed room unloads once the last visitor is more than 7 m from an opening. Each split-screen player sees the transition from their own position. [Room lighting](../../engine/docs/interior-light.md#streamed-ground-floors-in-luau)
- **Vehicles.** The city places 31 parked sedans along its curbs and 12 more on moving routes. Slow vehicles separate walkers from the chassis. On a faster impact the pedestrian keeps their walking velocity. The moving bumper and bonnet then catch individual limbs, allowing someone to tip over the front of a sedan. Warp Humans briefly draws the arms and knees inward during a fall. People can get back up once the road clears. Sideways and this game use the same four-wheel Rapier solver at 120 Hz, including suspension, tyre slip, automatic gears and drivetrain substeps. The sedans also use an adapted Sideways car asset. The painted body panels preserve their surface normals as the mesh simplifies in the distance, so reflections run across the doors and wheel arches. [Vehicle physics](../../engine/docs/physics.md) Â· [Model credits](assets/cars/CREDITS.md)
- **Street furniture.** Cast-iron grates sit in the gutters, and round access covers are set into the asphalt between lane markings. The road shader draws their rims, slots and tread at walking distance and filters them as the camera pulls away. Along the kerb, curved streetlamps light the pavement after dusk and public litter bins sit by busy frontages and transit stops. At the start of a match you can see a grate, bin and lamp together on the east side of Exchange Avenue. The city shares two texture-free meshes and keeps up to five nearby lamp lights per local seat.
- **West Cape.** The city and cape cover 0.60 km². The forest rises to 29 m between the two coastal roads. A shared grid of 220 height samples aligns the visible hills with walking, vehicle suspension, landing and mission checkpoints. Gravel, asphalt and grass have different tyre grip. Ground colours mark hay meadows, woodland, heath and rocky ledges. Woodland stands and roadside planting reuse texture-free models, with trunk colliders shared between nearby players. The cape mesh, including both paved branches and the rally trail, fits in 76 KiB compressed.
- **Sound.** Solver RPM, load, shift cuts, road speed and tyre slip feed the same exhaust and tyre synthesis used by Sideways. The sedan muffler rolls off upper harmonics at idle and opens with engine load. Car and rotor voices move with their vehicles and mix against the active cameras. [Engine audio](../../engine/docs/audio.md)
- **People and trees.** The city clock advances 696 pedestrians and neighbours along sidewalks and around parks. Companions walk abreast and fewer commuters are out after dark. Desks, chairs, sofas and loading aisles determine where workers and guests spend their time. You can see them from the street as you approach a window. Hotels host eight people, offices ten and warehouses nine. All views share up to 40 fitted residents and four reserved player avatars. Further down the street, the same residents walk as instanced, low-detail Warp Humans. The 72 resident appearances combine seeded builds, skin and hair with nine outfit and groom combinations, 24 shirt and jacket colors, eight trouser colors and six shoe colors. A personâ€™s seed links height, frame, shoulder breadth, waist and hips across the mesh and animation rig. Broadleaf trees and low flowering shrub beds line the park paths. Seeded planting extends along Exchange Avenue and behind the residential blocks. All 52 broadleaf trees and 27 planting beds reuse two texture-free models (50,868 bytes combined before compression). Luau places and rotates them locally from the city seed. Lido Gardens and Market Green use Warpâ€™s GPU meadow grass. Each camera sees individual blades up close and mown stripes in the distance. The park paths and tree grates cut through the turf. [Game kit](../../engine/docs/kit.md)

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
