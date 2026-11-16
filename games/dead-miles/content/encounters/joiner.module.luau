-- Someone alone on the path who wants to come along. Most mean it.
return {
	id = "joiner",
	title = "Survivor",
	says = "Please, take me with you. I can carry my weight.",
	where = "path",
	weight = 2,
	held = "pipe",
	temperament = { honest = 4, thief = 1 },
	choices = {
		{
			label = "Come along (1 food)",
			cost = { food = 1 },
			results = {
				{ temperament = "honest", join = true, text = "They join you" },
				{ temperament = "thief", lose = { ammo = 2 }, hostile = true, text = "They grab ammo and run" },
			},
		},
		{ label = "Walk on", results = { { text = "..." } } },
	},
}
