-- 12. A camp someone left in a hurry, with a diary: where the dead sleep.
return {
	id = "abandoned_camp",
	title = "Abandoned camp",
	says = "A tent, a cold fire, a diary: 'They sleep in the day under the overpass. The road north is quiet.'",
	where = "path",
	biomes = { "forest", "farm", "suburb" },
	weight = 2,
	prop = "tent",
	nobody = true,
	verb = "look",
	choices = {
		{ label = "Take what's left and the advice", results = { { gain = { food = 1, batteries = 1 }, bands = { "quiet", "quiet" }, text = "+1 food. The next streets should be quiet" } } },
		{ label = "Leave it", results = { { text = "..." } } },
	},
}
