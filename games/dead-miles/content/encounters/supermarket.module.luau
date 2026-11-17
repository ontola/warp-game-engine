-- 11. Shop shelves still half full, at the back of a dark store. Loading
-- up takes long enough for the shuffling to find you.
return {
	id = "supermarket",
	title = "Shop shelves",
	says = "Tins at the back. It's dark, and something shuffles behind the shelves.",
	where = "room",
	biomes = { "city", "suburb" },
	weight = 2,
	prop = "shelves",
	nobody = true,
	verb = "look",
	choices = {
		{ label = "Fill your bags", results = { { weight = 2, gain = { food = 4 }, zombies = 3, text = "+4 food. They heard you" }, { weight = 1, gain = { food = 3, battery = 30 }, zombies = 1, text = "+3 food, batteries" } } },
		{ label = "Take a little, quietly", results = { { gain = { food = 1 }, text = "+1 food" } } },
	},
}
