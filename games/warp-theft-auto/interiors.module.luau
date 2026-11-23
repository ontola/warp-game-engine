-- Doors and their approach paths fit within the street block that owns them.
-- Each walker looks up only that block's one or two ground floors.
local city = require("world")
local room = require("room")
local interiors = {}
local ENTER, LEAVE, OPEN = 4.5, 7, 1.25
local BIAS = vector.create(.02,.02,.02)

function interiors.new(buildings)
	local lots, warmed = {}, {}
	for _, b in buildings do
		local key = city.block_index(b.x, b.z)
		lots[key] = lots[key] or {}
		table.insert(lots[key], b.id)
		local surface = b.proxySurface or world.get(b.shell).surface
		local look = b.color .. tostring(surface)
		if not warmed[look] then
			world.warm("cube",{color=b.color,surface=surface,coverage=1})
			warmed[look] = true
		end
	end
	room.warm(buildings)
	return {lots = lots, active = {}, activities = {}, facades = {}, count = 0}
end

local function smooth(x)
	x = math.clamp(x,0,1)
	return x*x*(3-2*x)
end

-- Full detail just before the opening. Details are completely covered at
-- the load radius, including on the way back through the hysteresis band.
local function detail(distance)
	return smooth((ENTER-distance)/(ENTER-OPEN))
end

local function visit(stream, buildings, seat)
	seat.building, seat.doorway, seat.nearRoom = nil, nil, nil
	seat.doorBlend = 0
	local pos = seat.pos
	if seat.mode ~= "walk" or pos.y < -0.5 or pos.y > 1.5 then return nil end
	local ids = stream.lots[city.block_index(pos.x, pos.z)]
	if not ids then return nil end

	local nearest, door, best, second = nil, math.huge, math.huge, math.huge
	for _, id in ids do
		local b = buildings[id]
		if math.abs(pos.x-b.x)<b.w/2 and math.abs(pos.z-b.z)<b.d/2 then
			seat.building, seat.doorBlend = id, 1
			return id, 0
		end
		local radius = if stream.active[id] then LEAVE else ENTER
		if math.abs(pos.x-b.x)>b.w/2+radius or math.abs(pos.z-b.z)>b.d/2+radius then continue end
		local dx, dz = pos.x-b.door.x, pos.z-b.door.z
		local doorSq = dx*dx + dz*dz + pos.y*pos.y
		local distanceSq = math.min(doorSq, room.window_distance_sq(b,pos))
		if distanceSq > radius*radius then continue end
		if distanceSq < best then
			second, best, nearest, door = best, distanceSq, id, doorSq
		else second = math.min(second,distanceSq) end
	end
	if not nearest then return nil end
	local radius = if stream.active[nearest] then LEAVE else ENTER
	seat.doorway = if door <= radius*radius then nearest else nil
	seat.nearRoom = nearest
	-- A player between two neighboring hotel windows can only stream one
	-- furnished room. Both views converge on their cheap facade at the bisector.
	local distance = math.sqrt(best)
	local handoff = smooth((math.sqrt(second)-distance)/1.25)
	seat.doorBlend = detail(math.sqrt(door))*handoff
	local remaining = 1-detail(distance)*handoff
	return nearest, math.floor(remaining*255+.5)
end

-- The original shell shows the most opaque view. Each less opaque view
-- that still needs shell pixels gets a private GPU coverage mask. Most
-- rooms need no extra entities at all, including one fully entered alone.
local function cover(b, facade, views)
	local highest = 0
	for _, view in views do
		highest = math.max(highest, if view.room==b.id then view.coverage else 255)
	end
	local hide = 0
	for i, view in views do
		local coverage = if view.room==b.id then view.coverage else 255
		local bit = 2^i
		if coverage<highest then hide += bit end
		local copy = facade.copies[i]
		if coverage>0 and coverage<highest then
			if not copy then
				local id = world.spawn("cube",{pos=vector.create(b.x,b.h/2,b.z), size=facade.size,
					color=b.color,surface=facade.surface,coverage=coverage/255,
					hide_views=255-bit,solid=false})
				facade.copies[i] = {id=id,coverage=coverage}
			elseif copy.coverage~=coverage then
				world.set(copy.id,{coverage=coverage/255})
				copy.coverage=coverage
			end
		elseif copy then world.remove(copy.id); facade.copies[i]=nil end
	end
	for i, copy in facade.copies do
		if not views[i] then world.remove(copy.id); facade.copies[i]=nil end
	end
	if highest~=facade.coverage or hide~=facade.hide then
		world.set(b.shell,{size=facade.size,solid=false,visible=highest>0,hide_views=hide,coverage=highest/255})
		facade.coverage, facade.hide = highest, hide
	end
end

local function lightRoom(b, pieces)
	local v = vector.create
	local color = if b.kind == "hotel" then "#ffce99" elseif b.kind == "office" then "#fff0d9" else "#dfedff"
	local panels = {}
	for _, x in {-1, 1} do
		for _, z in {-1, 1} do
			table.insert(panels, {pos = v(b.x + x*b.w*.23, b.h-.29, b.z + z*b.d*.22),
				width = 1.5, depth = .6, color = color,
				intensity = if b.kind == "hotel" then 700 else 1250, range = 18})
		end
	end
	local lower, upper = room.light_bounds(b)
	lights.room(b.id, {min = lower, max = upper,
		entities = pieces, panels = panels})
end

function interiors.release_unused(stream, buildings, seats)
	local wanted, views = {}, {}
	-- View indices are reassigned when the roster changes. Player IDs survive
	-- those changes and cannot be used as bits in the renderer's view mask.
	for _, info in players.list() do
		local seat = seats[info.id]
		if not seat then continue end
		local id, coverage = visit(stream, buildings, seat)
		if id then wanted[id] = true end
		if info.view~=nil and not views[info.view] then
			views[info.view] = {room=id,coverage=coverage}
		end
	end
	stream.facades = stream.facades or {}
	-- Release first so a four-player move between city blocks never needs eight rooms.
	for id, pieces in stream.active do
		if not wanted[id] then
			lights.room(id, nil)
			for _, piece in pieces do world.remove(piece) end
			local facade, b = stream.facades[id], buildings[id]
			if facade then
				for _, copy in facade.copies do world.remove(copy.id) end
				stream.facades[id] = nil
			end
			world.set(b.shell,{size=vector.create(b.w,b.h,b.d),visible=true,solid=true,hide_views=0,coverage=false})
			stream.active[id], stream.activities[id] = nil, nil
			stream.count -= 1
		end
	end
	return wanted, views
end

function interiors.load_visible(stream, buildings, wanted, views)
	for id in wanted do
		local b = buildings[id]
		if not stream.active[id] then
			stream.active[id], stream.activities[id] = room.build(b)
			lightRoom(b, stream.active[id])
			stream.count += 1
		end
		if not stream.facades[id] then
			stream.facades[id] = {surface=b.proxySurface or world.get(b.shell).surface,
				size=vector.create(b.w,b.h,b.d)+BIAS,copies={}}
		end
		cover(b,stream.facades[id],views)
	end
end

return interiors
