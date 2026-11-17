-- 6. Two survivors fighting over a bag. Take a side or walk away.
return {
	id = "quarrel",
	title = "Two of them, one bag",
	says = "It's mine! Tell her it's mine! I found it first!",
	where = "path",
	weight = 2,
	held = "pipe",
	look = { shirt = "#5a4a3a" },
	with = { { at = vector.create(1.3, 0, 0.3), held = "bat", look = { shirt = "#3a4a5a" } } },
	choices = {
		{ label = "Side with him", results = { { join = true, text = "He comes with you. She curses you." } } },
		{ label = "Side with her", results = { { gain = { food = 2, meds = 1 }, leave = true, text = "She shares the bag: +2 food, +1 first aid" } } },
		{ label = "Take the bag (loud)", results = { { weight = 1, gain = { food = 3, ammo = 3 }, hostile = true, noise = 14, text = "+3 food, +3 ammo. They scream after you" }, { weight = 1, injure = true, hostile = true, noise = 14, text = "They both turn on you" } } },
		{ label = "Not our fight", results = { { text = "..." } } },
	},
}
