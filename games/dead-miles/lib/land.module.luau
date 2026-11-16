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
M.HALF_Z = 14 -- the strip runs from z -14 to 14
M.DAY_CHUNKS = 7 -- chunks of walking per day, then the camp
local WALL_H, WALL_T = 1.35, 0.25

M.BIOMES = { "suburb", "forest", "city", "farm" }

local LOOKS = {
	suburb = { ground = "#55623f", path = "#2c2c2e", edge = "#8c8984", walls = { "#c9b79c", "#b7c2b0", "#d6c4a8", "#a9b4bf", "#c7a88f" }, floors = { "#7a5a3e", "#8b6b4a", "#9a8f80" } },
	forest = { ground = "#3f4a2e", path = "#6b5a42", edge = "#4d4433", walls = { "#6f5a44", "#7a644b" }, floors = { "#5e4a36" } },
	city = { ground = "#4a4846", path = "#2a2a2c", edge = "#6e6b66", walls = { "#8e8a84", "#a39c90", "#7c8088", "#9a8a7a" }, floors = { "#5c5a56", "#6b665e", "#4e5056" } },
	farm = { ground = "#6b6a3c", path = "#7b6646", edge = "#5d5032", walls = { "#8a3a2a", "#a0503a", "#c9b79c" }, floors = { "#6b5a3e", "#7a6a48" } },
}
local CAR_COLORS = { "#6b2b2b", "#2f4d6b", "#5d6650", "#8a8478", "#3a3a40", "#7a5a2a" }
local CROP = { "#7d8a3a", "#8f8a40", "#6a7a32" }

function M.biome(day) return M.BIOMES[(day - 1) % #M.BIOMES + 1] end

-- Every look the land uses, warmed in init.
function M.warm()
	for _, l in LOOKS do
		for _, c in { l.ground, l.path, l.edge } do world.warm("cube", { color = c, roughness = 0.95 }) end
		for _, c in l.walls do world.warm("cube", { color = c, roughness = 0.95 }) end
		for _, c in l.floors do world.warm("cube", { color = c, roughness = 0.95 }) end
	end
	for _, c in CAR_COLORS do world.warm("cube", { color = c, roughness = 0.4, metal = 0.3 }) end
	for _, c in CROP do world.warm("cube", { color = c, roughness = 0.95 }) end
	for _, c in { "#d8d0a0", "#4a3a2a", "#5a4632", "#202024", "#e8e8f0", "#1d3a8a", "#8b6b4a", "#6b4e37", "#7a7470", "#5f5a55", "#c9b25a", "#3a3632" } do
		world.warm("cube", { color = c, roughness = 0.9 })
		world.warm("cube", { color = c, roughness = 0.8 })
		world.warm("cube", { color = c, roughness = 0.95 })
	end
	for _, c in { "#3c5a2c", "#2f4a26", "#26401f", "#4a6a32" } do world.warm("sphere", { color = c, roughness = 0.9 }) end
	world.warm("sphere", { color = "#7a7470", roughness = 0.9 })
	world.warm("sphere", { color = "#ff9a3a", emissive = 1.2 })
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
		pos = v((x0 + x1) / 2, y, (z0 + z1) / 2),
		size = v(math.abs(x1 - x0), 0.02, math.abs(z1 - z0)),
		color = color,
		roughness = 0.95,
		solid = false,
		tag = "ground",
	}))
end

local function deco(c, shape, pos, size, color, extra)
	local o = { pos = pos, size = size, color = color, roughness = 0.9, solid = false, tag = "deco" }
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

