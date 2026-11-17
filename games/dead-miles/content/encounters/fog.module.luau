-- 18. Fog rolls in: for a while you see almost nothing and hear everything.
return {
	id = "fog",
	title = "Fog",
	where = "path",
	biomes = { "forest", "farm", "suburb" },
	weight = 1,
	nobody = true,
	auto = true,
	radius = 4,
	results = { { fog = 50, text = "Fog rolls in" } },
}
