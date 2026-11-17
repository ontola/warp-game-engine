-- 3. A stranger with a bandaged arm who swears it's glass. Sometimes it
-- is. Sometimes, half a mile on, it isn't.
return {
	id = "bitten",
	title = "Stranger with a bandage",
	says = "It's glass, from a window. Just a scratch. I can fight, take me along.",
	where = "path",
	weight = 2,
	min_day = 2,
	held = "bat",
	look = { shirt = "#8a8478" },
	temperament = { scratch = 1, bitten = 2 },
	choices = {
		{
			label = "Come along",
			results = {
				{ temperament = "scratch", join = true, text = "They join you" },
				{ temperament = "bitten", join = true, bitten = 75, text = "They join you" },
			},
		},
		{
			label = "Show me that arm",
			results = {
				{ temperament = "scratch", join = true, text = "Glass, really. They join you" },
				{ temperament = "bitten", leave = true, text = "Teeth marks. They run" },
			},
		},
		{ label = "Walk on", results = { { text = "..." } } },
	},
}