local function car(c, x, z, police, yaw_x)
	local color = if police then "#e8e8f0" else CAR_COLORS[random.int(1, #CAR_COLORS)]
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
			if gap[1] - xa > 0.05 then box(c, xa, z - WALL_T / 2, gap[1], z + WALL_T / 2, WALL_H, wall) end
			if xb - gap[2] > 0.05 then box(c, gap[2], z - WALL_T / 2, xb, z + WALL_T / 2, WALL_H, wall) end
		else
			box(c, xa, z - WALL_T / 2, xb, z + WALL_T / 2, WALL_H, wall)
		end
	end
	local function wall_z(za, zb, x, gap)
		if za > zb then za, zb = zb, za end
		if gap then
			if gap[1] - za > 0.05 then box(c, x - WALL_T / 2, za, x + WALL_T / 2, gap[1], WALL_H, wall) end
			if zb - gap[2] > 0.05 then box(c, x - WALL_T / 2, gap[2], x + WALL_T / 2, zb, WALL_H, wall) end
		else
			box(c, x - WALL_T / 2, za, x + WALL_T / 2, zb, WALL_H, wall)
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
	-- A cupboard in a corner of most rooms, sleepers in some.
	for _, r in b.rooms do
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

----------------------------------------------------------------------------
-- One chunk per biome

local function lot(c, side, x0, x1, look, biome)
	local span = x1 - x0
	if biome == "suburb" then
		if random.chance(0.75) then
			local w = math.min(random.range(8, 11), span - 1)
			local bx = random.range(x0 + 0.5, x1 - w - 0.5)
			building(c, bx, w, random.range(6.5, 8.5), 2, if random.chance(0.5) then 1 else 2, side, look, { "kitchen", "closet", "closet" }, 1)
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
		if random.chance(0.85) then
			local w = math.min(random.range(11, 16), span - 1)
			local bx = random.range(x0 + 0.5, x1 - w - 0.5)
			local roll = random.float()
			local kinds = if roll < 0.2 then { "pharmacy", "office" } elseif roll < 0.5 then { "shop", "shop", "office" } elseif roll < 0.6 then { "police", "office" } else { "office", "kitchen", "closet" }
			building(c, bx, w, random.range(7.5, 9.5), 3, 2, side, look, kinds, 2)
		end
		for _ = 1, random.int(1, 3) do
			local x, z = random.range(x0, x1), side * random.range(1.8, 2.5)
			box(c, x - 0.4, z - 0.3, x + 0.4, z + 0.3, 0.35, { color = "#5f5a55" })
		end
	else -- farm
		local r = random.float()
		if r < 0.4 then
			local w = math.min(random.range(9, 12), span - 1)
			local bx = random.range(x0 + 0.5, x1 - w - 0.5)
			building(c, bx, w, random.range(7, 9), 2, 1, side, look, { "barn", "barn", "kitchen" }, 2)
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
local function lamp(c, x, side)
	local z = side * 3.45
	box(c, x - 0.09, z - 0.09, x + 0.09, z + 0.09, 4.2, { color = "#3a3d42", roughness = 0.6, metal = 0.6 })
	local lit = random.chance(0.45)
	deco(c, "cube", v(x, 4.15, z - side * 0.5), v(0.5, 0.14, 0.3), if lit then "#fff0c8" else "#4a4640", if lit then { emissive = 1.5 } else nil)
	if lit then table.insert(c.lamps, { pos = v(x, 4, z - side * 0.5), flicker = random.chance(0.3), seed = random.range(0, 10) }) end
end

function M.chunk(index, day, in_day)
	local x0 = (index - 1) * M.CHUNK
	local x1 = x0 + M.CHUNK
	local biome = M.biome(day)
	local look = LOOKS[biome]
	local c = { index = index, x0 = x0, x1 = x1, biome = biome, ents = {}, rects = {}, containers = {}, buildings = {}, weapons = {}, inside = {}, outside = {}, lamps = {} }
	local HZ = M.HALF_Z
	add(c, world.spawn("cube", { pos = v((x0 + x1) / 2, -0.5, 0), size = v(M.CHUNK, 1, HZ * 2 + 30), color = look.ground, roughness = 1, solid = false, tag = "ground" }))
	if biome == "city" then
		flat(c, x0, -HZ, x1, HZ, 0.004, look.path)
		flat(c, x0, -3.2, x1, -2.9, 0.008, look.edge)
		flat(c, x0, 2.9, x1, 3.2, 0.008, look.edge)
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
		return c
	end
	-- The first chunk of a run stays clear: the group starts there.
	if index > 1 then
		for _, side in { -1, 1 } do
			-- One or two lots per side.
			if random.chance(0.4) then
				lot(c, side, x0, x0 + 12, look, biome)
				lot(c, side, x0 + 12, x1, look, biome)
			else
				lot(c, side, x0, x1, look, biome)
			end
			table.insert(c.outside, v(random.range(x0, x1), 0, side * random.range(4, HZ - 1)))
		end
		-- Street lights in town.
		if biome == "suburb" or biome == "city" then
			for x = x0 + 4, x1 - 4, 16 do
				if random.chance(0.8) then lamp(c, x + random.range(-2, 2), if random.chance(0.5) then -1 else 1) end
			end
		end
		-- Wrecks on the path, now and then a police car.
		if biome ~= "forest" and random.chance(0.6) then
			car(c, random.range(x0 + 3, x1 - 3), (if random.chance(0.5) then -1 else 1) * random.range(0.6, 1.6), biome == "city" and random.chance(0.25), true)
		end
		if biome == "forest" and random.chance(0.4) then
			-- A fallen tree across part of the trail.
			local x = random.range(x0 + 3, x1 - 3)
			local z = if random.chance(0.5) then -1.3 else 1.3
			box(c, x - 0.3, z - 2.2, x + 0.3, z + 2.2, 0.5, { color = "#5a4632" })
		end
		for _ = 1, 3 do table.insert(c.outside, v(random.range(x0, x1), 0, random.range(-3, 3))) end
		if random.chance(0.45) then
			local kinds = if day >= 2 then { "pipe", "bat", "machete", "axe" } else { "pipe", "pipe", "bat" }
			table.insert(c.weapons, { kind = kinds[random.int(1, #kinds)], pos = v(random.range(x0 + 1, x1 - 1), 0, random.range(-3, 3)) })
		end
	elseif day == 1 then
		table.insert(c.weapons, { kind = "pipe", pos = v(x0 + 14, 0, 1.2) })
	end
	return c
end

return M
