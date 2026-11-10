-- Stack Together: one shared twelve-column well. Each of the two players
-- owns six columns and drops their own pieces there; rows only clear when
-- both halves are full, so you build together. Clear TARGET rows to win; the
-- round is lost when a stack reaches the top. Y swaps your falling piece
-- with the one slot both players share, so pieces can be passed across.
--
-- Ported from the LÖVE party pack (gamenight-games love-party/games/coop/
-- stack.lua and stack_view.lua): the same well, pieces, bag, rotation kicks,
-- fall speed, shared hold slot and bot. No text on screen: the players'
-- faces, previews, the shared Y slot and a row meter carry everything.

-- Tuning, live: the `knobs` and `set_knob` tools (or `knob NAME VALUE` in
-- the console) change a value here and reload the game; play keeps going.
-- @knobs
local KNOBS = {
	target = 12, -- rows to clear together to win (the LÖVE setting, 4 to 40)
	speed = 100, -- falling speed in percent (50 to 175)
	fall_start = 0.85, -- seconds per row at the start (at 100 % speed)
	fall_step = 0.025, -- that many seconds faster per cleared row
	fall_min = 0.26, -- the fastest a piece falls on its own, seconds per row
	soft_drop = 0.045, -- seconds per row while holding down
	shift_delay = 0.19, -- hold left or right this long before it repeats
	shift_repeat = 0.075, -- seconds between repeated steps after that
	bot_patience = 0.7, -- a bot waits this long before hard-dropping a piece
	bot_lead = 12, -- a bot stops hard-dropping when its half has this many more blocks
	glide = 28, -- how fast drawn pieces catch up with their cell (per second)
	block_glow = 0.018, -- how much the blocks shine
	ghost_glow = 0.03, -- brightness of the landing outline
	bloom = 0.55, -- glow strength, 0 to 2
	bloom_threshold = 0.4, -- how bright something must be before it glows
	vignette = 0.45, -- dark corners, 0 to 1
	saturation = 1.15, -- colour strength, 1 neutral
	lamp = 260, -- brightness of the lamp over each falling piece
}
-- @end

local W, H = 12, 18 -- the well, in cells
local LANE = 6 -- columns per player
local MAX_PLAYERS = 2

