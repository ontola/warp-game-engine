-- 22. A horde asleep where the street opens up. Walk past, don't run.
return {
	id = "sleeping_horde",
	title = "A sleeping horde",
	where = "path",
	min_day = 2,
	weight = 1,
	nobody = true,
	sleepers = { 12, 18 },
	auto = true,
	radius = 16,
	results = { { text = "Dozens, asleep. Walk. Don't run." } },
}
