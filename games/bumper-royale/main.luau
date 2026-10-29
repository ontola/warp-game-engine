-- Bumper Royale: own the ring, bump your friends out. Everyone slides
-- around a round platform that shrinks for the first minute; ride off the
-- edge and you lose a point, and whoever bumped you in the last three
-- seconds scores 3. A dash (A) gives a burst of speed every 1.1 s.
-- Rounds last ROUND seconds; the highest score wins the round.
--
-- Ported from the LÖVE party pack (love-party/games/bumper.lua): the same
-- arena units, speeds, friction, dash, bump impulse, shrinking ring,
-- respawn, scoring and bot, simulated at LÖVE's 120 Hz (two steps a tick).
-- `sync()` shows it in 3D: pucks on a floating platform under a night sky,
-- and a puck that leaves the ring tumbles off the edge.

local ROUND = 60 -- seconds per round, as the LÖVE runner
local RESULTS = 3 -- seconds of results before the next round
local MAX_PLAYERS = 4
local STEP = 1 / 120 -- LÖVE's fixed step

-- LÖVE's arena units: the ring is centred on (640, 410).
local CX, CY = 640, 410
local START_RADIUS, SHRINK, MIN_SHRINK = 270, 1.2, 70
local PUCK = 20 -- drawn radius
local CONTACT = 42 -- centre distance at which two pucks touch
local OUT = 22 -- how far past the ring a centre may go
local ACCEL, FRICTION = 800, 2.3
local DASH, DASH_COOL = 480, 1.1
local CREDIT = 3 -- seconds a bump counts towards a knock-out
local FLASH = 0.8 -- seconds a respawned puck is a ghost

local S = 1 / 40 -- metres per arena unit
local PUCK_H = 0.42 -- puck height in metres
local RIM_SEGMENTS = 64
local GRAVITY = 22 -- m/s^2 for pucks falling off the edge

local function len(x, y)
	return math.sqrt(x * x + y * y)
end

local function clamp(v, a, b)
	return math.max(a, math.min(b, v))
end

local function at(x, y, h)
	return vector.create((x - CX) * S, h, (y - CY) * S)
end

local s -- the round: state.s

