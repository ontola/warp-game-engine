-- What happens at the camp while the group sleeps. {a} and {b} are two of
-- them (as in content/talk); {fallen} who died on the way today.
-- needs: stocks there must be; people: at least this many in the group.
-- A result: gain/lose stocks, injure/heal "a", "b" or "all", dawn = how
-- many of the dead are on your heels when you leave, calm = seconds of
-- quiet after, text = what happened.
return {
	id = "robbed",
	weight = 3,
	needs = { food = 1 },
	text = "In the small hours {a} wakes to a torch in the face. Two strangers are going through the packs.",
	choices = {
		{ label = "Fight them off", results = {
			{ weight = 2, text = "They run. {b} took a knife across the arm.", injure = "b" },
			{ weight = 1, text = "They run, and drop a bag on the way out.", gain = { food = 1 } },
		} },
		{ label = "Let them take it", results = {
			{ text = "They take food and a battery and go. Nobody sleeps after that.", lose = { food = 1, batteries = 1 } },
		} },
	},
}
