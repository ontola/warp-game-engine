-- Paint Rush: leave your colour everywhere. Every tile you roll over turns
-- your colour; A paints a diamond around you (every 4 s). Two players
-- claiming the same tile in the same step leave it as it was. Your score
-- is the number of tiles you own; after ROUND seconds the most tiles win.
--
-- Ported from the LÖVE party pack (love-party/games/paint.lua and
-- shared/arena.lua): the same 37 x 16 tile field, speed, burst, cooldown,
-- contested tiles, scoring and bot, simulated at LÖVE's 120 Hz (two steps
-- a tick). `sync()` shows it in 3D: painted floor tiles that pop up as they
-- change colour, under one shared camera.

local ROUND = 60 -- seconds per round, as the LÖVE runner
local RESULTS = 3 -- seconds of results before the next round
local MAX_PLAYERS = 4
local STEP = 1 / 120 -- LÖVE's fixed step

-- LÖVE's arena units: tiles of 30 from (80, 170), players kept inside.
local COLS, ROWS, TILE = 37, 16, 30
local LEFT, TOP = 80, 170
local MIN_X, MAX_X, MIN_Y, MAX_Y = 90, 1190, 180, 650
local SPEED = 260
local BURST, COOLDOWN = 2, 4
local CX, CY = LEFT + COLS * TILE / 2, TOP + ROWS * TILE / 2

local S = 1 / TILE -- metres per arena unit: a tile is 1 m
-- A player's diameter: LÖVE draws 24 units; a little larger reads better
-- in 3D over tiles of the same colour.
local BALL = 32 * S
local FLOOR = vector.create(0.075, 0.1, 0.145)
local POP = 0.25 -- seconds a freshly painted tile takes to settle

local function len(x, y)
	return math.sqrt(x * x + y * y)
end

local function clamp(v, a, b)
	return math.max(a, math.min(b, v))
end

local function at(x, y, h)
	return vector.create((x - CX) * S, h, (y - CY) * S)
end

local function tile_at(key)
	return at(LEFT + (key % COLS + 0.5) * TILE, TOP + (math.floor(key / COLS) + 0.5) * TILE, -0.05)
end

local function paint(c)
	return c * 0.62
end

local s -- the round: state.s

