-- Neon Trails: leave a trail, leave no escape. Riders move one cell at a
-- time on a 48 x 26 grid and can only turn; a wall, any trail or a head-on
-- crash ends a rider's round. The last rider standing scores 3. First to
-- WIN points wins the match.
--
-- Ported from the LÖVE party pack (love-party/games/trails.lua): the same
-- grid, step time, start positions, collisions, scoring and bot. The arena
-- is 3D here: glowing trail blocks on a dark floor under a night sky.

local W, H = 48, 26 -- cells
local STEP = 0.105 -- seconds per cell at 100 % speed
local SPEED = 100 -- percent
local ROUND_PAUSE = 1.4 -- seconds between rounds
local WIN = 15 -- points to win the match
local MAX_RIDERS = 4

-- LÖVE's start cells and directions, per rider.
local START_X = { 8, 39, 8, 39 }
local START_Y = { 6, 19, 19, 6 }
local START_DX = { 1, -1, 1, -1 }

local TRAIL = vector.create(0.9, 0.6, 0.9)

-- A trail cell runs on into the next along the rider's way: one unbroken wall.
local function trail_size(r)
	if r.dx ~= 0 then return vector.create(1.02, 0.5, 0.55) end
	return vector.create(0.55, 0.5, 1.02)
end
local HEAD = vector.create(0.62, 0.9, 0.62)

local function key(x, y)
	return y * W + x
end

-- A cell's centre in the world: 1 m cells, the arena centred on the origin.
local function at(x, y, height)
	return vector.create(x - (W - 1) / 2, height, y - (H - 1) / 2)
end

local function dim(c)
	return c * 0.35
end

-- Neon: deepen a colour (1 stays 1, the rest drops), so it stays a colour
-- under the glow instead of fading to pastel.
local function neon(v)
	return vector.create(v.x ^ 1.4, v.y ^ 1.4, v.z ^ 1.4)
end

-- Every look a round uses, spawned once (hidden) so later spawns never
-- rebuild the renderer mid-round.
local function warm(p)
	for _, look in { { neon(p.color), 0.03 }, { dim(neon(p.color)), 0.004 } } do
		local id = world.spawn("cube", { size = TRAIL, color = look[1], emissive = look[2], visible = false, solid = false, tag = "warm" })
		world.remove(id)
	end
end

local function clear_round()
	for _, tag in { "trail", "head" } do
		for _, id in world.find(tag) do
			world.remove(id)
		end
	end
end

local function new_round()
	clear_round()
	state.grid = {}
	state.clock = 0
	state.pause = nil
	state.round += 1
	local riders = {}
	for i, p in players.list() do
		if i > MAX_RIDERS then break end
		local r = {
			id = p.id,
			bot = p.bot,
			color = neon(p.color),
			x = START_X[i],
			y = START_Y[i],
			dx = START_DX[i],
			dy = 0,
			alive = true,
			cells = {},
		}
		r.head = world.spawn("cube", { pos = at(r.x, r.y, 0.45), size = HEAD, color = "#ffffff", emissive = 0.06, solid = false, tag = "head" })
		state.grid[key(r.x, r.y)] = r.id
		table.insert(r.cells, world.spawn("cube", { pos = at(r.x, r.y, 0.25), size = trail_size(r), color = r.color, emissive = 0.03, solid = false, tag = "trail" }))
		table.insert(riders, r)
	end
	state.riders = riders
end

local function score_of(id)
	return state.scores[tostring(id)] or 0
end

local function add_score(id, n)
	state.scores[tostring(id)] = score_of(id) + n
end

