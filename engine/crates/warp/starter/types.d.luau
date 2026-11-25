-- The Luau API of games run by the `warp` executable. Not loaded as a
-- module (files ending in .d.luau are skipped); point an editor's Luau
-- language server at it (luau-lsp: --definitions=types.d.luau) for
-- completion and checks. Full guide: engine/docs/warp-cli.md.
--
-- Units: metres, seconds, radians (camera fov in degrees). +Y is up; a yaw
-- of 0 faces -Z. Colours are sRGB 0..1: "#rrggbb", "#rrggbbaa", a vector,
-- {r, g, b, a} or a number (grey). Options tables reject unknown keys.
--
-- A module (main.luau, or any other *.luau outside sounds/, assets/ and
-- scripts/) returns a table of hooks, all optional:
--   init()               once, when the module first loads (and on restart)
--   tick(dt, input)      every tick, at game.toml's tick_rate; dt = 1 / tick_rate
--   draw()               every displayed frame: HUD and camera only
--   reload()             after a saved edit reloads the module
--   player_joined(p)     Splitscreen Club seated someone new mid-game (p: Player)
--   player_left(p)       a player left mid-game
--   players_changed()    names, colours or seats changed (after joins and leaves too)
--   couch_changed(order) the people's left-to-right couch order changed (a move in
--                        the lobby mid-game, or a join or leave); order = players.by_couch()
-- Folder games: local lib = require("lib/name") loads lib/name.module.luau at module load time.
-- libraries("content/cards") lists the libraries in a folder, sorted, for content
-- collections: `for _, name in libraries("content/cards") do local card = require(name) end`.
-- Modules run their hooks in path order. `state` is a table per module that
-- survives reloads; main.luau's `state` is the game's JSON state (`/state`).
--
-- Players: people on controllers and bots, `players.list()`. Under
-- Splitscreen Club the party's seats decide them (a seat change never restarts
-- the game); without it game.toml's players and bots (or --players and
-- --bots), filled with bots up to min_players. `input[p.id]` is a
-- person's controls; a bot's input is neutral and the scripts drive it.
-- Start (Enter, Escape) opens the start menu (title, Resume, New round):
-- the game is frozen while it is open, and New round restarts it.
--
-- Determinism: the same seed and inputs play out the same way. Everything
-- that changes the game (world.spawn/set/remove/move/clear, random.*,
-- sound.play) is an error inside draw(), which runs a varying number of
-- times per tick. There is no clock, no file access and no math.random.

export type Vec = vector
export type Color = string | vector | { number } | { r: number, g: number, b: number, a: number? } | number

export type Buttons = {
	south: boolean, east: boolean, west: boolean, north: boolean,
	left_shoulder: boolean, right_shoulder: boolean, left_trigger: boolean, right_trigger: boolean,
	select: boolean, start: boolean, up: boolean, down: boolean, left: boolean, right: boolean,
	-- Aliases: jump = south (Space, A), fire = right_trigger (left mouse, RT), aim = left_trigger.
	jump: boolean, fire: boolean, aim: boolean,
}

-- One local player's input this tick (keyboard and mouse, or a gamepad).
export type Input = {
	move_x: number, -- left stick or A/D: -1 left .. 1 right
	move_y: number, -- left stick or W/S: -1 back .. 1 forward
	look_x: number, -- look change this tick, radians, right positive (mouse, right stick)
	look_y: number, -- up positive
	aim_x: number, -- where the right stick points (twin-stick aim): -1 left .. 1 right, 0 on keyboard and mouse
	aim_y: number, -- -1 down .. 1 up
	held: Buttons, -- held now
	pressed: Buttons, -- pressed since the previous tick
}

