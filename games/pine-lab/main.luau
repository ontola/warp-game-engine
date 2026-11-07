-- Pine lab: a row of Jeffrey pines from seedling to old tree, regrown from
-- the rules in the panel. Arrows pick and change a rule, F toggles the full
-- (voxel) cook, R resets a rule, Tab hides the panel. Changes are saved in
-- tuning.json. Rules are render::plant_growth::Species and Look fields.

-- @knobs
local KNOBS = {
	speed = 12, -- camera speed, m/s
	settle = 0.15, -- seconds without changes before the row regrows
}
-- @end

-- One row per tunable value: where it lives (`species` or `look`), its
-- field, the array index for list fields, the step, and limits.
local PARAMS = {
	{ "scene", "oldest", nil, 5, 3, 400, "oldest tree in the row, years" },
	{ "scene", "count", nil, 1, 1, 12, "trees in the row" },
	{ "scene", "seed", nil, 1, 1, 9999, "seed of every tree" },
	{ "species", "plumes", nil, 0.25, 1, 6, "needle brushes per side shoot" },
	{ "look", "brush_needles", nil, 10, 10, 600, "needles in one brush" },
	{ "look", "needle_length", nil, 0.01, 0.05, 0.5, "needle length, m" },
	{ "look", "brush_spread", nil, 0.05, 0.2, 3, "needle splay at a brush's back, rad" },
	{ "look", "brush_reach", nil, 0.02, 0.02, 1, "shoot length carrying needles, m" },
	{ "look", "needle_width", nil, 0.0005, 0.0005, 0.02, "needle radius, m" },
	{ "look", "brush_round", nil, 0.05, 0, 1, "brush shading as one ball (less speckle)" },
	{ "species", "live_crown", nil, 0.05, 0.1, 1, "crown share with live limbs when old" },
	{ "species", "lift_years", nil, 5, 15, 400, "years until the crown has lifted" },
	{ "species", "height", nil, 1, 2, 80, "height the leader approaches, m" },
	{ "species", "pace", nil, 2, 5, 300, "years to reach most of that height" },
	{ "species", "shape", nil, 0.05, 0.5, 4, "height curve exponent" },
	{ "species", "first_whorl", nil, 1, 1, 20, "year of the first side branches" },
	{ "species", "whorl", 1, 1, 0, 12, "branches per whorl, fewest" },
	{ "species", "whorl", 2, 1, 0, 12, "branches per whorl, most" },
	{ "species", "laterals", nil, 0.05, 0, 4, "side shoots a shoot sets per year" },
	{ "species", "max_order", nil, 1, 1, 6, "finest branch order" },
	{ "species", "limb_step", nil, 0.02, 0.02, 2, "yearly limb shoot length, m" },
	{ "species", "side_length", nil, 0.05, 0.05, 1.5, "finer shoot length over its parent" },
	{ "species", "limb_life", 1, 5, 1, 600, "limb life, shortest, years" },
	{ "species", "limb_life", 2, 5, 1, 600, "limb life, longest, years" },
	{ "species", "crowding", nil, 1, 1, 60, "live tips per 1.5 m cell" },
	{ "species", "angle", 1, 0.05, 0, 3.14, "branch angle young, rad" },
	{ "species", "angle", 2, 0.05, 0, 3.14, "branch angle old, rad" },
	{ "species", "rise", nil, 0.01, -0.5, 0.5, "upward bend per year" },
	{ "species", "droop", nil, 0.002, 0, 0.2, "droop of older wood" },
	{ "species", "needle_years", nil, 1, 1, 12, "years needles stay" },
	{ "species", "shade", nil, 0.002, 0, 0.3, "shading per foliage above" },
	{ "species", "min_light", nil, 0.02, 0, 1, "light under which a tip dies" },
	{ "species", "tip_radius", nil, 0.001, 0.001, 0.05, "twig tip radius, m" },
	{ "species", "wood", nil, 0.0005, 0.0005, 0.02, "trunk wood per year" },
	{ "species", "limb_wood", nil, 0.02, 0.05, 1, "limb wood over trunk wood" },
	{ "species", "dead_years", nil, 1, 0, 100, "years a dead limb stands" },
	{ "look", "needles", 1, 0.01, 0, 1, "needle colour, red" },
	{ "look", "needles", 2, 0.01, 0, 1, "needle colour, green" },
	{ "look", "needles", 3, 0.01, 0, 1, "needle colour, blue" },
	{ "look", "wood", 1, 0.01, 0, 1, "bark colour, red" },
	{ "look", "wood", 2, 0.01, 0, 1, "bark colour, green" },
	{ "look", "wood", 3, 0.01, 0, 1, "bark colour, blue" },
}

