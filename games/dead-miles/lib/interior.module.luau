-- What is inside the rooms of the open ruins: a kitchen, a bedroom, a
-- living room, a shop, an office, a pharmacy, a police station, a cabin,
-- a barn. Big things stand against the walls, clear of the doors and of
-- the middle of the room (bots and the dead walk through the middle from
-- door to door); small things lie about loose.
--
-- land.module.luau calls M.furnish with its own builders (`box` solid and
-- drawn, `deco` drawn only, `hidden` solid and not drawn, `container`) in
-- chunk-local coordinates.

local v = vector.create

local M = {}

-- Colours (each has a material in lib/mat and a warm in M.warm).
local C = {
	cabinet = "#7a6a58", top = "#9a968e", door = "#2a2622", steel = "#7a7d82", dark_steel = "#5f6266",
	white = "#c8c8c0", stove = "#303034", wood = "#5a4632", light_wood = "#8b6b4a", mattress = "#c8c0ae",
	pillow = "#d8d2c4", screen = "#161618", paper = "#d8d0a0", hay = "#c9b25a", crate = "#8b6b4a",
	blankets = { "#4a5a6a", "#6a4040", "#4a5a40", "#5a4a5a" },
	sofas = { "#4a5a6a", "#6a4040", "#5a5040" },
	rugs = { "#6a3a34", "#3a4a5a", "#5a4a32" },
	goods = { "#8a2420", "#2a4a7a", "#c9b25a", "#3a5a3a", "#d8d2c4" },
	floor = { kitchen = "#a8a49a", shop = "#8a8a84", pharmacy = "#b0aca2", police = "#8a8a84" },
}
M.COLORS = C

-- Which room a container kind makes, and what else a house's rooms are.
local ROOM_OF = { kitchen = "kitchen", closet = "bedroom", cabin = "cabin", barn = "barn", shop = "shop", office = "office", pharmacy = "pharmacy", police = "police" }

-- A slot against one wall of room r: an item `w` wide along the wall and
-- `d` deep, its centre and which way its front faces (into the room).
type Slot = { x: number, z: number, along_x: boolean, fx: number, fz: number, w: number, d: number }

local INSET = 0.16 -- half a wall and a little

local function rect_of(s: Slot)
	local hw, hd = if s.along_x then s.w / 2 else s.d / 2, if s.along_x then s.d / 2 else s.w / 2
	return s.x - hw, s.z - hd, s.x + hw, s.z + hd
end

-- Is the ground under s free: inside the room, clear of the doors and of
-- what is there already, and out of the middle?
local function free(b, r, taken, s: Slot)
	local x0, z0, x1, z1 = rect_of(s)
	if x0 < r.x0 + INSET - 0.01 or x1 > r.x1 - INSET + 0.01 or z0 < r.z0 + INSET - 0.01 or z1 > r.z1 - INSET + 0.01 then return false end
	for _, d in b.doors do
		local m = (d.pa + d.pb) / 2
		local cx, cz = math.clamp(m.x, x0, x1), math.clamp(m.z, z0, z1)
		if (cx - m.x) ^ 2 + (cz - m.z) ^ 2 < 1.2 * 1.2 then return false end
	end
	for _, t in taken do
		if x0 < t[3] + 0.1 and x1 > t[1] - 0.1 and z0 < t[4] + 0.1 and z1 > t[2] - 0.1 then return false end
	end
	-- The middle stays open (the room's stand point, where bots go).
	local mx, mz = (r.x0 + r.x1) / 2, (r.z0 + r.z1) / 2
	local cx, cz = math.clamp(mx, x0, x1), math.clamp(mz, z0, z1)
	if (cx - mx) ^ 2 + (cz - mz) ^ 2 < 0.8 * 0.8 then return false end
	return true
end

-- Somewhere against a wall for an item w by d, or nil.
local function place(b, r, taken, w: number, d: number): Slot?
	for _ = 1, 12 do
		local wall = random.int(1, 4)
		local s: Slot
		if wall <= 2 then
			local z = if wall == 1 then r.z0 + INSET + d / 2 else r.z1 - INSET - d / 2
			s = { x = random.range(r.x0 + INSET + w / 2, math.max(r.x0 + INSET + w / 2, r.x1 - INSET - w / 2)), z = z, along_x = true, fx = 0, fz = if wall == 1 then 1 else -1, w = w, d = d }
		else
			local x = if wall == 3 then r.x0 + INSET + d / 2 else r.x1 - INSET - d / 2
			s = { x = x, z = random.range(r.z0 + INSET + w / 2, math.max(r.z0 + INSET + w / 2, r.z1 - INSET - w / 2)), along_x = false, fx = if wall == 3 then 1 else -1, fz = 0, w = w, d = d }
		end
		if free(b, r, taken, s) then
			local x0, z0, x1, z1 = rect_of(s)
			table.insert(taken, { x0, z0, x1, z1 })
			return s
		end
	end
	return nil
end

-- A point in the item's frame: `a` along the wall, `f` out of it (towards
-- the front), `y` up; and a size the same way.
local function at(s: Slot, a: number, f: number, y: number)
	if s.along_x then return v(s.x + a, y, s.z + s.fz * f) end
	return v(s.x + s.fx * f, y, s.z + a)
