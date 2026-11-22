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

-- A phase on a shared sidewalk loop determines a pedestrian's position even
-- after a long absence. Two parallel lanes let companions walk abreast.
local v = vector.create
local function sampleNoise(seed, index, salt)
	local n = (seed + index*7385609 + salt*834927) % 2147483647
	return (((n*48271)%2147483647)*16807)%2147483647 / 2147483647
end

local function sidewalk(loop, lane, direction)
	local center, points = (loop[1]+loop[3])*.5, {}
	for i = 1, 4 do
		local p = loop[1 + ((i-1)*direction)%4]
		table.insert(points, p + v(math.sign(p.x-center.x)*lane, 0, math.sign(p.z-center.z)*lane))
	end
	local legs = {}
	for i, a in points do
		local delta = points[i%4+1] - a
		table.insert(legs, {a=a, delta=delta, yaw=math.atan2(delta.x, delta.z)})
	end
	return legs
end

local function addBlock(pop, loop, block, seed)
	local center = (loop[1]+loop[3])*.5
	local firstPath = #pop.paths+1
	for _, direction in {-1,1} do
		for _, lane in {-.38,.40} do table.insert(pop.paths, sidewalk(loop,lane,direction)) end
	end
	local count = if math.abs(center.x)<57 and math.abs(center.z)<60 then 21
		elseif center.x>110 then 12 else 16
	local shift = sampleNoise(seed,block,27)
	for pair = 1, count do
		local key = block*32 + pair
		local speed = .90 + sampleNoise(seed,key,3)*.95
		local walkTime = vector.magnitude(loop[2]-loop[1])/speed
		local segment = walkTime + .45 + 1.7*sampleNoise(seed,key,17)
		local phase = 4*segment*(shift + (pair-1+.4*sampleNoise(seed,key,9))/count)
		local direction = if pair%2==0 then 0 else 2
		local friends = pair%6==1
		for person=1,if friends then 2 else 1 do
			local lane = if friends then person-1 else direction/2
			table.insert(pop.residents, {
				role=if friends then "companions" else "commuter", path=firstPath+direction+lane,
				phase=phase, walkTime=walkTime, segment=segment, pos=center, yaw=0,
				night=sampleNoise(seed,key,15)<.54,
			})
		end
	end
end

local function social(pop, parks, seed)
	for i, center in parks do
		for group=1,3 do
			local x = center.x + (if group==1 then -6.2 elseif group==2 then 7.1 else -4.6)
			local z = center.z + (if group==1 then 8.5 elseif group==2 then -6.9 else -8.1)
			for side=-1,1,2 do
				table.insert(pop.residents, {
					role="neighbour", pos=v(x+.7*side,0,z), yaw=-side*math.pi/2,
					action=if side>0 then "wave" else "idle", phase=sampleNoise(seed,i*4+group,side)*11,
					night=group~=2,
				})
			end
		end
	end
end

function population.new(city, seed)
	plant_city(city.plantSites, seed)
	local pop = {residents={}, paths={}, rooms={}, avatars={}, pedestrians=crowd.new("resident",72), elapsed=0, selectionIn=0}
	for block, loop in city.walkLoops do addBlock(pop,loop,block,seed) end
	social(pop,city.parks,seed)
	pop.streetCount = #pop.residents
	return pop
end

-- Activity anchors and furniture share the building generator. Their identities
-- are per lot, so two visitors see the same workers and guests. The city clock
-- continues their routines when a room is revisited.
local function localResidents(pop, interiors, city)
	local changed = false
	for id, ids in pop.rooms do
		if not interiors.active[id] then
			for _, index in ids do pop.residents[index] = nil end
			pop.rooms[id] = nil
			changed = true
		end
	end
	for id, anchors in interiors.activities do
		if pop.rooms[id] then continue end
		local ids = {}
		local b = city.buildings[id]
		for i, anchor in anchors do
			local index = pop.streetCount + (i-1)*#city.buildings + id
			local phase = sampleNoise(b.seed,i,3)
			local c = {building=id, role=anchor.role, action=anchor.pose, pos=anchor.pos,
				yaw=anchor.yaw, phase=phase*17, night=b.kind=="hotel" or i%4~=0}
			if anchor.to then
				c.a, c.delta = anchor.pos, anchor.to-anchor.pos
				c.walkTime = vector.magnitude(c.delta)/(.8+.48*phase)
				c.segment = c.walkTime + 1.5 + phase*3
				c.phase = phase*c.segment*2
				c.heading = math.atan2(c.delta.x,c.delta.z)
			end
			pop.residents[index] = c
			table.insert(ids,index)
		end
		pop.rooms[id] = ids
		changed = true
	end
	return changed
end

local function sample(pop, c)
	local t = pop.elapsed+c.phase
	if c.path then
		local leg = pop.paths[c.path][1 + math.floor(t/c.segment)%4]
		local fraction = math.min(1,(t%c.segment)/c.walkTime)
		c.pos, c.yaw = leg.a+leg.delta*fraction, leg.yaw
		c.pose = if fraction<1 then "walk" else "idle"
	elseif c.delta then
		local returning = math.floor(t/c.segment)%2==1
		local fraction = math.min(1,(t%c.segment)/c.walkTime)
		c.pos = c.a+c.delta*(if returning then 1-fraction else fraction)
		c.yaw = c.heading+(if returning then math.pi else 0)
		c.pose = if fraction<1 then "walk" else "idle"
	elseif c.action=="reach" or c.action=="wave" then
		c.pose = if t%7.5 < (if c.action=="reach" then 2.8 else 1.1) then c.action else "idle"
	else
		c.pose = c.action or "idle"
	end
end

local function updateAvatars(pop, seats)
	local ids, people, observers = {}, {}, {}
	for id in seats do table.insert(ids,id) end
	table.sort(ids)
	for _, id in ids do
		local seat = seats[id]
		local model = pop.avatars[id]
		if not model then
			model = humans.spawn("local", {pos=seat.pos,yaw=seat.yaw+math.pi})
			pop.avatars[id] = model
		end
		if model then humans.set(model, {pos=seat.pos,yaw=seat.yaw+math.pi,visible=seat.mode=="walk"}) end
		if not seat.bot then
			table.insert(people,seat)
			table.insert(observers,seat.pos)
		end
	end
	if #people==0 and ids[1] then
		table.insert(people,seats[ids[1]])
		table.insert(observers,seats[ids[1]].pos)
	end
	for id, model in pop.avatars do
		if not seats[id] then humans.remove(model); pop.avatars[id] = nil end
	end
	return people, observers
end

function population.tick(pop, city, seats, dt, daylight, interiors)
	pop.elapsed += dt
	pop.selectionIn -= dt
	local people, observers = updateAvatars(pop,seats)
	local changed = localResidents(pop,interiors,city)
	if changed or pop.selectionIn<=0 then
		pop.selectionIn = .40
		for id, c in pop.residents do
			sample(pop,c)
			c.active = daylight or c.night or pop.pedestrians.active[id]~=nil
		end
		-- Each view claims its own nearest people in turn. Prefer the visitor's
		-- room and the one visible through nearby glass or a doorway. Spare slots still
		-- draw the street outside its windows.
		local function rank(c, view, distance2)
			local seat = people[view]
			if c.building then
				if c.building~=seat.building and c.building~=seat.nearRoom then return nil end
				return distance2*.24
			end
			if seat.building then return distance2+530 end
			return distance2
		end
		pop.pedestrians:select(pop.residents,observers,rank)
	else
		for id in pop.pedestrians.active do sample(pop,pop.residents[id]) end
	end
	pop.pedestrians:update(pop.residents)
end

return population
