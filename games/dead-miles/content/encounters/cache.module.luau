-- A barricaded supply room. Breaking in is loud.
return {
	id = "cache",
	title = "Barricaded door",
	says = "Boards and a padlock. Someone stocked this room.",
	where = "room",
	weight = 2,
	look = { shirt = "#3a3a3a", skin = "#3a3a3a", pants = "#3a3a3a" },
	choices = {
		{
			label = "Break it open (loud)",
			results = {
				{ weight = 2, gain = { ammo = 4, food = 2 }, noise = 30, text = "+4 ammo, +2 food" },
				{ weight = 1, gain = { meds = 1 }, weapon = "axe", noise = 30, text = "first aid and an axe" },
				{ weight = 1, noise = 30, text = "Empty. And loud." },
			},
		},
		{ label = "Leave it", results = { { text = "..." } } },
	},
}
