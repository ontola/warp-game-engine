-- A bounded set of Warp Humans for an arbitrary population. Candidate array
-- indices are stable identities. A game owns their paths, poses and clock.
local crowd = {}
local methods = {}
methods.__index = methods

function crowd.new(pool, radius)
	return setmetatable({pool = pool, radius = radius, active = {}}, methods)
end

-- Call periodically with candidate feet positions and the observers' world
-- positions. Each observer gets a turn at the nearest unclaimed person.
function methods:select(candidates, observers)
	local free, capacity = humans.room(self.pool)
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
				local rank = if self.active[id] then distance2 * 0.8 else distance2
				table.insert(list, {id = id, rank = rank})
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
	for id, candidate in ipairs(candidates) do
		if chosen[id] and not self.active[id] then
			self.active[id] = humans.spawn(self.pool, {pos = candidate.pos, yaw = candidate.yaw})
		end
	end
end

-- Call after moving the selected candidates each tick. yaw follows humans.set
-- (+Z is 0), and pose defaults to Warp Humans' distance-driven walk.
function methods:update(candidates)
	for id, actor in self.active do
		local c = candidates[id]
		humans.set(actor, {pos = c.pos, yaw = c.yaw, pose = c.pose})
	end
end

return crowd
