local util = require("kit/util")
local v = vector.create
local ambience = {}

local function inEarshot(pos, seats, range)
	for _, seat in seats do
		if vector.magnitude(seat.pos - pos) < range then return true end
	end
	return false
end

-- Car voices stay attached to their emitters across every listener's view.
function ambience.tick(fleet)
	for _, car in fleet.vehicles do
		local audible = (car.owner or car.auto) and inEarshot(car.pos, fleet.players, if car.voice then 58 else 50)
		if audible then
			local engine = car.kind == "car"
			if not car.voice then car.voice = sound.play(if engine then "motor" else "rotor", { at = car.pos, db = if engine then -1 else -4 }) end
			sound.move_to(car.voice, car.pos)
			sound.set_param(car.voice, "speed", math.abs(car.speed or 0))
			if engine then
				sound.set_param(car.voice, "rpm", car.rpm or 900)
				sound.set_param(car.voice, "throttle", car.throttle or 0)
				sound.set_param(car.voice, "load", car.load)
				sound.set_param(car.voice, "gear", car.gear or 1)
				sound.set_param(car.voice, "cut", if car.cut then 1 else 0)
				sound.set_param(car.voice, "slip", car.slip)
				sound.set_param(car.voice, "contact", car.contact)
				sound.set_param(car.voice, "loose", car.loose)
				sound.set_param(car.voice, "soft", car.soft)
			end
		elseif car.voice then
			sound.stop(car.voice)
			car.voice = nil
		end
	end
end

function ambience.headlights(seats, clock)
	if clock.lampLevel == 0 and not clock.headlights then return end
	clock.headlights = clock.lampLevel > 0
	local beams = {}
	if clock.headlights then
		for _, seat in seats do
			if not seat.vehicle then continue end
			local forward = util.forward(seat.yaw)
			local aircraft = seat.mode == "fly"
			local mount = seat.pos + forward * (if aircraft then 1.4 else 2.05) + v(0, if aircraft then 0.7 else 0.64, 0)
			table.insert(beams, {pos = mount, dir = forward + v(0, if aircraft then -0.7 else -0.13, 0),
				color = "#ffebcc", intensity = 6200 * clock.lampLevel, angle = 25, range = 48})
		end
	end
	lights.spots(beams)
end

function ambience.lamps(city, seats, clock)
	-- Streets keep a night-time pool of light around each occupied district.
	local lamps = {}
	if clock.lampLevel > 0 then
		local selected = {}
		for _, seat in seats do
			local near = {}
			for index, pos in city.lamps do
				local distance = vector.magnitude(pos - seat.pos)
				if distance < 68 then table.insert(near, {distance = distance, index = index}) end
			end
			table.sort(near, function(a,b) return a.distance < b.distance end)
			for i = 1, math.min(5,#near) do
				local index = near[i].index
				if not selected[index] then
					selected[index] = true
					table.insert(lamps, {pos = city.lamps[index], color = "#ffc88f", intensity = 580 * clock.lampLevel, range = 23})
				end
			end
		end
	end
	lights.set(lamps)
end

return ambience
