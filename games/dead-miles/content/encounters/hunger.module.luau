-- 25. At the fire there isn't enough food for everyone. Who eats?
return {
	id = "hunger",
	title = "Not enough food",
	says = "Not enough to go round tonight. Who eats?",
	where = "camp",
	weight = 3,
	nobody = true,
	auto = true,
	radius = 4,
	when = function(g) return g.food < g.people end,
	choices = {
		{ label = "Everyone a little", results = { { stamina = 0.4, text = "Everyone goes to sleep hungry" } } },
		{ label = "Those who fight eat", cost = { food = 1 }, results = { { heal = true, text = "The fighters eat. They're patched up" } } },
		{ label = "Go and find more (risky)", results = { { weight = 1, gain = { food = 2 }, text = "+2 food from the dark" }, { weight = 1, injure = true, zombies = 2, text = "Something found you out there" } } },
	},
}