end
local function sz(s: Slot, a: number, y: number, f: number)
	if s.along_x then return v(a, y, f) end
	return v(f, y, a)
end

-- The builders get (api, s): api.box(x0, z0, x1, z1, h, look) solid and
-- drawn; api.deco(shape, pos, size, color, extra); api.hidden(x0, z0, x1,
-- z1, h) solid only. Each returns the id of its solid part or nil.

local function body(api, s: Slot, h: number, color: string, extra)
	local x0, z0, x1, z1 = rect_of(s)
	return api.box(x0, z0, x1, z1, h, { color = color, roughness = (extra and extra.roughness) or 0.85, metal = extra and extra.metal })
end

local ITEMS = {}

-- A run of kitchen units: worktop, doors, a sink.
function ITEMS.counter(api, s: Slot)
	local id = body(api, s, 0.86, C.cabinet)
	api.deco("cube", at(s, 0, -0.02, 0.89), sz(s, s.w + 0.04, 0.05, s.d + 0.04), C.top, { roughness = 0.6 })
	for a = -s.w / 2 + 0.3, s.w / 2 - 0.25, 0.6 do
		api.deco("cube", at(s, a, s.d / 2 + 0.005, 0.45), sz(s, 0.02, 0.7, 0.015), C.door, { roughness = 0.9 })
	end
	api.deco("cube", at(s, random.range(-s.w / 4, s.w / 4), 0, 0.9), sz(s, 0.5, 0.03, 0.36), C.dark_steel, { roughness = 0.35, metal = 0.7 })
	return id
end

function ITEMS.fridge(api, s: Slot)
	local id = body(api, s, 1.75, C.white, { roughness = 0.5 })
	api.deco("cube", at(s, -s.w / 2 + 0.08, s.d / 2 + 0.02, 1.1), sz(s, 0.03, 0.4, 0.03), C.dark_steel, { roughness = 0.35, metal = 0.7 })
	api.deco("cube", at(s, 0, s.d / 2 + 0.005, 1.25), sz(s, s.w - 0.02, 0.012, 0.012), C.door, { roughness = 0.9 })
	return id
end

function ITEMS.stove(api, s: Slot)
	local id = body(api, s, 0.88, C.stove, { roughness = 0.5, metal = 0.5 })
	for _, o in { { -0.13, -0.1 }, { 0.13, -0.1 }, { -0.13, 0.12 }, { 0.13, 0.12 } } do
		api.deco("sphere", at(s, o[1], o[2], 0.885), v(0.16, 0.012, 0.16), C.door, { roughness = 0.9 })
	end
	return id
end

-- A wardrobe: two doors with a handle each.
function ITEMS.wardrobe(api, s: Slot)
	local id = body(api, s, 1.9, C.wood)
	for _, a in { -s.w / 4, s.w / 4 } do
		api.deco("cube", at(s, a, s.d / 2 + 0.01, 0.95), sz(s, s.w / 2 - 0.06, 1.75, 0.02), C.light_wood, { roughness = 0.85 })
		api.deco("cube", at(s, a - math.sign(a) * (s.w / 4 - 0.08), s.d / 2 + 0.03, 1.0), sz(s, 0.03, 0.14, 0.03), C.dark_steel, { roughness = 0.35, metal = 0.7 })
	end
	return id
end

