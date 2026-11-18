return {
	id = "hungry_boy",
	weight = 2,
	text = "A boy steps into the firelight with his hands up. He hasn't eaten in days.",
	choices = {
		{ label = "Share food", cost = { food = 1 }, results = {
			{ weight = 2, text = "He eats, then points east: a pharmacy nobody has found yet. He's gone by morning. There was something there.", gain = { meds = 1 } },
			{ weight = 1, text = "He eats like a dog and slips away before morning." },
		} },
		{ label = "Send him away", results = {
			{ text = "He goes. {b} doesn't say a word until morning." },
		} },
	},
}
