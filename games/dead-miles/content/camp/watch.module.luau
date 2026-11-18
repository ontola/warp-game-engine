return {
	id = "watch",
	weight = 2,
	people = 2,
	text = "Someone has to keep watch.",
	choices = {
		{ label = "{a} takes it", results = {
			{ weight = 2, text = "A quiet night. {a} is dead on their feet in the morning.", injure = "a" },
			{ weight = 1, text = "{a} sees them coming in time. You slip out the back before they reach the fire.", dawn = 3 },
		} },
		{ label = "Everyone sleeps", results = {
			{ weight = 2, text = "Everyone sleeps. Nothing comes. This time." },
			{ weight = 1, text = "Something got into the packs in the night.", lose = { food = 1 } },
		} },
	},
}
