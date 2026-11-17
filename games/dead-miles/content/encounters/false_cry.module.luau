-- 5. A cry for help from a back room. Sometimes it's real; sometimes
-- it's how robbers get you inside.
return {
	id = "false_cry",
	title = "A cry for help",
	says = "Help! In here! I can't get up!",
	where = "room",
	weight = 2,
	min_day = 2,
	pose = "down",
	held = "machete",
	temperament = { real = 1, bait = 2 },
	choices = {
		{
			label = "Go to them",
			results = {
				{ temperament = "real", join = true, text = "They were stuck. They join you" },
				{ temperament = "bait", lose = { food = 2, ammo = 2 }, injure = true, hostile = true, noise = 10, text = "A trap. They take food and rounds" },
			},
		},
		{ label = "Back away", results = { { text = "The cries stop when you leave." } } },
	},
}
