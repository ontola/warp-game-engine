-- 2. A mother and her child who want to walk with you as far as the
-- bridge. Slow company, but she knows where food is kept.
return {
	id = "mother_child",
	title = "Mother and child",
	says = "Can we walk with you to the bridge? It's not far. I know where there's food.",
	where = "path",
	weight = 2,
	look = { shirt = "#7a5a6a" },
	with = { { pool = "child", look = { shirt = "#c9b25a", pants = "#3a4a6a" }, at = vector.create(0.7, 0, 0.5) } },
	choices = {
		{
			label = "Walk them to the bridge",
			results = { { escort = { metres = 60, gain = { food = 3 }, text = "The bridge. She shows you a stash: +3 food" }, text = "They walk with you" } },
		},
		{ label = "We can't wait for you", results = { { leave = true, text = "She holds the child close" } } },
	},
}
