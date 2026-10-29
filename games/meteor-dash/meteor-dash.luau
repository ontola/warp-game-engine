-- Meteor Dash: chase the stars, dodge everything else. Fly with the stick
-- (or WASD); A (or RB / Space) is a shield dash: 0.3 s of shield at more
-- than double speed, every 1.5 s. A star is +5, a meteor -10 (and 0.75 s
-- of shield after the hit); staying alive earns a point a second. Most
-- points after 60 seconds wins.
--
-- Ported from the LÖVE party pack (gamenight-games love-party/games/
-- meteor.lua): the same field, spawn rate, speeds, radii, shield timings,
-- scoring and bot, simulated in the LÖVE screen units (1280 x 800) and
-- shown in 3D, 40 units to the metre, under one shared angled camera.
--! title = "Meteor Dash"
--! id = "meteor-dash-warp"
--! tick_rate = 60
--! split = false
--! players = 1
--! bots = 1
--! min_players = 2
--! seed = 11
--! sky = "night:clear"
--! max_entities = 1500

local ROUND = 60 -- seconds
local SPAWN = 0.12 -- seconds between objects
local STAR_CHANCE = 0.2
local LEFT, RIGHT = 85, 1195 -- the field in LÖVE units
local TOP, BOTTOM = 140, 700 -- objects appear at TOP and leave past BOTTOM
local MIN_Y, MAX_Y = 180, 660 -- where ships can fly
local SPEED, DASH_SPEED = 280, 620
local COOLDOWN, DASH, HIT_SHIELD = 1.5, 0.3, 0.75
local SHIP_R = 15
local S = 1 / 40 -- metres per unit
local CX, CY = 640, 420

local ROCK = { 0.43, 0.49, 0.63 }
local GOLD = { 1, 0.82, 0.32 }
local TAIL = { 1, 0.45, 0.25 }

local function rgb(c, k)
	k = k or 1
	return vector.create(c[1] * k, c[2] * k, c[3] * k)
end

local function at(x, y, h)
	return vector.create((x - CX) * S, h, (y - CY) * S)
end

local function clamp(v, a, b)
	return math.max(a, math.min(b, v))
end

----------------------------------------------------------------------------
-- Ships

local function warm(p)
	world.warm("cube", { color = p.color, emissive = 0.08 })
	world.warm("sphere", { color = p.color * 0.7, emissive = 0.05 })
	world.warm("cube", { color = p.color, emissive = 0.06 }) -- bursts
end

local function new_ship(p, i)
	local ship = {
		id = p.id,
		name = p.name,
		color = p.color,
		bot = p.bot,
		slot = i,
		x = 220 + i * 165,
		y = 590,
		cool = 0,
		shield = 0,
		score = 0,
		bank = 0,
	}
	ship.body = world.spawn("cube", { pos = at(ship.x, ship.y, 0.35), size = vector.create(0.55, 0.16, 0.7), color = p.color, emissive = 0.08, solid = false, tag = "ship" })
	ship.wing = world.spawn("cube", { pos = at(ship.x, ship.y, 0.33), size = vector.create(1.0, 0.06, 0.28), color = p.color, emissive = 0.08, solid = false, tag = "ship" })
	ship.nose = world.spawn("cube", { pos = at(ship.x, ship.y, 0.4), size = vector.create(0.18, 0.14, 0.36), color = "#ffffff", emissive = 0.1, solid = false, tag = "ship" })
	ship.bubble = world.spawn("sphere", { pos = at(ship.x, ship.y, 0.35), size = vector.create(54 * S, 0.03, 54 * S), color = p.color * 0.7, emissive = 0.05, solid = false, visible = false, tag = "ship" })
	return ship
end

local function remove_ship(ship)
	for _, id in { ship.body, ship.wing, ship.nose, ship.bubble } do
		world.remove(id)
	end
end

local function show_ship(ship)
	local pos = at(ship.x, ship.y, 0.35)
	local rot = vector.create(0, 0, ship.bank)
	world.set(ship.body, { pos = pos, rot = rot })
	world.set(ship.wing, { pos = pos + vector.create(0, -0.02, 0.12), rot = rot })
	world.set(ship.nose, { pos = pos + vector.create(0, 0.05, -0.42), rot = rot })
	local on = ship.shield > 0
	if on ~= ship.shown then
		world.set(ship.bubble, { visible = on })
		ship.shown = on
	end
	if on then world.place(ship.bubble, at(ship.x, ship.y, 0.03)) end
end

----------------------------------------------------------------------------
-- Stars and meteors

