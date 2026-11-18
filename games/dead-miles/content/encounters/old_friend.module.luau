-- 7. Someone who knew one of you before. A few words, and what they can spare.
return {
	id = "old_friend",
	title = "A face you know",
	says = "Is that you? From the depot? I thought everyone from there was gone.",
	where = "path",
	weight = 1,
	look = { shirt = "#4a6a5a" },
	choices = {
		{ label = "Talk a while", results = { { weight = 2, gain = { food = 2, batteries = 1 }, text = "Old times. +2 food, batteries" }, { weight = 1, gain = { ammo = 3 }, text = "They press rounds into your hand: +3" } } },
		{ label = "Ask them to come", results = { { weight = 1, join = true, text = "They join you" }, { weight = 1, gain = { food = 1 }, text = "They're waiting for someone. +1 food" } } },
	},
}
