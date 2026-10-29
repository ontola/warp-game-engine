-- Anticonception: protect the egg. A co-op game: every guard is an arc on
-- a ring around the egg and turns left or right around it (stick or A/D),
-- A (or RB / Space) boosts the turn and B widens the guard for a second,
-- every four seconds. Swimmers come in from the edge, faster as time goes
-- on; every one a guard catches is 10 points for the team, every one that
-- reaches the egg costs one of its five lives. Survive 90 seconds.
--
-- Ported from the LÖVE party pack (gamenight-games love-party/games/
-- orbit.lua, "orbit-guard"): the same ring radii, guard widths, turn
-- speeds, cooldowns, spawn interval, swimmer speeds, scoring and bot,
-- simulated in the LÖVE screen units (1280 x 800) and shown in 3D, 40
-- units to the metre, under one shared angled camera.
--! title = "Anticonception"
--! id = "anticonception-warp"
--! tick_rate = 60
--! split = false
--! players = 1
--! bots = 1
--! min_players = 1
--! seed = 9
--! sky = "night:clear"
--! max_entities = 1500

local DURATION = 90 -- seconds
local LIVES = 5
local RING, OUTER, EGG_R = 180, 300, 35 -- units
local NARROW, WIDE = 0.25, 0.55 -- guard half-widths, radians
local TURN, BOOST = 1.9, 3.2 -- radians per second
local WIDE_TIME, WIDE_COOL = 1, 4
local MAX_ROCKS = 80
local SEGMENTS = 9 -- cubes per guard arc
local S = 1 / 40 -- metres per unit
local TAU = math.pi * 2

local SWIMMER = { 1, 0.43, 0.3 }
local EGG = { 0.98, 0.9, 0.74 }
local TRACK = { 0.12, 0.18, 0.24 }

local function rgb(c, k)
	k = k or 1
	return vector.create(c[1] * k, c[2] * k, c[3] * k)
end

-- A point at `r` units from the egg in direction `a` (LÖVE's screen
-- angle: x right, y down, so it turns clockwise seen from the camera).
local function polar(a, r, h)
	return vector.create(math.cos(a) * r * S, h, math.sin(a) * r * S)
end

local function difference(a, b)
	return (a - b + math.pi) % TAU - math.pi
end

local function clamp(v, a, b)
	return math.max(a, math.min(b, v))
end

----------------------------------------------------------------------------
-- Guards

local function warm(p)
	world.warm("cube", { color = p.color, emissive = 0.1 })
	world.warm("cube", { color = p.color, emissive = 0.2 })
	world.warm("cube", { color = p.color, emissive = 0.06 })
end

local function new_guard(p, angle)
	local g = { id = p.id, name = p.name, color = p.color, bot = p.bot, angle = angle, cool = 0, wide = 0, segs = {} }
	for _ = 1, SEGMENTS do
		table.insert(g.segs, world.spawn("cube", { size = vector.create(0.3, 0.3, 0.3), color = p.color, emissive = 0.1, solid = false, tag = "guard" }))
	end
	return g
end

local function remove_guard(g)
	for _, id in g.segs do world.remove(id) end
end

local function show_guard(g)
	local width = if g.wide > 0 then WIDE else NARROW
	local wide = g.wide > 0
	local step = width * 2 / SEGMENTS
	if wide ~= g.shown then
		-- Each cube covers its share of the arc, a little overlapping.
		for _, id in g.segs do
			world.set(id, { size = vector.create(0.26, 0.32, step * RING * S * 1.15), emissive = if wide then 0.2 else 0.1 })
		end
		g.shown = wide
	end
	for i, id in g.segs do
		local a = g.angle - width + (i - 0.5) * step
		-- The cube's length runs along the ring: its yaw faces the tangent.
		world.place(id, polar(a, RING, 0.25), -a)
	end
end

----------------------------------------------------------------------------
-- The game

local s

local function ring(r, color, glow, n, width)
	for i = 1, n do
		local a = (i - 0.5) / n * TAU
		world.spawn("cube", { pos = polar(a, r, 0.02), rot = vector.create(0, -a, 0), size = vector.create(width, 0.03, TAU * r * S / n * 1.05), color = color, emissive = glow, solid = false, tag = "arena" })
	end
end

