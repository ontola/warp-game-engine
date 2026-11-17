-- 14. A radio left playing on a crate. It draws the dead until someone
-- turns it off; taken along, it tells you what's ahead.
return {
	id = "radio",
	title = "A radio",
	says = "Static, then a voice: '...north road clear... avoid the...' It's loud.",
	where = "path",
	weight = 2,
	prop = "radio",
	nobody = true,
	verb = "look",
	lure = { every = 1.6, radius = 22, sound = "radio" },
	choices = {
		{ label = "Switch it off", results = { { quiet = true, text = "Quiet again" } } },
		{ label = "Take it and listen", results = { { quiet = true, bands = { "quiet", "busy", "quiet" }, text = "The broadcast tells you where they are" } } },
	},
}
