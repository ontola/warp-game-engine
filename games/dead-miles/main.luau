-- Dead Miles (working name): a co-op road trip through the end of the
-- world. One to four survivors stop in a generated street, search houses
-- and cars for food, fuel and the very little ammo left, and get back to
-- the car before the street fills up. Melee weapons wear out, guns are
-- loud, and a director swings the dead from a quiet street to a siege.
-- A survivor who goes down can be revived; one who dies comes back in
-- seconds as a ghost and then as a new survivor at the car. The run ends
-- only when nobody is standing.
--
-- Controls: left stick moves, right stick aims, RT fires the gun, X
-- swings the melee weapon, B shoves, A (hold) searches, picks up, revives
-- and drives off at the car, Y uses a medkit. Keyboard: WASD, left mouse
-- fires, R swings, Shift shoves, Space uses, F heals.
--
-- Design: the "Zombie road-trip co-op: design" doc. The rules live here;
-- the street is lib/street, the people lib/figure.

local figure = require("lib/figure")
local street = require("lib/street")
local v = vector.create

-- @knobs
local KNOBS = {
	walk = 4.3, -- survivor speed, m/s
	shambler = 1.25, -- shambler speed, m/s
	runner = 4.4, -- runner speed, m/s
	start_ammo = 6, -- rounds at the start of a run
	start_food = 4, -- food at the start of a run
	start_fuel = 2, -- fuel at the start of a run
	pressure = 0.006, -- how fast the director's pressure rises per second
	peak = 12, -- seconds of a director peak (a horde from the edge)
	calm = 25, -- seconds of calm after a peak
	downed = 8, -- seconds a downed survivor can wait for a revive
	ghost = 12, -- seconds as a ghost before a new survivor arrives
	revive = 2, -- seconds a teammate holds A to revive
	search = 1.2, -- seconds to search a container
	cam_height = 17, -- camera height over the group, m
	cam_back = 12, -- camera distance behind (south of) the group, m
	fov = 40, -- camera field of view, degrees
	torch = 380, -- torch brightness at night
	density = 1, -- zombie count multiplier
	autopilot = 0, -- 1: bots drive the people too (demos, store clips, tests)
}
-- @end

local HX, HZ = street.HALF_X, street.HALF_Z
local RADIUS = 0.32 -- survivor and zombie collision radius, m
local LEASH = 13 -- how far a survivor may stray from the group, m

local MELEE = {
	fists = { dmg = 0.6, range = 1.3, arc = 0.5, cd = 0.38, noise = 2, cleave = 1, knock = 2 },
	pipe = { dmg = 1, range = 1.6, arc = 0.35, cd = 0.45, dur = 25, noise = 6, cleave = 2, knock = 3 },
	bat = { dmg = 1.5, range = 1.75, arc = 0.25, cd = 0.52, dur = 30, noise = 6, cleave = 3, knock = 4 },
	machete = { dmg = 2, range = 1.55, arc = 0.35, cd = 0.38, dur = 20, noise = 5, cleave = 2, knock = 2 },
	axe = { dmg = 3, range = 1.75, arc = 0.3, cd = 0.72, dur = 15, noise = 7, cleave = 3, knock = 3.5 },
}
local RANK = { fists = 0, pipe = 1, bat = 2, machete = 3, axe = 4 }
local GUNS = {
	pistol = { dmg = 2, cd = 0.42, pellets = 1, spread = 0.025, range = 20, noise = 25 },
	shotgun = { dmg = 1.6, cd = 0.95, pellets = 5, spread = 0.14, range = 11, noise = 35, dur = 40 },
}
local HELD = {
	pipe = { len = 0.8, w = 0.05, color = "#8a8f96", metal = 0.8, roughness = 0.4 },
	bat = { len = 0.85, w = 0.08, color = "#a8784a" },
	machete = { len = 0.6, w = 0.06, color = "#c8ccd0", metal = 0.9, roughness = 0.3 },
	axe = { len = 0.75, w = 0.08, color = "#7a4a2a" },
}

local ZOMBIES = {
	shambler = { hp = 3, sense = 9, reach = 1.35, windup = 0.55, strike = 0.2, recover = 0.8 },
	runner = { hp = 2, sense = 13, reach = 3.6, windup = 0.6, strike = 0.35, recover = 1.1, lunge = 8.5 },
}
local ZOMBIE_SKIN = { "#7d8f6a", "#8a9178", "#6f7f66", "#93977f" }
local ZOMBIE_SHIRTS = { "#5b5148", "#4d5560", "#6a4f45", "#56604a", "#6b6458", "#3f4650" }
local ZOMBIE_PANTS = { "#2f3238", "#3b3530", "#2a2f2a" }
local SKINS = { "#f1c9a5", "#d9a37e", "#b97d56", "#8d5a3a", "#5e3b26" }
local PANTS = { "#2b3a55", "#3a3a3a", "#4a3d2e", "#2e3f33" }

-- The time of day for each stop in a cycle of six; the first night is stop 5.
local TIMES = { "morning:haze", "noon:clear", "afternoon:haze", "sunset:haze", "night:clear", "night:mist" }

-- What a container holds, by kind: { weight, item, min, max }.
local LOOT = {
	kitchen = { { 32, "food", 1, 2 }, { 10, "meds", 1, 1 }, { 8, "ammo", 1, 1 }, { 50, nil } },
	closet = { { 28, "weapon" }, { 12, "ammo", 1, 2 }, { 12, "meds", 1, 1 }, { 8, "food", 1, 1 }, { 40, nil } },
	car = { { 32, "fuel", 1, 1 }, { 16, "food", 1, 1 }, { 10, "ammo", 1, 2 }, { 42, nil } },
	police = { { 60, "ammo", 2, 4 }, { 30, "shotgun" }, { 10, "meds", 1, 1 } },
	shed = { { 45, "weapon" }, { 30, "fuel", 1, 1 }, { 25, nil } },
}
local ITEM_COLORS = { ammo = "#e0b84a", food = "#7fc25a", fuel = "#e0573a", meds = "#f2f2f2" }

local s -- the run: state.s
local warm_all -- defined with the hooks, used when a stop is built
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

----------------------------------------------------------------------------
-- Collision against the street's boxes (a grid of 4 m cells)

local CELL = 4
local function cell_key(cx, cz) return (cx + 100) * 1000 + (cz + 100) end

local function build_grid()
	grid = {}
	for i, r in s.rects do
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
end

-- Push a circle out of every box it overlaps; returns the new position.
local function collide(p, r)
	local x, z = p.x, p.z
	for _ = 1, 2 do
		local list = grid[cell_key(math.floor(x / CELL), math.floor(z / CELL))]
		if list then
			for _, i in list do
				local b = s.rects[i]
				if not b.gone then
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
							-- Inside the box: out along the shortest side.
							local l, rr, t, bt = x - b.x0, b.x1 - x, z - b.z0, b.z1 - z
							local m = math.min(l, rr, t, bt)
							if m == l then x = b.x0 - r elseif m == rr then x = b.x1 + r elseif m == t then z = b.z0 - r else z = b.z1 + r end
						end
					end
				end
			end
		end
	end
	x = math.clamp(x, -HX + 0.5, HX - 0.5)
	z = math.clamp(z, -HZ + 0.5, HZ - 0.5)
	return v(x, 0, z)