export type EntityOptions = {
	shape: string?, -- world.set only: "cube", "sphere", "capsule", "outline6", "wire" or "assets/x.glb"
	pos: Vec?, -- centre, default (0, 0, 0)
	rot: Vec?, -- radians: x pitch, y yaw, z roll; collision boxes follow the yaw only
	size: (Vec | number)?, -- metres; 1 fills a 1 m box (the capsule is 2 m tall at size 1)
	color: Color?, -- default light grey
	emissive: number?, -- glow: 0 off, 1 bright in daylight
	roughness: number?, -- 0 mirror .. 1 matte (default 0.7)
	metal: number?, -- 0 .. 1
	surface: number?, -- a kind (0 .. 255) of the game's own shader, surface.wgsl; false for none
	coverage: (number | false)?, -- opaque primitive proxy coverage from 0 to 1; false restores the ordinary material
	points: { Vec }?, -- "wire" only: points in the entity's own space (set them each tick to animate)
	edges: { { number } }?, -- "wire" only: pairs of 1-based point numbers, or a flat list
	width: number?, -- "wire" only: edge thickness in metres at size 1 (default 0.05)
	solid: boolean?, -- blocks world.move and raycasts (default true)
	visible: boolean?, -- default true
	hide_views: number?, -- mask: bit i hides it from view i (shadows stay); default 0
	shadow_only: boolean?, -- casts shadows but no camera sees it (a ceiling over a roofless room); default false
	tag: string?, -- for world.find, overlap and raycast filters
}

export type Player = {
	id: number, -- 1, 2, ...: stable for the session, never reused
	name: string,
	color: Vec, -- sRGB 0..1, the lobby's colour under Splitscreen Club
	bot: boolean,
	human: boolean,
	view: number?, -- assigned zero-based camera index, including shared split-screen views
	seat: number?, -- the Splitscreen Club seat
	couch: number?, -- couch position from the lobby, 0 leftmost
}

export type Entity = {
	id: number, parent: number?, world_pos: Vec, shape: string, pos: Vec, rot: Vec, size: Vec, color: Vec,
	emissive: number, roughness: number, metal: number, surface: number?, coverage: number?, solid: boolean, visible: boolean, shadow_only: boolean, hide_views: number, tag: string,
}

export type Query = {
	tag: string?, -- only entities with this tag
	ignore: (number | { number })?, -- skip these entities
	all: boolean?, -- raycast: include entities that are not solid
}

export type Hit = { id: number, distance: number, point: Vec, normal: Vec }

-- Triangle indices are 1-based. Supply a unit normal per vertex and UVs
-- as vector.create(u,v,0). The part's material applies to all its triangles.
export type MeshPart = {
    positions: {Vec}, normals: {Vec}, uvs: {Vec}?, indices: {number},
    color: Color?, surface: number?, metal: number?, roughness: number?,
}

declare world: {
    -- One editable-world mesh entity with immutable local geometry. Removing
    -- the entity releases its mesh. Solid uses its bounding box; use a
    -- heightfield for a physical terrain. Each mesh can have up to 32 parts,
    -- 65536 vertices and 196608 indices.
    -- Returns (id,nil) or (nil,error) without modifying the world on an invalid request.
    mesh: (parts: {MeshPart}, opts: {pos:Vec?, rot:Vec?, size:(Vec | number)?, tag:string?, solid:boolean?, visible:boolean?}?) -> (number?, string?),
	-- A new entity; returns its id. Shapes: "cube", "sphere", "capsule",
	-- "outline<sides>[:<width>]" (a flat hollow polygon, "outline6"), or a
	-- glTF model in the game folder ("assets/crate.glb"). A new shape and look
	-- adds a mesh: free in init, a short stall later.
	spawn: (shape: string, opts: EntityOptions?, parent: number?) -> number,
	-- Keep pos/rot local to the new parent (nil detaches). Removing a parent removes its subtree.
	set_parent: (id: number, parent: number?) -> (),
	-- Change any options of an entity.
	set: (id: number, opts: EntityOptions) -> (),
	-- Only move (and turn, yaw in radians) an entity: the cheap call for
	-- things that move every tick.
	place: (id: number, pos: Vec, yaw: number?) -> (),
	-- Prepare a shape and look in init, so spawning it later never stalls.
	warm: (shape: string, look: { color: Color?, emissive: number?, roughness: number?, metal: number?, surface: number?, coverage: (number | false)? }?) -> (),
	-- An entity's fields, or nil once removed.
	get: (id: number) -> Entity?,
	exists: (id: number) -> boolean,
	remove: (id: number) -> boolean,
	clear: () -> (),
	-- Ids with this tag, in id order.
	find: (tag: string) -> { number },
	count: () -> number,
	-- The first solid entity box along a ray (max default 1000 m).
	raycast: (origin: Vec, dir: Vec, max: number?, opts: Query?) -> Hit?,
	-- Entities (solid or not) whose box touches a sphere, in id order.
	overlap: (center: Vec, radius: number, opts: Query?) -> { number },
	-- Move an entity by vel * dt as an upright cylinder (its box's width and
	-- height), sliding along solid boxes.
	-- Returns the velocity after collisions and whether it stands on something.
	move: (id: number, vel: Vec, dt: number, opts: Query?) -> (Vec, boolean),
}