----------------------------------------------------------------------------
-- The simulation (LÖVE's M.new, M.update and M.bot)

local function new_round()
	for _, tag in { "player" } do
		for _, id in world.find(tag) do
			world.remove(id)
		end
	end
	for key = 0, COLS * ROWS - 1 do
		world.set(state.tiles[key + 1], { pos = tile_at(key), color = FLOOR, emissive = 0 })
	end
	local list = {}
	for _, p in players.list() do
		if #list < MAX_PLAYERS then table.insert(list, p) end
	end
	-- tiles: tile key -> owner id; pops: tile keys still settling.
	s = { players = {}, tiles = {}, pops = {}, time = 0, remaining = ROUND, results = nil, sfx = {} }
	state.s = s
	state.round += 1
	for i, p in list do
		local q = {
			id = p.id,
			bot = p.bot,
			color = p.color,
			x = 200 + (i - 1) * 280,
			y = if i % 2 == 0 then 580 else 240,
			cool = 0,
			score = 0,
		}
		q.body = world.spawn("sphere", { pos = at(q.x, q.y, BALL / 2), size = BALL, color = q.color, emissive = 0.12, roughness = 0.3, solid = false, tag = "player" })
		q.shine = world.spawn("sphere", { size = BALL * 0.3, color = "#ffffff", emissive = 0.2, roughness = 0.2, solid = false, tag = "player" })
		q.halo = world.spawn("sphere", { size = vector.create(36 * S, 0.03, 36 * S), color = q.color, emissive = 0.35, solid = false, tag = "player" })
		table.insert(s.players, q)
	end
	sound.play("round", { pitch = 0.75 })
end

local function update(dt, controls)
	s.time += dt
	local claims = {}
	for i, q in s.players do
		local c = controls[i]
		local n = math.max(1, len(c.x, c.y))
		q.x = clamp(q.x + c.x / n * SPEED * dt, MIN_X, MAX_X)
		q.y = clamp(q.y + c.y / n * SPEED * dt, MIN_Y, MAX_Y)
		q.cool = math.max(0, q.cool - dt)
		local radius = 0
		if c.action and q.cool == 0 then
			radius = BURST
			q.cool = COOLDOWN
			table.insert(s.sfx, { name = "burst", at = at(q.x, q.y, 0.05), color = q.color })
		end
		local cx, cy = math.floor((q.x - LEFT) / TILE), math.floor((q.y - TOP) / TILE)
		for y = cy - radius, cy + radius do
			for x = cx - radius, cx + radius do
				if x >= 0 and x < COLS and y >= 0 and y < ROWS and math.abs(x - cx) + math.abs(y - cy) <= radius then
					local key = y * COLS + x
					-- A contested tile stays unchanged, independent of player order.
					local entry = claims[key]
					if entry == nil then
						claims[key] = q.id
					elseif entry ~= q.id then
						claims[key] = false
					end
				end
			end
		end
	end
	for key, id in claims do
		if id and s.tiles[key] ~= id then
			s.tiles[key] = id
			s.pops[key] = POP
		end
	end
	local owned = {}
	for _, id in s.tiles do
		owned[id] = (owned[id] or 0) + 1
	end
	for _, q in s.players do
		q.score = owned[q.id] or 0
	end
end

-- LÖVE's bot: sweep a figure across the field, bursting whenever it can.
local function bot(i, q)
	local angle = s.time * 0.65 + i * 1.57
	local tx = 640 + math.sin(angle) * 500
	local ty = 410 + math.sin(angle * 1.7) * 220
	local dx, dy = tx - q.x, ty - q.y
	local n = math.max(1, len(dx, dy))
	return { x = dx / n, y = dy / n, action = true }
end

-- A person's controls in arena terms (y grows down the screen, towards the
-- camera).
local function controls(i)
	if not i then return { x = 0, y = 0, action = false } end
	local x, y = i.move_x, -i.move_y
	if i.held.left then x = -1 elseif i.held.right then x = 1 end
	if i.held.up then y = -1 elseif i.held.down then y = 1 end
	if math.abs(x) < 0.2 then x = 0 end
	if math.abs(y) < 0.2 then y = 0 end
	return { x = x, y = y, action = i.held.south or i.held.right_shoulder }
end

----------------------------------------------------------------------------
-- The 3D view

local function sync(dt)
	local colors = {}
	for _, q in s.players do
		colors[q.id] = q.color
	end
	for key, t in s.pops do
		local tile = state.tiles[key + 1]
		if t == POP then
			local c = colors[s.tiles[key]]
			world.set(tile, { color = if c then paint(c) else FLOOR, emissive = if c then 0.03 else 0 })
		end
		t -= dt
		if t <= 0 then
			world.place(tile, tile_at(key))
			s.pops[key] = nil
		else
			local k = t / POP
			world.place(tile, tile_at(key) + vector.create(0, 0.18 * math.sin(k * math.pi), 0))
			s.pops[key] = t
		end
	end
	for _, q in s.players do
		local base = at(q.x, q.y, 0)
		world.place(q.body, base + vector.create(0, BALL / 2 + 0.02, 0))
		world.place(q.shine, base + vector.create(BALL * 0.14, BALL * 0.86, -BALL * 0.14))
		world.place(q.halo, base + vector.create(0, 0.04, 0))
		world.set(q.halo, { visible = q.cool == 0 })
	end
end

