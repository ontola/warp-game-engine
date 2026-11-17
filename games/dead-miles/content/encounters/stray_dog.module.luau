-- 20. A stray that follows you. Fed, it stays and warns you of the dead;
-- it barks, though.
return {
	id = "stray_dog",
	title = "A stray dog",
	says = "A thin dog, wagging, keeping its distance. It looks at your bag.",
	where = "path",
	weight = 2,
	nobody = true,
	dog = true,
	verb = "look",
	choices = {
		{ label = "Feed it (1 food)", cost = { food = 1 }, results = { { dog = true, text = "It stays at your heel" } } },
		{ label = "Shoo it away", results = { { dog_off = true, text = "It runs off" } } },
	},
}