-- A bed with its head to the wall: frame, mattress, a blanket half off, a pillow.
function ITEMS.bed(api, s: Slot)
	local id = body(api, s, 0.32, C.wood)
	api.deco("cube", at(s, 0, -s.d / 2 + 0.04, 0.45), sz(s, s.w, 0.9, 0.08), C.wood, { roughness = 0.85 })
	api.deco("cube", at(s, 0, 0.02, 0.4), sz(s, s.w - 0.1, 0.16, s.d - 0.14), C.mattress, { roughness = 0.95 })
	api.deco("cube", at(s, random.range(-0.15, 0.15), 0.35, 0.5), sz(s, s.w - 0.05, 0.05, s.d * 0.55), C.blankets[random.int(1, #C.blankets)],
		{ roughness = 0.95, rot = v(0, random.range(-0.25, 0.25), 0) })
	api.deco("cube", at(s, 0, -s.d / 2 + 0.3, 0.52), sz(s, s.w * 0.6, 0.1, 0.32), C.pillow, { roughness = 0.95 })
	return id
end

function ITEMS.nightstand(api, s: Slot)
	local id = body(api, s, 0.55, C.light_wood)
	if random.chance(0.6) then api.deco("cube", at(s, 0.05, 0, 0.6), sz(s, 0.16, 0.1, 0.12), C.paper, { roughness = 0.95 }) end
	return id
end

-- A sofa: seat, back, arms.
function ITEMS.sofa(api, s: Slot)
	local col = C.sofas[random.int(1, #C.sofas)]
	local id = body(api, s, 0.42, col, { roughness = 0.95 })
	api.deco("cube", at(s, 0, -s.d / 2 + 0.12, 0.62), sz(s, s.w, 0.5, 0.24), col, { roughness = 0.95 })
	for _, a in { -s.w / 2 + 0.1, s.w / 2 - 0.1 } do
		api.deco("cube", at(s, a, 0, 0.55), sz(s, 0.2, 0.3, s.d), col, { roughness = 0.95 })
	end
	return id
end

function ITEMS.tv(api, s: Slot)
	local id = body(api, s, 0.5, C.wood)
	if random.chance(0.7) then
		api.deco("cube", at(s, 0, 0, 0.8), sz(s, s.w * 0.75, 0.55, 0.08), C.screen, { roughness = 0.1 })
	end
	return id
end

-- Shelving: uprights, four shelves and what is left on them.
function ITEMS.shelf(api, s: Slot)
	local x0, z0, x1, z1 = rect_of(s)
	local id = api.hidden(x0, z0, x1, z1, 1.8)
	for _, a in { -s.w / 2 + 0.03, s.w / 2 - 0.03 } do
		api.deco("cube", at(s, a, 0, 0.9), sz(s, 0.05, 1.8, s.d), C.steel, { roughness = 0.5, metal = 0.6 })
	end
	for k = 0, 3 do
		local y = 0.1 + k * 0.52
		api.deco("cube", at(s, 0, 0, y), sz(s, s.w, 0.03, s.d), C.steel, { roughness = 0.5, metal = 0.6 })
		for _ = 1, random.int(0, 3) do
			local g = random.range(0.12, 0.25)
			api.deco("cube", at(s, random.range(-s.w / 2 + 0.2, s.w / 2 - 0.2), random.range(-0.05, 0.05), y + g / 2 + 0.015), v(g, g, g * 0.8), C.goods[random.int(1, #C.goods)],
				{ roughness = 0.8, rot = v(0, random.range(0, 3), 0) })
		end
	end
	return id
end

function ITEMS.till(api, s: Slot)
	local id = body(api, s, 1.0, C.cabinet)
	api.deco("cube", at(s, 0, 0, 1.02), sz(s, s.w + 0.06, 0.04, s.d + 0.06), C.top, { roughness = 0.6 })
	api.deco("cube", at(s, s.w / 4, 0, 1.12), sz(s, 0.4, 0.16, 0.36), C.stove, { roughness = 0.5, metal = 0.5 })
	return id
end

-- A desk with a screen and papers; a chair that may be on its side.
function ITEMS.desk(api, s: Slot)
	local x0, z0, x1, z1 = rect_of(s)
	local id = api.hidden(x0, z0, x1, z1, 0.75)
	api.deco("cube", at(s, 0, 0, 0.73), sz(s, s.w, 0.04, s.d), C.light_wood, { roughness = 0.8 })
	for _, a in { -s.w / 2 + 0.03, s.w / 2 - 0.03 } do
		api.deco("cube", at(s, a, 0, 0.36), sz(s, 0.04, 0.72, s.d - 0.04), C.light_wood, { roughness = 0.8 })
	end
	if random.chance(0.7) then
		api.deco("cube", at(s, random.range(-0.2, 0.2), -0.12, 0.95), sz(s, 0.5, 0.36, 0.04), C.screen, { roughness = 0.1, rot = v(0, random.range(-0.2, 0.2), 0) })
	end
	for _ = 1, random.int(0, 3) do
		api.deco("cube", at(s, random.range(-s.w / 2 + 0.2, s.w / 2 - 0.2), random.range(0, 0.2), 0.755), sz(s, 0.22, 0.004, 0.3), C.paper,
			{ roughness = 0.95, rot = v(0, random.range(-0.5, 0.5), 0) })
	end
	local tipped = random.chance(0.4)
	api.deco("cube", at(s, random.range(-0.2, 0.2), s.d / 2 + 0.35, if tipped then 0.22 else 0.45), sz(s, 0.44, 0.06, 0.44), C.door,
		{ roughness = 0.8, rot = if tipped then v(math.pi / 2, random.range(-1, 1), 0) else nil })
	return id
end

function ITEMS.cabinet(api, s: Slot)
	local id = body(api, s, 1.3, C.dark_steel, { roughness = 0.5, metal = 0.6 })
	for k = 0, 3 do
		api.deco("cube", at(s, 0, s.d / 2 + 0.005, 0.18 + k * 0.32), sz(s, s.w - 0.06, 0.012, 0.012), C.door, { roughness = 0.9 })
	end
	return id
end

function ITEMS.lockers(api, s: Slot)
	local id = body(api, s, 1.9, C.steel, { roughness = 0.5, metal = 0.6 })
	for a = -s.w / 2 + 0.4, s.w / 2 - 0.1, 0.4 do
		api.deco("cube", at(s, a, s.d / 2 + 0.005, 0.95), sz(s, 0.015, 1.8, 0.012), C.door, { roughness = 0.9 })
	end
	return id
end

function ITEMS.stove_iron(api, s: Slot)
	local id = body(api, s, 0.7, C.stove, { roughness = 0.6, metal = 0.5 })
	api.deco("cube", at(s, 0, -0.1, 1.6), sz(s, 0.14, 1.8, 0.14), C.stove, { roughness = 0.6, metal = 0.5 })
	return id
end

function ITEMS.table(api, s: Slot)
	local x0, z0, x1, z1 = rect_of(s)
	local id = api.hidden(x0, z0, x1, z1, 0.75)
	api.deco("cube", at(s, 0, 0, 0.73), sz(s, s.w, 0.05, s.d), C.wood, { roughness = 0.85 })
	for _, o in { { -1, -1 }, { 1, -1 }, { -1, 1 }, { 1, 1 } } do
		api.deco("cube", at(s, o[1] * (s.w / 2 - 0.06), o[2] * (s.d / 2 - 0.06), 0.36), v(0.06, 0.72, 0.06), C.wood, { roughness = 0.85 })
	end
	return id
end

-- Hay in bales, two high.
function ITEMS.bales(api, s: Slot)
	local id = body(api, s, 0.5, C.hay, { roughness = 0.97 })
	if random.chance(0.6) then
		api.deco("cube", at(s, random.range(-0.2, 0.2), 0, 0.75), sz(s, s.w * 0.6, 0.5, s.d - 0.05), C.hay, { roughness = 0.97, rot = v(0, random.range(-0.2, 0.2), 0) })
	end
	return id
end

function ITEMS.crates(api, s: Slot)
	local id = body(api, s, 0.6, C.crate)
	if random.chance(0.5) then api.deco("cube", at(s, 0.1, 0, 0.9), sz(s, 0.6, 0.6, 0.6), C.crate, { roughness = 0.9, rot = v(0, random.range(-0.4, 0.4), 0) }) end
	return id
end

-- Each item: its builder, width along the wall, depth.
local SIZE = {
	counter = { 2.2, 0.62 }, fridge = { 0.7, 0.66 }, stove = { 0.6, 0.6 }, wardrobe = { 1.1, 0.58 }, bed = { 1.4, 2.0 },
	nightstand = { 0.45, 0.4 }, sofa = { 1.9, 0.86 }, tv = { 1.2, 0.42 }, shelf = { 1.8, 0.45 }, till = { 1.5, 0.62 },
	desk = { 1.4, 0.7 }, cabinet = { 0.5, 0.6 }, lockers = { 1.6, 0.5 }, stove_iron = { 0.6, 0.6 }, table = { 1.2, 0.8 },
	bales = { 1.6, 1.0 }, crates = { 1.0, 0.8 },
}

-- Per room: the piece the container is (searched with A), then the rest,
-- each tried once.
local ROOMS = {
	kitchen = { searched = "counter", rest = { "fridge", "stove", "table" } },
	bedroom = { searched = "wardrobe", rest = { "bed", "nightstand" } },
	living = { searched = "cabinet", rest = { "sofa", "tv", "table" } },
	cabin = { searched = "wardrobe", rest = { "bed", "stove_iron", "table" } },
	barn = { searched = "crates", rest = { "bales", "bales", "crates" } },
	shop = { searched = "shelf", rest = { "shelf", "shelf", "till" } },
	office = { searched = "cabinet", rest = { "desk", "desk", "cabinet" } },
	pharmacy = { searched = "shelf", rest = { "shelf", "till", "cabinet" } },
	police = { searched = "lockers", rest = { "desk", "cabinet", "desk" } },
}

-- Furnish room r of building b. `kind` is the container kind (or nil for
-- a room without one; `kinds` then picks what sort of room it is).
-- Returns the container's solid id and rect bounds when there is one.
function M.furnish(api, b, r, kind: string?, kinds: { string })
	local room = ROOM_OF[kind or kinds[random.int(1, #kinds)]] or "living"
	if room == "bedroom" and random.chance(0.4) then room = "living" end
	local plan = ROOMS[room]
	local taken = {}
	local found = nil
	if kind then
		local size = SIZE[plan.searched]
		local s = place(b, r, taken, size[1], size[2])
		if s then
			local id = ITEMS[plan.searched](api, s)
			local x0, z0, x1, z1 = rect_of(s)
			found = { id = id, x = s.x, z = s.z, x0 = x0, z0 = z0, x1 = x1, z1 = z1 }
		end
	end
	for _, name in plan.rest do
		local size = SIZE[name]
		local s = place(b, r, taken, size[1], size[2])
		if s then ITEMS[name](api, s) end
	end
	-- The floor: tiles in kitchens and shops, a rug in a living room or a
	-- bedroom; and something knocked over in the middle.
	local mx, mz = (r.x0 + r.x1) / 2, (r.z0 + r.z1) / 2
	local fc = C.floor[room]
	if fc then
		api.deco("cube", v(mx, 0.014, mz), v(r.x1 - r.x0 - 0.2, 0.004, r.z1 - r.z0 - 0.2), fc, { roughness = 0.6 })
	elseif room == "living" or room == "bedroom" then
		api.deco("cube", v(mx + random.range(-0.3, 0.3), 0.016, mz + random.range(-0.3, 0.3)), v(random.range(1.6, 2.4), 0.008, random.range(1.2, 1.8)), C.rugs[random.int(1, #C.rugs)],
			{ roughness = 0.97, rot = v(0, random.range(-0.2, 0.2), 0) })
	end
	if room ~= "barn" and random.chance(0.5) then
		api.deco("cube", v(mx + random.range(-0.6, 0.6), 0.22, mz + random.range(-0.6, 0.6)), v(0.44, 0.06, 0.44), C.door,
			{ roughness = 0.8, rot = v(math.pi / 2, random.range(0, 3), 0) })
	end
	return found
end

-- Every look the rooms use, warmed in init (land.warm calls this).
function M.warm(warm)
	local seen = {}
	local function w(shape, color, rough, metal)
		local key = shape .. color .. rough .. tostring(metal)
		if seen[key] then return end
		seen[key] = true
		warm(shape, { color = color, roughness = rough, metal = metal })
	end
	for _, c in { C.cabinet, C.wood, C.light_wood, C.crate } do w("cube", c, 0.85) w("cube", c, 0.8) w("cube", c, 0.9) end
	w("cube", C.top, 0.6)
	w("cube", C.door, 0.9) w("cube", C.door, 0.8)
	w("sphere", C.door, 0.9)
	w("cube", C.dark_steel, 0.35, 0.7) w("cube", C.dark_steel, 0.5, 0.6)
	w("cube", C.steel, 0.5, 0.6)
	w("cube", C.white, 0.5)
	w("cube", C.stove, 0.5, 0.5) w("cube", C.stove, 0.6, 0.5)
	w("cube", C.mattress, 0.95) w("cube", C.pillow, 0.95)
	for _, c in C.blankets do w("cube", c, 0.95) end
	for _, c in C.sofas do w("cube", c, 0.95) end
	for _, c in C.rugs do w("cube", c, 0.97) end
	for _, c in C.goods do w("cube", c, 0.8) end
	w("cube", C.screen, 0.1)
	w("cube", C.paper, 0.95)
	w("cube", C.hay, 0.97)
	for _, c in C.floor do w("cube", c, 0.6) end
end

return M