local function init()
	-- A glossy dark deck floating in space, a glowing rim with running
	-- lights, beacons on the corners and a faint grid every 8 cells.
	world.spawn("cube", { pos = vector.create(0, -0.15, 0), size = vector.create(W + 1, 0.3, H + 1), color = "#0e1626", emissive = 0.004, roughness = 0.3, metal = 0.25, tag = "arena" })
	for _, side in { -1, 1 } do
		world.spawn("cube", { pos = vector.create(side * (W / 2 + 0.7), 0.05, 0), size = vector.create(0.4, 0.6, H + 1.8), color = "#0b1220", roughness = 0.35, metal = 0.6, tag = "arena" })
		world.spawn("cube", { pos = vector.create(0, 0.05, side * (H / 2 + 0.7)), size = vector.create(W + 1.8, 0.6, 0.4), color = "#0b1220", roughness = 0.35, metal = 0.6, tag = "arena" })
		world.spawn("cube", { pos = vector.create(side * (W / 2 + 0.5), 0.36, 0), size = vector.create(0.06, 0.04, H + 1.1), color = "#2f7fff", emissive = 0.05, solid = false, tag = "arena" })
		world.spawn("cube", { pos = vector.create(0, 0.36, side * (H / 2 + 0.5)), size = vector.create(W + 1.1, 0.04, 0.06), color = "#2f7fff", emissive = 0.05, solid = false, tag = "arena" })
		for x = 2, W - 1, 4 do
			world.spawn("cube", { pos = vector.create(x - W / 2, 0.38, side * (H / 2 + 0.7)), size = vector.create(0.14, 0.06, 0.14), color = "#7fd0ff", emissive = 0.05, solid = false, tag = "arena" })
		end
		for y = 2, H - 1, 4 do
			world.spawn("cube", { pos = vector.create(side * (W / 2 + 0.7), 0.38, y - H / 2), size = vector.create(0.14, 0.06, 0.14), color = "#7fd0ff", emissive = 0.05, solid = false, tag = "arena" })
		end
	end
	state.beacons = {}
	for _, cx in { -1, 1 } do
		for _, cz in { -1, 1 } do
			local x, z = cx * (W / 2 + 1.3), cz * (H / 2 + 1.3)
			world.spawn("cube", { pos = vector.create(x, 0.4, z), size = vector.create(0.8, 1.2, 0.8), color = "#101a2c", roughness = 0.3, metal = 0.7, tag = "arena" })
			world.spawn("cube", { pos = vector.create(x, 1.06, z), size = vector.create(0.6, 0.12, 0.6), color = "#2f7fff", emissive = 0.06, solid = false, tag = "arena" })
			table.insert(state.beacons, vector.create(x, 1.6, z))
		end
	end
	for x = 8, W - 1, 8 do
		world.spawn("cube", { pos = vector.create(x - W / 2, 0.004, 0), size = vector.create(0.04, 0.01, H), color = "#1d4f8f", emissive = 0.006, solid = false, tag = "arena" })
	end
	for y = 8, H - 1, 8 do
		world.spawn("cube", { pos = vector.create(0, 0.004, y - H / 2), size = vector.create(W, 0.01, 0.04), color = "#1d4f8f", emissive = 0.006, solid = false, tag = "arena" })
	end
	for _ = 1, 260 do
		world.spawn("cube", { pos = vector.create(random.range(-90, 90), random.range(-70, -35), random.range(-60, 60)), size = random.range(0.06, 0.14), color = "#cfe0ff", emissive = 0.08, solid = false, tag = "space" })
	end
	state.flashes = {}
	-- Soft neon glow, a darker frame and a little film.
	post.set({ bloom = 0.55, bloom_radius = 3, bloom_threshold = 0.35, saturation = 1.2, contrast = 1.08, vignette = 0.55, grain = 0.2, aberration = 1.2 })
	for _, p in players.list() do
		warm(p)
	end
	state.scores = {}
	state.round = 0
	state.winner = nil
	new_round()
	-- One camera over the whole arena.
	camera.set(1, { pos = vector.create(0, 32, 19), target = vector.create(0, 0, 1), fov = 50 })
end

-- Read a turn from the stick or the d-pad; never straight back.
local function steer(r, p)
	local dx, dy = 0, 0
	local x, y = p.move_x, -p.move_y -- grid y grows towards the camera
	if p.held.left then x = -1 elseif p.held.right then x = 1 end
	if p.held.up then y = -1 elseif p.held.down then y = 1 end
	if math.abs(x) > math.abs(y) and math.abs(x) > 0.3 then
		dx = if x > 0 then 1 else -1
	elseif math.abs(y) > 0.3 then
		dy = if y > 0 then 1 else -1
	end
	if (dx ~= 0 or dy ~= 0) and (dx ~= -r.dx or dy ~= -r.dy) then
		r.next_dx, r.next_dy = dx, dy
	end
end

local function free(x, y)
	return x >= 0 and x < W and y >= 0 and y < H and not state.grid[key(x, y)]
end

-- LÖVE's bot: keep going, else turn left, else right, looking two cells ahead.
local function bot(r)
	for _, v in { { r.dx, r.dy }, { -r.dy, r.dx }, { r.dy, -r.dx } } do
		if free(r.x + v[1] * 2, r.y + v[2] * 2) and free(r.x + v[1], r.y + v[2]) then
			r.next_dx, r.next_dy = v[1], v[2]
			return
		end
	end
end

