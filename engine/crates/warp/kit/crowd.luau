-- A bounded set of Warp Humans for an arbitrary population. Candidate array
-- indices are stable identities; a streaming game can leave gaps when it
-- unloads part of the world. A game owns their paths, poses and clock.
local crowd = {}
local methods = {}
methods.__index = methods

function crowd.new(pool, radius)
	return setmetatable({pool = pool, radius = radius, active = {}}, methods)
end

-- Call periodically with candidate feet positions and the observers' world
-- positions. Each observer gets a turn at the nearest unclaimed person.
-- An optional rank(candidate, observerIndex, squaredDistance) can prioritize
-- or exclude candidates for each view. Return nil to exclude; lower is closer.
function methods:select(candidates, observers, rankCandidate)
	local free, capacity = humans.room(self.pool)
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
			while list[row.next] and chosen[list[row.next].id] do row.next += 1 end
			local entry = list[row.next]
			if entry then
				chosen[entry.id] = true
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
		self.active[id] = humans.spawn(self.pool, {pos = c.pos, yaw = c.yaw})
	end
end

-- Call after moving the selected candidates each tick. yaw follows humans.set
-- (+Z is 0), and pose defaults to Warp Humans' distance-driven walk.
function methods:update(candidates)
	for id, actor in self.active do
		local c = candidates[id]
		if c then
			humans.set(actor, {pos = c.pos, yaw = c.yaw, pose = c.pose})
		else
			humans.remove(actor)
			self.active[id] = nil
		end
	end
end

return crowd
