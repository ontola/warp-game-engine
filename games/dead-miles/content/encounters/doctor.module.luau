-- 8. A doctor kneeling by a patient in a back room. Treatment has a price.
return {
	id = "doctor",
	title = "Doctor",
	says = "Don't come close, I'm working. If you're hurt, I can help. Not for free.",
	where = "room",
	biomes = { "suburb", "city" },
	weight = 2,
	pose = "crouch",
	look = { shirt = "#d8d8d0", pants = "#5a6a7a" },
	with = { { at = vector.create(0.9, 0, 0), pose = "down", look = { shirt = "#6a4a3a" } } },
	choices = {
		{ label = "Patch everyone up (2 food)", cost = { food = 2 }, results = { { heal = true, text = "Everyone patched up" } } },
		{ label = "Buy a first aid kit (3 rounds)", cost = { ammo = 3 }, results = { { gain = { meds = 1 }, text = "+1 first aid" } }, stay = true },
		{ label = "Leave them be", results = { { text = "Close the door behind you." } } },
	},
}
