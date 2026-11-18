-- 9. A police car left at the kerb. The boot holds rounds, and an alarm.
return {
	id = "police_car",
	title = "Police car",
	says = "The boot is shut and locked. Something heavy inside. The alarm light still blinks.",
	where = "path",
	biomes = { "suburb", "city" },
	weight = 2,
	prop = "police_car",
	nobody = true,
	verb = "look",
	choices = {
		{ label = "Force the boot (alarm)", results = { { weight = 2, gain = { ammo = 5 }, open = true, noise = 25, lure = 10, lure_sound = "alarm", text = "+5 ammo. The alarm screams" }, { weight = 1, gain = { ammo = 2, meds = 1 }, open = true, noise = 25, lure = 10, lure_sound = "alarm", text = "+2 ammo, +1 first aid. The alarm!" } } },
		{ label = "Leave it", results = { { text = "..." } } },
	},
}
