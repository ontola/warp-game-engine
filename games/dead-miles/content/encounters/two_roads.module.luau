-- 24. A fork: the short way past the hospital, or the long way through
-- the park. What comes next depends on it.
return {
	id = "two_roads",
	title = "Two roads",
	says = "A signpost: HOSPITAL, short. PARK, long. Someone wrote 'NO' under hospital.",
	where = "path",
	weight = 1,
	prop = "sign",
	nobody = true,
	auto = true,
	radius = 3,
	choices = {
		{ label = "The hospital: short, busy", results = { { skip = 30, bands = { "horde", "busy" }, gain = { meds = 2 }, text = "Past the hospital: +2 first aid, and them" } } },
		{ label = "The park: long, quiet", results = { { lose = { food = 1 }, bands = { "quiet", "quiet", "quiet" }, text = "The long way. -1 food, and quiet" } } },
	},
}
