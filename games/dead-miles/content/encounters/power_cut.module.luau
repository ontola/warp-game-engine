-- 17. The power goes: the street lights die one after another, and it's
-- just your torches.
return {
	id = "power_cut",
	title = "Power cut",
	where = "path",
	biomes = { "suburb", "city" },
	weight = 1,
	nobody = true,
	auto = true,
	radius = 4,
	results = { { power_cut = true, text = "The lights go out" } },
}