local DEFAULT_SCENE = { oldest = 50, count = 6, seed = 1 }

local function copy(t)
	if type(t) ~= "table" then return t end
	local out = {}
	for k, v in t do out[k] = copy(v) end
	return out
end

-- The ages in the row: from 3 years to the oldest, evenly on a log scale.
local function ages()
	local t, n = state.tune.scene, state.tune.scene.count
	local out = {}
	for i = 1, n do
		local f = if n == 1 then 1 else (i - 1) / (n - 1)
		out[i] = math.max(1, math.floor(3 * (t.oldest / 3) ^ f + 0.5))
	end
	return out
end

local function grow_row()
	plants.clear()
	local x = 0
	local tune = state.tune
	state.row = {}
	for i, age in ages() do
		-- Room for the crown: old pines spread wider.
		local room = math.clamp(age * 0.12, 2, 14)
		x += room
		plants.grow({ age = age, seed = tune.scene.seed + i, pos = vector.create(x, 0, 0), full = state.full, species = tune.species, look = tune.look })
		table.insert(state.row, { age = age, x = x })
		x += room
	end
	state.row_end = x
	state.dirty_at = nil
end

local function get(p)
	local group, key, index = p[1], p[2], p[3]
	local v = state.tune[group][key]
	if index then return v[index] end
	return v
end

local function set(p, value)
	local group, key, index = p[1], p[2], p[3]
	local step = p[4]
	value = math.clamp(value, p[5], p[6])
	-- Whole steps stay whole numbers (counts and years are integers).
	if step == math.floor(step) then value = math.floor(value + 0.5) else value = math.floor(value / step + 0.5) * step end
	if index then state.tune[group][key][index] = value else state.tune[group][key] = value end
	state.dirty_at = game.time()
end

local function init()
	local d = plants.defaults()
	state.defaults = { species = d.species, look = d.look, scene = DEFAULT_SCENE }
	local saved = plants.load("tuning")
	state.tune = copy(state.defaults)
	if type(saved) == "table" then
		-- tuning.json holds only what differs from the defaults, so new
		-- defaults in the engine still come through (an older file without
		-- `version` kept every rule: only its scene is used).
		local groups = if saved.version == 2 then { "species", "look", "scene" } else { "scene" }
		for _, group in groups do
			for k, v in (saved[group] or {}) do
				if state.tune[group][k] ~= nil then state.tune[group][k] = v end
			end
		end
	end
	state.pick = 1
	state.full = false
	state.panel = true
	state.repeat_at = 0
	world.spawn("cube", { pos = vector.create(0, -0.5, 0), size = vector.create(2000, 1, 2000), color = "#7b8f5a", roughness = 0.95, tag = "floor" })
	grow_row()
	local mid = state.row_end * 0.5
	state.cam = { pos = vector.create(mid, 8, state.row_end * 0.7 + 10), yaw = 0, pitch = -0.15 }
end

local function same(a, b)
	if type(a) ~= "table" or type(b) ~= "table" then return a == b end
	for k, v in a do if not same(v, b[k]) then return false end end
	for k in b do if a[k] == nil then return false end end
	return true
end

-- The rules that differ from the defaults, for tuning.json.
local function changes()
	local out = { version = 2 }
	for _, group in { "species", "look", "scene" } do
		out[group] = {}
		for k, v in state.tune[group] do
			if not same(v, state.defaults[group][k]) then out[group][k] = v end
		end
	end
	return out
end

local function step_param(dir)
	local p = PARAMS[state.pick]
	set(p, get(p) + dir * p[4])
end

