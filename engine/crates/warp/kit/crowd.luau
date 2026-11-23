-- A bounded set of Warp Humans for an arbitrary population. Candidate array
-- indices are stable identities; a streaming game can leave gaps when it
-- unloads part of the world. A game owns their paths, poses and clock.
local crowd = {}
local methods = {}
methods.__index = methods

function crowd.new(pool, radius, stableLooks)
	return setmetatable({pool = pool, radius = radius, stableLooks = stableLooks, active = {}}, methods)
end

-- Call periodically with candidate feet positions and the observers' world
-- positions. Each observer gets a turn at the nearest unclaimed person.
-- An optional rank(candidate, observerIndex, squaredDistance) can prioritize
-- or exclude candidates for each view. Return nil to exclude; lower is closer.
function methods:select(candidates, observers, rankCandidate)
	local free, capacity, variety, looks = humans.room(self.pool)
	-- A device can fit fewer skins than the initially configured CPU pool.
	-- Reclaim the old slot assignments once so no selected actor is off-budget.
	if self.capacity and capacity < self.capacity then
		for _, actor in self.active do humans.remove(actor) end
		table.clear(self.active)
		free = humans.room(self.pool)
	end
	self.capacity = capacity
	local owned = 0
	for _ in self.active do owned += 1 end
	local limit = math.min(capacity, free + owned)
	local radius2 = self.radius * self.radius
	local remaining = {}
	if self.stableLooks then
		-- Slot i holds appearance i % variety. Reserve that share during
		-- selection so a busy appearance never displaces a nearer eligible one.
		for look = 0, variety-1 do remaining[look] = math.floor((capacity+variety-1-look)/variety) end
		for id, c in candidates do
			local key = id-1 + (if type(self.stableLooks)=="number" then self.stableLooks % 9973 else 0)
			c.look = (key*31 + math.floor(key/8) + math.floor(key/64)) % looks
		end
		if free + owned < capacity then
			for look = 0, variety-1 do remaining[look] = humans.room(self.pool,look) end
			for id in self.active do
				local c = candidates[id]
				if c then remaining[c.look % variety] += 1 end
			end
		end
	end
	local lists = {}
	for j, observer in observers do
		local list = {}
		for id, candidate in candidates do
			local offset = candidate.pos - observer
			local distance2 = vector.dot(offset, offset)
			if candidate.active ~= false and distance2 <= radius2 then
				-- Keep a fitted person at the selection boundary until another
				-- candidate is clearly closer, to avoid flickering between them.
				local rank = distance2
				if rankCandidate then rank = rankCandidate(candidate, j, distance2) end
				if rank ~= nil then
					if self.active[id] then rank -= math.abs(rank)*0.2 end
					table.insert(list, {id = id, rank = rank})
				end
			end
		end
		table.sort(list, function(a, b)
			return a.rank < b.rank or (a.rank == b.rank and a.id < b.id)
		end)
		lists[j] = {list = list, next = 1}
	end
	local chosen, count = {}, 0
	while count < limit do
		local before = count
		for _, row in lists do
			local list = row.list
			local entry = list[row.next]
			while entry and (chosen[entry.id] or (self.stableLooks and remaining[candidates[entry.id].look % variety] == 0)) do
				row.next += 1
				entry = list[row.next]
			end
			if entry then
				chosen[entry.id] = true
				if self.stableLooks then remaining[candidates[entry.id].look % variety] -= 1 end
				count += 1
				row.next += 1
				if count == limit then break end
			end
		end
		if count == before then break end
	end
	for id, actor in self.active do
		if not chosen[id] then
			humans.remove(actor)
			self.active[id] = nil
		end
	end
	-- Stable allocation makes fitted appearances deterministic even when the
	-- candidate set has holes from unloaded rooms.
	local arrivals = {}
	for id in chosen do
		if not self.active[id] then table.insert(arrivals, id) end
	end
	table.sort(arrivals)
	for _, id in arrivals do
		local c = candidates[id]
		self.active[id] = humans.spawn(self.pool, {pos = c.pos, yaw = c.yaw, look = if self.stableLooks then c.look else nil, resident = if self.stableLooks then id else nil})
	end
end

-- Fit an immediately interacting resident between periodic selections. A full
-- pool can lend the farthest compatible slot outside every observer's reach.
function methods:allocate(opts, candidates, observers)
	local _, _, variety = humans.room(self.pool)
	local actor = humans.spawn(self.pool, opts)
	if not actor then
		local farthest, best = nil, 25
		for other in self.active do
			local person = candidates[other]
			if not person or person.externalPose or person.look % variety ~= opts.look % variety then continue end
			local nearest = math.huge
			for _, pos in observers do
				local delta = person.pos-pos
				nearest = math.min(nearest, vector.dot(delta,delta))
			end
			if nearest > best or (nearest == best and farthest and other < farthest) then farthest, best = other, nearest end
		end
		if not farthest then return nil end
		humans.remove(self.active[farthest])
		self.active[farthest] = nil
		actor = humans.spawn(self.pool, opts)
	end
	return actor
end

function methods:promote(id, candidates, observers)
	if self.active[id] then return self.active[id] end
	local c = candidates[id]
	if not c or not self.stableLooks then return nil end
	local actor = self:allocate({pos=c.pos, yaw=c.yaw, look=c.look, resident=id}, candidates, observers)
	self.active[id] = actor
	return actor
end

-- Call after moving the selected candidates each tick. yaw follows humans.set
-- (+Z is 0), and pose defaults to Warp Humans' distance-driven walk.
function methods:update(candidates)
	for id, actor in self.active do
		local c = candidates[id]
		if c then
			if not c.externalPose then humans.set(actor, {pos = c.pos, yaw = c.yaw, pose = c.pose}) end
		else
			humans.remove(actor)
			self.active[id] = nil
		end
	end
end

return crowd
