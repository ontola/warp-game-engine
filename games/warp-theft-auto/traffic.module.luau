local util = require("kit/util")
local city = require("world")
local mobility = require("mobility")
local v = vector.create

-- Rectangular circuits in the right-hand lane. Stop short of each
-- intersection, then turn into the outgoing lane.
local function circuit(fleet, west, north, east, south, start, pace)
	local lane = 2.7
	local points = {
		v(west + lane, 0, north + lane), v(east - lane, 0, north + lane),
		v(east - lane, 0, south - lane), v(west + lane, 0, south - lane),
	}
	local i = start
	local goal = points[i % 4 + 1]
	local from = points[i]
	local pos
	for _,fraction in {.5,.34,.66,.22,.78} do
		local candidate=from+(goal-from)*fraction
		local clear=true
		for _,other in fleet.vehicles do
			if vector.magnitude(candidate-other.pos)<6 then clear=false; break end
		end
		if clear then pos=candidate; break end
	end
	assert(pos,"traffic circuit has no clear starting position")
	local car = mobility.add_car(fleet, pos, util.yaw_to(goal - pos))
	table.insert(fleet.traffic, #fleet.vehicles)
	local waiting = 0
	local stopped = false
	car.auto = function(vehicle, dt)
		local p = vehicle.pos
		local delta = util.flat(goal - p)
		local distance = vector.magnitude(delta)
		local error = util.angle_diff(util.yaw_to(delta), vehicle.yaw)
		local speed = math.abs(vehicle.speed or 0)
		local target = pace
		if distance < 13 and not stopped then
			target = math.max(0, (distance - 7) * 0.9)
			if speed < 0.7 and distance < 8.5 then
				waiting += dt
				if waiting >= 1.8 then
					stopped = true
					waiting = 0
				end
			end
		elseif stopped then
			target = 4
		end
		if distance < 4 and stopped then
			i = i % 4 + 1
			goal = points[i % 4 + 1]
			stopped = false
		end
		-- Yield to a vehicle directly ahead (and to people crossing the lane).
		local heading = util.forward(vehicle.yaw)
		local function inLane(other, range, width)
			local offset = util.flat(other - p)
			local along = vector.dot(heading, offset)
			return along > 0 and along < range and vector.magnitude(offset - heading * along) < width
		end
		for _, other in fleet.vehicles do
			local width = if other.auto or other.owner then 2.1 else 1.9
			if vehicle ~= other and inLane(other.pos, 7, width) and math.abs(other.pos.y - p.y) < 2.5 then target = 0 end
		end
		for _, rider in fleet.players do
			if rider.mode == "walk" and inLane(rider.pos, 7, 1.65) then target = 0 end
		end
		return {
			throttle = math.clamp((target - speed) * 0.23, 0, 0.65),
			brake = math.clamp((speed - target) * 0.4, 0, 1),
			steer = math.clamp(-error * 1.7, -1, 1), handbrake = if target < 0.1 and speed < 0.7 then 1 else 0,
		}
	end
end

local function add(fleet)
	local b = city.STEP
	fleet.traffic = {}
	-- Six circuits cover all districts. The second driver starts on the
	-- opposite side of each circuit, a half lap behind the first.
	local routes = {
		{-1,-1, 1,1, 4,8}, {1,-2, 3,1, 1,9}, {-3,0, -1,2, 2,7},
		{-3,-3, -1,-1, 3,8}, {0,1, 3,3, 4,8.5}, {-3,2, 0,3, 1,7.5},
	}
	for lap=0,1 do
		for _,r in routes do
			local start = (r[5]-1+2*lap)%4+1
			circuit(fleet,r[1]*b,r[2]*b,r[3]*b,r[4]*b,start,r[6]-.35*lap)
		end
	end
	fleet.baseCars=#fleet.vehicles
	fleet.lastCarId=fleet.baseCars
end

return { add = add }
