-- Dead Miles (working name): a co-op walk east through the end of the
-- world. One to four survivors cross a scrolling land, a biome a day
-- (suburb, forest, ruined city, farmland), and choose which buildings
-- to search on the way. The dead gather behind you, so lingering is the
-- risk. Melee weapons wear out, ammo is nearly gone, guns are loud, and
-- the nights are long: torches eat batteries. A hit leaves you injured
-- (you limp until a first aid kit), a second hit puts you down; a dead
-- survivor comes back in seconds as a ghost and then as a new survivor.
-- The run ends only when nobody is standing.
--
-- Controls: left stick moves, right stick aims, RB (or X) a quick hit, RT a
-- heavy one, charged while held (a haymaker with bare hands; with a loaded
-- gun RT fires), LT sprints (short; it costs stamina), A (hold) searches, picks
-- up, revives and talks to strangers, Y uses a first aid kit, LB switches
-- the torch. Keyboard: WASD, left mouse attacks, right mouse runs, Space
-- uses, F heals, Q torch.
--
-- Design: the "Zombie road-trip co-op: design" doc. The rules live here;
-- the land is lib/land, the people lib/figure, and the encounters
-- (traders, strangers, caches) are content files in content/encounters.

local figure = require("lib/figure")
local body = require("lib/body")
local land = require("lib/land")
local props = require("lib/props")
local dog = require("lib/dog")
local v = vector.create

-- Every encounter file in content/encounters, in name order.
local ENCOUNTERS = {}
for _, name in libraries("content/encounters") do
	local e = require(name)
	e.file = name
	table.insert(ENCOUNTERS, e)
end

