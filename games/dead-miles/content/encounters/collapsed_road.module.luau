-- 21. The road fell in ahead. Through the dark building beside it, or
-- over the roofs.
return {
	id = "collapsed_road",
	title = "The road fell in",
	says = "A hole where the road was, too wide to jump. Through the building, or up over the roofs.",
	where = "path",
	biomes = { "city", "suburb" },
	weight = 1,
	prop = "collapse",
	nobody = true,
	verb = "look",
	choices = {
		{ label = "Through the building (dark)", results = { { weight = 2, zombies = 3, skip = 24, text = "Through the dark. They were in there" }, { weight = 1, gain = { food = 2 }, skip = 24, text = "Through. +2 food on the way" } } },
		{ label = "Over the roofs (tiring)", results = { { weight = 3, stamina = 0, skip = 24, text = "Over the roofs, out of breath" }, { weight = 1, stamina = 0, injure = true, skip = 24, text = "A bad landing" } } },
		{ label = "Walk around it", results = { { text = "Around the long way" } } },
	},
}