local function crash(r)
	r.alive = false
	world.remove(r.head)
	for _, id in r.cells do
		world.set(id, { color = dim(r.color), emissive = 0.004 })
	end
	sound.play("crash", { at = at(r.x, r.y, 0.5) })
	table.insert(state.flashes, { pos = at(r.x, r.y, 1), color = r.color, ttl = 0.5 })
	fx.burst(at(r.x, r.y, 0.4), { color = r.color, emissive = 0.03, count = 24, speed = 6, life = 0.6, size = 0.1, stretch = 0.03, flat = true })
	fx.ring(at(r.x, r.y, 0.2), { color = r.color, emissive = 0.03, radius = 3, life = 0.4, width = 0.08 })
	players.rumble(r.id, 0.8, 0.3)
end

local function step()
	local targets = {}
	for _, r in state.riders do
		if r.alive then
			r.dx, r.dy = r.next_dx or r.dx, r.next_dy or r.dy
			r.next_dx, r.next_dy = nil, nil
			r.nx, r.ny = r.x + r.dx, r.y + r.dy
			local k = key(r.nx, r.ny)
			targets[k] = (targets[k] or 0) + 1
		end
	end
	local survivors = 0
	for _, r in state.riders do
		if r.alive then
			if not free(r.nx, r.ny) or targets[key(r.nx, r.ny)] > 1 then
				crash(r)
			else
				r.x, r.y = r.nx, r.ny
				survivors += 1
			end
		end
	end
	for _, r in state.riders do
		if r.alive then
			state.grid[key(r.x, r.y)] = r.id
			table.insert(r.cells, world.spawn("cube", { pos = at(r.x, r.y, 0.25), size = trail_size(r), color = r.color, emissive = 0.03, solid = false, tag = "trail" }))
			world.set(r.head, { pos = at(r.x, r.y, 0.45) })
		end
	end
	if survivors <= 1 then
		for _, r in state.riders do
			if r.alive then
				add_score(r.id, 3)
				local p = players.get(r.id)
				if score_of(r.id) >= WIN and p and not state.winner then
					state.winner = p.name
					game.log(p.name .. " won Neon Trails")
					game.finish()
				end
			end
		end
		sound.play("round")
		state.pause = ROUND_PAUSE
	end
end

-- Light: a lamp over every rider's head, crash flashes and the beacons.
local function light_up(dt)
	local lamps = {}
	for i = #state.flashes, 1, -1 do
		local f = state.flashes[i]
		f.ttl -= dt
		if f.ttl <= 0 then table.remove(state.flashes, i) elseif #lamps < 24 then table.insert(lamps, { pos = f.pos, color = f.color, intensity = 1500 * f.ttl, range = 8 }) end
	end
	for _, r in state.riders do
		if r.alive then table.insert(lamps, { pos = at(r.x, r.y, 1), color = r.color, intensity = 300, range = 6 }) end
	end
	for _, b in state.beacons do table.insert(lamps, { pos = b, color = "#2f7fff", intensity = 200, range = 6 }) end
	lights.set(lamps)
end

local function tick(dt, input)
	light_up(dt)
	if state.winner then return end
	if state.pause then
		state.pause -= dt
		if state.pause <= 0 then new_round() end
		return
	end
	for _, r in state.riders do
		if r.alive then
			if r.bot then bot(r) elseif input[r.id] then steer(r, input[r.id]) end
		end
	end
	state.clock += dt
	local step_time = STEP * 100 / SPEED
	if state.clock >= step_time then
		state.clock -= step_time
		step()
	end
end

local function draw()
	local w, h = hud.size()
	local s = h / 720
	-- Every player's score along the top, in their colour.
	local list = players.list()
	local slot = w / math.max(#list, 1)
	for i, p in list do
		local x = (i - 1) * slot
		hud.text(p.name, x, 14 * s, { size = 24 * s, align = "center", width = slot, color = p.color })
		hud.text(tostring(score_of(p.id)), x, 40 * s, { size = 44 * s, align = "center", width = slot, color = p.color })
	end
	if state.winner then
		hud.text(state.winner .. " wins!", 0, h * 0.42, { size = 72 * s, align = "center", width = w })
	elseif state.pause then
		hud.text("LAST RIDER +3  /  NEXT ROUND", 0, h * 0.45, { size = 40 * s, align = "center", width = w })
	end
end

-- Someone joins: they ride from the next round (their colours warmed now).
local function player_joined(p)
	warm(p)
end

-- Someone leaves: their rider crashes out of this round.
local function player_left(p)
	for _, r in state.riders do
		if r.id == p.id and r.alive then crash(r) end
	end
end

return { init = init, tick = tick, draw = draw, player_joined = player_joined, player_left = player_left }
