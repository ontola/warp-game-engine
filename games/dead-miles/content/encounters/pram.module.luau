-- 13. A pram in the middle of the street. A parcel inside, or bait.
return {
	id = "pram",
	title = "A pram",
	says = "A pram, alone. A wrapped bundle inside. Too neat.",
	where = "path",
	weight = 2,
	prop = "pram",
	in_road = true,
	nobody = true,
	verb = "look",
	temperament = { gift = 2, bait = 1 },
	choices = {
		{
			label = "Look inside",
			results = {
				{ temperament = "gift", gain = { food = 2, meds = 1 }, text = "Someone left it for others: +2 food, +1 first aid" },
				{ temperament = "bait", zombies = 3, noise = 12, text = "A tripwire and cans. They're coming" },
			},
		},
		{ label = "Leave it", results = { { text = "..." } } },
	},
}
