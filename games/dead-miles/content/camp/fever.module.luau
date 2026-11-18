return {
	id = "fever",
	weight = 2,
	text = "{a} is burning up. Could be a cut gone bad. Could be the other thing.",
	choices = {
		{ label = "Use the meds", cost = { meds = 1 }, results = {
			{ text = "By morning the fever breaks.", heal = "a" },
		} },
		{ label = "Wait it out", results = {
			{ weight = 2, text = "It breaks on its own, but {a} is weak in the morning.", injure = "a" },
			{ weight = 1, text = "{a} sweats through the night and wakes up fine. Lucky." },
		} },
	},
}