-- Effects advance on the game clock. Bursts and rings use non-solid entities
-- tagged "fx" and can warm their looks in init.
declare fx: {
    -- Growing Guns fire, smoke, embers, heat distortion and local lights in all views.
    -- Radius is in metres (0 < radius <= 200). This visual effect uses no entity slots.
    warm_explosions: () -> (),
    explosion: (pos: Vec, radius: number, lightRange: number?) -> (),
    gib_mist: (pos: Vec, intensity: number) -> (),
    -- One upright flame rooted at pos, in metres and seconds, or one rising smoke puff.
    flame: (pos: Vec, width: number, height: number, life: number) -> (),
    exhaust_smoke: (pos: Vec, radius: number) -> (),
	-- Pieces flying out from pos: each at 25-100 % of speed (m/s), living
	-- 50-100 % of life (s) and shrinking. flat: in the ground plane only;
	-- stretch: seconds of velocity as length (streaks); gravity: m/s^2 down.
	-- Defaults: 12 cubes of 0.12 m, speed 4, life 0.4, emissive 0.06.
	burst: (pos: Vec, opts: { color: Color?, emissive: number?, roughness: number?, metal: number?, shape: string?, count: number?, speed: number?, life: number?, size: number?, stretch: number?, gravity: number?, flat: boolean? }?) -> (),
	-- A ring of thin cubes in the ground plane, growing out to radius (m)
	-- over life (s). Defaults: radius 2, life 0.35, width 0.08, 24 segments.
	ring: (pos: Vec, opts: { color: Color?, emissive: number?, roughness: number?, metal: number?, radius: number?, life: number?, width: number?, segments: number? }?) -> (),
	-- Effects alive now: particles, rings.
	count: () -> (number, number),
}

declare players: {
	-- Every player: people first (in couch order when the game starts; the order stays
	-- put mid-game, see by_couch), then bots.
	list: () -> { Player },
	get: (id: number) -> Player?,
	count: () -> number,
	-- Rumble a person's controller: strength 0..1 for `seconds` (default 0.2).
	-- Only under Splitscreen Club while the game runs; otherwise nothing happens.
	rumble: (id: number, strength: number, seconds: number?) -> (),
}

declare camera: {
	-- A player's camera (each person has a view). fov is vertical, in degrees (default 60).
	-- Bots' cameras are kept but not shown.
	set: (player: number, opts: { pos: Vec?, target: Vec?, fov: number? }) -> (),
	get: (player: number?) -> { pos: Vec, target: Vec, fov: number },
}

declare sound: {
	-- Play sounds/NAME.luau once; returns a handle. Options: at (a world
	-- position; without it the sound is 2D), db (gain), pitch (rate, 1 normal);
	-- any other key sets the patch parameter of that name. A recording named
	-- NAME (or NAME.1, NAME.2 ... a different one each time) in samples.lock
	-- plays instead once fetched; it takes only `rate` and `gain`.
	play: (name: string, opts: { [string]: number | Vec }?) -> number,
	-- Change a parameter of a playing sound.
	set_param: (handle: number, param: string, value: number) -> (),
	-- Move a spatial sound along with its emitter.
	move_to: (handle: number, pos: Vec) -> (),
	-- Release a playing sound (looping sounds play until stopped).
	stop: (handle: number) -> (),
	list: () -> { string },
	-- Whether play(name) plays anything (a patch, or a fetched recording).
	has: (name: string) -> boolean,
	-- Whether play(name) plays a recording now (not the synth patch).
	recorded: (name: string) -> boolean,
}

