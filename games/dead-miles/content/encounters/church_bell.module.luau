-- 19. A bell starts tolling by itself (or someone's ringing it). The dead
-- head for it: a chance to slip past.
return {
	id = "church_bell",
	title = "A bell",
	where = "path",
	weight = 1,
	prop = "bell",
	nobody = true,
	auto = true,
	radius = 14,
	results = { { lure = 40, lure_sound = "bell", text = "A bell tolls. They turn towards it" } },
}
