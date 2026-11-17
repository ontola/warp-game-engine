-- 1. Someone lying hurt in the road, asking for a first aid kit. Most are
-- grateful; helping costs a kit you may need yourself.
return {
	id = "hurt_stranger",
	title = "Hurt in the road",
	says = "Please... my leg. Have you got a kit? I'll pay you back.",
	where = "path",
	weight = 2,
	pose = "down",
	temperament = { grateful = 3, scared = 1 },
	choices = {
		{
			label = "Patch them up (1 first aid)",
			cost = { meds = 1 },
			results = {
				{ temperament = "grateful", weight = 1, join = true, text = "They get up and come with you" },
				{ temperament = "grateful", weight = 1, gain = { food = 2, ammo = 2 }, text = "Thank you. Take this. +2 food, +2 ammo" },
				{ temperament = "scared", leave = true, text = "They limp off without a word" },
			},
		},
		{ label = "Walk on", results = { { text = "..." } } },
	},
}
