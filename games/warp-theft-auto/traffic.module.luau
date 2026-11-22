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
	local pos = (from + goal) * 0.5
	local car = mobility.add_car(fleet, pos, util.yaw_to(goal - pos))
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
		local function inLane(other, range)
			local offset = util.flat(other - p)
			local along = vector.dot(heading, offset)
			return along > 0 and along < range and vector.magnitude(offset - heading * along) < 2.2
		end
		for _, other in fleet.vehicles do
			if vehicle ~= other and inLane(other.pos, 7) and math.abs(other.pos.y - p.y) < 2.5 then target = 0 end
		end
		for _, rider in fleet.players do
			if rider.mode == "walk" and inLane(rider.pos, 7) then target = 0 end
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
	circuit(fleet, -b, -b, b, b, 4, 8)
	circuit(fleet, b, -2*b, 3*b, b, 1, 9)
	circuit(fleet, -3*b, 0, -b, 2*b, 2, 7)
end

return { add = add }
