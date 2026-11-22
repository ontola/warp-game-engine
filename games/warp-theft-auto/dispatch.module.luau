-- One dispatch board for the whole crew. Any player can cover the next leg.
local dispatch = {}
local modes = { "drive", "fly", "walk" }
local titles = { "Harbor transfer", "Cross-town relay", "Night courier" }

function dispatch.new(city)
	local stops = { drive = {}, fly = {}, walk = {} }
	for _, place in city.landmarks do table.insert(stops[place.kind], place) end
	return { stops = stops, completed = 0, leg = 1, cash = 0, pause = 0 }
end

function dispatch.stop(job, fleet)
	if job.leg == 1 and job.recovery then
		local car = fleet.vehicles[job.recovery]
		if not car.owner then
			return { name = string.format("PM-%03d", job.recovery * 17 + 200), pos = car.pos, kind = "recover" }
		end
	end
	local options = job.stops[modes[job.leg]]
	-- The first run brings a car to the ferry, near the eastern heliport,
	-- then asks for a landing at West Field and a handoff in Lido Gardens.
	return options[(job.completed + 1) % #options + 1]
end

function dispatch.title(job)
	return titles[job.completed % #titles + 1]
end

function dispatch.tick(job, fleet, dt)
	if job.pause > 0 then job.pause = math.max(0, job.pause - dt); return end
	local goal = dispatch.stop(job, fleet)
	for id, seat in fleet.players do
		if seat.mode ~= goal.kind then continue end
		if job.leg == 1 and job.recovery and seat.vehicle ~= job.recovery then continue end
		local dist = vector.magnitude(seat.pos - goal.pos)
		local radius = if goal.kind == "walk" then 3.5 else 9
		if dist > radius or (goal.kind ~= "walk" and math.abs(seat.speed) > 3) then continue end
		if goal.kind == "fly" and (not fleet.vehicles[seat.vehicle].grounded or seat.pos.y > goal.pos.y + 1.2) then continue end
		job.lastDriver = id
		sound.play("dispatch", { db = -5 })
		if job.leg < #modes then
			job.leg += 1
		else
			job.completed += 1
			job.cash += 750
			job.pause = 4
			job.leg = 1
			-- Recover a particular vehicle next time, wherever its last driver left it.
			local start = (job.completed * 7) % #fleet.vehicles
			for offset = 1, #fleet.vehicles do
				local index = (start + offset - 1) % #fleet.vehicles + 1
				if fleet.vehicles[index].kind == "car" and not fleet.vehicles[index].owner then
					job.recovery = index
					break
				end
			end
			game.log("Relay delivered. Crew earned $750.")
		end
		break
	end
end

return dispatch
