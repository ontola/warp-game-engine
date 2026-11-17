-- Someone else's camp fire at night.
return {
	id = "fire",
	title = "Old man at the fire",
	says = "Sit. The dead don't like the light. Mostly.",
	where = "camp",
	weight = 2,
	look = { shirt = "#6a5040", skin = "#d9a37e" },
	choices = {
		{ label = "Share food (1)", cost = { food = 1 }, results = { { heal = true, text = "Everyone patched up" } } },
		{ label = "Ask him to come", results = { { weight = 1, join = true, text = "He joins you" }, { weight = 2, text = "Too old for walking." } } },
		{ label = "Leave him be", results = { { text = "Good luck." } } },
	},
}
