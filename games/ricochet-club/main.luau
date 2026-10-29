-- Ricochet Club: every wall is another angle. Tanks fire shots that bounce
-- off the arena walls (four bounces, then they fade) and chip away at
-- wooden cover. Hitting another tank scores 3 and costs them 1; your own
-- shot can hit you once it has bounced. B raises a shield for 0.6 s
-- (every 3 s). The highest score after 60 seconds wins the round; the
-- next round starts three seconds later.
--
-- Ported from the LÖVE party pack (love-party/games/ricochet.lua and
-- shared/arena.lua): the same arena units (1280 x 720 screen, the field
-- from 70,160 to 1210,670), speeds, timings, collisions, scoring and bot,
-- simulated in 1/120 s steps as there. sync() shows it in 3D, 40 units to
-- the metre, under one shared camera.

-- Settings (the LÖVE pack's defaults).
local BOUNCES = 4 -- wall bounces before a shot fades
local SHOT_SPEED = 100 -- percent
local COVER = true -- destructible cover

local ROUND = 60 -- seconds per round
local RESULTS = 3 -- seconds of results before the next round
local STEP = 1 / 120 -- the LÖVE simulation step
local MAX_TANKS = 4
local MAX_SHOTS = 80

local S = 1 / 40 -- metres per arena unit
local CX, CY = 640, 415 -- the field's centre
local FIELD = { x = 70, y = 160, w = 1140, h = 510 }
local SEGMENTS = 16 -- shield ring pieces

local CRATE = { 0.32, 0.24, 0.13 }
local CRATE_TOP = { 0.78, 0.58, 0.31 }
local CRATE_HEIGHT = { 0.5, 0.68, 0.86 } -- by hp: battered crates sag

local s -- the simulation: state.s

----------------------------------------------------------------------------
-- Helpers

local function len(x, y)
	return math.sqrt(x * x + y * y)
end

local function at(x, y, h)
	return vector.create((x - CX) * S, h, (y - CY) * S)
end

local function rgb(c, k)
	k = k or 1
	return vector.create(c[1] * k, c[2] * k, c[3] * k)
end

local function event(name)
	s.sfx[name] = (s.sfx[name] or 0) + 1
end

local function tank_of(id)
	for _, p in s.players do
		if p.id == id then return p end
	end
	return nil
end

----------------------------------------------------------------------------
-- The LÖVE simulation

local function overlaps(x, y, radius, block)
	return math.abs(x - block.x) < block.size / 2 + radius and math.abs(y - block.y) < block.size / 2 + radius
end

local function blocked(x, y)
	for _, block in s.cover do
		if overlaps(x, y, 18, block) then return true end
	end
	return false
end

-- shared/arena.lua's A.move: a stick moves at up to `speed`, clamped to the field.
local function arena_move(p, c, speed, dt)
	local x, y = c.x or 0, c.y or 0
	local n = math.max(1, len(x, y))
	p.x = math.clamp(p.x + x / n * speed * dt, 90, 1190)
	p.y = math.clamp(p.y + y / n * speed * dt, 180, 650)
end

local function move_tank(p, c, dt)
	local steps = math.max(1, math.ceil(dt * 220 / 6))
	for _ = 1, steps do
		local x, y = p.x, p.y
		arena_move(p, c, 220, dt / steps)
		local nx, ny = p.x, p.y
		p.x, p.y = x, y
		if not blocked(nx, y) then p.x = nx end
		if not blocked(p.x, ny) then p.y = ny end
	end
end

local function remove_block(index)
	local block = s.cover[index]
	for _, id in block.ents or {} do world.remove(id) end
	table.remove(s.cover, index)
end

local function hit_cover(index)
	local block = s.cover[index]
	block.hp -= 1
	block.flash = 0.1
	event(if block.hp == 0 then "explode" else "hit")
	-- The LÖVE debris: 12 chips when it breaks, 4 on a hit, half a second.
	fx.burst(at(block.x, block.y, CRATE_HEIGHT[math.max(1, block.hp)] * 0.6), {
		color = rgb(CRATE_TOP), count = if block.hp == 0 then 12 else 4, speed = 170 * S, life = 0.5, size = 0.1, flat = true,
	})
	if block.hp == 0 then remove_block(index) end
end

local function remove_shot(i)
	local b = s.shots[i]
	for _, id in b.ents or {} do world.remove(id) end
	table.remove(s.shots, i)
end

local function tank_hit(p, b)
	b.ttl = 0
	if p.shield == 0 then
		local owner = tank_of(b.owner)
		if p == owner then
			p.score -= 1
		else
			if owner then owner.score += 3 end
			p.score -= 1
			event("point")
		end
		p.invul = 1.2
		event("explode")
		fx.burst(at(p.x, p.y, 0.3), { color = p.color, count = 14, speed = 6, life = 0.5, size = 0.1, stretch = 0.02, flat = true })
		players.rumble(p.id, 0.8, 0.3)
	else
		event("hit")
		fx.ring(at(p.x, p.y, 0.25), { color = p.color, radius = 30 * S, life = 0.2, width = 0.05, segments = SEGMENTS })
	end
end

local function step_shots(dt)
	local speed = 430 * SHOT_SPEED / 100
	for i = #s.shots, 1, -1 do
		local b = s.shots[i]
		b.ttl -= dt
		local steps = math.max(1, math.ceil(dt * speed / 6))
		for _ = 1, steps do
			if b.ttl <= 0 then break end
			b.x += b.vx * dt / steps
			b.y += b.vy * dt / steps
			if b.x < 78 or b.x > 1202 then
				b.x = math.clamp(b.x, 78, 1202)
				b.vx = -b.vx
				b.bounces += 1
				event("bounce")
			end
			if b.y < 168 or b.y > 662 then
				b.y = math.clamp(b.y, 168, 662)
				b.vy = -b.vy
				b.bounces += 1
				event("bounce")
			end
			for index, block in s.cover do
				if overlaps(b.x, b.y, 4, block) then
					hit_cover(index)
					b.ttl = 0
					break
				end
			end
			if b.ttl <= 0 then break end
			for _, p in s.players do
				if (p.id ~= b.owner or b.bounces > 0) and p.invul == 0 and len(p.x - b.x, p.y - b.y) < 19 then
					tank_hit(p, b)
					break
				end
			end
			if b.bounces > BOUNCES then b.ttl = 0 end
		end
		if b.ttl <= 0 then remove_shot(i) end
	end
end

local function step(dt, inputs)
	s.time += dt
	for _, block in s.cover do
		block.flash = math.max(0, block.flash - dt)
	end
	for _, p in s.players do
		local c = inputs[p.id]
		move_tank(p, c, dt)
		for _, k in { "cool", "guard", "shield", "invul" } do
			p[k] = math.max(0, p[k] - dt)
		end
		local ax, ay = c.aimX or 0, c.aimY or 0
		if len(ax, ay) < 0.2 then ax, ay = c.x, c.y end
		local n = len(ax, ay)
		if n > 0.2 then p.ax, p.ay = ax / n, ay / n end
		if c.secondary and p.guard == 0 then
			p.shield = 0.6
			p.guard = 3
			event("shield")
			fx.ring(at(p.x, p.y, 0.25), { color = p.color, radius = 26 * S, life = 0.15, width = 0.05, segments = SEGMENTS })
		end
		if c.action and p.cool == 0 and #s.shots < MAX_SHOTS then
			p.cool = 0.45
			event("fire")
			local speed = 430 * SHOT_SPEED / 100
			table.insert(s.shots, {
				x = p.x + p.ax * 22,
				y = p.y + p.ay * 22,
				vx = p.ax * speed,
				vy = p.ay * speed,
				owner = p.id,
				ttl = 4,
				bounces = 0,
			})
		end
	end
	-- Small substeps keep collisions reliable, as in LÖVE.
	step_shots(dt)
end

-- The LÖVE bot: weave on a sine, aim at the next tank, fire and shield
-- whenever ready.
local function bot(p, slot)
	local target = s.players[slot % #s.players + 1]
	if target == p then target = s.players[1] end
	local dx, dy = target.x - p.x, target.y - p.y
	local n = math.max(1, len(dx, dy))
	return {
		x = math.sin(s.time + slot),
		y = math.cos(s.time * 0.7 + slot),
		aimX = dx / n,
		aimY = dy / n,
		action = true,
		secondary = p.guard == 0,
	}
end

-- A person's controls in arena terms (y grows down the screen). Fire is
-- RB or RT (Space, Shift, E or the left mouse button on a keyboard); B (F
-- or the right mouse button)
-- shields.
local function controls(i)
	if not i then return { x = 0, y = 0 } end
	local x, y = i.move_x, -i.move_y
	if i.held.left then x = -1 elseif i.held.right then x = 1 end
	if i.held.up then y = -1 elseif i.held.down then y = 1 end
	return {
		x = x,
		y = y,
		aimX = i.aim_x,
		aimY = -i.aim_y,
		action = i.held.right_shoulder or i.held.right_trigger or i.held.south,
		secondary = i.held.east or i.held.north,
	}
end

----------------------------------------------------------------------------
-- 3D

local function crate_look(block)
	local k = ({ 0.6, 0.8, 1 })[math.max(1, block.hp)]
	if block.flash > 0 then
		return rgb({ CRATE[1] + 0.4, CRATE[2] + 0.4, CRATE[3] + 0.4 }), rgb(CRATE_TOP, 1.15), 0.12
	end
	-- A faint glow: the night sky alone leaves wood nearly black.
	return rgb(CRATE, k * 1.35), rgb(CRATE_TOP, k), 0.03
end

local function show_block(block)
	local look = (if block.flash > 0 then "flash" else "") .. block.hp
	if block.look == look then return end
	block.look = look
	local body, top, glow = crate_look(block)
	local h = CRATE_HEIGHT[math.max(1, block.hp)]
	local size = block.size * S
	if not block.ents then
		block.ents = {
			world.spawn("cube", { pos = at(block.x, block.y, h / 2), size = vector.create(size, h, size), color = body, emissive = glow, roughness = 0.9, solid = false, tag = "cover" }),
			world.spawn("cube", { pos = at(block.x, block.y, h), size = vector.create(size * 0.8, 0.04, size * 0.8), color = top, emissive = glow, roughness = 0.9, solid = false, tag = "cover" }),
		}
		return
	end
	world.set(block.ents[1], { pos = at(block.x, block.y, h / 2), size = vector.create(size, h, size), color = body, emissive = glow })
	world.set(block.ents[2], { pos = at(block.x, block.y, h), color = top, emissive = glow })
end

local function show_tank(p)
	local yaw = math.atan2(-p.ax, -p.ay)
	local fwd = vector.create(p.ax, 0, p.ay)
	local pos = at(p.x, p.y, 0)
	if not p.ents then
		p.ents = {
			body = world.spawn("cube", { size = vector.create(24 * S, 0.26, 26 * S), color = p.color, emissive = 0.04, solid = false, tag = "tank" }),
			turret = world.spawn("cube", { size = vector.create(0.34, 0.16, 0.34), color = p.color * 0.6, emissive = 0.02, solid = false, tag = "tank" }),
			barrel = world.spawn("cube", { size = vector.create(0.14, 0.12, 24 * S), color = "#e8eef8", emissive = 0.04, solid = false, tag = "tank" }),
			ring = {},
		}
		for _ = 1, SEGMENTS do
			table.insert(p.ents.ring, world.spawn("cube", { size = vector.create(0.05, 0.1, 2 * math.pi * 26 * S / SEGMENTS + 0.02), color = p.color, emissive = 0.12, solid = false, visible = false, tag = "shield" }))
		end
		p.visible = true
		p.shielded = false
	end
	-- Hit tanks blink while they cannot be hit again (half-transparent in LÖVE).
	local visible = p.invul == 0 or math.floor(s.time * 12) % 2 == 0
	if visible ~= p.visible then
		p.visible = visible
		for _, id in { p.ents.body, p.ents.turret, p.ents.barrel } do world.set(id, { visible = visible }) end
	end
	world.place(p.ents.body, pos + vector.create(0, 0.18, 0), yaw)
	world.place(p.ents.turret, pos + vector.create(0, 0.38, 0), yaw)
	world.place(p.ents.barrel, pos + vector.create(0, 0.38, 0) + fwd * (12 * S + 0.12), yaw)
	local shielded = p.shield > 0
	if shielded ~= p.shielded then
		p.shielded = shielded
		for _, id in p.ents.ring do world.set(id, { visible = shielded }) end
	end
	if shielded then
		for i, id in p.ents.ring do
			local a = (i - 1) / SEGMENTS * math.pi * 2 + s.time * 3
			world.place(id, pos + vector.create(math.cos(a), 0, math.sin(a)) * (26 * S) + vector.create(0, 0.3, 0), math.atan2(math.sin(a), -math.cos(a)))
		end
	end
end

local function remove_tank(p)
	if not p.ents then return end
	for _, id in { p.ents.body, p.ents.turret, p.ents.barrel } do world.remove(id) end
	for _, id in p.ents.ring do world.remove(id) end
	p.ents = nil
end

local function show_shot(b)
	local owner = tank_of(b.owner)
	local color = if owner then owner.color else vector.create(1, 1, 1)
	local n = math.max(1, len(b.vx, b.vy))
	local yaw = math.atan2(-b.vx, -b.vy)
	local pos = at(b.x, b.y, 0.38)
	-- LÖVE's streak: a line back along 0.025 s of flight.
	local tail = 0.025 * n * S
	local back = vector.create(-b.vx / n, 0, -b.vy / n) * (tail / 2)
	if not b.ents then
		b.ents = {
			world.spawn("sphere", { pos = pos, size = 8 * S, color = color, emissive = 0.2, solid = false, tag = "shot" }),
			world.spawn("cube", { pos = pos + back, rot = vector.create(0, yaw, 0), size = vector.create(0.07, 0.07, tail), color = color, emissive = 0.2, solid = false, tag = "shot" }),
		}
	else
		world.place(b.ents[1], pos)
		world.place(b.ents[2], pos + back, yaw)
	end
end

local function sync()
	for _, block in s.cover do show_block(block) end
	for _, p in s.players do show_tank(p) end
	for _, b in s.shots do show_shot(b) end
end

local function arena()
	local w, h = FIELD.w * S, FIELD.h * S
	world.spawn("cube", { pos = vector.create(0, -0.6, 0), size = vector.create(w + 30, 1, h + 30), color = "#03050b", roughness = 0.95, tag = "arena" })
	world.spawn("cube", { pos = vector.create(0, -0.05, 0), size = vector.create(w, 0.1, h), color = rgb({ 0.055, 0.075, 0.11 }), roughness = 0.85, tag = "arena" })
	-- The walls the shots bounce off.
	for _, side in { -1, 1 } do
		world.spawn("cube", { pos = vector.create(side * (w / 2 + 0.1), 0.2, 0), size = vector.create(0.2, 0.4, h + 0.4), color = rgb({ 0.15, 0.2, 0.27 }, 1.6), emissive = 0.02, tag = "arena" })
		world.spawn("cube", { pos = vector.create(0, 0.2, side * (h / 2 + 0.1)), size = vector.create(w + 0.4, 0.4, 0.2), color = rgb({ 0.15, 0.2, 0.27 }, 1.6), emissive = 0.02, tag = "arena" })
	end
	-- A faint grid every 80 units.
	for x = FIELD.x + 80, FIELD.x + FIELD.w - 1, 80 do
		world.spawn("cube", { pos = at(x, CY, 0.005), size = vector.create(0.025, 0.01, h), color = "#142033", emissive = 0.01, solid = false, tag = "arena" })
	end
	for y = FIELD.y + 85, FIELD.y + FIELD.h - 1, 85 do
		world.spawn("cube", { pos = at(CX, y, 0.005), size = vector.create(w, 0.01, 0.025), color = "#142033", emissive = 0.01, solid = false, tag = "arena" })
	end
end

-- Every look a round uses, prepared now so nothing stalls mid-round.
local function warm(p)
	world.warm("cube", { color = p.color, emissive = 0.04 })
	world.warm("cube", { color = p.color * 0.6, emissive = 0.02 })
	world.warm("cube", { color = p.color, emissive = 0.12 })
	world.warm("cube", { color = p.color, emissive = 0.2 })
	world.warm("sphere", { color = p.color, emissive = 0.2 })
	world.warm("cube", { color = p.color, emissive = 0.06 }) -- bursts and rings
end

local function warm_all()
	for _, block in { { hp = 1, flash = 0 }, { hp = 2, flash = 0 }, { hp = 3, flash = 0 }, { hp = 3, flash = 1 } } do
		local body, top, glow = crate_look(block)
		world.warm("cube", { color = body, emissive = glow, roughness = 0.9 })
		world.warm("cube", { color = top, emissive = glow, roughness = 0.9 })
	end
	world.warm("cube", { color = rgb(CRATE_TOP), emissive = 0.06 })
	world.warm("cube", { color = "#e8eef8", emissive = 0.04 })
	world.warm("sphere", { color = vector.create(1, 1, 1), emissive = 0.2 })
	world.warm("cube", { color = vector.create(1, 1, 1), emissive = 0.2 })
	for _, p in players.list() do warm(p) end
end

----------------------------------------------------------------------------
-- Rounds

-- shared/arena.lua's A.roster: alternating top and bottom, left to right.
local function new_tank(p, i)
	return {
		id = p.id,
		name = p.name,
		color = p.color,
		bot = p.bot,
		x = 200 + (i - 1) * 280,
		y = if i % 2 == 0 then 580 else 240,
		score = 0,
		ax = 1,
		ay = 0,
		cool = 0,
		guard = 0,
		shield = 0,
		invul = 1,
	}
end

local function new_round()
	for i = #s.shots, 1, -1 do remove_shot(i) end
	for i = #s.cover, 1, -1 do remove_block(i) end
	for _, p in s.players do remove_tank(p) end
	s.players = {}
	for i, p in players.list() do
		if i > MAX_TANKS then break end
		table.insert(s.players, new_tank(p, i))
	end
	-- Symmetric islands leave wide routes and every spawn clear.
	if COVER then
		for _, x in { 360, 640, 920 } do
			for _, y in { 320, 490 } do
				for _, offset in { -26, 26 } do
					table.insert(s.cover, { x = x + offset, y = y, size = 44, hp = 3, flash = 0 })
				end
			end
		end
	end
	s.time = 0
	s.remaining = ROUND
	s.finished = false
	s.results = 0
	s.round += 1
	s.winners = nil
	sync()
end

local function finish_round()
	s.finished = true
	local best = -math.huge
	for _, p in s.players do best = math.max(best, math.floor(p.score)) end
	local names = {}
	for _, p in s.players do
		if math.floor(p.score) == best then table.insert(names, p.name) end
	end
	s.winners = names
	s.best = best
	event("round")
	game.log(table.concat(names, " + ") .. " won Ricochet Club with " .. best .. " points")
	game.finish()
end

local function init()
	state.s = { players = {}, shots = {}, cover = {}, sfx = {}, cool = {}, time = 0, round = 0, remaining = ROUND, finished = false, results = 0 }
	s = state.s
	warm_all()
	arena()
	new_round()
	camera.set(1, { pos = vector.create(0, 18.5, 9.8), target = vector.create(0, 0, 0.9), fov = 50 })
end

local SOUNDS = {
	fire = { db = -4, gap = 4 },
	bounce = { db = -10, gap = 3 },
	hit = { db = -4, gap = 3 },
	shield = { db = -4, gap = 3 },
	explode = { db = 0, gap = 3 },
	point = { db = -4, gap = 3 },
	round = { db = 0, gap = 3 },
}

local function play_sounds()
	-- One of each per tick, a few ticks apart (LÖVE's 0.07 s and 0.045 s).
	for name, spec in SOUNDS do
		s.cool[name] = math.max(0, (s.cool[name] or 0) - 1)
		if s.sfx[name] and s.cool[name] == 0 then
			s.cool[name] = spec.gap
			sound.play(name, { db = spec.db, pitch = if name == "explode" then 0.9 + random.float() * 0.2 else 1 })
		end
	end
	s.sfx = {}
end

local function tick(dt, input)
	s = state.s
	if s.finished then
		s.results += dt
		if s.results >= RESULTS then new_round() end
		play_sounds()
		return
	end
	-- LÖVE steps the game at 120 Hz: two steps per 60 Hz tick.
	local steps = math.max(1, math.floor(dt / STEP + 0.5))
	for _ = 1, steps do
		local inputs = {}
		for i, p in s.players do
			inputs[p.id] = if p.bot then bot(p, i) else controls(input[p.id])
		end
		step(STEP, inputs)
		s.remaining = math.max(0, s.remaining - STEP)
		if s.remaining == 0 then
			finish_round()
			break
		end
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
	local dim = { 0.55, 0.62, 0.7, 1 }
	-- The clock top centre (red in the last ten seconds), the tanks around it.
	hud.text(string.format("%02d", math.ceil(s.remaining)), w / 2 - 100 * k, 18 * k, { size = 30 * k, align = "center", width = 200 * k, color = if s.remaining < 10 then { 1, 0.43, 0.4, 1 } else dim })
	local positions = { 28 * k, w * 0.23, w * 0.59, w - 248 * k }
	for i, p in s.players do
		local x = positions[i]
		hud.box(x, 22 * k, 24 * k, 24 * k, p.color)
		hud.text(p.name, x + 36 * k, 20 * k, { size = 24 * k, color = p.color })
		hud.text(tostring(math.floor(p.score)), x + 150 * k, 20 * k, { size = 24 * k, align = "right", width = 55 * k, color = { 0.85, 0.88, 0.92, 1 } })
	end
	if s.finished and s.winners then
		hud.box(w / 2 - 380 * k, 255 * k, 760 * k, 290 * k, { 0.02, 0.03, 0.06, 0.92 })
		hud.text(if #s.winners == 1 then "ROUND WINNER" else "SHARED VICTORY", 0, 285 * k, { size = 22 * k, align = "center", width = w, color = { 0.3, 0.92, 0.83, 1 } })
		hud.text(table.concat(s.winners, " + "), 0, 325 * k, { size = 48 * k, align = "center", width = w })
		hud.text(s.best .. " POINTS", 0, 390 * k, { size = 64 * k, align = "center", width = w })
		hud.text("NEXT ROUND IN " .. math.max(1, math.ceil(RESULTS - s.results)), 0, 485 * k, { size = 22 * k, align = "center", width = w, color = dim })
	end
end

-- Someone joins: a tank from the next free spawn, if there is room.
local function player_joined(p)
	s = state.s
	warm(p)
	if #s.players < MAX_TANKS and not s.finished then
		table.insert(s.players, new_tank(p, #s.players + 1))
	end
end

-- Someone leaves: their tank goes; their shots fly on.
local function player_left(p)
	s = state.s
	for i, t in s.players do
		if t.id == p.id then
			remove_tank(t)
			table.remove(s.players, i)
			return
		end
	end
end

local function players_changed()
	s = state.s
	for _, p in players.list() do
		for _, t in s.players do
			if t.id == p.id then t.name, t.color = p.name, p.color end
		end
	end
end

return { init = init, tick = tick, draw = draw, player_joined = player_joined, player_left = player_left, players_changed = players_changed }
