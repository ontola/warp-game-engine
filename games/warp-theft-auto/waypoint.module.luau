-- A single world-space objective for the shared city. Children are in metres
-- relative to the target so a moving actor keeps the same marker in every view.
local dispatch = require("dispatch")
local waypoint = {}
local v = vector.create
local gold = {color="#ffc244", emissive=.68, roughness=1}
local diamond, circle = "outline4:0.10", "outline32:0.026"

function waypoint.warm()
	for _, shape in {diamond, circle, "cube"} do world.warm(shape, gold) end
end

local function part(root, shape, tag, pos, size, rot)
	local o = table.clone(gold)
	o.tag, o.pos, o.size, o.rot, o.solid = tag, pos, size, rot, false
	return world.spawn(shape, o, root)
end

local function build(pos, kind, radius)
	local root = world.spawn("cube", {tag="objective-anchor", pos=pos, solid=false, visible=false})
	if radius then
		part(root, circle, "objective-zone", v(0,.21,0), v(radius*2,.12,radius*2))
	end
	if kind == "checkpoint" then
		-- A checkpoint accepts crossings from every direction. Its four corners
		-- stay open to vehicles and mark the same radius as the crossing test.
		for i=0,3 do
			local angle = math.pi * (.25 + .5*i)
			part(root, "cube", "objective-post", v(math.cos(angle)*radius,1.6,math.sin(angle)*radius), v(.22,3,.22))
		end
	end
	local height = if radius then 5.0 else 3.05
	local width = if radius then 2.35 else 1.05
	for i=0,1 do
		part(root, diamond, "objective-beacon", v(0,height,0), v(width,.10,width*1.32), v(math.pi/2,i*math.pi/2,0))
	end
	return root
end

function waypoint.update(state)
	local pos, kind, radius, key
	local story = state.story
	if story then
		if story.status == "active" and story.target then
			pos, kind, key = story.target, story.action, story.step
			if kind == "checkpoint" or kind == "dropoff" then radius = story.radius end
		end
	elseif state.job.pause == 0 then
		local goal = dispatch.stop(state.job, state.fleet)
		pos, kind, key = goal.pos, goal.kind, goal.name
		if kind ~= "recover" then radius = dispatch.radius(kind) end
	end

	local current = state.waypoint
	-- Replace the geometry between objectives so interpolation never sweeps a
	-- checkpoint across the city. A walking target keeps its existing geometry.
	if current and (not pos or current.kind ~= kind or current.radius ~= radius
		or current.key ~= key or vector.magnitude(pos-current.pos) > 8) then
		world.remove(current.root)
		state.waypoint, current = nil, nil
	end
	if not pos then return end
	if not current then
		state.waypoint = {root=build(pos,kind,radius), kind=kind, radius=radius, pos=pos, key=key}
	elseif pos ~= current.pos then
		world.place(current.root, pos)
		current.pos = pos
	end
end

return waypoint
