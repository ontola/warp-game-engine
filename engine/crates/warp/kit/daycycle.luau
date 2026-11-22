-- A clock shared by every view. Warm only the next atmospheric bake.
local daycycle = {}
local phases = {
	{ hour = 0, sky = "night" }, { hour = 5.5, sky = "sunrise" },
	{ hour = 7, sky = "morning" }, { hour = 11, sky = "noon" },
	{ hour = 14, sky = "afternoon" }, { hour = 18, sky = "sunset" },
	{ hour = 20, sky = "night" },
}

function daycycle.new(startHour, secondsPerDay, weather)
	assert(secondsPerDay > 0, "secondsPerDay must be positive")
	return { hour = startHour % 24, secondsPerDay = secondsPerDay, weather = weather or "clear" }
end

function daycycle.tick(clock, dt)
	clock.hour = (clock.hour + dt * 24 / clock.secondsPerDay) % 24
	local index = #phases
	while clock.hour < phases[index].hour do index -= 1 end
	local phase = phases[index].sky
	if clock.phase ~= phase then
		sky.set(phase .. ":" .. clock.weather)
		-- Midnight spans two entries. Prepare sunrise even if time starts after dusk.
		local nextPhase = if phase == "night" then "sunrise" else phases[index + 1].sky
		sky.warm(nextPhase .. ":" .. clock.weather)
		clock.phase = phase
	end
	clock.daylight = clock.hour >= 6 and clock.hour < 19
	return clock.hour
end

return daycycle
