-- 10. An ambulance on its side, doors open. Medicine inside, and someone
-- still strapped to the stretcher.
return {
	id = "ambulance",
	title = "Ambulance",
	says = "On its side, back doors open. Something moves on the stretcher.",
	where = "path",
	weight = 2,
	prop = "ambulance",
	nobody = true,
	verb = "look",
	choices = {
		{ label = "Climb in for the medicine", results = { { gain = { meds = 2 }, zombies = 1, text = "+2 first aid. It's loose!" } } },
		{ label = "Leave it", results = { { text = "..." } } },
	},
}