declare hud: {
	-- Text in pixels from the top left, drawn this frame (call from draw).
	text: (text: any, x: number, y: number, opts: { size: number?, color: Color?, align: ("left" | "center" | "right")?, width: number? }?) -> (),
	-- Pad button `button` ("A", "B", "X", "Y", "LB", "RB", "Start", "Back", or "move")
	-- as player `player` (id, table or nil for the first) presses it: a coloured
	-- disc on a controller, the key's cap on a keyboard or keyboard half (two
	-- players on one keyboard). Top left at x, y; size in pixels high (default 32);
	-- alpha 0..1. Returns its width. Call from draw (or menu).
	button: (player: any, button: string, x: number, y: number, size: number?, alpha: number?) -> number,
	-- The key that presses `button` for that player ("Space", "R Shift", "WASD"
	-- for "move"), or nil on a controller.
	key: (player: any, button: string) -> string?,
	-- A filled rectangle (default colour black at half alpha).
	box: (x: number, y: number, w: number, h: number, color: Color?) -> (),
	-- A person's Splitscreen Club face (their profile art on a head in their skin
	-- colour; a plain face outside Splitscreen Club), size pixels square at x, y
	-- (top left). false for a bot or an unknown id.
	face: (player: number, x: number, y: number, size: number) -> boolean,
	-- The output size in pixels.
	size: () -> (number, number),
	-- Pixels per point of a 720-line layout: height / 720, larger on a phone
	-- or tablet (their screens are small). Scale HUD sizes and margins by it.
	unit: () -> number,
	-- One line of text in the HUD font: its width and height in pixels.
	measure: (text: string, size: number?) -> (number, number),
	-- A person's split-screen view: x, y, width, height in pixels (a bot: the whole output).
	view: (player: number?) -> (number, number, number, number),
	-- Where a world point shows in pixels (in that player's view; default
	-- the first), for labels over things. nil when behind the camera.
	project: (pos: Vec, player: number?) -> (number?, number?),
}

-- The look after lighting. Looks only (allowed in draw); restart resets it.
declare post: {
	-- bloom 0..2, bloom_radius 1..6 blur rounds, bloom_wide (true: soft
	-- screen-blended glow; false: a tight halo for neon lines),
	-- bloom_threshold (exposed brightness where it starts), contrast and
	-- saturation (1 neutral), vignette 0..1, grain 0..1, aberration (pixels
	-- of red and blue apart at the corners).
	set: (opts: { bloom: number?, bloom_radius: number?, bloom_threshold: number?, bloom_wide: boolean?, contrast: number?, saturation: number?, vignette: number?, grain: number?, aberration: number? }) -> (),
	reset: () -> (),
}

-- Coloured point lights without shadows, at most 32 (`set`), and shadowed
-- spotlights, at most 8 (`spots`). intensity is about
-- lux at 1 m (a few hundred under the night sky), range in metres.
declare lights: {
	-- Opt in with room_lighting = true. Static entities are baked when visited.
	-- Set up to four rooms. Clear a room with lights.room(id, nil) on leaving.
	room: (id: number, opts: { min: Vec, max: Vec, entities: {number}, panels: {{pos: Vec, color: Color, intensity: number, range: number, width: number, depth: number}}? }?) -> (),
	set: (list: { { pos: Vec, color: Color?, intensity: number?, range: number? } }) -> (),
	-- Spotlights that cast shadows, at most 8 (torches, headlights, a lamp
	-- over a door): dir is where the beam points, angle the outer half-angle
	-- in degrees (2 to 74, default 30), inner where it starts to fade (default
	-- 0.6 of angle), intensity in candela (default 1000, falls off with the
	-- square of distance), range in metres (default 15). Replaces the list.
	spots: (list: { { pos: Vec, dir: Vec?, color: Color?, intensity: number?, range: number?, angle: number?, inner: number? } }) -> (),
	clear: () -> (),
}