-- The seven pieces, as cells in a 4 x 2 box (LÖVE's order).
local SHAPES = {
	{ { 0, 0 }, { 1, 0 }, { 2, 0 }, { 3, 0 } },
	{ { 0, 0 }, { 1, 0 }, { 0, 1 }, { 1, 1 } },
	{ { 1, 0 }, { 0, 1 }, { 1, 1 }, { 2, 1 } },
	{ { 1, 0 }, { 2, 0 }, { 0, 1 }, { 1, 1 } },
	{ { 0, 0 }, { 1, 0 }, { 1, 1 }, { 2, 1 } },
	{ { 0, 0 }, { 0, 1 }, { 1, 1 }, { 2, 1 } },
	{ { 2, 0 }, { 0, 1 }, { 1, 1 }, { 2, 1 } },
}
local BLOCKS = { "#57c9e8", "#ffcf66", "#b880e8", "#5edba8", "#f7707d", "#668ff5", "#ff9e54" }
local SLOT_COLORS = { vector.create(1, 0.58, 0.36), vector.create(0.36, 0.9, 0.73) }
local GOLD = "#ffcf66"
local MINT = "#5ce6ba"
local CORAL = "#f7707d"
local WELL = "#080a14"
local DEAD = "#6a6278"
local RIM = "#8fa6ff"

-- ---------------------------------------------------------------------------
-- Rules (the LÖVE sim, on a flat board: board[(y - 1) * W + x], 0 = empty)

local function cells(kind, rot)
	local c = {}
	local minx, miny = 10, 10
	for _, v in SHAPES[kind] do
		local x, y = v[1], v[2]
		for _ = 1, rot do
			x, y = -y, x
		end
		table.insert(c, { x, y })
		minx = math.min(minx, x)
		miny = math.min(miny, y)
	end
	for _, v in c do
		v[1] -= minx
		v[2] -= miny
	end
	return c
end

local function at_board(x, y)
	return state.board[(y - 1) * W + x]
end

local function fits(p, x, y, rot)
	for _, v in cells(p.kind, rot) do
		local cx, cy = x + v[1], y + v[2]
		if cx < p.lane or cx > p.lane + LANE - 1 or cy > H then return false end
		if cy > 0 and at_board(cx, cy) ~= 0 then return false end
	end
	return true
end

local function drop_y(p, x, rot)
	local y = p.y
	if not fits(p, x, y, rot) then return nil end
	while fits(p, x, y + 1, rot) do
		y += 1
	end
	return y
end

local function event(name)
	table.insert(state.events, name)
end

local function next_piece(p)
	if #p.bag == 0 then
		p.bag = { 1, 2, 3, 4, 5, 6, 7 }
		for i = 7, 2, -1 do
			local j = random.int(1, i)
			p.bag[i], p.bag[j] = p.bag[j], p.bag[i]
		end
	end
	return table.remove(p.bag)
end

local function lose()
	state.phase = "lost"
	event("lost")
end

local function enter(p)
	p.x, p.y, p.rot, p.fall, p.repeat_time = p.lane + 1, 1, 0, 0, 0
	p.ai = nil
	p.age = 0
	p.snap = true
	if not fits(p, p.x, p.y, 0) then lose() end
end

local function spawn_piece(p)
	p.kind = p.next or next_piece(p)
	p.next = next_piece(p)
	p.held = false
	enter(p)
end

-- Y swaps the falling piece with the one slot both players share (or takes
-- the next piece when it is empty), so a piece can be passed to the teammate.
-- Once per piece, so swapping back and forth can't stall the round.
local function hold(p)
	if p.held then return false end
	if state.hold ~= 0 then
		p.kind, state.hold = state.hold, p.kind
	else
		state.hold = p.kind
		p.kind = p.next
		p.next = next_piece(p)
	end
	state.hold_by = p.id
	enter(p)
	p.held = true
	event("hold")
	return true
end

local function clear_rows()
	local n = 0
	local kept = {}
	local cleared = {}
	for y = H, 1, -1 do
		local full = true
		for x = 1, W do
			if at_board(x, y) == 0 then
				full = false
				break
			end
		end
		if full then
			n += 1
			table.insert(cleared, y)
			-- Active pieces above a cleared row fall with the settled stack.
			for _, p in state.players do
				if p.y < y then p.y += 1 end
			end
		else
			table.insert(kept, 1, y)
		end
	end
	if n == 0 then return 0 end
	-- Bursts where the rows were, in each block's colour.
	for _, y in cleared do
		for x = 1, W, 2 do
			table.insert(state.bursts, { x, y, at_board(x, y) })
		end
	end
	local board = table.create(W * H, 0)
	local row = H - #kept
	for _, y in kept do
		row += 1
		for x = 1, W do
			board[(row - 1) * W + x] = at_board(x, y)
		end
	end
	state.board = board
	state.rows += n
	state.flash = 0.6
	event("clear")
	if state.rows >= KNOBS.target then
		state.phase = "won"
		event("won")
	end
	return n
end

local function lock(p)
	for _, v in cells(p.kind, p.rot) do
		local x, y = p.x + v[1], p.y + v[2]
		if y < 1 then
			lose()
			return
		end
		state.board[(y - 1) * W + x] = p.kind
	end
	event("lock")
	clear_rows()
	if state.phase == "play" then spawn_piece(p) end
end

-- LÖVE's bot: try every column and rotation in its own half, score the
-- resulting stack (holes, height, bumpiness, full rows) and take the best.
local function plan(p)
	local best, cost = nil, math.huge
	for r = 0, 3 do
		for x = p.lane, p.lane + LANE - 1 do
			local y = drop_y(p, x, r)
			if y then
				local added = {}
				for _, v in cells(p.kind, r) do
					added[(y + v[2]) * 20 + x + v[1]] = true
				end
				local function filled(cx, cy)
					return at_board(cx, cy) ~= 0 or added[cy * 20 + cx]
				end
				local heights, holes, complete = {}, 0, 0
				for cx = p.lane, p.lane + LANE - 1 do
					local top = 0
					for cy = 1, H do
						if filled(cx, cy) and top == 0 then
							top = H - cy + 1
						elseif not filled(cx, cy) and top > 0 then
							holes += 1
						end
					end
					table.insert(heights, top)
				end
				for cy = 1, H do
					local full = true
					for cx = p.lane, p.lane + LANE - 1 do
						if not filled(cx, cy) then full = false end
					end
					if full then complete += 1 end
				end
				local score = holes * 18 - complete * 11
				for i, h in heights do
					score += h * 0.65 + h * h * 0.05
					if i > 1 then score += math.abs(h - heights[i - 1]) * 0.7 end
				end
				if score < cost then best, cost = { x = x, rot = r }, score end
			end
		end
	end
	return best
end

local function bot(p)
	if not p.ai then p.ai = plan(p) end
	if not p.ai then return { drop = true } end
	-- Release rotate/drop between presses; bots use exactly the human move rules.
	if p.rot ~= p.ai.rot then return { rotate = not p.prev.rotate } end
	if p.x ~= p.ai.x then return { move = if p.x < p.ai.x then 1 else -1 } end
	local own, other = 0, 0
	for i = 1, W * H do
		if state.board[i] ~= 0 then
			local x = (i - 1) % W + 1
			if x >= p.lane and x < p.lane + LANE then own += 1 else other += 1 end
		end
	end
	-- Let a slower human catch up instead of hard-dropping a tower on their behalf.
	return { drop = own - other <= KNOBS.bot_lead and p.age > KNOBS.bot_patience and not p.prev.drop }
end

-- A person's controls: stick or d-pad moves, A rotates, B (Shift) drops,
-- Y (F) holds, down falls faster.
local function controls(i)
	if not i then return {} end
	local x = i.move_x
	if i.held.left then x = -1 elseif i.held.right then x = 1 end
	return {
		move = if x < -0.35 then -1 elseif x > 0.35 then 1 else 0,
		rotate = i.held.south,
		drop = i.held.east or i.held.right_shoulder,
		hold = i.held.north,
		down = i.move_y < -0.35 or i.held.down,
	}
end

local function step(input, dt)
	state.time += dt
	state.flash = math.max(0, state.flash - dt)
	for _, p in state.players do
		p.age += dt
		local a = if p.bot then bot(p) else controls(input[p.id])
		local move = a.move or 0
		if move ~= 0 then
			p.repeat_time -= dt
			if move ~= p.prev.move or p.repeat_time <= 0 then
				if fits(p, p.x + move, p.y, p.rot) then p.x += move end
				p.repeat_time = if move ~= p.prev.move then KNOBS.shift_delay else KNOBS.shift_repeat
			end
		else
			p.repeat_time = 0
		end
		if a.hold and not p.prev.hold then hold(p) end
		if state.phase ~= "play" then break end
		if a.rotate and not p.prev.rotate then
			local r = (p.rot + 1) % 4
			for _, kick in { 0, -1, 1, -2, 2 } do
				if fits(p, p.x + kick, p.y, r) then
					p.x, p.rot = p.x + kick, r
					break
				end
			end
		end
		if a.drop and not p.prev.drop then
			p.y = drop_y(p, p.x, p.rot) or p.y
			p.slam = true
			lock(p)
		else
			p.fall += dt
			local interval = if a.down
				then KNOBS.soft_drop
				else math.max(KNOBS.fall_min, KNOBS.fall_start - state.rows * KNOBS.fall_step) * 100 / KNOBS.speed
			if p.fall >= interval then
				p.fall = 0
				if fits(p, p.x, p.y + 1, p.rot) then p.y += 1 else lock(p) end
			end
		end
		p.prev = { move = move, rotate = a.rotate or false, drop = a.drop or false, hold = a.hold or false }
		if state.phase ~= "play" then break end
	end
end

-- ---------------------------------------------------------------------------
-- The scene: the well stands upright facing the camera, 1 m cells.

local function cell_pos(x, y, z)
	return vector.create(x - (W + 1) / 2, H + 0.5 - y, z or 0)
end

local function hexrgb(c)
	if type(c) == "string" then
		return vector.create(tonumber(c:sub(2, 3), 16) / 255, tonumber(c:sub(4, 5), 16) / 255, tonumber(c:sub(6, 7), 16) / 255)
	end
	return c
end

local function player_color(p)
	local pl = players.get(p.id)
	if pl and pl.color then return pl.color end
	return SLOT_COLORS[(p.lane - 1) // LANE % 2 + 1]
end

-- Each side of the well: x of its middle.
local function side_x(p)
	return if p.lane == 1 then -13 else 13
end

local function block_look(kind, dim)
	local c = hexrgb(BLOCKS[kind])
	if dim then return { color = c * 0.35, emissive = KNOBS.block_glow * 0.2, roughness = 0.4 } end
	return { color = c, emissive = KNOBS.block_glow, roughness = 0.35, metal = 0.1 }
end

local function warm_player(c)
	world.warm("sphere", { color = c, emissive = 0.01, roughness = 0.45 })
	world.warm("cube", { color = c * 0.18, emissive = 0.002, roughness = 0.8 })
	world.warm("cube", { color = c * 0.3, emissive = 0.006, roughness = 0.8 })
	world.warm("cube", { color = c * 0.35, emissive = 0.003, roughness = 0.6 })
	for _, e in { 0.008, 0.02, 0.035 } do
		world.warm("wire", { color = c, emissive = e })
	end
end

local function warm()
	for k = 1, #BLOCKS do
		for _, dim in { false, true } do
			local l = block_look(k, dim)
			world.warm("cube", l)
		end
		world.warm("wire", { color = BLOCKS[k], emissive = KNOBS.ghost_glow })
		-- Bursts use the block's colour with the engine's default glow.
		world.warm("cube", { color = BLOCKS[k], emissive = 0.06 })
	end
	world.warm("cube", { color = DEAD, emissive = 0.006, roughness = 0.8 })
	world.warm("wire", { color = RIM, emissive = 0.02 })
	world.warm("cube", { color = MINT, emissive = 0.015 })
	world.warm("wire", { color = GOLD, emissive = 0.008 })
	world.warm("wire", { color = GOLD, emissive = 0.02 })
	world.warm("wire", { color = GOLD, emissive = 0.035 })
	world.warm("wire", { color = GOLD, emissive = 0.04 })
	world.warm("wire", { color = CORAL, emissive = 0.025 })
	for _, c in SLOT_COLORS do
		warm_player(c)
	end
	for _, pl in players.list() do
		warm_player(pl.color)
	end
end

-- A rectangle outline as a wire, w x h metres, in the XY plane.
local function frame_points(w, h)
	local a, b = w / 2, h / 2
	return { vector.create(-a, -b, 0), vector.create(a, -b, 0), vector.create(a, b, 0), vector.create(-a, b, 0) }, { 1, 2, 2, 3, 3, 4, 4, 1 }
end

-- The well, its frame, the two half tints, the row meters and the dots.
local function build_well()
	local cx, cy = 0, H / 2
	world.spawn("cube", { pos = vector.create(cx, cy, -0.75), size = vector.create(W + 0.6, H + 0.6, 0.3), color = WELL, emissive = 0.001, roughness = 0.5, tag = "well" })
	for _, s in { -1, 1 } do
		world.spawn("cube", { pos = vector.create(s * (W / 2 + 0.3), cy, -0.2), size = vector.create(0.3, H + 0.9, 1.2), color = "#1a1f33", roughness = 0.35, metal = 0.5, tag = "well" })
	end
	world.spawn("cube", { pos = vector.create(cx, -0.3, -0.2), size = vector.create(W + 0.9, 0.6, 1.2), color = "#1a1f33", roughness = 0.35, metal = 0.5, tag = "well" })
	-- A soft glowing rim around the well.
	local pts, edges = frame_points(W + 0.35, H + 0.35)
	world.spawn("wire", { points = pts, edges = edges, pos = vector.create(cx, cy, 0.45), width = 0.12, color = RIM, emissive = 0.02, solid = false, tag = "well" })
	-- A faint seam between the two halves.
	world.spawn("cube", { pos = vector.create(cx, cy, -0.58), size = vector.create(0.05, H, 0.02), color = "#c8d0ff", emissive = 0.004, solid = false, tag = "well" })
	-- A dot behind every cell.
	for y = 1, H do
		for x = 1, W do
			world.spawn("cube", { pos = cell_pos(x, y, -0.58), size = vector.create(0.08, 0.08, 0.02), color = "#c8d0ff", emissive = 0.001, solid = false, tag = "well" })
		end
	end
	-- The shared row meter on both outer edges: a dark rail, a mint fill and
	-- a notch per row.
	state.meters = {}
	for _, s in { -1, 1 } do
		local mx = s * (W / 2 + 1.1)
		world.spawn("cube", { pos = vector.create(mx, cy, -0.2), size = vector.create(0.45, H, 0.3), color = "#2a3150", emissive = 0.003, roughness = 0.5, tag = "well" })
		table.insert(state.meters, world.spawn("cube", { pos = vector.create(mx, 0, 0), size = vector.create(0.32, 0.01, 0.36), color = MINT, emissive = 0.015, solid = false, tag = "well" }))
		for i = 1, KNOBS.target - 1 do
			world.spawn("cube", { pos = vector.create(mx, H * i / KNOBS.target, 0.05), size = vector.create(0.5, 0.06, 0.4), color = WELL, solid = false, tag = "well" })
		end
	end
end

-- Slowly falling ghost blocks far behind the well.
local function build_backdrop()
	state.ghosts = {}
	for i = 1, 22 do
		local id = world.spawn("cube", { pos = vector.create(0, 0, -14), size = 2.2, color = hexrgb(BLOCKS[i % 7 + 1]) * 0.22, emissive = 0.0015, roughness = 0.8, solid = false, tag = "backdrop" })
		table.insert(state.ghosts, id)
	end
	world.spawn("cube", { pos = vector.create(0, 8, -20), size = vector.create(120, 70, 0.5), color = "#0d0f22", emissive = 0.0008, roughness = 0.9, tag = "backdrop" })
end

-- A player's corner: a big round face beside their half, the next piece
-- under it and the shared slot under that.
local function build_side(p)
	local c = player_color(p)
	local sx = side_x(p)
	local look = if p.lane == 1 then 1 else -1 -- towards the well
	local v = {}
	v.head = world.spawn("sphere", { pos = vector.create(sx, 13.2, 0), size = 3.4, color = c, emissive = 0.01, roughness = 0.45, tag = "side" })
	v.eyes, v.pupils = {}, {}
	for _, e in { -0.55, 0.55 } do
		table.insert(v.eyes, world.spawn("sphere", { pos = vector.create(sx + e + look * 0.25, 13.6, 1.45), size = 0.85, color = "#f4f4ec", roughness = 0.3, solid = false, tag = "side" }))
		table.insert(v.pupils, world.spawn("sphere", { pos = vector.create(sx + e + look * 0.4, 13.6, 1.8), size = 0.38, color = "#141418", roughness = 0.2, solid = false, tag = "side" }))
	end
	v.mouth = world.spawn("wire", { points = { vector.create(-0.45, 0.05, 0), vector.create(0, -0.2, 0), vector.create(0.45, 0.05, 0) }, edges = { 1, 2, 2, 3 }, pos = vector.create(sx + look * 0.25, 12.5, 1.55), width = 0.09, color = "#141418", solid = false, tag = "side" })
	-- The next piece, on a panel tinted in the player's colour.
	world.spawn("cube", { pos = vector.create(sx, 8.4, -0.4), size = vector.create(4.6, 3.2, 0.2), color = c * 0.18, emissive = 0.002, roughness = 0.8, tag = "side" })
	v.next = {}
	for _ = 1, 4 do
		table.insert(v.next, world.spawn("cube", { pos = vector.create(sx, 8.4, 0), size = 0.78, color = BLOCKS[1], emissive = KNOBS.block_glow, roughness = 0.35, metal = 0.1, solid = false, tag = "side" }))
	end
	-- The shared slot, on both sides: rimmed in the colour of whoever put
	-- the piece in, with a small Y button.
	world.spawn("cube", { pos = vector.create(sx, 3.9, -0.4), size = vector.create(4, 3.2, 0.2), color = c * 0.18, emissive = 0.002, roughness = 0.8, tag = "side" })
	local pts, edges = frame_points(4, 3.2)
	v.rim = world.spawn("wire", { points = pts, edges = edges, pos = vector.create(sx, 3.9, -0.2), width = 0.1, color = GOLD, emissive = 0.035, solid = false, tag = "side" })
	v.hold = {}
	for _ = 1, 4 do
		table.insert(v.hold, world.spawn("cube", { pos = vector.create(sx, 3.9, 0), size = 0.7, color = BLOCKS[1], emissive = KNOBS.block_glow, roughness = 0.35, metal = 0.1, solid = false, visible = false, tag = "side" }))
	end
	local bx, by = sx + 2, 5.5
	v.badge = world.spawn("sphere", { pos = vector.create(bx, by, 0.1), size = vector.create(0.95, 0.95, 0.35), color = GOLD, emissive = 0.012, roughness = 0.4, solid = false, tag = "side" })
	v.y = world.spawn("wire", { points = { vector.create(-0.17, 0.2, 0), vector.create(0, 0, 0), vector.create(0.17, 0.2, 0), vector.create(0, -0.22, 0) }, edges = { 1, 2, 3, 2, 2, 4 }, pos = vector.create(bx, by, 0.3), width = 0.08, color = "#1f170a", solid = false, tag = "side" })
	-- The half of the well this player owns, tinted in their colour.
	v.tint = world.spawn("cube", { pos = vector.create((p.lane - 1 + LANE / 2) - W / 2, H / 2, -0.59), size = vector.create(LANE, H, 0.01), color = c * 0.3, emissive = 0.006, roughness = 0.8, solid = false, tag = "side" })
	-- The falling piece and where it will land.
	v.piece = {}
	for _ = 1, 4 do
		table.insert(v.piece, world.spawn("cube", { pos = vector.create(0, -10, 0), size = 0.92, color = BLOCKS[1], emissive = KNOBS.block_glow, roughness = 0.35, metal = 0.1, solid = false, tag = "side" }))
	end
	v.ghost = world.spawn("wire", { points = { vector.create(0, 0, 0), vector.create(0.001, 0, 0) }, edges = { 1, 2 }, width = 0.07, color = BLOCKS[1], emissive = KNOBS.ghost_glow, solid = false, tag = "side" })
	v.color = c
	v.kind, v.next_kind, v.hold_kind, v.dim = 0, 0, -1, nil
	return v
end

local function build_board()
	state.cells = {}
	state.shown = {}
	for y = 1, H do
		for x = 1, W do
			table.insert(state.cells, world.spawn("cube", { pos = cell_pos(x, y), size = 0.92, color = BLOCKS[1], emissive = KNOBS.block_glow, roughness = 0.35, metal = 0.1, solid = false, visible = false, tag = "board" }))
			table.insert(state.shown, 0)
		end
	end
end

-- The end of a round: a gold star for a win, a coral cross for a loss.
local function build_ends()
	local star, star_edges = {}, {}
	for i = 0, 9 do
		local a = -math.pi / 2 + i * math.pi / 5
		local r = if i % 2 == 0 then 3.6 else 1.6
		table.insert(star, vector.create(math.cos(a) * r, -math.sin(a) * r, 0))
		table.insert(star_edges, i + 1)
		table.insert(star_edges, (i + 1) % 10 + 1)
	end
	state.star = world.spawn("wire", { points = star, edges = star_edges, pos = vector.create(0, H / 2, 2), width = 0.3, color = GOLD, emissive = 0.04, solid = false, visible = false, tag = "ends" })
	state.cross = world.spawn("wire", { points = { vector.create(-4, 2.4, 0), vector.create(4, -2.4, 0), vector.create(4, 2.4, 0), vector.create(-4, -2.4, 0) }, edges = { 1, 2, 3, 4 }, pos = vector.create(0, H / 2, 2), width = 0.38, color = CORAL, emissive = 0.025, solid = false, visible = false, tag = "ends" })
end

local function build_sides()
	for _, id in world.find("side") do
		world.remove(id)
	end
	state.vis = {}
	for _, p in state.players do
		state.vis[tostring(p.id)] = build_side(p)
	end
end

local function look()
	light.set({ sun_lux = 0, sky_lux = 0.6, fog_density = 0 })
	post.set({ bloom = KNOBS.bloom, bloom_radius = 3, bloom_threshold = KNOBS.bloom_threshold, saturation = KNOBS.saturation, contrast = 1.06, vignette = KNOBS.vignette, grain = 0.12 })
end

local function build_scene()
	for _, tag in { "well", "backdrop", "board", "ends", "side" } do
		for _, id in world.find(tag) do
			world.remove(id)
		end
	end
	build_well()
	build_backdrop()
	build_board()
	build_ends()
	build_sides()
	look()
	camera.set(1, { pos = vector.create(0, H / 2 + 0.2, 23.5), target = vector.create(0, H / 2 + 0.2, 0), fov = 50 })
end

-- ---------------------------------------------------------------------------
-- Showing the state

local function show_cubes(ids, kind, rot, cx, cy, size, dim, visible)
	local cs = cells(kind, rot)
	local mw, mh = 0, 0
	for _, v in cs do
		mw = math.max(mw, v[1] + 1)
		mh = math.max(mh, v[2] + 1)
	end
	for i, id in ids do
		local v = cs[i]
		local l = block_look(kind, dim)
		world.set(id, { pos = vector.create(cx + (v[1] + 0.5 - mw / 2) * size, cy - (v[2] + 0.5 - mh / 2) * size, 0), color = l.color, emissive = l.emissive, roughness = l.roughness, metal = l.metal or 0, visible = visible })
	end
end

local function show_piece(p, v, dt)
	local cs = cells(p.kind, p.rot)
	-- Drawn pieces glide to their cell (a new piece and a hard drop jump).
	if p.snap or v.dx == nil then
		v.dx, v.dy = p.x, p.y
		p.snap = false
	else
		local k = 1 - math.exp(-KNOBS.glide * dt)
		v.dx += (p.x - v.dx) * k
		v.dy += (p.y - v.dy) * k
	end
	local changed = v.kind ~= p.kind
	v.kind = p.kind
	local l = block_look(p.kind, false)
	for i, id in v.piece do
		local c = cs[i]
		local y = v.dy + c[2]
		local opts: any = { pos = cell_pos(v.dx + c[1], y, 0.05), visible = state.phase == "play" and y >= 0.5 }
		if changed then
			opts.color, opts.emissive = l.color, l.emissive
		end
		world.set(id, opts)
	end
	-- Where it lands: an outline of every cell.
	local gy = state.phase == "play" and drop_y(p, p.x, p.rot)
	if gy then
		local pts, edges = {}, {}
		for _, c in cs do
			local o = cell_pos(p.x + c[1], gy + c[2], 0.02)
			local n = #pts
			for _, d in { { -1, -1 }, { 1, -1 }, { 1, 1 }, { -1, 1 } } do
				table.insert(pts, o + vector.create(d[1] * 0.4, d[2] * 0.4, 0))
			end
			for e = 1, 4 do
				table.insert(edges, n + e)
				table.insert(edges, n + e % 4 + 1)
			end
		end
		world.set(v.ghost, { points = pts, edges = edges, color = BLOCKS[p.kind], emissive = KNOBS.ghost_glow, visible = true })
	else
		world.set(v.ghost, { visible = false })
	end
	-- Eyes follow the piece.
	local sx = side_x(p)
	local px = cell_pos(v.dx + 1, v.dy).x
	local look = math.clamp((px - sx) / 10, -1, 1) * 0.22
	local lookup = math.clamp((cell_pos(0, v.dy).y - 13.6) / 12, -1, 1) * 0.2
	for i, id in v.pupils do
		local e = if i == 1 then -0.55 else 0.55
		local base = if p.lane == 1 then 0.25 else -0.25
		world.place(id, vector.create(sx + e + base + look, 13.6 + lookup, 1.8))
	end
end

local function owner_color()
	for _, p in state.players do
		if p.id == state.hold_by then return state.vis[tostring(p.id)].color end
	end
	return hexrgb(GOLD)
end

local function show_side(p, v)
	if v.next_kind ~= p.next then
		v.next_kind = p.next
		show_cubes(v.next, p.next, 0, side_x(p), 8.4, 0.78, false, true)
	end
	local dim = p.held or state.phase ~= "play"
	if v.hold_kind ~= state.hold or v.dim ~= dim or v.hold_by ~= state.hold_by then
		v.hold_kind, v.dim, v.hold_by = state.hold, dim, state.hold_by
		if state.hold ~= 0 then
			show_cubes(v.hold, state.hold, 0, side_x(p), 3.9, 0.7, dim, true)
		else
			for _, id in v.hold do
				world.set(id, { visible = false })
			end
		end
		-- Dimmed for a player who already swapped this piece: Y only works
		-- again after it lands.
		local oc = owner_color()
		world.set(v.rim, { color = oc, emissive = if dim then 0.008 else 0.02 })
		world.set(v.badge, { color = if dim then hexrgb(GOLD) * 0.35 else GOLD })
	end
end

local function show_board()
	local lost = state.phase == "lost"
	for i = 1, W * H do
		local k = state.board[i]
		local want = if k == 0 then 0 elseif lost then -k else k
		if state.shown[i] ~= want then
			state.shown[i] = want
			if want == 0 then
				world.set(state.cells[i], { visible = false })
			elseif want < 0 then
				world.set(state.cells[i], { visible = true, color = DEAD, emissive = 0.006, roughness = 0.8, metal = 0 })
			else
				local l = block_look(want, false)
				world.set(state.cells[i], { visible = true, color = l.color, emissive = l.emissive, roughness = l.roughness, metal = l.metal })
			end
		end
	end
	local done = math.min(1, state.rows / math.max(1, KNOBS.target))
	for _, id in state.meters do
		local x = world.get(id).pos.x
		world.set(id, { pos = vector.create(x, H * done / 2, 0), size = vector.create(0.32, math.max(0.01, H * done), 0.36) })
	end
end

local function show_backdrop()
	local t = state.time
	for i, id in state.ghosts do
		local x = ((i * 263) % 68) - 34
		local y = 30 - ((i * 151 + t * (0.5 + (i % 4) * 0.3)) % 46)
		local spin = i + t * 0.05 * (if i % 2 == 0 then 1 else -1)
		world.set(id, { pos = vector.create(x, y, -12 - (i % 3) * 2), rot = vector.create(spin * 0.3, spin * 0.5, spin) })
	end
end

local function show_ends(dt)
	world.set(state.star, { visible = state.phase == "won" })
	world.set(state.cross, { visible = state.phase == "lost" })
	if state.phase == "won" then
		state.end_clock += dt
		local s = 1 + 0.06 * math.sin(state.end_clock * 4)
		world.set(state.star, { size = s, rot = vector.create(0, 0, math.sin(state.end_clock * 1.5) * 0.15) })
		-- Confetti of blocks out of the star.
		state.confetti -= dt
		if state.confetti <= 0 then
			state.confetti = 0.18
			local k = random.int(1, #BLOCKS)
			fx.burst(vector.create(random.range(-5, 5), random.range(4, 14), 1.5), { color = BLOCKS[k], count = 14, speed = 7, life = 1.1, size = 0.35, gravity = 6 })
		end
	elseif state.phase == "lost" then
		state.end_clock += dt
	end
end

local function lamps()
	local list = {}
	for _, p in state.players do
		local v = state.vis[tostring(p.id)]
		if v and v.dx and state.phase == "play" then
			table.insert(list, { pos = cell_pos(v.dx + 1, v.dy + 1, 2.2), color = BLOCKS[p.kind], intensity = KNOBS.lamp, range = 6 })
		end
		table.insert(list, { pos = vector.create(side_x(p), 13, 5), color = player_color(p), intensity = 180, range = 9 })
	end
	if state.flash > 0 then
		table.insert(list, { pos = vector.create(0, H / 2, 6), color = MINT, intensity = 2600 * state.flash, range = 18 })
	end
	if state.phase == "won" then
		table.insert(list, { pos = vector.create(0, H / 2, 5), color = GOLD, intensity = 900, range = 16 })
	end
	lights.set(list)
end

local SOUNDS = { lock = "lock", clear = "clear", hold = "hold", won = "won", lost = "lost" }

local function play_events()
	for _, e in state.events do
		if SOUNDS[e] then sound.play(SOUNDS[e]) end
	end
	state.events = {}
	for _, b in state.bursts do
		local pos = cell_pos(b[1] + 0.5, b[2], 0.5)
		fx.burst(pos, { color = BLOCKS[math.max(1, b[3])], count = 10, speed = 6, life = 0.6, size = 0.3, gravity = 4 })
	end
	if #state.bursts > 0 then fx.ring(cell_pos((W + 1) / 2, state.bursts[1][2], 0.6), { color = MINT, radius = 7, life = 0.45, width = 0.12 }) end
	state.bursts = {}
end

-- ---------------------------------------------------------------------------
-- Rounds and players

-- The two players in couch order (left sits left), people before bots.
local function seats()
	local list = {}
	for i, pl in players.list() do
		if i > MAX_PLAYERS then break end
		table.insert(list, pl)
	end
	table.sort(list, function(a, b)
		return (a.couch or a.id + 100) < (b.couch or b.id + 100)
	end)
	return list
end

local function new_round()
	state.board = table.create(W * H, 0)
	state.players = {}
	state.rows = 0
	state.phase = "play"
	state.time = 0
	state.flash = 0
	state.hold = 0
	state.hold_by = 0
	state.events = {}
	state.bursts = {}
	state.finished = false
	state.end_clock = 0
	state.confetti = 0
	for i, pl in seats() do
		table.insert(state.players, {
			id = pl.id,
			bot = pl.bot,
			lane = (i - 1) * LANE + 1,
			bag = {},
			prev = {},
			age = 0,
		})
	end
	for _, p in state.players do
		spawn_piece(p)
	end
	build_scene()
end

local function init()
	warm()
	new_round()
end

local function reload()
	-- Knobs that feed the scene (meter notches, glow, post) show at once.
	build_scene()
end

local function tick(dt, input)
	if state.phase == "play" then
		step(input, dt)
		if state.phase ~= "play" and not state.finished then
			state.finished = true
			game.log(if state.phase == "won" then "Stacked " .. state.rows .. " rows together" else "The stack reached the top at " .. state.rows .. " rows")
			game.finish()
			for _, p in state.players do
				players.rumble(p.id, if state.phase == "won" then 0.4 else 0.8, 0.4)
			end
		end
	else
		state.time += dt
		-- After a round, any person's A starts the next one.
		if state.end_clock > 1 then
			for _, p in state.players do
				local i = not p.bot and input[p.id]
				if i and i.pressed.south then
					new_round()
					return
				end
			end
		end
	end
	for _, p in state.players do
		if p.slam then
			p.slam = nil
			players.rumble(p.id, 0.25, 0.08)
		end
	end
	play_events()
	show_board()
	for _, p in state.players do
		local v = state.vis[tostring(p.id)]
		if v then
			show_piece(p, v, dt)
			show_side(p, v)
		end
	end
	show_backdrop()
	show_ends(dt)
	lamps()
end

-- Someone joins mid-round: a person takes over a bot's half.
local function player_joined(pl)
	warm_player(pl.color)
	if pl.bot then return end
	for _, p in state.players do
		if p.bot then
			p.id = pl.id
			p.bot = false
			build_sides()
			return
		end
	end
	if #state.players < MAX_PLAYERS then
		new_round()
	end
end

-- Someone leaves: a bot plays their half on, so the round goes on.
local function player_left(pl)
	for _, p in state.players do
		if p.id == pl.id then p.bot = true end
	end
end

local function players_changed()
	for _, pl in players.list() do
		warm_player(pl.color)
	end
	build_sides()
end

return {
	init = init,
	reload = reload,
	tick = tick,
	player_joined = player_joined,
	player_left = player_left,
	players_changed = players_changed,
}