local function spawn_object(s)
	local o = {
		x = random.int(LEFT, RIGHT),
		y = TOP,
		r = random.int(10, 24),
		speed = random.int(160, 290) + s.time * 2,
		star = random.float() < STAR_CHANCE,
		spin = random.range(0, math.pi * 2),
	}
	if o.star then
		o.ent = world.spawn("cube", { pos = at(o.x, o.y, 0.45), size = o.r * 1.3 * S, color = rgb(GOLD), emissive = 0.12, solid = false, tag = "star" })
	else
		o.ent = world.spawn("sphere", { pos = at(o.x, o.y, 0.45), size = o.r * 2 * S, color = rgb(ROCK), emissive = 0.012, roughness = 0.9, solid = false, tag = "meteor" })
		-- A glowing tail behind it, pointing back where it came from.
		o.tail = world.spawn("cube", { pos = at(o.x, o.y - o.r * 2.3, 0.45), size = vector.create(o.r * 0.45 * S, o.r * 0.3 * S, o.r * 3.2 * S), color = rgb(TAIL), emissive = 0.04, solid = false, tag = "meteor" })
	end
	table.insert(s.objects, o)
end

local function remove_object(o)
	world.remove(o.ent)
	if o.tail then world.remove(o.tail) end
end

local function show_object(s, o)
	if o.star then
		world.set(o.ent, { pos = at(o.x, o.y, 0.45 + math.sin(s.time * 4 + o.spin) * 0.08), rot = vector.create(0.62, s.time * 2.5 + o.spin, 0.62) })
	else
		world.place(o.ent, at(o.x, o.y, 0.45))
		world.place(o.tail, at(o.x, o.y - o.r * 2.3, 0.45))
	end
end

----------------------------------------------------------------------------
-- The game

local s

local function arena()
	local w, h = (RIGHT - LEFT + 60) * S, (BOTTOM - TOP + 20) * S
	local mid = at((LEFT + RIGHT) / 2, (TOP + BOTTOM) / 2, 0)
	world.spawn("cube", { pos = vector.create(0, -0.6, 0), size = vector.create(w + 40, 1, h + 40), color = "#03050b", roughness = 0.95, tag = "arena" })
	world.spawn("cube", { pos = mid + vector.create(0, -0.05, 0), size = vector.create(w, 0.1, h), color = "#0b1220", emissive = 0.004, roughness = 0.85, tag = "arena" })
	for _, side in { -1, 1 } do
		world.spawn("cube", { pos = mid + vector.create(side * (w / 2 + 0.06), 0.1, 0), size = vector.create(0.12, 0.2, h + 0.24), color = "#3a6aa8", emissive = 0.03, tag = "arena" })
		world.spawn("cube", { pos = mid + vector.create(0, 0.1, side * (h / 2 + 0.06)), size = vector.create(w + 0.24, 0.2, 0.12), color = "#3a6aa8", emissive = 0.03, tag = "arena" })
	end
	-- The flight zone's top edge: ships stay below it.
	world.spawn("cube", { pos = at((LEFT + RIGHT) / 2, MIN_Y - SHIP_R, 0.01), size = vector.create(w, 0.01, 0.04), color = "#24406a", emissive = 0.02, solid = false, tag = "arena" })
	-- Lanes of faint dust streaking towards the camera.
	for _ = 1, 120 do
		local len = random.range(0.2, 0.8)
		world.spawn("cube", { pos = mid + vector.create(random.range(-w / 2, w / 2), 0.01, random.range(-h / 2, h / 2)), size = vector.create(0.03, 0.01, len), color = "#9fb8e8", emissive = 0.02, solid = false, tag = "arena" })
	end
end

local function warm_all()
	world.warm("cube", { color = rgb(GOLD), emissive = 0.12 })
	world.warm("sphere", { color = rgb(ROCK), emissive = 0.012, roughness = 0.9 })
	world.warm("cube", { color = rgb(TAIL), emissive = 0.04 })
	world.warm("cube", { color = rgb(GOLD), emissive = 0.06 })
	world.warm("cube", { color = rgb(ROCK), emissive = 0.06 })
	world.warm("cube", { color = "#ffffff", emissive = 0.1 })
	for _, p in players.list() do warm(p) end
end

local function init()
	state.s = { objects = {}, ships = {}, clock = 0, time = 0, remaining = ROUND, over = false, winners = nil, best = 0, sfx = {} }
	s = state.s
	warm_all()
	arena()
	for i, p in players.list() do
		table.insert(s.ships, new_ship(p, i))
	end
	camera.set(1, { pos = vector.create(0, 18.5, 11.5), target = vector.create(0, 0, 0.4), fov = 50 })
end

