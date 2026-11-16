-- A trader holed up in a back room. Swaps what you have for what you need.
-- Encounter files return one table; main.luau finds every file in this
-- folder. See README.md ("Writing an encounter") for every field.
return {
	id = "trader",
	title = "Trader",
	says = "Easy. I trade, I don't fight. What have you got?",
	where = "room",
	biomes = { "suburb", "city" },
	weight = 3,
	look = { shirt = "#5a6a4a" },
	choices = {
		{ label = "2 food for 3 rounds", cost = { food = 2 }, results = { { gain = { ammo = 3 }, text = "+3 ammo" } } },
		{ label = "3 rounds for a first aid kit", cost = { ammo = 3 }, results = { { gain = { meds = 1 }, text = "+1 first aid" } } },
		{ label = "1 food for batteries", cost = { food = 1 }, results = { { gain = { battery = 40 }, text = "batteries" } } },
		{ label = "Leave", results = { { text = "Stay safe." } }, stay = true },
	},
}
