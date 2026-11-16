-- The land the survivors walk through, generated a chunk at a time as
-- the group heads east (+x). A path runs along the middle of a strip
-- 28 m deep; each day crosses one biome (suburb, forest, ruined city,
-- farmland) and ends at a camp fire. Buildings stand beside the path,
-- each with its own rooms joined by doors, cupboards to search and
-- sleepers inside. Going in is a choice: the best loot and the most
-- danger are indoors.
--
-- chunk(s, index) spawns one 24 m chunk and returns what the rules need:
-- its entities, solid boxes (rects), containers, buildings (rooms and
-- doors for walkers), weapons lying around and places for zombies.

local v = vector.create

local M = {}

M.CHUNK = 24 -- metres of land per chunk
M.HALF_Z = 18 -- the strip runs 18 m either side of the path
M.TURN = 7 -- metres at the west end of a chunk for a street that turns north or south

-- Each chunk's path runs at its own z (its centre): when it differs from
-- the chunk before, a street at the west end climbs or drops to it. The
-- helpers below place everything at z + ZC; chunk() sets ZC.
local ZC = 0
M.DAY_CHUNKS = 7 -- chunks of walking per day, then the camp
local WALL_T = 0.25

M.BIOMES = { "suburb", "forest", "city", "farm" }

local LOOKS = {
	-- height: wall tops vary between these (m); ruin: share of wall pieces
	-- fallen to a stub with rubble around it.
	suburb = { ground = "#55623f", path = "#2c2c2e", edge = "#8c8984", walls = { "#a8998a", "#9aa294", "#b0a08a", "#8d97a0", "#a38c78" }, floors = { "#6a4e36", "#7a5e42", "#827868" }, height = { 2.5, 3 }, ruin = 0.12 },
	forest = { ground = "#3f4a2e", path = "#6b5a42", edge = "#4d4433", walls = { "#5f4c3a", "#6a5640" }, floors = { "#4e3e2e" }, height = { 2.3, 2.7 }, ruin = 0.15 },
	-- The city in colours apart: dark asphalt, pale pavement, dusty lots,
	-- brick, ochre and cream plaster, soot.
	city = { ground = "#4a4236", path = "#1f1f22", edge = "#8a857c", walls = { "#7a4a3a", "#9a845e", "#a89c86", "#5a524a", "#6e7a72", "#8a5a44" }, floors = { "#6a5440", "#5a5446", "#7a6a52" }, height = { 3.2, 4.6 }, ruin = 0.32 },
	farm = { ground = "#6b6a3c", path = "#7b6646", edge = "#5d5032", walls = { "#7a3a2c", "#8a4a38", "#a8998a" }, floors = { "#5e4e36", "#6a5c40" }, height = { 2.6, 3.4 }, ruin = 0.15 },
}
local CAR_COLORS = { "#6b2b2b", "#2f4d6b", "#5d6650", "#8a8478", "#3a3a40", "#7a5a2a" }
local BURNT = { color = "#2a2624", roughness = 0.9 }
local RUST = { "#6a4030", "#5a4a3a", "#4a5048", "#6a5a48" } -- wrecks left to rot
local ROOFS = { "#3a3632", "#4a4640", "#433c36", "#2e2c2a" }
-- Set dressing (not solid), by name: every look is warmed in M.warm.
local PROP = {
	plank = "#6b4e37", board = "#8b6b4a", dark = "#161618", sandbag = "#8a7a5a", bag = "#1d1d1f",
	dumpster = "#2f4a3a", barrel = "#6a3a2a", hedge = "#2f4a26", fence = "#8a8478", tyre = "#1a1a1a",
}
local DEBRIS = { "#5f5a55", "#7a7470", "#4a4640", "#3a3632" }
local CROP = { "#7d8a3a", "#8f8a40", "#6a7a32" }