-- LÖVE's controls: the stick (normalised), A or RB for the dash.
local function controls(p)
	if not p then return { x = 0, y = 0, action = false } end
	local x, y = p.move_x, -p.move_y
	if p.held.left then x = -1 elseif p.held.right then x = 1 end
	if p.held.up then y = -1 elseif p.held.down then y = 1 end
	local n = math.max(1, math.sqrt(x * x + y * y))
	return { x = x / n, y = y / n, action = p.held.south or p.held.right_shoulder or p.held.fire }
end

-- LÖVE's bot: drift side to side, step towards a star above, away from a meteor.
local function bot(ship)
	local x = math.sin(s.time + ship.slot) * 0.5
	for _, o in s.objects do
		if math.abs(o.x - ship.x) < 75 and o.y < ship.y and ship.y - o.y < 170 then
			x = if o.star then (if o.x > ship.x then 1 else -1) else (if o.x > ship.x then -1 else 1)
		end
	end
	return { x = x, y = 0, action = false }
end

local function sfx(name)
	s.sfx[name] = true
end

local function fly(ship, c, dt)
	ship.cool = math.max(0, ship.cool - dt)
	ship.shield = math.max(0, ship.shield - dt)
	if c.action and ship.cool == 0 then
		ship.cool = COOLDOWN
		ship.shield = DASH
		sfx("dash")
	end
	local speed = if ship.shield > 0 then DASH_SPEED else SPEED
	ship.x = clamp(ship.x + c.x * speed * dt, LEFT, RIGHT)
	ship.y = clamp(ship.y + c.y * speed * dt, MIN_Y, MAX_Y)
	ship.score += dt
	ship.bank += (-c.x * 0.5 - ship.bank) * math.min(1, dt * 10)
end

local function finish()
	s.over = true
	local best = -math.huge
	for _, ship in s.ships do best = math.max(best, math.floor(ship.score)) end
	local winners = {}
	for _, ship in s.ships do
		if math.floor(ship.score) == best then table.insert(winners, ship.name) end
	end
	s.best, s.winners = best, winners
	sfx("finish")
	game.log(table.concat(winners, " + ") .. " won Meteor Dash with " .. best .. " points")
	game.finish()
end

local function tick(dt, input)
	s = state.s
	if not s.over then
		s.time += dt
		s.clock += dt
		if s.clock > SPAWN then
			s.clock -= SPAWN
			spawn_object(s)
		end
		for _, ship in s.ships do
			fly(ship, if ship.bot then bot(ship) else controls(input[ship.id]), dt)
		end
		for i = #s.objects, 1, -1 do
			local o = s.objects[i]
			o.y += o.speed * dt
			local gone = o.y > BOTTOM
			for _, ship in s.ships do
				if not gone and math.sqrt((o.x - ship.x) ^ 2 + (o.y - ship.y) ^ 2) < o.r + SHIP_R then
					if o.star then
						ship.score += 5
						gone = true
						sfx("point")
						fx.burst(at(o.x, o.y, 0.45), { color = rgb(GOLD), emissive = 0.06, count = 14, speed = 4, life = 0.4 })
						fx.ring(at(ship.x, ship.y, 0.05), { color = rgb(GOLD), emissive = 0.06, radius = 1, life = 0.3 })
					elseif ship.shield == 0 then
						ship.score -= 10
						ship.shield = HIT_SHIELD
						gone = true
						sfx("hit")
						players.rumble(ship.id, 0.7, 0.25)
						s.shake = 1
						fx.burst(at(o.x, o.y, 0.45), { color = rgb(ROCK), emissive = 0.06, count = 16, speed = 5, life = 0.5 })
						fx.burst(at(ship.x, ship.y, 0.4), { color = ship.color, emissive = 0.06, count = 10, speed = 3, life = 0.35 })
					end
				end
			end
			if gone then
				remove_object(o)
				table.remove(s.objects, i)
			end
		end
		s.remaining = math.max(0, ROUND - s.time)
		if s.remaining == 0 then finish() end
	end
	for _, o in s.objects do show_object(s, o) end
	for _, ship in s.ships do show_ship(ship) end
	-- Shake the camera a little on a hit.
	s.shake = math.max(0, (s.shake or 0) - dt * 4)
	local k = s.shake * 0.12
	camera.set(1, { pos = vector.create(math.sin(s.time * 83) * k, 18.5, 11.5 + math.cos(s.time * 97) * k), target = vector.create(math.sin(s.time * 83) * k, 0, 0.4), fov = 50 })
	for name in s.sfx do
		sound.play(name, { pitch = if name == "point" then 0.95 + random.float() * 0.1 else 1 })
	end
	s.sfx = {}
end

----------------------------------------------------------------------------
-- HUD