end

-- Distance from a point to a box in the ground plane.
local function to_rect(p, b)
	local dx = math.max(b.x0 - p.x, 0, p.x - b.x1)
	local dz = math.max(b.z0 - p.z, 0, p.z - b.z1)
	return math.sqrt(dx * dx + dz * dz)
end

-- How far a shot along d from p travels before a wall.
local function wall_distance(p, d, max)
	local hit = world.raycast(v(p.x, 1.1, p.z), d, max, { tag = "wall" })
	return if hit then hit.distance else max
end

-- Walkers find their way through front doors and the gap in the inner
-- wall: the next point to walk to on the way from p to goal.
local function room(h, p)
	if p.x < h.x0 or p.x > h.x1 or p.z < h.z0 or p.z > h.z1 then return 0 end
	return if (p.z - h.zi) * h.side < 0 then 1 else 2
end

local function waypoint(p, goal)
	for _, h in s.houses do
		local a, b = room(h, p), room(h, goal)
		if a ~= b then
			if a == 0 then
				-- Outside, going in: to the door, then through it.
				if dist(p, h.door_out) > 0.8 and (p.z - h.zn) * h.side < -0.3 then return h.door_out end
				if b == 2 then return h.gap_front end
				return h.door_in
			elseif a == 1 then
				if b == 2 then return if dist(p, h.gap_front) > 0.6 then h.gap_front else h.gap_back end
				return if dist(p, h.door_in) > 0.6 then h.door_in else h.door_out
			else
				return if dist(p, h.gap_back) > 0.6 then h.gap_back else h.gap_front
			end
		end
	end
	return goal
end

----------------------------------------------------------------------------
-- Survivors

local function standing(u) return u.state == "ok" end
local function in_body(u) return u.state == "ok" or u.state == "down" end

local function survivor_look(u)
	return { shirt = u.color, skin = u.skin, pants = u.pants }
end

local function give_body(u, pos)
	u.pos = pos
	u.yaw = -math.pi / 2
	u.vel = v(0, 0, 0)
	u.hp, u.maxhp = 3, 3
	u.state = "ok"
	u.t = 0
	u.melee, u.dur = "fists", nil
	u.gun, u.gun_dur = "pistol", nil
	u.cd_melee, u.cd_gun, u.cd_shove, u.swing, u.shove_t = 0, 0, 0, 0, 0
	u.invul = 1
	u.search_t, u.revive_t, u.depart_t = 0, 0, 0
	u.skin = pick(SKINS)
	u.pants = pick(PANTS)
	u.fig = figure.build(pos, u.yaw, survivor_look(u))
	if u.ghost_ent then
		world.remove(u.ghost_ent)
		u.ghost_ent = nil
	end
end

local function new_survivor(p, i)
	local u = {
		id = p.id,
		name = p.name,
		color = p.color,
		bot = p.bot,
		lives = 3,
		aim = v(1, 0, 0),
		popup = nil,
	}
	local c = s.car_pos
	give_body(u, collide(v(c.x + 3 + (i - 1) * 0.9, 0, c.z + (if i % 2 == 0 then 1.6 else -1.6)), RADIUS))
	return u
end

local function hold_weapon(u)
	figure.hold(u.fig, if u.melee ~= "fists" then u.melee else nil, HELD)
end

local function popup(u, text, color)
	u.popup = { text = text, t = 1.6, color = color or "#ffffff" }
end

----------------------------------------------------------------------------
-- Zombies

local function zombie_look(z)
	return { shirt = z.shirt, skin = z.skin, pants = z.pants, eyes = "#1a1a1a" }
end

local function spawn_zombie(kind, pos, mode, goal)
	local z = {
		kind = kind,
		pos = collide(pos, RADIUS),
		yaw = random.range(-math.pi, math.pi),
		hp = ZOMBIES[kind].hp,
		state = mode or "idle",
		t = random.range(0, 3),
		goal = goal,
		knock = v(0, 0, 0),
		skin = pick(ZOMBIE_SKIN),
		shirt = pick(ZOMBIE_SHIRTS),
		pants = pick(ZOMBIE_PANTS),
		stuck = 0,
		side = 0,
	}
	if kind == "runner" then z.shirt = "#6e3a36" end
	z.fig = figure.build(z.pos, z.yaw, zombie_look(z))
	if z.state == "sleep" then figure.lie(z.fig, z.pos, z.yaw, true) end
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

-- A sound in the street: zombies within the radius walk to it, sleepers
-- close enough wake up, and the director's pressure rises.
local function noise(pos, radius, amount)
	count("noises")
	for _, z in s.zombies do
		if z.state ~= "dead" then
			local d = dist(z.pos, pos)
			if d < radius then
				if z.state ~= "sleep" or d < radius * 0.6 or radius >= 20 then
					wake(z, flat(pos))
				end
			end
		end
	end
	s.pressure = math.min(1.3, s.pressure + (amount or 0))
	if radius >= 10 then
		fx.ring(v(pos.x, 0.06, pos.z), { radius = radius, life = 0.7, color = "#e8dfc4", emissive = 0.05, width = 0.07, segments = 48 })
	end
end

local function kill_zombie(z, from)
	z.state = "dead"
	z.t = 15
	figure.eyes(z.fig, false, { color = "#1a1a1a", emissive = 0 })
	figure.lie(z.fig, z.pos, z.yaw, true)
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
	if z.hp <= 0 then
		kill_zombie(z, from)
		return
	end
	z.knock = away * (knock or 2)
	if z.state == "sleep" then
		z.state = "rise"
		z.t = 0.5
	elseif z.state ~= "rise" then
		-- A hit cancels a wind-up: hitting first is always a defence.
		z.state = "stagger"
		z.t = 0.35
	end
	figure.eyes(z.fig, false, { color = "#1a1a1a", emissive = 0 })
	fx.burst(v(z.pos.x, 1.1, z.pos.z), { color = "#6a2a20", count = 5, speed = 2.5, life = 0.35, size = 0.08, gravity = 9 })
end

----------------------------------------------------------------------------
-- Building a stop

local function stop_band(stop)
	local r = random.float()
	if stop == 1 then return if r < 0.6 then "quiet" else "busy" end
	local horde = math.min(0.4, 0.15 + 0.04 * stop)
	if r < horde then return "horde" end
	if r < horde + 0.45 then return "busy" end
	return "quiet"
end

local BAND = {
	quiet = { min = 6, max = 12, sleep = 0.5 },
	busy = { min = 18, max = 32, sleep = 0.55 },
	horde = { min = 48, max = 80, sleep = 0.8 },
}

local function night() return s.time_of_day:sub(1, 5) == "night" end

