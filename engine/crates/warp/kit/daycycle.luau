-- The sun, sky and clouds share this clock across all views.
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
	clock.phase = phase
	sky.set_time(clock.hour, clock.weather)
	local dusk = math.clamp((math.abs(clock.hour - 12) - 5) / 1.25, 0, 1)
	clock.lampLevel = dusk * dusk * (3 - 2 * dusk)
	clock.daylight = clock.hour >= 6 and clock.hour < 19
	return clock.hour
end

return daycycle
