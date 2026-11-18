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
local mat = require("lib/mat")
local interior = require("lib/interior")

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
local ROOFS = { "#5a5650", "#6a665e", "#4e4a44", "#625a50" } -- tar and gravel
local TILES = { "#6a3226", "#5a2e24", "#3a3a3e", "#4a3a30" } -- roof tiles: terracotta, slate, brown
local SIGNS = { "#6a2a24", "#2a4a5a", "#5a5a2a", "#3a3a3a" } -- faded shop fascias
local FRAME = "#c8c2b4" -- window and door frames
local KERB = "#9a968e"
local DRAIN = "#26272a"
local CANS = { "#8a2420", "#7a7d82", "#2a4a7a" }
local BOTTLE = "#2a4a2a"
local FALLEN = { "#6b4a22", "#5a3e1e", "#7a5a30" }
local BARK = "#4a3e32"
local LEAVES = { "#3c5a2c", "#4a6a32", "#56703a", "#5e6a30" }
local DARK_LEAVES = { "#26401f", "#2f4a26", "#2a3f24" }
local GLASS = "#202024"
local HOLE = "#08080a"
-- Set dressing (not solid), by name: every look is warmed in M.warm.
local PROP = {
	plank = "#6b4e37", board = "#8b6b4a", dark = "#161618", sandbag = "#8a7a5a", bag = "#1d1d1f",
	dumpster = "#2f4a3a", barrel = "#6a3a2a", hedge = "#2f4a26", fence = "#8a8478", tyre = "#1a1a1a",
}
local SHROUD = "#060607"
local DEBRIS = { "#5f5a55", "#7a7470", "#4a4640", "#3a3632" }
local CROP = { "#7d8a3a", "#8f8a40", "#6a7a32" }

