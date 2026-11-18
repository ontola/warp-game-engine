-- Human lab: eight seeded Warp Humans in a row and fixed portrait cameras,
-- to judge faces, skin, hair and clothes up close in the full renderer.
-- Left/right pick a person, up/down a view, south turns them a quarter.
-- From a script or the console: state.walk = 1.4 (m/s), state.pose = "wave".

-- @knobs
local KNOBS = {
	spacing = 1.2, -- metres between people
}
-- @end

-- Camera relative to the head: distance (m), side angle (rad), height
-- above the head (m), where it aims below the head (m), fov (degrees).
local VIEWS = {
	{ name = "face", dist = 0.62, side = 0, up = 0.0, drop = 0.03, fov = 30 },
	{ name = "three-quarter", dist = 0.66, side = 0.7, up = 0.03, drop = 0.03, fov = 30 },
	{ name = "profile", dist = 0.68, side = 1.5708, up = 0.0, drop = 0.03, fov = 30 },
	{ name = "bust", dist = 1.5, side = 0.35, up = 0.05, drop = 0.22, fov = 30 },
	{ name = "body", dist = 4.4, side = 0.35, up = -0.3, drop = 0.75, fov = 30 },
	{ name = "row", dist = 7.5, side = 0.2, up = 0.3, drop = 0.8, fov = 40 },
}

-- state.walk (m/s, set from a script or the console) walks everyone ahead.
local function place(i)
	return vector.create((i - 4.5) * KNOBS.spacing, 0, state.ahead or 0)
end

local function init()
	world.clear()
	world.spawn("cube", { pos = vector.create(0, -0.5, 0), size = vector.create(400, 1, 400), color = "#8a8d86", roughness = 0.9, tag = "floor" })
	state.people = {}
	state.person = state.person or 1
	state.view = state.view or 1
	state.turn = state.turn or 0
	for i = 1, 8 do
		-- A yaw of 0 faces +Z here, where the cameras stand.
		state.people[i] = humans.spawn("person", { pos = place(i), yaw = 0 })
	end
end

local function tick(_, input)
	local pad = input[1]
	if pad then
		if pad.pressed.right then state.person = state.person % 8 + 1 end
		if pad.pressed.left then state.person = (state.person - 2) % 8 + 1 end
		if pad.pressed.down then state.view = state.view % #VIEWS + 1 end
		if pad.pressed.up then state.view = (state.view - 2) % #VIEWS + 1 end
		if pad.pressed.south then state.turn = (state.turn + 1) % 4 end
	end
	state.ahead = (state.ahead or 0) + (state.walk or 0) / 60
	for i, id in state.people do
		local yaw = (if i == state.person then state.turn * math.pi / 2 else 0)
		humans.set(id, { pos = place(i), yaw = yaw, pose = state.pose or "idle" })
	end
end

local function draw()
	local v = VIEWS[state.view]
	local id = state.people[state.person]
	local head = id and humans.head(id)
	if head then
		local side = if v.name == "row" then 0.2 else v.side
		local centre = if v.name == "row" then vector.create(0, head.y, state.ahead or 0) else head
		local pos = centre + vector.create(math.sin(side) * v.dist, v.up, math.cos(side) * v.dist)
		camera.set(1, { pos = pos, target = centre - vector.create(0, v.drop, 0), fov = v.fov })
	end
	local w = hud.size()
	hud.text(`person {state.person}/8  {v.name}`, w - 16, 12, { size = 16, align = "right", color = "#ffffffaa" })
end

return { init = init, tick = tick, draw = draw, reload = init }