-- Warp Humans from game.toml's [humans.NAME] pools: spawn returns nil when
-- the pool is full. Set pos (feet) and yaw each tick; the legs follow.
-- pose: walk, idle, sit, crouch, wave, reach, raise, strike, aim, hold, shove, attack, punch, jab,
-- hit, hit_head, cast, roll, dead, down, feed (crouched, bent over something on the
-- ground), rise (getting up off the ground over t); t is its progress, 0 to 1.
-- The shared four-wheel vehicle simulation. Reset snapshots solid boxes once.
declare vehicles: {
	contact_profile: (name: string, bottom: number, sections: {{z:number, width:number, top:number, roofWidth:number?}}) -> (),
	reset: (static_tag: string?, exclude_ids: {number}?) -> (),
	set_static_region: (name: string, entity_ids: {number}, boxes: {{pos: vector, size: vector, yaw: number?}}) -> (),
	relocate: (id: number, feet: vector, yaw: number?) -> boolean,
	spawn: (ground_pos: Vec, yaw: number?) -> number,
	remove: (id: number) -> boolean,
	drive: (id: number, controls: { throttle: number?, brake: number?, steer: number?, handbrake: number? }?) -> (),
	step: (dt: number) -> (),
	get: (id: number) -> { pos: Vec, rot: Vec, speed: number, rpm: number, gear: number, cut: boolean,
		parked: boolean, torque: number, load: number, slip: number, contact: number, loose: number, soft: number, wheels: { Vec } }?,
}

declare humans: {
	spawn: (pool: string, opts: { pos: Vec?, yaw: number?, look: number?, resident: number? }?) -> number?,
	-- tired 0..1: bent over, out of breath, over any pose; coil -1..1: wound up for a heavy blow (1), swung through (< 0)
	-- hold: grip = right shoulder to wrist, support = right wrist to left wrist (+Z forward, metres before pitch).
	-- pitch is in radians, positive up. t blends in the head following that pitch.
	set: (id: number, opts: { pos: Vec?, yaw: number?, pose: string?, t: number?, tired: number?, coil: number?, visible: boolean?, grip: Vec?, support: Vec?, pitch: number? }) -> boolean,
	-- A blow along dir (its length the strength, ~1 heavy): the upper body jolts and sways back
	hit: (id: number, dir: Vec) -> boolean,
	ragdoll_raycast: (id: number, origin: Vec, dir: Vec, max: number) -> number?,
	ragdoll_closest: (id: number, point: Vec) -> Vec?,
	ragdoll_hit: (id: number, point: Vec, velocity: Vec) -> boolean,
	-- Release with this tick's movement (or an explicit velocity), then kick the limb nearest the world contact.
	ragdoll_impact: (id: number, point: Vec, kick: Vec, carriedVelocity: Vec?) -> boolean,
	-- Limp from the pose it stands in, thrown by push (m/s); up again with any pose but dead/down
	ragdoll: (id: number, push: Vec?, spin: Vec?) -> boolean,
	-- Replace the shared solid boxes for active ragdolls. Positions are box centers.
	obstacles: (boxes: {{id:number, pos:Vec, size:Vec, yaw:number?, prevPos:Vec?, prevYaw:number?, vehicle:number?, profile:string?}}) -> (),
	-- Once settled, start the rise pose and return landed feet. Advance rise with set(id,{pose="rise",t=...}).
	recover: (id:number) -> Vec?,
	remove: (id: number) -> boolean,
	head: (id: number) -> Vec?,
	-- head, neck, chest, hips, shoulder_l/r, elbow_l/r, hand_l/r, knee_l/r, foot_l/r
	joint: (id: number, name: string) -> Vec?,
	-- Free slots, total slots, number of fitted bodies and number of complete appearances.
	room: (pool: string, body: number?) -> (number, number, number, number),
	hide_distant: (pool: string, resident: number) -> boolean, -- hide immediately until the next crowd snapshot
	-- One cheap silhouette per array index, except IDs already in fitted. Interpolates
	-- feet and heading toward nextPos/nextYaw over seconds. Reserve [humans.NAME].distant.
	distant: (pool: string, rows: { {pos: Vec, yaw: number?, nextPos: Vec?, nextYaw: number?, active: boolean?, look: number?} }, fitted: {[number]: number}, seconds: number) -> (),
}

type GrassRect = { pos: Vec, width: number, depth: number, yaw: number? }

declare grass: {
    -- Replace the horizontal GPU grass field, or clear it with nil. Rectangles use
    -- world metres and optional yaw in radians. The top of each ground surface is pos.y.
    set: (opts: { patches: {GrassRect}, cutouts: {GrassRect}?, seed: number?,
        growth: number?, distance: number? }?) -> (),
}