function M.biome(day) return M.BIOMES[(day - 1) % #M.BIOMES + 1] end

-- Every look the land uses, warmed in init.
function M.warm()
	for _, l in LOOKS do
		for _, c in { l.ground, l.path, l.edge } do mat.warm("cube", { color = c, roughness = 0.95 }) end
		mat.warm("cube", { color = l.ground, roughness = 1 })
		for _, c in l.walls do mat.warm("cube", { color = c, roughness = 0.95 }) end
		for _, c in l.floors do mat.warm("cube", { color = c, roughness = 0.95 }) end
	end
	for _, c in CAR_COLORS do
		mat.warm("cube", { color = c, roughness = 0.4, metal = 0.3 })
		mat.warm("cube", { color = c, roughness = 0.95 })
	end
	mat.warm("cube", { color = "#e8e8f0", roughness = 0.95 })
	mat.warm("cube", { color = "#e8e8f0", roughness = 0.4, metal = 0.3 })
	mat.warm("cube", BURNT)
	for _, c in RUST do mat.warm("cube", { color = c, roughness = 0.9 }) end
	for _, c in ROOFS do mat.warm("cube", { color = c, roughness = 0.95 }) end
	for _, c in DEBRIS do
		mat.warm("cube", { color = c, roughness = 0.95 })
		mat.warm("cube", { color = c, roughness = 0.9 })
	end
	for _, c in PROP do
		mat.warm("cube", { color = c, roughness = 0.9 })
		mat.warm("cube", { color = c, roughness = 0.95 })
		mat.warm("sphere", { color = c, roughness = 0.9 })
	end

	for _, c in CROP do mat.warm("cube", { color = c, roughness = 0.95 }) end
	for _, c in { "#2a2624", "#d8d0a0", "#4a3a2a", "#5a4632", "#202024", "#e8e8f0", "#1d3a8a", "#8b6b4a", "#6b4e37", "#7a7470", "#5f5a55", "#c9b25a", "#3a3632" } do
		mat.warm("cube", { color = c, roughness = 0.9 })
		mat.warm("cube", { color = c, roughness = 0.8 })
		mat.warm("cube", { color = c, roughness = 0.95 })
	end
	for _, c in { "#3c5a2c", "#2f4a26", "#26401f", "#4a6a32" } do mat.warm("sphere", { color = c, roughness = 0.9 }) end
	mat.warm("sphere", { color = "#7a7470", roughness = 0.9 })
	mat.warm("cube", { color = SHROUD, roughness = 1 })
	mat.warm("cube", { color = "#3a3a3c", roughness = 0.95 })
	mat.warm("sphere", { color = "#ff9a3a", emissive = 1.2, roughness = 0.9 })
	mat.warm("cube", { color = "#3a3d42", roughness = 0.6, metal = 0.6 })
	mat.warm("cube", { color = "#fff0c8", roughness = 0.9, emissive = 1.5 })
	mat.warm("cube", { color = "#4a4640", roughness = 0.9 })
	-- Roofs, windows, shopfronts.
	for _, c in TILES do mat.warm("cube", { color = c, roughness = 0.85, surface = mat.TILES }) end
	for _, c in SIGNS do mat.warm("cube", { color = c, roughness = 0.7 }) end
	mat.warm("cube", { color = FRAME, roughness = 0.8 })
	for _, c in { "#7a7470", "#5f5a55", "#7a4a3a" } do mat.warm("cube", { color = c, roughness = 0.95 }) end
	for _, c in { "#7a7d82", "#5f6266", "#5a5a5e" } do mat.warm("cube", { color = c, roughness = 0.6, metal = 0.5 }) end
	mat.warm("sphere", { color = "#3a3d42", roughness = 0.6, metal = 0.6 })
	mat.warm("cube", { color = "#6a3a2a", roughness = 0.9 })
	-- Trees, kerbs, litter.
	mat.warm("cube", { color = BARK, roughness = 0.95 })
	mat.warm("cube", { color = BARK, roughness = 0.9 })
	for _, c in LEAVES do mat.warm("sphere", { color = c, roughness = 0.9 }) end
	for _, c in DARK_LEAVES do mat.warm("sphere", { color = c, roughness = 0.9 }) end
	mat.warm("cube", { color = KERB, roughness = 0.95 })
	mat.warm("cube", { color = DRAIN, roughness = 0.6, metal = 0.5 })
	mat.warm("sphere", { color = DRAIN, roughness = 0.6, metal = 0.5 })
	for _, c in CANS do mat.warm("cube", { color = c, roughness = 0.6, metal = 0.5 }) end
	mat.warm("cube", { color = BOTTLE, roughness = 0.1 })
	for _, c in FALLEN do mat.warm("cube", { color = c, roughness = 0.9 }) end
	mat.warm("cube", { color = "#d8d0a0", roughness = 0.95 })
	interior.warm(function(shape, look)
		look.indoor = true
		mat.warm(shape, look)
	end)
	for _, l in LOOKS do
		for _, c in l.floors do mat.warm("cube", { color = c, roughness = 0.95, indoor = true }) end
		for _, c in l.walls do mat.warm("cube", { color = c, roughness = 0.95, indoor = true }) end
	end
	mat.warm("cube", { color = "#d8d0a0", roughness = 0.95, indoor = true })
	-- Cars.
	for _, c in { GLASS, HOLE } do mat.warm("cube", { color = c, roughness = 0.1 }) end
	mat.warm("cube", { color = "#2a2a2c", roughness = 0.7 })
	for _, c in { "#d8d0a0", "#7a1a14" } do mat.warm("cube", { color = c, roughness = 0.3 }) end
	for _, c in { "#202024", "#1d3a8a", "#8a1d1d" } do mat.warm("cube", { color = c, roughness = 0.4 }) end
	mat.warm("sphere", { color = "#1a1a1a", roughness = 0.85 })
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
	local id = add(c, mat.spawn("cube", {
		pos = v((x0 + x1) / 2, h / 2, (z0 + z1) / 2),
		size = v(x1 - x0, h, z1 - z0),
		color = look.color,
		roughness = look.roughness or 0.95,
		metal = look.metal,
		indoor = look.indoor,
		tag = tag or "wall",
	}))
	local r = { x0 = x0, z0 = z0, x1 = x1, z1 = z1, h = h, id = id }
	table.insert(c.rects, r)
	return r
end

local function flat(c, x0, z0, x1, z1, y, color, indoor)
	return add(c, mat.spawn("cube", {
		pos = v((x0 + x1) / 2, y, (z0 + z1) / 2 + ZC),
		size = v(math.abs(x1 - x0), 0.02, math.abs(z1 - z0)),
		color = color,
		roughness = 0.95,
		solid = false,
		tag = "ground",
		indoor = indoor,
	}))
end

local function deco(c, shape, pos, size, color, extra)
	local o = { pos = pos + v(0, 0, ZC), size = size, color = color, roughness = 0.9, solid = false, tag = "deco" }
	if extra then
		for k, x in extra do o[k] = x end
	end
	return add(c, mat.spawn(shape, o))
end

local function container(c, kind, x, z, w, d, h, color)
	local r = box(c, x - w / 2, z - d / 2, x + w / 2, z + d / 2, h, { color = color, roughness = 0.8 })
	table.insert(c.containers, { kind = kind, x = x, z = z, rect = r, id = r.id, searched = false })
end

-- A tree: a solid trunk with a branch or two and a crown of several
-- clumps; in the woods, darker and taller, the clumps in tiers.
local function tree(c, x, z, dark)
	local h = if dark then random.range(3.4, 4.6) else random.range(2.6, 3.6)
	box(c, x - 0.16, z - 0.16, x + 0.16, z + 0.16, h, { color = BARK })
	deco(c, "cube", v(x, 0.25, z), v(0.46, 0.5, 0.46), BARK, { rot = v(0, random.range(0, 1.5), 0) })
	local ys = random.range(0, math.pi * 2)
	for k = 1, random.int(1, 2) do
		local a = ys + k * 2.4
		local len = random.range(0.8, 1.3)
		local d = v(math.cos(a), 0, math.sin(a))
		deco(c, "cube", v(x, h - 0.6, z) + d * (len * 0.35) + v(0, len * 0.3, 0), v(0.1, len, 0.1), BARK,
			{ rot = v(math.sin(a) * 0.8, 0, -math.cos(a) * 0.8) })
	end
	local cols = if dark then DARK_LEAVES else LEAVES
	if dark then
		-- Clumps round the top in no order, the highest smallest.
		local r = random.range(1.5, 2.0)
		deco(c, "sphere", v(x, h + 0.2, z), v(r * 1.8, r * 1.3, r * 1.8), cols[random.int(1, #cols)])
		for _ = 1, random.int(3, 5) do
			local a = random.range(0, math.pi * 2)
			local up = random.range(-0.6, 1.4)
			local s = r * random.range(0.5, 0.8) * (1 - up * 0.15)
			local out = r * random.range(0.45, 0.9)
			deco(c, "sphere", v(x + math.cos(a) * out, h + up, z + math.sin(a) * out), v(s * 2, s * 1.5, s * 2), cols[random.int(1, #cols)])
		end
	else
		local r = random.range(1.1, 1.5)
		deco(c, "sphere", v(x, h + 0.5, z), r * 2, cols[random.int(1, #cols)])
		for _ = 1, random.int(2, 4) do
			local a = random.range(0, math.pi * 2)
			local s = r * random.range(0.55, 0.85)
			deco(c, "sphere", v(x + math.cos(a) * r * 0.75, h + random.range(-0.1, 0.9), z + math.sin(a) * r * 0.75), s * 2, cols[random.int(1, #cols)])
		end
	end
end

-- The floor of a wood: bushes, a stump, ferns, drifts of dead leaves.
local function undergrowth(c, x0, x1, za, zb, n)
	if za > zb then za, zb = zb, za end
	if x0 > x1 then x0, x1 = x1, x0 end
	for _ = 1, n do
		local x, z = random.range(x0, x1), random.range(za, zb)
		local roll = random.float()
		if roll < 0.45 then
			local s = random.range(0.6, 1.3)
			deco(c, "sphere", v(x, s * 0.25, z), v(s * 1.4, s * 0.7, s * 1.2), DARK_LEAVES[random.int(1, #DARK_LEAVES)])
		elseif roll < 0.55 then
			deco(c, "cube", v(x, 0.18, z), v(0.42, 0.36, 0.42), BARK, { rot = v(0, random.range(0, 1.5), 0) })
		elseif roll < 0.75 then
			for _ = 1, random.int(3, 5) do
				deco(c, "sphere", v(x + random.range(-0.4, 0.4), 0.1, z + random.range(-0.4, 0.4)), v(0.5, 0.18, 0.5), LEAVES[random.int(1, #LEAVES)])
			end
		else
			for _ = 1, random.int(5, 10) do
				deco(c, "cube", v(x + random.range(-0.8, 0.8), 0.01, z + random.range(-0.8, 0.8)), v(0.1, 0.004, 0.07), FALLEN[random.int(1, #FALLEN)], { roughness = 0.9, rot = v(0, random.range(0, 3), 0) })
			end
		end
	end
end

local function rock(c, x, z)
	local s = random.range(0.6, 1.4)
	box(c, x - s / 2, z - s / 2, x + s / 2, z + s / 2, s * 0.6, { color = "#7a7470" })
end

-- A car in parts on an invisible root (its length along local x): a body
-- with sills, a glass cabin under its roof, wheels, bumpers and lamps.
-- o: color, burnt, rust (a rotted wreck), flipped, police, bare (no wheels).
local function car_model(c, pos, yaw, o)
	local root = add(c, world.spawn("cube", { pos = pos + v(0, 0, ZC), rot = v(if o.flipped then math.pi else 0, yaw, 0), size = 0.05, visible = false, solid = false }))
	local function part(shape, at, size, color, extra)
		local t = { pos = at, size = size, color = color, roughness = 0.9, solid = false, tag = "deco" }
		if extra then for k, x in extra do t[k] = x end end
		return mat.spawn(shape, t, root)
	end
	local paint = if o.burnt then BURNT.color else o.color
	local pl = if o.burnt or o.rust then nil else { roughness = 0.4, metal = 0.3 }
	-- (A flipped one is turned over on its root, which sits at roof height.)
	local function at(x, y, z) return v(x, y, z) end
	-- Body: a long low box, the bonnet and boot a step down from the waist.
	part("cube", at(0, 0.55, 0), v(4.1, 0.56, 1.76), paint, pl)
	part("cube", at(1.45, 0.86, 0), v(1.15, 0.08, 1.7), paint, pl)
	part("cube", at(-1.6, 0.86, 0), v(0.85, 0.08, 1.7), paint, pl)
	-- The cabin: glass all round (gone, on a burnt one), and the roof.
	local glass = if o.burnt then HOLE else GLASS
	part("cube", at(-0.2, 1.08, 0), v(2.0, 0.46, 1.58), glass, { roughness = 0.1 })
	part("cube", at(-0.3, 1.33, 0), v(1.55, 0.07, 1.5), paint, pl)
	-- Pillars between the windows.
	for _, sx in { 0.78, -1.15 } do
		part("cube", at(sx, 1.08, 0.77), v(0.08, 0.46, 0.06), paint, pl)
		part("cube", at(sx, 1.08, -0.77), v(0.08, 0.46, 0.06), paint, pl)
	end
	-- Bumpers and lamps.
	part("cube", at(2.08, 0.42, 0), v(0.14, 0.2, 1.74), "#2a2a2c", { roughness = 0.7 })
	part("cube", at(-2.08, 0.42, 0), v(0.14, 0.2, 1.74), "#2a2a2c", { roughness = 0.7 })
	if not o.burnt then
		for _, sz in { 0.62, -0.62 } do
			part("cube", at(2.06, 0.66, sz), v(0.06, 0.12, 0.32), "#d8d0a0", { roughness = 0.3 })
			part("cube", at(-2.06, 0.66, sz), v(0.06, 0.12, 0.3), "#7a1a14", { roughness = 0.3 })
		end
	end
	-- Wheels (sometimes taken).
	if not o.bare then
		for _, w in { v(1.32, 0.31, 0.8), v(1.32, 0.31, -0.8), v(-1.32, 0.31, 0.8), v(-1.32, 0.31, -0.8) } do
			part("sphere", at(w.x, w.y, w.z), v(0.64, 0.64, 0.24), "#1a1a1a", { roughness = 0.85 })
		end
	end
	if o.police then
		part("cube", at(-0.3, 1.42, 0), v(0.3, 0.1, 1.1), "#202024", { roughness = 0.4 })
		part("cube", at(-0.3, 1.43, 0.35), v(0.28, 0.1, 0.3), "#1d3a8a", { roughness = 0.4 })
		part("cube", at(-0.3, 1.43, -0.35), v(0.28, 0.1, 0.3), "#8a1d1d", { roughness = 0.4 })
		part("cube", at(0, 0.62, 0.885), v(3.2, 0.18, 0.01), "#1d3a8a", { roughness = 0.4 })
		part("cube", at(0, 0.62, -0.885), v(3.2, 0.18, 0.01), "#1d3a8a", { roughness = 0.4 })
	end
	return root
end

-- Is the ground from x0,z0 to x1,z1 (chunk-local z) clear of anything
-- solid placed so far?
local function clear(c, x0, z0, x1, z1)
	for _, r in c.rects do
		if x0 < r.x1 and x1 > r.x0 and z0 + ZC < r.z1 and z1 + ZC > r.z0 then return false end
	end
	return true
end

local function car(c, x, z, police, yaw_x, burnt)
	if not clear(c, x - 2.3, z - 1.2, x + 2.3, z + 1.2) then return end
	local color = if police then "#e8e8f0" else CAR_COLORS[random.int(1, #CAR_COLORS)]
	local w, d = if yaw_x then 2.1 else 0.92, if yaw_x then 0.92 else 2.1
	local r = box(c, x - w, z - d, x + w, z + d, 1.3, { color = color })
	world.set(r.id, { visible = false })
	local root = car_model(c, v(x, 0, z), if yaw_x then random.range(-0.06, 0.06) else math.pi / 2 + random.range(-0.06, 0.06), { color = color, police = police, burnt = burnt, bare = burnt and random.chance(0.6) })
	if burnt then return end
	-- Shown once it has been gone through: a door hanging open, the boot up.
	local side = if random.chance(0.5) then 1 else -1
	local open = {
		mat.spawn("cube", { pos = v(0.6 - 0.5 * math.cos(1.0), 0.78, side * (0.88 + 0.5 * math.sin(1.0))), rot = v(0, side * 1.0, 0), size = v(1.0, 0.62, 0.05), color = color, roughness = 0.4, metal = 0.3, solid = false, tag = "deco", visible = false }, root),
		mat.spawn("cube", { pos = v(-1.85, 1.25, 0), rot = v(0, 0, -1.1), size = v(0.85, 0.06, 1.66), color = color, roughness = 0.4, metal = 0.3, solid = false, tag = "deco", visible = false }, root),
	}
	table.insert(c.containers, { kind = if police then "police" else "car", x = x, z = z, rect = r, id = r.id, searched = false, reveal = open })
end

-- Rubble: a few tumbled blocks of a wall's colour around a point.
local function rubble(c, x, z, spread, look, n)
	for _ = 1, n do
		local s = random.range(0.15, 0.45)
		deco(c, "cube", v(x + random.range(-spread, spread), s * 0.35, z + random.range(-spread, spread)), v(s * random.range(1, 2.2), s * 0.7, s), look.color,
			{ roughness = 0.95, indoor = look.indoor, rot = v(random.range(-0.4, 0.4), random.range(-math.pi, math.pi), random.range(-0.4, 0.4)) })
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
	flat(c, x0, z0, x0 + w, z1, 0.012, floor, true)
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
	local room_api = {
		box = function(x0, z0, x1, z1, h, l)
			l.indoor = true
			return box(c, x0, z0, x1, z1, h, l, "furniture").id
		end,
		hidden = function(x0, z0, x1, z1, h)
			local r = box(c, x0, z0, x1, z1, h, { color = SHROUD }, "furniture")
			world.set(r.id, { visible = false })
			return r.id
		end,
		deco = function(shape, pos, size, color, extra)
			local e = { indoor = true }
			for k, x in extra or {} do e[k] = x end
			return deco(c, shape, pos, size, color, e)
		end,
	}
	-- A ceiling no camera sees: the moon and the sun stay out, so inside it
	-- is dark but for torches and what comes in through doors and holes.
	add(c, mat.spawn("cube", { pos = v((b.x0 + b.x1) / 2, look.height[2] + 0.4, (b.z0 + b.z1) / 2 + ZC), size = v(b.x1 - b.x0 + 0.3, 0.2, b.z1 - b.z0 + 0.3),
		color = SHROUD, roughness = 1, solid = false, shadow_only = true, tag = "deco" }))
	for _, r in b.rooms do
		if random.chance(0.6) then
			rubble(c, random.range(r.x0 + 0.6, r.x1 - 0.6), random.range(r.z0 + 0.6, r.z1 - 0.6), 0.6, { color = wall.color, indoor = true }, random.int(2, 5))
		end
		for _ = 1, random.int(0, 2) do
			local x, z = random.range(r.x0 + 0.3, r.x1 - 0.6), random.range(r.z0 + 0.3, r.z1 - 0.6)
			flat(c, x, z, x + random.range(0.2, 0.5), z + random.range(0.15, 0.4), 0.016, "#d8d0a0", true)
		end
		-- Furniture against the walls; most rooms have something to search.
		local kind = if random.chance(0.75) then kinds[random.int(1, #kinds)] else nil
		local found = interior.furnish(room_api, b, r, kind, kinds)
		if found and kind then
			local rect = c.rects[#c.rects]
			for i = #c.rects, 1, -1 do
				if c.rects[i].id == found.id then rect = c.rects[i] break end
			end
			table.insert(c.containers, { kind = kind, x = found.x, z = found.z, rect = rect, id = found.id, searched = false, stand = v((r.x0 + r.x1) / 2, 0, (r.z0 + r.z1) / 2) })
		end
		for _ = 1, random.int(0, sleepers) do
			table.insert(c.inside, v(random.range(r.x0 + 0.7, r.x1 - 0.7), 0, random.range(r.z0 + 0.7, r.z1 - 0.7)))
		end
		-- Fog of war: the room is dark until somebody is in it or looks in
		-- through a door or a hole in the wall (main.luau lifts it).
		local h = look.height[2] + 0.15
		r.shroud = add(c, mat.spawn("cube", { pos = v((r.x0 + r.x1) / 2, h / 2, (r.z0 + r.z1) / 2 + ZC), size = v(r.x1 - r.x0 - 0.2, h, r.z1 - r.z0 - 0.2), color = SHROUD, roughness = 1, solid = false, tag = "deco" }))
	end
	table.insert(c.buildings, b)
	return b
end

-- A building you cannot enter: a solid block with a roof, its doors
-- boarded up and its windows dark or nailed shut. main.luau hides it (and
-- shows a low cut-away in its place) when it would hide a survivor from
-- the camera.
local function closed(c, x0, w, d, side, look, h, style)
	local zn = side * 4.6
	local zf = side * (4.6 + d)
	local z0, z1 = math.min(zn, zf), math.max(zn, zf)
	local x1 = x0 + w
	local color = look.walls[random.int(1, #look.walls)]
	local blk = { x0 = x0, x1 = x1, z0 = z0, z1 = z1, h = h, parts = {}, rooms = {}, doors = {} }
	local function part(id) table.insert(blk.parts, id) return id end
	local r = box(c, x0, z0, x1, z1, h, { color = color })
	part(r.id)
	local xm, zm = (x0 + x1) / 2, (z0 + z1) / 2
	if style == "pitched" then
		-- A gable roof of tiles, the ridge along the street, eaves over the
		-- walls; a wedge under it closes the gable ends; a chimney.
		local tile = TILES[random.int(1, #TILES)]
		local pitch = random.range(0.5, 0.62)
		local a = d / 2 + 0.35
		local eave = h - 0.12
		local ridge = eave + a * math.tan(pitch)
		local len = a / math.cos(pitch) + 0.05
		for _, sz in { 1, -1 } do
			part(deco(c, "cube", v(xm, eave + a / 2 * math.tan(pitch) + 0.04, zm + sz * a / 2), v(w + 0.5, 0.09, len), tile,
				{ roughness = 0.85, surface = mat.TILES, rot = v(sz * pitch, 0, 0) }))
		end
		local s2 = (ridge - 0.1 - h) * math.sqrt(2)
		if s2 > 0.2 then
			part(deco(c, "cube", v(xm, h, zm), v(w - 0.06, s2, s2), color, { roughness = 0.95, rot = v(math.pi / 4, 0, 0) }))
		end
		part(deco(c, "cube", v(xm, ridge + 0.03, zm), v(w + 0.5, 0.1, 0.18), tile, { roughness = 0.85, surface = mat.TILES }))
		if random.chance(0.7) then
			local cx = if random.chance(0.5) then x0 + random.range(0.8, 1.6) else x1 - random.range(0.8, 1.6)
			local cz = zm + random.range(-0.4, 0.8) * a
			local top = ridge + random.range(0.3, 0.7)
			part(deco(c, "cube", v(cx, (h + top) / 2, cz), v(0.55, top - h, 0.55), "#7a4a3a", { roughness = 0.95 }))
			part(deco(c, "cube", v(cx, top + 0.04, cz), v(0.68, 0.08, 0.68), "#5f5a55", { roughness = 0.95 }))
		end
		blk.h = ridge + 0.1
	else
		-- A flat roof: tar inside a parapet, and what stands on roofs:
		-- air conditioners, vents, a water tank, a stair hut, a skylight.
		local roof = ROOFS[random.int(1, #ROOFS)]
		part(deco(c, "cube", v(xm, h + 0.03, zm), v(w - 0.3, 0.06, d - 0.3), roof, { roughness = 0.95 }))
		local ph = random.range(0.35, 0.7)
		local function wall_cap(cx, cz, sx, sz) part(deco(c, "cube", v(cx, h + ph / 2, cz), v(sx, ph, sz), color, { roughness = 0.95 })) end
		wall_cap(xm, z0 + 0.12, w, 0.24)
		wall_cap(xm, z1 - 0.12, w, 0.24)
		wall_cap(x0 + 0.12, zm, 0.24, d - 0.48)
		wall_cap(x1 - 0.12, zm, 0.24, d - 0.48)
		-- Coping along the top of the parapet.
		part(deco(c, "cube", v(xm, h + ph + 0.03, z1 - 0.12), v(w + 0.06, 0.06, 0.32), "#7a7470", { roughness = 0.95 }))
		part(deco(c, "cube", v(xm, h + ph + 0.03, z0 + 0.12), v(w + 0.06, 0.06, 0.32), "#7a7470", { roughness = 0.95 }))
		local function spot(m) return random.range(x0 + 0.6 + m, x1 - 0.6 - m), random.range(z0 + 0.6 + m, z1 - 0.6 - m) end
		for _ = 1, random.int(1, 3) do
			local x, z = spot(0.6)
			local sx, sz = random.range(0.8, 1.3), random.range(0.6, 0.9)
			part(deco(c, "cube", v(x, h + 0.35, z), v(sx, 0.6, sz), "#7a7d82", { roughness = 0.6, metal = 0.5 }))
			part(deco(c, "sphere", v(x, h + 0.66, z), v(sz * 0.8, 0.05, sz * 0.8), "#3a3d42", { roughness = 0.6, metal = 0.6 }))
		end
		for _ = 1, random.int(1, 4) do
			local x, z = spot(0.3)
			local ht = random.range(0.4, 1.1)
			part(deco(c, "cube", v(x, h + ht / 2, z), v(0.16, ht, 0.16), "#5f6266", { roughness = 0.6, metal = 0.5 }))
			part(deco(c, "cube", v(x, h + ht + 0.03, z), v(0.28, 0.06, 0.28), "#5f6266", { roughness = 0.6, metal = 0.5 }))
		end
		if h > 7 and random.chance(0.5) then
			local x, z = spot(1.2)
			part(deco(c, "cube", v(x, h + 1.3, z), v(1.6, 1.4, 1.6), "#6a3a2a", { roughness = 0.9 }))
			for _, o in { v(-0.6, 0, -0.6), v(0.6, 0, -0.6), v(-0.6, 0, 0.6), v(0.6, 0, 0.6) } do
				part(deco(c, "cube", v(x + o.x, h + 0.3, z + o.z), v(0.1, 0.6, 0.1), "#3a3d42", { roughness = 0.6, metal = 0.6 }))
			end
		end
		if random.chance(0.6) then
			local x, z = spot(1.4)
			part(deco(c, "cube", v(x, h + 1.1, z), v(2.2, 2.2, 1.8), color, { roughness = 0.95 }))
			part(deco(c, "cube", v(x, h + 2.24, z), v(2.4, 0.08, 2.0), roof, { roughness = 0.95 }))
		end
		if random.chance(0.5) then
			local x, z = spot(1)
			part(deco(c, "cube", v(x, h + 0.2, z), v(1.4, 0.3, 1.0), "#5f6266", { roughness = 0.6, metal = 0.5 }))
			part(deco(c, "cube", v(x, h + 0.36, z), v(1.25, 0.04, 0.85), GLASS, { roughness = 0.1 }))
		end
		if random.chance(look.ruin * 2) then
			-- Part of the roof fell in: rubble on top.
			for _ = 1, 4 do
				local s = random.range(0.25, 0.6)
				part(deco(c, "cube", v(random.range(x0 + 0.8, x1 - 0.8), h + s * 0.3, random.range(z0 + 0.8, z1 - 0.8)), v(s * 1.6, s * 0.6, s), color,
					{ roughness = 0.95, rot = v(random.range(-0.4, 0.4), random.range(-math.pi, math.pi), 0) }))
			end
		end
		blk.h = h + ph
	end
	-- The face towards the path: a boarded-up door.
	local fz = zn - side * 0.02
	local dx = random.range(x0 + 1, x1 - 2)
	part(deco(c, "cube", v(dx + 0.6, 1.05, fz), v(1.2, 2.1, 0.06), PROP.dark))
	part(deco(c, "cube", v(dx + 0.6, 1.05, fz + side * 0.01), v(1.4, 2.25, 0.06), FRAME, { roughness = 0.8 }))
	for k = 0, 2 do
		part(deco(c, "cube", v(dx + 0.6, 0.5 + k * 0.6, fz - side * 0.04), v(1.5, 0.18, 0.05), if k % 2 == 0 then PROP.plank else PROP.board,
			{ rot = v(0, 0, random.range(-0.25, 0.25)) }))
	end
	-- Windows on the faces the camera sees (south, +z) and the street face:
	-- a frame, glass (or a dark hole), a cross bar, a sill; some boarded.
	-- A ledge at every floor and a plinth along the foot.
	local floors = math.max(1, math.floor(h / 3))
	local faces = if side < 0 then { { z = z1, out = 1, door = true } } else { { z = z0, out = -1, door = true, simple = true }, { z = z1, out = 1 } }
	for _, f in faces do
		local fzz = f.z + f.out * 0.005
		local function o(k) return fzz + f.out * k end
		if not f.simple then
			part(deco(c, "cube", v(xm, 0.25, o(0.02)), v(w + 0.04, 0.5, 0.04), "#5f5a55", { roughness = 0.95 }))
			for fl = 1, floors - 1 do
				part(deco(c, "cube", v(xm, fl * 3 + 0.05, o(0.05)), v(w + 0.1, 0.14, 0.1), "#7a7470", { roughness = 0.95 }))
			end
		end
		local shop = style == "city" and not f.simple
		if shop then
			-- A shopfront: a roller shutter, half down or all, and a fascia.
			local sx = random.range(x0 + 1, x1 - 4)
			local sw = math.min(random.range(3, 5), x1 - 0.6 - sx)
			local down = random.range(0.55, 1)
			part(deco(c, "cube", v(sx + sw / 2, 2.6 - 2.4 * down / 2, o(0.03)), v(sw, 2.4 * down, 0.05), "#5a5a5e", { roughness = 0.6, metal = 0.5 }))
			if down < 1 then part(deco(c, "cube", v(sx + sw / 2, (2.6 - 2.4 * down) / 2 + 0.1, o(0.01)), v(sw, 2.6 - 2.4 * down, 0.03), PROP.dark)) end
			part(deco(c, "cube", v(sx + sw / 2, 2.85, o(0.06)), v(sw + 0.4, 0.5, 0.1), SIGNS[random.int(1, #SIGNS)], { roughness = 0.7 }))
		end
		for fl = 0, floors - 1 do
			for x = x0 + 1, x1 - 1.4, 2.6 do
				local skip = (f.door and fl == 0 and math.abs(x - dx) < 1.6) or (shop and fl == 0)
				if not skip then
					local y = 1.7 + fl * 3
					local cx = x + 0.5
					if f.simple then
						part(deco(c, "cube", v(cx, y, o(0.02)), v(1, 1.1, 0.05), PROP.dark))
					else
						local broken = random.chance(0.35)
						part(deco(c, "cube", v(cx, y, o(0.0)), v(1.16, 1.26, 0.05), FRAME, { roughness = 0.8 }))
						part(deco(c, "cube", v(cx, y, o(0.015)), v(1.0, 1.1, 0.05), if broken then HOLE else GLASS, { roughness = 0.1 }))
						if not broken then
							part(deco(c, "cube", v(cx, y, o(0.04)), v(0.05, 1.1, 0.03), FRAME, { roughness = 0.8 }))
							part(deco(c, "cube", v(cx, y + 0.12, o(0.04)), v(1.0, 0.05, 0.03), FRAME, { roughness = 0.8 }))
						end
						part(deco(c, "cube", v(cx, y - 0.62, o(0.06)), v(1.3, 0.07, 0.14), "#7a7470", { roughness = 0.95 }))
						part(deco(c, "cube", v(cx, y + 0.66, o(0.03)), v(1.3, 0.12, 0.06), "#7a7470", { roughness = 0.95 }))
						if random.chance(0.3) then
							part(deco(c, "cube", v(cx, y + random.range(-0.3, 0.3), o(0.08)), v(1.25, 0.16, 0.04), PROP.plank, { rot = v(0, 0, random.range(-0.5, 0.5)) }))
						end
					end
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
	blk.cut = add(c, mat.spawn("cube", { pos = v((x0 + x1) / 2, 0.45, (z0 + z1) / 2 + ZC), size = v(w, 0.9, d), color = color, roughness = 0.95, solid = false, tag = "deco", visible = false }))
	table.insert(c.blocks, blk)
	table.insert(c.buildings, blk)
	return blk
end

-- A dead car: rusted or burnt, parked crooked, sometimes on its roof.
local function wreck(c, x, z)
	if not clear(c, x - 2.4, z - 1.3, x + 2.4, z + 1.3) then return end
	local yaw = random.range(-0.35, 0.35) + (if random.chance(0.5) then 0 else math.pi)
	local burnt = random.chance(0.35)
	local col = if burnt then BURNT.color else RUST[random.int(1, #RUST)]
	local flipped = not burnt and random.chance(0.15)
	local h = if flipped then 0.9 else 1.3
	-- Solid as a box around it; the look is turned.
	local r = box(c, x - 2.2, z - 1.1, x + 2.2, z + 1.1, h, { color = col, roughness = 0.9 })
	world.set(r.id, { visible = false })
	car_model(c, v(x, if flipped then 1.45 else 0, z), yaw, { color = col, rust = true, burnt = burnt, flipped = flipped, bare = random.chance(if burnt then 0.7 else 0.3) })
	if random.chance(0.4) then deco(c, "sphere", v(x + random.range(-3, 3), 0.12, z + random.range(-1.6, 1.6)), v(0.64, 0.24, 0.64), PROP.tyre) end
	-- Glass and bits around it.
	for _ = 1, random.int(2, 5) do
		deco(c, "cube", v(x + random.range(-2.4, 2.4), 0.01, z + random.range(-1.4, 1.4)), v(random.range(0.05, 0.2), 0.01, random.range(0.05, 0.15)), GLASS, { roughness = 0.1, rot = v(0, random.range(0, 3), 0) })
	end
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

-- What stops you at a closed-off street: sandbags or planks in town, a
-- fallen trunk out of it. axis "x": a line along x at z = at, from a to b;
-- "z": across the road at x = at.
local function blockade(c, biome, axis, at, a, b)
	if axis == "z" and (biome == "city" or biome == "suburb" or biome == "farm") then
		barricade(c, at, a, b)
		return
	end
	local town = biome == "city" or biome == "suburb"
	local look = if town then { color = PROP.sandbag } else { color = "#5a4632" }
	local h = if town then 0.7 else 0.55
	if axis == "x" then
		box(c, a, at - 0.35, b, at + 0.35, h, look)
		if town then
			for x = a + 0.3, b - 0.3, 0.55 do
				deco(c, "cube", v(x, 0.78, at + random.range(-0.05, 0.05)), v(0.5, 0.18, 0.5), PROP.sandbag, { rot = v(0, random.range(-0.3, 0.3), 0) })
			end
		end
	else
		box(c, at - 0.3, a, at + 0.3, b, h, look)
	end
end

-- A band beside a street that runs north or south, from z0 to z1: near is
-- the x of its edge by the street, side -1 west of it, 1 east.
local function side_fill(c, biome, look, near, side, z0, z1)
	if z1 - z0 < 2 then return end
	local function x(d) return near + side * d end
	if biome == "city" then
		-- Shells of blocks with gaps of rubble between.
		local z = z0
		while z < z1 - 1 do
			local len = math.min(random.range(6, 12), z1 - z)
			if random.chance(0.8) then
				box(c, x(0.4), z + 0.2, x(random.range(8, 12)), z + len - 0.2, random.range(5, 11), { color = look.walls[random.int(1, #look.walls)] })
			else
				rubble(c, x(2), z + len / 2, 1.4, { color = DEBRIS[random.int(1, #DEBRIS)] }, random.int(4, 8))
				box(c, x(2.5), z + 0.2, x(12), z + len - 0.2, random.range(0.6, 1.4), { color = DEBRIS[random.int(1, #DEBRIS)] })
			end
			z += len
		end
	elseif biome == "suburb" then
		-- Hedges along the pavement, sheds and gardens behind.
		box(c, x(0.4), z0 + 0.3, x(0.8), z1 - 0.3, 0.9, { color = PROP.hedge })
		local z = z0
		while z < z1 - 4 do
			local len = math.min(random.range(7, 11), z1 - z)
			if random.chance(0.5) then
				box(c, x(3), z + 1, x(random.range(7, 9)), z + len - 1, random.range(2.6, 3.4), { color = look.walls[random.int(1, #look.walls)] })
			end
			tree(c, x(random.range(9, 12)), z + len / 2)
			z += len
		end
	elseif biome == "forest" then
		for _ = 1, math.floor((z1 - z0) / 1.6) do tree(c, x(random.range(1.2, 12)), random.range(z0, z1), true) end
		for _ = 1, math.floor((z1 - z0) / 8) do rock(c, x(random.range(1.5, 10)), random.range(z0, z1)) end
		undergrowth(c, near + side * 1, near + side * 12, z0, z1, math.floor((z1 - z0) / 2.5))
	else -- farm
		box(c, x(0.4), z0, x(0.52), z1, 0.7, { color = "#8b6b4a" })
		local crop = CROP[random.int(1, #CROP)]
		for xd = 2, 11, 1.4 do flat(c, x(xd), z0 + 0.5, x(xd + 0.6), z1 - 0.5, 0.03, crop) end
		if random.chance(0.4) then tree(c, x(random.range(4, 11)), random.range(z0, z1)) end
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
			closed(c, bx, w, random.range(6, 8), side, look, random.range(3.2, 4.2), "pitched")
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
			undergrowth(c, x0, x1, side * 3.4, side * 13.5, random.int(5, 9))
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
			closed(c, bx, w, random.range(7, 9.4), side, look, random.range(6, 12), "city")
		end
	else -- farm
		local r = random.float()
		if r < 0.4 then
			local w = math.min(random.range(9, 12), span - 1)
			local bx = random.range(x0 + 0.5, x1 - w - 0.5)
			if random.chance(0.5) then
				building(c, bx, w, random.range(8, 11), 3, 2, side, look, { "barn", "barn", "kitchen" }, 2)
			else
				closed(c, bx, w, random.range(7, 9), side, look, random.range(4.2, 5.5), "pitched")
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
	-- A foot, the arm out over the road and the head round the bulb.
	deco(c, "cube", v(x, 0.3, z), v(0.3, 0.6, 0.3), "#3a3d42", { roughness = 0.6, metal = 0.6 })
	deco(c, "cube", v(x, 4.22, z - side * 0.3), v(0.07, 0.07, 0.6), "#3a3d42", { roughness = 0.6, metal = 0.6 })
	deco(c, "cube", v(x, 4.24, z - side * 0.55), v(0.46, 0.1, 0.26), "#3a3d42", { roughness = 0.6, metal = 0.6 })
	local lit = random.chance(if city then 0.25 else 0.45)
	local bulb = deco(c, "cube", v(x, 4.17, z - side * 0.55), v(0.38, 0.05, 0.2), if lit then "#fff0c8" else "#4a4640", if lit then { emissive = 1.5 } else nil)
	if lit then table.insert(c.lamps, { pos = v(x, 4, z - side * 0.5), flicker = random.chance(0.3), seed = random.range(0, 10), bulb = bulb }) end
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
	add(c, mat.spawn("cube", { pos = v((x0 + x1) / 2, -0.5, (lo + hi) / 2), size = v(M.CHUNK, 1, hi - lo + HZ * 2 + 30), color = look.ground, roughness = 1, solid = false, tag = "ground" }))
	if turn then
		-- The street that turns: from the old centre to the new one. Now
		-- and then it is a crossroads: the street goes on past the corner
		-- and the old road on past the turn, both closed off.
		local half = if biome == "city" then 3.2 else 2.6
		local dir = if zc > zc_prev then 1 else -1
		local far = zc + dir * half
		local cross = random.chance(if biome == "forest" then 0.3 else 0.55)
		local town = biome == "city" or biome == "suburb"
		local n0 = #c.containers
		flat(c, x0, lo - half, x0 + M.TURN, hi + half, 0.004, look.path)
		if cross then
			flat(c, x0, far, x0 + M.TURN, far + dir * 12, 0.004, look.path)
			flat(c, x0 + M.TURN, zc_prev - half, x0 + M.TURN + 13, zc_prev + half, 0.004, look.path)
			blockade(c, biome, "x", far + dir * 3, x0 - 0.4, x0 + M.TURN + 0.4)
			blockade(c, biome, "z", x0 + M.TURN + 0.9, zc_prev - half - 0.3, zc_prev + half + 0.3)
			if biome ~= "forest" then
				-- A pile of dead cars behind each, for good.
				wreck(c, x0 + 2.2, far + dir * 8)
				wreck(c, x0 + M.TURN - 2.2, far + dir * 9.5)
				wreck(c, x0 + M.TURN + 6, zc_prev + random.range(-1, 1))
			else
				for _ = 1, 4 do tree(c, x0 + random.range(0.5, M.TURN - 0.5), far + dir * random.range(6, 11), true) end
			end
		end
		if town then
			-- The kerb on the far side, up to where the new path leaves
			-- (cut where the old road goes on).
			if cross then
				flat(c, x0 + M.TURN, zc_prev + dir * half, x0 + M.TURN + 1.2, zc - dir * half, 0.008, look.edge)
				flat(c, x0 + M.TURN, far, x0 + M.TURN + 1.2, far + dir * 12, 0.008, look.edge)
			else
				flat(c, x0 + M.TURN, zc_prev - dir * half, x0 + M.TURN + 1.2, zc - dir * half, 0.008, look.edge)
			end
			-- And on the near side, past where the old path comes in.
			flat(c, x0 - 1.2, zc_prev + dir * half, x0, far + (if cross then dir * 12 else 0), 0.008, look.edge)
			-- Unmarked asphalt reads as a hole at night: a centre line,
			-- patches, and a lit lamp on the corner.
			local mx = x0 + M.TURN / 2
			local dash = if biome == "city" then "#c9b25a" else "#d8d0a0"
			for z = lo - half + 1, hi + half - 2, 4 do
				if math.abs(z + 1 - zc) > half and math.abs(z + 1 - zc_prev) > half then
					flat(c, mx - 0.07, z, mx + 0.07, z + 2, 0.009, dash)
				end
			end
			for _ = 1, 3 do
				local px, pz = random.range(x0 + 0.5, x0 + M.TURN - 2), random.range(lo, hi)
				flat(c, px, pz, px + random.range(1, 2.2), pz + random.range(0.8, 2), 0.006, "#3a3a3c")
			end
			for lz = lo + 8, hi - 4, 16 do
				local lx = x0 + M.TURN + 0.6
				box(c, lx - 0.09, lz - 0.09, lx + 0.09, lz + 0.09, 4.2, { color = "#3a3d42", roughness = 0.6, metal = 0.6 })
				local bulb = deco(c, "cube", v(lx - 0.5, 4.15, lz), v(0.3, 0.14, 0.5), "#fff0c8", { emissive = 1.5 })
				-- shift() below moves recorded z by zc; this one is already placed.
				table.insert(c.lamps, { pos = v(lx - 0.5, 4, lz - zc), flicker = false, seed = random.range(0, 10), bulb = bulb })
			end
		end
		-- Both sides of the street, where neither road's lots are: ground
		-- to the west, and houses, ruins, trees or fields along it.
		add(c, mat.spawn("cube", { pos = v(x0 - 7, -0.51, (lo + hi) / 2), size = v(14, 1, hi - lo + HZ * 2), color = look.ground, roughness = 1, solid = false, tag = "ground" }))
		local ea, eb = zc_prev - dir * HZ, zc - dir * HZ
		if cross then
			side_fill(c, biome, look, x0 + M.TURN + 1.2, 1, math.min(ea, zc_prev - dir * (half + 0.5)), math.max(ea, zc_prev - dir * (half + 0.5)))
			side_fill(c, biome, look, x0 + M.TURN + 1.2, 1, math.min(eb, zc_prev + dir * (half + 0.5)), math.max(eb, zc_prev + dir * (half + 0.5)))
		else
			side_fill(c, biome, look, x0 + M.TURN + 1.2, 1, math.min(ea, eb), math.max(ea, eb))
		end
		local wa, wb = zc_prev + dir * HZ, zc + dir * HZ
		side_fill(c, biome, look, x0 - 1.2, -1, math.min(wa, wb), math.max(wa, wb))
		-- Somewhere for the dead to come from, and searched cars stay put.
		for _ = 1, 2 do table.insert(c.outside, v(x0 + random.range(1, M.TURN - 1), 0, random.range(lo, hi) - zc)) end
		for i = n0 + 1, #c.containers do c.containers[i].z -= zc end
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
	-- Kerbstones along the pavements, a drain now and then, a manhole.
	local kerb = if biome == "city" then 3.2 elseif biome == "suburb" then 2.6 else nil
	if kerb then
		for _, sd in { -1, 1 } do
			deco(c, "cube", v((x0 + x1) / 2, 0.05, sd * (kerb + 0.07)), v(x1 - x0, 0.1, 0.16), KERB, { roughness = 0.95 })
			for x = x0 + random.range(1, 5), x1 - 1, random.range(7, 10) do
				deco(c, "cube", v(x, 0.008, sd * (kerb - 0.25)), v(0.45, 0.012, 0.3), DRAIN, { roughness = 0.6, metal = 0.5 })
			end
		end
		if random.chance(0.6) then
			deco(c, "sphere", v(random.range(x0 + 2, x1 - 2), 0.0, random.range(-1.6, 1.6)), v(0.75, 0.02, 0.75), DRAIN, { roughness = 0.6, metal = 0.5 })
		end
	end
	-- Litter on the road and the verges: paper, cans, bottles, leaves.
	if biome ~= "forest" or random.chance(0.3) then
		for _ = 1, random.int(6, if biome == "city" then 16 else 10) do
			local x, z = random.range(x0, x1), random.range(-4.4, 4.4)
			local roll = random.float()
			if roll < 0.35 then
				deco(c, "cube", v(x, 0.012, z), v(random.range(0.15, 0.3), 0.004, random.range(0.12, 0.22)), "#d8d0a0", { roughness = 0.95, rot = v(0, random.range(0, 3), 0) })
			elseif roll < 0.55 then
				deco(c, "cube", v(x, 0.035, z), v(0.12, 0.065, 0.065), CANS[random.int(1, #CANS)], { roughness = 0.6, metal = 0.5, rot = v(0, random.range(0, 3), 0) })
			elseif roll < 0.65 then
				deco(c, "cube", v(x, 0.04, z), v(0.24, 0.07, 0.07), BOTTLE, { roughness = 0.1, rot = v(0, random.range(0, 3), 0) })
			else
				for _ = 1, random.int(3, 7) do
					deco(c, "cube", v(x + random.range(-0.5, 0.5), 0.01, z + random.range(-0.5, 0.5)), v(0.08, 0.004, 0.06), FALLEN[random.int(1, #FALLEN)], { roughness = 0.9, rot = v(0, random.range(0, 3), 0) })
				end
			end
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
