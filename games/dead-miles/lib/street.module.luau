-- A generated stop: one street of a small town. A road runs east to west
-- through the middle with the group's car at the west end; houses with an
-- open door to the road line both sides, with cupboards and closets to
-- search, parked cars on the road and trees in the yards. Walls are cut
-- low so the high camera sees into the rooms.
--
-- build(stop, rng) spawns it all and returns what the rules need:
-- rects (solid boxes in the ground plane for collision and shots),
-- containers, weapons lying around, places to put zombies, the car.

local v = vector.create

local M = {}

M.HALF_X, M.HALF_Z = 38, 23 -- the lot's half size in metres
local WALL_H, WALL_T = 1.35, 0.25
local ROAD_Z = 4 -- the road is 8 m wide
local HOUSE_Z = 6.8 -- houses start this far from the middle of the road

local WALL_COLORS = { "#c9b79c", "#b7c2b0", "#d6c4a8", "#a9b4bf", "#c7a88f", "#bfb6a6" }
local FLOOR_COLORS = { "#7a5a3e", "#8b6b4a", "#6f6252", "#9a8f80", "#6b4e37" }
local CAR_COLORS = { "#6b2b2b", "#2f4d6b", "#5d6650", "#8a8478", "#3a3a40", "#7a5a2a" }
M.GRASS = "#55623f"
M.ROAD = "#2c2c2e"
M.WALK = "#8c8984"

-- Every look the street uses, warmed in init.
function M.warm()
	for _, c in WALL_COLORS do world.warm("cube", { color = c, roughness = 0.95 }) end
	for _, c in FLOOR_COLORS do world.warm("cube", { color = c, roughness = 0.9 }) end
	for _, c in CAR_COLORS do
		world.warm("cube", { color = c, roughness = 0.4, metal = 0.3 })
	end
	for _, c in { M.GRASS, M.ROAD, M.WALK, "#d8d0a0", "#4a3a2a", "#3c5a2c", "#2f4a26", "#5a4632", "#202024", "#9fb4c8", "#e8e8f0", "#1d3a8a" } do
		world.warm("cube", { color = c, roughness = 0.9 })
	end
	world.warm("sphere", { color = "#3c5a2c", roughness = 0.9 })
	world.warm("sphere", { color = "#2f4a26", roughness = 0.9 })
	world.warm("cube", { color = "#ffe9b0", emissive = 0.6 })
	world.warm("cube", { color = "#ff2a2a", emissive = 0.5 })
end

local function box(rects, x0, z0, x1, z1, h, look, tag)
	if x1 < x0 then x0, x1 = x1, x0 end
	if z1 < z0 then z0, z1 = z1, z0 end
	local id = world.spawn("cube", {
		pos = v((x0 + x1) / 2, h / 2, (z0 + z1) / 2),
		size = v(x1 - x0, h, z1 - z0),
		color = look.color,
		roughness = look.roughness or 0.9,
		metal = look.metal,
		tag = tag or "wall",
	})
	local r = { x0 = x0, z0 = z0, x1 = x1, z1 = z1, h = h, id = id }
	table.insert(rects, r)
	return r
end

local function flat(x0, z0, x1, z1, y, color, tag)
	return world.spawn("cube", {
		pos = v((x0 + x1) / 2, y, (z0 + z1) / 2),
		size = v(math.abs(x1 - x0), 0.02, math.abs(z1 - z0)),
		color = color,
		roughness = 0.95,
		solid = false,
		tag = tag or "ground",
	})
end

-- A wall along x (from x0 to x1 at z) with an optional gap [g0, g1].
local function wall_x(rects, x0, x1, z, gap, look)
	if gap then
		if gap[1] - x0 > 0.05 then box(rects, x0, z - WALL_T / 2, gap[1], z + WALL_T / 2, WALL_H, look) end
		if x1 - gap[2] > 0.05 then box(rects, gap[2], z - WALL_T / 2, x1, z + WALL_T / 2, WALL_H, look) end
	else
		box(rects, x0, z - WALL_T / 2, x1, z + WALL_T / 2, WALL_H, look)
	end
end

local function wall_z(rects, z0, z1, x, gap, look)
	if gap then
		if gap[1] - z0 > 0.05 then box(rects, x - WALL_T / 2, z0, x + WALL_T / 2, gap[1], WALL_H, look) end
		if z1 - gap[2] > 0.05 then box(rects, x - WALL_T / 2, gap[2], x + WALL_T / 2, z1, WALL_H, look) end
	else
		box(rects, x - WALL_T / 2, z0, x + WALL_T / 2, z1, WALL_H, look)
	end
end

-- A searchable box against a wall. kind decides the loot table.
local function container(out, rects, kind, x, z, w, d, h, color)
	local r = box(rects, x - w / 2, z - d / 2, x + w / 2, z + d / 2, h, { color = color, roughness = 0.8 })
	table.insert(out.containers, { kind = kind, x = x, z = z, rect = r, id = r.id, searched = false, color = color })
end

