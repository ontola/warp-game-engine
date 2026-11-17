-- 15. A locked yard gate: a short cut past the next streets, if you can
-- get through it.
return {
	id = "locked_gate",
	title = "Locked gate",
	says = "A yard gate, padlocked. Through there you'd skip a few streets.",
	where = "path",
	biomes = { "suburb", "city", "farm" },
	weight = 2,
	prop = "gate",
	nobody = true,
	verb = "look",
	choices = {
		{ label = "Climb over (tiring)", results = { { stamina = 0.1, skip = 30, text = "Over the gate and through the yards" } } },
		{ label = "Break the lock (loud)", results = { { noise = 18, calm = 25, text = "It gives. The noise carries" } } },
		{ label = "Look for the key", results = { { weight = 1, calm = 40, gain = { ammo = 1 }, text = "Under a brick. And a round" }, { weight = 1, zombies = 2, text = "No key. Something found you" } } },
	},
}