local function arena()
	world.spawn("cube", { pos = vector.create(0, -0.6, 0), size = vector.create(60, 1, 60), color = "#03050b", roughness = 0.95, tag = "arena" })
	world.spawn("sphere", { pos = vector.create(0, -0.05, 0), size = vector.create((OUTER + 40) * 2 * S, 0.1, (OUTER + 40) * 2 * S), color = "#0b1220", emissive = 0.004, roughness = 0.85, tag = "arena" })
	ring(OUTER, rgb(TRACK), 0.03, 64, 0.05)
	ring(RING, rgb(TRACK, 1.3), 0.04, 48, 0.05)
	-- Faint specks drifting on the dark.
	for _ = 1, 90 do
		local a, r = random.range(0, TAU), random.range(50, OUTER + 30)
		local size = random.range(0.03, 0.07)
		world.spawn("cube", { pos = polar(a, r, 0.01), size = vector.create(size, 0.01, size), color = "#bfd6ff", emissive = 0.03, solid = false, tag = "arena" })
	end
	-- The egg: an upright glowing ellipsoid (25 x 31 units in LÖVE) and its
	-- bright nucleus, with one pip per life around it.
	s.egg = world.spawn("sphere", { pos = vector.create(0, 31 * S, 0), size = vector.create(50 * S, 62 * S, 50 * S), color = rgb(EGG), emissive = 0.08, roughness = 0.4, solid = false, tag = "egg" })
	world.spawn("sphere", { pos = vector.create(-0.14, 0.9, 0.42), size = vector.create(0.26, 0.34, 0.26), color = "#fffae6", emissive = 0.25, solid = false, tag = "egg" })
	s.pips = {}
	for i = 1, LIVES do
		local a = i * TAU / LIVES
		table.insert(s.pips, world.spawn("sphere", { pos = polar(a, 38 + 12, 0.1), size = 7 * S, color = rgb(EGG), emissive = 0.15, solid = false, tag = "pip" }))
	end
end

local function warm_all()
	world.warm("sphere", { color = rgb(SWIMMER), emissive = 0.1 })
	world.warm("cube", { color = rgb(SWIMMER, 0.8), emissive = 0.04 })
	world.warm("cube", { color = rgb(SWIMMER), emissive = 0.06 })
	world.warm("cube", { color = rgb(EGG), emissive = 0.06 })
	for _, p in players.list() do warm(p) end
end

