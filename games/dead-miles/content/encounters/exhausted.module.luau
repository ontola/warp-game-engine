-- 23. A bench by the road and legs that won't go on. Rest costs time
-- (the dead catch up); walking on costs breath.
return {
	id = "exhausted",
	title = "A bench",
	says = "Your legs are lead. A bench. Five minutes?",
	where = "path",
	weight = 1,
	prop = "bench",
	nobody = true,
	auto = true,
	radius = 3,
	choices = {
		{ label = "Rest a while", results = { { stamina = 1, zombies = 3, text = "Rested. They caught up" } } },
		{ label = "Keep going", results = { { stamina = 0.3, text = "On, out of breath" } } },
	},
}
