-- 16. A car burning in the street: light to see by, and light the dead
-- walk towards.
return {
	id = "burning_car",
	title = "A burning car",
	where = "path",
	weight = 2,
	prop = "burning_car",
	nobody = true,
	light = { color = "#ff8a3a", intensity = 900, range = 18 },
	lure = { every = 4, radius = 20 },
	auto = true,
	radius = 12,
	results = { { text = "A car burns. The light draws them" } },
}