declare plants: {
	-- A grown plant (a Jeffrey pine unless `species` changes its rules): its
	-- age in years sets how many shoots, limbs and needle brushes it has, not
	-- its scale. It cooks on a background thread and appears when done.
	-- `scale` multiplies the cooked plant at this placement (default 1).
	-- `species` and `look` override fields of plants.defaults() (unknown
	-- names are an error). `full` also cooks the voxel levels and proxies
	-- (LOD and far views; minutes for an old tree); without it a preview of
	-- the source meshes takes a fraction of a second. Returns the plant's id.
	grow: (opts: { age: number, seed: number?, pos: Vec?, yaw: number?, scale: number?, full: boolean?, species: { [string]: any }?, look: { [string]: any }? }) -> number,
	remove: (id: number) -> boolean,
	clear: () -> (),
	-- Plants placed, shown, still cooking with nothing to show, failed (and the
	-- last error); upgrading: full plants showing their preview until their
	-- full (voxel) cook is done.
	status: () -> { placed: number, shown: number, cooking: number, failed: number, upgrading: number, error: string? },
	-- The default growth rules and colours, with what each field is in render::plant_growth::Species.
	defaults: () -> { species: { [string]: any }, look: { [string]: any } },
	-- A JSON file NAME.json in the game folder: save a table, load it back (nil when missing).
	save: (name: string, value: any) -> (),
	load: (name: string) -> any,
}

export type CloudSettings = {
	coverage: number?, density: number?, shadow: number?,
	base: number?, thickness: number?, far: number?,
	wind_x: number?, wind_z: number?, steps: number?, light_steps: number?,
	aerial: boolean?,
}

declare sky: {
	-- Move the sun continuously. Hours wrap at midnight and weather defaults to clear.
	set_time: (hour: number, weather: string?) -> (),
	-- Set the shared native cumulus layer in init or tick. True uses fair-weather defaults.
	-- Heights and distances are metres, wind is metres/second and density is per metre.
	-- aerial=false keeps the game's surface fog. False disables the volume.
	clouds: (settings: boolean | CloudSettings) -> (),
	-- "gradient" or TIME[:WEATHER]: sunrise, morning, noon, afternoon, sunset,
	-- night; clear, haze, mist ("sunset:haze"). A preset bakes in about 0.2 s.
	set: (name: string) -> (),
	get: () -> string,
}

declare light: {
	-- Override the sky's light: sun (a direction towards the sun), sun_lux,
	-- sky_lux, exposure (EV100), fog_start, fog_end (m), fog_density (per m).
	set: (opts: { sun: Vec?, sun_lux: number?, sky_lux: number?, exposure: number?, fog_start: number?, fog_end: number?, fog_density: number? }) -> (),
	reset: () -> (),
}

-- Seeded from game.toml's seed (or --seed); restart reseeds it.
declare random: {
	float: () -> number, -- [0, 1)
	range: (lo: number, hi: number) -> number, -- [lo, hi)
	int: (lo: number, hi: number) -> number, -- lo .. hi inclusive
	chance: (p: number) -> boolean,
	seed: (seed: number) -> (),
}

declare game: {
	tick: () -> number, -- ticks since the start
	time: () -> number, -- seconds of game time (tick / tick_rate)
	dt: () -> number,
	players: () -> number, -- players.count()
	title: () -> string,
	-- A phone or tablet host (its cheaper preset): spend less (fewer shadowed lights, people). WARP_PHONE=1 on a desktop.
	phone: () -> boolean,
	-- The match is over: under Splitscreen Club the party moves on to the next game.
	finish: () -> (),
	-- A line for the room log in the Splitscreen Club app ("Joep won"), at most 120 characters.
	log: (text: any) -> (),
}

-- Per module, kept across reloads; main.luau's is the game's JSON state.
declare state: { [any]: any }


-- Versioned local JSON documents. Enable storage in game.toml and declare the
-- schema in save-schema.json (settings-schema.json or profile-schema.json for those kinds). Use revision 0 for an unused slot.
export type StorageKind = "save" | "settings" | "profile"
declare storage: {
    read: (kind: StorageKind, slot: string) -> {version: number, revision: number, data: any}?,
    write: (kind: StorageKind, slot: string, expectedRevision: number, data: any) -> number,
    array: () -> {any},
    null: any,
}
