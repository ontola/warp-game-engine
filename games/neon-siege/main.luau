-- Neon Siege: one swarm, one team, keep moving. A co-op twin-stick
-- shooter over 24 waves with a boss every fourth wave and the
-- Singularity at the end. Move with the left stick, aim and fire with the
-- right stick (no auto-aim for people), LB (or A) dashes, RB (or B) pulses. A downed pilot is
-- revived by a teammate standing close, or after six seconds.
--
-- Ported from the LÖVE party pack (love-party/games/siege*.lua): the same
-- arena units (800 high), enemies, waves, bosses, black hole, power-ups,
-- scoring and bot. The simulation runs in those 2D units; sync() shows it
-- in 3D, 40 units to the metre, seen straight down: neon outlines on a
-- glowing grid, Geometry Wars style.

-- Settings (the LÖVE pack's defaults).
local DIFFICULTY = "standard" -- relaxed, standard or intense
local START_WAVE = 1
local LIVES = 3
-- Seconds a teammate has to stay close to revive a downed pilot.
local REVIVE_TIME = 2.5
local GRAVITY = 0 -- percent; 0 turns the black hole off (Joep: the mechanic may go)

-- The look, tweakable live: the `knobs` and `set_knob` tools (or the
-- `knob NAME VALUE` command) change a value here and reload the game.
-- @knobs
local KNOBS = {
	bloom = 0.28, -- glow strength around everything that shines, 0 to 2
	bloom_radius = 2, -- how far the glow spreads, 1 (tight) to 6
	bloom_threshold = 0.9, -- how bright something must be before it glows
	line_glow = 0.04, -- brightness of enemy, pilot and pickup outlines
	shot_glow = 0.05, -- brightness of shots and enemy bullets
	spark_glow = 0.045, -- brightness of explosion sparks and rings
	grid_glow = 0.0016, -- brightness of the grid (major lines are about twice it)
	line_width = 3.2, -- outline width in arena units (the arena is 800 high)
	draw_scale = 1.3, -- how much larger things are drawn than they collide
	stars = 0, -- stars far below the arena (off until the cell backdrop shader lands)
	star_glow = 0.012, -- brightness of the stars
	star_drift = 0.6, -- how fast the star field drifts past (m/s for the nearest layer)
	shot_jitter = 0.05, -- random wobble in each shot's direction, radians either way
	cells = 0, -- big faint cells drifting under the stars (stand-in, off: the shader replaces them)
	cell_glow = 0.0025, -- brightness of the cell walls and nuclei
	vignette = 0.4, -- dark corners, 0 to 1
	grain = 0.1, -- film grain, 0 to 1
	aberration = 0.8, -- colour fringes at the corners, in pixels
	saturation = 1.25, -- colour strength, 1 neutral
	contrast = 1.1, -- 1 neutral
}
-- @end

local FINAL_WAVE = 24
local WIDTH, HEIGHT = 1422, 800 -- arena units, 16:9
local S = 1 / 40 -- metres per arena unit
local LIMITS = { enemies = 96, shots = 600, hostile = 240, pickups = 36, rings = 40 }
local AMMO = { spread = 90, pierce = 60, rapid = 300 }
-- The order the active powers sit in under a ship.
local POWER_ORDER = { "spread", "pierce", "rapid" }
local SEGMENTS = 12

local SPECS = {
	-- The first boss (wave 4): a giant virion. Half its old 650 hp (Joep, 11/24: far too tough).
	boss = { hp = 325, r = 46, speed = 50, value = 1000, boss = true, extra = 70 },
	chaser = { hp = 1, r = 11, speed = 115, value = 10 },
	weaver = { hp = 1, r = 10, speed = 145, value = 15 },
	splitter = { hp = 4, r = 21, speed = 65, value = 45 },
	fort = { hp = 4, r = 18, speed = 43, value = 55 },
	shard = { hp = 1, r = 7, speed = 190, value = 8 },
	dasher = { hp = 2, r = 12, speed = 70, value = 25 },
	orbiter = { hp = 2, r = 13, speed = 150, value = 30 },
	hive = { hp = 650, r = 46, speed = 32, value = 1200, boss = true, extra = 140 },
	serpent = { hp = 800, r = 26, speed = 170, value = 1400, boss = true, extra = 160 },
	segment = { hp = 1, r = 17, speed = 0, value = 5, armor = true },
	prism = { hp = 340, r = 30, speed = 0, value = 1600, boss = true, extra = 75 },
	sun = { hp = 1000, r = 56, speed = 28, value = 2400, boss = true, extra = 200 },
	singularity = { hp = 520, r = 60, speed = 0, value = 6000, boss = true, final = true },
	node = { hp = 1, r = 15, speed = 0, value = 0, armor = true },
	-- The final boss: a tumour that keeps dividing; while its offspring live it
	-- cannot be hurt. Metastases are those offspring.
	tumor = { hp = 640, r = 56, speed = 22, value = 6000, boss = true, final = true, extra = 120 },
	metastasis = { hp = 5, r = 16, speed = 85, value = 60 },
}

local COLORS = {
	boss = { 0.85, 0.35, 1 },
	chaser = { 1, 0.27, 0.57 },
	weaver = { 0.4, 1, 0.5 },
	splitter = { 1, 0.76, 0.23 },
	fort = { 1, 0.42, 0.19 },
	shard = { 1, 0.9, 0.5 },
	dasher = { 1, 0.2, 0.25 },
	orbiter = { 0.35, 0.75, 1 },
	hive = { 1, 0.85, 0.15 },
	serpent = { 0.25, 1, 0.75 },
	segment = { 0.2, 0.75, 0.6 },
	prism = { 0.75, 0.95, 1 },
	sun = { 1, 0.62, 0.18 },
	singularity = { 0.78, 0.4, 1 },
	node = { 0.55, 0.3, 0.95 },
	tumor = { 0.95, 0.3, 0.45 },
	metastasis = { 1, 0.5, 0.6 },
}

-- How each kind is drawn: neon outlines seen from above, as the LÖVE
-- screen draws them. Each part is a polygon with n sides at a share k of
-- the kind's radius, turning with the kind (spin -1 turns the other way).
-- Hollow parts have their own line width; `solid` parts are filled.
local SHAPES = {
	boss = { { n = 8 }, { n = 8, k = 0.65, spin = -1 } },
	-- The swarm is a germ zoo: round spiky viruses, a rod-shaped bacterium
	-- and a phage (see SPIKES for their spikes, flagella and legs).
	chaser = { { n = 16 } },
	weaver = { { n = 20, long = 1.7 } },
	splitter = { { n = 6 }, { n = 6, k = 0.45, spin = -1 } },
	fort = { { n = 4 }, { n = 16, k = 0.33, still = true } },
	shard = { { n = 12 } },
	dasher = { { n = 6 } },
	orbiter = { { n = 20 }, { n = 24, k = 0.45, still = true } },
	hive = { { n = 6 }, { n = 6, k = 0.65, spin = -1 } },
	serpent = { { n = 7 }, { n = 24, k = 0.4, still = true } },
	segment = { { n = 6 } },
	prism = { { n = 4, k = 1.25 }, { n = 4, k = 0.8, spin = -1.5 }, { n = 4, k = 0.45, spin = 2, white = true } },
	sun = { { n = 32, still = true }, { n = 32, k = 0.62, still = true, white = true }, { n = 12, k = 1.3, spin = 0.6 }, { n = 12, k = 1.5, spin = -0.4, dim = true } },
	singularity = { { n = 32, k = 1.04, still = true, white = true }, { n = 32, k = 1.18, still = true } },
	node = { { n = 3 } },
	-- Lumpy, turning membranes around a white nucleus.
	tumor = { { n = 11 }, { n = 9, k = 0.78, spin = -0.6 }, { n = 7, k = 0.38, spin = 1.3, white = true } },
	metastasis = { { n = 9 }, { n = 6, k = 0.4, spin = -1, white = true } },
}

-- Lines that stick out of a body, turning with it: a = angle from the
-- body's heading, from = where it starts (in body radii), len = its length
-- (in body radii), knob = a small ball at the tip.
local function around(count, len, knob)
	local list = {}
	for i = 1, count do table.insert(list, { a = i * 2 * math.pi / count, len = len, knob = knob }) end
	return list
end
local SPIKES = {
	chaser = around(8, 0.45, true),
	shard = around(6, 0.55, true),
	orbiter = around(10, 0.35, true),
	tumor = around(9, 0.1, true),
	metastasis = around(5, 0.18, true),
	-- The phage: a tail behind its head and three legs splayed at its end.
	dasher = { { a = math.pi, len = 1.1 }, { a = math.pi - 0.55, from = 1.9, len = 0.7 }, { a = math.pi, from = 2.1, len = 0.5 }, { a = math.pi + 0.55, from = 1.9, len = 0.7 } },
}

-- Flagella: wavy tails that whip as a germ swims. a and from as for spikes;
-- heading = trail behind the way it swims (towards the nearest pilot)
-- instead of turning with the body.
local FLAGELLA = {
	weaver = { { a = math.pi - 0.15, from = 1.6 }, { a = math.pi + 0.15, from = 1.6 } },
	chaser = { { a = math.pi, from = 1.15, heading = true } },
}
local FLAG_SEGMENTS, FLAG_LENGTH = 8, 1.7 -- pieces per tail, length in body radii
-- The line every outline is drawn with, in arena units (the LÖVE 2 px line
-- plus its glow).
local LINE = KNOBS.line_width
-- Everything is drawn a little larger than it collides, so it reads on a TV.
local DRAW = KNOBS.draw_scale
-- The camera looks straight down; a narrow lens keeps the view nearly flat.
local CAMERA_FOV = 28
local CAMERA_HEIGHT = 10.6 / math.tan(math.rad(CAMERA_FOV / 2))
-- Grid colours per boss stage (each stage's nebula in the LÖVE sky).
local STAGE_GRID = { { 0.3, 0.32, 1 }, { 0.95, 0.45, 0.3 }, { 1, 0.55, 0.15 }, { 0.2, 0.85, 0.75 }, { 1, 0.3, 0.7 }, { 0.62, 0.3, 1 } }

local PICKUP_COLORS = { repair = { 1, 0.3, 0.42 }, spread = { 1, 0.75, 0.2 }, pierce = { 1, 0.45, 0.85 }, rapid = { 0.25, 1, 1 } }
-- What a pickup does, drawn inside its white ring as line segments
-- {x1, y1, x2, y2} in arena units (y grows downwards): repair is a heart,
-- spread a fan of three shots, pierce an arrow through a wall, rapid a
-- double chevron.
local PICKUP_ICONS = {
	repair = {
		{ 0, 7, -8, -1 }, { -8, -1, -8, -4 }, { -8, -4, -6, -7 }, { -6, -7, -3, -7 }, { -3, -7, 0, -4 },
		{ 0, -4, 3, -7 }, { 3, -7, 6, -7 }, { 6, -7, 8, -4 }, { 8, -4, 8, -1 }, { 8, -1, 0, 7 },
	},
	spread = { { 0, 7, -7, -6 }, { 0, 7, 0, -8 }, { 0, 7, 7, -6 } },
	pierce = { { -9, 0, 9, 0 }, { 9, 0, 4, -4 }, { 9, 0, 4, 4 }, { -1, -7, -1, 7 } },
	rapid = { { -7, -6, -1, 0 }, { -1, 0, -7, 6 }, { 0, -6, 6, 0 }, { 6, 0, 0, 6 } },
}
local SHOT_COLORS ={ pierce = { 1, 0.45, 0.85 }, spread = { 1, 0.75, 0.2 }, rapid = { 0.25, 1, 1 } }
local HOSTILE_COLOR = { 1, 0.62, 0.3 }
local WHITE = { 1, 1, 1 }

local WAVE_NAMES = { "SWEEP", "PINCER", "CORNERS", "CROSSWIND" }
local BOSSES = {
	{ kind = "boss", name = "THE VIRION" },
	{ kind = "hive", name = "HIVE QUEEN" },
	{ kind = "serpent", name = "SERPENT" },
	{ kind = "prism", name = "PRISM TWINS" },
	{ kind = "sun", name = "SOLAR WARDEN" },
	{ kind = "tumor", name = "THE TUMOR" },
}
local WAVE_REST, WAVE_WARNING = 2, 1.4
-- Beam timings in seconds: off, warn, live.
local SUN_CYCLE = { 2.2, 1.2, 2.6 }
local CORE_CYCLE = { 2.6, 1.2, 2.2 }
local TWIN_CYCLE = { 1.8, 1.1, 2.1 }
local HOLE_WAVE = 7

local s -- the simulation: state.s
local byid = {} -- enemies by uid, rebuilt from s.enemies

local function len(x, y)
	return math.sqrt(x * x + y * y)
end

local function at(x, y, h)
	return vector.create((x - s.width / 2) * S, h, (y - s.height / 2) * S)
end

-- Neon: deepen a colour (1 stays 1, the rest drops), so it stays a colour
-- under the glow instead of fading to pastel.
local function rgb(c, k)
	k = k or 1
	return vector.create(c[1] ^ 1.4 * k, c[2] ^ 1.4 * k, c[3] ^ 1.4 * k)
end

local function neon(v)
	return vector.create(v.x ^ 1.4, v.y ^ 1.4, v.z ^ 1.4)
end

local function enemy(uid)
	local e = uid and byid[uid]
	if e and not e.dead then return e end
	return nil
end

local function alive(p)
	return p.hp > 0
end

local function nearest(x, y)
	local best, distance = nil, math.huge
	for _, p in s.players do
		if alive(p) then
			local d = (p.x - x) ^ 2 + (p.y - y) ^ 2
			if d < distance then best, distance = p, d end
		end
	end
	return best, distance
end

local function sfx(name)
	s.sfx[name] = (s.sfx[name] or 0) + 1
end

local function tough(e)
	return SPECS[e.kind].boss or SPECS[e.kind].armor
end

-- The tumour cannot be hurt while any of its metastases is alive.
local function shielded(e)
	if e.kind ~= "tumor" then return false end
	for _, other in s.enemies do
		if other.kind == "metastasis" and not other.dead then return true end
	end
	return false
end

local function multiplier()
	return math.min(5, 1 + math.floor(s.chain / 12))
end

local FX_GLOW = KNOBS.spark_glow

-- Effects are the engine's: a ring grows out, a burst of streaks flies off.
local function ring(x, y, r, color)
	if s.rings >= LIMITS.rings then return end
	s.rings += 1
	-- A short flash of light on the deck, the bigger the ring the brighter.
	if #s.flashes < 12 then table.insert(s.flashes, { x = x, y = y, color = color, power = math.min(3, r / 60), ttl = 0.3 }) end
	fx.ring(at(x, y, 0.15), { color = rgb(color), radius = r * S, life = 0.35, width = 0.05, emissive = FX_GLOW, segments = if r > 100 then 32 else 16 })
end

local function burst(x, y, color, count)
	-- Geometry Wars sparks: many thin fast streaks.
	fx.burst(at(x, y, 0.2), { color = rgb(color), count = math.ceil(count * 1.5), speed = 10, life = 0.7, emissive = FX_GLOW, size = 0.045, stretch = 0.035, flat = true })
end

-- The black hole: late, every other wave from HOLE_WAVE, and it kills.
local function hole_wave()
	return s.wave >= HOLE_WAVE and (s.wave - HOLE_WAVE) % 2 == 0
end

local function fields()
	if GRAVITY == 0 or not hole_wave() then return {} end
	return { { x = s.width * 0.73, y = s.height * 0.67, mass = 6500000 * GRAVITY / 100, r = 24, lethal = true } }
end

local function ramp()
	return math.clamp((s.waveClock - 1) / 3, 0, 1)
end

local function strength()
	return math.clamp((s.time - 3) / 3, 0, 1)
end

local function force(x, y)
	local ax, ay = 0, 0
	for _, f in fields() do
		local dx, dy = f.x - x, f.y - y
		local d2 = dx * dx + dy * dy + 65 * 65
		local a = f.mass / (d2 * math.sqrt(d2)) * strength() * ramp()
		ax, ay = ax + dx * a, ay + dy * a
	end
	return ax, ay
end

----------------------------------------------------------------------------
-- Spawning, killing, hurting

local spawn -- forward

local function shield(e, delay)
	for _, uid in e.nodes do
		local n = enemy(uid)
		if n then n.dead = true end
	end
	e.nodes = {}
	for i = 1, 8 do
		local node = spawn("node", e.x, e.y, delay or 0.3)
		if not node then break end
		node.head, node.slot = e.uid, i
		table.insert(e.nodes, node.uid)
	end
end

local function boss_spawned(e, delay)
	e.beams, e.clock, e.armAngle = {}, 0, 0
	if e.kind == "prism" and not s.spawningTwin then
		s.spawningTwin = true
		local twin = spawn("prism", s.width - e.x, e.y, delay)
		s.spawningTwin = nil
		if twin then
			e.twin, twin.twin, twin.follower = twin.uid, e.uid, true
		end
	elseif e.kind == "singularity" then
		e.phase, e.nodes = 1, {}
		shield(e, delay)
	end
end

function spawn(kind, x, y, delay)
	if #s.enemies >= LIMITS.enemies then return nil end
	local def = SPECS[kind]
	s.uid += 1
	local e = {
		uid = s.uid,
		kind = kind,
		x = x,
		y = y,
		hp = def.hp,
		r = def.r,
		phase = random.float() * 6.28,
		warm = delay or 0.6,
		fire = 1.0 + random.float(),
		flash = 0,
	}
	if def.boss then e.hp = def.hp + math.max(0, #s.players - 1) * (def.extra or 60) end
	e.maxHp = e.hp
	table.insert(s.enemies, e)
	byid[e.uid] = e
	if kind == "prism" or kind == "sun" or kind == "singularity" or kind == "node" then boss_spawned(e, delay) end
	if kind == "serpent" then
		-- The body trails off-screen behind the head and follows it link by link.
		e.segments = {}
		local lead = e.uid
		for i = 1, SEGMENTS do
			local segment = spawn("segment", x, y - i * 26, delay)
			if not segment then break end
			segment.lead, segment.head = lead, e.uid
			table.insert(e.segments, segment.uid)
			lead = segment.uid
		end
	end
	return e
end

local kill -- forward

local function boss_killed(e)
	e.beams = {}
	local twin = enemy(e.twin)
	if e.kind == "prism" and twin then
		ring(twin.x, twin.y, 120, COLORS.prism)
	elseif e.kind == "singularity" or e.kind == "tumor" then
		s.won = true
		s.victoryClock = 0
		ring(e.x, e.y, 400, WHITE)
		ring(e.x, e.y, 260, COLORS.singularity)
		s.hostile = {}
		for _, other in s.enemies do
			if other ~= e and not other.dead then kill(other) end
		end
	end
end

function kill(e)
	if e.dead then return end
	e.dead = true
	s.kills += 1
	s.chain += 1
	s.comboTime = 2.5
	s.teamScore += SPECS[e.kind].value * multiplier()
	local color = COLORS[e.kind]
	burst(e.x, e.y, color, if e.kind == "splitter" then 22 else 10)
	ring(e.x, e.y, e.r * 2, color)
	s.shake = math.min(5, s.shake + 0.7)
	sfx("burst")
	if e.kind == "segment" then
		-- The link behind a swallowed one follows the one ahead.
		for _, other in s.enemies do
			if other.lead == e.uid then other.lead = e.lead end
		end
	end
	if e.segments then
		for _, uid in e.segments do
			local seg = enemy(uid)
			if seg then kill(seg) end
		end
	end
	if e.kind == "prism" or e.kind == "sun" or e.kind == "singularity" or e.kind == "tumor" then boss_killed(e) end
	if SPECS[e.kind].boss then
		s.shake = 5
		ring(e.x, e.y, 160, color)
		burst(e.x, e.y, color, 60)
	end
	if e.kind == "splitter" then
		for i = 1, 3 do
			local a = i * 2.094
			spawn("shard", e.x + math.cos(a) * 18, e.y + math.sin(a) * 18, 0.3)
		end
	end
	-- A boss's escorts carry power-ups more often: the way to beat it.
	if #s.pickups < LIMITS.pickups and random.float() < (if e.escort then 0.35 else 0.09) then
		local kinds = { "spread", "pierce", "rapid", "repair" }
		table.insert(s.pickups, { x = e.x, y = e.y, kind = kinds[random.int(1, 4)], ttl = 12 })
	end
end

local function hurt(p)
	if not alive(p) or p.invul > 0 or p.dash > 0 then return false end
	p.hp -= 1
	p.invul = 1.0
	s.chain = 0
	s.comboTime = 0
	s.shake = 5
	sfx("hurt")
	burst(p.x, p.y, { 0.75, 0.85, 1 }, 20)
	if not p.bot then players.rumble(p.id, 0.6, 0.25) end
	if p.hp == 0 then
		sfx("death")
		p.downTime = 0
		p.revive = 0
		ring(p.x, p.y, 70, { 1, 0.25, 0.3 })
		-- The ship blows apart: shards of hull in its colour, white sparks,
		-- and a shockwave that sweeps across the whole arena so a teammate
		-- sees it too.
		local pos = at(p.x, p.y, 0.2)
		fx.burst(pos, { color = rgb(p.rgb), count = 36, speed = 7, life = 1.3, emissive = FX_GLOW, size = 0.08, stretch = 0.06, flat = true })
		fx.burst(pos, { color = rgb(WHITE), count = 24, speed = 12, life = 0.6, emissive = FX_GLOW, size = 0.04, stretch = 0.04, flat = true })
		fx.ring(at(p.x, p.y, 0.15), { color = rgb(p.rgb), radius = 30, life = 0.9, width = 0.12, emissive = FX_GLOW, segments = 48 })
		fx.ring(at(p.x, p.y, 0.15), { color = rgb(WHITE), radius = 6, life = 0.4, width = 0.08, emissive = FX_GLOW, segments = 32 })
		s.shake = 14
		if not p.bot then players.rumble(p.id, 1, 0.5) end
	end
	return true
end

local function pulse(p)
	if not alive(p) or p.pulseClock > 0 then return false end
	p.pulseClock = 10
	p.invul = math.max(p.invul, 0.5)
	ring(p.x, p.y, 190, { 0.4, 0.85, 1 })
	s.shake = 5
	sfx("pulse")
	if not p.bot then players.rumble(p.id, 0.4, 0.2) end
	local count = #s.enemies
	for i = 1, count do
		local e = s.enemies[i]
		if not e.dead and (e.x - p.x) ^ 2 + (e.y - p.y) ^ 2 < 190 ^ 2 then
			if SPECS[e.kind].armor then
				e.flash = 0.2
			elseif tough(e) then
				e.hp -= 12
				e.flash = 0.2
				if e.hp <= 0 then kill(e) end
			else
				kill(e)
			end
		end
	end
	for i = #s.hostile, 1, -1 do
		local b = s.hostile[i]
		if (b.x - p.x) ^ 2 + (b.y - p.y) ^ 2 < 220 ^ 2 then table.remove(s.hostile, i) end
	end
	return true
end

-- Earliest intersection along the segment: even very fast bullets cannot tunnel.
local function segment_hit(ax, ay, bx, by, cx, cy, r)
	local dx, dy = bx - ax, by - ay
	local fx_, fy = ax - cx, ay - cy
	local c = fx_ * fx_ + fy * fy - r * r
	if c <= 0 then return 0 end
	local a = dx * dx + dy * dy
	if a < 1e-12 then return nil end
	local b = 2 * (fx_ * dx + fy * dy)
	local discriminant = b * b - 4 * a * c
	if discriminant < 0 then return nil end
	local t = (-b - math.sqrt(discriminant)) / (2 * a)
	if t >= 0 and t <= 1 then return t end
	return nil
end

----------------------------------------------------------------------------
-- Waves

local function plan(number, count_players)
	local pressure = ({ relaxed = 0.7, standard = 1, intense = 1.35 })[DIFFICULTY] or 1
	local count = math.min(60, math.floor((14 + (number - 1) * 4 + math.max(0, count_players - 2) * 4) * pressure))
	-- Boss waves bring a lighter escort so the fight stays readable.
	if number % 4 == 0 then count = math.floor(count * (if number >= 16 then 0.35 else 0.6)) end
	local pattern = (number - 1) % 4 + 1
	local rotation = math.floor((number - 1) / 4) % 4
	local groups = if pattern == 1 then 2 else 3
	local width, height = s.width, s.height
	local out = {}
	for i = 1, count do
		local group = (i - 1) % groups
		local lane = math.floor((i - 1) / groups)
		local lanes = math.ceil((count - group) / groups)
		local t = (lane + 1) / (lanes + 1)
		local side
		if pattern == 1 then
			side = rotation
		elseif pattern == 2 then
			side = (rotation + (group % 2) * 2) % 4
		elseif pattern == 3 then
			side = (rotation + group) % 4
			t = 0.12 + t * 0.25
		else
			side = (rotation + group) % 4
		end
		local x, y
		if side == 0 then
			x, y = 30, 60 + t * (height - 120)
		elseif side == 1 then
			x, y = 60 + t * (width - 120), 30
		elseif side == 2 then
			x, y = width - 30, 60 + t * (height - 120)
		else
			x, y = 60 + t * (width - 120), height - 30
		end
		local kind = "chaser"
		if number >= 6 and i % 11 == 0 then
			kind = "orbiter"
		elseif number >= 5 and i % 9 == 0 then
			kind = "dasher"
		elseif number >= 4 and i % 8 == 0 then
			kind = "fort"
		elseif number >= 3 and i % 7 == 0 then
			kind = "splitter"
		elseif number >= 2 and i % 3 == 0 then
			kind = "weaver"
		end
		table.insert(out, { x = x, y = y, kind = kind, at = group * 2.2 })
	end
	-- Every fourth wave a boss joins the formation.
	if number % 4 == 0 then
		local boss = BOSSES[math.min(#BOSSES, number / 4)]
		local middle = boss.kind == "singularity" or boss.kind == "prism" or boss.kind == "tumor"
		table.insert(out, { x = if boss.kind == "prism" then width * 0.2 else width / 2, y = if middle then height / 2 else 50, kind = boss.kind, at = 0 })
		return out, boss.name, true
	end
	return out, WAVE_NAMES[pattern], false
end

local function waves(dt)
	if s.won then return end
	if s.wavePhase == "rest" then
		s.waveRest = math.max(0, s.waveRest - dt)
		if s.waveRest == 0 then
			s.wave += 1
			s.waveAt = s.time -- the HUD flies the new number into its corner
			s.waveClock = 0
			s.wavePhase = "attack"
			local boss
			s.waveQueue, s.waveName, boss = plan(s.wave, #s.players)
			sfx(if boss then "boss" else "wave")
		end
		return
	end
	s.waveClock += dt
	for i = #s.waveQueue, 1, -1 do
		local entry = s.waveQueue[i]
		if entry.at <= s.waveClock then
			spawn(entry.kind, entry.x, entry.y, WAVE_WARNING)
			table.remove(s.waveQueue, i)
		end
	end
	if #s.waveQueue == 0 and #s.enemies == 0 then
		s.wavePhase = "rest"
		s.waveRest = WAVE_REST
		-- Stray turret bullets must not spoil the earned breathing room.
		s.hostile = {}
		-- Clearing a wave brings every downed pilot back.
		for _, p in s.players do
			if p.hp <= 0 then
				p.hp, p.invul, p.revive, p.downTime = s.lives, 2, 0, 0
				sfx("pickup")
				ring(p.x, p.y, 55, { 0.3, 1, 0.8 })
			end
		end
	end
end

----------------------------------------------------------------------------
-- Pilots

local function shoot(p, dx, dy)
	local angles = if p.powers.spread then { -0.30, -0.15, 0, 0.15, 0.30 } else { 0 }
	local special = if p.powers.pierce then "pierce" elseif p.powers.spread then "spread" elseif p.powers.rapid then "rapid" else nil
	for _, a in angles do
		if #s.shots >= LIMITS.shots then break end
		a += random.range(-KNOBS.shot_jitter, KNOBS.shot_jitter)
		local x, y = dx * math.cos(a) - dy * math.sin(a), dx * math.sin(a) + dy * math.cos(a)
		table.insert(s.shots, {
			x = p.x + x * 15,
			y = p.y + y * 15,
			vx = x * 860,
			vy = y * 860,
			ttl = 1.35,
			owner = p.id,
			hits = if p.powers.pierce then 5 else 1,
			damage = if p.powers.pierce then 2 else 1,
			special = special,
			hit = {},
		})
	end
	for _, kind in { "spread", "pierce", "rapid" } do
		local left = p.powers[kind]
		if left then p.powers[kind] = if left - 1 > 0 then left - 1 else nil end
	end
	sfx("shot")
end

local function pilot_step(p, c, dt)
	p.invul = math.max(0, p.invul - dt)
	p.dashClock = math.max(0, p.dashClock - dt)
	p.pulseClock = math.max(0, p.pulseClock - dt)
	if not alive(p) then
		p.downTime += dt
		local helper = nearest(p.x, p.y)
		-- A teammate within about two ship lengths revives (65 was too tight to notice).
		if helper and (helper.x - p.x) ^ 2 + (helper.y - p.y) ^ 2 < 120 ^ 2 then
			p.revive += dt
		else
			-- Leave and the progress drains fast: you have to stay to revive.
			p.revive = math.max(0, p.revive - dt * 2)
		end
		-- No revive by waiting: only a teammate (or clearing the wave) brings you back.
		if helper and p.revive >= REVIVE_TIME then
			p.hp = s.lives
			p.invul = 2
			p.revive = 0
			p.downTime = 0
			sfx("pickup")
			ring(p.x, p.y, 55, { 0.3, 1, 0.8 })
		end
		return
	end
	local dx, dy = c.x, c.y
	local length = len(dx, dy)
	if length > 1 then dx, dy = dx / length, dy / length end
	if c.action and p.dashClock == 0 and length > 0.1 then
		p.dash = 0.16
		p.dashClock = 1.2
		p.dashX, p.dashY = dx, dy
		ring(p.x, p.y, 32, { 0.3, 0.8, 1 })
	end
	p.dash = math.max(0, p.dash - dt)
	local speed = 310
	if p.dash > 0 then
		dx, dy = p.dashX, p.dashY
		speed = 920
	end
	p.x = math.clamp(p.x + dx * speed * dt, 18, s.width - 18)
	p.y = math.clamp(p.y + dy * speed * dt, 18, s.height - 18)
	if c.secondary then pulse(p) end
	local ax, ay = c.aimX or 0, c.aimY or 0
	local aim = len(ax, ay)
	p.fireClock = math.max(0, p.fireClock - dt)
	if aim > 0.2 then
		ax, ay = ax / aim, ay / aim
	elseif c.autoAim then
		local best, d = nil, math.huge
		for _, e in s.enemies do
			if not e.dead and e.warm <= 0 then
				local q = (e.x - p.x) ^ 2 + (e.y - p.y) ^ 2
				if q < d then best, d = e, q end
			end
		end
		if not best then return end -- Auto aim must not keep firing into an empty arena.
		ax, ay = best.x - p.x, best.y - p.y
		local n = math.max(0.001, len(ax, ay))
		ax, ay = ax / n, ay / n
	else
		return
	end
	p.aimX, p.aimY = ax, ay
	if p.fireClock <= 0 then
		shoot(p, ax, ay)
		p.fireClock = if p.powers.rapid then 0.045 else 0.095
	end
end

-- LÖVE's bot: orbit the middle, shy away from bullets, the hole and the
-- swarm; dash and pulse when crowded; auto-aim.
local function bot(p)
	local angle = s.time * 0.45 + p.slot * 1.7
	local dx = (s.width / 2 + math.cos(angle) * s.width * 0.25 - p.x) * 0.008
	local dy = (s.height / 2 + math.sin(angle) * s.height * 0.25 - p.y) * 0.008
	for _, b in s.hostile do
		local x, y = p.x - b.x, p.y - b.y
		local d = x * x + y * y
		if d < 110 ^ 2 then
			dx += x / math.max(100, d) * 150
			dy += y / math.max(100, d) * 150
		end
	end
	for _, f in fields() do
		local x, y = p.x - f.x, p.y - f.y
		local d = x * x + y * y
		if d < 180 ^ 2 then
			dx += x / math.max(100, d) * 320
			dy += y / math.max(100, d) * 320
		end
	end
	local nearby = 0
	for _, e in s.enemies do
		if not e.dead then
			local x, y = p.x - e.x, p.y - e.y
			local d = x * x + y * y
			if d < 180 ^ 2 then
				dx += x / math.max(100, d) * 110
				dy += y / math.max(100, d) * 110
				nearby += 1
			end
		end
	end
	local n = math.max(1, len(dx, dy))
	return { x = dx / n, y = dy / n, action = nearby > 5, secondary = nearby > 8, autoAim = true }
end

-- A person's controls in arena terms (y grows down the screen).
local function controls(i)
	if not i then return { x = 0, y = 0 } end
	return {
		x = i.move_x,
		y = -i.move_y,
		aimX = i.aim_x,
		aimY = -i.aim_y,
		-- LB dashes, RB pulses (Joep, 11/24); A and B stay as a second way.
		action = i.held.left_shoulder or i.held.south,
		secondary = i.held.right_shoulder or i.held.east,
		-- No auto-aim for people: RT aiming for you was cheating (Joep, 11/24). Bots keep it.
	}
end

----------------------------------------------------------------------------
-- Enemies

local function hostile_shot(x, y, a, speed, ttl, size)
	if #s.hostile < LIMITS.hostile then
		table.insert(s.hostile, { x = x, y = y, vx = math.cos(a) * speed, vy = math.sin(a) * speed, ttl = ttl or 7, size = size })
	end
end

-- During a boss fight, small packs of escorts drift in from the edges now and
-- then (never more than six alive); they drop power-ups more often.
local ESCORT_KINDS = { "chaser", "chaser", "weaver", "dasher" }
local function boss_escorts(e, dt)
	if e.kind == "singularity" or e.kind == "tumor" or e.follower then return end
	e.escortT = (e.escortT or 6) - dt
	if e.escortT > 0 then return end
	e.escortT = if e.hp < e.maxHp / 2 then 7 else 9
	local alive_escorts = 0
	for _, o in s.enemies do
		if not o.dead and o.escort then alive_escorts += 1 end
	end
	for i = 1, math.min(3, 6 - alive_escorts) do
		local a = random.float() * math.pi * 2
		local o = spawn(ESCORT_KINDS[random.int(1, #ESCORT_KINDS)], s.width / 2 + math.cos(a) * s.width * 0.47, s.height / 2 + math.sin(a) * s.height * 0.45, 1.0)
		if o then o.escort = true end
	end
end

-- `size` (default 1) scales a bullet's look and how wide it hits.
local function fire_at(e, dx, dy, count, spread, speed, size)
	for i = 1, count do
		if #s.hostile >= LIMITS.hostile then return end
		local a = (i - (count + 1) / 2) * spread
		local x, y = dx * math.cos(a) - dy * math.sin(a), dx * math.sin(a) + dy * math.cos(a)
		table.insert(s.hostile, { x = e.x + x * e.r, y = e.y + y * e.r, vx = x * speed, vy = y * speed, ttl = 6, size = size })
	end
end

local function cycle(clock, timing)
	local t = clock % (timing[1] + timing[2] + timing[3])
	if t < timing[1] then return nil end
	return if t < timing[1] + timing[2] then "warn" else "live"
end

local function line_distance(px, py, x1, y1, x2, y2)
	local dx, dy = x2 - x1, y2 - y1
	local t = math.clamp(((px - x1) * dx + (py - y1) * dy) / math.max(1, dx * dx + dy * dy), 0, 1)
	return len(px - x1 - dx * t, py - y1 - dy * t)
end

local function burn_beams(e)
	for _, beam in e.beams do
		if beam.state == "live" then
			for _, p in s.players do
				if p.hp > 0 and line_distance(p.x, p.y, beam[1], beam[2], beam[3], beam[4]) < 9 + 11 then hurt(p) end
			end
		end
	end
end

-- Arms radiate from the centre and keep turning while they burn.
local function arms(e, count, state_, length)
	e.beams = {}
	if not state_ then return end
	for i = 1, count do
		local a = e.armAngle + i * math.pi * 2 / count
		e.beams[i] = { e.x + math.cos(a) * e.r, e.y + math.sin(a) * e.r, e.x + math.cos(a) * length, e.y + math.sin(a) * length, state = state_ }
	end
end

-- Shield nodes ride a ring around the core and die with it.
local function node_step(e)
	local h = enemy(e.head)
	if not h then
		kill(e)
		return
	end
	local a = (h.spin or 0) + e.slot * math.pi / 4
	local radius = if h.phase == 3 then 150 else 125
	e.x, e.y = h.x + math.cos(a) * radius, h.y + math.sin(a) * radius
end

-- The three late bosses; every beam warns with a thin line before it burns.
local function late_boss(e, dx, dy, dt)
	e.clock += dt
	local enraged = e.hp < e.maxHp / 2
	if e.kind == "prism" then
		local twin = enemy(e.twin)
		local lead = if e.follower and twin then twin else e
		if lead == e then e.orbit = (e.orbit or 0) + dt * (if twin then 0.32 else 0.6) end
		local cx, cy = s.width / 2, s.height / 2
		local radius = math.min(s.width, s.height) * 0.36 + math.sin(s.time * 0.6) * 60
		local a = (lead.orbit or 0) + (if lead == e then 0 else math.pi)
		local tx, ty = cx + math.cos(a) * radius * 1.35, cy + math.sin(a) * radius
		e.x, e.y = e.x + (tx - e.x) * math.min(1, dt * 2), e.y + (ty - e.y) * math.min(1, dt * 2)
		e.fire -= dt
		if twin then
			-- The leader owns the beam strung between the twins.
			if lead == e then
				local st = cycle(e.clock, TWIN_CYCLE)
				e.beams = if st then { { e.x, e.y, twin.x, twin.y, state = st } } else {}
				burn_beams(e)
			end
			if e.fire <= 0 then
				-- An aimed fan of big orbs.
				e.fire = if enraged then 1.6 else 2.2
				for i = -2, 2 do hostile_shot(e.x, e.y, math.atan2(dy, dx) + i * 0.2, 220, nil, 1.6) end
			end
			-- Prism nova: the twins take turns blasting a turning ring of
			-- orbs; slip through the gaps.
			e.nova = (e.nova or (if e.follower then 2.4 else 4.8)) - dt
			if e.nova <= 0 then
				e.nova = if enraged then 3.4 else 4.8
				e.novaTurn = (e.novaTurn or 0) + 0.13
				ring(e.x, e.y, 70, COLORS.prism)
				for i = 0, 13 do hostile_shot(e.x, e.y, e.novaTurn + i * math.pi * 2 / 14, 150, nil, 1.4) end
			end
		else
			-- Alone, the survivor spins a dense spiral (ruthless enough, Joep).
			e.beams = {}
			if e.fire <= 0 then
				e.fire = 0.14
				e.armAngle += 0.5
				for i = 0, 2 do hostile_shot(e.x, e.y, e.armAngle + i * math.pi * 2 / 3, 170) end
			end
		end
	elseif e.kind == "sun" then
		local cx, cy = s.width / 2 + math.sin(s.time * 0.3) * 180, s.height / 2 + math.cos(s.time * 0.4) * 90
		e.x, e.y = e.x + (cx - e.x) * dt * 0.6, e.y + (cy - e.y) * dt * 0.6
		e.armAngle += dt * (if enraged then 0.55 else 0.38)
		arms(e, if enraged then 4 else 3, cycle(e.clock, SUN_CYCLE), 1600)
		burn_beams(e)
		e.fire -= dt
		if e.fire <= 0 then
			-- Solar flare: a ring with a wide gap to slip through.
			e.fire = if enraged then 2.2 else 3
			local gap = random.float() * math.pi * 2
			for i = 3, 23 do hostile_shot(e.x, e.y, gap + i * math.pi / 12, 165) end
		end
	elseif e.kind == "singularity" then
		local phase = if e.hp > e.maxHp * 2 / 3 then 1 elseif e.hp > e.maxHp / 3 then 2 else 3
		if phase ~= e.phase then
			e.phase = phase
			ring(e.x, e.y, 260, COLORS.singularity)
			ring(e.x, e.y, 180, WHITE)
			s.shake = 5
			e.clock, e.summon = 0, 3
			shield(e, 0.8)
		end
		e.x, e.y = e.x + (s.width / 2 - e.x) * dt, e.y + (s.height / 2 - e.y) * dt
		e.spin = (e.spin or 0) + dt * (if phase == 3 then 1.1 else 0.7)
		-- The event horizon drags everyone in; a dash breaks free.
		local pull = ({ 55, 85, 125 })[phase]
		for _, p in s.players do
			if p.hp > 0 and p.dash == 0 then
				local x, y = e.x - p.x, e.y - p.y
				local d = math.max(1, len(x, y))
				p.x, p.y = p.x + x / d * pull * dt, p.y + y / d * pull * dt
			end
		end
		e.fire -= dt
		if e.fire <= 0 then
			e.fire = if phase == 1 then 0.16 else 0.13
			e.armAngle += 0.29
			local count = if phase == 3 then 3 else 2
			for i = 0, count - 1 do hostile_shot(e.x, e.y, e.armAngle + i * math.pi * 2 / count, if phase == 3 then 175 else 150) end
		end
		if phase >= 2 then
			e.beamAngle = (e.beamAngle or 0) - dt * (if phase == 3 then 0.45 else 0.3)
			local save = e.armAngle
			e.armAngle = e.beamAngle
			arms(e, if phase == 3 then 4 else 2, cycle(e.clock, CORE_CYCLE), 1600)
			e.armAngle = save
			burn_beams(e)
			e.summon = (e.summon or 3) - dt
			if e.summon <= 0 then
				-- Echoes of the earlier waves pour in from the edges.
				e.summon = if phase == 3 then 7 else 10
				for i = 1, 4 do
					local a = random.float() * math.pi * 2
					spawn(if i == 4 then "dasher" else "chaser", s.width / 2 + math.cos(a) * s.width * 0.48, s.height / 2 + math.sin(a) * s.height * 0.46, 1.2)
				end
			end
		else
			e.beams = {}
		end
	end
	return 0
end

-- The speed for the kinds that do more than chase, or nil to keep chasing.
-- The tumour divides: while it is unprotected a clock runs down and it buds
-- off metastases (more as it weakens), and at two thirds and one third of
-- its health it bursts into five at once. Kill them to hurt it again.
local function tumor_step(e, dt)
	local stage = if e.hp > e.maxHp * 2 / 3 then 1 elseif e.hp > e.maxHp / 3 then 2 else 3
	local count = 0
	if stage ~= (e.stage or 1) then
		e.stage = stage
		count = 5
	end
	if not shielded(e) then
		e.divide = (e.divide or 2.5) - dt
		if e.divide <= 0 then
			e.divide = ({ 6, 5, 4 })[stage]
			count = math.max(count, stage + 1)
		end
	end
	if count > 0 then
		local start = random.float() * math.pi * 2
		for i = 1, count do
			local a = start + i * 2 * math.pi / count
			spawn("metastasis", e.x + math.cos(a) * e.r, e.y + math.sin(a) * e.r, 0.35)
		end
		ring(e.x, e.y, 140, COLORS.tumor)
		sfx("boss")
		s.shake = math.max(s.shake, 4)
	end
	-- It fights back the whole time: a pulse of big orbs with a gap to slip
	-- through, quicker as it weakens; from the second stage a spore fan
	-- aimed at the nearest pilot between pulses.
	e.fire -= dt
	if e.fire <= 0 then
		e.fire = ({ 3.0, 2.4, 1.8 })[stage]
		local gap = random.float() * math.pi * 2
		local n = ({ 14, 18, 22 })[stage]
		for i = 2, n - 1 do hostile_shot(e.x, e.y, gap + i * 2 * math.pi / n, 140 + stage * 15, nil, 1.6) end
		ring(e.x, e.y, 90, COLORS.tumor)
	end
	if stage >= 2 then
		e.spore = (e.spore or 1.2) - dt
		if e.spore <= 0 then
			e.spore = if stage == 3 then 1.1 else 1.6
			local p = nearest(e.x, e.y)
			if p then
				local a = math.atan2(p.y - e.y, p.x - e.x)
				for i = -2, 2 do hostile_shot(e.x, e.y, a + i * 0.14, 230, nil, 1.3) end
			end
		end
	end
	return nil
end

local function special(e, dx, dy, length, dt)
	if e.kind == "tumor" then return tumor_step(e, dt) end
	if e.kind == "dasher" then
		-- Telegraphed charge: stop, lock a line, then rush along it.
		e.fire -= dt
		if e.rush and e.rush > 0 then
			e.rush -= dt
			e.x = math.clamp(e.x + e.lockX * 600 * dt, e.r, s.width - e.r)
			e.y = math.clamp(e.y + e.lockY * 600 * dt, e.r, s.height - e.r)
			if e.rush <= 0 then e.fire = 1.4 end
			return 0
		elseif e.aim and e.aim > 0 then
			e.aim -= dt
			if e.aim <= 0 then
				e.rush = 0.5
				sfx("pulse")
			end
			return 0
		elseif e.fire <= 0 and length < 360 then
			e.aim, e.lockX, e.lockY = 0.6, dx, dy
			return 0
		end
	elseif e.kind == "orbiter" then
		-- Circles at range and lobs small aimed fans.
		e.fire -= dt
		if e.fire <= 0 then
			e.fire = 2.4
			fire_at(e, dx, dy, 3, 0.22, 210)
		end
		local turn = if e.phase > 3.14 then 1 else -1
		local radial = math.clamp((length - 230) / 80, -1, 1)
		local tx, ty = dx * radial - dy * turn, dy * radial + dx * turn
		local n = math.max(0.001, len(tx, ty))
		e.x, e.y = e.x + tx / n * SPECS.orbiter.speed * dt, e.y + ty / n * SPECS.orbiter.speed * dt
		return 0
	elseif e.kind == "hive" then
		-- Hive queen: slow, keeps hatching chasers and fires wide aimed fans.
		local enraged = e.hp < e.maxHp / 2
		e.fire -= dt
		e.hatch = (e.hatch or 2) - dt
		if e.fire <= 0 then
			e.fire = if enraged then 1.6 else 2.4
			fire_at(e, dx, dy, if enraged then 7 else 5, 0.2, 200)
		end
		if e.hatch <= 0 then
			e.hatch = if enraged then 2.2 else 3.4
			for i = 1, 3 do
				if #s.enemies < LIMITS.enemies - 8 then
					local a = random.float() * math.pi * 2
					spawn(if enraged and i == 2 then "weaver" else "chaser", e.x + math.cos(a) * 60, e.y + math.sin(a) * 60, 0.45)
				end
			end
			ring(e.x, e.y, 90, COLORS.hive)
		end
		if length < 200 then return 0 end
	elseif e.kind == "serpent" then
		-- Weaves toward the team; its armoured body blocks shots. The head
		-- spits a wide fan of big orbs; angry, it spits faster.
		e.fire -= dt
		if e.fire <= 0 then
			e.fire = if e.hp < e.maxHp / 2 then 1.3 else 2.0
			fire_at(e, dx, dy, 7, 0.16, 240, 1.8)
		end
		local wave = math.sin(s.time * 2.4 + e.phase) * 0.9
		local x, y = dx - dy * wave, dy + dx * wave
		local n = math.max(0.001, len(x, y))
		local speed = SPECS.serpent.speed * (if e.hp < e.maxHp / 2 then 1.25 else 1)
		e.x = math.clamp(e.x + x / n * speed * dt, e.r, s.width - e.r)
		e.y = math.clamp(e.y + y / n * speed * dt, e.r, s.height - e.r)
		return 0
	elseif e.kind == "prism" or e.kind == "sun" or e.kind == "singularity" then
		return late_boss(e, dx, dy, dt)
	end
	return nil
end

local function touch(e, also_kill)
	for _, pilot in s.players do
		if not e.dead and alive(pilot) and (pilot.x - e.x) ^ 2 + (pilot.y - e.y) ^ 2 < (e.r + 11) ^ 2 then
			-- Fodder that touches a pilot is spent, even while the pilot is
			-- still blinking from the last hit.
			hurt(pilot)
			if also_kill and not tough(e) then kill(e) end
		end
	end
end

local function enemies_step(dt)
	for _, e in s.enemies do
		if not e.dead then
			if e.warm > 0 then
				e.warm = math.max(0, e.warm - dt)
				if e.warm == 0 then
					local _, distance = nearest(e.x, e.y)
					if distance < 80 ^ 2 then e.warm = 0.15 end
				end
			end
			e.flash = math.max(0, e.flash - dt)
			if SPECS[e.kind].boss and e.warm == 0 then boss_escorts(e, dt) end
			local p = nearest(e.x, e.y)
			if e.kind == "node" then
				node_step(e)
				p = nil
			elseif e.kind == "segment" then
				-- A lost head takes the whole body.
				local lead = enemy(e.lead)
				if not enemy(e.head) or not lead then
					kill(e)
				elseif e.warm == 0 then
					local dx, dy = e.x - lead.x, e.y - lead.y
					local d = len(dx, dy)
					if d > 24 then e.x, e.y = lead.x + dx / d * 24, lead.y + dy / d * 24 end
					if d > 0.001 then e.heading = math.atan2(-dy, -dx) end
					if p then touch(e, false) end
					-- Every body segment spits an orb sideways now and then,
					-- staggered so the body ripples with fire.
					e.fire = (e.fire or (e.phase % 1) * 3.5) - dt
					if e.fire <= 0 and p then
						e.fire = 3.5
						local nx, ny = -dy, dx
						local n = math.max(0.001, len(nx, ny))
						nx, ny = nx / n, ny / n
						if nx * (p.x - e.x) + ny * (p.y - e.y) < 0 then nx, ny = -nx, -ny end
						fire_at(e, nx, ny, 1, 0, 190, 1.4)
					end
				end
				p = nil
			end
			if p and e.warm == 0 then
				local dx, dy = p.x - e.x, p.y - e.y
				local length = math.max(1, len(dx, dy))
				dx, dy = dx / length, dy / length
				local speed = SPECS[e.kind].speed * (1 + math.min(0.30, math.max(0, s.wave - 1) * 0.035))
				if e.kind == "weaver" then
					local wave = math.sin(s.time * 5 + e.phase) * 0.85
					dx, dy = dx - dy * wave, dy + dx * wave
				end
				if e.kind == "boss" then
					e.fire -= dt
					if length < 260 then speed = 0 end
					if e.fire <= 0 then
						e.fire = if e.hp < e.maxHp / 2 then 1.1 else 1.8
						-- A rotating radial volley has visible lanes to dodge.
						for i = 1, 12 do hostile_shot(e.x, e.y, i * math.pi / 6 + s.time * 0.4, 190, 6) end
					end
					-- Wounded, it sheds new virus particles from its spikes.
					if e.hp < e.maxHp / 2 then
						e.shed = (e.shed or 4) - dt
						if e.shed <= 0 then
							e.shed = 5
							for i = 1, 3 do
								local a = s.time + i * 2 * math.pi / 3
								spawn("chaser", e.x + math.cos(a) * e.r * 1.4, e.y + math.sin(a) * e.r * 1.4, 0.4)
							end
						end
					end
				end
				speed = special(e, dx, dy, length, dt) or speed
				if e.kind == "fort" then
					if length < 300 then speed = 0 end
					e.fire -= dt
					if e.fire <= 0 and #s.hostile < LIMITS.hostile then
						e.fire = 1.5
						table.insert(s.hostile, { x = e.x, y = e.y, vx = dx * 230, vy = dy * 230, ttl = 5 })
					end
				end
				e.x, e.y = e.x + dx * speed * dt, e.y + dy * speed * dt
				-- Where it is swimming, for germs drawn nose-first (flagella behind).
				if speed > 0 then e.heading = math.atan2(dy, dx) end
				touch(e, true)
			end
		end
	end
end

local function crossed(ax, ay, bx, by, x, y, r)
	local dx, dy = bx - ax, by - ay
	local t = math.clamp(((x - ax) * dx + (y - ay) * dy) / math.max(0.0001, dx * dx + dy * dy), 0, 1)
	return (ax + dx * t - x) ^ 2 + (ay + dy * t - y) ^ 2 < r * r
end

-- Softened inverse-square pull, and the lethal core.
local function gravity_step(dt)
	local fs = fields()
	if #fs == 0 then return end
	local lethal = s.waveClock >= 3
	for _, list in { s.players, s.enemies, s.pickups, s.shots, s.hostile } do
		local projectile = list == s.shots or list == s.hostile
		for _, v in list do
			if not v.dead and (not v.hp or v.hp > 0) then
				local ax, ay = force(v.x, v.y)
				if projectile then
					v.vx, v.vy = v.vx + ax * dt, v.vy + ay * dt
				else
					local damping = math.exp(-2.5 * dt)
					v.gravityX = ((v.gravityX or 0) + ax * dt) * damping
					v.gravityY = ((v.gravityY or 0) + ay * dt) * damping
					v.x = math.clamp(v.x + v.gravityX * dt, 18, s.width - 18)
					v.y = math.clamp(v.y + v.gravityY * dt, 18, s.height - 18)
				end
				local px, py = v.spaceX or v.x, v.spaceY or v.y
				if lethal then
					for _, f in fs do
						if crossed(px, py, v.x, v.y, f.x, f.y, f.r) then
							if list == s.players then
								v.hp, v.downTime, v.revive, v.gravityX, v.gravityY = 0, 0, 0, 0, 0
								-- Leave a rescuable marker outside the lethal core.
								v.x, v.y = f.x - 95, f.y
								sfx("death")
								s.shake = 5
								if not v.bot then players.rumble(v.id, 1, 0.5) end
							elseif list == s.enemies then
								v.dead = true
							else
								v.ttl = 0
							end
						end
					end
				end
				v.spaceX, v.spaceY = v.x, v.y
			end
		end
		if projectile or list == s.pickups then
			for i = #list, 1, -1 do
				if list[i].ttl <= 0 then table.remove(list, i) end
			end
		end
	end
end

local function shots_step(dt)
	local grid = {}
	for _, e in s.enemies do
		if not e.dead and e.warm == 0 then
			local k = math.floor(e.y / 64) * 32 + math.floor(e.x / 64)
			grid[k] = grid[k] or {}
			table.insert(grid[k], e)
		end
	end
	for i = #s.shots, 1, -1 do
		local b = s.shots[i]
		local x, y = b.x + b.vx * dt, b.y + b.vy * dt
		b.ttl -= dt
		while b.hits > 0 do
			local best, t = nil, math.huge
			for gy = math.floor((math.min(b.y, y) - 45) / 64), math.floor((math.max(b.y, y) + 45) / 64) do
				for gx = math.floor((math.min(b.x, x) - 45) / 64), math.floor((math.max(b.x, x) + 45) / 64) do
					local cell = grid[gy * 32 + gx]
					if cell then
						for _, e in cell do
							if not e.dead and not b.hit[e.uid] then
								local hit = segment_hit(b.x, b.y, x, y, e.x, e.y, e.r + 3)
								if hit and hit < t then best, t = e, hit end
							end
						end
					end
				end
			end
			if not best then break end
			b.hit[best.uid] = true
			b.hits -= 1
			if not SPECS[best.kind].armor and not shielded(best) then best.hp -= b.damage end
			best.flash = 0.09
			burst(b.x + (x - b.x) * t, b.y + (y - b.y) * t, { 0.8, 0.9, 1 }, 3)
			if best.hp <= 0 then kill(best) else sfx("hit") end
		end
		b.x, b.y = x, y
		if b.ttl <= 0 or b.hits == 0 or x < -20 or x > s.width + 20 or y < -20 or y > s.height + 20 then
			s.shots[i] = s.shots[#s.shots]
			s.shots[#s.shots] = nil
		end
	end
	for i = #s.hostile, 1, -1 do
		local b = s.hostile[i]
		local x, y = b.x + b.vx * dt, b.y + b.vy * dt
		b.ttl -= dt
		for _, p in s.players do
			if alive(p) and segment_hit(b.x, b.y, x, y, p.x, p.y, 14 + 4 * ((b.size or 1) - 1)) then
				hurt(p)
				b.ttl = 0
				break
			end
		end
		b.x, b.y = x, y
		if b.ttl <= 0 or x < -20 or x > s.width + 20 or y < -20 or y > s.height + 20 then
			s.hostile[i] = s.hostile[#s.hostile]
			s.hostile[#s.hostile] = nil
		end
	end
end

local function pickups_step(dt)
	for i = #s.pickups, 1, -1 do
		local q = s.pickups[i]
		q.ttl -= dt
		local p, d = nearest(q.x, q.y)
		-- Once a pickup notices you it keeps homing in and speeds up, so it
		-- always catches you (a fixed pull trailed a fleeing pilot forever).
		if p and (q.magnet or d < 80 ^ 2) then
			q.magnet = true
			q.speed = (q.speed or 200) + 1600 * dt
			local dist = math.max(1, math.sqrt(d))
			local step = math.min(dist, q.speed * dt)
			q.x += (p.x - q.x) / dist * step
			q.y += (p.y - q.y) / dist * step
		end
		if p and d < 22 ^ 2 then
			if q.kind == "repair" then
				p.hp = math.min(s.lives, p.hp + 1)
			else
				p.powers[q.kind] = AMMO[q.kind]
			end
			q.ttl = 0
			-- Each upgrade has its own sound (sounds/up_KIND.luau).
			sfx("up_" .. q.kind)
			ring(p.x, p.y, 40, { 0.5, 1, 0.7 })
		end
		if q.ttl <= 0 then table.remove(s.pickups, i) end
	end
end

----------------------------------------------------------------------------
-- Showing it: entities follow the simulation every tick.

-- Glow: how bright the lines burn (emissive), per kind of thing.
local GLOW = {
	dim = KNOBS.line_glow * 0.6,
	line = KNOBS.line_glow,
	white = KNOBS.line_glow * 1.25,
	warm = KNOBS.line_glow * 0.15,
	shot = KNOBS.shot_glow,
	grid = KNOBS.grid_glow,
	major = KNOBS.grid_glow * 1.9,
	hostile = KNOBS.shot_glow * 0.6,
}

-- The outline shape name for a polygon of radius r (arena units): its line
-- stays LINE wide whatever the size, in steps so few meshes are made.
local function outline(n, r, line)
	local w = (line or LINE) / (2 * r)
	w = math.clamp(math.round(w * 200) / 200, 0.005, 0.5)
	return "outline" .. n .. ":" .. w
end

local function part_look(kind, part, mode)
	local c = COLORS[kind]
	if mode == "flash" or (part.white and mode ~= "warm") then return rgb(WHITE), GLOW.white end
	if mode == "warm" then return rgb(c, 0.5), GLOW.warm end
	if part.dim then return rgb(c, 0.6), GLOW.line * 0.5 end
	return rgb(c), GLOW.line
end

-- How big a kind is drawn, apart from how big it is to hit: the weak germs
-- (one or two hits) look smaller, so size reads as toughness.
local DRAW_SCALE = { chaser = 0.7, shard = 0.75, weaver = 0.75, dasher = 0.8, orbiter = 0.85 }
local function draw_r(kind)
	return SPECS[kind].r * DRAW * (DRAW_SCALE[kind] or 1)
end

local function part_shape(kind, part)
	return outline(part.n, draw_r(kind) * (part.k or 1))
end

-- Remember every entity this tick shows; the rest are spent.
local function seen(id)
	s.ents[id] = s.stamp
	return id
end

-- Where a beam leaves the arena (the LÖVE screen clips them).
local function clip(x1, y1, x2, y2)
	local dx, dy = x2 - x1, y2 - y1
	local t = 1
	if dx > 0 then t = math.min(t, (s.width - x1) / dx) elseif dx < 0 then t = math.min(t, -x1 / dx) end
	if dy > 0 then t = math.min(t, (s.height - y1) / dy) elseif dy < 0 then t = math.min(t, -y1 / dy) end
	t = math.max(0, t)
	return x1 + dx * t, y1 + dy * t
end

-- A flat line from (x1, y1) to (x2, y2) in arena units.
local function line(x1, y1, x2, y2, width, color, glow, tag)
	local dx, dy = x2 - x1, y2 - y1
	return world.spawn("cube", {
		pos = at((x1 + x2) / 2, (y1 + y2) / 2, 0.12),
		rot = vector.create(0, math.atan2(-dx, -dy), 0),
		size = vector.create(width * S, 0.03, len(dx, dy) * S),
		color = color,
		emissive = glow,
		solid = false,
		tag = tag,
	})
end

-- The angle the LÖVE screen turns a kind's polygon to.
local function enemy_angle(e)
	local angle = if e.kind == "fort" then math.pi / 4 else s.time * 0.7 + e.phase
	if e.kind == "dasher" then
		if e.lockX and ((e.aim or 0) > 0 or (e.rush or 0) > 0) then angle = math.atan2(e.lockY, e.lockX) end
	elseif e.kind == "serpent" and e.segments and e.segments[1] then
		local seg = enemy(e.segments[1])
		if seg then angle = math.atan2(e.y - seg.y, e.x - seg.x) end
	elseif (e.kind == "weaver" or e.kind == "chaser") and e.heading then
		angle = e.heading
	elseif e.kind == "segment" and e.heading then
		angle = e.heading
	elseif e.kind == "segment" or e.kind == "hive" then
		angle = e.phase
	elseif e.kind == "prism" then
		angle = s.time * (if enemy(e.twin) then 0.8 else 2.2)
	elseif e.kind == "node" then
		angle = e.phase * 2
	end
	return angle
end

-- Germs in 3D: neon wire meshes that tumble and move. A mesh is a function
-- of time and the enemy that returns its points (model units: radius about
-- 1, +x forward, +y up) and a fixed list of edges { i, j, white? }. Heading
-- meshes point +x where the germ swims; the rest tumble freely.
local function v3(x, y, z) return vector.create(x, y, z) end
local OCTA = { v3(1, 0, 0), v3(-1, 0, 0), v3(0, 1, 0), v3(0, -1, 0), v3(0, 0, 1), v3(0, 0, -1) }
local OCTA_EDGES = { { 1, 3 }, { 1, 4 }, { 1, 5 }, { 1, 6 }, { 2, 3 }, { 2, 4 }, { 2, 5 }, { 2, 6 }, { 3, 5 }, { 3, 6 }, { 4, 5 }, { 4, 6 } }
local TETRA = { v3(0.577, 0.577, 0.577), v3(0.577, -0.577, -0.577), v3(-0.577, 0.577, -0.577), v3(-0.577, -0.577, 0.577) }
local TETRA_EDGES = { { 1, 2 }, { 1, 3 }, { 1, 4 }, { 2, 3 }, { 2, 4 }, { 3, 4 } }
local CUBE = {}
for _, x in { -0.577, 0.577 } do for _, y in { -0.577, 0.577 } do for _, z in { -0.577, 0.577 } do table.insert(CUBE, v3(x, y, z)) end end end
local CUBE_EDGES = {}
for i = 1, 8 do
	for j = i + 1, 8 do
		local d = CUBE[i] - CUBE[j]
		if math.abs(d.x * d.x + d.y * d.y + d.z * d.z - 1.332) < 0.01 then table.insert(CUBE_EDGES, { i, j }) end
	end
end
local PHI = (1 + math.sqrt(5)) / 2
local ICOSA = {}
for _, p in { { 0, 1, PHI }, { 0, -1, PHI }, { 0, 1, -PHI }, { 0, -1, -PHI }, { 1, PHI, 0 }, { -1, PHI, 0 }, { 1, -PHI, 0 }, { -1, -PHI, 0 }, { PHI, 0, 1 }, { -PHI, 0, 1 }, { PHI, 0, -1 }, { -PHI, 0, -1 } } do
	local n = math.sqrt(p[1] ^ 2 + p[2] ^ 2 + p[3] ^ 2)
	table.insert(ICOSA, v3(p[1] / n, p[2] / n, p[3] / n))
end
local ICOSA_EDGES = {}
for i = 1, 12 do
	for j = i + 1, 12 do
		local d = ICOSA[i] - ICOSA[j]
		if math.abs(d.x * d.x + d.y * d.y + d.z * d.z - 1.1056) < 0.01 then table.insert(ICOSA_EDGES, { i, j }) end
	end
end

-- Append a shape's points (scaled, moved) and edges (offset) to a mesh.
local function add(points, edges, shape, shape_edges, scale, at_, white)
	local base = #points
	for _, p in shape do table.insert(points, p * scale + at_) end
	for _, e in shape_edges do table.insert(edges, { e[1] + base, e[2] + base, white }) end
end

local ZERO = v3(0, 0, 0)
local MESHES = {}

-- A virus: an octahedral capsid with six spikes that pulse.
MESHES.chaser = function(t, e)
	local pts, edges = {}, {}
	add(pts, edges, OCTA, OCTA_EDGES, 0.7, ZERO)
	for i, p in OCTA do
		table.insert(pts, p * (1.3 + 0.12 * math.sin(t * 6 + i + e.phase)))
		table.insert(edges, { i, #pts })
	end
	return pts, edges
end

-- A hexagonal ring of radius `r` across the x axis at `x`; returns its first index.
local function hex_ring(pts, edges, x, r, twist)
	local base = #pts
	for i = 0, 5 do
		local a = i * math.pi / 3 + (twist or 0)
		table.insert(pts, v3(x, math.cos(a) * r, math.sin(a) * r))
	end
	for i = 0, 5 do table.insert(edges, { base + i + 1, base + (i + 1) % 6 + 1 }) end
	return base + 1
end

-- The serpent's head (+x forward): a hexagonal skull that narrows to a snout,
-- two pairs of mandibles that snap open and shut, and a white eye core.
MESHES.serpent = function(t, e)
	local pts, edges = {}, {}
	local back = hex_ring(pts, edges, -0.8, 0.75)
	local mid = hex_ring(pts, edges, 0.1, 1.0, math.pi / 6)
	local front = hex_ring(pts, edges, 0.85, 0.5)
	for i = 0, 5 do
		table.insert(edges, { back + i, mid + i })
		table.insert(edges, { mid + i, front + i })
	end
	local bite = 0.35 + 0.3 * math.abs(math.sin(t * 5 + e.phase))
	for _, side in { -1, 1 } do
		for _, up in { -0.35, 0.35 } do
			table.insert(pts, v3(0.7, side * 0.45, up))
			table.insert(pts, v3(1.6, side * bite, up * 0.6))
			table.insert(pts, v3(1.35, side * (bite - 0.25), up * 0.4))
			local n = #pts
			table.insert(edges, { n - 2, n - 1 })
			table.insert(edges, { n - 1, n })
		end
	end
	add(pts, edges, OCTA, OCTA_EDGES, 0.3, v3(0.25, 0, 0), true)
	return pts, edges
end

-- A serpent body segment: an armoured hexagonal drum with a pair of legs
-- that paddle in turn down the body.
MESHES.segment = function(t, e)
	local pts, edges = {}, {}
	local a = hex_ring(pts, edges, -0.45, 0.9)
	local b = hex_ring(pts, edges, 0.45, 0.9)
	for i = 0, 5 do table.insert(edges, { a + i, b + i }) end
	local stroke = math.sin(t * 7 + e.phase) * 0.45
	for _, side in { -1, 1 } do
		table.insert(pts, v3(0, side * 0.9, 0))
		table.insert(pts, v3(stroke * side, side * 1.55, -0.2))
		table.insert(edges, { #pts - 1, #pts })
	end
	add(pts, edges, TETRA, TETRA_EDGES, 0.3, ZERO, true)
	return pts, edges
end

-- A small virus: a tetrahedron with four long spikes.
MESHES.shard = function(t, e)
	local pts, edges = {}, {}
	add(pts, edges, TETRA, TETRA_EDGES, 0.8, ZERO)
	for i, p in TETRA do
		table.insert(pts, p * (1.5 + 0.1 * math.sin(t * 8 + i)))
		table.insert(edges, { i, #pts })
	end
	return pts, edges
end

-- A corona-like virus: a capsid with six stalks, each crowned by a bar.
MESHES.orbiter = function(t, e)
	local pts, edges = {}, {}
	add(pts, edges, OCTA, OCTA_EDGES, 0.65, ZERO)
	for i, p in OCTA do
		local tip = p * 1.1
		-- A bar across the stalk: the cross product with a fixed slant.
		local cx, cy, cz = p.y * 0.7 - p.z * 0.6, p.z * 0.3 - p.x * 0.7, p.x * 0.6 - p.y * 0.3
		local cn = math.sqrt(cx * cx + cy * cy + cz * cz)
		local side = v3(cx / cn, cy / cn, cz / cn) * 0.22
		table.insert(pts, tip)
		table.insert(edges, { i, #pts })
		table.insert(pts, tip + side)
		table.insert(pts, tip - side)
		table.insert(edges, { #pts - 1, #pts })
	end
	return pts, edges
end

-- A bacillus: a rod of pentagon rings with pointed caps, rolling as it
-- swims, two flagella whipping behind.
MESHES.weaver = function(t, e)
	local pts, edges = {}, {}
	local function ring(x)
		local first = #pts + 1
		for i = 0, 4 do
			local a = i * 2 * math.pi / 5
			table.insert(pts, v3(x, math.cos(a) * 0.42, math.sin(a) * 0.42))
		end
		for i = 0, 4 do table.insert(edges, { first + i, first + (i + 1) % 5 }) end
		return first
	end
	local front, back = ring(0.75), ring(-0.75)
	for i = 0, 4 do table.insert(edges, { front + i, back + i }) end
	table.insert(pts, v3(1.2, 0, 0))
	for i = 0, 4 do table.insert(edges, { front + i, #pts }) end
	table.insert(pts, v3(-1.2, 0, 0))
	local tail = #pts
	for i = 0, 4 do table.insert(edges, { back + i, tail }) end
	for side = -1, 1, 2 do
		local last = tail
		for k = 1, 4 do
			local wave = math.sin(t * 14 - k * 1.3 + side + e.phase) * 0.22 * k / 4
			table.insert(pts, v3(-1.2 - k * 0.38, wave, side * 0.12 * k))
			table.insert(edges, { last, #pts })
			last = #pts
		end
	end
	return pts, edges
end

-- A phage: an octahedral head, a three-sided sheath and three walking legs.
MESHES.dasher = function(t, e)
	local pts, edges = {}, {}
	add(pts, edges, OCTA, OCTA_EDGES, 0.42, v3(0.75, 0, 0))
	local function tri(x)
		local first = #pts + 1
		for i = 0, 2 do
			local a = i * 2 * math.pi / 3
			table.insert(pts, v3(x, math.cos(a) * 0.12, math.sin(a) * 0.12))
		end
		for i = 0, 2 do table.insert(edges, { first + i, first + (i + 1) % 3 }) end
		return first
	end
	local top, bottom = tri(0.33), tri(-0.45)
	for i = 0, 2 do table.insert(edges, { top + i, bottom + i }) end
	table.insert(edges, { 2, top })
	for leg = 0, 2 do
		local a = leg * 2 * math.pi / 3 + math.pi / 3
		local step = math.sin(t * 9 + leg * 2.1 + e.phase) * 0.15
		table.insert(pts, v3(-0.55 + step, math.cos(a) * 0.45, math.sin(a) * 0.45))
		table.insert(edges, { bottom + leg, #pts })
		table.insert(pts, v3(-0.85 + step, math.cos(a) * 0.7, math.sin(a) * 0.7))
		table.insert(edges, { #pts - 1, #pts })
	end
	return pts, edges
end

-- A dividing cell: two capsules pulling apart and back together.
MESHES.splitter = function(t, e)
	local pts, edges = {}, {}
	local gap = 0.35 + 0.12 * math.sin(t * 3 + e.phase)
	add(pts, edges, OCTA, OCTA_EDGES, 0.6, v3(gap, 0, 0))
	add(pts, edges, OCTA, OCTA_EDGES, 0.6, v3(-gap, 0, 0))
	return pts, edges
end

-- A cyst: a hard cube shell around a turning core.
MESHES.fort = function(t, e)
	local pts, edges = {}, {}
	add(pts, edges, CUBE, CUBE_EDGES, 0.9, ZERO)
	add(pts, edges, OCTA, OCTA_EDGES, 0.45 + 0.05 * math.sin(t * 4), ZERO, true)
	return pts, edges
end

-- The virion (first boss): an icosahedral capsid with twelve spike fibres,
-- each ending in a three-pronged head, a membrane ring turning around it
-- and a white genome core spinning the other way.
MESHES.boss = function(t, e)
	local pts, edges = {}, {}
	local pulse = 1 + 0.05 * math.sin(t * 2.5)
	add(pts, edges, ICOSA, ICOSA_EDGES, 0.72 * pulse, ZERO)
	for i, p in ICOSA do
		local reach = 1.2 + 0.1 * math.sin(t * 4 + i * 0.9)
		local tip = p * reach
		table.insert(pts, tip)
		local stem = #pts
		table.insert(edges, { i, stem })
		-- Two directions across the fibre for its prongs.
		local ref = if math.abs(p.y) < 0.9 then v3(0, 1, 0) else v3(1, 0, 0)
		local ax, ay, az = p.y * ref.z - p.z * ref.y, p.z * ref.x - p.x * ref.z, p.x * ref.y - p.y * ref.x
		local an = math.sqrt(ax * ax + ay * ay + az * az)
		local a = v3(ax / an, ay / an, az / an)
		local b = v3(p.y * a.z - p.z * a.y, p.z * a.x - p.x * a.z, p.x * a.y - p.y * a.x)
		for k = 0, 2 do
			local turn = k * 2 * math.pi / 3 + t * 1.5
			table.insert(pts, tip + p * 0.12 + (a * math.cos(turn) + b * math.sin(turn)) * 0.13)
			table.insert(edges, { stem, #pts })
		end
	end
	-- The membrane ring, turning on its own.
	local first = #pts + 1
	for k = 0, 19 do
		local a = k * 2 * math.pi / 20 + t * 0.7
		table.insert(pts, v3(math.cos(a) * 0.98, 0.08 * math.sin(a * 3 + t * 3), math.sin(a) * 0.98))
	end
	for k = 0, 19 do table.insert(edges, { first + k, first + (k + 1) % 20 }) end
	-- The genome core, counter-spinning.
	local spin = -t * 2
	local core = {}
	for _, p in OCTA do
		table.insert(core, v3(p.x * math.cos(spin) - p.z * math.sin(spin), p.y, p.x * math.sin(spin) + p.z * math.cos(spin)))
	end
	add(pts, edges, core, OCTA_EDGES, 0.3, ZERO, true)
	return pts, edges
end

-- The hive queen: a colony mother. A soft icosahedral body, six egg sacs
-- on tethers around her that swell as the next hatch comes near, and a
-- white nucleus.
MESHES.hive = function(t, e)
	local pts, edges = {}, {}
	add(pts, edges, ICOSA, ICOSA_EDGES, 0.62 + 0.04 * math.sin(t * 2), ZERO)
	local swell = 1 - math.clamp((e.hatch or 2) / 3.4, 0, 1)
	for i = 0, 5 do
		local a = i * math.pi / 3 + t * 0.4
		local at_ = v3(math.cos(a) * 1.15, 0.15 * math.sin(t * 3 + i), math.sin(a) * 1.15)
		local before = #pts
		add(pts, edges, OCTA, OCTA_EDGES, 0.16 + 0.12 * swell, at_)
		-- A tether from the nearest body corner to the sac.
		local best, d = 1, math.huge
		for k, p in ICOSA do
			local q = p * 0.62 - at_
			local dd = q.x * q.x + q.y * q.y + q.z * q.z
			if dd < d then best, d = k, dd end
		end
		table.insert(edges, { best, before + 1 })
	end
	add(pts, edges, TETRA, TETRA_EDGES, 0.25, ZERO, true)
	return pts, edges
end

-- A prism twin: a long crystal (two hexagonal pyramids on a hexagonal
-- prism), a white seed spinning inside.
MESHES.prism = function(t, e)
	local pts, edges = {}, {}
	local function hex(y, r)
		local first = #pts + 1
		for k = 0, 5 do
			local a = k * math.pi / 3
			table.insert(pts, v3(math.cos(a) * r, y, math.sin(a) * r))
		end
		for k = 0, 5 do table.insert(edges, { first + k, first + (k + 1) % 6 }) end
		return first
	end
	local top, bottom = hex(0.45, 0.55), hex(-0.45, 0.55)
	for k = 0, 5 do table.insert(edges, { top + k, bottom + k }) end
	table.insert(pts, v3(0, 1.25, 0))
	for k = 0, 5 do table.insert(edges, { top + k, #pts }) end
	table.insert(pts, v3(0, -1.25, 0))
	for k = 0, 5 do table.insert(edges, { bottom + k, #pts }) end
	add(pts, edges, OCTA, OCTA_EDGES, 0.22 + 0.04 * math.sin(t * 6), ZERO, true)
	return pts, edges
end

-- The solar warden: a radiolarian, a glassy shell inside a shell with
-- long spines that breathe in and out.
MESHES.sun = function(t, e)
	local pts, edges = {}, {}
	add(pts, edges, ICOSA, ICOSA_EDGES, 0.85, ZERO)
	add(pts, edges, ICOSA, ICOSA_EDGES, 0.45, ZERO)
	for i = 1, 12 do table.insert(edges, { i, 12 + i }) end
	for i, p in ICOSA do
		table.insert(pts, p * (1.45 + 0.18 * math.sin(t * 2 + i)))
		table.insert(edges, { i, #pts })
	end
	add(pts, edges, OCTA, OCTA_EDGES, 0.2, ZERO, true)
	return pts, edges
end

-- A metastasis: a lumpy cell whose corners never sit still, around a nucleus.
MESHES.metastasis = function(t, e)
	local pts, edges = {}, {}
	for i, p in OCTA do table.insert(pts, p * (0.85 + 0.22 * math.sin(t * 5 + i * 1.7 + e.phase))) end
	for _, ed in OCTA_EDGES do table.insert(edges, { ed[1], ed[2] }) end
	add(pts, edges, TETRA, TETRA_EDGES, 0.35, ZERO, true)
	return pts, edges
end

-- The tumour: a big icosahedral mass bulging and shrinking, a nucleus inside.
MESHES.tumor = function(t, e)
	local pts, edges = {}, {}
	for i, p in ICOSA do table.insert(pts, p * (1 + 0.12 * math.sin(t * 3 + i * 2.1))) end
	for _, ed in ICOSA_EDGES do table.insert(edges, { ed[1], ed[2] }) end
	add(pts, edges, OCTA, OCTA_EDGES, 0.4, ZERO, true)
	return pts, edges
end

-- Meshes that point where they swim (the rest tumble).
local MESH_HEADING = { weaver = true, dasher = true, serpent = true, segment = true }
local WIRE = 0.02 -- edge thickness, metres

-- Does this engine have wire entities (one entity per mesh)? Older builds
-- only have cubes, so the cube path below stays as a fallback.
local wire_ok = nil
local function has_wire()
	if wire_ok == nil then
		wire_ok = pcall(function()
			world.remove(world.spawn("wire", { points = { v3(0, 0, 0), v3(0, 0, 1) }, edges = { { 1, 2 } }, width = 0.01, solid = false, visible = false }))
		end)
	end
	return wire_ok
end

local function show_mesh(e, mode, grow, angle)
	local t = s.time
	local pts, edges = MESHES[e.kind](t, e)
	local color, glow = part_look(e.kind, {}, mode)
	local white, whiteGlow = part_look(e.kind, { white = true }, mode)
	-- Model to world: roll or tumble, then turn to the heading; scale to size.
	local scale = draw_r(e.kind) * S * grow
	local center = at(e.x, e.y, 0.4)
	local r1, r2, yaw
	if MESH_HEADING[e.kind] then
		r1, r2, yaw = t * 2 + e.phase, 0, -angle
	else
		r1, r2, yaw = t * 0.9 + e.phase, t * 0.6 + e.phase * 2, -(t * 0.5 + e.phase)
	end
	local c1, s1, c2, s2, cy, sy = math.cos(r1), math.sin(r1), math.cos(r2), math.sin(r2), math.cos(yaw), math.sin(yaw)
	local rel = {}
	for i, p in pts do
		-- roll about x
		local x, y, z = p.x, p.y * c1 - p.z * s1, p.y * s1 + p.z * c1
		-- tilt about z
		x, y = x * c2 - y * s2, x * s2 + y * c2
		-- turn about the world's up axis
		x, z = x * cy + z * sy, -x * sy + z * cy
		rel[i] = v3(x, y, z) * scale
	end
	local thick = WIRE * math.max(0.4, grow)
	if has_wire() then
		-- One wire for the coloured edges and one for the white nucleus.
		if not e.wires then
			local body, core = {}, {}
			for _, ed in edges do table.insert(if ed[3] then core else body, { ed[1], ed[2] }) end
			e.wires = { world.spawn("wire", { pos = center, points = rel, edges = body, width = thick, color = color, emissive = glow, solid = false, tag = "enemy" }) }
			if #core > 0 then
				table.insert(e.wires, world.spawn("wire", { pos = center, points = rel, edges = core, width = thick, color = white, emissive = whiteGlow, solid = false, tag = "enemy" }))
			end
			e.wireMode = mode
		end
		if e.wireMode ~= mode then
			world.set(e.wires[1], { color = color, emissive = glow })
			if e.wires[2] then world.set(e.wires[2], { color = white, emissive = whiteGlow }) end
			e.wireMode = mode
		end
		for _, id in e.wires do
			world.set(id, { pos = center, points = rel, width = thick })
			seen(id)
		end
		return
	end
	if not e.wire or #e.wire ~= #edges then
		e.wire = {}
		for _, ed in edges do
			local c, g = color, glow
			if ed[3] then c, g = white, whiteGlow end
			table.insert(e.wire, world.spawn("cube", { size = vector.create(WIRE, WIRE, 0.1), color = c, emissive = g, solid = false, tag = "enemy" }))
		end
		e.wireMode = mode
	end
	if e.wireMode ~= mode then
		for i, ed in edges do
			local c, g = color, glow
			if ed[3] then c, g = white, whiteGlow end
			world.set(e.wire[i], { color = c, emissive = g })
		end
		e.wireMode = mode
	end
	for i, ed in edges do
		local a, b = center + rel[ed[1]], center + rel[ed[2]]
		local d = b - a
		local n = math.max(1e-5, math.sqrt(d.x * d.x + d.y * d.y + d.z * d.z))
		world.set(e.wire[i], { pos = (a + b) * 0.5, rot = v3(math.asin(math.clamp(-d.y / n, -1, 1)), math.atan2(d.x, d.z), 0), size = v3(thick, thick, n) })
		seen(e.wire[i])
	end
end

local function show_enemy(e)
	local parts = if MESHES[e.kind] then {} else SHAPES[e.kind]
	-- Bosses and armour take fire all the time: their bar shows the damage, not a flash.
	-- A shielded tumour is drawn dim: bright means it can be hurt.
	local mode = if e.warm > 0 or shielded(e) then "warm" elseif e.flash > 0 and not tough(e) then "flash" else "normal"
	local grow = if e.warm > 0 then 0.35 + 0.65 * (1 - math.min(1, e.warm / WAVE_WARNING)) else 1
	local angle = enemy_angle(e)
	local pos = at(e.x, e.y, 0.1)
	if not e.parts then
		e.parts = {}
		for _, part in parts do
			local color, glow = part_look(e.kind, part, mode)
			table.insert(e.parts, world.spawn(part_shape(e.kind, part), { pos = pos, size = 1, color = color, emissive = glow, solid = false, tag = "enemy" }))
		end
		e.grow = -1
	end
	if e.mode ~= mode then
		for i, part in parts do
			local color, glow = part_look(e.kind, part, mode)
			world.set(e.parts[i], { color = color, emissive = glow })
		end
		e.mode = mode
	end
	for i, part in parts do
		local id = e.parts[i]
		if e.grow ~= grow then
			local d = draw_r(e.kind) * 2 * S * (part.k or 1) * grow
			world.set(id, { size = vector.create(d * (part.long or 1), 0.04, d) })
		end
		local a = if part.still then 0 else angle * (part.spin or 1)
		world.place(id, pos, -a)
		seen(id)
	end
	e.grow = grow
	if MESHES[e.kind] then show_mesh(e, mode, grow, angle) end
	-- Spikes, flagella and legs grow in with the body while it spawns, so the
	-- germ has its real shape from the first frame.
	local spikes = not MESHES[e.kind] and SPIKES[e.kind]
	if spikes then
		local r = draw_r(e.kind) * grow
		local color, glow = part_look(e.kind, parts[1], mode)
		if not e.spikes then
			e.spikes = {}
			for _, sp in spikes do
				local l = r * sp.len
				table.insert(e.spikes, {
					line = world.spawn("cube", { size = vector.create(2.2 * S, 0.03, l * S), color = color, emissive = glow, solid = false, tag = "enemy" }),
					knob = sp.knob and world.spawn(outline(16, 4), { size = 8 * S, color = color, emissive = glow, solid = false, tag = "enemy" }) or nil,
				})
			end
			e.spikeMode = mode
		end
		if e.spikeMode ~= mode then
			for _, ids in e.spikes do
				world.set(ids.line, { color = color, emissive = glow })
				if ids.knob then world.set(ids.knob, { color = color, emissive = glow }) end
			end
			e.spikeMode = mode
		end
		if e.spikeGrow ~= grow then
			for i, sp in spikes do
				world.set(e.spikes[i].line, { size = vector.create(2.2 * S, 0.03, r * sp.len * S) })
				if e.spikes[i].knob then world.set(e.spikes[i].knob, { size = 8 * S * grow }) end
			end
			e.spikeGrow = grow
		end
		for i, sp in spikes do
			local a = angle + sp.a
			local dx, dy = math.cos(a), math.sin(a)
			local r1, r2 = r * (sp.from or 1), r * ((sp.from or 1) + sp.len)
			local ids = e.spikes[i]
			world.place(ids.line, at(e.x + dx * (r1 + r2) / 2, e.y + dy * (r1 + r2) / 2, 0.1), math.atan2(-dx, -dy))
			seen(ids.line)
			if ids.knob then
				world.place(ids.knob, at(e.x + dx * r2, e.y + dy * r2, 0.1))
				seen(ids.knob)
			end
		end
	end
	-- Flagella: a travelling sine wave that grows towards the tip.
	local flagella = not MESHES[e.kind] and FLAGELLA[e.kind]
	if flagella then
		local r = draw_r(e.kind) * grow
		local color, glow = part_look(e.kind, parts[1], mode)
		if not e.flagella then
			e.flagella = {}
			for _ = 1, #flagella * FLAG_SEGMENTS do
				table.insert(e.flagella, world.spawn("cube", { size = vector.create(1.8 * S, 0.03, r * FLAG_LENGTH / FLAG_SEGMENTS * 1.2 * S), color = color, emissive = glow * 0.8, solid = false, tag = "enemy" }))
			end
			e.flagellaMode = mode
		end
		if e.flagellaMode ~= mode then
			for _, id in e.flagella do world.set(id, { color = color, emissive = glow * 0.8 }) end
			e.flagellaMode = mode
		end
		if e.flagellaGrow ~= grow then
			for _, id in e.flagella do world.set(id, { size = vector.create(1.8 * S, 0.03, r * FLAG_LENGTH / FLAG_SEGMENTS * 1.2 * S) }) end
			e.flagellaGrow = grow
		end
		local n = 0
		for f, fl in flagella do
			local base = angle
			if fl.heading then
				local target = nearest(e.x, e.y)
				if target then base = math.atan2(target.y - e.y, target.x - e.x) end
			end
			local c, sn = math.cos(base + fl.a), math.sin(base + fl.a)
			local px, py
			for i = 0, FLAG_SEGMENTS do
				local t = i / FLAG_SEGMENTS
				local along = r * (fl.from + t * FLAG_LENGTH)
				local side = math.sin(t * 9 - s.time * 14 + e.phase + f) * r * 0.3 * t
				local x, y = e.x + c * along - sn * side, e.y + sn * along + c * side
				if i > 0 then
					n += 1
					local dx, dy = x - px, y - py
					world.place(e.flagella[n], at((x + px) / 2, (y + py) / 2, 0.1), math.atan2(-dx, -dy))
					seen(e.flagella[n])
				end
				px, py = x, y
			end
		end
	end
	-- The hive's six cells ring its core.
	if e.kind == "hive" and e.warm <= 0 and not MESHES.hive then
		local color, glow = part_look("hive", parts[1], mode)
		for i = 1, 6 do
			local a = i * math.pi / 3 + e.phase
			local id = seen(world.spawn(outline(6, 12), { pos = at(e.x + math.cos(a) * 26, e.y + math.sin(a) * 26, 0.1), size = vector.create(24 * S, 0.04, 24 * S), color = color, emissive = glow, solid = false, tag = "enemy" }))
			world.place(id, at(e.x + math.cos(a) * 26, e.y + math.sin(a) * 26, 0.1), -e.phase)
		end
	end
	-- The singularity: a black core inside spinning rings of light.
	if e.kind == "singularity" and e.warm <= 0 then
		local phase = e.phase or 1
		-- Seven rings kept from tick to tick (spawning them anew every tick in
		-- an ever-changing colour dropped the game to ~20 fps); the colour
		-- steps through 24 hues so only a few looks ever exist.
		e.rings = e.rings or {}
		for i = 1, 7 do
			local k = i / 7
			local r = e.r * (1.3 + k * 1.6)
			local squash = 0.32 + 0.1 * math.sin(s.time + i)
			local hue = math.floor((s.time * (0.2 + phase * 0.15) + k * 4) * 24 / (2 * math.pi)) * 2 * math.pi / 24
			local color = rgb({ 0.6 + 0.4 * math.sin(hue), 0.35 + 0.3 * math.sin(hue + 2), 1 }, 1 - k * 0.5)
			if not e.rings[i] then
				e.rings[i] = world.spawn(outline(32, r, LINE * 1.2), { pos = pos, size = 1, color = color, emissive = GLOW.line, solid = false, tag = "enemy" })
			end
			local id = e.rings[i]
			if e["ringHue" .. i] ~= hue then
				world.set(id, { color = color })
				e["ringHue" .. i] = hue
			end
			world.set(id, { pos = pos, rot = vector.create(0, -s.time * (0.6 + k) * (if i % 2 == 0 then 1 else -1), 0), size = vector.create(r * 2 * S, 0.04, r * 2 * squash * S) })
			seen(id)
		end
	end
	-- A dasher shows its charge line before the rush.
	if e.kind == "dasher" and (e.aim or 0) > 0 and e.lockX then
		local x2, y2 = clip(e.x, e.y, e.x + e.lockX * 300, e.y + e.lockY * 300)
		seen(line(e.x, e.y, x2, y2, 1.5, rgb(COLORS.dasher, 0.8), GLOW.line * 0.6, "beam"))
	end
	for _, b in e.beams or {} do
		local x2, y2 = clip(b[1], b[2], b[3], b[4])
		if b.state == "live" then
			seen(line(b[1], b[2], x2, y2, 14, rgb(COLORS[e.kind]), GLOW.line, "beam"))
			seen(line(b[1], b[2], x2, y2, 4, rgb(WHITE), GLOW.white, "beam"))
		else
			seen(line(b[1], b[2], x2, y2, 2, rgb(COLORS[e.kind], 0.7), GLOW.line * 0.5, "beam"))
		end
	end
end

local function shot_color(b)
	if b.special then return SHOT_COLORS[b.special] end
	for _, p in s.players do
		if p.id == b.owner then return p.rgb end
	end
	return WHITE
end

-- The pilots are tiny doctors in spaceships: a hull of neon lines in the
-- player's colour (x points where they aim, units are arena units), a white
-- cockpit and two engines that flicker.
local SHIP_HULL = { { 20, 0 }, { 6, 4 }, { -2, 15 }, { -10, 15 }, { -7, 5 }, { -12, 5 }, { -12, -5 }, { -7, -5 }, { -10, -15 }, { -2, -15 }, { 6, -4 } }
local SHIP_ENGINES = { { -12, 3 }, { -12, -3 } }

-- A segment of the ship from local (x1, y1) to (x2, y2), placed for heading a.
local function place_ship_line(id, p, a, x1, y1, x2, y2)
	local c, sn = math.cos(a), math.sin(a)
	local mx, my = (x1 + x2) / 2, (y1 + y2) / 2
	local dx, dy = (x2 - x1) * c - (y2 - y1) * sn, (x2 - x1) * sn + (y2 - y1) * c
	world.place(id, at(p.x + mx * c - my * sn, p.y + mx * sn + my * c, 0.15), math.atan2(-dx, -dy))
end

local function show_pilot(p)
	local a = math.atan2(p.aimY, p.aimX)
	local pos = at(p.x, p.y, 0.15)
	local mode = if not alive(p) then "down" elseif p.invul > 0 and math.floor(s.time * 12) % 2 == 0 then "blink" else "up"
	if not p.hull then
		p.hull = {}
		for i, q in SHIP_HULL do
			local r = SHIP_HULL[i % #SHIP_HULL + 1]
			table.insert(p.hull, world.spawn("cube", { pos = pos, size = vector.create(3 * S, 0.03, len(r[1] - q[1], r[2] - q[2]) * S), color = p.color, emissive = GLOW.line, solid = false, tag = "pilot" }))
		end
		p.cockpit = world.spawn(outline(16, 4, 2), { pos = pos, size = vector.create(12 * S, 0.04, 7 * S), color = "#ffffff", emissive = GLOW.white, solid = false, tag = "pilot" })
		p.engines = {}
		for _ in SHIP_ENGINES do
			table.insert(p.engines, world.spawn("sphere", { pos = pos, size = vector.create(6 * S, 0.02, 4 * S), color = "#ffffff", emissive = GLOW.white, solid = false, tag = "pilot" }))
		end
		p.halo = world.spawn(outline(24, 20), { pos = pos, size = 40 * S, color = p.color, emissive = GLOW.line, solid = false, visible = false, tag = "pilot" })
		p.mode = "up"
	end
	if p.mode ~= mode then
		local up = mode == "up"
		for _, id in p.hull do world.set(id, { visible = up }) end
		for _, id in p.engines do world.set(id, { visible = up }) end
		world.set(p.cockpit, { visible = up })
		world.set(p.halo, { visible = false }) -- the HUD's ring of dots shows a downed pilot
		p.mode = mode
	end
	for i, q in SHIP_HULL do
		local r = SHIP_HULL[i % #SHIP_HULL + 1]
		place_ship_line(p.hull[i], p, a, q[1], q[2], r[1], r[2])
		seen(p.hull[i])
	end
	local c, sn = math.cos(a), math.sin(a)
	world.place(p.cockpit, at(p.x + 5 * c, p.y + 5 * sn, 0.16), -a)
	seen(p.cockpit)
	for i, e in SHIP_ENGINES do
		-- Engines burn behind the hull, flickering.
		local back = e[1] - 3 - math.sin(s.time * 31 + i * 2 + p.id) * 1.5
		world.place(p.engines[i], at(p.x + back * c - e[2] * sn, p.y + back * sn + e[2] * c, 0.14), -a)
		seen(p.engines[i])
	end
	-- Active powers ride under the ship: the pickup's icon, small, shrinking
	-- further as that power's ammo runs out (no ring: that read as a pickup).
	p.badges = p.badges or {}
	local active = {}
	for _, kind in POWER_ORDER do
		if p.powers[kind] and mode ~= "down" then table.insert(active, kind) end
	end
	for i, kind in active do
		local color = rgb(PICKUP_COLORS[kind])
		local b = p.badges[kind]
		if not b or b.ring then
			b = { icon = {} }
			for _ in PICKUP_ICONS[kind] do
				table.insert(b.icon, world.spawn("cube", { pos = pos, size = vector.create(1.6 * S, 0.03, S), color = color, emissive = GLOW.line, solid = false, tag = "pilot" }))
			end
			p.badges[kind] = b
		end
		local bx, by = p.x + (i - (#active + 1) / 2) * 16, p.y + 34
		local k = 0.45 * (0.4 + 0.6 * p.powers[kind] / AMMO[kind])
		for j, seg in PICKUP_ICONS[kind] do
			local dx, dy = seg[3] - seg[1], seg[4] - seg[2]
			world.set(b.icon[j], {
				pos = at(bx + (seg[1] + seg[3]) / 2 * k, by + (seg[2] + seg[4]) / 2 * k, 0.16),
				rot = vector.create(0, math.atan2(-dx, -dy), 0),
				size = vector.create(1.6 * S, 0.03, math.max(1, len(dx, dy) * k) * S),
			})
			seen(b.icon[j])
		end
	end
	for kind in p.badges do
		if not p.powers[kind] or mode == "down" then p.badges[kind] = nil end
	end
	if mode == "down" then
		-- The ring fills in as the revive comes closer.
		local k = math.min(1, math.max(p.revive / REVIVE_TIME, p.downTime / 6))
		-- The ring sits where the pilot went down (it was left at its spawn point).
		world.set(p.halo, { pos = pos, size = vector.create((40 + 10 * k + math.sin(s.time * 5) * 4) * S, 0.04, (40 + 10 * k + math.sin(s.time * 5) * 4) * S) })
	end
	seen(p.halo)
end

local function show_hole()
	local fs = fields()
	local f = fs[1]
	if not f then
		if s.hole then
			for _, id in s.hole do world.remove(id) end
			s.hole = nil
		end
		return
	end
	local k = strength() * (0.3 + 0.7 * ramp())
	if not s.hole then
		s.hole = { world.spawn("sphere", { pos = at(f.x, f.y, 0.05), size = vector.create(f.r * 2 * S, 0.02, f.r * 2 * S), color = "#000000", roughness = 1, solid = false, tag = "hole" }) }
		table.insert(s.hole, world.spawn(outline(32, f.r + 4), { pos = at(f.x, f.y, 0.1), size = (f.r + 4) * 2 * S, color = rgb(COLORS.singularity), emissive = GLOW.white, solid = false, tag = "hole" }))
		for _ = 1, 12 do
			table.insert(s.hole, world.spawn("cube", { size = vector.create(0.3, 0.03, 0.04), color = rgb(COLORS.singularity), emissive = GLOW.line, solid = false, tag = "hole" }))
		end
	end
	-- Twelve spokes wheel in around the core.
	for i = 3, #s.hole do
		local j = i - 3
		local a = j * math.pi / 6 + s.time * 0.9
		local r = (34 + (math.sin(s.time * 3 + j) + 1) * 6) * S
		local p = at(f.x, f.y, 0.12) + vector.create(math.cos(a), 0, math.sin(a)) * (r * (0.4 + 0.6 * k))
		world.place(s.hole[i], p, -a + math.pi / 2)
	end
end

-- Deep space drifts past far below: three layers of stars, the nearer ones
-- bigger and faster, and the bright ones twinkle.
local CELL_COLORS = { { 0.9, 0.35, 0.6 }, { 0.5, 0.4, 1 }, { 0.3, 0.8, 0.75 } }
local STAR_LAYERS ={ { speed = 0.35, size = 0.035 }, { speed = 0.6, size = 0.05 }, { speed = 1, size = 0.075 } }

local function build_stars()
	s.stars = {}
	for i = 1, KNOBS.stars do
		local layer = STAR_LAYERS[i % 3 + 1]
		local big = i % 9 == 0
		local size = layer.size * (if big then 1.6 else random.range(0.8, 1.1))
		local x, z = random.range(-26, 26), random.range(-16, 16)
		local glow = if big then KNOBS.star_glow * 2.5 else KNOBS.star_glow
		local id = world.spawn("cube", { pos = vector.create(x, -10.5, z), size = vector.create(size, 0.01, size), color = if big then "#dfe8ff" else "#b8c8ff", emissive = glow, solid = false, tag = "stars" })
		table.insert(s.stars, { id = id, x = x, z = z, speed = layer.speed, glow = glow, big = big, phase = random.float() * 6.28 })
	end
end

local function drift_stars()
	-- After a hot reload: drop the old still stars and the coloured grid.
	if not s.stars then
		for _, id in world.find("space") do
			local e = world.get(id)
			if e and e.size.x < 1 then world.remove(id) end
		end
		build_stars()
	end
	if s.grid and #s.grid > 0 then
		for _, g in s.grid do world.remove(g.id) end
		s.grid = {}
	end
	local dt = game.dt()
	-- The drift slowly swings, so the field never moves in a straight line.
	local angle = math.sin(s.time * 0.05) * 0.6
	local dx, dz = -math.cos(angle) * KNOBS.star_drift * dt, math.sin(angle) * KNOBS.star_drift * dt
	for _, st in s.stars do
		st.x += dx * st.speed
		st.z += dz * st.speed
		if st.x < -26 then st.x += 52 elseif st.x > 26 then st.x -= 52 end
		if st.z < -16 then st.z += 32 elseif st.z > 16 then st.z -= 32 end
		world.place(st.id, vector.create(st.x, -10.5, st.z))
		if st.big then world.set(st.id, { emissive = st.glow * (0.6 + 0.4 * math.sin(s.time * 2.3 + st.phase)) }) end
	end
	-- Under the stars, a field of big faint cells drifts and breathes: a
	-- stand-in until the backdrop shader exists.
	if not s.cells then
		s.cells = {}
		for i = 1, KNOBS.cells do
			local r = random.range(1.4, 3.4)
			local c = CELL_COLORS[i % #CELL_COLORS + 1]
			local cell = { x = random.range(-24, 24), z = random.range(-14, 14), r = r, phase = random.float() * 6.28, spin = random.range(-0.15, 0.15), nx = random.range(-0.3, 0.3), nz = random.range(-0.3, 0.3) }
			cell.wall = world.spawn(outline(32, r * 40, 2.4), { size = r * 2, color = rgb(c), emissive = KNOBS.cell_glow, solid = false, tag = "cells" })
			cell.core = world.spawn(outline(24, r * 12, 1.2), { size = r * 0.6, color = rgb(c, 0.7), emissive = KNOBS.cell_glow, solid = false, tag = "cells" })
			table.insert(s.cells, cell)
		end
	end
	for _, cell in s.cells do
		cell.x += dx * 0.25
		cell.z += dz * 0.25
		if cell.x < -28 then cell.x += 56 elseif cell.x > 28 then cell.x -= 56 end
		if cell.z < -18 then cell.z += 36 elseif cell.z > 18 then cell.z -= 36 end
		local wobble = math.sin(s.time * 0.8 + cell.phase) * 0.08
		local d = cell.r * 2
		world.set(cell.wall, { pos = vector.create(cell.x, -11, cell.z), size = vector.create(d * (1 + wobble), 0.02, d * (1 - wobble)) })
		world.place(cell.core, vector.create(cell.x + cell.nx * cell.r, -10.95, cell.z + cell.nz * cell.r), s.time * cell.spin)
	end
end

local function sync()
	s.stamp += 1
	drift_stars()
	for _, p in s.players do show_pilot(p) end
	for _, e in s.enemies do
		if not e.dead then show_enemy(e) end
	end
	for _, b in s.shots do
		local yaw = math.atan2(-b.vx, -b.vy)
		local pos = at(b.x, b.y, 0.14)
		if not b.ent then
			local width = if b.special == "pierce" then 0.14 elseif b.special then 0.09 else 0.06
			b.ent = world.spawn("cube", { pos = pos, rot = vector.create(0, yaw, 0), size = vector.create(width, 0.03, if b.special == "pierce" then 0.7 else 0.5), color = rgb(shot_color(b)), emissive = GLOW.shot, solid = false, tag = "shot" })
		else
			world.place(b.ent, pos, yaw)
		end
		seen(b.ent)
	end
	for _, b in s.hostile do
		local pos = at(b.x, b.y, 0.14)
		if not b.ent then
			local k = b.size or 1
			b.ent = world.spawn("sphere", { pos = pos, size = vector.create(0.17 * k, 0.05 * k, 0.17 * k), color = rgb(HOSTILE_COLOR), emissive = GLOW.hostile, solid = false, tag = "hostile" })
		else
			world.place(b.ent, pos)
		end
		seen(b.ent)
	end
	for _, q in s.pickups do
		local pos = at(q.x, q.y + math.sin(s.time * 3 + q.x) * 2, 0.12)
		local pulse = 1 + 0.08 * math.sin(s.time * 6)
		-- A white ring sets pickups apart from the coloured enemies; the
		-- upgrade is the colour of the dot inside.
		-- (Rebuilt when an icon gains or loses lines in a live edit.)
		if not q.ring or #q.icon ~= #PICKUP_ICONS[q.kind] then
			q.ring = world.spawn(outline(24, 18), { pos = pos, size = 36 * S, color = rgb(WHITE), emissive = GLOW.white, solid = false, tag = "pickup" })
			q.icon = {}
			for _, seg in PICKUP_ICONS[q.kind] do
				local length = len(seg[3] - seg[1], seg[4] - seg[2])
				table.insert(q.icon, world.spawn("cube", { size = vector.create(2.4 * S, 0.03, length * S), color = rgb(PICKUP_COLORS[q.kind]), emissive = GLOW.line, solid = false, tag = "pickup" }))
			end
		end
		world.set(q.ring, { pos = pos, size = vector.create(36 * S * pulse, 0.04, 36 * S * pulse) })
		seen(q.ring)
		local bob = math.sin(s.time * 3 + q.x) * 2
		for i, seg in PICKUP_ICONS[q.kind] do
			local dx, dy = seg[3] - seg[1], seg[4] - seg[2]
			world.place(q.icon[i], at(q.x + (seg[1] + seg[3]) / 2, q.y + bob + (seg[2] + seg[4]) / 2, 0.13), math.atan2(-dx, -dy))
			seen(q.icon[i])
		end
	end
	for id, stamp in s.ents do
		if stamp ~= s.stamp then
			world.remove(id)
			s.ents[id] = nil
		end
	end
	show_hole()
	-- The grid takes the colour of each boss stage.
	local stage = math.clamp(math.ceil(math.max(1, s.wave) / 4), 1, 6)
	if stage ~= s.stage then
		s.stage = stage
		for _, g in s.grid do
			world.set(g.id, { color = rgb(STAGE_GRID[stage], if g.major then 0.6 else 0.4), emissive = if g.major then GLOW.major else GLOW.grid })
		end
	end
	-- Straight down on the arena; it shakes a little on hits and kills.
	local sx, sy = math.sin(s.time * 83) * s.shake * S, math.cos(s.time * 97) * s.shake * 0.6 * S
	for _, p in players.list() do
		camera.set(p.id, { pos = vector.create(sx, CAMERA_HEIGHT, sy + 0.05), target = vector.create(sx, 0, sy), fov = CAMERA_FOV })
		break
	end
end

----------------------------------------------------------------------------
-- The game

local function new_pilot(p, i)
	return {
		id = p.id,
		name = p.name,
		color = neon(p.color),
		rgb = { p.color.x, p.color.y, p.color.z },
		bot = p.bot,
		slot = i,
		x = s.width / 2 + (i - (players.count() + 1) / 2) * 65,
		y = s.height / 2 + 15,
		hp = s.lives,
		invul = 1.5,
		fireClock = 0,
		dash = 0,
		dashClock = 0,
		pulseClock = 0,
		aimX = 0,
		aimY = -1,
		powers = {},
		revive = 0,
		downTime = 0,
	}
end

-- Every look a game uses, prepared now so nothing stalls mid-wave.
local function warm(p)
	local c = neon(p.color)
	world.warm(outline(3, 22, 3.6), { color = c, emissive = GLOW.line })
	world.warm(outline(24, 20), { color = c, emissive = GLOW.line })
	world.warm("cube", { color = rgb({ p.color.x, p.color.y, p.color.z }), emissive = GLOW.shot })
	world.warm("cube", { color = c, emissive = FX_GLOW })
	world.warm("cube", { color = c, emissive = GLOW.line }) -- ship hull
end

local function warm_all()
	for kind, parts in SHAPES do
		for _, part in parts do
			for _, mode in { "normal", "warm", "flash" } do
				local color, glow = part_look(kind, part, mode)
				world.warm(part_shape(kind, part), { color = color, emissive = glow })
			end
		end
		world.warm("cube", { color = rgb(COLORS[kind]), emissive = FX_GLOW }) -- bursts and rings
		world.warm("cube", { color = rgb(COLORS[kind]), emissive = GLOW.line }) -- live beams
		world.warm("cube", { color = rgb(COLORS[kind], 0.7), emissive = GLOW.line * 0.5 }) -- warning beams
	end
	world.warm(outline(6, 12), { color = rgb(COLORS.hive), emissive = GLOW.line })
	world.warm(outline(6, 12), { color = rgb(WHITE), emissive = GLOW.white })
	world.warm("cube", { color = rgb(COLORS.dasher, 0.8), emissive = GLOW.line * 0.6 })
	world.warm("cube", { color = rgb(WHITE), emissive = GLOW.white })
	for _, c in { WHITE, { 0.75, 0.85, 1 }, { 0.8, 0.9, 1 }, { 1, 0.25, 0.3 }, { 0.4, 0.85, 1 }, { 0.3, 1, 0.8 }, { 0.3, 0.8, 1 }, { 0.5, 1, 0.7 } } do
		world.warm("cube", { color = rgb(c), emissive = FX_GLOW })
	end
	for _, c in SHOT_COLORS do world.warm("cube", { color = rgb(c), emissive = GLOW.shot }) end
	-- 3D wire germs: their coloured and white edges in every mode.
	for kind in MESHES do
		for _, mode in { "normal", "warm", "flash" } do
			for _, part in { {}, { white = true } } do
				local color, glow = part_look(kind, part, mode)
				world.warm("cube", { color = color, emissive = glow })
			end
		end
	end
	-- Spikes, flagella and legs share their body's look; knobs are outlines.
	for kind in SPIKES do
		for _, mode in { "normal", "warm", "flash" } do
			local color, glow = part_look(kind, SHAPES[kind][1], mode)
			world.warm("cube", { color = color, emissive = glow })
			world.warm(outline(16, 4), { color = color, emissive = glow })
		end
	end
	world.warm(outline(24, 18), { color = rgb(WHITE), emissive = GLOW.white })
	for _, c in PICKUP_COLORS do
		world.warm("cube", { color = rgb(c), emissive = GLOW.line })
		world.warm(outline(24, 14, 2), { color = rgb(c), emissive = GLOW.line }) -- power badges
	end
	for _, c in STAGE_GRID do
		world.warm("cube", { color = rgb(c, 0.4), emissive = GLOW.grid })
		world.warm("cube", { color = rgb(c, 0.6), emissive = GLOW.major })
	end
	world.warm("sphere", { color = rgb(HOSTILE_COLOR), emissive = GLOW.hostile })
	world.warm("sphere", { color = "#000000", roughness = 1 })
	world.warm("cube", { color = "#dfe8ff", emissive = KNOBS.star_glow * 2.5 })
	world.warm("cube", { color = "#b8c8ff", emissive = KNOBS.star_glow })
	world.warm(outline(32, 64), { color = rgb(COLORS.singularity), emissive = GLOW.white })
	world.warm("cube", { color = rgb(COLORS.singularity), emissive = GLOW.line })
	world.warm(outline(16, 4, 2), { color = rgb(WHITE), emissive = GLOW.white }) -- cockpit
	world.warm("sphere", { color = rgb(WHITE), emissive = GLOW.white }) -- engines
	for _, p in players.list() do warm(p) end
end

-- The arena as Geometry Wars draws it: a dark floor under a glowing grid,
-- a bright rim, and dust far below for depth.
local function arena()
	local w, h = s.width * S, s.height * S
	-- Deep space: black, with stars far below the grid. (Stacked discs for a
	-- nebula showed their rings, so for now it is only stars.)
	world.spawn("cube", { pos = vector.create(0, -12, 0), size = vector.create(w * 3, 0.2, h * 3), color = "#000000", roughness = 1, tag = "space" })
	build_stars()
	-- No grid: the coloured lines drew the eye away from the fight.
	s.grid, s.stage = {}, 1
end

-- Light and post: no sky light or haze, only what glows shows.
local function look()
	light.set({ sun_lux = 0, sky_lux = 0.05, fog_density = 0 })
	post.set({ bloom = KNOBS.bloom, bloom_radius = KNOBS.bloom_radius, bloom_threshold = KNOBS.bloom_threshold, bloom_wide = false, saturation = KNOBS.saturation, contrast = KNOBS.contrast, vignette = KNOBS.vignette, grain = KNOBS.grain, aberration = KNOBS.aberration })
end

local function init()
	state.s = {
		players = {},
		enemies = {},
		shots = {},
		hostile = {},
		pickups = {},
		sfx = {},
		ents = {},
		flashes = {},
		stamp = 0,
		uid = 0,
		rings = 0,
		time = 0,
		wave = START_WAVE - 1,
		lives = LIVES,
		width = WIDTH,
		height = HEIGHT,
		wavePhase = "rest",
		waveRest = 2,
		waveClock = 0,
		waveQueue = {},
		waveName = "SWEEP",
		teamScore = 0,
		chain = 0,
		comboTime = 0,
		kills = 0,
		shake = 0,
		wipe = 0,
		over = false,
		shotSound = 0,
	}
	s = state.s
	byid = {}
	warm_all()
	arena()
	-- Soft neon glow, a darker frame and a little film.
	look()
	-- Left to right as they sit on the couch.
	for i, p in players.by_couch() do
		table.insert(s.players, new_pilot(p, i))
	end
	sync()
end

-- The file changed (a knob or an edit): rebuild the arena and re-apply the
-- look, keeping the match going.
local function reload()
	s = state.s
	if not s then return end
	for _, tag in { "arena", "space" } do
		for _, id in world.find(tag) do world.remove(id) end
	end
	warm_all()
	arena()
	look()
	for _, e in s.enemies do e.mode = nil end
end

local SOUNDS = { "shot", "hit", "burst", "hurt", "death", "pulse", "pickup", "up_repair", "up_spread", "up_pierce", "up_rapid", "wave", "boss", "win" }

local function play_sounds()
	-- One of each per tick; shots at most every fourth tick (they repeat fast).
	s.shotSound -= 1
	for _, name in SOUNDS do
		local n = s.sfx[name]
		if n and (name ~= "shot" or s.shotSound <= 0) then
			if name == "shot" then s.shotSound = 4 end
			sound.play(name, { db = if name == "shot" then -8 elseif name == "hit" then -6 else 0, pitch = if name == "burst" then 0.9 + random.float() * 0.2 else 1 })
		end
	end
	s.sfx = {}
end

local function finish(won)
	s.over = true
	sfx(if won then "win" else "death")
	local text = if won then "Neon Siege: in remission, " .. s.teamScore .. " points" else "Neon Siege: wave " .. math.max(1, s.wave) .. ", " .. s.teamScore .. " points"
	game.log(text)
	-- A win also tells the host the session is done; either way A plays again.
	if won then game.finish() end
end

local function tick(dt, input)
	s = state.s
	if s.over then
		s.overClock = (s.overClock or 0) + dt
		-- The swarm drifts on aimlessly behind the end screen.
		s.time += dt
		for _, e in s.enemies do
			if not e.dead and e.kind ~= "segment" and e.kind ~= "node" then
				local a = e.phase + s.time * 0.5 + math.sin(s.time * 0.7 + e.phase * 3)
				local speed = math.min(60, SPECS[e.kind].speed * 0.4)
				e.x = math.clamp(e.x + math.cos(a) * speed * dt, e.r, s.width - e.r)
				e.y = math.clamp(e.y + math.sin(a) * speed * dt, e.r, s.height - e.r)
			end
		end
		if s.overClock >= 1 then
			for _, p in s.players do
				local i = not p.bot and input[p.id]
				if i and i.pressed.south then
					world.clear()
					init()
					return
				end
			end
		end
		sync()
		return
	end
	byid = {}
	for _, e in s.enemies do byid[e.uid] = e end
	s.time += dt
	s.rings = 0
	for i = #s.flashes, 1, -1 do
		local f = s.flashes[i]
		f.ttl -= dt
		if f.ttl <= 0 then table.remove(s.flashes, i) end
	end
	s.shake = math.max(0, s.shake - dt * 20)
	s.comboTime = math.max(0, s.comboTime - dt)
	if s.comboTime == 0 then s.chain = 0 end
	waves(dt)
	for _, p in s.players do
		pilot_step(p, if p.bot then bot(p) else controls(input[p.id]), dt)
	end
	enemies_step(dt)
	gravity_step(dt)
	shots_step(dt)
	pickups_step(dt)
	for i = #s.enemies, 1, -1 do
		if s.enemies[i].dead then
			s.enemies[i] = s.enemies[#s.enemies]
			s.enemies[#s.enemies] = nil
		end
	end
	local live = 0
	for _, p in s.players do
		if alive(p) then live += 1 end
	end
	if live == 0 and #s.players > 0 then s.wipe += dt else s.wipe = 0 end
	-- Alone there is no one to revive you: straight to the end screen after
	-- the ship has blown apart; together the others get two seconds.
	if s.wipe >= (if #s.players > 1 then 2 else 0.8) then finish(false) end
	if s.won and not s.over then
		s.victoryClock += dt
		if s.victoryClock >= 2.5 then finish(true) end
	end
	play_sounds()
	sync()
end

----------------------------------------------------------------------------
-- HUD

local function draw()
	s = state.s
	if not s then return end
	local w, h = hud.size()
	local k = h / 720
	local dim = { 0.55, 0.7, 0.85, 0.8 }
	-- Only the score, the multiplier and the wave, small in the corners.
	hud.text(tostring(s.teamScore), 20 * k, 12 * k, { size = 26 * k, color = { 0.85, 0.95, 1, 0.95 } })
	hud.text("x" .. multiplier(), 20 * k, 42 * k, { size = 16 * k, color = dim })
	-- A new wave's number is the same corner text, popping: it slides down
	-- into place, swells and glows, then settles back to its usual size and colour.
	local t = if s.waveAt then math.clamp((s.time - s.waveAt) / 1.2, 0, 1) else 1
	local pop = math.sin(t * math.pi)
	local settle = 1 - (1 - t) ^ 3
	local color = { dim[1] + (1 - dim[1]) * pop, dim[2] + (0.95 - dim[2]) * pop, dim[3] + (0.75 - dim[3]) * pop, dim[4] + (1 - dim[4]) * pop }
	hud.text(math.max(1, s.wave) .. "/" .. FINAL_WAVE, w - 220 * k, (12 - 24 * (1 - settle)) * k, { size = 20 * k * (1 + 0.6 * pop), align = "right", width = 200 * k, color = color })
	-- One thin bar for the boss, no name.
	local total, max = 0, 0
	local c
	for _, e in s.enemies do
		if SPECS[e.kind].boss and not e.dead and e.warm <= 0 then
			total, max = total + math.max(0, e.hp), max + e.maxHp
			c = c or COLORS[e.kind]
		end
	end
	if max > 0 then
		hud.box(w / 2 - 180 * k, 18 * k, 360 * k, 3 * k, { 0.15, 0.1, 0.2, 0.8 })
		hud.box(w / 2 - 180 * k, 18 * k, 360 * k * total / max, 3 * k, { c[1], c[2], c[3], 1 })
	end
	-- Lives as small dots under each pilot.
	for _, p in s.players do
		if p.hp > 0 then
			local x, y = hud.project(at(p.x, p.y, 0.15))
			if x then
				for i = 1, p.hp do
					hud.box(x + (i - (p.hp + 1) / 2) * 8 * k - 2 * k, y + 20 * k, 4 * k, 4 * k, p.color)
				end
			end
		end
	end
	-- A downed pilot is a ring of dots that fills while a teammate stays close.
	if not s.over then
		for _, p in s.players do
			if p.hp <= 0 and #s.players > 1 then
				local x, y = hud.project(at(p.x, p.y, 0.15))
				if x then
					-- A ring of dots fills clockwise while a teammate stays close.
					local done = math.floor(math.min(1, p.revive / REVIVE_TIME) * 24 + 0.5)
					for i = 1, 24 do
						local a = -math.pi / 2 + (i - 0.5) * math.pi / 12
						local lit = i <= done
						local d = (if lit then 5 else 3) * k
						hud.box(x + math.cos(a) * 30 * k - d / 2, y + math.sin(a) * 30 * k - d / 2, d, d, if lit then { p.rgb[1], p.rgb[2], p.rgb[3], 1 } else { 1, 1, 1, 0.18 })
					end
				end
			end
		end
	end
	if s.over or s.won then
		-- Dim the arena over a second and a half so the text reads.
		local fade = math.min(1, (s.overClock or 0) / 1.5)
		hud.box(0, 0, w, h, { 0, 0, 0, 0.55 * fade })
		hud.text(if s.won then "IN REMISSION" else "WIPED OUT", 0, h * 0.42, { size = 54 * k, align = "center", width = w })
		hud.text(tostring(s.teamScore), 0, h * 0.42 + 70 * k, { size = 26 * k, align = "center", width = w, color = { 0.85, 0.95, 1, 0.95 } })
		-- After a win too: the end screen always says how to go on.
		if (s.overClock or 0) >= 1 then
			hud.text("PRESS A TO PLAY AGAIN", 0, h * 0.42 + 120 * k, { size = 22 * k, align = "center", width = w, color = { 0.85, 0.95, 1, 0.6 + 0.3 * math.sin(s.overClock * 3) } })
		end
	end
end

-- Someone joins: a fresh pilot in the middle; the wave and score stay.
local function player_joined(p)
	s = state.s
	warm(p)
	local pilot = new_pilot(p, #s.players + 1)
	pilot.x, pilot.y = s.width / 2, s.height / 2
	pilot.invul = 3
	table.insert(s.players, pilot)
end

local function player_left(p)
	s = state.s
	for i, pilot in s.players do
		if pilot.id == p.id then
			table.remove(s.players, i)
			return
		end
	end
end

local function players_changed()
	s = state.s
	for _, p in players.list() do
		for _, pilot in s.players do
			if pilot.id == p.id then pilot.name = p.name end
		end
	end
end

-- The start menu shows what the buttons do, under its items.
local CONTROLS = {
	{ "Left stick", "Move" },
	{ "Right stick", "Aim and fire" },
	{ "LB", "Dash" },
	{ "RB", "Pulse: clears enemies and bullets nearby" },
}

local function menu(top)
	local w, h = hud.size()
	local k = h / 720
	local rowH = 30 * k
	local y = top + 36 * k
	local mid = w / 2
	hud.text("CONTROLS", 0, y, { size = 18 * k, align = "center", width = w, color = { 0.55, 0.8, 1, 0.85 } })
	y += 36 * k
	for _, row in CONTROLS do
		hud.text(row[1], mid - 320 * k, y, { size = 20 * k, align = "right", width = 300 * k, color = { 0.4, 0.95, 1, 1 } })
		hud.text(row[2], mid + 20 * k, y, { size = 20 * k, width = 360 * k, color = { 0.9, 0.93, 1, 0.95 } })
		y += rowH
	end
end

-- Helpers for the live console (run_luau, `lua`): debug.wave(12) jumps to
-- wave 12, debug.spawn("sun", 700, 300) drops an enemy, debug.lose() ends.
local debug = {
	wave = function(n)
		s = state.s
		s.wave, s.enemies, s.hostile, s.waveQueue = n - 1, {}, {}, {}
		s.wavePhase, s.waveRest = "rest", 0.01
		for _, p in s.players do p.hp, p.invul = s.lives, 3 end
		return "next: wave " .. n
	end,
	spawn = function(kind, x, y)
		s = state.s
		return spawn(kind, x or s.width / 2, y or s.height / 3, 0)
	end,
	lose = function()
		s = state.s
		finish(false)
		return "wiped out"
	end,
}

return { init = init, tick = tick, draw = draw, menu = menu, reload = reload, debug = debug, player_joined = player_joined, player_left = player_left, players_changed = players_changed }
