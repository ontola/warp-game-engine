-- Doors and their approach paths fit within the street block that owns them.
-- Each walker looks up only that block's one or two ground floors.
local city = require("world")
local room = require("room")
local interiors = {}
local ENTER, LEAVE = 4.5, 7

function interiors.new(buildings)
	local lots = {}
	for _, b in buildings do
		local key = city.block_index(b.x, b.z)
		lots[key] = lots[key] or {}
		table.insert(lots[key], b.id)
	end
	room.warm(buildings)
	return {lots = lots, active = {}, activities = {}, count = 0}
end

local function visit(stream, buildings, seat)
	seat.building, seat.doorway, seat.nearRoom = nil, nil, nil
	local pos = seat.pos
	if seat.mode ~= "walk" or pos.y < -0.5 or pos.y > 1.5 then return nil end
	local ids = stream.lots[city.block_index(pos.x, pos.z)]
	if not ids then return nil end

	local nearest, door, best = nil, nil, math.huge
	for _, id in ids do
		local b = buildings[id]
		if math.abs(pos.x - b.x) < b.w / 2 and math.abs(pos.z - b.z) < b.d / 2 then
			seat.building = id
			return id
		end
		local dx, dz = pos.x - b.door.x, pos.z - b.door.z
		local doorDistance = dx*dx + dz*dz + pos.y*pos.y
		local radius = if stream.active[id] then LEAVE else ENTER
		if math.abs(pos.x - b.x) > b.w/2 + radius or math.abs(pos.z - b.z) > b.d/2 + radius then continue end
		local distance = math.min(doorDistance, room.window_distance_sq(b, pos))
		if distance <= radius*radius and distance < best then
			nearest, best = id, distance
			door = if doorDistance <= radius*radius then id else nil
		end
	end
	seat.doorway, seat.nearRoom = door, nearest
	return nearest
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
	lights.room(b.id, {min = v(b.x-b.w/2, 0, b.z-b.d/2), max = v(b.x+b.w/2, b.h, b.z+b.d/2),
		entities = pieces, panels = panels})
end

function interiors.update(stream, buildings, seats)
	local wanted = {}
	for _, seat in seats do
		local id = visit(stream, buildings, seat)
		if id then wanted[id] = true end
	end

	-- Release first so a four-player move between city blocks never needs eight rooms.
	for id, pieces in stream.active do
		if not wanted[id] then
			lights.room(id, nil)
			for _, piece in pieces do world.remove(piece) end
			world.set(buildings[id].shell, {visible = true, solid = true})
			stream.active[id], stream.activities[id] = nil, nil
			stream.count -= 1
		end
	end
	for id in wanted do
		if not stream.active[id] then
			local b = buildings[id]
			stream.active[id], stream.activities[id] = room.build(b)
			world.set(b.shell, {visible = false, solid = false})
			lightRoom(b, stream.active[id])
			stream.count += 1
		end
	end
end

return interiors