local function draw()
	s = state.s
	if not s then return end
	local w, h = hud.size()
	local k = h / 720
	local dim = { 0.55, 0.62, 0.7, 1 }
	-- The timer in the middle, every ship's score along the top.
	hud.text(string.format("%02d", math.ceil(s.remaining)), 0, 14 * k, { size = 30 * k, align = "center", width = w, color = if s.remaining < 10 then { 1, 0.43, 0.4, 1 } else dim })
	-- LÖVE's four score slots across the top.
	local slots = { 28 / 1280, 0.23, 0.59, 1 - 248 / 1280 }
	for i, ship in s.ships do
		local x = w * (slots[i] or ((i - 1) % 4 * 0.25 + 0.12))
		hud.text(ship.name, x, 12 * k, { size = 18 * k, color = ship.color })
		hud.text(tostring(math.floor(ship.score)), x, 32 * k, { size = 34 * k, color = ship.color })
	end
	-- A dash meter under each ship.
	for _, ship in s.ships do
		local x, y = hud.project(at(ship.x, ship.y, 0.35))
		if x then
			local ready = 1 - ship.cool / COOLDOWN
			hud.box(x - 18 * k, y + 20 * k, 36 * k, 4 * k, { 0.1, 0.12, 0.2, 0.8 })
			hud.box(x - 18 * k, y + 20 * k, 36 * k * ready, 4 * k, if ship.cool == 0 then ship.color else { 0.5, 0.55, 0.65, 0.8 })
		end
	end
	if s.over and s.winners then
		hud.box(w * 0.2, h * 0.32, w * 0.6, h * 0.34, { 0.02, 0.03, 0.06, 0.88 })
		hud.text(if #s.winners == 1 then "ROUND WINNER" else "SHARED VICTORY", 0, h * 0.35, { size = 22 * k, align = "center", width = w, color = { 0.3, 0.92, 0.83, 1 } })
		hud.text(table.concat(s.winners, " + "), 0, h * 0.35 + 34 * k, { size = 46 * k, align = "center", width = w })
		hud.text(s.best .. " POINTS", 0, h * 0.35 + 96 * k, { size = 64 * k, align = "center", width = w })
	end
end

-- Someone joins: a fresh ship (score 0) for the rest of the round.
local function player_joined(p)
	s = state.s
	warm(p)
	table.insert(s.ships, new_ship(p, #s.ships + 1))
end

local function player_left(p)
	s = state.s
	for i, ship in s.ships do
		if ship.id == p.id then
			remove_ship(ship)
			table.remove(s.ships, i)
			return
		end
	end
end

local function players_changed()
	s = state.s
	for _, p in players.list() do
		for _, ship in s.ships do
			if ship.id == p.id then ship.name, ship.color = p.name, p.color end
		end
	end
end

----------------------------------------------------------------------------
-- Sounds (engine/docs/audio.md): sound.play("point") plays SOUNDS.point.

local SOUNDS = {
	-- A star: a bright two-note chime, 740 Hz up to 1110 Hz. 0.3 s.
	point = [==[
length(0.3)
local a = expr("sin(tau * 740 * t) * exp(-t * 18) * 0.35")
local b = expr("sin(tau * 1110 * (t - 0.05)) * exp(-(t - 0.05) * 12) * 0.35 * step(0.05, t)")
local c = expr("sin(tau * 2220 * (t - 0.05)) * exp(-(t - 0.05) * 30) * 0.1 * step(0.05, t)")
return expr("tanh((a + b + c) * 1.3) * 0.55 * (t < 0.3)", { a = a, b = b, c = c })
]==],
	-- A meteor hit: a crunchy noise burst over a falling buzz. 0.4 s.
	hit = [==[
length(0.4)
local buzz = saw(sweep(220, 55, 8)) * decay(0.35, 2) * 0.3
local crunch = lowpass(noise(), 1600) * decay(0.2, 3) * 0.7
return (buzz + crunch) * 0.8
]==],
	-- The shield dash: a quick rising whoosh. 0.25 s.
	dash = [==[
length(0.25)
local whoosh = lowpass(noise(), 3200) * decay(0.22, 2) * 0.3
local zip = expr("sin(tau * lerp(300, 900, t / 0.25) * t) * exp(-t * 12) * 0.18")
return expr("(a + b) * (t < 0.25)", { a = whoosh, b = zip })
]==],
	-- Time is up: four rising notes. 1 s.
	finish = [==[
length(1)
local function note(f, at) return expr("sin(tau * f * (t - s)) * exp(-(t - s) * 5) * 0.3 * step(s, t)", { f = f, s = at }) end
return expr("tanh((a + b + c + d) * 1.4) * 0.5 * (t < 1)", { a = note(587, 0), b = note(740, 0.12), c = note(880, 0.24), d = note(1175, 0.36) })
]==],
}

return { sounds = SOUNDS, init = init, tick = tick, draw = draw, player_joined = player_joined, player_left = player_left, players_changed = players_changed }