local function house(out, x0, x1, side)
	local rects = out.rects
	local look = { color = WALL_COLORS[random.int(1, #WALL_COLORS)], roughness = 0.95 }
	local d = random.range(7, 9.5)
	-- side -1: north of the road (z < 0), the door faces +z; side 1: south.
	local zn = side * HOUSE_Z -- the wall towards the road
	local zf = side * (HOUSE_Z + d) -- the back wall
	local za, zb = math.min(zn, zf), math.max(zn, zf)
	flat(x0, za, x1, zb, 0.012, FLOOR_COLORS[random.int(1, #FLOOR_COLORS)], "floor")
	local door = random.range(x0 + 1.2, x1 - 2.8)
	wall_x(rects, x0, x1, zn, { door, door + 1.7 }, look)
	local back = if random.chance(0.5) then random.range(x0 + 1, x1 - 2.2) else nil
	wall_x(rects, x0, x1, zf, back and { back, back + 1.2 }, look)
	wall_z(rects, za, zb, x0, nil, look)
	wall_z(rects, za, zb, x1, nil, look)
	-- One inner wall splits it into a front room and a back room, with a gap.
	local zi = (zn + zf) / 2
	local gap = random.range(x0 + 1, x1 - 2.4)
	wall_x(rects, x0, x1, zi, { gap, gap + 1.3 }, look)
	-- Cupboards and closets along the side walls.
	local inset = 0.55
	container(out, rects, "kitchen", x0 + inset, (zn + zi) / 2, 0.6, 1.4, 0.95, "#8b6b4a")
	container(out, rects, "closet", x1 - inset, (zi + zf) / 2, 0.6, 1.6, 1.3, "#5a4632")
	if random.chance(0.5) then
		container(out, rects, if random.chance(0.5) then "kitchen" else "closet", x1 - inset, (zn + zi) / 2, 0.6, 1.2, 0.95, "#6b4e37")
	end
	-- Zombies asleep inside.
	for _ = 1, random.int(1, 3) do
		table.insert(out.inside, v(random.range(x0 + 1, x1 - 1), 0, random.range(za + 1, zb - 1)))
	end
	-- Waypoints for walkers: outside and inside the front door, and the
	-- gap in the inner wall. zi splits the front room from the back room.
	table.insert(out.houses, {
		x0 = x0, x1 = x1, z0 = za, z1 = zb, zi = zi, side = side, zn = zn,
		door_out = v(door + 0.85, 0, zn - side * 0.9),
		door_in = v(door + 0.85, 0, zn + side * 0.9),
		gap_front = v(gap + 0.65, 0, zi - side * 0.7),
		gap_back = v(gap + 0.65, 0, zi + side * 0.7),
	})
end

local function shed(out, x0, x1, side)
	local rects = out.rects
	local look = { color = "#6f6252", roughness = 0.95 }
	local w = random.range(3, 4)
	local cx = random.range(x0 + w / 2 + 1, x1 - w / 2 - 1)
	local zn = side * (HOUSE_Z + 3)
	local zf = side * (HOUSE_Z + 6.5)
	local za, zb = math.min(zn, zf), math.max(zn, zf)
	flat(cx - w / 2, za, cx + w / 2, zb, 0.012, "#6f6252", "floor")
	wall_x(rects, cx - w / 2, cx + w / 2, zn, { cx - 0.6, cx + 0.6 }, look)
	wall_x(rects, cx - w / 2, cx + w / 2, zf, nil, look)
	wall_z(rects, za, zb, cx - w / 2, nil, look)
	wall_z(rects, za, zb, cx + w / 2, nil, look)
	container(out, rects, "shed", cx, (zn + zf) / 2 + side * 1, w - 1, 0.6, 1.0, "#4a3a2a")
end

local function tree(out, x, z)
	local r = box(out.rects, x - 0.22, z - 0.22, x + 0.22, z + 0.22, 3.2, { color = "#5a4632" }, "wall")
	local c = if random.chance(0.5) then "#3c5a2c" else "#2f4a26"
	world.spawn("sphere", { pos = v(x, 3.4 + random.range(0, 0.6), z), size = random.range(1.8, 2.6), color = c, roughness = 0.9, solid = false, tag = "deco" })
	return r
end

local function parked_car(out, x, z, police)
	local color = if police then "#e8e8f0" else CAR_COLORS[random.int(1, #CAR_COLORS)]
	local r = box(out.rects, x - 2.1, z - 0.92, x + 2.1, z + 0.92, 1.15, { color = color, roughness = 0.4, metal = 0.3 })
	world.spawn("cube", { pos = v(x + 0.2, 1.45, z), size = v(2.1, 0.6, 1.6), color = "#202024", roughness = 0.3, solid = false, tag = "deco" })
	if police then
		world.spawn("cube", { pos = v(x + 0.2, 1.8, z), size = v(0.3, 0.12, 1.2), color = "#1d3a8a", roughness = 0.5, solid = false, tag = "deco" })
	end
	table.insert(out.containers, { kind = if police then "police" else "car", x = x, z = z, rect = r, id = r.id, searched = false, color = color })
end

function M.build(stop)
	local out = { rects = {}, containers = {}, weapons = {}, inside = {}, outside = {}, houses = {} }
	local HX, HZ = M.HALF_X, M.HALF_Z
	-- Grass, road, pavements and the dashed centre line.
	world.spawn("cube", { pos = v(0, -0.5, 0), size = v(HX * 2 + 40, 1, HZ * 2 + 40), color = M.GRASS, roughness = 1, solid = false, tag = "ground" })
	flat(-HX - 20, -ROAD_Z, HX + 20, ROAD_Z, 0.004, M.ROAD)
	flat(-HX - 20, -ROAD_Z - 1.6, HX + 20, -ROAD_Z, 0.008, M.WALK)
	flat(-HX - 20, ROAD_Z, HX + 20, ROAD_Z + 1.6, 0.008, M.WALK)
	for x = -HX - 18, HX + 18, 4 do
		flat(x, -0.08, x + 2, 0.08, 0.009, "#d8d0a0")
	end

	-- The group's car waits at the west end.
	out.car = v(-HX + 6, 0, 1.8)

	-- Lots along both sides.
	for _, side in { -1, 1 } do
		local x = -HX + 11
		while x < HX - 6 do
			local w = random.range(11, 15)
			local x1 = math.min(x + w, HX - 1)
			if x1 - x < 8 then break end
			local roll = random.float()
			if roll < 0.75 then
				local hw = math.min(random.range(7.5, 10.5), x1 - x - 1.2)
				local hx = random.range(x + 0.5, x1 - hw - 0.5)
				house(out, hx, hx + hw, side)
				if random.chance(0.6) then
					tree(out, random.range(x + 1, x1 - 1), side * random.range(HOUSE_Z + 10.5, M.HALF_Z - 1))
				end
			else
				shed(out, x, x1, side)
				for _ = 1, random.int(1, 3) do
					tree(out, random.range(x + 1, x1 - 1), side * random.range(HOUSE_Z + 0.5, HOUSE_Z + 2.5))
				end
				table.insert(out.weapons, { kind = if random.chance(0.6) then "pipe" else "bat", pos = v(random.range(x + 1, x1 - 1), 0, side * (HOUSE_Z + 1.5)) })
			end
			-- Somewhere in the yards zombies stand around.
			table.insert(out.outside, v(random.range(x, x1), 0, side * random.range(HOUSE_Z, HZ - 1)))
			x = x1 + random.range(0.5, 1.5)
		end
	end

	-- Parked and crashed cars on the road; sometimes a police car.
	local police = stop >= 2 and random.chance(0.35)
	local cx = -HX + 14
	for i = 1, random.int(2, 4) do
		cx += random.range(8, 16)
		if cx > HX - 4 then break end
		parked_car(out, cx, (if random.chance(0.5) then -1 else 1) * random.range(1.6, 2.4), police and i == 1)
	end
	-- Zombies on the road and pavements too.
	for _ = 1, 6 do
		table.insert(out.outside, v(random.range(-HX + 16, HX - 2), 0, random.range(-ROAD_Z - 1, ROAD_Z + 1)))
	end
	-- A pipe lies somewhere on the road.
	table.insert(out.weapons, { kind = "pipe", pos = v(random.range(-HX + 14, -HX + 24), 0, random.range(-3, 3)) })
	return out
end

-- The group's car: a station wagon, with its headlights for the night.
function M.car(pos)
	local root = world.spawn("cube", { pos = pos + v(0, 0.6, 0), size = v(4.4, 1.15, 2), color = "#8a3a22", roughness = 0.45, metal = 0.3, tag = "car" })
	world.spawn("cube", { pos = v(-0.3, 0.85, 0), size = v(2.7, 0.6, 1.7), color = "#202024", roughness = 0.3, solid = false, tag = "deco" }, root)
	world.spawn("cube", { pos = v(-0.4, 1.2, 0), size = v(2.2, 0.08, 1.4), color = "#4a3a2a", roughness = 0.9, solid = false, tag = "deco" }, root)
	world.spawn("cube", { pos = v(2.21, 0.05, 0.6), size = v(0.05, 0.18, 0.3), color = "#ffe9b0", emissive = 0.6, solid = false, tag = "deco" }, root)
	world.spawn("cube", { pos = v(2.21, 0.05, -0.6), size = v(0.05, 0.18, 0.3), color = "#ffe9b0", emissive = 0.6, solid = false, tag = "deco" }, root)
	world.spawn("cube", { pos = v(-2.21, 0.05, 0.65), size = v(0.05, 0.15, 0.25), color = "#ff2a2a", emissive = 0.5, solid = false, tag = "deco" }, root)
	world.spawn("cube", { pos = v(-2.21, 0.05, -0.65), size = v(0.05, 0.15, 0.25), color = "#ff2a2a", emissive = 0.5, solid = false, tag = "deco" }, root)
	return root, { x0 = pos.x - 2.2, z0 = pos.z - 1, x1 = pos.x + 2.2, z1 = pos.z + 1, h = 1.2, id = root }
end

return M