-- @knobs
local KNOBS = {
	walk = 3.4, -- survivor jog, m/s (the everyday pace)
	stamina = 1, -- stamina scale: a swing costs 0.12 (fists) to 0.45 (axe) of an average person's
	breath = 0.3, -- stamina back per second while not swinging (of an average person's)
	run = 1.75, -- sprinting (hold LT) is this much faster than the jog
	sprint_cost = 0.33, -- stamina a second of sprinting costs (of an average person's: about 3 s flat out)
	blown = 0.3, -- after sprinting yourself empty, the stamina you need back before you can sprint again
	run_noise = 6, -- how far the dead hear you run, m
	car_noise = 9, -- how far they hear you rummage through a car, m
	limp = 0.72, -- share of that speed while injured
	shambler = 1.25, -- shambler speed, m/s
	runner = 4.4, -- runner speed, m/s
	start_ammo = 5, -- rounds at the start of a run
	start_food = 4, -- food at the start of a run
	start_battery = 100, -- torch battery at the start, percent
	drain = 0.22, -- battery percent per second for each lit torch
	pressure = 0, -- how fast the director's pressure rises by itself per second (noise raises it)
	peak = 12, -- seconds of a director peak (a horde from behind)
	calm = 25, -- seconds of calm after a peak
	downed = 8, -- seconds a downed survivor can wait for a revive
	ghost = 12, -- seconds as a ghost before a new survivor arrives
	revive = 2, -- seconds a teammate holds A to revive
	search = 1.2, -- seconds to search a container
	cam_height = 22, -- camera height over the group, m
	cam_back = 7, -- camera distance behind (south of) the group, m
	fov = 33, -- camera field of view, degrees
	torch = 2000, -- torch brightness, candela
	lamp = 1000, -- street light brightness, candela
	moon = 6, -- moonlight at night, lux
	night_ev = 5.1, -- exposure at night (EV100; lower is brighter)
	density = 0.55, -- zombie count multiplier
	toughness = 2, -- zombie health multiplier
	grab = 0.3, -- how much each zombie within reach slows you (0.3: two of them, half speed)
	day_alert = 1.8, -- how much further the dead see in daylight
	day_speed = 1.3, -- how much faster they move in daylight
	trap = 0, -- seconds inside a building before the dead outside close in on it (0: off; noise does it)
	turns = 0.55, -- chance the way turns north or south at a chunk
	encounters = 0.35, -- chance of an encounter in a chunk
	autopilot = 0, -- 1: bots drive the people too (demos, store clips, tests)
}
-- @end

local HZ = land.HALF_Z
local CHUNK = land.CHUNK
local RADIUS = 0.32 -- survivor and zombie collision radius, m
local LEASH = 13 -- how far a survivor may stray from the group, m
local BEHIND = 13 -- how far behind the group the left edge of the world is

local MELEE = {
	fists = { dmg = 0.6, range = 1.3, arc = 0.5, cd = 0.6, noise = 2, cleave = 1, knock = 2, stamina = 0.12 },
	pipe = { dmg = 1, range = 1.6, arc = 0.35, cd = 0.75, dur = 25, noise = 6, cleave = 2, knock = 3, stamina = 0.24 },
	bat = { dmg = 1.5, range = 1.75, arc = 0.25, cd = 0.9, dur = 30, noise = 6, cleave = 3, knock = 4, stamina = 0.32 },
	machete = { dmg = 2, range = 1.55, arc = 0.35, cd = 0.65, dur = 20, noise = 5, cleave = 2, knock = 2, stamina = 0.2 },
	axe = { dmg = 3, range = 1.75, arc = 0.3, cd = 1.2, dur = 15, noise = 7, cleave = 3, knock = 3.5, stamina = 0.45 },
}
local RANK = { fists = 0, pipe = 1, bat = 2, pistol = 2.5, machete = 3, shotgun = 3.5, axe = 4 }
local GUNS = {
	pistol = { dmg = 2, cd = 0.42, pellets = 1, spread = 0.025, range = 20, noise = 25 },
	shotgun = { dmg = 1.6, cd = 0.95, pellets = 5, spread = 0.14, range = 11, noise = 35, dur = 40 },
}
local HELD = {
	pipe = { len = 0.8, w = 0.05, color = "#8a8f96", metal = 0.8, roughness = 0.4 },
	bat = { len = 0.85, w = 0.08, color = "#a8784a" },
	machete = { len = 0.6, w = 0.06, color = "#c8ccd0", metal = 0.9, roughness = 0.3 },
	axe = { len = 0.75, w = 0.08, color = "#7a4a2a" },
	pistol = { len = 0.28, w = 0.07, color = "#2a2a2c", metal = 0.6, roughness = 0.5 },
	shotgun = { len = 0.85, w = 0.07, color = "#4a3a2c", metal = 0.3, roughness = 0.6 },
}

local ZOMBIES = {
	shambler = { hp = 3, sense = 8, reach = 1.35, windup = 1.0, strike = 0.25, recover = 1.0 },
	runner = { hp = 2, sense = 12, reach = 3.6, windup = 0.9, strike = 0.35, recover = 1.2, lunge = 8.5 },
}
local ZOMBIE_SKIN = { "#7d8f6a", "#8a9178", "#6f7f66", "#93977f" }
local ZOMBIE_SHIRTS = { "#5b5148", "#4d5560", "#6a4f45", "#56604a", "#6b6458", "#3f4650" }
local ZOMBIE_PANTS = { "#2f3238", "#3b3530", "#2a2f2a" }
local SKINS = { "#f1c9a5", "#d9a37e", "#b97d56", "#8d5a3a", "#5e3b26" }
local PANTS = { "#2b3a55", "#3a3a3a", "#4a3d2e", "#2e3f33" }
local STRANGER_SHIRTS = { "#7a7a6a", "#6a5a7a", "#8a6a4a", "#4a6a6a" }

-- The light through a day: a short grey morning, a long dusk and night.
-- Index by chunk within the day (0 is the camp).
-- The group walks by night: by day the dead are alert and quick. Each
-- leg starts at dusk, runs through the night and ends in a dash through
-- the first daylight to a shelter, where they sit out the day.
-- The leg starts in the blue hour: the night sky, with the last light
-- fading in the west (light_step), so there is no cut from a sunset sky.
local TIMES = { [1] = "night:clear", [2] = "night:clear", [3] = "night:clear", [4] = "night:clear", [5] = "night:clear", [6] = "sunrise:clear", [7] = "morning:clear", [0] = "morning:clear" }

-- What a container holds, by kind: { weight, item, min, max }.
local LOOT = {
	kitchen = { { 42, "food", 1, 2 }, { 10, "meds", 1, 1 }, { 8, "battery", 1, 1 }, { 8, "ammo", 1, 1 }, { 32, nil } },
	closet = { { 25, "weapon" }, { 14, "ammo", 1, 2 }, { 12, "meds", 1, 1 }, { 14, "battery", 1, 1 }, { 10, "food", 1, 1 }, { 25, nil } },
	-- Cars are picked over: the real supplies are indoors.
	car = { { 6, "food", 1, 1 }, { 14, "battery", 1, 1 }, { 4, "ammo", 1, 1 }, { 76, nil } },
	police = { { 60, "ammo", 2, 4 }, { 30, "shotgun" }, { 10, "meds", 1, 1 } },
	shop = { { 40, "food", 1, 3 }, { 15, "battery", 1, 1 }, { 45, nil } },
	office = { { 14, "meds", 1, 1 }, { 14, "battery", 1, 1 }, { 8, "ammo", 1, 1 }, { 10, "food", 1, 1 }, { 54, nil } },
	pharmacy = { { 55, "meds", 1, 2 }, { 45, nil } },
	cabin = { { 30, "food", 1, 2 }, { 25, "weapon" }, { 15, "ammo", 1, 2 }, { 30, nil } },
	barn = { { 35, "weapon" }, { 25, "food", 1, 2 }, { 40, nil } },
	hay = { { 15, "weapon" }, { 10, "food", 1, 1 }, { 75, nil } },
}
local ITEM_COLORS = { ammo = "#e0b84a", food = "#7fc25a", battery = "#7ab8e8", meds = "#f2f2f2" }
local BATTERY = 40 -- percent in one battery

local s -- the run: state.s
local warm_all -- defined with the hooks, used when the world is built
local grid = {} -- rect indices per 4 m cell, rebuilt from s.rects

----------------------------------------------------------------------------
-- Small helpers

local function flat(p) return v(p.x, 0, p.z) end
local function dist(a, b)
	local dx, dz = a.x - b.x, a.z - b.z
	return math.sqrt(dx * dx + dz * dz)
end
local function dir(yaw) return v(-math.sin(yaw), 0, -math.cos(yaw)) end
local function yaw_of(d) return math.atan2(-d.x, -d.z) end
local function count(name, n) s.counts[name] = (s.counts[name] or 0) + (n or 1) end
local function sfx(name, at, opts)
	opts = opts or {}
	if at then opts.at = v(at.x, 1, at.z) end
	sound.play(name, opts)
end
local function pick(list) return list[random.int(1, #list)] end
local function rumble(u, k, t)
	if not u.recruit then players.rumble(u.id, k, t) end
end

----------------------------------------------------------------------------
-- Collision against the land's boxes (a grid of 4 m cells)

local CELL = 4
local function cell_key(cx, cz) return (cx + 10000) * 1000 + (cz + 100) end

local function grid_add(i, r)
	for cx = math.floor((r.x0 - RADIUS) / CELL), math.floor((r.x1 + RADIUS) / CELL) do
		for cz = math.floor((r.z0 - RADIUS) / CELL), math.floor((r.z1 + RADIUS) / CELL) do
			local k = cell_key(cx, cz)
			local list = grid[k]
			if not list then
				list = {}
				grid[k] = list
			end
			table.insert(list, i)
		end
	end
end

local function build_grid()
	grid = {}
	for i, r in s.rects do grid_add(i, r) end
end

-- Where the path runs at x (it turns north or south now and then).
local function path_z(x) return land.center(s.zcs, x) end

local function collide(p, r)
	local x, z = p.x, p.z
	for _ = 1, 2 do
		local list = grid[cell_key(math.floor(x / CELL), math.floor(z / CELL))]
		if list then
			for _, i in list do
				local b = s.rects[i]
				local cx = math.clamp(x, b.x0, b.x1)
				local cz = math.clamp(z, b.z0, b.z1)
				local dx, dz = x - cx, z - cz
				local d2 = dx * dx + dz * dz
				if d2 < r * r then
					if d2 > 1e-8 then
						local d = math.sqrt(d2)
						x += dx / d * (r - d)
						z += dz / d * (r - d)
					else
						local l, rr, t, bt = x - b.x0, b.x1 - x, z - b.z0, b.z1 - z
						local m = math.min(l, rr, t, bt)
						if m == l then x = b.x0 - r elseif m == rr then x = b.x1 + r elseif m == t then z = b.z0 - r else z = b.z1 + r end
					end
				end
			end
		end
	end
	local lo, hi = land.zrange(s.zcs, x)
	z = math.clamp(z, lo + 0.5, hi - 0.5)
	return v(x, 0, z)
end

local function to_rect(p, b)
	local dx = math.max(b.x0 - p.x, 0, p.x - b.x1)
	local dz = math.max(b.z0 - p.z, 0, p.z - b.z1)
	return math.sqrt(dx * dx + dz * dz)
end

local function wall_distance(p, d, max)
	local hit = world.raycast(v(p.x, 1.1, p.z), d, max, { tag = "wall" })
	return if hit then hit.distance else max
end

-- Walkers find their way through buildings by their doors: the room a
-- point is in (0 outside), and the next point on the way to a goal.
local function room_of(b, p)
	if p.x < b.x0 or p.x > b.x1 or p.z < b.z0 or p.z > b.z1 then return 0 end
	for i, r in b.rooms do
		if p.x >= r.x0 and p.x <= r.x1 and p.z >= r.z0 and p.z <= r.z1 then return i end
	end
	return 0
end

local function first_door(b, from, to)
	-- Breadth first over rooms (0 is outside); returns the first door.
	local prev = { [from] = -1 }
	local via = {}
	local queue = { from }
	local head = 1
	while head <= #queue do
		local r = queue[head]
		head += 1
		if r == to then break end
		for _, d in b.doors do
			local n = if d.a == r then d.b elseif d.b == r then d.a else nil
			if n and prev[n] == nil then
				prev[n] = r
				via[n] = d
				table.insert(queue, n)
			end
		end
	end
	if prev[to] == nil then return nil end
	local r = to
	while prev[r] ~= from do
		if prev[r] == -1 then return nil end
		r = prev[r]
	end
	return via[r], r
end

-- Does the segment p..q cross the box (slab test)?
local function crosses(p, q, x0, x1, z0, z1)
	local t0, t1 = 0, 1
	local dx, dz = q.x - p.x, q.z - p.z
	for _, slab in { { p.x, dx, x0, x1 }, { p.z, dz, z0, z1 } } do
		local o, d, lo, hi = slab[1], slab[2], slab[3], slab[4]
		if math.abs(d) < 1e-6 then
			if o < lo or o > hi then return false end
		else
			local a, b = (lo - o) / d, (hi - o) / d
			if a > b then a, b = b, a end
			t0, t1 = math.max(t0, a), math.min(t1, b)
			if t0 > t1 then return false end
		end
	end
	return true
end

-- Outside a building with the goal on its far side: walk round a corner.
local function around(b, p, goal)
	local m = 0.7
	local best, bd = nil, math.huge
	for _, c in { v(b.x0 - m, 0, b.z0 - m), v(b.x1 + m, 0, b.z0 - m), v(b.x0 - m, 0, b.z1 + m), v(b.x1 + m, 0, b.z1 + m) } do
		if dist(p, c) > 0.3 and not crosses(p, c, b.x0 + 0.02, b.x1 - 0.02, b.z0 + 0.02, b.z1 - 0.02) then
			local d = dist(p, c) + dist(c, goal)
			if d < bd then best, bd = c, d end
		end
	end
	return best
end

local function waypoint(p, goal)
	local lo, hi = math.min(p.x, goal.x), math.max(p.x, goal.x)
	for _, b in s.buildings do
		if hi > b.x0 - 1 and lo < b.x1 + 1 then
			local a, g = room_of(b, p), room_of(b, goal)
			-- Keep a little room off the walls, unless already that close
			-- (pressed into a corner by a post): then only the wall counts.
			local m = 0.3
			if p.x > b.x0 - m and p.x < b.x1 + m and p.z > b.z0 - m and p.z < b.z1 + m then m = 0.02 end
			if a == 0 and g == 0 and crosses(p, goal, b.x0 - m, b.x1 + m, b.z0 - m, b.z1 + m) then
				local c = around(b, p, goal)
				if c then return c end
			end
			if a ~= g then
				local d = first_door(b, a, g)
				if d then
					local near, far = d.pa, d.pb
					if d.b == a then near, far = d.pb, d.pa end
					-- Once at the near side, or already in the doorway, go through.
					if dist(p, near) > 0.55 and dist(p, far) > dist(near, far) - 0.1 then return near end
					return far
				end
			end
		end
	end
	return goal
end

----------------------------------------------------------------------------
-- Survivors

local function standing(u) return u.state == "ok" end
local function in_body(u) return u.state == "ok" or u.state == "down" end
-- The nearest person (not a bot) in a body, for a bot to stay by.
local function nearest_person(p)
	local best, bd = nil, math.huge
	for _, o in s.survivors do
		if not o.bot and KNOBS.autopilot ~= 1 and in_body(o) and o.pos then
			local d = dist(o.pos, p)
			if d < bd then best, bd = o.pos, d end
		end
	end
	return best
end

-- One thing in the hand: a gun, a melee weapon, or nothing.
local function held_kind(u) return u.gun or (if u.melee ~= "fists" then u.melee else nil) end
local function hold_weapon(u)
	body.hold(u.fig, held_kind(u), HELD)
end

local function give_body(u, pos, look)
	u.pos = pos
	u.yaw = -math.pi / 2
	u.vel = v(0, 0, 0)
	u.injured = false
	u.state = "ok"
	u.t = 0
	u.melee, u.dur = "fists", nil
	u.gun, u.gun_dur = nil, nil
	-- Some people last longer than others.
	u.stamina_max = KNOBS.stamina * pick({ 0.75, 0.9, 1, 1, 1.15, 1.35 })
	u.stamina = u.stamina_max
	u.cd_melee, u.cd_gun, u.cd_shove, u.swing, u.shove_t = 0, 0, 0, 0, 0
	u.invul = 1
	u.search_t, u.revive_t = 0, 0
	u.torch = true
	u.skin = (look and look.skin) or pick(SKINS)
	u.pants = (look and look.pants) or pick(PANTS)
	u.fig = body.build(pos, u.yaw, { shirt = u.color, skin = u.skin, pants = u.pants }, "survivor")
	if u.ghost_ent then
		world.remove(u.ghost_ent)
		u.ghost_ent = nil
	end
end

local function new_survivor(p, i)
	local u = { id = p.id, name = p.name, color = p.color, bot = p.bot, lives = 3, aim = v(1, 0, 0) }
	local x = s.min_x + 4 + (i - 1) * 0.9
	give_body(u, collide(v(x, 0, path_z(x) + (i % 2 == 0 and 0.5 or -0.5)), RADIUS))
	-- The group has one pistol between them at the start.
	if i == 1 then u.melee, u.dur, u.gun = "fists", nil, "pistol" end
	hold_weapon(u)
	return u
end

local function popup(u, text, color)
	u.popup = { text = text, t = 1.8, color = color or "#ffffff" }
end

-- How to use what you just got, the first two times per person: a row of
-- { button, what } under the popup.
local function tip(u, key, row)
	if u.bot then return end
	u.tips = u.tips or {}
	u.tips[key] = (u.tips[key] or 0) + 1
	if u.tips[key] > 2 then return end
	u.tip = { row = row, t = 3.5 }
end

----------------------------------------------------------------------------
-- Zombies

-- Warp Humans go to the dead the camera can see (game.toml's zombie and
-- runner pools); the rest are box figures until they come near.
-- slack widens the test (a body leaves the pool only well out of view).
local function near_view(p, slack)
	slack = slack or 0
	if not s.cams or #s.cams == 0 then return true end
	for _, c in s.cams do
		if math.abs(p.x - c.x) < 26 + slack and math.abs(p.z - c.z) < 18 + slack then return true end
	end
	return false
end

-- How far p is from the nearest view's centre, on the ground.
local function view_dist(p)
	local best = math.huge
	for _, c in s.cams or {} do best = math.min(best, dist(v(p.x, 0, p.z), v(c.x, 0, c.z))) end
	return best
end

local function spawn_zombie(kind, pos, mode, goal)
	local z = {
		kind = kind,
		pos = collide(pos, RADIUS),
		yaw = random.range(-math.pi, math.pi),
		hp = ZOMBIES[kind].hp * KNOBS.toughness,
		state = mode or "idle",
		t = random.range(0, 3),
		goal = goal,
		knock = v(0, 0, 0),
		stuck = 0,
		side = 0,
	}
	local shirt = if kind == "runner" then "#6e3a36" else pick(ZOMBIE_SHIRTS)
	z.fig = body.build(z.pos, z.yaw, { shirt = shirt, skin = pick(ZOMBIE_SKIN), pants = pick(ZOMBIE_PANTS), eyes = "#1a1a1a" }, if near_view(z.pos) then kind else nil)
	z.pool = kind
	if z.state == "sleep" then body.lie(z.fig, z.pos, z.yaw, true) end
	table.insert(s.zombies, z)
	return z
end

local function wake(z, goal)
	if z.state == "sleep" then
		z.state = "rise"
		z.t = random.range(0.6, 1.2)
		z.goal = goal
		sfx("groan", z.pos, { pitch = random.range(0.8, 1.15) })
	elseif z.state == "idle" or z.state == "wander" or z.state == "investigate" then
		z.state = "investigate"
		z.goal = goal
	end
end

local function noise(pos, radius, amount)
	count("noises")
	-- The last loud thing: where a horde heads.
	if radius >= 6 then s.heard, s.heard_at = flat(pos), game.time() end
	for _, z in s.zombies do
		if z.state ~= "dead" then
			local d = dist(z.pos, pos)
			if d < radius and (z.state ~= "sleep" or d < radius * 0.6 or radius >= 20) then
				wake(z, flat(pos))
			end
		end
	end
	s.pressure = math.min(1.3, s.pressure + (amount or 0))
end

local NO_GLOW = { color = "#1a1a1a", emissive = 0 }

-- Blood on the ground: flat dark pools that stay (the oldest go first).
local BLOOD = { { color = "#4a0f0c", roughness = 0.25 }, { color = "#5e1410", roughness = 0.3 }, { color = "#36100c", roughness = 0.35 } }
local BLOOD_MAX = 90

local function blood_drop(at, r)
	s.blood = s.blood or {}
	local look = BLOOD[random.int(1, #BLOOD)]
	local id = world.spawn("sphere", { pos = v(at.x, 0.012 + #s.blood % 7 * 0.0012, at.z), size = v(r * random.range(0.8, 1.25), 0.012, r * random.range(0.8, 1.25)), rot = v(0, random.range(0, math.pi), 0), color = look.color, roughness = look.roughness, solid = false, tag = "deco" })
	table.insert(s.blood, id)
	if #s.blood > BLOOD_MAX then world.remove(table.remove(s.blood, 1)) end
end

-- A splat at `at`, flung along `away`: one pool and drops trailing off.
local function blood_splat(at, away, size)
	blood_drop(at + away * (0.2 * size), 0.28 * size)
	for i = 1, 2 + math.floor(size * 3) do
		local f = random.range(0.3, 1.4) * size
		local side = v(-away.z, 0, away.x) * random.range(-0.35, 0.35) * f
		blood_drop(at + away * (0.35 + f) + side, random.range(0.05, 0.12) * size)
	end
	fx.burst(v(at.x, 1.05, at.z), { color = "#6a1410", count = math.floor(6 + 8 * size), speed = 2.5 + 2 * size, life = 0.45, size = 0.07 + 0.03 * size, gravity = 12 })
end

local function kill_zombie(z, away, strength)
	z.state = "dead"
	z.t = 15
	away = away or v(1, 0, 0)
	body.eyes(z.fig, false, NO_GLOW)
	-- Thrown by the blow: limp, the way it was hit.
	body.ragdoll(z.fig, away * (2.2 + 1.6 * (strength or 1)))
	body.lie(z.fig, z.pos, z.yaw, true, true)
	blood_splat(z.pos, away, 1.6 + (strength or 1) * 0.4)
	fx.burst(v(z.pos.x, 0.8, z.pos.z), { color = "#5a1e18", count = 10, speed = 3, life = 0.5, size = 0.1, gravity = 9 })
	sfx("splat", z.pos, { pitch = random.range(0.85, 1.15) })
	count("kills")
	s.kills += 1
end

local function hurt_zombie(z, dmg, from, knock)
	if z.state == "dead" then return end
	z.hp -= dmg
	local away = z.pos - from
	local d = vector.magnitude(away)
	away = if d > 0.01 then away / d else v(1, 0, 0)
	local heavy = math.min(1.5, 0.45 + 0.35 * dmg)
	if z.hp <= 0 then
		kill_zombie(z, away, heavy)
		return
	end
	z.knock = away * (knock or 2)
	-- The blow carries through: the upper body snaps away and sways back.
	body.hit(z.fig, away * heavy)
	blood_splat(z.pos, away, 0.6 + 0.4 * heavy)
	if z.state == "sleep" then
		z.state = "rise"
		z.t = 0.5
	elseif z.state ~= "rise" then
		-- A hit cancels a wind-up: hitting first is always a defence.
		z.state = "stagger"
		z.t = 0.35
	end
	body.eyes(z.fig, false, NO_GLOW)
end

local function remove_zombie_body(z)
	if z.fig then
		body.remove(z.fig)
		z.fig = nil
	end
end

----------------------------------------------------------------------------
-- Encounters: content files describe who you meet and what you can do.

local function encounter_fits(e, where, biome, day)
	if e.where ~= where then return false end
	if (e.min_day or 1) > day then return false end
	if e.when and not e.when({ food = s.food, ammo = s.ammo, meds = s.meds, battery = s.battery, day = day, people = #s.survivors }) then return false end
	if e.biomes then
		local ok = false
		for _, b in e.biomes do
			if b == biome then ok = true end
		end
		if not ok then return false end
	end
	return true
end

local function choose_encounter(where, biome, day)
	local total = 0
	for _, e in ENCOUNTERS do
		if encounter_fits(e, where, biome, day) then total += e.weight or 1 end
	end
	if total <= 0 then return nil end
	local r = random.range(0, total)
	for _, e in ENCOUNTERS do
		if encounter_fits(e, where, biome, day) then
			r -= e.weight or 1
			if r <= 0 then return e end
		end
	end
	return nil
end

-- side: which side of the path a prop stands (1: +z, the far kerb); its
-- front faces the path. c: the chunk, which owns the prop.
local function place_stranger(e, pos, chunk, c, side)
	side = side or 1
	local yaw0 = if side == 1 then 0 else math.pi
	local st = { file = e.file, pos = pos, chunk = chunk, done = false, yaw = 0, pose = e.pose, nobody = e.nobody, reach = 1.6 }
	if e.prop then
		local pp = props.build(e.prop, pos, yaw0)
		for _, id in pp.ents do table.insert(c.ents, id) end
		table.insert(c.rects, pp.rect)
		table.insert(s.rects, pp.rect)
		grid_add(#s.rects, pp.rect)
		local foot = props.DEFS[e.prop].foot
		st.prop_at = pos
		-- Talk from the path side, or reach the prop from anywhere round it.
		local stand = e.stand or v(0, 0, -(foot[2] / 2 + 0.6))
		st.pos = pos + v(side * stand.x, 0, side * stand.z)
		st.lift = stand.y
		st.reach = if e.nobody then math.max(foot[1], foot[2]) / 2 + 1.2 else 1.6 + (stand.y or 0) * 0.5
		st.pos = collide(st.pos, RADIUS)
	end
	if not e.nobody then
		local look = e.look or {}
		local fpos = if st.lift then st.pos + v(0, st.lift, 0) else st.pos
		st.fig = body.build(fpos, random.range(-math.pi, math.pi), { shirt = look.shirt or pick(STRANGER_SHIRTS), skin = look.skin or pick(SKINS), pants = look.pants or pick(PANTS) }, e.pool or "stranger")
		if e.held then body.hold(st.fig, e.held, HELD) end
	end
	-- Whoever is with them: a child, a patient, the other side of a quarrel.
	st.with = {}
	for _, w in e.with or {} do
		local look = w.look or {}
		local at = v(w.at.x * side, 0, w.at.z * side)
		local f = body.build(st.pos + at, 0, { shirt = look.shirt or pick(STRANGER_SHIRTS), skin = look.skin or pick(SKINS), pants = look.pants or pick(PANTS) }, w.pool or "stranger")
		if w.held then body.hold(f, w.held, HELD) end
		table.insert(st.with, { fig = f, at = at, pose = w.pose })
	end
	if e.dog then
		st.dog = dog.build(st.pos + v(0.8, 0, 0), 0)
		st.reach = 2.2
	end
	if e.lure then st.lure = { t = math.huge, every = e.lure.every or 2.5, radius = e.lure.radius or 26, sound = e.lure.sound, next = 0 } end
	if e.light then st.light = { t = math.huge, color = e.light.color or "#ff9a4a", intensity = e.light.intensity or 700, range = e.light.range or 16 } end
	-- A horde asleep where the street opens up: walk, don't run.
	if e.sleepers then
		for _ = 1, random.int(e.sleepers[1], e.sleepers[2]) do
			local p = pos + v(random.range(-7, 7), 0, random.range(-3.2, 3.2))
			spawn_zombie(if random.chance(0.15) then "runner" else "shambler", p, "sleep")
		end
	end
	-- A hidden temperament, rolled once: what the content's results may depend on.
	if e.temperament then
		local total = 0
		for _, w in e.temperament do total += w end
		local r = random.range(0, total)
		-- Walk the names in a fixed order so the same seed picks the same way.
		local keys = {}
		for name in e.temperament do table.insert(keys, name) end
		table.sort(keys)
		for _, name in keys do
			r -= e.temperament[name]
			if r <= 0 and not st.temperament then st.temperament = name end
		end
		st.temperament = st.temperament or keys[#keys]
	end
	table.insert(s.strangers, st)
	return st
end

-- Everything a stranger brought along goes.
local function drop_stranger(st)
	if st.fig then body.remove(st.fig) end
	st.fig = nil
	for _, w in st.with or {} do body.remove(w.fig) end
	st.with = {}
	if st.dog then dog.remove(st.dog) end
	st.dog = nil
end

local function encounter_of(st)
	for _, e in ENCOUNTERS do
		if e.file == st.file then return e end
	end
	return nil
end

local function has(cost)
	if not cost then return true end
	for item, n in cost do
		if (s[item] or 0) < n then return false end
	end
	return true
end

local GLOW = "#f2e6b0"
local function place_weapon(kind, pos, dur)
	local w = HELD[kind]
	if not w then return end
	local ent = world.spawn("cube", { pos = v(pos.x, 0.06, pos.z), rot = v(0, random.range(0, math.pi), 0), size = v(w.w, w.w, w.len), color = w.color, metal = w.metal, roughness = w.roughness or 0.6, solid = false, tag = "pickup" })
	-- A faint glow under it, so it stands out a little in the dark.
	local ring = world.spawn("sphere", { pos = v(pos.x, 0.02, pos.z), size = v(0.75, 0.02, 0.75), color = GLOW, emissive = 0.35, roughness = 1, solid = false, tag = "pickup" })
	local d = dur or (if MELEE[kind] then MELEE[kind].dur else GUNS[kind].dur)
	table.insert(s.pickups, { kind = kind, pos = flat(pos), dur = d, ent = ent, ring = ring })
end

-- A stranger who joins: a recruit the bots' code drives, and the body a
-- player takes over when theirs dies.
local function recruit(st, u)
	s.recruits += 1
	local r = {
		id = 1000 + s.recruits,
		name = "Stranger",
		color = v(0.55, 0.55, 0.5),
		bot = true,
		recruit = true,
		lives = 1,
		aim = v(1, 0, 0),
	}
	give_body(r, st.pos)
	r.melee = (encounter_of(st) or {}).held or "fists"
	r.dur = MELEE[r.melee].dur
	hold_weapon(r)
	table.insert(s.survivors, r)
	drop_stranger(st)
	count("recruits")
	game.log("Someone joined the group")
end

----------------------------------------------------------------------------
-- Street events: what an encounter file can ask for beyond talk and
-- trade. The files stay data; the machinery is here.

-- The nearest person or survivor in a body to a point.
local function nearest_body(pos, people_only)
	local best, bd = nil, math.huge
	for _, u in s.survivors do
		if in_body(u) and u.pos and not (people_only and u.bot) then
			local d = dist(u.pos, pos)
			if d < bd then best, bd = u, d end
		end
	end
	return best, bd
end

-- The dead come for someone: n of them from a few metres off, awake.
local function call_dead(at, n, target)
	for _ = 1, n do
		local a = random.range(-math.pi, math.pi)
		local p = at + v(math.cos(a), 0, math.sin(a)) * random.range(3.5, 7)
		local kind = if random.chance(math.min(0.4, 0.08 * s.day)) then "runner" else "shambler"
		spawn_zombie(kind, p, "investigate", flat(target or at))
	end
end

local function adopt_dog(st)
	if not st.dog then return end
	if s.dog then dog.remove(s.dog.d) end
	s.dog = { d = st.dog, pos = st.dog.pos, bark = 0, check = 0 }
	st.dog = nil
	count("dogs")
end

-- Everything a result can do besides items: see README "Street events".
local function event_effects(u, st, res)
	if res.zombies then call_dead(st.pos, res.zombies, u.pos) end
	if res.lure then st.lure = { t = res.lure, every = st.lure and st.lure.every or 2.5, radius = 34, sound = res.lure_sound or (st.lure and st.lure.sound) or "alarm", next = 0 } end
	if res.quiet then st.lure = nil end
	if res.light then st.light = { t = res.light, color = "#ff9a4a", intensity = 700, range = 16 } end
	if res.power_cut then
		for _, ch in s.chunks do
			for _, l in ch.lamps or {} do
				if dist(l.pos, st.pos) < 70 and not l.off then
					l.off = true
					if l.bulb then world.set(l.bulb, { color = "#4a4640", emissive = 0 }) end
				end
			end
		end
		s.power_cut_x = st.pos.x + 70
	end
	if res.fog then s.fog_t = res.fog end
	if res.calm then
		s.dir = { mode = "calm", t = res.calm, spawn_t = 8 }
		s.pressure = 0
	end
	if res.bands then
		s.band_queue = s.band_queue or {}
		for _, b in res.bands do table.insert(s.band_queue, b) end
	end
	if res.stamina then
		for _, o in s.survivors do
			if in_body(o) and o.stamina_max then o.stamina = o.stamina_max * res.stamina end
		end
	end
	if res.bitten then
		-- The last one to join is who was bitten.
		local r = s.survivors[#s.survivors]
		if r and r.recruit then r.turns = res.bitten end
	end
	if res.dog then adopt_dog(st) end
	if res.dog_off and st.dog then st.dog_run = 3 end
	if res.escort then
		st.escort = { to = s.route + (res.escort.metres or 48), gain = res.escort.gain, text = res.escort.text }
		st.done = false
		st.following = true
	end
	if res.leave and st.fig then st.fleeing = 4 end
	-- A short cut: the group comes out further along the way.
	if res.skip then
		for _, o in s.survivors do
			if in_body(o) then
				local x = o.pos.x + res.skip
				o.pos = collide(v(x, 0, path_z(x) + (o.pos.z - path_z(o.pos.x)) * 0.3), RADIUS)
				if o.fig then body.place(o.fig, o.pos, o.yaw) end
			end
		end
		if s.dog then s.dog.pos = collide(s.dog.pos + v(res.skip, 0, 0), 0.25) end
		for _, o in s.strangers do
			if o.following then o.pos = collide(v(o.pos.x + res.skip, 0, path_z(o.pos.x + res.skip)), RADIUS) end
		end
	end
end

local function pick_result(st, results)
	local pool, total = {}, 0
	for _, r in results do
		if r.temperament == nil or r.temperament == st.temperament then
			table.insert(pool, r)
			total += r.weight or 1
		end
	end
	local roll = random.range(0, total)
	for _, r in pool do
		roll -= r.weight or 1
		if roll <= 0 then return r end
	end
	return pool[#pool]
end

local function apply_result(u, st, res)
	for item, n in res.gain or {} do
		s[item] = (s[item] or 0) + n
		count("gained_" .. item, n)
	end
	for item, n in res.lose or {} do s[item] = math.max(0, (s[item] or 0) - n) end
	if res.weapon then place_weapon(res.weapon, st.pos + v(0.8, 0, 0)) end
	if res.noise then noise(st.pos, res.noise, 0.1) end
	if res.injure and in_body(u) then u.injured = true end
	if res.heal then
		for _, o in s.survivors do o.injured = false end
	end
	if res.join then
		recruit(st, u)
	elseif res.hostile then
		-- They turn on you: a shove, and they run off with what they took.
		u.vel = (u.pos - st.pos) / math.max(0.1, dist(u.pos, st.pos)) * 5
		st.fleeing = 4
		sfx("shove", st.pos)
	end
	event_effects(u, st, res)
	if res.text then popup(u, res.text, res.color or "#e8e0c8") end
	count("encounter_choices")
end

local function choose(u, st, choice)
	local e = encounter_of(st)
	if not e then return end
	if not has(choice.cost) then
		popup(u, "not enough", "#ff8a6a")
		sfx("click", u.pos)
		return
	end
	for item, n in choice.cost or {} do s[item] -= n end
	-- Results may depend on the stranger's hidden temperament.
	local res = pick_result(st, choice.results or { choice })
	if not (choice.stay or (res and res.stay)) then st.done = true end
	if res then apply_result(u, st, res) end
	if e.on_choice then e.on_choice(st, choice, res) end
	sfx("pickup", st.pos, { pitch = 0.9 })
	u.talk = nil
end

----------------------------------------------------------------------------
-- The land, a chunk at a time

local function chunk_day(index)
	local cycle = land.DAY_CHUNKS + 1
	local day = (index - 1) // cycle + 1
	local pos = (index - 1) % cycle + 1
	return day, if pos <= land.DAY_CHUNKS then pos else 0
end

local BAND = {
	quiet = { inside = 0.4, outside = { 1, 3 }, sleep = 0.5 },
	busy = { inside = 0.75, outside = { 4, 8 }, sleep = 0.55 },
	horde = { inside = 1, outside = { 12, 20 }, sleep = 0.85 },
}

local function roll_band(day, index)
	if s.band_queue and #s.band_queue > 0 then return table.remove(s.band_queue, 1) end
	if index <= 2 then return "quiet" end
	local r = random.float()
	local horde = math.min(0.3, 0.06 + 0.04 * day)
	if r < horde then return "horde" end
	if r < horde + 0.45 then return "busy" end
	return "quiet"
end

local function populate(c, day, in_day)
	local band = BAND[c.band]
	local scale = KNOBS.density * (1 + 0.15 * (day - 1)) * (if in_day >= 5 then 1.3 else 1)
	local runners = math.min(0.35, 0.05 + 0.07 * (day - 1))
	for _, p in c.inside do
		if random.chance(math.min(1, band.inside * scale)) then
			spawn_zombie(if random.chance(runners) then "runner" else "shambler", p, "sleep")
		end
	end
	local n = math.floor(random.int(band.outside[1], band.outside[2]) * scale)
	for i = 1, n do
		local spot = c.outside[(i - 1) % #c.outside + 1]
		local p = spot + v(random.range(-3, 3), 0, random.range(-2, 2))
		local mode = if random.chance(band.sleep) then "sleep" else (if random.chance(0.5) then "idle" else "wander")
		spawn_zombie(if random.chance(runners) then "runner" else "shambler", p, mode)
	end
end

-- A free place for a prop of footprint foot ({length, depth}) at one
-- kerb of chunk c, or nil.
local function kerb_spot(c, foot, side)
	local w, d = foot[1], foot[2]
	local z = c.zc + side * (0.9 + d / 2)
	for _ = 1, 10 do
		local x = random.range(c.x0 + land.TURN + 1 + w / 2, c.x1 - 1 - w / 2)
		local free = true
		for _, r in c.rects do
			if x + w / 2 + 0.6 > r.x0 and x - w / 2 - 0.6 < r.x1 and z + d / 2 + 0.6 > r.z0 and z - d / 2 - 0.6 < r.z1 then
				free = false
				break
			end
		end
		if free then return v(x, 0, z) end
	end
	return nil
end

local function add_chunk()
	local index = s.next_chunk
	s.next_chunk += 1
	local day, in_day = chunk_day(index)
	-- Now and then the way turns north or south, never right after the
	-- start or into a camp.
	local prev = s.zcs[index - 1] or 0
	local zc = prev
	if index > 2 and in_day > 0 and in_day < 7 and random.chance(KNOBS.turns) then
		-- Long enough to feel like walking up or down the screen for a while.
		local dz = random.range(24, 54) * (if random.chance(0.5) then 1 else -1)
		if math.abs(prev + dz) > 160 then dz = -dz end
		zc = prev + dz
	end
	s.zcs[index] = zc
	local c = land.chunk(index, day, in_day, zc, prev)
	c.day, c.in_day = day, in_day
	c.band = roll_band(day, index)
	for _, r in c.rects do
		table.insert(s.rects, r)
		grid_add(#s.rects, r)
	end
	for _, k in c.containers do table.insert(s.containers, k) end
	for _, b in c.buildings do table.insert(s.buildings, b) end
	for _, b in c.blocks or {} do table.insert(s.blocks, b) end
	for _, w in c.weapons do place_weapon(w.kind, w.pos) end
	if c.camp then s.camp = c.camp end
	if index > 1 and in_day > 0 then populate(c, day, in_day) end
	-- An encounter: in a room of a building, on the path or at the camp.
	if index > 1 and (in_day == 0 or random.chance(KNOBS.encounters)) then
		local open = {}
		for _, b in c.buildings do
			if #b.rooms > 0 then table.insert(open, b) end
		end
		local where = if in_day == 0 then "camp" elseif #open > 0 and random.chance(0.6) then "room" else "path"
		local e = choose_encounter(where, c.biome, day)
		if e then
			local pos, side = nil, if random.chance(0.5) then 1 else -1
			if where == "room" then
				local b = pick(open)
				local r = b.rooms[random.int(1, #b.rooms)]
				pos = v((r.x0 + r.x1) / 2, 0, (r.z0 + r.z1) / 2)
				side = if (r.z0 + r.z1) / 2 > c.zc then 1 else -1
			elseif where == "camp" then
				pos = c.camp + v(2.5, 0, -2.8)
			elseif e.prop and not e.in_road then
				-- At the kerb, half on the road, where nothing else stands;
				-- the middle of the path stays open.
				pos = kerb_spot(c, props.DEFS[e.prop].foot, side)
				if not pos then side = -side pos = kerb_spot(c, props.DEFS[e.prop].foot, side) end
			else
				pos = v(random.range(c.x0 + land.TURN + 2, c.x1 - 4), 0, c.zc + random.range(-2, 2))
			end
			if pos then
				if not e.prop then pos = collide(pos, RADIUS) end
				place_stranger(e, pos, index, c, side)
				count("encounters")
			end
		end
	end
	table.insert(s.chunks, c)
end

local function drop_chunk(c)
	for _, id in c.ents do world.remove(id) end
	local gone = {}
	for _, r in c.rects do gone[r] = true end
	local keep = {}
	for _, r in s.rects do
		if not gone[r] then table.insert(keep, r) end
	end
	s.rects = keep
	build_grid()
	local function filter(list, test)
		local out = {}
		for _, x in list do
			if test(x) then table.insert(out, x) end
		end
		return out
	end
	s.containers = filter(s.containers, function(k) return k.x > c.x1 end)
	s.buildings = filter(s.buildings, function(b) return b.x1 > c.x1 end)
	s.blocks = filter(s.blocks, function(b) return b.x1 > c.x1 end)
	s.pickups = filter(s.pickups, function(p)
		if p.pos.x > c.x1 then return true end
		world.remove(p.ent)
		world.remove(p.ring)
		return false
	end)
	s.strangers = filter(s.strangers, function(st)
		if st.pos.x > c.x1 or st.escort then return true end
		drop_stranger(st)
		return false
	end)
end

local function stream_land()
	while (s.next_chunk - 1) * CHUNK < s.front + 60 do add_chunk() end
	while #s.chunks > 0 and s.chunks[1].x1 < s.min_x - 18 do
		drop_chunk(s.chunks[1])
		table.remove(s.chunks, 1)
	end
	-- The dead far behind the group are gone for good.
	if game.tick() % 30 == 0 then
		local keep = {}
		for _, z in s.zombies do
			if z.pos.x < s.min_x - 22 then
				remove_zombie_body(z)
			elseif z.state ~= "dead" or z.fig then
				table.insert(keep, z)
			end
		end
		s.zombies = keep
	end
end

----------------------------------------------------------------------------
-- The look of the hour

local function current_time()
	local index = math.max(1, math.floor((s.center or v(0, 0, 0)).x / CHUNK) + 1)
	local _, in_day = chunk_day(index)
	return TIMES[in_day], in_day
end

local function dark() return s.time_now ~= nil and s.time_now:sub(1, 5) == "night" end
-- Broad daylight: the dead see further and move faster.
local function daylight()
	local t = s.time_now or ""
	return t:sub(1, 7) == "morning" or t:sub(1, 4) == "noon" or t:sub(1, 9) == "afternoon"
end

local function look_of_hour()
	local t = current_time()
	if t == s.time_now then return end
	-- A new sky brings its own light: the exposure jumps with it rather
	-- than easing from the old one through a white or black flash.
	if s.time_now ~= nil then s.ev = nil end
	s.time_now = t
	sky.set(t)
	if dark() then
		-- A faint moon from the south-west, so the ruins still show
		-- away from the torches and lamps.
		-- The camera is over 20 m up: fog that starts nearer greys the whole
		-- view instead of hiding the distance. The dark limits sight.
		light.reset()
		light.set({ fog_start = 30, fog_end = 70, fog_density = 0 })
		post.set({ vignette = 0.6, grain = 0.14, saturation = 0.7, contrast = 1.12 })
	else
		light.reset()
		light.set({ fog_start = 34, fog_end = 90, fog_density = 0 })
		post.set({ vignette = 0.45, grain = 0.1, saturation = 0.72, contrast = 1.08 })
	end
end

-- Exposure and the last and first light, every tick, along the walk. A
-- fixed exposure that eases towards a curve: no metering, so nothing jumps
-- when a torch swings past the camera, and no cut when the sky changes.
local function lerp(a, b, t) return a + (b - a) * math.clamp(t, 0, 1) end
local function smooth(t) t = math.clamp(t, 0, 1) return t * t * (3 - 2 * t) end
local function light_step(dt)
	local x = (s.center or v(0, 0, 0)).x
	local index = math.max(1, math.floor(x / CHUNK) + 1)
	local _, in_day = chunk_day(index)
	local u = (if in_day == 0 then land.DAY_CHUNKS else in_day - 1) + (x % CHUNK) / CHUNK
	local ev, sun, lux
	if in_day >= 1 and in_day <= 5 then
		-- Blue hour fading to night over the first chunk and a half.
		local t = smooth(u / 1.5)
		ev = lerp(7.2, KNOBS.night_ev, t)
		lux = KNOBS.moon * (30 ^ (1 - t))
		local d = v(-0.9 + 0.55 * t, lerp(0.22, 0.8, t), lerp(0.25, 0.5, t))
		sun = d / vector.magnitude(d)
	elseif in_day == 6 then
		-- First light: the sunrise sky with its own sun and sky light, so
		-- its own exposure (a night exposure here washes the screen white).
		ev = lerp(10.6, 11, smooth(u - 5))
	else
		ev = lerp(12, 12.8, smooth((u - 6) * 2))
	end
	s.ev = if s.ev then s.ev + (ev - s.ev) * (1 - math.exp(-dt * 1.5)) else ev
	light.set({ exposure = s.ev })
	if lux then light.set({ sun_lux = lux }) end
	if sun then light.set({ sun = sun }) end
end

----------------------------------------------------------------------------
-- A run

local function new_run()
	-- Warp Humans outlive world.clear: give them back to their pools.
	if state.s then
		for _, list in { state.s.zombies, state.s.survivors, state.s.strangers, state.s.corpses or {} } do
			for _, o in list do
				if o.fig then body.remove(o.fig) end
			end
		end
	end
	world.clear()
	warm_all()
	local old = state.s
	state.s = {
		ammo = KNOBS.start_ammo,
		food = KNOBS.start_food,
		meds = 1,
		battery = KNOBS.start_battery,
		kills = 0,
		day = 1,
		survivors = {},
		zombies = {},
		pickups = {},
		strangers = {},
		corpses = {},
		blocks = {},
		chunks = {},
		rects = {},
		containers = {},
		buildings = {},
		counts = {},
		next_chunk = 1,
		zcs = {},
		min_x = 0,
		front = 0,
		route = 0, -- metres walked along the way, turns included
		start_x = 4,
		phase = "play",
		pressure = 0.1,
		dir = { mode = "build", t = 0, spawn_t = 8 },
		recruits = 0,
		best = (old and old.best) or 0,
		runs = ((old and old.runs) or 0) + 1,
	}
	s = state.s
	grid = {}
	stream_land()
	for i, p in players.list() do table.insert(s.survivors, new_survivor(p, i)) end
	s.time_now = nil
	s.ev = nil
	look_of_hour()
	light_step(0)
end

----------------------------------------------------------------------------
-- Intents: what a person's controller or a bot wants this tick

local function from_input(i)
	if not i then return { mx = 0, mz = 0, ax = 0, az = 0 } end
	return {
		mx = i.move_x, mz = -i.move_y, ax = i.aim_x, az = -i.aim_y,
		-- RB (or X): a quick hit. RT: a heavy one, charged while held
		-- (with a loaded gun RT fires instead).
		light = i.held.right_shoulder or i.held.west, heavy = i.held.right_trigger,
		use = i.held.south, use_pressed = i.pressed.south,
		med = i.pressed.north, torch = i.pressed.left_shoulder, run = i.held.left_trigger,
		up = i.pressed.up, down = i.pressed.down, back = i.pressed.east,
		stick_y = i.move_y,
	}
end

local function nearest_zombie(p, maxd, awake_only)
	local best, bd = nil, maxd
	for _, z in s.zombies do
		if z.state ~= "dead" and (not awake_only or z.state ~= "sleep") then
			local d = dist(z.pos, p)
			if d < bd then best, bd = z, d end
		end
	end
	return best, bd
end

local function steer(u, goal)
	local to = waypoint(u.pos, goal) - u.pos
	local d = math.sqrt(to.x * to.x + to.z * to.z)
	if d < 0.05 then return 0, 0 end
	local mx, mz = to.x / d, to.z / d
	if u.side_t and u.side_t > 0 then
		mx, mz = mx * 0.3 - mz * u.side_dir, mz * 0.3 + mx * u.side_dir
	end
	return mx, mz
end

-- A bot gives up on a target (a weapon, a cupboard) it has chased for
-- six seconds without getting there: it is behind a wall it cannot route.
local function chasing(u, target, dt)
	u.ignore = u.ignore or {}
	if u.chase ~= target then u.chase, u.chase_t = target, 0 end
	u.chase_t += dt
	if u.chase_t > 6 then
		u.ignore[target] = true
		u.chase = nil
		count("gave_up")
		return false
	end
	return true
end

-- A bot: revive, fight what is close, search what is near, walk on.
-- Is this container in a building the dead are closing in on?
local function trapped_at(c)
	for _, b in s.buildings do
		if b.closed_in and c.x > b.x0 and c.x < b.x1 and c.z > b.z0 and c.z < b.z1 then return true end
	end
	return false
end

local function bot_intent(u)
	local dt = 1 / 60
	local it = { mx = 0, mz = 0, ax = 0, az = 0 }
	local humans = {}
	for _, o in s.survivors do
		if not o.bot and KNOBS.autopilot ~= 1 and in_body(o) then table.insert(humans, o) end
	end
	if u.state == "ghost" then
		local friend = humans[1] or s.survivors[1]
		if friend and friend ~= u and friend.pos then it.mx, it.mz = steer(u, friend.pos + v(4, 0, 3)) end
		local z, d = nearest_zombie(u.pos, 6, true)
		it.use_pressed = z ~= nil and d < 6 and random.chance(0.05)
		return it
	end
	if u.state == "down" then return it end
	it.torch = dark() ~= u.torch and s.battery > 0

	-- A sidekick: stays near its person, a step behind, and lets them lead.
	local lead = nil
	if #humans > 0 then
		local h, hd = nil, math.huge
		for _, o in humans do
			if o ~= u then
				local d = dist(o.pos, u.pos)
				if d < hd then h, hd = o, d end
			end
		end
		lead = h
		if h and hd > 7 then
			it.mx, it.mz = steer(u, h.pos)
			-- Left behind: a short sprint to catch up, with breath to spare.
			it.run = hd > 10 and (u.stamina or 1) > 0.5 * (u.stamina_max or 1)
			return it
		end
	elseif s.center and dist(u.pos, s.center) > 9 and not nearest_zombie(u.pos, 2.5, true) then
		-- Only bots: keep the group within a few steps.
		it.mx, it.mz = steer(u, s.center)
		return it
	end

	-- Revive a downed teammate.
	for _, o in s.survivors do
		if o ~= u and o.state == "down" and dist(o.pos, u.pos) < 16 then
			if dist(o.pos, u.pos) < 1.0 then
				it.use = true
			else
				it.mx, it.mz = steer(u, o.pos)
			end
			local z, d = nearest_zombie(u.pos, 1.6, true)
			if z and d < 1.6 then
				local to = z.pos - u.pos
				it.ax, it.az = to.x, to.z
				it.melee = true
				it.use = false
			end
			return it
		end
	end

	if u.injured and s.meds > 0 then it.med = true end

	-- Fight what is close or in the way; walk away from the rest.
	local z, d = nearest_zombie(u.pos, if lead then 4.5 else 7, true)
	-- Not through a wall: one in the next room is no fight yet.
	if z and d > 0.6 then
		local to = z.pos - u.pos
		-- Low enough to catch fallen wall stubs, which still block.
		local hit = world.raycast(v(u.pos.x, 0.3, u.pos.z), to / d, d, { tag = "wall" })
		if hit and hit.distance < d - 0.3 then z = nil end
	end
	if z and d > 2.6 and z.pos.x < u.pos.x - 1 and z.kind ~= "runner" then z = nil end
	-- One it could not reach for a while (behind a wreck): leave it be
	-- until it comes within a swing.
	if z and u.ignore and u.ignore[z] and d > MELEE[u.melee].range + 0.4 then z = nil end
	if z then
		local to = z.pos - u.pos
		it.ax, it.az = to.x, to.z
		local w = MELEE[u.melee]
		local close = 0
		for _, o in s.zombies do
			if o.state ~= "dead" and o.state ~= "sleep" and dist(o.pos, u.pos) < 5 then close += 1 end
		end
		local gun = u.gun and s.ammo > 0
		local tired = (u.stamina or 1) < w.stamina * KNOBS.stamina
		if gun and (d < 2 or (z.kind == "runner" and d < 7) or (close >= 4 and s.ammo > 2)) then
			it.fire = true
		elseif d < w.range - 0.3 and not gun and not tired then
			it.melee = true
		elseif tired and d < 2.5 then
			-- Out of breath: step back from it.
			it.mx, it.mz = -to.x / d, -to.z / d
		elseif d < 4 and chasing(u, z, dt) then
			it.mx, it.mz = to.x / d, to.z / d
		end
		if close >= 6 and u.injured then
			-- Too many and hurt: keep walking east, away from them.
			it.mx, it.mz = steer(u, u.pos + v(8, 0, (path_z(u.pos.x) - u.pos.z) * 0.3))
			it.fire = u.gun ~= nil and s.ammo > 0
		end
		if it.melee or it.fire or it.shove or it.mx ~= 0 then return it end
	end

	-- A better weapon nearby.
	for _, p in s.pickups do
		if RANK[p.kind] > RANK[held_kind(u) or "fists"] and dist(p.pos, u.pos) < 9 and (not lead or dist(p.pos, lead.pos) < 6) and not (u.ignore and u.ignore[p]) and chasing(u, p, dt) then
			if dist(p.pos, u.pos) < 0.9 then it.use_pressed = true else it.mx, it.mz = steer(u, p.pos) end
			return it
		end
	end

	-- The nearest unsearched container near the group, nobody else on it,
	-- unless the dead are gathering (then walk on).
	if s.pressure < 0.85 then
		local best, bd = nil, 16
		for _, c in s.containers do
			if not c.searched and (c.claim == nil or c.claim == u.id) and c.x > s.min_x + 3 and not (u.ignore and u.ignore[c]) and not trapped_at(c)
				and (not lead or (dist(v(c.x, 0, c.z), lead.pos) < 6 and c.x < lead.pos.x + 2)) then
				local dd = dist(v(c.x, 0, c.z), u.pos)
				if dd < bd then best, bd = c, dd end
			end
		end
		if best and not chasing(u, best, dt) then
			best.claim = nil
			best = nil
		end
		if best then
			best.claim = u.id
			if to_rect(u.pos, best.rect) < 0.75 then
				u.chase_t = 0
				it.use = true
			else
				local cpos = v(best.x, 0, best.z)
				local toward = if best.stand then best.stand - cpos else v(0, 0, path_z(best.x) - best.z)
				local m = vector.magnitude(toward)
				local goal = if m > 0.01 then cpos + toward / m * 0.9 else cpos
				it.mx, it.mz = steer(u, goal)
			end
			return it
		end
	end

	-- Walk on: east along the path, or to the camp fire.
	if s.camp and s.camp.x > u.pos.x - 6 and s.camp.x < u.pos.x + 30 and (not lead or dist(lead.pos, s.camp) < 10) then
		it.mx, it.mz = steer(u, s.camp + v(if u.id % 2 == 0 then 1.2 else -1.2, 0, 1.2))
		return it
	end
	-- Where the way turns, go along the turning street first.
	local pz = path_z(u.pos.x)
	local ahead = v(u.pos.x + 6, 0, pz + (u.id % 3 - 1) * 1.2)
	if math.abs(u.pos.z - pz) > 2.2 then ahead = v(u.pos.x + 0.6, 0, pz + (u.id % 3 - 1) * 0.8) end
	if lead then
		-- A step or two behind the person, off to one side.
		local side = if u.id % 2 == 0 then 1.4 else -1.4
		ahead = v(lead.pos.x - 1.8, 0, lead.pos.z + side)
		if dist(u.pos, ahead) < 0.8 then return it end
	end
	it.mx, it.mz = steer(u, ahead)
	return it
end

----------------------------------------------------------------------------
-- Survivor actions

local function hurt(u, from)
	if u.invul > 0 or not in_body(u) then return end
	if u.state == "down" then
		u.t -= 2.5
		u.invul = 0.5
		sfx("hurt", u.pos, { pitch = 0.8 })
		return
	end
	u.invul = 0.8
	u.search_t, u.revive_t = 0, 0
	u.talk = nil
	local away = u.pos - from
	local d = vector.magnitude(away)
	if d > 0.01 then u.vel = away / d * 4 end
	rumble(u, 0.5, 0.25)
	sfx("hurt", u.pos)
	fx.burst(v(u.pos.x, 1.1, u.pos.z), { color = "#8a1a14", count = 6, speed = 2.5, life = 0.35, size = 0.08, gravity = 9 })
	if not u.injured then
		u.injured = true
		count("injuries")
		return
	end
	u.state = "down"
	u.t = KNOBS.downed
	u.revive_t = 0
	body.lie(u.fig, u.pos, u.yaw, true, true)
	sfx("down", u.pos)
	count("downs")
	game.log(u.name .. " is down")
end

local function drop_melee(u)
	local at = u.pos + v(random.range(-0.4, 0.4), 0, random.range(-0.4, 0.4))
	if u.gun then place_weapon(u.gun, at, u.gun_dur) end
	if u.melee ~= "fists" then place_weapon(u.melee, at, u.dur) end
	u.melee, u.dur, u.gun, u.gun_dur = "fists", nil, nil, nil
end

local function respawn(u)
	-- A new survivor walks in from the west edge.
	give_body(u, collide(v(s.min_x + 1, 0, path_z(s.min_x + 1) + random.range(-2, 2)), RADIUS))
	if random.chance(0.4) then u.melee, u.dur = "pipe", MELEE.pipe.dur end
	hold_weapon(u)
	count("respawns")
	u.invul = 2
	sfx("pickup", u.pos, { pitch = 0.7 })
	fx.ring(v(u.pos.x, 0.05, u.pos.z), { radius = 1.5, life = 0.5, color = u.color, emissive = 0.2 })
end

local function die(u)
	drop_melee(u)
	body.hold(u.fig, nil, HELD)
	body.tint(u.fig, figure.CORPSE)
	-- The body stays a while; a human goes back to its pool later.
	table.insert(s.corpses, { fig = u.fig, t = 20 })
	u.fig = nil
	count("deaths")
	if u.recruit then
		u.state = "gone"
		return
	end
	-- A player takes over a recruit's body if the group has one.
	for i, r in s.survivors do
		if r.recruit and standing(r) then
			u.pos, u.yaw, u.fig, u.melee, u.dur, u.injured = r.pos, r.yaw, r.fig, r.melee, r.dur, r.injured
			u.state, u.invul, u.t = "ok", 1.5, 0
			u.search_t, u.revive_t = 0, 0
			body.tint(u.fig, { color = u.color, roughness = 0.9 })
			table.remove(s.survivors, i)
			count("takeovers")
			sfx("revive", u.pos, { pitch = 0.8 })
			return
		end
	end
	local alone = #s.survivors == 1
	if alone then u.lives -= 1 end
	u.state = "ghost"
	u.t = if alone then 0.5 else KNOBS.ghost
	u.ghost_ent = world.spawn("sphere", { pos = v(u.pos.x, 1.4, u.pos.z), size = 0.55, color = "#cfe6ff", emissive = 0.35, solid = false, tag = "ghost" })
	u.cd_moan = 0
	sfx("moan", u.pos, { pitch = 1.3 })
end

-- A swing takes its time: a slow wind-up, the blow at the middle, a heavy
-- follow-through. The more out of breath, the slower and weaker it is; with
-- no breath left it is a feeble, dragging swing.
local IMPACT = 0.5
local CHARGE = 1 -- seconds to charge a heavy blow fully
local CHARGE_HOLD = 1 -- seconds it can be held at full charge

-- No weapon, heavy: a haymaker, slow and wide, knocks them flat back.
local HAYMAKER = { dmg = 1.4, range = 1.35, arc = 0.45, cd = 0.95, noise = 3, cleave = 1, knock = 4.5, stamina = 0.22 }

-- Each clip's moments: where the wind-up tops out and where the blow lands.
local CLIPS = { attack = { 0.22, 0.28 }, punch = { 0.17, 0.25 }, jab = { 0.15, 0.22 } }

local function swing_stats(u)
	if u.melee == "fists" and u.swing_kind == "heavy" then return HAYMAKER end
	return MELEE[u.melee]
end

local function swing_pose(u)
	if u.melee == "fists" then return if u.swing_kind == "heavy" then "punch" else "jab" end
	return "attack"
end

-- Swing progress p (0..1) to clip time: a slow rise to the top, a fast
-- blow through the landing, then the recovery.
local function swing_clip(p, pose)
	local c = CLIPS[pose] or CLIPS.attack
	local up, land = c[1], c[2]
	if p < 0.45 then
		local a = p / 0.45
		return up * a * a * (3 - 2 * a)
	elseif p < 0.55 then
		return up + (p - 0.45) / 0.1 * (land - up + 0.04)
	end
	return land + 0.04 + (p - 0.55) / 0.45 * (1 - land - 0.04)
end

local function tiredness(u)
	if not u.stamina_max then u.stamina_max = KNOBS.stamina u.stamina = u.stamina_max end
	return math.clamp(1 - u.stamina / u.stamina_max, 0, 1)
end

-- kind: "light" (RB: quick, cheap, light) or "heavy" (RT: slow, charged
-- while held at the top of the wind-up).
local function swing(u, kind)
	u.swing_kind = kind
	local w = swing_stats(u)
	local light = kind == "light"
	local tired = tiredness(u)
	local cost = w.stamina * KNOBS.stamina * (if light then 0.5 else 1)
	local winded = u.stamina < cost
	local len = w.cd * (if light then 0.55 else 1) * (1 + 1.6 * tired ^ 1.4)
	u.swing_power = 1 - 0.35 * tired
	if winded then
		if (u.winded_t or 0) <= 0 then
			popup(u, "out of breath", "#c8c0b4")
			u.winded_t = 1.2
		end
		len *= 1.5
		u.swing_power = 0.4
	end
	u.stamina = math.max(0, u.stamina - cost)
	u.breath_t = 0.7
	u.swing_len = math.min(3.2, len)
	u.cd_melee = u.swing_len
	u.swing = 0.0001
	u.swing_t = 0
	u.swing_landed = false
	u.hitstop = 0
	u.charging = not light and not winded
	u.charge, u.charge_full = 0, 0
	count(if light then "light_swings" else "heavy_swings")
	count("swings")
end

-- The blow lands: everyone in the arc now takes it.
local function swing_land(u)
	local w = swing_stats(u)
	local light = u.swing_kind == "light"
	local charge = u.charge or 0
	local power = (u.swing_power or 1) * (if light then 0.55 else 1 + charge)
	local knock = w.knock * (if light then 0.5 else 1 + 0.8 * charge)
	local cleave = if light then 1 else w.cleave
	local f = dir(u.yaw)
	local hits = {}
	for _, z in s.zombies do
		if z.state ~= "dead" then
			local to = z.pos - u.pos
			local d = math.sqrt(to.x * to.x + to.z * to.z)
			-- A little more reach than the wind-up promised: the blow
			-- follows through, so a step in during the wind-up still lands.
			if d < w.range + 0.25 and d > 0.01 and (to.x * f.x + to.z * f.z) / d > w.arc - 0.08 then
				table.insert(hits, { z = z, d = d })
			end
		end
	end
	table.sort(hits, function(a, b) return a.d < b.d end)
	if #hits == 0 then count("misses") end
	local hit = 0
	for i = 1, math.min(#hits, cleave) do
		hurt_zombie(hits[i].z, w.dmg * power, u.pos, knock * (0.5 + 0.5 * math.min(1, u.swing_power or 1)))
		hit += 1
		count("melee_hits")
	end
	sfx(if hit > 0 then "hit" else "swing", u.pos, { pitch = (if light then 1.15 else 0.85 - 0.12 * charge) * random.range(0.92, 1.05) })
	if hit > 0 then
		-- A beat of stillness on contact sells the weight.
		local heft = w.dmg * power
		u.hitstop = if light then 0.03 else 0.06 + 0.025 * heft
		u.shake = math.max(u.shake or 0, if light then 0.03 else 0.06 + 0.04 * heft)
		rumble(u, math.min(1, 0.2 + 0.15 * heft), if light then 0.06 else 0.14)
		noise(u.pos, w.noise, 0.005)
		if u.dur then
			u.dur -= 1
			if u.dur <= 0 then
				fx.burst(v(u.pos.x + f.x * 0.8, 1, u.pos.z + f.z * 0.8), { color = HELD[u.melee].color, count = 9, speed = 3.5, life = 0.5, size = 0.07, gravity = 9 })
				sfx("crack", u.pos)
				popup(u, u.melee .. " broke", "#ff9a7a")
				u.melee, u.dur = "fists", nil
				hold_weapon(u)
				count("breaks")
			end
		end
	end
end

-- Each tick of a swing: the hit-stop holds it, the blow lands at IMPACT.
local function swing_step(u, dt, held)
	if not u.swing_len or u.swing <= 0 or u.cd_melee <= 0 then return end
	if u.hitstop > 0 then
		u.hitstop -= dt
		u.cd_melee += dt
		return
	end
	if u.charging then
		-- A bot lets go at a third of a charge.
		if u.bot and u.charge >= 0.35 then held = false end
		local top = 0.45 * u.swing_len
		if not held or u.stamina <= 0 or u.charge_full > CHARGE_HOLD then
			u.charging = false
		elseif u.swing_len - u.cd_melee >= top then
			-- Held at the top of the wind-up: the blow gathers.
			u.cd_melee = u.swing_len - top
			u.breath_t = math.max(u.breath_t or 0, 0.7)
			u.stamina = math.max(0, u.stamina - dt * 0.15 * KNOBS.stamina)
			local was = u.charge
			u.charge = math.min(1, u.charge + dt / CHARGE)
			if u.charge >= 1 then
				if was < 1 then rumble(u, 0.25, 0.08) end
				u.charge_full += dt
			end
		end
	end
	u.swing_t = u.swing_len - u.cd_melee
	if not u.swing_landed and u.swing_t >= IMPACT * u.swing_len then
		u.swing_landed = true
		swing_land(u)
	end
end

local function shoot(u)
	local g = GUNS[u.gun]
	u.cd_gun = g.cd
	if s.ammo <= 0 then
		sfx("click", u.pos)
		count("dry")
		return
	end
	s.ammo -= 1
	count("shots")
	local base = u.yaw
	local f = dir(u.yaw)
	local best, bd = nil, g.range
	for _, z in s.zombies do
		if z.state ~= "dead" then
			local to = z.pos - u.pos
			local d = math.sqrt(to.x * to.x + to.z * to.z)
			if d < bd and d > 0.3 and (to.x * f.x + to.z * f.z) / d > 0.93 then best, bd = z, d end
		end
	end
	if best then base = yaw_of(best.pos - u.pos) end
	for _ = 1, g.pellets do
		local d = dir(base + random.range(-g.spread, g.spread))
		local maxd = wall_distance(u.pos, d, g.range)
		local target, td = nil, maxd
		for _, z in s.zombies do
			if z.state ~= "dead" then
				local to = z.pos - u.pos
				local along = to.x * d.x + to.z * d.z
				if along > 0 and along < td then
					local px, pz = to.x - d.x * along, to.z - d.z * along
					if px * px + pz * pz < 0.45 * 0.45 then target, td = z, along end
				end
			end
		end
		if target then hurt_zombie(target, g.dmg, u.pos, 3) end
		fx.burst(v(u.pos.x + d.x * td, 1.1, u.pos.z + d.z * td), { color = "#fff2c0", emissive = 0.6, count = 3, speed = 2, life = 0.12, size = 0.05 })
	end
	fx.burst(v(u.pos.x + f.x * 0.6, 1.2, u.pos.z + f.z * 0.6), { color = "#ffd27a", emissive = 1.2, count = 5, speed = 3, life = 0.08, size = 0.07 })
	-- The muzzle flash lights the street for a moment.
	u.flash = 0.06
	sfx(if u.gun == "shotgun" then "shotgun" else "gun", u.pos)
	rumble(u, 0.35, 0.1)
	noise(u.pos, g.noise, if u.gun == "shotgun" then 0.14 else 0.09)
	if u.gun_dur then
		u.gun_dur -= 1
		if u.gun_dur <= 0 then
			popup(u, "shotgun broke", "#ff9a7a")
			sfx("crack", u.pos)
			u.gun, u.gun_dur = nil, nil
			hold_weapon(u)
		end
	end
end

local function shove(u)
	u.cd_shove = 0.8
	u.shove_t = 0.2
	count("shoves")
	local f = dir(u.yaw)
	for _, z in s.zombies do
		if z.state ~= "dead" and z.state ~= "sleep" then
			local to = z.pos - u.pos
			local d = math.sqrt(to.x * to.x + to.z * to.z)
			if d < 1.8 and d > 0.01 and (to.x * f.x + to.z * f.z) / d > 0.2 then
				z.knock = to / d * 6
				z.state = "stagger"
				z.t = 0.8
				body.eyes(z.fig, false, NO_GLOW)
			end
		end
	end
	sfx("shove", u.pos)
	noise(u.pos, 3, 0)
end

local function roll(kind)
	local t = LOOT[kind] or LOOT.closet
	local total = 0
	for _, e in t do total += e[1] end
	local r = random.range(0, total)
	for _, e in t do
		r -= e[1]
		if r <= 0 then return e end
	end
	return t[#t]
end

local function search_done(u, c)
	c.searched = true
	-- A car is loud to go through: doors, glass, the boot. Cupboards less so.
	if c.kind == "car" or c.kind == "police" then noise(v(c.x, 0, c.z), KNOBS.car_noise, 0.05) else noise(v(c.x, 0, c.z), 3, 0) end
	world.set(c.id, { color = "#3b3631", roughness = 0.8, metal = 0, emissive = 0 })
	if c.reveal then
		for _, id in c.reveal do world.set(id, { visible = true }) end
	end
	count("searches")
	local e = roll(c.kind)
	local item = e[2]
	local at = v(c.x, 1.2, c.z)
	if item == nil then
		popup(u, "nothing", "#9a9a9a")
		sfx("search", at, { pitch = 0.8 })
		return
	end
	if item == "weapon" then
		local kinds = if s.day >= 2 then { "pipe", "bat", "machete", "axe", "pistol" } else { "pipe", "bat", "machete", "pipe" }
		place_weapon(pick(kinds), u.pos + (v(c.x, 0, c.z) - u.pos) * 0.5)
		popup(u, "a weapon", "#f2e6b0")
		tip(u, "take", { { "A", "pick it up" } })
	elseif item == "shotgun" then
		place_weapon("shotgun", u.pos + (v(c.x, 0, c.z) - u.pos) * 0.5)
		popup(u, "a shotgun", "#e0b84a")
		tip(u, "take", { { "A", "pick it up" } })
		count("shotguns")
	elseif item == "battery" then
		s.battery = math.min(100, s.battery + BATTERY)
		popup(u, "batteries", ITEM_COLORS.battery)
		tip(u, "battery", { { "LB", "torch on/off" } })
		count("found_battery")
	else
		local n = random.int(e[3], e[4])
		s[item] += n
		popup(u, "+" .. n .. " " .. (if item == "meds" then "first aid" else item), ITEM_COLORS[item])
		if item == "meds" then tip(u, "meds", { { "Y", "patch up when hurt" } })
		elseif item == "ammo" then tip(u, "ammo", { { "RT", "fire" }, { "RS", "aim" } })
		elseif item == "food" then tip(u, "food", { { "", "eaten at the camp fire" } }) end
		count("found_" .. item, n)
	end
	fx.burst(at, { color = ITEM_COLORS[item] or "#f2e6b0", emissive = 0.3, count = 6, speed = 1.5, life = 0.4, size = 0.08 })
	sfx("pickup", at)
end

local function standing_count()
	local n = 0
	for _, u in s.survivors do
		if standing(u) then n += 1 end
	end
	return n
end

-- Night camp: everyone at the fire. Eat, the dead wander off, a new day.
local function make_camp()
	count("camps")
	local mouths = 0
	for _, u in s.survivors do
		if u.state ~= "gone" then mouths += 1 end
	end
	local short = math.max(0, mouths - s.food)
	s.food = math.max(0, s.food - mouths)
	local hungry = 0
	for i = #s.survivors, 1, -1 do
		local u = s.survivors[i]
		if hungry < short and in_body(u) then
			u.injured = true
			hungry += 1
		end
	end
	s.camp_wait = 0
	for _, u in s.survivors do
		-- Stragglers find their way in by morning.
		if u.state == "ok" and dist(u.pos, s.camp) > 4.5 then
			u.pos = s.camp + v(random.range(-2.5, 2.5), 0, random.range(1.5, 3))
			body.place(u.fig, u.pos, u.yaw)
		end
		if u.state == "down" then
			u.state = "ok"
			u.injured = true
			body.lie(u.fig, u.pos, u.yaw, false)
		elseif u.state == "ghost" then
			u.t = 0
		end
	end
	for _, z in s.zombies do
		if z.state ~= "dead" and z.state ~= "sleep" and math.abs(z.pos.x - s.camp.x) < 45 then
			remove_zombie_body(z)
			z.state = "dead"
			z.t = 0
		end
	end
	s.phase = "camp"
	s.camp_t = 4
	s.camp_short = short
	s.pressure = 0
	s.dir = { mode = "calm", t = 20, spawn_t = 8 }
	game.log("Dead Miles: shelter " .. s.day .. ", " .. math.floor(s.route) .. " m, " .. s.food .. " food left")
	s.day += 1
	s.camp_used = s.camp
	s.camp = nil
	sfx("revive", s.camp_used, { pitch = 0.7 })
end

local function nearest_stranger(u)
	for _, st in s.strangers do
		if (st.fig or st.nobody) and not st.done and not st.fleeing and not st.following and dist(st.pos, u.pos) < st.reach then
			local e = encounter_of(st)
			if e and not (e.auto and not e.choices) then return st end
		end
	end
	return nil
end

local function act(u, it, dt)
	u.invul = math.max(0, u.invul - dt)
	u.cd_melee = math.max(0, u.cd_melee - dt)
	u.shake = math.max(0, (u.shake or 0) - dt * 0.6)
	swing_step(u, dt, it.heavy)
	u.cd_gun = math.max(0, u.cd_gun - dt)
	u.cd_shove = math.max(0, u.cd_shove - dt)
	u.shove_t = math.max(0, u.shove_t - dt)
	if u.flash then
		u.flash -= dt
		if u.flash <= 0 then u.flash = nil end
	end
	if u.side_t then u.side_t -= dt end
	if u.tip then
		u.tip.t -= dt
		if u.tip.t <= 0 then u.tip = nil end
	end
	if u.popup then
		u.popup.t -= dt
		if u.popup.t <= 0 then u.popup = nil end
	end
	if u.state == "gone" then return end

	if u.state == "ghost" then
		u.t -= dt
		u.cd_moan = math.max(0, (u.cd_moan or 0) - dt)
		u.pos = u.pos + v(it.mx, 0, it.mz) * 5 * dt
		local lo, hi = land.zrange(s.zcs, u.pos.x)
		u.pos = v(math.max(u.pos.x, s.min_x), 0, math.clamp(u.pos.z, lo, hi))
		world.place(u.ghost_ent, v(u.pos.x, 1.4 + math.sin(game.time() * 3) * 0.15, u.pos.z))
		if it.use_pressed and u.cd_moan <= 0 then
			u.cd_moan = 3
			sfx("moan", u.pos, { pitch = random.range(0.9, 1.1) })
			noise(u.pos, 9, 0)
			count("moans")
		end
		if u.t <= 0 and (u.lives > 0 or #s.survivors > 1) then respawn(u) end
		return
	end

	local mv = v(it.mx, 0, it.mz)
	local m = vector.magnitude(mv)
	if m > 1 then mv = mv / m end

	if u.state == "down" then
		u.t -= dt
		u.pos = collide(u.pos + mv * 0.6 * dt, RADIUS)
		u.pos = v(math.max(u.pos.x, s.min_x), 0, u.pos.z)
		body.place(u.fig, u.pos, u.yaw)
		body.fall(u.fig, dt)
		if u.t <= 0 then die(u) end
		return
	end

	-- Talking to a stranger: the stick picks a choice, A takes it, B walks away.
	if u.talk then
		local st = u.talk
		local e = encounter_of(st)
		if not e or st.done or dist(st.pos, u.pos) > st.reach + 1 then
			u.talk = nil
		else
			local n = #e.choices
			if it.up then u.talk_i = (u.talk_i - 2) % n + 1 end
			if it.down then u.talk_i = u.talk_i % n + 1 end
			if it.use_pressed and (u.talk_age or 0) > 0.25 then
				choose(u, st, e.choices[u.talk_i])
			elseif it.back then
				u.talk = nil
			end
			u.talk_age = (u.talk_age or 0) + dt
			body.place(u.fig, u.pos, u.yaw)
			body.animate(u.fig, dt, 0, "swing")
			return
		end
	end

	local am = it.ax * it.ax + it.az * it.az
	if am > 0.09 then
		u.aim = v(it.ax, 0, it.az)
	elseif m > 0.2 then
		u.aim = mv
	end
	local want = yaw_of(u.aim)
	local dy = (want - u.yaw + math.pi) % (2 * math.pi) - math.pi
	u.yaw += math.clamp(dy, -14 * dt, 14 * dt)

	if it.torch then
		u.torch = not u.torch
		sfx("click", u.pos, { pitch = 1.6 })
	end

	local busy = false
	if it.use then
		for _, o in s.survivors do
			if o ~= u and o.state == "down" and dist(o.pos, u.pos) < 1.3 then
				busy = true
				u.revive_t += dt
				if u.revive_t >= KNOBS.revive then
					o.state = "ok"
					o.injured = true
					o.invul = 1.5
					body.lie(o.fig, o.pos, o.yaw, false)
					u.revive_t = 0
					sfx("revive", o.pos)
					count("revives")
				end
				break
			end
		end
		if not busy then
			local best, bd = nil, 0.85
			for _, c in s.containers do
				if not c.searched then
					local d = to_rect(u.pos, c.rect)
					if d < bd then best, bd = c, d end
				end
			end
			if best then
				busy = true
				u.search_t += dt
				if u.search_t >= KNOBS.search then
					u.search_t = 0
					search_done(u, best)
				end
			end
		end
	end
	if not busy then u.search_t, u.revive_t = 0, 0 end

	if it.use_pressed and not busy then
		local st = nearest_stranger(u)
		if st and not u.bot then
			u.talk, u.talk_i, u.talk_age = st, 1, 0
			sfx("search", st.pos, { pitch = 1.3 })
			count("talks")
			return
		elseif st and u.bot and KNOBS.autopilot == 1 then
			-- Autopilot bots take the first choice they can pay for.
			local e = encounter_of(st)
			for _, ch in e.choices do
				if has(ch.cost) then
					choose(u, st, ch)
					break
				end
			end
		end
		for i, p in s.pickups do
			if dist(p.pos, u.pos) < 1.1 then
				world.remove(p.ent)
				world.remove(p.ring)
				table.remove(s.pickups, i)
				-- One thing in the hand: what you held goes down here.
				drop_melee(u)
				if GUNS[p.kind] then
					u.gun, u.gun_dur = p.kind, p.dur
					tip(u, "gun", { { "RT", "fire" }, { "RS", "aim" }, { "RB", "hit" } })
				else
					u.melee, u.dur = p.kind, p.dur
					tip(u, "melee", { { "RB", "quick hit" }, { "RT", "heavy (hold: charge)" } })
				end
				hold_weapon(u)
				sfx("pickup", u.pos, { pitch = 1.2 })
				count("pickups")
				break
			end
		end
	end

	if it.med and s.meds > 0 and u.injured then
		s.meds -= 1
		u.injured = false
		sfx("revive", u.pos, { pitch = 1.3 })
		popup(u, "patched up", ITEM_COLORS.meds)
		count("meds_used")
	end

	-- One attack: fire what is in the hand, or swing it (a gun without
	-- rounds is swung like a fist). Bots ask with fire or melee.
	if u.bot and it.melee then
		-- Bots: a charged heavy on a lone one with breath to spare, else quick hits.
		-- A quick hit to break one's wind-up (any hit does).
		local near, threat = 0, false
		for _, z in s.zombies do
			if z.state ~= "dead" and z.state ~= "sleep" and dist(z.pos, u.pos) < 3.5 then
				near += 1
				threat = threat or z.state == "windup"
			end
		end
		if u.charging or (near <= 1 and not threat and u.stamina > 0.6 * u.stamina_max) then it.heavy = true else it.light = true end
	end
	if not busy then
		local loaded = u.gun and s.ammo > 0
		if loaded and (it.heavy or it.fire) then
			if u.cd_gun <= 0 then shoot(u) end
		elseif u.cd_melee <= 0 and (it.heavy or it.light) then
			-- A new heavy needs a fresh press: holding RT after one does not
			-- start another.
			if it.heavy and not u.heavy_was then swing(u, "heavy")
			elseif it.light then swing(u, "light") end
		end
	end
	u.heavy_was = it.heavy and not (u.bot)
	u.winded_t = math.max(0, (u.winded_t or 0) - dt)
	u.breath_t = math.max(0, (u.breath_t or 0) - dt)

	-- Hands on you: every one of the dead within reach drags at you.
	local grabs = 0
	for _, z in s.zombies do
		if z.state ~= "dead" and z.state ~= "sleep" then
			local dx, dz = z.pos.x - u.pos.x, z.pos.z - u.pos.z
			if dx * dx + dz * dz < 1.6 * 1.6 then grabs += 1 end
		end
	end
	if not u.stamina_max then u.stamina_max = KNOBS.stamina u.stamina = u.stamina_max end
	if not u.bot and grabs == 0 and (u.tips == nil or (u.tips.sprint or 0) < 2) then
		for _, z in s.zombies do
			if z.kind == "runner" and z.state == "chase" and dist(z.pos, u.pos) < 12 then
				tip(u, "sprint", { { "LT", "sprint (short, tiring)" } })
				break
			end
		end
	end
	-- Sprinting is short: it burns stamina fast, and once you are blown
	-- you jog until you have your breath back.
	if u.blown and u.stamina >= KNOBS.blown * u.stamina_max then u.blown = nil end
	local running = it.run and not busy and not u.injured and m > 0.5 and not u.blown and u.cd_melee <= 0
	if running then
		u.stamina = math.max(0, u.stamina - dt * KNOBS.sprint_cost * KNOBS.stamina)
		u.breath_t = math.max(u.breath_t, 0.5)
		if u.stamina <= 0 then
			u.blown = true
			if (u.winded_t or 0) <= 0 then
				popup(u, "out of breath", "#c8c0b4")
				u.winded_t = 1.2
			end
			sfx("swing", u.pos, { pitch = 0.55, db = -10 })
		end
	elseif u.breath_t <= 0 then
		u.stamina = math.min(u.stamina_max, u.stamina + dt * KNOBS.breath * KNOBS.stamina)
	end
	-- Worn out, even the jog drags.
	local worn = 1 - 0.25 * math.clamp((tiredness(u) - 0.6) / 0.4, 0, 1)
	local speed = KNOBS.walk * worn * (if busy then 0 else (if u.charging then 0.35 elseif u.cd_melee > 0.2 then 0.5 else 1)) * (if u.injured then KNOBS.limp else 1) / (1 + KNOBS.grab * grabs) * (if running then KNOBS.run else 1)
	-- Running is heard: footfalls every so often.
	u.run_t = (u.run_t or 0) - dt
	if running and u.run_t <= 0 then
		u.run_t = 0.5
		noise(u.pos, KNOBS.run_noise, 0.004)
	end
	local go = mv * speed + u.vel
	u.vel = u.vel * math.max(0, 1 - 10 * dt)
	local np = u.pos + go * dt
	-- Bots keep near the people; people go where they like (the screen
	-- splits when they walk apart).
	local anchor = if u.bot then (nearest_person(u.pos) or s.center) else nil
	if anchor then
		local away = np - anchor
		local d = math.sqrt(away.x * away.x + away.z * away.z)
		if d > LEASH and d > dist(u.pos, anchor) then
			np = anchor + away / d * math.max(LEASH, dist(u.pos, anchor))
		end
	end
	-- No way back west: the land behind is gone.
	np = v(math.max(np.x, s.min_x), 0, np.z)
	local before = u.pos
	u.pos = collide(np, RADIUS)
	if u.bot and m > 0.5 and dist(before, u.pos) < speed * dt * 0.2 then
		u.stuck = (u.stuck or 0) + dt
		if u.stuck > 0.5 then
			-- Stuck again soon after a sidestep: keep going the same way and
			-- for longer, to get round a long hedge or a wreck.
			local again = u.side_at and game.tick() - u.side_at < 240
			u.side_len = if again then math.min(4, (u.side_len or 0.8) * 2) else 0.8
			if not again then u.side_dir = if random.chance(0.5) then 1 else -1 end
			u.side_t, u.stuck, u.side_at = u.side_len, 0, game.tick()
		end
	else
		u.stuck = 0
	end

	body.place(u.fig, u.pos, u.yaw)
	local moved = dist(before, u.pos) / dt
	-- Out of breath shows: bent over and heaving once half the breath is gone.
	local tired = tiredness(u)
	body.tired(u.fig, if tired > 0.4 then math.min(1, (tired - 0.4) / 0.55) ^ 1.2 else 0)
	local coil = 0
	if u.cd_melee > 0 and u.swing > 0 and u.swing_len then
		local p = math.clamp(1 - u.cd_melee / u.swing_len, 0, 1)
		local pose = swing_pose(u)
		body.animate(u.fig, dt, moved, pose, swing_clip(p, pose))
		-- A heavy blow winds the body up (more with the charge) and swings through.
		if u.swing_kind == "heavy" then
			local wind = 0.45 + 0.55 * (u.charge or 0)
			if p < 0.45 then
				local a = p / 0.45
				coil = wind * a * a * (3 - 2 * a)
			elseif p < 0.55 then
				coil = wind - (p - 0.45) / 0.1 * (wind + 0.4)
			else
				coil = -0.4 * (1 - (p - 0.55) / 0.45)
			end
		end
	elseif u.shove_t > 0 then
		body.animate(u.fig, dt, moved, "shove")
	elseif it.fire or u.cd_gun > 0.2 then
		body.animate(u.fig, dt, moved, "aim")
	else
		body.animate(u.fig, dt, moved * (if u.injured then 0.7 else 1), "swing")
	end
	body.coil(u.fig, coil)
end

----------------------------------------------------------------------------
-- Zombie behaviour

local zcells = {}
local function zkey(p) return cell_key(math.floor(p.x / 1.5), math.floor(p.z / 1.5)) end

-- How far a zombie notices a survivor: further when a torch shines.
local function noticed_at(spec, u)
	local k = 1
	if dark() then k = if (u.torch and s.battery > 0) or u.in_light then 1.35 else 0.55 end
	if daylight() then k = KNOBS.day_alert end
	return spec.sense * k
end

local function zombie_step(z, dt)
	local spec = ZOMBIES[z.kind]
	local speed = if z.kind == "runner" then KNOBS.runner else KNOBS.shambler
	if daylight() then speed *= KNOBS.day_speed end
	z.t -= dt
	if z.state == "dead" then
		if z.t <= 0 then remove_zombie_body(z) elseif z.fig then body.fall(z.fig, dt) end
		return
	end
	if not z.fig then return end
	if z.state == "sleep" then
		for _, u in s.survivors do
			if in_body(u) and dist(u.pos, z.pos) < 2.2 then wake(z, flat(u.pos)) end
		end
		return
	end

	local target, td = nil, math.huge
	for _, u in s.survivors do
		if in_body(u) then
			local d = dist(u.pos, z.pos)
			if d < noticed_at(spec, u) and d < td then target, td = u, d end
		end
	end

	local move = v(0, 0, 0)
	local arms = "reach"
	if z.state == "rise" then
		if z.t <= 0 then
			body.lie(z.fig, z.pos, z.yaw, false)
			z.state = if target then "chase" else "investigate"
		end
		return
	elseif z.state == "stagger" then
		move = z.knock
		z.knock = z.knock * math.max(0, 1 - 6 * dt)
		if z.t <= 0 then z.state = if target then "chase" else "investigate" end
	elseif z.state == "windup" then
		arms = "raise"
		if z.target and z.target.pos then z.yaw = yaw_of(z.target.pos - z.pos) end
		if z.t <= 0 then
			z.state = "strike"
			z.t = spec.strike
			z.hit = false
			body.eyes(z.fig, false, NO_GLOW)
			if z.kind == "runner" then count("lunges") end
		end
	elseif z.state == "strike" then
		local f = dir(z.yaw)
		if z.kind == "runner" then move = f * spec.lunge end
		if not z.hit then
			for _, u in s.survivors do
				if in_body(u) then
					local to = u.pos - z.pos
					local d = math.sqrt(to.x * to.x + to.z * to.z)
					local reach = if z.kind == "runner" then 0.85 else spec.reach + 0.1
					if d < reach and (d < 0.3 or (to.x * f.x + to.z * f.z) / d > 0.2) then
						hurt(u, z.pos)
						z.hit = true
						count("zombie_hits")
						break
					end
				end
			end
		end
		if z.t <= 0 then
			z.state = "recover"
			z.t = spec.recover
		end
	elseif z.state == "recover" then
		move = z.knock
		z.knock = z.knock * 0.9
		if z.t <= 0 then z.state = if target then "chase" else "investigate" end
	else
		if target then
			z.state = "chase"
			z.target = target
			z.goal = flat(target.pos)
		end
		if z.state == "chase" then
			if not target then
				z.state = "investigate"
			elseif td < spec.reach and z.t <= 0 then
				z.state = "windup"
				z.t = spec.windup
				z.target = target
				body.eyes(z.fig, true)
				if random.chance(0.4) then sfx("groan", z.pos, { pitch = random.range(1.1, 1.3) }) end
			else
				local to = waypoint(z.pos, target.pos) - z.pos
				local d = math.sqrt(to.x * to.x + to.z * to.z)
				if d > 0.05 then move = to / d * speed end
			end
		end
		if z.state == "investigate" then
			if z.goal and dist(z.goal, z.pos) > 1 then
				local to = waypoint(z.pos, z.goal) - z.pos
				local d = math.sqrt(to.x * to.x + to.z * to.z)
				if d > 0.05 then move = to / d * speed * 0.8 end
			else
				z.state = "wander"
				z.goal = nil
				z.t = random.range(1, 4)
			end
		elseif z.state == "wander" then
			if z.t <= 0 then
				z.yaw += random.range(-1.5, 1.5)
				z.t = random.range(2, 5)
				if random.chance(0.3) then z.state = "idle" end
			end
			move = dir(z.yaw) * speed * 0.35
		elseif z.state == "idle" then
			arms = "swing"
			if z.t <= 0 then
				z.state = "wander"
				z.t = random.range(2, 5)
			end
		end
		if z.side ~= 0 and move.x * move.x + move.z * move.z > 0.01 then
			move = move + v(-move.z, 0, move.x) * z.side
			z.side_t -= dt
			if z.side_t <= 0 then z.side = 0 end
		end
	end

	local push = v(0, 0, 0)
	local cx, cz = math.floor(z.pos.x / 1.5), math.floor(z.pos.z / 1.5)
	for ox = -1, 1 do
		for oz = -1, 1 do
			local list = zcells[cell_key(cx + ox, cz + oz)]
			if list then
				for _, o in list do
					if o ~= z then
						local dx, dz = z.pos.x - o.pos.x, z.pos.z - o.pos.z
						local d2 = dx * dx + dz * dz
						if d2 < 0.42 and d2 > 1e-6 then
							local d = math.sqrt(d2)
							push = push + v(dx / d, 0, dz / d) * (0.65 - d) * 6
						end
					end
				end
			end
		end
	end
	local before = z.pos
	z.pos = collide(z.pos + (move + push) * dt, RADIUS)
	local mv = vector.magnitude(move)
	if mv > 0.3 and z.state ~= "strike" then
		z.yaw = yaw_of(move)
		if dist(before, z.pos) < mv * dt * 0.25 then
			z.stuck += dt
			if z.stuck > 0.6 and z.side == 0 then
				z.side = if random.chance(0.5) then 1 else -1
				z.side_t = 1
				z.stuck = 0
			end
		else
			z.stuck = 0
		end
	end
	body.place(z.fig, z.pos, z.yaw)
	body.animate(z.fig, dt, dist(before, z.pos) / dt, arms)
end

local function spawn_edge(kind, mode, goal, behind)
	for _ = 1, 8 do
		local p
		if behind then
			local x = s.min_x - random.range(2, 6)
			p = v(x, 0, path_z(x) + random.range(-HZ + 2, HZ - 2))
		elseif random.chance(0.5) then
			local x = s.front + random.range(20, 26)
			p = v(x, 0, path_z(x) + random.range(-HZ + 2, HZ - 2))
		else
			local x = s.center.x + random.range(-8, 18)
			p = v(x, 0, path_z(x) + (if random.chance(0.5) then -1 else 1) * (HZ - 0.8))
		end
		local ok = true
		for _, u in s.survivors do
			if u.pos and dist(u.pos, p) < 14 then ok = false end
		end
		if ok then return spawn_zombie(kind, p, mode, goal) end
	end
	return nil
end

-- The director: noise builds pressure; at the top the dead come for the
-- last loud thing they heard, then a calm. A hurt group gets breathing room.
local function director(dt)
	local d = s.dir
	d.t -= dt
	local awake, hurt_n, n = 0, 0, 0
	for _, z in s.zombies do
		if z.state ~= "dead" and z.state ~= "sleep" then awake += 1 end
	end
	for _, u in s.survivors do
		if u.state ~= "gone" then
			n += 1
			if not standing(u) or u.injured then hurt_n += 1 end
		end
	end
	local relief = n > 0 and hurt_n / n > 0.6
	s.relief = relief
	if d.mode == "calm" then
		if d.t <= 0 then d.mode = "build" end
		return
	end
	s.pressure = math.min(1.3, s.pressure + dt * KNOBS.pressure * (1 + 0.15 * s.day) * (if relief then 0.4 else 1))
	-- Quiet lets it settle: only noise builds it up.
	s.pressure = math.max(0, s.pressure - dt * 0.004)
	local heard = s.heard and s.heard_at and game.time() - s.heard_at < 20
	if d.mode == "build" then
		d.spawn_t -= dt
		if d.spawn_t <= 0 then
			d.spawn_t = 9 - 6 * math.min(1, s.pressure) + random.range(-1, 1)
			if awake < (14 + 5 * s.day) * KNOBS.density and not relief then
				local kind = if random.chance(math.min(0.3, 0.06 * s.day)) then "runner" else "shambler"
				-- Stragglers wander in; after a noise they head for it.
				if heard then spawn_edge(kind, "investigate", s.heard, random.chance(0.4)) else spawn_edge(kind, "wander", nil, random.chance(0.3)) end
				count("trickle")
			end
		end
		if s.pressure >= 1 and not relief then
			d.mode = "peak"
			d.t = KNOBS.peak
			d.peak_left = math.ceil((6 + 3 * s.day) * KNOBS.density)
			count("peaks")
			d.goal = if heard then s.heard else s.center
		end
	elseif d.mode == "peak" then
		if (d.peak_left or 0) > 0 and game.tick() % 20 == 0 then
			local kind = if random.chance(math.min(0.45, 0.1 + 0.05 * s.day)) then "runner" else "shambler"
			local z = spawn_edge(kind, "investigate", d.goal or s.center, random.chance(0.5))
			if z then
				d.peak_left -= 1
				if d.peak_left % 3 == 0 then sfx("groan", z.pos, { pitch = random.range(0.7, 0.95), db = 2 }) end
			end
		end
		if d.t <= 0 then
			d.mode = "calm"
			d.t = KNOBS.calm
			s.pressure = 0.25
		end
	end
end

----------------------------------------------------------------------------
-- The camera

local function group_center(members)
	local sum, n = v(0, 0, 0), 0
	for _, u in members do
		if standing(u) and not u.recruit then
			sum = sum + u.pos
			n += 1
		end
	end
	if n == 0 then
		for _, u in members do
			if u.pos and u.state ~= "gone" then
				sum = sum + u.pos
				n += 1
			end
		end
	end
	if n == 0 then return nil, 0 end
	local c = sum / n
	local spread = 0
	for _, u in members do
		if standing(u) and not u.recruit then spread = math.max(spread, dist(u.pos, c)) end
	end
	return c, spread
end

-- Split screen. People who walk apart each get their own part of the
-- screen and the group's camera stays with each; they share one again
-- when they come together. Apart is measured as the screen sees it: a
-- metre up or down the screen counts for more than one across it.
local SPLIT_AT, JOIN_AT = 18, 12
local function apart(a, b) return math.max(math.abs(a.x - b.x), math.abs(a.z - b.z) * 1.6) end

local function groups_step()
	local people = {}
	for _, u in s.survivors do
		if not u.bot and u.pos and u.state ~= "gone" then table.insert(people, u) end
	end
	local root = {}
	for i = 1, #people do root[i] = i end
	local function find(i)
		while root[i] ~= i do i = root[i] end
		return i
	end
	for i = 1, #people do
		for j = i + 1, #people do
			local a, b = people[i], people[j]
			local limit = if (a.group or 1) == (b.group or 1) then SPLIT_AT else JOIN_AT
			if apart(a.pos, b.pos) < limit then root[find(j)] = find(i) end
		end
	end
	local by_root, groups = {}, {}
	for i, u in people do
		local r = find(i)
		if not by_root[r] then
			by_root[r] = { members = {} }
			table.insert(groups, by_root[r])
		end
		table.insert(by_root[r].members, u)
	end
	if #groups == 0 then groups = { { members = {} } } end
	-- Bots go with the nearest person's group.
	for _, u in s.survivors do
		if u.bot or not u.pos or u.state == "gone" then
			local best, bd = groups[1], math.huge
			if u.pos then
				for _, g in groups do
					for _, m in g.members do
						local d = dist(m.pos, u.pos)
						if d < bd then best, bd = g, d end
					end
				end
			end
			table.insert(best.members, u)
		end
	end
	for _, g in groups do g.center, g.spread = group_center(g.members) end
	-- West to east: left to right on the screen.
	local function key(g) return if g.center then g.center.x else 0 end
	table.sort(groups, function(a, b) return key(a) < key(b) end)
	local views = {}
	for n, g in groups do
		for _, u in g.members do
			u.group = n
			views[u.id] = n
		end
	end
	local known = {}
	for _, p in players.list() do known[p.id] = true end
	for id in views do
		if not known[id] then views[id] = nil end
	end
	if next(views) then camera.views(views) end
	s.groups = groups
end

local function camera_step(dt)
	groups_step()
	local c, spread = group_center(s.survivors)
	c = c or v(s.min_x + 4, 0, 0)
	s.center = c
	local west, east = math.huge, -math.huge
	for _, g in s.groups do
		if g.center then
			west = math.min(west, g.center.x)
			east = math.max(east, g.center.x)
		end
	end
	if west == math.huge then west, east = c.x, c.x end
	s.front = math.max(s.front, east)
	-- Along the way: east, plus every street walked up or down.
	local i = math.floor(s.front / CHUNK) + 1
	local r = s.front - s.start_x
	for j = 2, i - 1 do r += math.abs((s.zcs[j] or 0) - (s.zcs[j - 1] or 0)) end
	local zc, prev = s.zcs[i] or 0, s.zcs[i - 1] or 0
	if zc ~= prev then
		local done = if s.front - (i - 1) * CHUNK > land.TURN then 1 else math.clamp((c.z - prev) / (zc - prev), 0, 1)
		r += math.abs(zc - prev) * done
	end
	s.route = math.max(s.route, r)
	s.min_x = math.max(s.min_x, west - BEHIND)
	-- One camera per part of the screen; a new part starts where the
	-- shared camera was, so it slides over to its people.
	s.cams = s.cams or {}
	s.spreads = s.spreads or {}
	s.eyes = s.eyes or {}
	local from = s.cams[1] or c
	for n, g in s.groups do
		local gc = g.center or c
		local cam = s.cams[n] or from
		cam = cam + (v(gc.x + 2, 0, gc.z) - cam) * math.min(1, dt * 3)
		s.cams[n] = cam
		local sp = s.spreads[n] or g.spread
		sp = sp + (g.spread - sp) * math.min(1, dt * 2)
		s.spreads[n] = sp
		local k = 1 + math.max(0, sp - 5) * 0.04
		local target = v(cam.x, 0.5, cam.z)
		local pos = target + v(0, KNOBS.cam_height * k, KNOBS.cam_back * k)
		s.eyes[n] = pos
		-- A heavy blow shakes the view of whoever is in it.
		local shake = 0
		for _, u in g.members do shake = math.max(shake, u.shake or 0) end
		if shake > 0.005 then
			local j = v(random.range(-1, 1), random.range(-0.5, 0.5), random.range(-1, 1)) * shake
			pos += j
			target += j * 0.5
		end
		local any = false
		for _, u in g.members do
			if not u.bot and players.get(u.id) then
				camera.set(u.id, { pos = pos, target = target, fov = KNOBS.fov })
				any = true
			end
		end
		if not any and n == 1 then
			local first = players.list()[1]
			if first then camera.set(first.id, { pos = pos, target = target, fov = KNOBS.fov }) end
		end
	end
	for n = #s.groups + 1, #s.cams do
		s.cams[n], s.spreads[n], s.eyes[n] = nil, nil, nil
	end
	s.cam, s.eye = s.cams[1], s.eyes[1]
end

-- Going in for supplies is a gamble: stay inside a building too long and
-- the dead outside hear it. They wake, more come down the street, and they
-- make for the front door, so the way out fills up. A groan and a warning
-- come first; a back door, if the building has one, is the way out.
local function front_door(b)
	for _, d in b.doors do
		if d.a == 0 then return d end
	end
	return nil
end

local function buildings_step(dt)
	for _, b in s.buildings do
		if #b.rooms > 0 and not b.closed_in then
			local inside = nil
			for _, u in s.survivors do
				if in_body(u) and u.pos and room_of(b, u.pos) > 0 then inside = u break end
			end
			if inside then
				b.inside_t = (b.inside_t or 0) + dt
			else
				b.inside_t = math.max(0, (b.inside_t or 0) - dt * 0.5)
			end
			local door = front_door(b)
			if KNOBS.trap > 0 and inside and door and b.inside_t > KNOBS.trap then
				b.closed_in = true
				count("closed_in")
				local at = door.pa
				sfx("groan", at, { pitch = 0.7, db = 4 })
				game.log("The dead are closing in on a building")
				for _, u in s.survivors do
					if in_body(u) and u.pos and room_of(b, u.pos) > 0 then popup(u, "they heard you, get out", "#ff6a4a") end
				end
				-- The dead nearby make for the door...
				for _, z in s.zombies do
					if z.state ~= "dead" and dist(z.pos, at) < 30 then wake(z, flat(at)) end
				end
				-- ...and more come down the street from both sides.
				local n = math.floor((2 + math.ceil(s.day / 2) + random.int(0, 2)) * KNOBS.density)
				s.pressure = math.min(1.3, s.pressure + 0.3)
				for i = 1, n do
					local side = if i % 2 == 0 then 1 else -1
					local p = v(at.x + side * random.range(9, 15), 0, at.z + random.range(-2, 2) - (if at.z > path_z(at.x) then 3 else -3))
					local z = spawn_zombie(if random.chance(0.15 + 0.05 * s.day) then "runner" else "shambler", p, "investigate", flat(at))
					if z then z.goal = flat(at) end
				end
			end
		end
	end
end

-- A closed building that stands between the camera and a survivor (or
-- just in front of one) gets out of the way: it hides and a low cut-away
-- shows its footprint, until nobody is behind it any more.
local function seg_hits_box(a, b, x0, x1, y0, y1, z0, z1)
	local t0, t1 = 0, 1
	for _, ax in { { a.x, b.x - a.x, x0, x1 }, { a.y, b.y - a.y, y0, y1 }, { a.z, b.z - a.z, z0, z1 } } do
		local o, d, lo, hi = ax[1], ax[2], ax[3], ax[4]
		if math.abs(d) < 1e-6 then
			if o < lo or o > hi then return false end
		else
			local p, q = (lo - o) / d, (hi - o) / d
			if p > q then p, q = q, p end
			t0, t1 = math.max(t0, p), math.min(t1, q)
			if t0 > t1 then return false end
		end
	end
	return true
end

local function blocks_step()
	if not s.eyes or #s.eyes == 0 then return end
	for _, b in s.blocks do
		local hide = false
		if view_dist(v((b.x0 + b.x1) / 2, 0, (b.z0 + b.z1) / 2)) < 34 then
			for _, u in s.survivors do
				local eye = s.eyes[u.group or 1] or s.eyes[1]
				if in_body(u) and u.pos then
					-- A little around each survivor, so the view clears early.
					for _, off in { v(0, 0, 0), v(-1.5, 0, -1.5), v(1.5, 0, -1.5) } do
						local p = v(u.pos.x + off.x, 1.2, u.pos.z + off.z)
						if seg_hits_box(eye, p, b.x0, b.x1, 0, b.h, b.z0, b.z1) then hide = true break end
					end
				end
				if hide then break end
			end
		end
		if hide ~= (b.hidden or false) then
			b.hidden = hide
			for _, id in b.parts do world.set(id, { visible = not hide }) end
			world.set(b.cut, { visible = hide })
		end
	end
end

-- Fog of war. A room stays dark until somebody stands in it or looks in
-- from just outside through a door or a hole in the wall; the dead show
-- only where somebody can see them: near, in a torch beam, or under a
-- working street light, and never through a wall. Every 4 ticks.
local function clear_line(a, b)
	local d = b - a
	local m = math.sqrt(d.x * d.x + d.z * d.z)
	if m < 0.3 then return true end
	local hit = world.raycast(v(a.x, 1.0, a.z), v(d.x / m, 0, d.z / m), m, { tag = "wall" })
	return not hit or hit.distance > m - 0.25
end

local function sight_step()
	local dt = 4 / 60
	local eyes = {}
	for _, u in s.survivors do
		if in_body(u) and u.pos then table.insert(eyes, u) end
	end
	local cx = (s.center or v(0, 0, 0)).x
	for _, b in s.buildings do
		if #b.rooms > 0 and math.abs((b.x0 + b.x1) / 2 - cx) < 40 then
			for _, r in b.rooms do
				if r.shroud then
					local seen = false
					for _, u in eyes do
						local p = u.pos
						local ox = math.max(r.x0 - p.x, 0, p.x - r.x1)
						local oz = math.max(r.z0 - p.z, 0, p.z - r.z1)
						if ox == 0 and oz == 0 then
							seen = true
						elseif ox < 2.2 and oz < 2.2 then
							local q = v(math.clamp(p.x, r.x0 + 0.5, r.x1 - 0.5), 0, math.clamp(p.z, r.z0 + 0.5, r.z1 - 0.5))
							seen = clear_line(p, q)
						end
						if seen then break end
					end
					r.seen_t = if seen then 0.8 else (r.seen_t or 0) - dt
					local dark_now = r.seen_t <= 0
					if dark_now ~= (r.dark ~= false) then
						r.dark = dark_now
						world.set(r.shroud, { visible = dark_now })
					end
				end
			end
		end
	end
	local day = daylight()
	local near = if day then 22 else (if dark() then 6.5 else 12)
	local lamps = {}
	if dark() then
		for _, ch in s.chunks do
			for _, l in ch.lamps or {} do
				if not l.off and math.abs(l.pos.x - cx) < 30 then table.insert(lamps, l) end
			end
		end
	end
	for _, z in s.zombies do
		if z.fig then
			local seen = false
			for _, u in eyes do
				local dx, dz = z.pos.x - u.pos.x, z.pos.z - u.pos.z
				local d = math.sqrt(dx * dx + dz * dz)
				local look = d < near
				if not look and d < 15 and u.torch and s.battery > 0 and dark() then
					local f = dir(u.yaw)
					look = (dx * f.x + dz * f.z) > d * 0.86
				end
				if not look and d < 22 then
					for _, l in lamps do
						local lx, lz = z.pos.x - l.pos.x, z.pos.z - l.pos.z
						if lx * lx + lz * lz < 20 then look = true break end
					end
				end
				if look and clear_line(u.pos, z.pos) then seen = true break end
			end
			if z.dog_t and z.dog_t > game.time() then seen = true end
			z.seen_t = if seen then 0.6 else (z.seen_t or 0) - dt
			body.hide(z.fig, z.seen_t <= 0)
		end
	end
end

----------------------------------------------------------------------------
-- Hooks

function warm_all()
	land.warm()
	props.warm()
	dog.warm()
	-- Bake the skies of the day in the background: switching to one that
	-- is not baked yet stalls a frame for a few hundred ms.
	local skies = {}
	for _, t in TIMES do
		if not skies[t] then skies[t] = true sky.warm(t) end
	end
	for _, list in { ZOMBIE_SKIN, ZOMBIE_SHIRTS, ZOMBIE_PANTS, SKINS, PANTS, STRANGER_SHIRTS } do
		for _, c in list do figure.warm({ shirt = c, skin = c, pants = c }) end
	end
	figure.warm({ shirt = "#6e3a36", skin = "#6e3a36", pants = "#6e3a36" })
	for _, e in ENCOUNTERS do
		if e.look then figure.warm({ shirt = e.look.shirt or "#777777", skin = e.look.skin or "#777777", pants = e.look.pants or "#777777" }) end
	end
	for _, p in players.list() do
		figure.warm({ shirt = p.color, skin = p.color, pants = p.color })
		world.warm("cube", { color = p.color, emissive = 0.2 })
	end
	figure.warm({ shirt = v(0.55, 0.55, 0.5), skin = "#777777", pants = "#777777" })
	world.warm("cube", figure.EYE_GLOW)
	world.warm("cube", NO_GLOW)
	world.warm("cube", figure.CORPSE)
	world.warm("sphere", { color = "#cfe6ff", emissive = 0.35 })
	for _, b in BLOOD do world.warm("sphere", b) end
	world.warm("cube", { color = "#3b3631", roughness = 0.8, metal = 0, emissive = 0 })
	world.warm("outline16", { color = "#f2e6b0", emissive = 0.12 })
	for _, w in HELD do world.warm("cube", { color = w.color, metal = w.metal, roughness = w.roughness or 0.6 }) end
	for _, c in { "#5a1e18", "#6a2a20", "#8a1a14" } do world.warm("cube", { color = c, emissive = 0.06 }) end
	-- A breaking weapon bursts into bits of its own colour.
	for _, w in HELD do world.warm("cube", { color = w.color, emissive = 0.06 }) end
	world.warm("sphere", { color = GLOW, emissive = 0.35, roughness = 1 })
	world.warm("cube", { color = "#fff2c0", emissive = 0.6 })
	world.warm("cube", { color = "#ffd27a", emissive = 1.2 })
	world.warm("cube", { color = "#e8dfc4", emissive = 0.05 })
	world.warm("cube", { color = "#ff3a2a", emissive = 0.25 })
	world.warm("cube", { color = "#f2e6b0", emissive = 0.3 })
	for _, c in ITEM_COLORS do world.warm("cube", { color = c, emissive = 0.3 }) end
end

local function init()
	new_run()
end

local function reload()
	s = state.s
	if s and s.rects then build_grid() end
	if s then
		s.time_now = nil
		look_of_hour()
		light_step(0)
	end
end

local function zombies_tick(dt)
	zcells = {}
	for _, z in s.zombies do
		if z.state ~= "dead" then
			local k = zkey(z.pos)
			local list = zcells[k]
			if not list then
				list = {}
				zcells[k] = list
			end
			table.insert(list, z)
		end
	end
	for _, z in s.zombies do zombie_step(z, dt) end
	-- A few times a second: humans for the dead in view, boxes for the rest.
	-- A full pool takes the human of the oldest corpse first.
	if game.tick() % 15 == 0 and s.cam then
		for _, z in s.zombies do
			if z.fig and body.is_human(z.fig) and not near_view(z.pos, 8) then
				body.demote(z.fig)
			end
		end
		for _, z in s.zombies do
			if z.fig and not body.is_human(z.fig) and z.state ~= "dead" and near_view(z.pos) then
				z.fig.pool = z.pool
				if not body.promote(z.fig) then
					local oldest = nil
					for _, o in s.zombies do
						if o.state == "dead" and o.pool == z.pool and o.fig and body.is_human(o.fig) and (oldest == nil or o.t < oldest.t) then oldest = o end
					end
					if oldest then
						body.demote(oldest.fig)
						body.promote(z.fig)
					end
				end
			end
		end
	end
end

-- Street events over time: lures, fires, fog, things that happen as you
-- pass, the dog, and the bitten.
local function events_step(dt)
	s.event_lights = {}
	for _, st in s.strangers do
		local e = encounter_of(st)
		-- Something that keeps making noise (a radio, an alarm, a bell).
		if st.lure then
			st.lure.t -= dt
			st.lure.next -= dt
			if st.lure.next <= 0 then
				st.lure.next = st.lure.every
				noise(st.prop_at or st.pos, st.lure.radius, 0.01)
				if st.lure.sound then sfx(st.lure.sound, st.prop_at or st.pos) end
			end
			if st.lure.t <= 0 then st.lure = nil end
		end
		if st.light then
			st.light.t -= dt
			local at = st.prop_at or st.pos
			table.insert(s.event_lights, { pos = v(at.x, 1.6, at.z), color = st.light.color, intensity = st.light.intensity * (0.85 + 0.15 * math.sin(game.time() * 11 + at.x)), range = st.light.range })
			if st.light.t <= 0 then st.light = nil end
		end
		-- Things that happen as you come near: no card, or a card that opens.
		if e and e.auto and not st.done then
			local u, d = nearest_body(st.prop_at or st.pos, e.choices ~= nil)
			if u and d < (e.radius or 7) then
				if e.choices then
					if not u.bot and not u.talk then
						u.talk, u.talk_i, u.talk_age = st, 1, 0
						sfx("search", st.pos, { pitch = 1.3 })
					elseif u.bot and KNOBS.autopilot == 1 then
						choose(u, st, e.choices[1])
					end
				else
					st.done = true
					local res = pick_result(st, e.results or {})
					if res then apply_result(u, st, res) end
				end
			end
		end
	end
	-- Fog rolls in for a while.
	if s.fog_t then
		s.fog_t -= dt
		if s.fog_t > 0 then
			light.set({ fog_start = 3, fog_end = 16, fog_density = 0 })
		else
			s.fog_t = nil
			if dark() then light.set({ fog_start = 30, fog_end = 70, fog_density = 0 }) else light.set({ fog_start = 34, fog_end = 90, fog_density = 0 }) end
		end
	end
	-- After a power cut, the lamps further on are dead too.
	if s.power_cut_x then
		for _, ch in s.chunks do
			for _, l in ch.lamps or {} do
				if not l.off and l.pos.x < s.power_cut_x + 200 and l.pos.x > s.power_cut_x - 80 then
					l.off = true
					if l.bulb then world.set(l.bulb, { color = "#4a4640", emissive = 0 }) end
				end
			end
		end
	end
	-- The dog walks at a person's heel and barks at the dead it smells.
	local dg = s.dog
	if dg then
		local u = nearest_body(dg.pos, true) or nearest_body(dg.pos)
		local speed = 0
		if u then
			local goal = u.pos + v(-1.2, 0, 1.0)
			local to = goal - dg.pos
			local d = math.sqrt(to.x * to.x + to.z * to.z)
			if d > 0.5 then
				speed = math.min(6, d * 2.5)
				dg.pos = collide(dg.pos + to / d * speed * dt, 0.25)
				dg.yaw = math.atan2(-to.z, to.x)
			end
		end
		dg.bark = math.max(0, dg.bark - dt * 3)
		dg.check -= dt
		if dg.check <= 0 then
			dg.check = 0.5
			for _, z in s.zombies do
				if z.state ~= "dead" and dist(z.pos, dg.pos) < 11 then
					-- Seen by the dog: shown for a while, through walls too.
					z.dog_t = game.time() + 3
					if not z.barked then
						z.barked = true
						dg.bark = 1
						sfx("bark", dg.pos, { pitch = random.range(0.9, 1.1) })
						noise(dg.pos, 5, 0)
						if u then popup(u, "the dog growls", "#e8c890") end
					end
				end
			end
		end
		dog.place(dg.d, dg.pos, dg.yaw or 0, speed, dt, dg.bark)
	end
	-- A bite they kept quiet about.
	for _, r in s.survivors do
		if r.turns and in_body(r) then
			r.turns -= dt
			if r.turns < 8 and not r.warned then
				r.warned = true
				local who = nearest_body(r.pos, true)
				if who then popup(who, "the stranger is shaking", "#ff8a6a") end
			end
			if r.turns <= 0 then
				r.turns = nil
				sfx("groan", r.pos, { pitch = 0.8 })
				local target = nearest_body(r.pos, true) or nearest_body(r.pos)
				spawn_zombie("runner", r.pos, "investigate", if target then flat(target.pos) else nil)
				if r.fig then body.remove(r.fig) end
				r.fig = nil
				r.state = "gone"
				count("turned")
			end
		end
	end
end

local function tick(dt, input)
	s = state.s
	if s.phase == "camp" then
		s.camp_t -= dt
		if s.camp_t <= 0 then
			s.phase = "play"
			s.time_now = nil
		end
		return
	end
	if s.phase == "over" then
		s.over_t -= dt
		zombies_tick(dt)
		local again = s.over_t <= 0
		for _, u in s.survivors do
			local i = not u.bot and input[u.id]
			if i and i.pressed.south and s.over_t < 3 then again = true end
		end
		if again then new_run() end
		return
	end

	camera_step(dt)
	if game.tick() % 4 == 0 then blocks_step() end
	if game.tick() % 4 == 2 then sight_step() end
	buildings_step(dt)
	stream_land()
	look_of_hour()
	light_step(dt)

	-- Standing under a street light that works: the dead see you from afar.
	for _, u in s.survivors do
		u.in_light = false
		if u.pos and dark() then
			for _, ch in s.chunks do
				for _, l in ch.lamps or {} do
					local dx, dz = l.pos.x - u.pos.x, l.pos.z - u.pos.z
					if dx * dx + dz * dz < 25 then u.in_light = true end
				end
			end
		end
	end

	-- The battery drains for every lit torch at night.
	if dark() then
		for _, u in s.survivors do
			if in_body(u) and u.torch then s.battery = math.max(0, s.battery - KNOBS.drain * dt) end
		end
	end

	for _, u in s.survivors do
		local it = if u.bot or KNOBS.autopilot == 1 then bot_intent(u) else from_input(input[u.id])
		act(u, it, dt)
	end
	-- Drop recruits that died.
	for i = #s.survivors, 1, -1 do
		if s.survivors[i].state == "gone" then table.remove(s.survivors, i) end
	end

	-- Strangers face the nearest survivor; a robber runs off; whoever is
	-- with them keeps their place beside them.
	for _, st in s.strangers do
		local speed = 0
		if st.fig and st.fleeing then
			st.fleeing -= dt
			st.pos = collide(st.pos + v(0, 0, if st.pos.z > path_z(st.pos.x) then 4 else -4) * dt + v(3, 0, 0) * dt, RADIUS)
			st.yaw = yaw_of(v(1, 0, if st.pos.z > path_z(st.pos.x) then 1 else -1))
			st.lift = nil
			body.place(st.fig, st.pos, st.yaw)
			body.animate(st.fig, dt, 4, "swing")
			speed = 4
			if st.fleeing <= 0 then drop_stranger(st) end
		elseif st.fig and st.following then
			-- Walking with the group, a few steps behind the nearest person.
			local u = nearest_body(st.pos, true) or nearest_body(st.pos)
			if u then
				local goal = u.pos + v(-1.6, 0, if st.pos.z > u.pos.z then 1 else -1)
				local to = goal - st.pos
				local d = math.sqrt(to.x * to.x + to.z * to.z)
				if d > 0.4 then
					speed = math.min(KNOBS.walk * 0.85, d * 2)
					st.pos = collide(st.pos + to / d * speed * dt, RADIUS)
					st.yaw = yaw_of(to)
				end
			end
			body.place(st.fig, st.pos, st.yaw)
			body.animate(st.fig, dt, speed, "swing")
			if st.escort and s.route >= st.escort.to then
				-- Where they were going: they thank you and head off.
				local who = nearest_body(st.pos, true) or nearest_body(st.pos)
				for item, n in st.escort.gain or {} do s[item] = (s[item] or 0) + n end
				if who and st.escort.text then popup(who, st.escort.text, "#e8e0c8") end
				sfx("pickup", st.pos, { pitch = 0.9 })
				st.escort, st.following, st.done = nil, nil, true
				st.fleeing = 5
			end
		elseif st.fig then
			local near = nil
			for _, u in s.survivors do
				if in_body(u) and dist(u.pos, st.pos) < 6 then near = u end
			end
			if near and st.pose ~= "down" then st.yaw = yaw_of(near.pos - st.pos) end
			body.place(st.fig, if st.lift then st.pos + v(0, st.lift, 0) else st.pos, st.yaw)
			if st.pose then body.pose(st.fig, st.pose) else body.animate(st.fig, dt, 0, "swing") end
		end
		for _, w in st.with or {} do
			local at = if st.following or st.fleeing then v(-0.9, 0, 0.7) else w.at
			local p = collide(st.pos + at, RADIUS)
			body.place(w.fig, p, if w.pose == "down" then 0 else st.yaw)
			if w.pose and not (st.following or st.fleeing) then body.pose(w.fig, w.pose) else
				if w.fig.pose ~= "walk" and w.fig.pose ~= "idle" then body.pose(w.fig, "walk") end
				body.animate(w.fig, dt, speed, "swing")
			end
		end
		-- A stray waits by the road, wagging, until fed or chased off.
		if st.dog then
			if st.dog_run then
				st.dog_run -= dt
				dog.place(st.dog, st.dog.pos + v(5 * dt, 0, 0), 0, 5, dt)
				if st.dog_run <= 0 then
					dog.remove(st.dog)
					st.dog = nil
				end
			else
				local u = nearest_body(st.dog.pos)
				local yaw = if u then math.atan2(-(u.pos.z - st.dog.pos.z), u.pos.x - st.dog.pos.x) else 0
				dog.place(st.dog, st.dog.pos, yaw, 0, dt, if u and dist(u.pos, st.dog.pos) < 5 then math.max(0, math.sin(game.time() * 3)) else 0)
			end
		end
	end

	-- A fallen survivor lies as a human for a while, then as boxes.
	for i = #s.corpses, 1, -1 do
		local c = s.corpses[i]
		body.fall(c.fig, dt)
		c.t -= dt
		if c.t <= 0 or not body.is_human(c.fig) then
			body.demote(c.fig)
			table.remove(s.corpses, i)
		end
	end

	zombies_tick(dt)
	director(dt)
	events_step(dt)

	if game.tick() % 50 == 0 then
		local z = nearest_zombie(s.center, 12, true)
		if z then sfx("groan", z.pos, { pitch = random.range(0.75, 1.1), db = -6 }) end
	end

	-- Camp: everyone standing is at the fire.
	if s.camp then
		local all, here = true, false
		for _, u in s.survivors do
			if standing(u) then
				if dist(u.pos, s.camp) > 4.5 then all = false else here = true end
			end
		end
		-- Somebody waits at the fire; a straggler gets a while to come in.
		s.camp_wait = if here then (s.camp_wait or 0) + dt else 0
		if here and (all or s.camp_wait > 15) then make_camp() end
	end

	local alone = #s.survivors == 1 and s.survivors[1].lives > 0
	if standing_count() == 0 and #s.survivors > 0 and not alone then
		s.phase = "over"
		s.over_t = 7
		s.walked = math.floor(s.route)
		s.best = math.max(s.best, s.walked)
		count("wipes")
		sfx("down", s.center, { pitch = 0.6 })
		game.log("Dead Miles: overrun after " .. s.walked .. " m on day " .. s.day .. ", " .. s.kills .. " dead put down")
	end
end

----------------------------------------------------------------------------
-- The HUD: stocks in a corner, injuries and wear under each survivor.

local function icon(x, y, k, color, n)
	hud.box(x, y + 4 * k, 16 * k, 16 * k, color)
	hud.text(tostring(n), x + 22 * k, y, { size = 22 * k, color = if n == 0 then "#ff6a4a" else "#f4efe6" })
end

-- Gamepad buttons as plain, high-contrast chips: { { "A", "search" }, ... }
-- centred on x.
local BUTTON_COLORS = { A = "#3fae4a", B = "#d8433a", X = "#3a7bd8", Y = "#e0b330" }
local function button_row(cx, y, row, k, alpha)
	alpha = alpha or 1
	local size = 18 * k
	local widths, total = {}, 0
	for i, e in row do
		local chip = if e[1] ~= "" then (if #e[1] == 1 then 24 else 34) * k else 0
		local w = chip + (if chip > 0 then 6 * k else 0) + #e[2] * size * 0.52
		widths[i] = { chip, w }
		total += w + (if i > 1 then 16 * k else 0)
	end
	local x = cx - total / 2
	hud.box(x - 8 * k, y - 5 * k, total + 16 * k, 34 * k, { 0, 0, 0, 0.62 * alpha })
	for i, e in row do
		local chip, w = widths[i][1], widths[i][2]
		if chip > 0 then
			hud.box(x, y, chip, 24 * k, BUTTON_COLORS[e[1]] or "#e8e0d0")
			hud.text(e[1], x, y + 1 * k, { size = 17 * k, width = chip, align = "center", color = if BUTTON_COLORS[e[1]] then "#ffffff" else "#111111" })
		end
		hud.text(e[2], x + chip + (if chip > 0 then 6 * k else 0), y + 1 * k, { size = size, width = w + 20 * k, color = { 1, 1, 1, alpha } })
		x += w + 16 * k
	end
end

-- What A does here for this person, and where: a downed friend, a
-- cupboard, a weapon on the ground, a stranger.
local function prompt_of(u)
	for _, o in s.survivors do
		if o ~= u and o.state == "down" and dist(o.pos, u.pos) < 1.3 then return { { "A", "hold: revive" } }, o.pos end
	end
	local best, bd = nil, 0.85
	for _, c in s.containers do
		if not c.searched then
			local d = to_rect(u.pos, c.rect)
			if d < bd then best, bd = c, d end
		end
	end
	if best then return { { "A", if best.kind == "car" or best.kind == "police" then "hold: search (loud)" else "hold: search" } }, v(best.x, 0, best.z) end
	for _, p in s.pickups do
		if dist(p.pos, u.pos) < 1.1 then return { { "A", "take the " .. p.kind } }, p.pos end
	end
	local st = nearest_stranger(u)
	if st then return { { "A", (encounter_of(st) or {}).verb or "talk" } }, st.pos end
	if u.injured and s.meds > 0 then return { { "Y", "first aid" } }, nil end
	return nil
end

local function bar(x, y, w, h, frac, color)
	hud.box(x - 1, y - 1, w + 2, h + 2, { 0, 0, 0, 0.55 })
	hud.box(x, y, w * math.clamp(frac, 0, 1), h, color)
end

-- Torches and street lights are shadowed spotlights (at most 8: torches
-- first, then the lamps nearest the view); the fire, muzzle flashes and a
-- little spill around each torch are point lights.
local function torches()
	local list = {}
	local spots = {}
	local flicker = s.battery < 15 and math.floor(game.time() * 13) % 5 == 0
	for _, u in s.survivors do
		if in_body(u) and u.pos then
			if u.torch and s.battery > 0 and dark() and not flicker then
				local f = dir(u.yaw)
				local down = if u.state == "down" then 0.05 else 0.22
				-- Out past the arms and the weapon: a Warp Human's own body
				-- would shadow a torch held at the chest.
				table.insert(spots, { pos = v(u.pos.x + f.x * 0.85, 1.3, u.pos.z + f.z * 0.85), dir = v(f.x, -down, f.z), color = "#ffe6c0", intensity = KNOBS.torch, range = 18, angle = 28, inner = 12 })
				table.insert(list, { pos = v(u.pos.x + f.x * 0.6, 1.6, u.pos.z + f.z * 0.6), color = "#ffe6c0", intensity = KNOBS.torch * 0.012, range = 3 })
			end
			if u.flash then table.insert(list, { pos = v(u.pos.x, 1.4, u.pos.z), color = "#ffd27a", intensity = 900, range = 14 }) end
		end
	end
	-- Street lights that still work, nearest the view first.
	if dark() then
		local near = {}
		for _, ch in s.chunks do
			for _, l in ch.lamps or {} do
				local off = l.off or l.flicker and math.sin(game.time() * 7 + l.seed) + math.sin(game.time() * 23 + l.seed * 3) > 1.2
				local d = view_dist(l.pos)
				if not off and d < 30 then table.insert(near, { l = l, d = d }) end
			end
		end
		table.sort(near, function(a, b) return a.d < b.d end)
		for _, n in near do
			if #spots < 8 then
				table.insert(spots, { pos = n.l.pos, dir = v(0, -1, 0), color = "#ffd9a0", intensity = KNOBS.lamp, range = 12, angle = 62, inner = 35 })
			else
				table.insert(list, { pos = n.l.pos, color = "#ffd9a0", intensity = KNOBS.lamp * 0.25, range = 11 })
			end
		end
	end
	for _, c in { s.camp, s.camp_used } do
		if c and view_dist(c) < 40 then
			table.insert(list, { pos = v(c.x, 0.8, c.z), color = "#ff9a4a", intensity = 500 + math.sin(game.time() * 9) * 60, range = 12 })
		end
	end
	for _, l in s.event_lights or {} do
		if view_dist(l.pos) < 40 then table.insert(list, 1, l) end
	end
	while #list > 32 do table.remove(list) end
	if #list > 0 then lights.set(list) else lights.clear() end
	lights.spots(spots)
end

-- A point on the screen, through the camera of the part of the screen u
-- is on (or the nearest group's, for things without a person).
local function proj(pos, u)
	if u and players.get(u.id) then return hud.project(pos, u.id) end
	local g = if u and u.group and s.groups then s.groups[u.group] else nil
	if not g and s.groups then
		local bd = math.huge
		for _, o in s.groups do
			if o.center and dist(o.center, pos) < bd then g, bd = o, dist(o.center, pos) end
		end
	end
	if g then
		for _, m in g.members do
			if players.get(m.id) then return hud.project(pos, m.id) end
		end
	end
	return hud.project(pos)
end

local function draw()
	s = state.s
	if not s or not s.cam then return end
	local W, H = hud.size()
	local k = H / 720
	torches()

	if s.phase == "camp" then
		hud.box(0, 0, W, H, { 0.02, 0.02, 0.03, 0.85 })
		hud.text("NIGHT " .. (s.day - 1), 0, H * 0.38, { size = 64 * k, align = "center", width = W, color = "#f4efe6" })
		hud.text(math.floor(s.route) .. " m walked", 0, H * 0.38 + 78 * k, { size = 26 * k, align = "center", width = W, color = "#b8b0a0" })
		if s.camp_short > 0 then
			hud.text("not enough food", 0, H * 0.38 + 118 * k, { size = 22 * k, align = "center", width = W, color = "#ff8a6a" })
		end
		return
	end

	local x, y = 24 * k, 18 * k
	hud.box(x - 10 * k, y - 8 * k, 330 * k, 44 * k, { 0, 0, 0, 0.4 })
	icon(x, y, k, ITEM_COLORS.ammo, s.ammo)
	icon(x + 70 * k, y, k, ITEM_COLORS.food, s.food)
	icon(x + 140 * k, y, k, ITEM_COLORS.meds, s.meds)
	-- The battery as a bar.
	hud.box(x + 210 * k, y + 4 * k, 16 * k, 16 * k, ITEM_COLORS.battery)
	bar(x + 234 * k, y + 8 * k, 70 * k, 8 * k, s.battery / 100, if s.battery < 15 then "#ff6a4a" else ITEM_COLORS.battery)
	hud.text("DAY " .. s.day .. "   " .. math.max(0, math.floor(s.route)) .. " m", W - 260 * k, y, { size = 22 * k, width = 240 * k, align = "right", color = "#f4efe6" })

	for _, u in s.survivors do
		if u.pos and u.state ~= "gone" then
			local px, py = proj(v(u.pos.x, 0, u.pos.z), u)
			if px then
				local c = { u.color.x, u.color.y, u.color.z, 1 }
				if u.state == "ok" then
					-- A colour tag under the feet, a red cross when injured.
					hud.box(px - 14 * k, py + 10 * k, 28 * k, 4 * k, c)
					if u.injured then
						hud.box(px + 18 * k, py + 4 * k, 12 * k, 4 * k, "#e8382a")
						hud.box(px + 22 * k, py, 4 * k, 12 * k, "#e8382a")
					end
					if u.dur then bar(px - 14 * k, py + 18 * k, 28 * k, 3 * k, u.dur / MELEE[u.melee].dur, "#e8e0c8") end
					-- Stamina, as long as the person's stamina is: some last longer.
					if u.stamina and u.stamina_max and u.stamina < u.stamina_max - 0.01 then
						local w = 30 * k * u.stamina_max / KNOBS.stamina
						bar(px - w / 2, py + 24 * k, w, 4 * k, u.stamina / u.stamina_max, if u.stamina < 0.15 * KNOBS.stamina then "#e8603a" else "#f0d060")
					end
					local prog = math.max(u.search_t / KNOBS.search, u.revive_t / KNOBS.revive)
					if prog > 0 then bar(px - 30 * k, py - 110 * k, 60 * k, 6 * k, prog, c) end
				elseif u.state == "down" then
					bar(px - 30 * k, py + 12 * k, 60 * k, 6 * k, u.t / KNOBS.downed, "#ff4a3a")
				elseif u.state == "ghost" then
					local gx, gy = proj(v(u.pos.x, 1.4, u.pos.z), u)
					if gx then bar(gx - 24 * k, gy + 26 * k, 48 * k, 4 * k, 1 - u.t / KNOBS.ghost, c) end
				end
				if u.popup then
					hud.text(u.popup.text, px - 100 * k, py - 140 * k - (1.8 - u.popup.t) * 20 * k, { size = 20 * k, align = "center", width = 200 * k, color = u.popup.color })
				end
				if not u.bot and u.state == "ok" and not u.talk then
					if u.tip then
						button_row(px, py - 112 * k, u.tip.row, k, math.min(1, u.tip.t * 2))
					else
						local row, at = prompt_of(u)
						if row then
							local qx, qy = px, py - 112 * k
							if at then
								local ax, ay = proj(v(at.x, 1.6, at.z), u)
								if ax then qx, qy = ax, ay - 30 * k end
							end
							button_row(qx, qy, row, k)
						end
					end
				end
				-- The talk card: who it is, what they say, the choices.
				if u.talk then
					local e = encounter_of(u.talk)
					if e then
						local cw = 420 * k
						local lines = #e.choices
						local ch = (64 + 30 * lines) * k
						local cx = math.clamp(px - cw / 2, 10 * k, W - cw - 10 * k)
						local cy = math.clamp(py - 150 * k - ch, 10 * k, H - ch - 10 * k)
						hud.box(cx, cy, cw, ch, { 0.05, 0.05, 0.06, 0.88 })
						hud.box(cx, cy, 4 * k, ch, c)
						hud.text(e.says or e.title or "", cx + 16 * k, cy + 10 * k, { size = 19 * k, width = cw - 28 * k, color = "#e8e0d0" })
						for i, choice in e.choices do
							local yy = cy + (44 + 30 * (i - 1)) * k
							local sel = i == u.talk_i
							if sel then hud.box(cx + 8 * k, yy - 2 * k, cw - 16 * k, 26 * k, { 1, 1, 1, 0.1 }) end
							local afford = has(choice.cost)
							hud.text(choice.label, cx + 18 * k, yy, { size = 19 * k, width = cw - 30 * k, color = if not afford then "#7a746c" elseif sel then "#ffffff" else "#c8c0b4" })
						end
					end
				end
			end
		end
	end

	-- A stranger you can talk to shows a small mark.
	for _, st in s.strangers do
		local e = encounter_of(st)
		if (st.fig or st.nobody) and not st.done and not st.fleeing and not st.following and e and not e.auto then
			local sx, sy = proj(v(st.pos.x, 2.1 + (st.lift or 0), st.pos.z))
			if sx then hud.box(sx - 4 * k, sy - 4 * k, 8 * k, 8 * k, "#f2e6b0") end
		end
	end

	if s.phase == "over" then
		hud.box(0, H * 0.34, W, 150 * k, { 0, 0, 0, 0.6 })
		hud.text("OVERRUN", 0, H * 0.34 + 22 * k, { size = 58 * k, align = "center", width = W, color = "#ff6a4a" })
		hud.text((s.walked or 0) .. " m, day " .. s.day .. "   best " .. s.best .. " m", 0, H * 0.34 + 94 * k, { size = 24 * k, align = "center", width = W, color = "#e8e0d0" })
	end
end

local function player_joined(p)
	s = state.s
	figure.warm({ shirt = p.color, skin = p.color, pants = p.color })
	table.insert(s.survivors, new_survivor(p, #s.survivors + 1))
end

local function player_left(p)
	s = state.s
	for i, u in s.survivors do
		if u.id == p.id then
			if u.fig then body.remove(u.fig) end
			if u.ghost_ent then world.remove(u.ghost_ent) end
			table.remove(s.survivors, i)
			return
		end
	end
end

local function players_changed()
	s = state.s
	for _, p in players.list() do
		for _, u in s.survivors do
			if u.id == p.id then
				u.name = p.name
				if u.color ~= p.color then
					u.color = p.color
					if u.fig then body.tint(u.fig, { color = p.color, roughness = 0.9 }) end
				end
			end
		end
	end
end

-- Console helpers: debug.walk(200) moves everyone 200 m east, debug.peak()
-- starts a horde, debug.ammo(20) hands out rounds.
local debug = {
	-- debug.knob("cam_height", 8): change a knob while it runs.
	knob = function(name, value)
		if KNOBS[name] == nil then return "no knob " .. tostring(name) end
		KNOBS[name] = value
		return name .. " = " .. tostring(value)
	end,
	walk = function(m)
		s = state.s
		for _, u in s.survivors do
			if u.pos then u.pos = collide(v(u.pos.x + m, 0, path_z(u.pos.x + m) + (u.pos.z - path_z(u.pos.x))), RADIUS) end
			if u.fig then body.place(u.fig, u.pos, u.yaw) end
		end
		return "walked " .. m
	end,
	-- debug.go("car"): the survivors to the nearest container of that kind
	-- ahead (any kind without one), to look at it.
	go = function(kind)
		s = state.s
		local u = s.survivors[1]
		local best = nil
		for _, c in s.containers do
			if (kind == nil or c.kind == kind) and c.x > u.pos.x and (best == nil or c.x < best.x) then best = c end
		end
		if not best then return "none" end
		for i, o in s.survivors do
			if o.pos then o.pos = collide(v(best.x - 1, 0, best.z + 2 + i * 0.6), RADIUS) end
			if o.fig then body.place(o.fig, o.pos, o.yaw) end
		end
		return "at " .. best.kind .. " " .. math.floor(best.x)
	end,
	peak = function()
		s = state.s
		s.pressure = 1.2
		return "pressure up"
	end,
	ammo = function(n)
		s = state.s
		s.ammo += n or 10
		return "ammo " .. s.ammo
	end,
	-- debug.event("radio"): that encounter a few metres ahead of the first
	-- survivor, to look at or test it.
	event = function(id)
		s = state.s
		local e = nil
		for _, x in ENCOUNTERS do
			if x.id == id then e = x end
		end
		if not e then return "no encounter " .. tostring(id) end
		local u = s.survivors[1]
		local x = u.pos.x + 7
		local index = math.floor(x / CHUNK) + 1
		local c = nil
		for _, ch in s.chunks do
			if ch.index == index then c = ch end
		end
		if not c then return "no land there" end
		local side = 1
		local pos = v(x, 0, path_z(x) + 1)
		if e.where == "room" then
			local best = nil
			for _, b in s.buildings do
				for _, r in b.rooms do
					local m = v((r.x0 + r.x1) / 2, 0, (r.z0 + r.z1) / 2)
					if m.x > u.pos.x and (best == nil or dist(m, u.pos) < dist(best, u.pos)) then best = m end
				end
			end
			pos = best or pos
			side = if pos.z > path_z(pos.x) then 1 else -1
		elseif e.prop and not e.in_road then
			pos = v(x, 0, path_z(x) + 0.9 + props.DEFS[e.prop].foot[2] / 2)
		end
		place_stranger(e, pos, index, c, side)
		return "placed " .. id
	end,
}

return { init = init, tick = tick, draw = draw, reload = reload, debug = debug, player_joined = player_joined, player_left = player_left, players_changed = players_changed }
