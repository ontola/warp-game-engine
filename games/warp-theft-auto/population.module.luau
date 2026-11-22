local crowd = require("kit/crowd")
local population = {}

-- Planted maritime pines. Two crown shapes share their cooked parts across
-- the whole city; placement scale and yaw follow each individual site.
local PINE = {
	height = 14, pace = 9, shape = 1.1, whorl = {3, 4}, laterals = 0.55,
	max_order = 2, limb_step = 0.42, side_length = 0.7, crowding = 4,
	wood = 0.015, tip_radius = 0.012, plumes = 1.35, needle_years = 4,
}
local NEEDLES = {
	brush_needles = 12, needle_length = 0.52, needle_width = 0.019,
	brush_reach = 0.24, needles = {0.18, 0.37, 0.22},
}

local function plant_city(sites, seed)
	for i, pos in ipairs(sites) do
		local phase = (seed % 283 + i * 173) % 283
		plants.grow({
			pos = pos, species = PINE, look = NEEDLES, age = 9,
			seed = 17 + i % 2, scale = 0.74 + 0.4 * phase / 283,
			yaw = phase * 2 * math.pi / 283,
		})
	end
end

-- A daily walker follows a closed sidewalk, pausing briefly at the corners.
-- Timed segments let faraway residents advance without a per-frame actor.
local function commuter(loop, index, seed)
	local hash = (index * 119 + seed % 257) % 257
	local speed = 1.03 + hash / 257 * 0.65
	local pause = (index % 4) * 0.85
	local direction = if index % 2 == 0 then -1 else 1
	local legs, duration = {}, 0
	for k = 1, #loop do
		local first = ((k - 1) * direction) % #loop + 1
		local a = loop[first]
		local b = loop[(first - 1 + direction) % #loop + 1]
		local delta = b - a
		local travel = vector.magnitude(delta) / speed
		if travel > 0.001 then
			table.insert(legs, {a = a, delta = delta, travel = travel, start = duration, yaw = math.atan2(delta.x, delta.z)})
			duration += travel + pause
		end
	end
	if #legs == 0 then return nil end
	return {legs = legs, period = duration, offset = duration * hash / 257, pos = legs[1].a, yaw = legs[1].yaw}
end

local function sample(c, time)
	local t = (time + c.offset) % c.period
	for i = #c.legs, 1, -1 do
		local leg = c.legs[i]
		if t >= leg.start then
			local f = math.min((t - leg.start) / leg.travel, 1)
			c.pos = leg.a + leg.delta * f
			c.yaw = leg.yaw
			return
		end
	end
end

function population.new(city, seed)
	plant_city(city.plantSites, seed)
	local residents = {}
	for i, loop in ipairs(city.walkLoops) do
		for shift = 1, 2 do
			local c = commuter(loop, (i - 1) * 2 + shift, seed)
			if c then table.insert(residents, c) end
		end
	end
	return {residents = residents, pedestrians = crowd.new("resident", 72), avatars = {}, elapsed = 0, selectionIn = 0}
end

function population.tick(pop, city, seats, dt, daylight)
	pop.elapsed += dt
	pop.selectionIn -= dt
	local ids = {}
	for id in seats do table.insert(ids, id) end
	table.sort(ids)
	local observers = {}
	for _, id in ipairs(ids) do
		local seat = seats[id]
		local model = pop.avatars[id]
		if not model then
			model = humans.spawn("local", {pos = seat.pos, yaw = seat.yaw + math.pi})
			pop.avatars[id] = model
		end
		if model then
			-- Retain identity in the cabin, including when two people swap cars.
			humans.set(model, {pos = seat.pos, yaw = seat.yaw + math.pi, visible = seat.mode == "walk"})
		end
		if not seat.bot then table.insert(observers, seat.pos) end
	end
	if #observers == 0 and ids[1] then table.insert(observers, seats[ids[1]].pos) end
	for id, model in pop.avatars do
		if not seats[id] then
			humans.remove(model)
			pop.avatars[id] = nil
		end
	end

	if pop.selectionIn <= 0 then
		pop.selectionIn = 0.35
		for id, c in ipairs(pop.residents) do
			sample(c, pop.elapsed)
			-- After dark, daytime walkers go home once out of sight.
			c.active = daylight or id % 3 == 1 or pop.pedestrians.active[id] ~= nil
		end
		pop.pedestrians:select(pop.residents, observers)
	else
		for id in pop.pedestrians.active do sample(pop.residents[id], pop.elapsed) end
	end
	pop.pedestrians:update(pop.residents)
end

return population
