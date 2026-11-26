local util = require("kit/util")
local detachment = require("detachment")
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
		local audible = not car.wrecked and (car.owner or car.auto) and inEarshot(car.pos, fleet.players, if car.voice then 58 else 50)
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
			else
				-- Local split screen shares its speakers. The occupied optic is the ear inside this rotor.
				local gunner = fleet.players[car.gunner]
				sound.set_param(car.voice, "cabin", if gunner and not gunner.bot then 1 else 0)
			end
		elseif car.voice then
			sound.stop(car.voice)
			car.voice = nil
		end
	end
end

-- One combined beam for the Toyama. Its foremost bumper vertex is at local Z = -2.221 m.
local CAR_LAMP = v(0,.31,-2.29)
local CAR_AIM = v(0,-.13,-1)
local function carHeadlight(car, level)
	local beam = {parent = car.body, pos = CAR_LAMP, dir = CAR_AIM,
		color = "#ffebcc", intensity = 6200 * level, angle = 25, range = 48}
	if not lights.supports_parent then
		-- Older hosts take world-space spots only. When reversing, the displayed
		-- bumper may be as far forward as its previous tick. Clear that pose too.
		beam.parent = nil
		beam.pos = detachment.worldPoint(car, CAR_LAMP)
		beam.dir = detachment.worldPoint(car, CAR_LAMP + CAR_AIM) - beam.pos
		if car.prevPos then
			local previous = car.prevPos - car.pos
			-- The slot renderer snaps movements over four metres in one tick.
			if vector.magnitude(previous) <= 4 then
				local forward = detachment.worldPoint(car, CAR_LAMP + v(0,0,-1)) - beam.pos
				beam.pos += forward * math.max(0, vector.dot(previous, forward))
			end
		end
	end
	return beam
end

function ambience.headlights(fleet, clock)
	local beams = {}
	for _, seat in fleet.players do
		if not seat.vehicle then continue end
		local forward = util.forward(seat.yaw)
		if seat.mode == "fly" and seat.flightRole ~= "gunner" then
			local heli = fleet.vehicles[seat.vehicle]
			if not heli.searchlight then continue end
			local nose = world.get(heli.searchlight).world_pos
			-- The lamp follows the nose in yaw but keeps illuminating the ground during pitch and roll.
			table.insert(beams, {pos = nose, dir = forward + v(0, -1.05, 0),
				color = "#ffefd9", intensity = 185000, angle = 15, inner = 9, range = 185})
		elseif seat.mode == "drive" and clock.lampLevel > 0 then
			table.insert(beams, carHeadlight(fleet.vehicles[seat.vehicle], clock.lampLevel))
		end
	end
	if #beams > 0 or clock.headlights then lights.spots(beams) end
	clock.headlights = #beams > 0
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
	clock.streetLights,clock.streetLightsChanged=lamps,true
end

function ambience.weaponLights(clock,players)
	local combined
	for id=1,4 do
		local p=players[id]
		if p and p.muzzleLight and p.tip then
			combined=combined or table.clone(clock.streetLights or {})
			table.insert(combined,{pos=p.tip,color="#ffbc72",range=2.55,intensity=p.muzzleLight})
		end
	end
	if combined or clock.hadMuzzleLight or clock.streetLightsChanged then
		lights.set(combined or clock.streetLights or {})
	end
	clock.hadMuzzleLight,clock.streetLightsChanged=combined~=nil,false
end

return ambience