local function play_sounds()
	local played = false
	for _, e in s.sfx do
		fx.ring(e.at, { color = e.color, emissive = 0.4, radius = 2.6, life = 0.3, width = 0.1 })
		fx.burst(e.at + vector.create(0, 0.2, 0), { color = e.color, emissive = 0.3, count = 14, speed = 5, life = 0.35, size = 0.14, gravity = 12 })
		if not played then
			played = true
			sound.play("burst", { at = e.at, pitch = 0.92 + random.float() * 0.16 })
		end
	end
	s.sfx = {}
end

----------------------------------------------------------------------------
-- Rounds

local function winners()
	local best, names = -1, {}
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
		game.log(table.concat(names, " and ") .. " won Paint Rush with " .. best .. " tiles")
	end
	game.finish()
end

local function warm(p)
	world.warm("sphere", { color = p.color, emissive = 0.12, roughness = 0.3 })
	world.warm("sphere", { color = p.color, emissive = 0.35 })
	world.warm("cube", { color = paint(p.color), emissive = 0.03 }) -- tiles
	world.warm("cube", { color = p.color, emissive = 0.4 }) -- burst rings
	world.warm("cube", { color = p.color, emissive = 0.3 }) -- burst pieces
end

local function init()
	-- A raised slab with a rim, and a tile for every cell.
	world.spawn("cube", { pos = vector.create(0, -0.4, 0), size = vector.create(COLS + 1.2, 0.6, ROWS + 1.2), color = "#1d2838", roughness = 0.8, solid = false, tag = "arena" })
	for _, side in { -1, 1 } do
		world.spawn("cube", { pos = vector.create(side * (COLS / 2 + 0.45), 0, 0), size = vector.create(0.3, 0.3, ROWS + 1.2), color = "#3a5a8a", emissive = 0.03, solid = false, tag = "arena" })
		world.spawn("cube", { pos = vector.create(0, 0, side * (ROWS / 2 + 0.45)), size = vector.create(COLS + 1.2, 0.3, 0.3), color = "#3a5a8a", emissive = 0.03, solid = false, tag = "arena" })
	end
	state.tiles = {}
	for key = 0, COLS * ROWS - 1 do
		state.tiles[key + 1] = world.spawn("cube", { pos = tile_at(key), size = vector.create(0.92, 0.1, 0.92), color = FLOOR, roughness = 0.6, solid = false, tag = "tile" })
	end
	world.warm("sphere", { color = "#ffffff", emissive = 0.2, roughness = 0.2 })
	for _, p in players.list() do
		warm(p)
	end
	state.round = 0
	new_round()
	camera.set(1, { pos = vector.create(0, 26, 19.5), target = vector.create(0, -1, 1.2), fov = 46 })
	light.set({ sun = vector.create(-0.4, 1, 0.6), sun_lux = 600, sky_lux = 200 })
end

local function tick(dt, input)
	s = state.s
	if s.results then
		s.results -= dt
		sync(dt)
		if s.results <= 0 then new_round() end
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
	sync(dt)
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
	-- Who owns how much of the floor, as one bar.
	local bw, bx, by = w * 0.5, w * 0.25, h - 30 * k
	hud.box(bx, by, bw, 8 * k, { 0.1, 0.13, 0.18, 0.9 })
	local x = bx
	for _, q in s.players do
		local part = bw * q.score / (COLS * ROWS)
		hud.box(x, by, part, 8 * k, q.color)
		x += part
	end
	hud.text(tostring(math.ceil(s.remaining)), 0, by - 46 * k, { size = 34 * k, align = "center", width = w, color = { 0.6, 0.7, 0.8, 1 } })
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

-- Someone leaves: their ball leaves now; their tiles stay painted but
-- count for nobody.
local function player_left(p)
	s = state.s
	for k, q in s.players do
		if q.id == p.id then
			for _, id in { q.body, q.shine, q.halo } do
				world.remove(id)
			end
			table.remove(s.players, k)
			break
		end
	end
end

return { init = init, tick = tick, draw = draw, player_joined = player_joined, player_left = player_left }