local function init()
	state.s = { guards = {}, rocks = {}, time = 0, clock = -3, hp = LIVES, teamScore = 0, kills = 0, wave = 1, over = false, survived = false, remaining = DURATION, sfx = {}, pulse = 0 }
	s = state.s
	warm_all()
	arena()
	local list = players.list()
	for i, p in list do
		table.insert(s.guards, new_guard(p, (i - 1) * TAU / #list))
	end
	camera.set(1, { pos = vector.create(0, 14, 12), target = vector.create(0, 0, 0.4), fov = 48 })
end

-- LÖVE's controls: x turns, A (or RB) boosts, B widens.
local function controls(p)
	if not p then return { x = 0, action = false, secondary = false } end
	local x = p.move_x
	if p.held.left then x = -1 elseif p.held.right then x = 1 end
	return { x = x, action = p.held.south or p.held.right_shoulder or p.held.fire, secondary = p.held.east }
end

-- LÖVE's bot: turn towards the nearest swimmer still outside the ring,
-- boost when it is far, widen when it is close. One change: a bot leaves a
-- swimmer to a guard that is closer to it, so bots that chased the same
-- one do not stay stacked for the rest of the round (in LÖVE they do).
local function closer_guard(g, r, d)
	for _, other in s.guards do
		if other ~= g and math.abs(difference(r.angle, other.angle)) < d then return true end
	end
	return false
end

local function bot(g)
	local best, dist = nil, math.huge
	for _, r in s.rocks do
		local d = math.abs(difference(r.angle, g.angle))
		if r.r > 170 and d < dist and not closer_guard(g, r, d) then best, dist = r, d end
	end
	local d = if best then difference(best.angle, g.angle) else 0
	return { x = clamp(d * 5, -1, 1), action = math.abs(d) > 0.5, secondary = dist < 0.3 }
end

local function sfx(name)
	s.sfx[name] = true
end

local function spawn_rock()
	local r = { angle = random.float() * TAU, r = OUTER, speed = math.min(55, 38 + s.time * 0.18), wiggle = random.range(0, TAU) }
	r.head = world.spawn("sphere", { pos = polar(r.angle, r.r, 0.2), size = 14 * S, color = rgb(SWIMMER), emissive = 0.1, solid = false, tag = "swimmer" })
	r.tail = world.spawn("cube", { pos = polar(r.angle, r.r + 9, 0.2), size = vector.create(0.05, 0.04, 18 * S), color = rgb(SWIMMER, 0.8), emissive = 0.04, solid = false, tag = "swimmer" })
	table.insert(s.rocks, r)
end

local function remove_rock(i)
	local r = s.rocks[i]
	world.remove(r.head)
	world.remove(r.tail)
	table.remove(s.rocks, i)
end

local function show_rock(r)
	world.place(r.head, polar(r.angle, r.r, 0.2))
	-- The tail points back outwards and wags.
	local wag = math.sin(s.time * 14 + r.wiggle) * 0.35
	local a = r.angle + wag * 0.04
	world.place(r.tail, polar(a, r.r + 13, 0.2), -r.angle + math.pi / 2 + wag)
end

local function finish()
	s.over = true
	s.survived = s.hp > 0
	sfx("finish")
	game.log("Anticonception: " .. (if s.survived then "the egg survived" else "the egg fell") .. ", " .. s.teamScore .. " points")
	game.finish()
end

local function tick(dt, input)
	s = state.s
	if not s.over then
		s.time += dt
		s.wave = 1 + math.floor(s.time / 10)
		for _, g in s.guards do
			local c = if g.bot then bot(g) else controls(input[g.id])
			g.angle = (g.angle + (c.x or 0) * (if c.action then BOOST else TURN) * dt) % TAU
			g.cool = math.max(0, g.cool - dt)
			g.wide = math.max(0, g.wide - dt)
			if c.secondary and g.cool == 0 then
				g.wide = WIDE_TIME
				g.cool = WIDE_COOL
				sfx("wide")
			end
		end
		s.clock += dt
		local interval = math.max(0.65, 1.2 - s.time * 0.004) / (1 + (#s.guards - 2) * 0.1)
		while s.clock >= interval do
			s.clock -= interval
			if #s.rocks < MAX_ROCKS then spawn_rock() end
		end
		for i = #s.rocks, 1, -1 do
			local rock = s.rocks[i]
			local before = rock.r
			rock.r -= rock.speed * dt
			local blocked = nil
			if before >= RING - 7 and rock.r <= RING + 7 then
				for _, g in s.guards do
					if math.abs(difference(rock.angle, g.angle)) < (if g.wide > 0 then WIDE else NARROW) then
						blocked = g
						break
					end
				end
			end
			if blocked then
				s.kills += 1
				s.teamScore += 10
				sfx("point")
				fx.burst(polar(rock.angle, RING, 0.25), { color = blocked.color, emissive = 0.06, count = 10, speed = 3, life = 0.3 })
				remove_rock(i)
			elseif rock.r < EGG_R then
				s.hp = math.max(0, s.hp - 1)
				sfx("hurt")
				s.pulse = 1
				fx.burst(polar(rock.angle, EGG_R, 0.6), { color = rgb(SWIMMER), emissive = 0.06, count = 16, speed = 3, life = 0.45 })
				fx.ring(vector.create(0, 0.05, 0), { color = rgb(SWIMMER), emissive = 0.06, radius = 2.2, life = 0.4 })
				for _, g in s.guards do players.rumble(g.id, 0.6, 0.25) end
				remove_rock(i)
			end
		end
		s.remaining = math.max(0, DURATION - s.time)
		if s.hp == 0 or s.remaining == 0 then finish() end
	end
	-- Show it: the guards, the swimmers, the egg's lives and its flinch.
	for _, g in s.guards do show_guard(g) end
	for _, r in s.rocks do show_rock(r) end
	for i, id in s.pips do
		local on = i <= s.hp
		local a = i * TAU / LIVES + s.time * 0.4
		world.set(id, { visible = on })
		if on then world.place(id, polar(a, 50, 0.12 + math.sin(s.time * 3 + i) * 0.04)) end
	end
	s.pulse = math.max(0, s.pulse - dt * 3)
	local k = 1 + s.pulse * 0.15 + math.sin(s.time * 2.2) * 0.02
	world.set(s.egg, { size = vector.create(50 * S * k, 62 * S * k, 50 * S * k), pos = vector.create(0, 31 * S * k, 0) })
	for name in s.sfx do
		sound.play(name, { pitch = if name == "point" then 0.95 + random.float() * 0.12 else 1 })
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
	hud.text(string.format("%02d", math.ceil(s.remaining)), 0, 14 * k, { size = 30 * k, align = "center", width = w, color = if s.remaining < 10 then { 1, 0.43, 0.4, 1 } else dim })
	-- Every guard's name and the egg's lives, in LÖVE's four slots.
	local slots = { 28 / 1280, 0.23, 0.59, 1 - 248 / 1280 }
	for i, g in s.guards do
		local x = w * (slots[i] or ((i - 1) % 4 * 0.25 + 0.12))
		hud.text(g.name, x, 12 * k, { size = 18 * k, color = g.color })
		local wide = if g.cool == 0 then "B READY" else "B " .. math.ceil(g.cool) .. "s"
		hud.text((if s.hp == 0 then "down" else string.rep("•", s.hp)) .. "   " .. wide, x, 34 * k, { size = 18 * k, color = dim })
	end
	hud.text(s.teamScore .. "   /   WAVE " .. s.wave, 0, h - 44 * k, { size = 26 * k, align = "center", width = w, color = { 0.4, 0.65, 0.63, 1 } })
	if s.clock < 0 and not s.over then
		hud.text("PROTECT THE EGG", 0, h * 0.2, { size = 34 * k, align = "center", width = w, color = dim })
	end
	if s.over then
		hud.box(w * 0.2, h * 0.32, w * 0.6, h * 0.34, { 0.02, 0.03, 0.06, 0.88 })
		hud.text(if s.survived then "THE EGG SURVIVED" else "TEAM DOWN", 0, h * 0.35, { size = 40 * k, align = "center", width = w, color = { 0.3, 0.92, 0.83, 1 } })
		hud.text(s.kills .. " CAUGHT / WAVE " .. s.wave, 0, h * 0.35 + 52 * k, { size = 24 * k, align = "center", width = w })
		hud.text(s.teamScore .. " POINTS", 0, h * 0.35 + 90 * k, { size = 64 * k, align = "center", width = w })
	end
end

-- Someone joins: a guard opposite the first one; the team's score stays.
local function player_joined(p)
	s = state.s
	warm(p)
	local a = if s.guards[1] then s.guards[1].angle + math.pi else 0
	table.insert(s.guards, new_guard(p, a % TAU))
end

local function player_left(p)
	s = state.s
	for i, g in s.guards do
		if g.id == p.id then
			remove_guard(g)
			table.remove(s.guards, i)
			return
		end
	end
end

local function players_changed()
	s = state.s
	for _, p in players.list() do
		for _, g in s.guards do
			if g.id == p.id then g.name, g.color = p.name, p.color end
		end
	end
end

----------------------------------------------------------------------------
-- Sounds (engine/docs/audio.md): sound.play("point") plays SOUNDS.point.

local SOUNDS = {
	-- A swimmer caught: a soft bright blip, 740 Hz with a falling pitch. 0.15 s.
	point = [==[
length(0.15)
local f = expr("lerp(740, 480, t / 0.15)")
local tone = expr("sin(tau * f * t) * exp(-t * 26) * 0.4", { f = f })
local tick = highpass(noise(), 3000) * decay(0.03, 6) * 0.15
return expr("tanh((a + b) * 1.3) * 0.6 * (t < 0.15)", { a = tone, b = tick })
]==],
	-- A swimmer reaches the egg: a low thud and a sinking wobble. 0.5 s.
	hurt = [==[
length(0.5)
local thud = expr("sin(tau * lerp(130, 50, t / 0.5) * t) * exp(-t * 7) * 0.55")
local wobble = expr("sin(tau * (220 + sin(tau * 9 * t) * 30) * t) * exp(-t * 5) * 0.18")
local hiss = lowpass(noise(), 1200) * decay(0.2, 3) * 0.25
return expr("tanh((a + b + c) * 1.2) * 0.75 * (t < 0.5)", { a = thud, b = wobble, c = hiss })
]==],
	-- The wide guard: a shimmering rising chord. 0.4 s.
	wide = [==[
length(0.4)
local a = expr("sin(tau * lerp(330, 495, t / 0.4) * t) * exp(-t * 6) * 0.2")
local b = expr("sin(tau * lerp(495, 742, t / 0.4) * t) * exp(-t * 6) * 0.15")
return expr("(a + b) * (t < 0.4)", { a = a, b = b })
]==],
	-- The end: four notes up (the egg survived or not, the round is over). 1 s.
	finish = [==[
length(1)
local function note(f, at) return expr("sin(tau * f * (t - s)) * exp(-(t - s) * 5) * 0.3 * step(s, t)", { f = f, s = at }) end
return expr("tanh((a + b + c + d) * 1.4) * 0.5 * (t < 1)", { a = note(523, 0), b = note(659, 0.12), c = note(784, 0.24), d = note(1047, 0.36) })
]==],
}

return { sounds = SOUNDS, init = init, tick = tick, draw = draw, player_joined = player_joined, player_left = player_left, players_changed = players_changed }
