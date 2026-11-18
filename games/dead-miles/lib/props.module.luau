-- Props for street events: a police car, an ambulance on its side, a
-- pram, a radio, a bell frame and so on. Each is plain data: parts in the
-- prop's own space (x along its length, y up, z across), a footprint that
-- is solid, and the looks. build() places one and returns its entities and
-- the solid box; warm() readies every look so nothing stalls mid-game.

local v = vector.create
local mat = require("lib/mat")

local M = {}

local WHITE = "#e8e8f0"
local DARK = "#202024"
local RUST = "#6a4030"
local STEEL = "#7a7d82"
local WOOD = "#6b4e37"
local CLOTH = "#4a5a6a"
local FIRE = "#ff8a3a"

-- part = { shape, pos, size, color, rough?, metal?, glow?, rot? }
local function p(shape, pos, size, color, extra)
	local t = { shape = shape, pos = pos, size = size, color = color, rough = 0.9 }
	for k, x in extra or {} do t[k] = x end
	return t
end

M.DEFS = {
	police_car = {
		foot = { 4.2, 1.85, 1.2 },
		parts = {
			p("cube", v(0, 0.6, 0), v(4.2, 0.95, 1.85), WHITE, { rough = 0.4, metal = 0.3 }),
			p("cube", v(0.2, 1.3, 0), v(2.1, 0.6, 1.6), DARK),
			p("cube", v(0.1, 1.66, -0.3), v(0.5, 0.12, 0.5), "#1d3a8a", { glow = 1.2 }),
			p("cube", v(0.1, 1.66, 0.3), v(0.5, 0.12, 0.5), "#8a1d1d", { glow = 1.2 }),
			-- The boot lid: shut until someone forces it (a result's `open`).
			p("cube", v(-1.75, 1.1, 0), v(0.75, 0.08, 1.7), WHITE, { rough = 0.4, metal = 0.3, open = { pos = v(-2.0, 1.32, 0), rot = v(0, 0, -0.9) } }),
		},
	},
	ambulance = {
		-- On its side: the roof faces the street.
		foot = { 5.4, 2.4, 2.2 },
		parts = {
			p("cube", v(0, 1.1, 0), v(5.4, 2.2, 2.3), WHITE, { rough = 0.5, metal = 0.2 }),
			p("cube", v(0, 1.1, -1.17), v(5.0, 0.35, 0.04), "#b02a2a"),
			p("cube", v(2.3, 1.1, -1.17), v(0.6, 0.6, 0.05), "#b02a2a"),
			-- The back doors hang open.
			p("cube", v(-3.0, 1.6, 0.6), v(0.06, 1.0, 1.1), WHITE, { rough = 0.5, metal = 0.2, rot = v(0, 0.9, 0) }),
			p("cube", v(-2.9, 0.5, 0.6), v(0.06, 1.0, 1.1), WHITE, { rough = 0.5, metal = 0.2, rot = v(0, -0.7, 0) }),
			p("sphere", v(1.8, 0.4, 1.25), v(0.7, 0.7, 0.3), "#1a1a1a"),
			p("sphere", v(-1.8, 0.4, 1.25), v(0.7, 0.7, 0.3), "#1a1a1a"),
		},
	},
	pram = {
		foot = { 0.9, 0.55, 1.0 },
		parts = {
			p("cube", v(0, 0.55, 0), v(0.8, 0.4, 0.5), CLOTH),
			p("sphere", v(-0.25, 0.8, 0), v(0.5, 0.5, 0.52), "#2f3a4a"),
			p("cube", v(0.5, 0.95, 0), v(0.06, 0.06, 0.5), STEEL, { metal = 0.6, rough = 0.5 }),
			p("sphere", v(0.3, 0.15, 0.25), v(0.25, 0.25, 0.06), "#1a1a1a"),
			p("sphere", v(-0.3, 0.15, 0.25), v(0.25, 0.25, 0.06), "#1a1a1a"),
			p("sphere", v(0.3, 0.15, -0.25), v(0.25, 0.25, 0.06), "#1a1a1a"),
			p("sphere", v(-0.3, 0.15, -0.25), v(0.25, 0.25, 0.06), "#1a1a1a"),
		},
	},
	radio = {
		foot = { 0.7, 0.7, 0.9 },
		parts = {
			p("cube", v(0, 0.3, 0), v(0.7, 0.6, 0.7), WOOD),
			p("cube", v(0, 0.75, 0), v(0.5, 0.3, 0.2), "#2a2a2c"),
			p("cube", v(0.08, 0.78, -0.105), v(0.18, 0.08, 0.01), "#ffcf6a", { glow = 0.8 }),
			p("cube", v(-0.18, 0.95, 0), v(0.02, 0.4, 0.02), STEEL, { metal = 0.6, rough = 0.5, rot = v(0, 0, 0.4) }),
		},
	},
	burning_car = {
		foot = { 4.2, 1.85, 1.1 },
		parts = {
			p("cube", v(0, 0.5, 0), v(4.2, 1.0, 1.85), "#2a2624"),
			p("cube", v(0, 1.15, 0), v(1.8, 0.3, 1.5), "#3a3632"),
			p("sphere", v(0.3, 1.4, 0), v(1.6, 1.1, 1.2), FIRE, { glow = 2.2 }),
			p("sphere", v(-0.9, 1.2, 0.2), v(0.9, 0.8, 0.8), "#ffb04a", { glow = 2.2 }),
			p("sphere", v(1.2, 1.1, -0.2), v(0.8, 0.7, 0.7), FIRE, { glow = 2.2 }),
		},
	},
	bell = {
		-- A bell on a timber frame, from a chapel that is gone.
		foot = { 1.8, 1.8, 4.6 },
		parts = {
			p("cube", v(-0.75, 2.3, -0.75), v(0.2, 4.6, 0.2), WOOD),
			p("cube", v(0.75, 2.3, -0.75), v(0.2, 4.6, 0.2), WOOD),
			p("cube", v(-0.75, 2.3, 0.75), v(0.2, 4.6, 0.2), WOOD),
			p("cube", v(0.75, 2.3, 0.75), v(0.2, 4.6, 0.2), WOOD),
			p("cube", v(0, 4.55, 0), v(1.9, 0.2, 1.9), WOOD),
			p("cube", v(0, 5.0, 0), v(1.6, 0.7, 1.6), "#3a3632", { rot = v(0, 0.785, 0) }),
			p("sphere", v(0, 3.8, 0), v(0.8, 0.9, 0.8), "#8a6a2a", { metal = 0.8, rough = 0.4 }),
		},
	},
	tent = {
		foot = { 2.4, 1.8, 1.2 },
		parts = {
			p("cube", v(0, 0.55, 0), v(2.4, 1.1, 1.1), "#4a5a3a", { rot = v(0.785, 0, 0) }),
			p("cube", v(1.9, 0.08, 0.2), v(0.8, 0.16, 0.8), "#2a2624"),
			p("cube", v(1.9, 0.12, -0.5), v(0.3, 0.24, 0.3), "#7a7470"),
			p("cube", v(2.4, 0.12, 0.6), v(0.3, 0.24, 0.3), "#7a7470"),
			p("cube", v(-0.2, 0.06, 1.2), v(1.8, 0.12, 0.7), "#6a3a2a"),
			p("cube", v(0.5, 0.1, -1.3), v(0.3, 0.2, 0.22), "#c9b25a"),
		},
	},
	bench = {
		foot = { 1.8, 0.6, 0.9 },
		parts = {
			p("cube", v(0, 0.45, 0), v(1.8, 0.08, 0.5), WOOD),
			p("cube", v(0, 0.8, 0.24), v(1.8, 0.4, 0.06), WOOD),
			p("cube", v(-0.8, 0.22, 0), v(0.08, 0.45, 0.5), STEEL, { metal = 0.6, rough = 0.5 }),
			p("cube", v(0.8, 0.22, 0), v(0.08, 0.45, 0.5), STEEL, { metal = 0.6, rough = 0.5 }),
		},
	},
	gate = {
		-- A locked yard gate in a fence: the shortcut on the other side.
		foot = { 4.4, 0.3, 2.0 },
		parts = {
			p("cube", v(-2.1, 1.0, 0), v(0.15, 2.0, 0.15), STEEL, { metal = 0.6, rough = 0.5 }),
			p("cube", v(2.1, 1.0, 0), v(0.15, 2.0, 0.15), STEEL, { metal = 0.6, rough = 0.5 }),
			p("cube", v(0, 1.0, 0), v(4.0, 1.8, 0.04), "#5f6266", { metal = 0.6, rough = 0.5 }),
			p("cube", v(0, 1.0, -0.05), v(0.14, 0.18, 0.06), "#c9b25a", { metal = 0.8, rough = 0.4 }),
		},
	},
	collapse = {
		-- Where the road fell in: a dark hole, a tilted slab, rubble and a
		-- barrier someone put up.
		foot = { 3.4, 2.4, 0.8 },
		parts = {
			p("cube", v(0, 0.012, 0), v(3.2, 0.02, 2.2), "#08080a", { rough = 1 }),
			p("cube", v(-1.7, 0.3, 0.2), v(1.6, 0.25, 2.0), "#3a3a3c", { rot = v(0, 0.2, 0.4) }),
			p("cube", v(1.8, 0.2, -0.6), v(0.6, 0.4, 0.5), "#5f5a55"),
			p("cube", v(1.5, 0.15, 0.7), v(0.5, 0.3, 0.4), "#7a7470"),
			p("cube", v(0, 0.55, -1.4), v(3.2, 0.25, 0.08), "#c94a3a"),
			p("cube", v(-1.4, 0.3, -1.4), v(0.08, 0.6, 0.08), STEEL, { metal = 0.6, rough = 0.5 }),
			p("cube", v(1.4, 0.3, -1.4), v(0.08, 0.6, 0.08), STEEL, { metal = 0.6, rough = 0.5 }),
		},
	},
	sign = {
		-- A signpost at a fork: two boards, two ways.
		foot = { 0.4, 0.4, 2.6 },
		parts = {
			p("cube", v(0, 1.3, 0), v(0.12, 2.6, 0.12), WOOD),
			p("cube", v(0.45, 2.2, 0), v(0.9, 0.25, 0.05), "#d8d0a0", { rot = v(0, 0, 0.08) }),
			p("cube", v(-0.4, 1.8, 0), v(0.8, 0.25, 0.05), "#d8d0a0", { rot = v(0, 0, -0.1) }),
		},
	},
	shelves = {
		-- A shop's shelves, half emptied.
		foot = { 2.6, 0.7, 1.8 },
		parts = {
			p("cube", v(0, 0.9, 0), v(2.6, 1.8, 0.6), "#5a5a5e", { metal = 0.4, rough = 0.6 }),
			p("cube", v(-0.8, 1.35, -0.32), v(0.3, 0.3, 0.1), "#c94a3a"),
			p("cube", v(0.1, 0.75, -0.32), v(0.4, 0.25, 0.1), "#d8d0a0"),
			p("cube", v(0.9, 1.35, -0.32), v(0.25, 0.3, 0.1), "#4a7a4a"),
			p("cube", v(0.6, 0.25, -0.32), v(0.5, 0.3, 0.1), "#c9b25a"),
		},
	},
	balcony = {
		-- A first-floor balcony on a lone bit of facade, a rope and basket.
		foot = { 3.0, 1.0, 5.0 },
		parts = {
			p("cube", v(0, 2.5, 0.3), v(3.0, 5.0, 0.4), "#9a845e", { rough = 0.95 }),
			p("cube", v(0, 3.0, -0.35), v(2.2, 0.15, 1.0), "#5f5a55", { rough = 0.95 }),
			p("cube", v(0, 3.5, -0.82), v(2.2, 0.9, 0.05), STEEL, { metal = 0.6, rough = 0.5 }),
			p("cube", v(0.5, 2.0, -0.9), v(0.02, 2.0, 0.02), "#d8d0a0"),
			p("cube", v(0.5, 0.9, -0.9), v(0.4, 0.25, 0.3), "#8b6b4a"),
			p("cube", v(-0.8, 1.6, 0.08), v(0.8, 1.2, 0.05), DARK),
		},
	},
}

-- The look of a part, exactly as spawned: what warm() readies.
local function look(t)
	return { color = t.color, roughness = t.rough, metal = t.metal, emissive = t.glow }
end

function M.warm()
	for _, d in M.DEFS do
		for _, t in d.parts do mat.warm(t.shape, look(t)) end
	end
end

-- Place prop `name` with its centre at pos, turned by yaw (0: its length
-- along +x). Returns { ents, rect }: rect is the solid footprint (axis
-- aligned, for the game's collision grid), with the id of an invisible
-- wall that stops sight lines too.
function M.build(name, pos, yaw)
	local d = M.DEFS[name]
	if not d then error("props: no prop " .. tostring(name)) end
	yaw = yaw or 0
	local ents, lids = {}, nil
	local root = mat.spawn("cube", { pos = pos, rot = v(0, yaw, 0), size = 0.05, visible = false, solid = false })
	table.insert(ents, root)
	for _, t in d.parts do
		local o = look(t)
		o.pos, o.size, o.rot, o.solid, o.tag = t.pos, t.size, t.rot, false, "deco"
		local id = mat.spawn(t.shape, o, root) -- goes with the root
		-- Parts that move when opened (a boot lid, a door).
		if t.open then
			lids = lids or {}
			table.insert(lids, { id = id, pos = t.open.pos, rot = t.open.rot })
		end
	end
	-- The footprint, turned to the nearest quarter.
	local w, dd, h = d.foot[1], d.foot[2], d.foot[3]
	local quarter = math.floor(yaw / (math.pi / 2) + 0.5) % 2 == 1
	if quarter then w, dd = dd, w end
	local wall = mat.spawn("cube", { pos = v(pos.x, h / 2, pos.z), size = v(w, h, dd), visible = false, tag = "wall" })
	table.insert(ents, wall)
	local rect = { x0 = pos.x - w / 2, x1 = pos.x + w / 2, z0 = pos.z - dd / 2, z1 = pos.z + dd / 2, h = h, id = wall }
	return { ents = ents, rect = rect, root = root, lids = lids }
end

-- Swing a prop's moving parts open (the `lids` build returned).
function M.open(lids)
	for _, l in lids or {} do world.set(l.id, { pos = l.pos, rot = l.rot or v(0, 0, 0) }) end
end

return M