local function populate()
	local band = BAND[s.band]
	local scale = KNOBS.density * (1 + 0.08 * (s.stop - 1)) * (if night() then 1.3 else 1)
	local n = math.floor(random.int(band.min, band.max) * scale)
	local runners = math.min(0.35, 0.08 * (s.stop - 1))
	local spots = {}
	for _, p in s.layout.inside do table.insert(spots, { p = p, inside = true }) end
	for _, p in s.layout.outside do table.insert(spots, { p = p, inside = false }) end
	for i = 1, n do
		local spot = spots[(i - 1) % #spots + 1]
		local p = spot.p + v(random.range(-2, 2), 0, random.range(-2, 2))
		-- Keep the car clear at the start.
		if dist(p, s.car_pos) > 12 then
			local kind = if random.chance(runners) then "runner" else "shambler"
			local mode = if spot.inside or random.chance(band.sleep) then "sleep" else (if random.chance(0.5) then "idle" else "wander")
			spawn_zombie(kind, p, mode)
		end
	end
end

local function place_weapon(kind, pos, dur)
	local w = HELD[kind]
	if not w then return end
	local ent = world.spawn("cube", { pos = v(pos.x, 0.06, pos.z), rot = v(0, random.range(0, math.pi), 0), size = v(w.w, w.w, w.len), color = w.color, metal = w.metal, roughness = w.roughness or 0.6, solid = false, tag = "pickup" })
	local ring = world.spawn("outline16", { pos = v(pos.x, 0.03, pos.z), size = 1.1, color = "#f2e6b0", emissive = 0.12, solid = false, tag = "pickup" })
	table.insert(s.pickups, { kind = kind, pos = flat(pos), dur = dur or MELEE[kind].dur, ent = ent, ring = ring })
end

local function look_of_day()
	sky.set(s.time_of_day)
	if night() then
		light.set({ fog_start = 8, fog_end = 45 })
		post.set({ vignette = 0.55, grain = 0.12, saturation = 0.8, contrast = 1.08 })
	else
		lights.clear()
		post.set({ vignette = 0.35, grain = 0.08, saturation = 0.85, contrast = 1.05 })
	end
end

local function build_stop()
	world.clear()
	warm_all()
	s.time_of_day = TIMES[(s.stop - 1) % #TIMES + 1]
	s.day = math.floor((s.stop - 1) / #TIMES) + 1
	s.layout = street.build(s.stop)
	s.rects = s.layout.rects
	s.houses = s.layout.houses
	s.containers = s.layout.containers
	s.car_pos = s.layout.car
	local car, rect = street.car(s.car_pos)
	s.car = car
	table.insert(s.rects, rect)
	build_grid()
	s.zombies = {}
	s.pickups = {}
	for _, w in s.layout.weapons do place_weapon(w.kind, w.pos) end
	populate()
	for i, u in s.survivors do
		local was = u.state
		u.fig = nil
		local melee, dur, gun, gun_dur, hp = u.melee, u.dur, u.gun, u.gun_dur, u.hp
		give_body(u, collide(v(s.car_pos.x + 3 + (i - 1) * 0.9, 0, s.car_pos.z + (if i % 2 == 0 then 1.6 else -1.6)), RADIUS))
		if was == "ok" or was == "down" then
			-- The same survivor gets out, with their gear and wounds.
			u.melee, u.dur, u.gun, u.gun_dur = melee or "fists", dur, gun or "pistol", gun_dur
			u.hp = math.max(1, hp or 3)
		end
		if u.hungry then
			u.hp = math.max(1, u.hp - 1)
			u.hungry = false
		end
		hold_weapon(u)
	end
	s.pressure = 0.1
	s.dir = { mode = "build", t = 0, spawn_t = 6 }
	s.stop_time = 0
	s.cam = nil
	s.phase = "play"
	look_of_day()
end

local function new_run()
	state.s = {
		stop = 1,
		day = 1,
		ammo = KNOBS.start_ammo,
		food = KNOBS.start_food,
		fuel = KNOBS.start_fuel,
		meds = 1,
		kills = 0,
		survivors = {},
		counts = {},
		phase = "play",
		pressure = 0,
		band = "quiet",
		time_of_day = TIMES[1],
		best = (state.s and state.s.best) or 0,
		runs = ((state.s and state.s.runs) or 0) + 1,
	}
	s = state.s
	s.band = stop_band(1)
	-- A first layout so survivors have somewhere to stand.
	s.car_pos = v(-HX + 6, 0, 1.8)
	for i, p in players.list() do
		table.insert(s.survivors, { id = p.id, name = p.name, color = p.color, bot = p.bot, lives = 3, aim = v(1, 0, 0), state = "ghost" })
	end
	build_stop()
end

----------------------------------------------------------------------------
-- Intents: what a person's controller or a bot wants this tick

local function from_input(i)
	if not i then return { mx = 0, mz = 0, ax = 0, az = 0 } end
	return {
		mx = i.move_x, mz = -i.move_y, ax = i.aim_x, az = -i.aim_y,
		fire = i.held.right_trigger, melee = i.held.west or i.held.right_shoulder,
		shove = i.pressed.east, use = i.held.south, use_pressed = i.pressed.south,
		med = i.pressed.north,
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

local function near_car(p, extra)
	return to_rect(p, s.rects[#s.rects]) < (extra or 1.6)
end

-- A bot: revive, fight what is close, search the street, then drive on.
local function bot_intent(u)
	local it = { mx = 0, mz = 0, ax = 0, az = 0 }
	local humans = {}
	for _, o in s.survivors do
		if not o.bot and KNOBS.autopilot ~= 1 and in_body(o) then table.insert(humans, o) end
	end
	if u.state == "ghost" then
		-- Drift near a teammate and moan to pull zombies off them.
		local friend = humans[1] or s.survivors[1]
		if friend and friend ~= u and friend.pos then
			it.mx, it.mz = steer(u, friend.pos + v(4, 0, 3))
		end
		local z, d = nearest_zombie(u.pos, 6, true)
		it.use_pressed = z ~= nil and d < 6 and random.chance(0.05)
		return it
	end
	if u.state == "down" then return it end

	-- Stay with the people.
	if #humans > 0 then
		local h, hd = nil, math.huge
		for _, o in humans do
			if o ~= u then
				local d = dist(o.pos, u.pos)
				if d < hd then h, hd = o, d end
			end
		end
		if h and hd > 14 then
			it.mx, it.mz = steer(u, h.pos)
			return it
		end
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

	-- Fight.
	local z, d = nearest_zombie(u.pos, 7, true)
	if z then
		local to = z.pos - u.pos
		it.ax, it.az = to.x, to.z
		local w = MELEE[u.melee]
		local close = 0
		for _, o in s.zombies do
			if o.state ~= "dead" and o.state ~= "sleep" and dist(o.pos, u.pos) < 5 then close += 1 end
		end
		if d < 1.8 and z.state == "windup" and u.cd_shove <= 0 then
			it.shove = true
		elseif d < w.range - 0.1 then
			it.melee = true
		elseif s.ammo > 0 and ((z.kind == "runner" and d < 7) or (close >= 4 and s.ammo > 2)) then
			it.fire = true
		elseif d < 4 then
			-- Step in to swing.
			it.mx, it.mz = to.x / d, to.z / d
		end
		if close >= 6 and u.hp <= 1 then
			-- Too many: back off towards the car.
			it.mx, it.mz = steer(u, s.car_pos)
			it.fire = s.ammo > 0
		end
		if u.hp <= 1 and s.meds > 0 then it.med = true end
		if it.melee or it.fire or it.shove or it.mx ~= 0 then return it end
	end

	-- A better weapon nearby.
	for _, p in s.pickups do
		if RANK[p.kind] > RANK[u.melee] and dist(p.pos, u.pos) < 9 then
			if dist(p.pos, u.pos) < 0.9 then
				it.use_pressed = true
			else
				it.mx, it.mz = steer(u, p.pos)
			end
			return it
		end
	end

	-- Leave when the street is searched, the clock runs, or the people wait at the car.
	-- A person holding A at the car calls everyone back.
	local called = false
	for _, h in humans do
		if h.calling then called = true end
	end
	local leaving = (s.stop_time > 110 or called) and s.fuel > 0
	if not leaving then
		-- The nearest unsearched container nobody else is on.
		local best, bd = nil, math.huge
		for ci, c in s.containers do
			if not c.searched and (c.claim == nil or c.claim == u.id) then
				local dd = dist(v(c.x, 0, c.z), u.pos)
				if dd < bd then best, bd = ci, dd end
			end
		end
		if best then
			local c = s.containers[best]
			c.claim = u.id
			if to_rect(u.pos, c.rect) < 0.75 then
				it.use = true
			else
				-- Walk to the side of the box that faces the room.
				local goal = v(c.x, 0, c.z)
				local cx = (c.rect.x0 + c.rect.x1) / 2
				local w = c.rect.x1 - c.rect.x0
				local dz = c.rect.z1 - c.rect.z0
				if w < dz then
					-- Against a side wall: stand beside it in the room.
					local inside = 0
					for _, h in s.houses do
						if room(h, goal) ~= 0 then inside = if math.abs(cx - h.x0) < math.abs(cx - h.x1) then 1 else -1 end
					end
					goal = v(cx + inside * (w / 2 + 0.5), 0, c.z)
				else
					goal = v(c.x, 0, c.z + (if c.z > 0 then -1 else 1) * (dz / 2 + 0.5))
				end
				it.mx, it.mz = steer(u, goal)
			end
			return it
		end
		leaving = s.fuel > 0
	end
	if leaving then
		if near_car(u.pos, 1.2) then
			it.use = #humans == 0
		else
			it.mx, it.mz = steer(u, s.car_pos + v(1.5, 0, if u.pos.z > s.car_pos.z then 1.6 else -1.6))
		end
	end
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
	u.hp -= 1
	u.invul = 0.8
	u.search_t, u.revive_t, u.depart_t = 0, 0, 0
	local away = u.pos - from
	local d = vector.magnitude(away)
	if d > 0.01 then u.vel = away / d * 4 end
	players.rumble(u.id, 0.5, 0.25)
	sfx("hurt", u.pos)
	fx.burst(v(u.pos.x, 1.1, u.pos.z), { color = "#8a1a14", count = 6, speed = 2.5, life = 0.35, size = 0.08, gravity = 9 })
	if u.hp <= 0 then
		u.state = "down"
		u.t = KNOBS.downed
		u.revive_t = 0
		figure.lie(u.fig, u.pos, u.yaw, true)
		sfx("down", u.pos)
		count("downs")
		game.log(u.name .. " is down")
	end
end

local function drop_melee(u)
	if u.melee ~= "fists" then place_weapon(u.melee, u.pos + v(random.range(-0.4, 0.4), 0, random.range(-0.4, 0.4)), u.dur) end
	u.melee, u.dur = "fists", nil
end

local function die(u)
	drop_melee(u)
	-- The body stays where it fell, dark and still.
	for _, id in { u.fig.torso } do world.set(id, figure.CORPSE) end
	u.corpse = u.fig
	u.fig = nil
	count("deaths")
	local alone = #s.survivors == 1
	if alone then
		u.lives -= 1
		u.state = "ghost"
		u.t = 0.5
	else
		u.state = "ghost"
		u.t = KNOBS.ghost
	end
	u.ghost_ent = world.spawn("sphere", { pos = v(u.pos.x, 1.4, u.pos.z), size = 0.55, color = "#cfe6ff", emissive = 0.35, solid = false, tag = "ghost" })
	u.cd_moan = 0
	sfx("moan", u.pos, { pitch = 1.3 })
end

local function respawn(u)
	-- A new survivor walks out from beside the car.
	local c = s.car_pos
	give_body(u, collide(v(c.x + 2.5, 0, c.z + random.range(-2, 2)), RADIUS))
	if random.chance(0.4) then
		u.melee, u.dur = "pipe", MELEE.pipe.dur
	end
	hold_weapon(u)
	count("respawns")
	u.invul = 2
	sfx("pickup", u.pos, { pitch = 0.7 })
	fx.ring(v(u.pos.x, 0.05, u.pos.z), { radius = 1.5, life = 0.5, color = u.color, emissive = 0.2 })
end

local function swing(u)
	local w = MELEE[u.melee]
	u.cd_melee = w.cd
	u.swing = 0.0001
	count("swings")
	local f = dir(u.yaw)
	local hits = {}
	for _, z in s.zombies do
		if z.state ~= "dead" then
			local to = z.pos - u.pos
			local d = math.sqrt(to.x * to.x + to.z * to.z)
			if d < w.range and d > 0.01 and (to.x * f.x + to.z * f.z) / d > w.arc then
				table.insert(hits, { z = z, d = d })
			end
		end
	end
	table.sort(hits, function(a, b) return a.d < b.d end)
	if #hits == 0 then count("misses") end
	local hit = 0
	for i = 1, math.min(#hits, w.cleave) do
		hurt_zombie(hits[i].z, w.dmg, u.pos, w.knock)
		hit += 1
		count("melee_hits")
	end
	sfx(if hit > 0 then "hit" else "swing", u.pos, { pitch = random.range(0.9, 1.1) })
	if hit > 0 then
		noise(u.pos, w.noise, 0.005)
		if u.dur then
			u.dur -= 1
			if u.dur <= 0 then
				-- It breaks: a crack and the pieces fly.
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
	-- Aim help: towards the nearest awake zombie within 22 degrees.
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
		local a = base + random.range(-g.spread, g.spread)
		local d = dir(a)
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
		-- A tracer: a thin bright streak along the shot.
		fx.burst(v(u.pos.x + d.x * td, 1.1, u.pos.z + d.z * td), { color = "#fff2c0", emissive = 0.6, count = 3, speed = 2, life = 0.12, size = 0.05 })
	end
	fx.burst(v(u.pos.x + f.x * 0.6, 1.2, u.pos.z + f.z * 0.6), { color = "#ffd27a", emissive = 1.2, count = 5, speed = 3, life = 0.08, size = 0.07 })
	sfx(if u.gun == "shotgun" then "shotgun" else "gun", u.pos)
	players.rumble(u.id, 0.35, 0.1)
	noise(u.pos, g.noise, if u.gun == "shotgun" then 0.14 else 0.09)
	if u.gun_dur then
		u.gun_dur -= 1
		if u.gun_dur <= 0 then
			popup(u, "shotgun broke", "#ff9a7a")
			sfx("crack", u.pos)
			u.gun, u.gun_dur = "pistol", nil
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
				figure.eyes(z.fig, false, { color = "#1a1a1a", emissive = 0 })
			end
		end
	end
	sfx("shove", u.pos)
	noise(u.pos, 3, 0)
end

local function roll(kind)
	local table_ = LOOT[kind]
	local total = 0
	for _, e in table_ do total += e[1] end
	local r = random.range(0, total)
	for _, e in table_ do
		r -= e[1]
		if r <= 0 then return e end
	end
	return table_[#table_]
end

local function search_done(u, c)
	c.searched = true
	world.set(c.id, { color = "#3b3631", roughness = 0.8, metal = 0, emissive = 0 })
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
		local kinds = if s.stop >= 3 then { "pipe", "bat", "machete", "axe" } else { "pipe", "bat", "machete" }
		place_weapon(pick(kinds), u.pos + (v(c.x, 0, c.z) - u.pos) * 0.5)
		popup(u, "a weapon", "#f2e6b0")
	elseif item == "shotgun" then
		u.gun, u.gun_dur = "shotgun", GUNS.shotgun.dur
		popup(u, "shotgun", "#e0b84a")
		count("shotguns")
	else
		local n = random.int(e[3], e[4])
		s[item] += n
		popup(u, "+" .. n .. " " .. item, ITEM_COLORS[item])
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

local function depart()
	count("stops")
	s.score = s.stop
	s.best = math.max(s.best, s.stop)
	sfx("car", s.car_pos)
	game.log("Dead Miles: left stop " .. s.stop .. " with " .. s.ammo .. " ammo, " .. s.food .. " food, " .. (s.fuel - 1) .. " fuel")
	-- The road: fuel and food for the leg.
	s.fuel -= 1
	local mouths = #s.survivors
	local short = math.max(0, mouths - s.food)
	s.food = math.max(0, s.food - mouths)
	local hungry = 0
	for i = #s.survivors, 1, -1 do
		local u = s.survivors[i]
		if hungry < short then
			u.hungry = true
			hungry += 1
		end
	end
	s.stop += 1
	s.band = stop_band(s.stop)
	s.phase = "road"
	s.road = { t = 3, short = short }
end

local function act(u, it, dt)
	u.invul = math.max(0, u.invul - dt)
	u.cd_melee = math.max(0, u.cd_melee - dt)
	u.cd_gun = math.max(0, u.cd_gun - dt)
	u.cd_shove = math.max(0, u.cd_shove - dt)
	u.shove_t = math.max(0, u.shove_t - dt)
	if u.side_t then u.side_t -= dt end
	if u.popup then
		u.popup.t -= dt
		if u.popup.t <= 0 then u.popup = nil end
	end

	if u.state == "ghost" then
		u.t -= dt
		u.cd_moan = math.max(0, (u.cd_moan or 0) - dt)
		local m = v(it.mx, 0, it.mz)
		u.pos = u.pos + m * 5 * dt
		u.pos = v(math.clamp(u.pos.x, -HX, HX), 0, math.clamp(u.pos.z, -HZ, HZ))
		world.place(u.ghost_ent, v(u.pos.x, 1.4 + math.sin(game.time() * 3) * 0.15, u.pos.z))
		if it.use_pressed and u.cd_moan <= 0 then
			-- A moan pulls the dead nearby away from the living.
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
		-- Crawl, slowly.
		u.pos = collide(u.pos + mv * 0.6 * dt, RADIUS)
		figure.lie(u.fig, u.pos, u.yaw, true)
		if u.t <= 0 then die(u) end
		return
	end

	-- Aim: the right stick, else the way you walk.
	local am = it.ax * it.ax + it.az * it.az
	if am > 0.09 then
		u.aim = v(it.ax, 0, it.az)
	elseif m > 0.2 then
		u.aim = mv
	end
	local want = yaw_of(u.aim)
	local dy = (want - u.yaw + math.pi) % (2 * math.pi) - math.pi
	u.yaw += math.clamp(dy, -14 * dt, 14 * dt)

	-- Searching, reviving and departing hold you still.
	local busy = false
	if it.use then
		-- Revive first.
		for _, o in s.survivors do
			if o ~= u and o.state == "down" and dist(o.pos, u.pos) < 1.3 then
				busy = true
				u.revive_t += dt
				if u.revive_t >= KNOBS.revive then
					o.state = "ok"
					o.hp = 1
					o.invul = 1.5
					figure.lie(o.fig, o.pos, o.yaw, false)
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
				u.searching = best
				if u.search_t >= KNOBS.search then
					u.search_t = 0
					search_done(u, best)
				end
			end
		end
		if not busy and near_car(u.pos) then
			busy = true
			u.calling = true
			local all = true
			for _, o in s.survivors do
				if standing(o) and not near_car(o.pos, 5) then all = false end
			end
			if s.fuel <= 0 then
				u.depart_t = 0
				if not u.fuel_warned then
					popup(u, "no fuel", ITEM_COLORS.fuel)
					u.fuel_warned = true
				end
			elseif all then
				u.depart_t += dt
				if u.depart_t >= 1 then
					depart()
					return
				end
			else
				u.depart_t = 0
			end
		end
	else
		u.fuel_warned = false
	end
	if not busy then u.search_t, u.revive_t, u.depart_t = 0, 0, 0 end
	if not (it.use and near_car(u.pos)) then u.calling = false end

	-- Pick up a weapon from the ground.
	if it.use_pressed then
		for i, p in s.pickups do
			if dist(p.pos, u.pos) < 1.1 then
				local old, old_dur = u.melee, u.dur
				u.melee, u.dur = p.kind, p.dur
				world.remove(p.ent)
				world.remove(p.ring)
				table.remove(s.pickups, i)
				if old ~= "fists" then place_weapon(old, u.pos, old_dur) end
				hold_weapon(u)
				sfx("pickup", u.pos, { pitch = 1.2 })
				count("pickups")
				break
			end
		end
	end

	if it.med and s.meds > 0 and u.hp < u.maxhp then
		s.meds -= 1
		u.hp = math.min(u.maxhp, u.hp + 2)
		sfx("revive", u.pos, { pitch = 1.3 })
		popup(u, "patched up", ITEM_COLORS.meds)
		count("meds_used")
	end

	if not busy then
		if it.shove and u.cd_shove <= 0 then
			shove(u)
		elseif it.melee and u.cd_melee <= 0 then
			swing(u)
		elseif it.fire and u.cd_gun <= 0 then
			shoot(u)
		end
	end

	-- Move: slower while swinging, still while busy.
	local speed = KNOBS.walk * (if busy then 0 else (if u.cd_melee > 0.2 then 0.6 else 1))
	local go = mv * speed + u.vel
	u.vel = u.vel * math.max(0, 1 - 10 * dt)
	local np = u.pos + go * dt
	-- The leash: nobody walks out of the shared view.
	if s.center then
		local away = np - s.center
		local d = math.sqrt(away.x * away.x + away.z * away.z)
		if d > LEASH and d > dist(u.pos, s.center) then
			np = s.center + away / d * math.max(LEASH, dist(u.pos, s.center))
		end
	end
	local before = u.pos
	u.pos = collide(np, RADIUS)
	-- Bots that walk into a wall step sideways for a moment.
	if u.bot and m > 0.5 and dist(before, u.pos) < speed * dt * 0.2 then
		u.stuck = (u.stuck or 0) + dt
		if u.stuck > 0.5 then
			u.side_t, u.side_dir, u.stuck = 0.8, if random.chance(0.5) then 1 else -1, 0
		end
	else
		u.stuck = 0
	end

	-- The body.
	figure.place(u.fig, u.pos, u.yaw)
	local moved = dist(before, u.pos) / dt
	if u.cd_melee > 0 and u.swing > 0 then
		local w = MELEE[u.melee]
		figure.animate(u.fig, dt, moved, "swing_r", 1 - u.cd_melee / w.cd)
	elseif u.shove_t > 0 then
		figure.animate(u.fig, dt, moved, "shove")
	elseif it.fire or u.cd_gun > 0.2 then
		figure.animate(u.fig, dt, moved, "aim")
	else
		figure.animate(u.fig, dt, moved, "swing")
	end
end

----------------------------------------------------------------------------
-- Zombie behaviour

local zcells = {}
local function zkey(p) return cell_key(math.floor(p.x / 1.5), math.floor(p.z / 1.5)) end

local function zombie_step(z, dt)
	local spec = ZOMBIES[z.kind]
	local speed = if z.kind == "runner" then KNOBS.runner else KNOBS.shambler
	z.t -= dt
	if z.state == "dead" then
		if z.t <= 0 and z.fig then
			figure.remove(z.fig)
			z.fig = nil
		end
		return
	end
	if z.state == "sleep" then
		for _, u in s.survivors do
			if in_body(u) and dist(u.pos, z.pos) < 2.2 then wake(z, flat(u.pos)) end
		end
		return
	end

	-- Who to go for: the nearest survivor in a body within its senses.
	local target, td = nil, spec.sense * (if night() then 1.2 else 1)
	for _, u in s.survivors do
		if in_body(u) then
			local d = dist(u.pos, z.pos)
			if d < td then target, td = u, d end
		end
	end

	local move = v(0, 0, 0)
	local arms = "reach"
	if z.state == "rise" then
		if z.t <= 0 then
			figure.lie(z.fig, z.pos, z.yaw, false)
			z.state = if target then "chase" else "investigate"
		end
		return
	elseif z.state == "stagger" then
		move = z.knock
		z.knock = z.knock * math.max(0, 1 - 6 * dt)
		if z.t <= 0 then z.state = if target then "chase" else "investigate" end
	elseif z.state == "windup" then
		arms = "raise"
		if z.target then
			local to = z.target.pos - z.pos
			z.yaw = yaw_of(to)
		end
		if z.t <= 0 then
			z.state = "strike"
			z.t = spec.strike
			z.hit = false
			figure.eyes(z.fig, false, { color = "#1a1a1a", emissive = 0 })
			if z.kind == "runner" then count("lunges") end
		end
	elseif z.state == "strike" then
		arms = "reach"
		local f = dir(z.yaw)
		if z.kind == "runner" then
			move = f * spec.lunge
		end
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
		arms = "reach"
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
			else
				local reach = spec.reach
				if td < reach and z.t <= 0 then
					-- The tell: arms up, eyes lit, a ring on the ground.
					z.state = "windup"
					z.t = spec.windup
					z.target = target
					figure.eyes(z.fig, true)
					fx.ring(v(z.pos.x, 0.05, z.pos.z), { radius = if z.kind == "runner" then 1.2 else reach, life = spec.windup, color = "#ff3a2a", emissive = 0.25, width = 0.06, segments = 24 })
					if random.chance(0.4) then sfx("groan", z.pos, { pitch = random.range(1.1, 1.3) }) end
				else
					local goal = waypoint(z.pos, target.pos)
					local to = goal - z.pos
					local d = math.sqrt(to.x * to.x + to.z * to.z)
					if d > 0.05 then move = to / d * speed end
				end
			end
		end
		if z.state == "investigate" then
			if z.goal and dist(z.goal, z.pos) > 1 then
				local goal = waypoint(z.pos, z.goal)
				local to = goal - z.pos
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

	-- Keep apart from the others in the same and nearby cells.
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
	local wanted = move + push
	z.pos = collide(z.pos + wanted * dt, RADIUS)
	local mv = vector.magnitude(move)
	if mv > 0.3 and z.state ~= "strike" then
		z.yaw = yaw_of(move)
		-- Stuck on a wall: try sliding one way for a second.
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
	figure.place(z.fig, z.pos, z.yaw)
	figure.animate(z.fig, dt, dist(before, z.pos) / dt, arms)
end

local function spawn_edge(kind, mode, goal)
	-- A point on the edge of the lot, far from everyone.
	for _ = 1, 8 do
		local p
		local side = random.int(1, 4)
		if side == 1 then p = v(HX - 1, 0, random.range(-HZ + 2, HZ - 2))
		elseif side == 2 then p = v(random.range(-HX + 12, HX - 2), 0, -HZ + 1)
		elseif side == 3 then p = v(random.range(-HX + 12, HX - 2), 0, HZ - 1)
		else p = v(random.range(-HX + 14, HX - 2), 0, if random.chance(0.5) then -3 else 3) end
		local ok = true
		for _, u in s.survivors do
			if u.pos and dist(u.pos, p) < 16 then ok = false end
		end
		if ok then return spawn_zombie(kind, p, mode, goal) end
	end
	return nil
end

-- The director: pressure rises with time and noise; at the top a horde
-- comes from the edge, then a calm. A hurt group gets breathing room.
local function director(dt)
	local d = s.dir
	d.t -= dt
	local awake, hp, maxhp = 0, 0, 0
	for _, z in s.zombies do
		if z.state ~= "dead" and z.state ~= "sleep" then awake += 1 end
	end
	for _, u in s.survivors do
		maxhp += 3
		if in_body(u) then hp += u.hp end
	end
	local relief = maxhp > 0 and hp / maxhp < 0.35
	s.relief = relief
	if d.mode == "calm" then
		if d.t <= 0 then d.mode = "build" end
		return
	end
	s.pressure = math.min(1.3, s.pressure + dt * KNOBS.pressure * (1 + 0.15 * s.stop) * (if relief then 0.4 else 1))
	if d.mode == "build" then
		d.spawn_t -= dt
		local cap = 14 + 5 * s.stop
		if d.spawn_t <= 0 then
			d.spawn_t = 9 - 6 * math.min(1, s.pressure) + random.range(-1, 1)
			if awake < cap and not relief then
				local kind = if random.chance(math.min(0.3, 0.06 * s.stop)) then "runner" else "shambler"
				spawn_edge(kind, "investigate", s.center)
				count("trickle")
			end
		end
		if s.pressure >= 1 and not relief then
			d.mode = "peak"
			d.t = KNOBS.peak
			d.peak_left = 6 + 2 * s.stop
			count("peaks")
			sfx("scream", s.center or v(0, 0, 0))
		end
	elseif d.mode == "peak" then
		if (d.peak_left or 0) > 0 and game.tick() % 20 == 0 then
			local kind = if random.chance(math.min(0.45, 0.1 + 0.05 * s.stop)) then "runner" else "shambler"
			if spawn_edge(kind, "investigate", s.center) then d.peak_left -= 1 end
		end
		if d.t <= 0 then
			d.mode = "calm"
			d.t = KNOBS.calm
			s.pressure = 0.25
		end
	end
end

----------------------------------------------------------------------------
-- The camera and the night

local function group_center()
	local sum, n = v(0, 0, 0), 0
	for _, u in s.survivors do
		if standing(u) then
			sum = sum + u.pos
			n += 1
		end
	end
	if n == 0 then
		for _, u in s.survivors do
			if u.pos then
				sum = sum + u.pos
				n += 1
			end
		end
	end
	if n == 0 then return s.car_pos, 0 end
	local c = sum / n
	local spread = 0
	for _, u in s.survivors do
		if standing(u) then spread = math.max(spread, dist(u.pos, c)) end
	end
	return c, spread
end

local function camera_step(dt)
	local c, spread = group_center()
	s.center = c
	if not s.cam then s.cam = c end
	s.cam = s.cam + (c - s.cam) * math.min(1, dt * 3)
	s.spread = (s.spread or spread) + (spread - (s.spread or spread)) * math.min(1, dt * 2)
	local k = 1 + math.max(0, s.spread - 5) * 0.045
	local target = v(s.cam.x, 0.5, s.cam.z)
	local pos = target + v(0, KNOBS.cam_height * k, KNOBS.cam_back * k)
	local any = false
	for _, p in players.list() do
		if p.human then
			camera.set(p.id, { pos = pos, target = target, fov = KNOBS.fov })
			any = true
		end
	end
	if not any then
		local first = players.list()[1]
		if first then camera.set(first.id, { pos = pos, target = target, fov = KNOBS.fov }) end
	end
end

----------------------------------------------------------------------------
-- Hooks

function warm_all()
	street.warm()
	for _, c in ZOMBIE_SKIN do figure.warm({ shirt = c, skin = c, pants = c }) end
	for _, c in ZOMBIE_SHIRTS do figure.warm({ shirt = c, skin = c, pants = c }) end
	for _, c in ZOMBIE_PANTS do figure.warm({ shirt = c, skin = c, pants = c }) end
	figure.warm({ shirt = "#6e3a36", skin = "#6e3a36", pants = "#6e3a36" })
	for _, c in SKINS do figure.warm({ shirt = c, skin = c, pants = c }) end
	for _, c in PANTS do figure.warm({ shirt = c, skin = c, pants = c }) end
	for _, p in players.list() do figure.warm({ shirt = p.color, skin = p.color, pants = p.color }) end
	world.warm("cube", figure.EYE_GLOW)
	world.warm("cube", { color = "#1a1a1a", emissive = 0 })
	world.warm("cube", figure.CORPSE)
	world.warm("sphere", { color = "#cfe6ff", emissive = 0.35 })
	world.warm("cube", { color = "#3b3631", roughness = 0.8 })
	world.warm("outline16", { color = "#f2e6b0", emissive = 0.12 })
	for _, w in HELD do
		world.warm("cube", { color = w.color, metal = w.metal, roughness = w.roughness or 0.6 })
	end
	-- Effects: bursts and rings in exactly the looks they spawn with.
	for _, c in { "#5a1e18", "#6a2a20", "#8a1a14" } do world.warm("cube", { color = c, emissive = 0.06 }) end
	world.warm("cube", { color = "#fff2c0", emissive = 0.6 })
	world.warm("cube", { color = "#ffd27a", emissive = 1.2 })
	world.warm("cube", { color = "#e8dfc4", emissive = 0.05 })
	world.warm("cube", { color = "#ff3a2a", emissive = 0.25 })
	world.warm("cube", { color = "#f2e6b0", emissive = 0.3 })
	world.warm("cube", { color = "#3b3631", roughness = 0.8, metal = 0, emissive = 0 })
	for _, c in ITEM_COLORS do world.warm("cube", { color = c, emissive = 0.3 }) end
	for _, p in players.list() do world.warm("cube", { color = p.color, emissive = 0.2 }) end
end

local function init()
	warm_all()
	new_run()
end

local function reload()
	s = state.s
	if s and s.rects then build_grid() end
	if s then look_of_day() end
end

local function tick(dt, input)
	s = state.s
	if s.phase == "road" then
		s.road.t -= dt
		if s.road.t <= 0 then build_stop() end
		return
	end
	if s.phase == "over" then
		s.over_t -= dt
		-- The dead keep shuffling behind the result.
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
		local again = s.over_t <= 0
		for _, u in s.survivors do
			local i = not u.bot and input[u.id]
			if i and i.pressed.south and s.over_t < 3 then again = true end
		end
		if again then
			new_run()
		end
		return
	end

	s.stop_time += dt
	camera_step(dt)

	for _, u in s.survivors do
		local it = if u.bot or KNOBS.autopilot == 1 then bot_intent(u) else from_input(input[u.id])
		act(u, it, dt)
		if s.phase ~= "play" then return end
	end

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
	-- Drop spent corpses from the list now and then.
	if game.tick() % 120 == 0 then
		local keep = {}
		for _, z in s.zombies do
			if z.state ~= "dead" or z.fig then table.insert(keep, z) end
		end
		s.zombies = keep
	end
	director(dt)

	-- A groan now and then from someone close.
	if game.tick() % 50 == 0 then
		local z = nearest_zombie(s.center or s.car_pos, 12, true)
		if z then sfx("groan", z.pos, { pitch = random.range(0.75, 1.1), db = -6 }) end
	end

	-- The run ends only when nobody is standing at once.
	local alone = #s.survivors == 1 and s.survivors[1].lives > 0
	if standing_count() == 0 and #s.survivors > 0 and not alone then
		-- Nobody stands, so nobody can revive: the run is over.
		s.phase = "over"
		s.over_t = 7
		s.best = math.max(s.best, s.stop - 1)
		count("wipes")
		sfx("down", s.center or s.car_pos, { pitch = 0.6 })
		game.log("Dead Miles: overrun at stop " .. s.stop .. ", " .. s.kills .. " dead put down")
	end
end

----------------------------------------------------------------------------
-- The HUD: stocks in a corner, health and wear under each survivor.

local function icon(x, y, k, color, n)
	hud.box(x, y + 4 * k, 16 * k, 16 * k, color)
	hud.text(tostring(n), x + 22 * k, y, { size = 22 * k, color = if n == 0 then "#ff6a4a" else "#f4efe6" })
end

local function bar(x, y, w, h, frac, color)
	hud.box(x - 1, y - 1, w + 2, h + 2, { 0, 0, 0, 0.55 })
	hud.box(x, y, w * math.clamp(frac, 0, 1), h, color)
end

local function draw()
	s = state.s
	if not s then return end
	local W, H = hud.size()
	local k = H / 720

	-- Torches at night: a light ahead of every standing survivor, the car's lamps.
	if night() then
		local list = {}
		for _, u in s.survivors do
			if in_body(u) and u.pos then
				local f = dir(u.yaw)
				table.insert(list, { pos = v(u.pos.x + f.x * 2.2, 1.7, u.pos.z + f.z * 2.2), color = "#ffe2b0", intensity = KNOBS.torch, range = 11 })
				table.insert(list, { pos = v(u.pos.x, 2.2, u.pos.z), color = "#ffe2b0", intensity = KNOBS.torch * 0.25, range = 5 })
			end
		end
		if s.car_pos then
			table.insert(list, { pos = v(s.car_pos.x + 4, 0.8, s.car_pos.z), color = "#fff0c8", intensity = 260, range = 12 })
		end
		lights.set(list)
	end

	if s.phase == "road" then
		hud.box(0, 0, W, H, { 0.02, 0.02, 0.03, 0.92 })
		hud.text("STOP " .. s.stop, 0, H * 0.38, { size = 64 * k, align = "center", width = W, color = "#f4efe6" })
		local band = if s.band == "horde" then "a horde street" elseif s.band == "busy" then "a busy street" else "a quiet street"
		hud.text(band, 0, H * 0.38 + 78 * k, { size = 26 * k, align = "center", width = W, color = "#b8b0a0" })
		if s.road.short > 0 then
			hud.text("not enough food", 0, H * 0.38 + 118 * k, { size = 22 * k, align = "center", width = W, color = "#ff8a6a" })
		end
		return
	end

	-- Stocks, top left.
	local x, y = 24 * k, 18 * k
	hud.box(x - 10 * k, y - 8 * k, 330 * k, 44 * k, { 0, 0, 0, 0.4 })
	icon(x, y, k, ITEM_COLORS.ammo, s.ammo)
	icon(x + 70 * k, y, k, ITEM_COLORS.food, s.food)
	icon(x + 140 * k, y, k, ITEM_COLORS.fuel, s.fuel)
	icon(x + 210 * k, y, k, ITEM_COLORS.meds, s.meds)
	hud.text("STOP " .. s.stop, W - 200 * k, y, { size = 22 * k, width = 180 * k, align = "right", color = "#f4efe6" })

	-- Each survivor: health pips and weapon wear under their feet.
	for _, u in s.survivors do
		if u.pos then
			local px, py = hud.project(v(u.pos.x, 0, u.pos.z))
			if px then
				local c = { u.color.x, u.color.y, u.color.z, 1 }
				if u.state == "ok" then
					for i = 1, u.maxhp do
						hud.box(px - 22 * k + (i - 1) * 16 * k, py + 10 * k, 12 * k, 6 * k, if i <= u.hp then c else { 0.15, 0.15, 0.15, 0.8 })
					end
					if u.dur then bar(px - 22 * k, py + 20 * k, 44 * k, 3 * k, u.dur / MELEE[u.melee].dur, "#e8e0c8") end
					local prog = math.max(u.search_t / KNOBS.search, u.revive_t / KNOBS.revive, u.depart_t)
					if prog > 0 then bar(px - 30 * k, py - 110 * k, 60 * k, 6 * k, prog, c) end
				elseif u.state == "down" then
					bar(px - 30 * k, py + 12 * k, 60 * k, 6 * k, u.t / KNOBS.downed, "#ff4a3a")
				elseif u.state == "ghost" then
					local gx, gy = hud.project(v(u.pos.x, 1.4, u.pos.z))
					if gx then bar(gx - 24 * k, gy + 26 * k, 48 * k, 4 * k, 1 - u.t / KNOBS.ghost, c) end
				end
				if u.popup then
					local a = math.min(1, u.popup.t * 2)
					local col = u.popup.color
					hud.text(u.popup.text, px - 100 * k, py - 140 * k - (1.6 - u.popup.t) * 20 * k, { size = 20 * k, align = "center", width = 200 * k, color = col })
				end
			end
		end
	end

	-- An empty tank shows over the car.
	if s.fuel <= 0 and s.car_pos then
		local cx, cy = hud.project(v(s.car_pos.x, 2, s.car_pos.z))
		if cx and math.floor(game.time() * 2) % 2 == 0 then
			hud.box(cx - 10 * k, cy - 10 * k, 20 * k, 20 * k, ITEM_COLORS.fuel)
		end
	end

	if s.phase == "over" then
		hud.box(0, H * 0.34, W, 150 * k, { 0, 0, 0, 0.6 })
		hud.text("OVERRUN", 0, H * 0.34 + 22 * k, { size = 58 * k, align = "center", width = W, color = "#ff6a4a" })
		hud.text("reached stop " .. s.stop .. "   best " .. math.max(s.best, s.stop), 0, H * 0.34 + 94 * k, { size = 24 * k, align = "center", width = W, color = "#e8e0d0" })
	end
end

local function player_joined(p)
	s = state.s
	figure.warm({ shirt = p.color, skin = p.color, pants = p.color })
	local u = new_survivor(p, #s.survivors + 1)
	hold_weapon(u)
	table.insert(s.survivors, u)
end

local function player_left(p)
	s = state.s
	for i, u in s.survivors do
		if u.id == p.id then
			if u.fig then figure.remove(u.fig) end
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
					if u.fig then world.set(u.fig.torso, { color = p.color }) end
				end
			end
		end
	end
end

-- Console helpers: debug.stop(4) jumps to stop 4, debug.peak() starts a
-- horde, debug.ammo(20) hands out rounds.
local debug = {
	stop = function(n)
		s = state.s
		s.stop = n
		s.band = stop_band(n)
		build_stop()
		return "stop " .. n .. ", " .. s.band
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
}

return { init = init, tick = tick, draw = draw, reload = reload, debug = debug, player_joined = player_joined, player_left = player_left, players_changed = players_changed }
