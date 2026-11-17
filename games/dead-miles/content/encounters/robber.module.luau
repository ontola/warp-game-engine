-- A stranger who asks for help. Some are bait.
return {
	id = "robber",
	title = "Stranger",
	says = "My leg's hurt. Do you have a first aid kit?",
	where = "path",
	min_day = 2,
	weight = 2,
	held = "machete",
	temperament = { hurt = 2, robber = 2 },
	choices = {
		{
			label = "Give a first aid kit",
			cost = { meds = 1 },
			results = {
				{ temperament = "hurt", gain = { food = 2 }, text = "Thank you. Take this." },
				{ temperament = "robber", lose = { food = 2 }, injure = true, hostile = true, text = "A knife. They run." },
			},
		},
		{
			label = "Search them",
			results = {
				{ temperament = "hurt", text = "Nothing. They limp off." },
				{ temperament = "robber", injure = true, hostile = true, noise = 10, text = "They fight back" },
			},
		},
		{ label = "Walk on", results = { { text = "..." } } },
	},
}
