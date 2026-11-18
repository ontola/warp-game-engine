-- 4. Someone on a first-floor balcony lowers a basket on a rope: trade
-- without coming down.
return {
	id = "balcony",
	title = "Up on the balcony",
	says = "Psst. Up here. Put something in the basket and I'll send something down.",
	where = "path",
	biomes = { "suburb", "city" },
	weight = 2,
	prop = "balcony",
	stand = vector.create(0, 3.1, -0.45),
	verb = "call up",
	pose = "wave",
	choices = {
		{ label = "2 food for batteries", cost = { food = 2 }, results = { { gain = { batteries = 2 }, text = "batteries come down" } }, stay = true },
		{ label = "2 rounds for a first aid kit", cost = { ammo = 2 }, results = { { gain = { meds = 1 }, text = "+1 first aid" } }, stay = true },
		{ label = "Nothing today", results = { { text = "Suit yourself." } } },
	},
}