function M.biome(day) return M.BIOMES[(day - 1) % #M.BIOMES + 1] end

-- Every look the land uses, warmed in init.
function M.warm()
	for _, l in LOOKS do
		for _, c in { l.ground, l.path, l.edge } do world.warm("cube", { color = c, roughness = 0.95 }) end
		world.warm("cube", { color = l.ground, roughness = 1 })
		for _, c in l.walls do world.warm("cube", { color = c, roughness = 0.95 }) end
		for _, c in l.floors do world.warm("cube", { color = c, roughness = 0.95 }) end
	end
	for _, c in CAR_COLORS do world.warm("cube", { color = c, roughness = 0.4, metal = 0.3 }) end
	world.warm("cube", BURNT)
	for _, c in RUST do world.warm("cube", { color = c, roughness = 0.9 }) end
	for _, c in ROOFS do world.warm("cube", { color = c, roughness = 0.95 }) end
	for _, c in PROP do
		world.warm("cube", { color = c, roughness = 0.9 })
		world.warm("cube", { color = c, roughness = 0.95 })
		world.warm("sphere", { color = c, roughness = 0.9 })
	end

	for _, c in CROP do world.warm("cube", { color = c, roughness = 0.95 }) end
	for _, c in { "#2a2624", "#d8d0a0", "#4a3a2a", "#5a4632", "#202024", "#e8e8f0", "#1d3a8a", "#8b6b4a", "#6b4e37", "#7a7470", "#5f5a55", "#c9b25a", "#3a3632" } do
		world.warm("cube", { color = c, roughness = 0.9 })
		world.warm("cube", { color = c, roughness = 0.8 })
		world.warm("cube", { color = c, roughness = 0.95 })
	end
	for _, c in { "#3c5a2c", "#2f4a26", "#26401f", "#4a6a32" } do world.warm("sphere", { color = c, roughness = 0.9 }) end
	world.warm("sphere", { color = "#7a7470", roughness = 0.9 })
	world.warm("sphere", { color = "#ff9a3a", emissive = 1.2, roughness = 0.9 })
	world.warm("cube", { color = "#3a3d42", roughness = 0.6, metal = 0.6 })
	world.warm("cube", { color = "#fff0c8", roughness = 0.9, emissive = 1.5 })
	world.warm("cube", { color = "#4a4640", roughness = 0.9 })
end

----------------------------------------------------------------------------
-- Building blocks

local function add(c, id)
	table.insert(c.ents, id)
	return id
end

local function box(c, x0, z0, x1, z1, h, look, tag)
	if x1 < x0 then x0, x1 = x1, x0 end
	if z1 < z0 then z0, z1 = z1, z0 end
	z0 += ZC
	z1 += ZC
	local id = add(c, world.spawn("cube", {
		pos = v((x0 + x1) / 2, h / 2, (z0 + z1) / 2),
		size = v(x1 - x0, h, z1 - z0),
		color = look.color,
		roughness = look.roughness or 0.95,
		metal = look.metal,
		tag = tag or "wall",
	}))
	local r = { x0 = x0, z0 = z0, x1 = x1, z1 = z1, h = h, id = id }
	table.insert(c.rects, r)
	return r
end

local function flat(c, x0, z0, x1, z1, y, color)
	return add(c, world.spawn("cube", {
		pos = v((x0 + x1) / 2, y, (z0 + z1) / 2 + ZC),
		size = v(math.abs(x1 - x0), 0.02, math.abs(z1 - z0)),
		color = color,
		roughness = 0.95,
		solid = false,
		tag = "ground",
	}))
end

local function deco(c, shape, pos, size, color, extra)
	local o = { pos = pos + v(0, 0, ZC), size = size, color = color, roughness = 0.9, solid = false, tag = "deco" }
	if extra then
		for k, x in extra do o[k] = x end
	end
	return add(c, world.spawn(shape, o))
end

local function container(c, kind, x, z, w, d, h, color)
	local r = box(c, x - w / 2, z - d / 2, x + w / 2, z + d / 2, h, { color = color, roughness = 0.8 })
	table.insert(c.containers, { kind = kind, x = x, z = z, rect = r, id = r.id, searched = false })
end

local function tree(c, x, z, dark)
	box(c, x - 0.2, z - 0.2, x + 0.2, z + 0.2, 3.4, { color = "#5a4632" })
	local col = if dark then (if random.chance(0.5) then "#26401f" else "#2f4a26") else (if random.chance(0.5) then "#3c5a2c" else "#4a6a32")
	deco(c, "sphere", v(x, 3.6 + random.range(0, 0.8), z), random.range(1.8, 2.8), col)
end

local function rock(c, x, z)
	local s = random.range(0.6, 1.4)
	box(c, x - s / 2, z - s / 2, x + s / 2, z + s / 2, s * 0.6, { color = "#7a7470" })
end

local function car(c, x, z, police, yaw_x, burnt)
	local color = if police then "#e8e8f0" else CAR_COLORS[random.int(1, #CAR_COLORS)]
	if burnt then
		-- A burnt-out wreck: black shell, no glass, nothing left inside.
		if yaw_x then box(c, x - 2.1, z - 0.92, x + 2.1, z + 0.92, 1.0, BURNT) else box(c, x - 0.92, z - 2.1, x + 0.92, z + 2.1, 1.0, BURNT) end
		deco(c, "cube", v(x, 1.15, z), if yaw_x then v(1.8, 0.3, 1.5) else v(1.5, 0.3, 1.8), "#3a3632")
		return
	end
	local r
	if yaw_x then
		r = box(c, x - 2.1, z - 0.92, x + 2.1, z + 0.92, 1.15, { color = color, roughness = 0.4, metal = 0.3 })
		deco(c, "cube", v(x + 0.2, 1.45, z), v(2.1, 0.6, 1.6), "#202024")
	else
		r = box(c, x - 0.92, z - 2.1, x + 0.92, z + 2.1, 1.15, { color = color, roughness = 0.4, metal = 0.3 })
		deco(c, "cube", v(x, 1.45, z + 0.2), v(1.6, 0.6, 2.1), "#202024")
	end
	if police then deco(c, "cube", v(x, 1.8, z), v(0.6, 0.12, 0.6), "#1d3a8a") end
	table.insert(c.containers, { kind = if police then "police" else "car", x = x, z = z, rect = r, id = r.id, searched = false })
end

-- Rubble: a few tumbled blocks of a wall's colour around a point.
local function rubble(c, x, z, spread, look, n)
	for _ = 1, n do
		local s = random.range(0.15, 0.45)
		deco(c, "cube", v(x + random.range(-spread, spread), s * 0.35, z + random.range(-spread, spread)), v(s * random.range(1, 2.2), s * 0.7, s), look.color,
			{ roughness = 0.95, rot = v(random.range(-0.4, 0.4), random.range(-math.pi, math.pi), random.range(-0.4, 0.4)) })
	end
end

-- A run of wall from a0 to a1 along x (at z = at) or along z (at x = at),
-- in pieces of different heights: an abandoned building with no roof and
-- broken tops. Some pieces have fallen to a stub, rubble at their feet.
local function wall_run(c, a0, a1, at, along_x, look, wall)
	local a = a0
	while a1 - a > 0.05 do
		local len = math.min(a1 - a, random.range(0.9, 2.2))
		if a1 - (a + len) < 0.5 then len = a1 - a end
		local fallen = random.chance(look.ruin)
		local h = if fallen then random.range(0.45, 1.1) else random.range(look.height[1], look.height[2])
		if along_x then
			box(c, a, at - WALL_T / 2, a + len, at + WALL_T / 2, h, wall)
		else
			box(c, at - WALL_T / 2, a, at + WALL_T / 2, a + len, h, wall)
		end
		if fallen then
			local m = a + len / 2
			local side = if random.chance(0.5) then 1 else -1
			if along_x then rubble(c, m, at + side * 0.5, len / 2, wall, 4) else rubble(c, at + side * 0.5, m, len / 2, wall, 4) end
		end
		a += len
	end
end

-- A building: a w x d footprint split into cols x rows rooms. Every room
-- is reachable: a random spanning tree of inner doors, plus a few extra.
-- The front door faces the path (side -1: north of it, so the door is on
-- the +z wall). kinds: the loot table of its cupboards.
local function building(c, x0, w, d, cols, rows, side, look, kinds, sleepers)
	local zn = side * 4.2 -- the wall facing the path
	local zf = side * (4.2 + d)
	local z0, z1 = math.min(zn, zf), math.max(zn, zf)
	local cw, rd = w / cols, d / rows
	local floor = look.floors[random.int(1, #look.floors)]
	local wall = { color = look.walls[random.int(1, #look.walls)] }
	flat(c, x0, z0, x0 + w, z1, 0.012, floor)
	local b = { x0 = x0, x1 = x0 + w, z0 = z0, z1 = z1, rooms = {}, doors = {}, cols = cols, rows = rows }
	-- Rooms by column i (west to east) and row j (from the path inwards).
	local function rid(i, j) return (j - 1) * cols + i end
	for j = 1, rows do
		for i = 1, cols do
			local za = zn + side * (j - 1) * rd
			local zb = zn + side * j * rd
			b.rooms[rid(i, j)] = { x0 = x0 + (i - 1) * cw, x1 = x0 + i * cw, z0 = math.min(za, zb), z1 = math.max(za, zb) }
		end
	end
	-- Which inner walls get a door: a random spanning tree over the grid.
	local open = {}
	local seen = { [1] = true }
	local frontier = { 1 }
	while #frontier > 0 do
		local k = random.int(1, #frontier)
		local r = frontier[k]
		local i, j = (r - 1) % cols + 1, (r - 1) // cols + 1
		local next = {}
		for _, o in { { i - 1, j }, { i + 1, j }, { i, j - 1 }, { i, j + 1 } } do
			if o[1] >= 1 and o[1] <= cols and o[2] >= 1 and o[2] <= rows and not seen[rid(o[1], o[2])] then
				table.insert(next, rid(o[1], o[2]))
			end
		end
		if #next == 0 then
			table.remove(frontier, k)
		else
			local n = next[random.int(1, #next)]
			seen[n] = true
			open[math.min(r, n) * 1000 + math.max(r, n)] = true
			table.insert(frontier, n)
		end
	end
	local function gap_at(a0, a1)
		local g = random.range(a0 + 0.6, a1 - 1.9)
		return { g, g + 1.3 }
	end
	-- Walls along x (constant z) and along z (constant x), with gaps.
	local function wall_x(xa, xb, z, gap)
		if gap then
			if gap[1] - xa > 0.05 then wall_run(c, xa, gap[1], z, true, look, wall) end
			if xb - gap[2] > 0.05 then wall_run(c, gap[2], xb, z, true, look, wall) end
		else
			wall_run(c, xa, xb, z, true, look, wall)
		end
	end
	local function wall_z(za, zb, x, gap)
		if za > zb then za, zb = zb, za end
		if gap then
			if gap[1] - za > 0.05 then wall_run(c, za, gap[1], x, false, look, wall) end
			if zb - gap[2] > 0.05 then wall_run(c, gap[2], zb, x, false, look, wall) end
		else
			wall_run(c, za, zb, x, false, look, wall)
		end
	end
	-- The front wall: one door in a random front room.
	local front = random.int(1, cols)
	for i = 1, cols do
		local r = b.rooms[rid(i, 1)]
		local gap = if i == front then gap_at(r.x0, r.x1) else nil
		wall_x(r.x0, r.x1, zn, gap)
		if gap then
			local mx = (gap[1] + gap[2]) / 2
			table.insert(b.doors, { a = 0, b = rid(i, 1), pa = v(mx, 0, zn - side * 0.8), pb = v(mx, 0, zn + side * 0.8) })
		end
	end
	-- The back wall: sometimes a back door too.
	local back = if random.chance(0.35) then random.int(1, cols) else nil
	for i = 1, cols do
		local r = b.rooms[rid(i, rows)]
		local gap = if i == back then gap_at(r.x0, r.x1) else nil
		wall_x(r.x0, r.x1, zf, gap)
		if gap then
			local mx = (gap[1] + gap[2]) / 2
			table.insert(b.doors, { a = 0, b = rid(i, rows), pa = v(mx, 0, zf + side * 0.8), pb = v(mx, 0, zf - side * 0.8) })
		end
	end
	wall_z(zn, zf, x0, nil)
	wall_z(zn, zf, x0 + w, nil)
	-- Inner walls between neighbouring rooms.
	for j = 1, rows do
		for i = 1, cols - 1 do
			local a, n = rid(i, j), rid(i + 1, j)
			local r = b.rooms[a]
			local gap = if open[a * 1000 + n] or random.chance(0.15) then gap_at(r.z0, r.z1) else nil
			wall_z(r.z0, r.z1, r.x1, gap)
			if gap then
				local mz = (gap[1] + gap[2]) / 2
				table.insert(b.doors, { a = a, b = n, pa = v(r.x1 - 0.7, 0, mz), pb = v(r.x1 + 0.7, 0, mz) })
			end
		end
	end
	for j = 1, rows - 1 do
		for i = 1, cols do
			local a, n = rid(i, j), rid(i, j + 1)
			local r = b.rooms[a]
			local zw = zn + side * j * rd
			local gap = if open[a * 1000 + n] or random.chance(0.15) then gap_at(r.x0, r.x1) else nil
			wall_x(r.x0, r.x1, zw, gap)
			if gap then
				local mx = (gap[1] + gap[2]) / 2
				table.insert(b.doors, { a = a, b = n, pa = v(mx, 0, zw - side * 0.7), pb = v(mx, 0, zw + side * 0.7) })
			end
		end
	end
	-- A cupboard in a corner of most rooms, sleepers in some; rubble and
	-- junk on the floors.
	for _, r in b.rooms do
		if random.chance(0.6) then
			rubble(c, random.range(r.x0 + 0.6, r.x1 - 0.6), random.range(r.z0 + 0.6, r.z1 - 0.6), 0.6, wall, random.int(2, 5))
		end
		for _ = 1, random.int(0, 2) do
			local x, z = random.range(r.x0 + 0.3, r.x1 - 0.6), random.range(r.z0 + 0.3, r.z1 - 0.6)
			flat(c, x, z, x + random.range(0.2, 0.5), z + random.range(0.15, 0.4), 0.016, "#d8d0a0")
		end
		if random.chance(0.75) then
			local east = random.chance(0.5)
			local deep = random.chance(0.5)
			local x = if east then r.x1 - 0.55 else r.x0 + 0.55
			local z = if deep then r.z1 - 0.45 else r.z0 + 0.45
			local kind = kinds[random.int(1, #kinds)]
			container(c, kind, x, z, 0.75, 0.6, if kind == "closet" then 1.3 else 0.95, if kind == "closet" then "#5a4632" else "#8b6b4a")
			c.containers[#c.containers].stand = v((r.x0 + r.x1) / 2, 0, (r.z0 + r.z1) / 2)
		end
		for _ = 1, random.int(0, sleepers) do
			table.insert(c.inside, v(random.range(r.x0 + 0.7, r.x1 - 0.7), 0, random.range(r.z0 + 0.7, r.z1 - 0.7)))
		end
	end
	table.insert(c.buildings, b)
	return b
end

-- A building you cannot enter: a solid block with a roof, its doors
-- boarded up and its windows dark or nailed shut. main.luau hides it (and
-- shows a low cut-away in its place) when it would hide a survivor from
-- the camera.
local function closed(c, x0, w, d, side, look, h)
	local zn = side * 4.6
	local zf = side * (4.6 + d)
	local z0, z1 = math.min(zn, zf), math.max(zn, zf)
	local x1 = x0 + w
	local color = look.walls[random.int(1, #look.walls)]
	local blk = { x0 = x0, x1 = x1, z0 = z0, z1 = z1, h = h, parts = {}, rooms = {}, doors = {} }
	local function part(id) table.insert(blk.parts, id) return id end
	local r = box(c, x0, z0, x1, z1, h, { color = color })
	part(r.id)
	-- The roof: a darker slab, a parapet edge and things on top.
	local roof = ROOFS[random.int(1, #ROOFS)]
	part(deco(c, "cube", v((x0 + x1) / 2, h + 0.03, (z0 + z1) / 2), v(w - 0.5, 0.06, d - 0.5), roof, { roughness = 0.95 }))
	for _ = 1, random.int(1, 3) do
		local s = random.range(0.6, 1.4)
		part(deco(c, "cube", v(random.range(x0 + 1, x1 - 1), h + s * 0.35, random.range(z0 + 1, z1 - 1)), v(s, s * 0.7, s * random.range(0.7, 1.3)), PROP.fence))
	end
	if random.chance(look.ruin * 2) then
		-- Part of the roof fell in: rubble on top.
		for _ = 1, 4 do
			local s = random.range(0.25, 0.6)
			part(deco(c, "cube", v(random.range(x0 + 0.8, x1 - 0.8), h + s * 0.3, random.range(z0 + 0.8, z1 - 0.8)), v(s * 1.6, s * 0.6, s), color,
				{ roughness = 0.95, rot = v(random.range(-0.4, 0.4), random.range(-math.pi, math.pi), 0) }))
		end
	end
	-- The face towards the path: a boarded-up door and dark windows.
	local fz = zn - side * 0.02
	local dx = random.range(x0 + 1, x1 - 2)
	part(deco(c, "cube", v(dx + 0.6, 1.05, fz), v(1.2, 2.1, 0.06), PROP.dark))
	for k = 0, 2 do
		part(deco(c, "cube", v(dx + 0.6, 0.5 + k * 0.6, fz - side * 0.04), v(1.5, 0.18, 0.05), if k % 2 == 0 then PROP.plank else PROP.board,
			{ rot = v(0, 0, random.range(-0.25, 0.25)) }))
	end
	local floors = math.max(1, math.floor(h / 3))
	for f = 0, floors - 1 do
		for x = x0 + 1, x1 - 1.4, 2.6 do
			if math.abs(x - dx) > 1.6 or f > 0 then
				local y = 1.7 + f * 3
				part(deco(c, "cube", v(x + 0.5, y, fz), v(1, 1.1, 0.05), PROP.dark))
				if random.chance(0.4) then
					part(deco(c, "cube", v(x + 0.5, y, fz - side * 0.04), v(1.2, 0.16, 0.05), PROP.plank, { rot = v(0, 0, random.range(-0.5, 0.5)) }))
				end
			end
		end
	end
	-- Junk against the wall.
	if random.chance(0.6) then
		local x, z = random.range(x0 + 0.8, x1 - 0.8), zn - side * 0.6
		if random.chance(0.5) then
			box(c, x - 0.9, z - 0.5, x + 0.9, z + 0.5, 1.2, { color = PROP.dumpster, roughness = 0.9 })
		else
			for _ = 1, random.int(3, 6) do
				deco(c, "sphere", v(x + random.range(-0.7, 0.7), 0.2, z + random.range(-0.3, 0.3)), random.range(0.3, 0.5), PROP.bag)
			end
		end
	end
	-- What main.luau shows instead when the block is in the way.
	blk.cut = add(c, world.spawn("cube", { pos = v((x0 + x1) / 2, 0.45, (z0 + z1) / 2 + ZC), size = v(w, 0.9, d), color = color, roughness = 0.95, solid = false, tag = "deco", visible = false }))
	table.insert(c.blocks, blk)
	table.insert(c.buildings, blk)
	return blk
end

-- A dead car: rusted or burnt, parked crooked, sometimes on its roof.
local function wreck(c, x, z)
	local yaw = random.range(-0.35, 0.35) + (if random.chance(0.5) then 0 else math.pi)
	local burnt = random.chance(0.35)
	local col = if burnt then BURNT.color else RUST[random.int(1, #RUST)]
	local flipped = not burnt and random.chance(0.15)
	local h = if flipped then 0.9 else 1.0
	-- Solid as a box around it; the look is turned.
	local r = box(c, x - 2.2, z - 1.1, x + 2.2, z + 1.1, h, { color = col, roughness = 0.9 })
	world.set(r.id, { rot = v(0, yaw, 0), size = v(4.0, h, 1.8) })
	if not flipped then
		deco(c, "cube", v(x, h + 0.25, z), v(1.9, 0.5, 1.5), if burnt then "#3a3632" else PROP.dark, { rot = v(0, yaw, 0) })
	end
	if random.chance(0.4) then deco(c, "sphere", v(x + random.range(-3, 3), 0.15, z + random.range(-1.6, 1.6)), 0.55, PROP.tyre) end
	if not burnt and random.chance(0.5) then
		table.insert(c.containers, { kind = "car", x = x, z = z, rect = r, id = r.id, searched = false })
	end
end

-- A barricade someone built and left: sandbags or planks on trestles,
-- across part of the street.
local function barricade(c, x, z0, z1)
	if z0 > z1 then z0, z1 = z1, z0 end
	if random.chance(0.5) then
		box(c, x - 0.35, z0, x + 0.35, z1, 0.7, { color = PROP.sandbag })
		for z = z0 + 0.3, z1 - 0.3, 0.55 do
			deco(c, "cube", v(x + random.range(-0.05, 0.05), 0.78, z), v(0.5, 0.18, 0.5), PROP.sandbag, { rot = v(0, random.range(-0.3, 0.3), 0) })
		end
	else
		box(c, x - 0.15, z0, x + 0.15, z1, 1.0, { color = PROP.plank })
		for z = z0 + 0.4, z1 - 0.4, 1.4 do
			deco(c, "cube", v(x, 0.5, z), v(0.9, 1.0, 0.08), PROP.board, { rot = v(0, 0, random.range(-0.2, 0.2)) })
		end
	end
end

----------------------------------------------------------------------------
-- One chunk per biome

local function lot(c, side, x0, x1, look, biome)
	local span = x1 - x0
	if biome == "suburb" then
		local w = math.min(random.range(8, 11), span - 1)
		local bx = random.range(x0 + 0.5, x1 - w - 0.5)
		if span >= 15 and random.chance(0.35) then
			-- A big house: most of the lot, two floors' worth of rooms.
			local bw = math.min(random.range(13, 18), span - 1)
			building(c, x0 + (span - bw) / 2, bw, random.range(9, 11.5), 3, 2, side, look, { "kitchen", "closet", "closet", "kitchen" }, 1)
		else
			closed(c, bx, w, random.range(6, 8), side, look, random.range(3.6, 5.2))
		end
		-- A front hedge or fence, gapped, and trees behind.
		if random.chance(0.5) then
			local g = random.range(x0 + 1, x1 - 3)
			box(c, x0 + 0.3, side * 4.25 - 0.2, g, side * 4.25 + 0.2, 0.9, { color = PROP.hedge })
			box(c, g + 2, side * 4.25 - 0.2, x1 - 0.3, side * 4.25 + 0.2, 0.9, { color = PROP.hedge })
		end
		for _ = 1, random.int(0, 2) do tree(c, random.range(x0 + 1, x1 - 1), side * random.range(12.4, 13.5)) end
	elseif biome == "forest" then
		if random.chance(0.3) then
			local w = random.range(5, 7)
			local bx = random.range(x0 + 0.5, x1 - w - 0.5)
			building(c, bx, w, random.range(4.5, 6), if random.chance(0.5) then 1 else 2, 1, side, look, { "cabin", "closet" }, 1)
			for _ = 1, 4 do
				local x = random.range(x0, x1)
				if x < bx - 1 or x > bx + w + 1 then tree(c, x, side * random.range(4.5, 13), true) end
			end
		else
			for _ = 1, random.int(6, 10) do tree(c, random.range(x0, x1), side * random.range(3.6, 13.5), true) end
			for _ = 1, random.int(0, 2) do rock(c, random.range(x0, x1), side * random.range(3.6, 13)) end
		end
	elseif biome == "city" then
		local w = math.min(random.range(11, 16), span - 1)
		local bx = random.range(x0 + 0.5, x1 - w - 0.5)
		if span >= 15 and random.chance(0.3) then
			-- A big open ruin: a store, a clinic, offices; three rows deep.
			local roll = random.float()
			local kinds = if roll < 0.2 then { "pharmacy", "office" } elseif roll < 0.5 then { "shop", "shop", "office" } elseif roll < 0.6 then { "police", "office" } else { "office", "kitchen", "closet" }
			local bw = math.min(random.range(17, 22), span - 1)
			building(c, x0 + (span - bw) / 2, bw, random.range(11, 13.5), 4, 3, side, look, kinds, 2)
		else
			closed(c, bx, w, random.range(7, 9.4), side, look, random.range(6, 12))
		end
	else -- farm
		local r = random.float()
		if r < 0.4 then
			local w = math.min(random.range(9, 12), span - 1)
			local bx = random.range(x0 + 0.5, x1 - w - 0.5)
			if random.chance(0.5) then
				building(c, bx, w, random.range(8, 11), 3, 2, side, look, { "barn", "barn", "kitchen" }, 2)
			else
				closed(c, bx, w, random.range(7, 9), side, look, random.range(4.5, 6.5))
			end
		else
			-- Crop rows behind a fence with a gap.
			local crop = CROP[random.int(1, #CROP)]
			for z = 5.5, 13, 1.4 do flat(c, x0 + 0.5, side * z, x1 - 0.5, side * (z + 0.6), 0.03, crop) end
			local g = random.range(x0 + 1, x1 - 3)
			box(c, x0, side * 4.3 - 0.06, g, side * 4.3 + 0.06, 0.7, { color = "#8b6b4a" })
			box(c, g + 2, side * 4.3 - 0.06, x1, side * 4.3 + 0.06, 0.7, { color = "#8b6b4a" })
			if random.chance(0.5) then
				local hx, hz = random.range(x0 + 2, x1 - 2), side * random.range(7, 12)
				container(c, "hay", hx, hz, 1.6, 1.1, 1.1, "#c9b25a")
			end
		end
	end
end

-- A camp at the end of each day: a fire on the path, logs around it.
local function camp(c, x0)
	local x = x0 + M.CHUNK / 2
	c.camp = v(x, 0, 0)
	deco(c, "sphere", v(x, 0.25, 0), 0.6, "#ff9a3a", { emissive = 1.2 })
	for i = 0, 3 do
		local a = i * math.pi / 2 + 0.4
		box(c, x + math.cos(a) * 2.2 - 0.6, math.sin(a) * 2.2 - 0.2, x + math.cos(a) * 2.2 + 0.6, math.sin(a) * 2.2 + 0.2, 0.4, { color = "#5a4632" })
	end
end

-- A street light: a pole and a lamp. Most are dead; a few still burn
-- (main.luau lights them at night), and some flicker.
local function lamp(c, x, side, city)
	local z = side * 3.45
	box(c, x - 0.09, z - 0.09, x + 0.09, z + 0.09, 4.2, { color = "#3a3d42", roughness = 0.6, metal = 0.6 })
	local lit = random.chance(if city then 0.25 else 0.45)
	deco(c, "cube", v(x, 4.15, z - side * 0.5), v(0.5, 0.14, 0.3), if lit then "#fff0c8" else "#4a4640", if lit then { emissive = 1.5 } else nil)
	if lit then table.insert(c.lamps, { pos = v(x, 4, z - side * 0.5), flicker = random.chance(0.3), seed = random.range(0, 10) }) end
end

-- Everything recorded in local z moves to the chunk's centre.
local function shift(c, dz)
	local function p(q) return if q then q + v(0, 0, dz) else nil end
	for _, k in c.containers do
		k.z += dz
		k.stand = p(k.stand)
	end
	for _, b in c.buildings do
		b.z0 += dz
		b.z1 += dz
		for _, rm in b.rooms do
			rm.z0 += dz
			rm.z1 += dz
		end
		for _, d in b.doors do
			d.pa, d.pb = p(d.pa), p(d.pb)
		end
	end
	for i, q in c.inside do c.inside[i] = p(q) end
	for i, q in c.outside do c.outside[i] = p(q) end
	for _, l in c.lamps do l.pos = p(l.pos) end
	for _, w in c.weapons do w.pos = p(w.pos) end
	c.camp = p(c.camp)
end

-- zc: this chunk's path centre; zc_prev: the one before (a turn between).
function M.chunk(index, day, in_day, zc, zc_prev)
	zc = zc or 0
	zc_prev = zc_prev or zc
	local x0 = (index - 1) * M.CHUNK
	local x1 = x0 + M.CHUNK
	local biome = M.biome(day)
	local look = LOOKS[biome]
	local c = { index = index, x0 = x0, x1 = x1, biome = biome, ents = {}, rects = {}, containers = {}, buildings = {}, blocks = {}, weapons = {}, inside = {}, outside = {}, lamps = {}, zc = zc, zc_prev = zc_prev }
	local HZ = M.HALF_Z
	local turn = zc ~= zc_prev
	ZC = 0
	local lo, hi = math.min(zc, zc_prev), math.max(zc, zc_prev)
	add(c, world.spawn("cube", { pos = v((x0 + x1) / 2, -0.5, (lo + hi) / 2), size = v(M.CHUNK, 1, hi - lo + HZ * 2 + 30), color = look.ground, roughness = 1, solid = false, tag = "ground" }))
	if turn then
		-- The street that turns: from the old centre to the new one.
		local half = if biome == "city" then 3.2 else 2.6
		flat(c, x0, lo - half, x0 + M.TURN, hi + half, 0.004, look.path)
		if biome == "city" or biome == "suburb" then
			-- The kerb on the far side, up to where the new path leaves.
			if zc > zc_prev then
				flat(c, x0 + M.TURN, zc_prev - half, x0 + M.TURN + 1.2, zc - half, 0.008, look.edge)
			else
				flat(c, x0 + M.TURN, zc + half, x0 + M.TURN + 1.2, zc_prev + half, 0.008, look.edge)
			end
		end
	end
	ZC = zc
	if biome == "city" then
		-- Asphalt, a faded centre line and pavements up to the buildings.
		flat(c, x0, -3.2, x1, 3.2, 0.004, look.path)
		flat(c, x0, -4.4, x1, -3.2, 0.008, look.edge)
		flat(c, x0, 3.2, x1, 4.4, 0.008, look.edge)
		for x = x0, x1 - 2, 4 do
			if random.chance(0.7) then flat(c, x, -0.06, x + 2, 0.06, 0.009, "#c9b25a") end
		end
	else
		flat(c, x0, -2.6, x1, 2.6, 0.004, look.path)
		if biome == "suburb" then
			flat(c, x0, -3.6, x1, -2.6, 0.008, look.edge)
			flat(c, x0, 2.6, x1, 3.6, 0.008, look.edge)
			for x = x0, x1 - 2, 4 do flat(c, x, -0.08, x + 2, 0.08, 0.009, "#d8d0a0") end
		end
	end
	if in_day == 0 then
		camp(c, x0)
		shift(c, zc)
		ZC = 0
		return c
	end
	-- The first chunk of a run stays clear: the group starts there.
	if index > 1 then
		for _, side in { -1, 1 } do
			-- One or two lots per side; after a turn the street takes the
			-- west end.
			local a = if turn then x0 + M.TURN + 1.4 else x0
			if not turn and random.chance(0.3) then
				lot(c, side, a, a + 12, look, biome)
				lot(c, side, a + 12, x1, look, biome)
			else
				lot(c, side, a, x1, look, biome)
			end
			table.insert(c.outside, v(random.range(x0, x1), 0, side * random.range(4, HZ - 1)))
		end
		-- Street lights in town.
		if biome == "suburb" or biome == "city" then
			for x = x0 + 4, x1 - 4, 16 do
				if random.chance(0.8) then lamp(c, x + random.range(-2, 2), if random.chance(0.5) then -1 else 1, biome == "city") end
			end
		end
		-- Wrecks on the path, now and then a police car.
		if biome ~= "forest" and random.chance(0.6) then
			car(c, random.range(x0 + 3, x1 - 3), (if random.chance(0.5) then -1 else 1) * random.range(1.75, 2.2), biome == "city" and random.chance(0.25), true)
		end
		-- The city is a ruin: burnt wrecks, collapsed slabs and rubble in
		-- the street, paper blowing about.
		if biome == "city" then
			if random.chance(0.5) then
				car(c, random.range(x0 + 3, x1 - 3), (if random.chance(0.5) then -1 else 1) * random.range(1.8, 2.3), false, random.chance(0.7), true)
			end
			for _ = 1, random.int(2, 4) do
				local side = if random.chance(0.5) then -1 else 1
				local x, z = random.range(x0 + 1, x1 - 1), side * random.range(2.6, 3.8)
				local look_ = { color = DEBRIS[random.int(1, #DEBRIS)] }
				if random.chance(0.4) then
					-- A fallen slab of facade, solid.
					local w = random.range(1, 2.2)
					box(c, x - w / 2, z - 0.35, x + w / 2, z + 0.35, random.range(0.3, 0.6), look_)
				end
				rubble(c, x, z, 0.9, look_, random.int(3, 7))
			end
			for _ = 1, random.int(3, 6) do
				local x, z = random.range(x0, x1), random.range(-HZ + 1, HZ - 1)
				flat(c, x, z, x + random.range(0.2, 0.5), z + random.range(0.2, 0.4), 0.012, "#d8d0a0")
			end
			-- Cracks in the asphalt.
			for _ = 1, random.int(1, 3) do
				local x, z = random.range(x0, x1 - 3), random.range(-2.5, 2.5)
				flat(c, x, z, x + random.range(1, 3), z + 0.06, 0.006, "#202024")
			end
		end
		if biome == "forest" and random.chance(0.4) then
			-- A fallen tree across part of the trail.
			local x = random.range(x0 + 3, x1 - 3)
			local z = if random.chance(0.5) then -2.5 else 2.5
			box(c, x - 0.3, z - 1.6, x + 0.3, z + 1.6, 0.5, { color = "#5a4632" })
		end
		-- Dead cars along the kerbs and now and then a barricade, in town.
		-- Nothing solid goes in the middle 1.6 m of the path: the way east
		-- is always open.
		if biome == "suburb" or biome == "city" or biome == "farm" then
			for _ = 1, random.int(1, if biome == "city" then 3 else 2) do
				local side = if random.chance(0.5) then -1 else 1
				wreck(c, random.range(x0 + 3, x1 - 3), side * random.range(2.0, 2.4))
			end
			if random.chance(if biome == "city" then 0.35 else 0.15) then
				local side = if random.chance(0.5) then -1 else 1
				barricade(c, random.range(x0 + 4, x1 - 4), side * 3.4, side * random.range(1.0, 1.6))
			end
		end
		for _ = 1, 3 do table.insert(c.outside, v(random.range(x0, x1), 0, random.range(-3, 3))) end
		-- Nobody waits inside a solid block.
		for i = #c.outside, 1, -1 do
			local p = c.outside[i]
			for _, b in c.blocks do
				if p.x > b.x0 - 1 and p.x < b.x1 + 1 and p.z > b.z0 - 1 and p.z < b.z1 + 1 then
					c.outside[i] = v(p.x, 0, random.range(-3, 3))
					break
				end
			end
		end
		if random.chance(0.45) then
			local kinds = if day >= 2 then { "pipe", "bat", "machete", "axe" } else { "pipe", "pipe", "bat" }
			table.insert(c.weapons, { kind = kinds[random.int(1, #kinds)], pos = v(random.range(x0 + 1, x1 - 1), 0, random.range(-3, 3)) })
		end
	elseif day == 1 then
		table.insert(c.weapons, { kind = "pipe", pos = v(x0 + 14, 0, 1.2) })
	end
	shift(c, zc)
	ZC = 0
	return c
end

-- Where the path runs at x, and how far north and south walkers may go,
-- from the chunks' centres (zcs[index]).
function M.center(zcs, x)
	zcs = zcs or {}
	local index = math.floor(x / M.CHUNK) + 1
	return zcs[index] or zcs[index - 1] or 0
end

function M.zrange(zcs, x)
	zcs = zcs or {}
	local index = math.floor(x / M.CHUNK) + 1
	local zc = zcs[index] or zcs[index - 1] or 0
	local prev = zcs[index - 1] or zc
	if x - (index - 1) * M.CHUNK < M.TURN + 1 then
		return math.min(zc, prev) - M.HALF_Z, math.max(zc, prev) + M.HALF_Z
	end
	return zc - M.HALF_Z, zc + M.HALF_Z
end

return M
