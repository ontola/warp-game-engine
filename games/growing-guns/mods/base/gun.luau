-- The rifle before any card: the numbers every card multiplies or adds to
-- (Godot's weapon.gd constants). Read by the sim through the mods.
--
-- When an edit applies: weapons are rebuilt from the cards at the start of
-- every round, so a saved change shows up next round (never in the middle
-- of one: a round stays one set of numbers and replays exactly). A file
-- with an error keeps the numbers from before and says why in the logs and
-- the `mods` live tool.
--
-- For quick feel changes during a match use knobs.luau instead (it applies
-- at once). The same numbers are built into the sim for builds without
-- Luau; a test keeps the two equal.

return {
	gun = {
		damage = 22.0, -- per bullet
		fire_interval = 0.22, -- seconds between triggers
		mag_size = 5, -- rounds
		reload_time = 1.2, -- seconds
		headshot = 2.0, -- damage multiplier on the head
		spread = 0.008, -- radians of random aim per shot (about 0.46 degrees)
		recoil = 0.018, -- view kick per shot
		bullet_drop = 30.0, -- m/s^2, like the player's gravity
		bullet_speed = 165.0, -- m/s
	},
}