local function tick(dt, input)
	local i = input[1]
	if not i then return end
	local held, pressed = i.held, i.pressed
	-- Picking and changing a parameter; holding an arrow repeats.
	if pressed.up then state.pick = (state.pick - 2) % #PARAMS + 1 end
	if pressed.down then state.pick = state.pick % #PARAMS + 1 end
	local dir = if held.right then 1 elseif held.left then -1 else 0
	if pressed.right or pressed.left then
		step_param(dir)
		state.repeat_at = game.time() + 0.35
	elseif dir ~= 0 and game.time() >= state.repeat_at then
		step_param(dir)
		state.repeat_at = game.time() + 0.06
	end
	if pressed.west then
		local p = PARAMS[state.pick]
		local d = state.defaults[p[1]][p[2]]
		set(p, if p[3] then d[p[3]] else d)
	end
	if pressed.north then
		state.full = not state.full
		grow_row()
	end
	if pressed.select then state.panel = not state.panel end
	state.status = plants.status()
	if state.dirty_at and game.time() - state.dirty_at >= KNOBS.settle then
		plants.save("tuning", changes())
		grow_row()
	end

	-- A fly camera: mouse looks, WASD moves, Shift or E up, Ctrl or Q down.
	local c = state.cam
	c.yaw -= i.look_x
	c.pitch = math.clamp(c.pitch + i.look_y, -1.5, 1.5)
	local fwd = vector.create(-math.sin(c.yaw) * math.cos(c.pitch), math.sin(c.pitch), -math.cos(c.yaw) * math.cos(c.pitch))
	local right = vector.create(math.cos(c.yaw), 0, -math.sin(c.yaw))
	local up = (if held.right_shoulder then 1 else 0) - (if held.left_shoulder then 1 else 0)
	c.pos += (fwd * i.move_y + right * i.move_x + vector.create(0, up, 0)) * KNOBS.speed * dt
	c.pos = vector.create(c.pos.x, math.max(c.pos.y, 0.3), c.pos.z)
	camera.set(1, { pos = c.pos, target = c.pos + fwd, fov = 55 })
end

local function fmt(v)
	if type(v) ~= "number" then return tostring(v) end
	if v == math.floor(v) then return tostring(v) end
	return string.format("%.4g", v)
end

local function draw()
	local w, h = hud.size()
	local s = h / 1080
	for _, t in state.row or {} do
		local x, y = hud.project(vector.create(t.x, -0.2, 0))
		if x then hud.text(t.age .. " y", x - 60 * s, y, { size = 26 * s, align = "center", width = 120 * s }) end
	end
	local st = plants.status()
	local status = string.format("%s   %d of %d shown%s%s", if state.full then "full cook (F)" else "preview (F)", st.shown, st.placed,
		if st.cooking > 0 then string.format(", %d cooking", st.cooking) else "",
		if state.dirty_at then ", waiting for changes to settle" else "")
	hud.text(status, 24 * s, h - 48 * s, { size = 26 * s })
	if st.error then hud.text("cook failed: " .. st.error, 24 * s, h - 84 * s, { size = 24 * s, color = "#ff8080" }) end
	if not state.panel then return end
	local line = 30 * s
	local rows = math.min(#PARAMS, math.floor((h - 200 * s) / line))
	local first = math.clamp(state.pick - math.floor(rows / 2), 1, #PARAMS - rows + 1)
	hud.box(16 * s, 16 * s, 620 * s, (rows + 2) * line + 20 * s, { 0, 0, 0, 0.55 })
	hud.text("Jeffrey pine rules   arrows pick and change, R resets, Tab hides", 30 * s, 26 * s, { size = 22 * s, color = "#c8d8c0" })
	for r = 0, rows - 1 do
		local n = first + r
		local p = PARAMS[n]
		local y = 26 * s + (r + 1.3) * line
		local name = p[2] .. (if p[3] then "[" .. p[3] .. "]" else "")
		local here = n == state.pick
		local d = state.defaults[p[1]][p[2]]
		if p[3] then d = d[p[3]] end
		local changed = get(p) ~= d
		local color = if here then "#ffe08a" elseif changed then "#a8e0ff" else "#ffffff"
		if here then hud.box(22 * s, y - 3 * s, 608 * s, line, { 1, 1, 1, 0.12 }) end
		hud.text(name, 34 * s, y, { size = 24 * s, color = color })
		hud.text(fmt(get(p)), 250 * s, y, { size = 24 * s, color = color })
		hud.text(p[7], 360 * s, y + 2 * s, { size = 18 * s, color = "#b0b0b0" })
	end
end

local function reload()
	print("pine lab reloaded")
end

return { init = init, tick = tick, draw = draw, reload = reload }