----------------------------------------------------------------------------
-- The simulation (LÖVE's M.new, M.update and M.bot)

local function new_round()
	for _, tag in { "puck", "faller" } do
		for _, id in world.find(tag) do
			world.remove(id)
		end
	end
	local list = {}
	for _, p in players.list() do
		if #list < MAX_PLAYERS then table.insert(list, p) end
	end
	s = { players = {}, radius = START_RADIUS, time = 0, remaining = ROUND, results = nil, fallers = {}, sfx = {} }
	state.s = s
	state.round += 1
	for i, p in list do
		local a = (i - 1) * math.pi * 2 / #list
		table.insert(s.players, {
			id = p.id,
			bot = p.bot,
			color = p.color,
			x = CX + math.cos(a) * 150,
			y = CY + math.sin(a) * 150,
			vx = 0,
			vy = 0,
			cool = 0,
			flash = 0,
			score = 0,
			hit = nil, -- the id of the last puck that bumped this one
			hitTime = 0,
			yaw = 0,
		})
	end
	for _, q in s.players do
		q.body = world.spawn("sphere", { pos = at(q.x, q.y, PUCK_H / 2), size = vector.create(PUCK * 2 * S, PUCK_H, PUCK * 2 * S), color = q.color, roughness = 0.35, metal = 0.1, solid = false, tag = "puck" })
		q.cap = world.spawn("sphere", { pos = at(q.x, q.y, PUCK_H * 0.82), size = vector.create(PUCK * 1.1 * S, PUCK_H * 0.35, PUCK * 1.1 * S), color = q.color * 1.25, emissive = 0.04, roughness = 0.3, solid = false, tag = "puck" })
		q.eyes = {
			world.spawn("sphere", { size = 0.13, color = "#0a0f1a", roughness = 0.2, solid = false, tag = "puck" }),
			world.spawn("sphere", { size = 0.13, color = "#0a0f1a", roughness = 0.2, solid = false, tag = "puck" }),
		}
		q.halo = world.spawn("sphere", { size = vector.create(PUCK * 2.5 * S, 0.03, PUCK * 2.5 * S), color = q.color, emissive = 0.3, solid = false, tag = "puck" })
	end
	sound.play("round", { pitch = 0.75 })
end

local function by_id(id)
	for _, q in s.players do
		if q.id == id then return q end
	end
	return nil
end

-- A puck has left the ring: LÖVE's scoring and respawn, and a copy of it
-- that tumbles off the edge.
local function fall(q, i)
	local hitter = q.hit and by_id(q.hit)
	if hitter and s.time - q.hitTime < CREDIT then
		hitter.score += 3
		table.insert(s.sfx, { name = "score" })
	end
	q.score -= 1
	local faller = world.spawn("sphere", { pos = at(q.x, q.y, PUCK_H / 2), size = vector.create(PUCK * 2 * S, PUCK_H, PUCK * 2 * S), color = q.color, roughness = 0.35, metal = 0.1, solid = false, tag = "faller" })
	table.insert(s.fallers, { id = faller, pos = at(q.x, q.y, PUCK_H / 2), vel = vector.create(q.vx * S, 1.5, q.vy * S), age = 0, spin = 0 })
	table.insert(s.sfx, { name = "fall", at = at(q.x, q.y, 0) })
	players.rumble(q.id, 0.7, 0.35)
	q.x, q.y = CX + (i - 2.5) * 25, CY
	q.vx, q.vy = 0, 0
	q.flash = FLASH
	q.hit = nil
end

local function update(dt, controls)
	s.time += dt
	s.radius = START_RADIUS - math.min(s.time * SHRINK, MIN_SHRINK)
	for i, q in s.players do
		local c = controls[i]
		q.cool = math.max(0, q.cool - dt)
		q.flash = math.max(0, q.flash - dt)
		local damp = math.exp(-FRICTION * dt)
		q.vx = (q.vx + c.x * ACCEL * dt) * damp
		q.vy = (q.vy + c.y * ACCEL * dt) * damp
		if c.action and q.cool == 0 and len(c.x, c.y) > 0.1 then
			q.vx, q.vy = q.vx + c.x * DASH, q.vy + c.y * DASH
			q.cool = DASH_COOL
			table.insert(s.sfx, { name = "dash", at = at(q.x, q.y, 0), color = q.color })
		end
		q.x, q.y = q.x + q.vx * dt, q.y + q.vy * dt
		if len(q.x - CX, q.y - CY) > s.radius + OUT then
			fall(q, i)
		end
	end
	for i, a in s.players do
		for j = i + 1, #s.players do
			local b = s.players[j]
			local dx, dy = b.x - a.x, b.y - a.y
			local n = len(dx, dy)
			if n < CONTACT and a.flash == 0 and b.flash == 0 then
				if n < 0.001 then
					dx, dy, n = 1, 0, 1
				end
				local nx, ny = dx / n, dy / n
				local overlap = (CONTACT - n) / 2
				a.x, a.y = a.x - nx * overlap, a.y - ny * overlap
				b.x, b.y = b.x + nx * overlap, b.y + ny * overlap
				local relative = (a.vx - b.vx) * nx + (a.vy - b.vy) * ny
				if relative > 0 then
					local impulse = relative * 0.95 + 65
					a.vx, a.vy = a.vx - nx * impulse, a.vy - ny * impulse
					b.vx, b.vy = b.vx + nx * impulse, b.vy + ny * impulse
					a.hit, a.hitTime = b.id, s.time
					b.hit, b.hitTime = a.id, s.time
					table.insert(s.sfx, { name = "bump", at = at((a.x + b.x) / 2, (a.y + b.y) / 2, PUCK_H / 2), impulse = impulse, a = a.id, b = b.id })
				end
			end
		end
	end
end

-- LÖVE's bot: head for the centre; once safely inside, chase the next
-- player, weaving, and dash when close.
local function bot(i, q)
	local target = s.players[i % #s.players + 1]
	if target == q then
		target = s.players[1]
	end
	local x, y = CX - q.x, CY - q.y
	if len(x, y) < s.radius * 0.65 and target then
		x, y = target.x - q.x, target.y - q.y
	end
	local n = math.max(1, len(x, y))
	local turn = math.sin(s.time * 1.3 + i * 1.8) * 0.6
	return { x = clamp(x / n - y / n * turn, -1, 1), y = clamp(y / n + x / n * turn, -1, 1), action = n < 160 }
end

-- A person's controls in arena terms (y grows down the screen, towards the
-- camera); the keyboard's diagonals are normalised as in LÖVE.
local function controls(i)
	if not i then return { x = 0, y = 0, action = false } end
	local x, y = i.move_x, -i.move_y
	if i.held.left then x = -1 elseif i.held.right then x = 1 end
	if i.held.up then y = -1 elseif i.held.down then y = 1 end
	if math.abs(x) < 0.2 then x = 0 end
	if math.abs(y) < 0.2 then y = 0 end
	local n = len(x, y)
	if n > 1 then x, y = x / n, y / n end
	return { x = x, y = y, action = i.held.south or i.held.right_shoulder }
end

----------------------------------------------------------------------------
-- The 3D view

local function place_rim()
	local r = (s.radius + 4) * S
	local seg = 2 * math.pi * r / RIM_SEGMENTS * 1.04
	for k, id in state.rim do
		local a = (k - 0.5) / RIM_SEGMENTS * math.pi * 2
		world.set(id, { pos = vector.create(math.cos(a) * r, 0.02, math.sin(a) * r), rot = vector.create(0, -a + math.pi / 2, 0), size = vector.create(seg, 0.06, 0.14) })
	end
	local d = (s.radius + 6) * 2 * S
	world.set(state.top, { size = vector.create(d, 0.08, d) })
	world.set(state.base, { size = vector.create(d, 1.6, d) })
	state.drawnRadius = s.radius
end

local function sync()
	if state.drawnRadius ~= s.radius then place_rim() end
	for _, q in s.players do
		local speed = len(q.vx, q.vy)
		if speed > 25 then
			-- Face where it slides, turning smoothly.
			local want = math.atan2(-q.vx, -q.vy)
			local d = (want - q.yaw + math.pi) % (math.pi * 2) - math.pi
			q.yaw = (q.yaw + d * 0.25) % (math.pi * 2)
		end
		local shown = q.flash == 0 or math.floor(q.flash * 12) % 2 == 0
		local base = at(q.x, q.y, 0)
		world.place(q.body, base + vector.create(0, PUCK_H / 2, 0), q.yaw)
		world.place(q.cap, base + vector.create(0, PUCK_H * 0.82, 0), q.yaw)
		world.set(q.body, { visible = shown })
		world.set(q.cap, { visible = shown })
		local fx, fz = -math.sin(q.yaw), -math.cos(q.yaw)
		local rx, rz = math.cos(q.yaw), -math.sin(q.yaw)
		for k, e in q.eyes do
			local side = if k == 1 then -1 else 1
			world.place(e, base + vector.create(fx * 0.36 + rx * side * 0.16, PUCK_H * 0.78, fz * 0.36 + rz * side * 0.16))
			world.set(e, { visible = shown })
		end
		world.place(q.halo, base + vector.create(0, 0.03, 0))
		world.set(q.halo, { visible = shown and q.cool == 0 })
	end
end

local function fallers(dt)
	for k = #s.fallers, 1, -1 do
		local f = s.fallers[k]
		f.age += dt
		f.vel = vector.create(f.vel.x * 0.995, f.vel.y - GRAVITY * dt, f.vel.z * 0.995)
		f.pos += f.vel * dt
		f.spin += dt * 6
		world.set(f.id, { pos = f.pos, rot = vector.create(f.spin, 0, f.spin * 0.6) })
		if f.age > 1.6 then
			world.remove(f.id)
			table.remove(s.fallers, k)
		end
	end
end

-- At most one of each sound a tick; bumps louder the harder they hit.
local function play_sounds()
	local done = {}
	for _, e in s.sfx do
		if e.name == "bump" then
			fx.burst(e.at, { color = "#fff2c0", emissive = 0.4, count = 10, speed = 3 + e.impulse * 0.01, life = 0.25, size = 0.08, flat = true, stretch = 0.03 })
			players.rumble(e.a, 0.4, 0.12)
			players.rumble(e.b, 0.4, 0.12)
		elseif e.name == "dash" then
			fx.burst(e.at + vector.create(0, 0.1, 0), { color = e.color, emissive = 0.3, count = 8, speed = 2.5, life = 0.3, size = 0.1, flat = true })
		elseif e.name == "fall" then
			fx.ring(e.at, { color = "#ff5a5a", emissive = 0.4, radius = 1.2, life = 0.4 })
		end
		if not done[e.name] then
			done[e.name] = true
			local opts: { [string]: any } = {}
			if e.at then opts.at = e.at end
			if e.name == "bump" then
				opts.db = math.min(0, -10 + e.impulse * 0.02)
				opts.pitch = 0.85 + random.float() * 0.3
			elseif e.name == "dash" then
				opts.pitch = 0.9 + random.float() * 0.2
			end
			sound.play(e.name, opts)
		end
	end
	s.sfx = {}
end

----------------------------------------------------------------------------
-- Rounds

local function winners()
	local best, names = -math.huge, {}
	for _, q in s.players do
		if q.score > best then best = q.score end
	end
	for _, q in s.players do
		local p = players.get(q.id)
		if q.score == best and p then table.insert(names, p.name) end
	end
	return names, best
end

local function end_round()
	s.results = RESULTS
	sound.play("round")
	local names, best = winners()
	state.winners = names
	if #names > 0 then
		game.log(table.concat(names, " and ") .. " won Bumper Royale with " .. best)
	end
	game.finish()
end

local function warm(p)
	world.warm("sphere", { color = p.color, roughness = 0.35, metal = 0.1 })
	world.warm("sphere", { color = p.color * 1.25, emissive = 0.04, roughness = 0.3 })
	world.warm("sphere", { color = p.color, emissive = 0.3 })
	world.warm("cube", { color = p.color, emissive = 0.3 }) -- dash streaks
end

local function init()
	-- A floating platform: a thick dark base under a lighter top, a glowing
	-- rim on the edge, and a faint centre mark.
	state.base = world.spawn("sphere", { pos = vector.create(0, -0.82, 0), size = vector.create(14, 1.6, 14), color = "#101826", roughness = 0.6, metal = 0.3, solid = false, tag = "arena" })
	state.top = world.spawn("sphere", { pos = vector.create(0, -0.04, 0), size = vector.create(14, 0.08, 14), color = "#1a2a3c", roughness = 0.75, solid = false, tag = "arena" })
	world.spawn("sphere", { pos = vector.create(0, 0.005, 0), size = vector.create(4.4, 0.01, 4.4), color = "#22364c", roughness = 0.8, solid = false, tag = "arena" })
	world.spawn("sphere", { pos = vector.create(0, 0.008, 0), size = vector.create(4, 0.01, 4), color = "#1a2a3c", roughness = 0.75, solid = false, tag = "arena" })
	state.rim = {}
	for k = 1, RIM_SEGMENTS do
		state.rim[k] = world.spawn("cube", { color = "#4debd4", emissive = 0.12, solid = false, tag = "arena" })
	end
	world.warm("sphere", { color = "#0a0f1a", roughness = 0.2 })
	world.warm("cube", { color = "#fff2c0", emissive = 0.4 })
	world.warm("cube", { color = "#ff5a5a", emissive = 0.4 })
	for _, p in players.list() do
		warm(p)
	end
	state.round = 0
	state.drawnRadius = nil
	new_round()
	camera.set(1, { pos = vector.create(0, 12.5, 13.5), target = vector.create(0, -0.6, 0.9), fov = 46 })
	-- A moonlit night: enough light on the platform to read its edge.
	light.set({ sun = vector.create(-0.4, 1, 0.6), sun_lux = 600, sky_lux = 200 })
end

local function tick(dt, input)
	s = state.s
	fallers(dt)
	if s.results then
		s.results -= dt
		if s.results <= 0 then new_round() end
		sync()
		return
	end
	local steps = math.max(1, math.floor(dt / STEP + 0.5))
	for _ = 1, steps do
		local c = {}
		for i, q in s.players do
			c[i] = if q.bot then bot(i, q) else controls(input[q.id])
		end
		update(dt / steps, c)
		s.remaining = math.max(0, s.remaining - dt / steps)
		if s.remaining == 0 then break end
	end
	sync()
	play_sounds()
	if s.remaining == 0 then end_round() end
end

local function draw()
	s = state.s
	local w, h = hud.size()
	local k = h / 720
	local slot = w / math.max(#s.players, 1)
	for i, q in s.players do
		local p = players.get(q.id)
		local x = (i - 1) * slot
		hud.text(if p then p.name else "", x, 12 * k, { size = 22 * k, align = "center", width = slot, color = q.color })
		hud.text(tostring(q.score), x, 36 * k, { size = 42 * k, align = "center", width = slot, color = q.color })
	end
	hud.text(tostring(math.ceil(s.remaining)), 0, h - 56 * k, { size = 36 * k, align = "center", width = w, color = { 0.6, 0.7, 0.8, 1 } })
	if s.results then
		local names = state.winners or {}
		hud.box(0, h * 0.38, w, h * 0.18, { 0, 0, 0, 0.55 })
		local title = if #names == 1 then names[1] .. " wins!" elseif #names > 1 then table.concat(names, " & ") .. " share the win" else "Round over"
		hud.text(title, 0, h * 0.41, { size = 60 * k, align = "center", width = w })
	end
end

-- Someone joins: they play from the next round (their colours warmed now).
local function player_joined(p)
	warm(p)
end

-- Someone leaves: their puck leaves the ring now.
local function player_left(p)
	s = state.s
	for k, q in s.players do
		if q.id == p.id then
			for _, id in { q.body, q.cap, q.halo, q.eyes[1], q.eyes[2] } do
				world.remove(id)
			end
			table.remove(s.players, k)
			break
		end
	end
	for _, q in s.players do
		if q.hit == p.id then q.hit = nil end
	end
end

return { init = init, tick = tick, draw = draw, player_joined = player_joined, player_left = player_left }
