-- The rifle before any card, and its RMB specials: the numbers every card
-- multiplies or adds to (Godot's weapon.gd and special constants). Read by
-- the sim through the mods.
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

		-- The RMB specials. Each card's special_cooldown_mult scales the reloads.
		specials = {
			grenade_reload = 3.0, -- seconds; zoom too
			cluster_reload = 4.0,
			air_strike_reload = 9.5,
			ion_cannon_reload = 14.0,
			sword_reload = 0.5,
			teleport_reload = 2.0,
			echo_delay = 0.16, -- seconds between a special and each echo
			teleport_range = 45.0, -- metres
			sword_range = 5.0, -- metres, times the melee_scale cards give
			sword_radius = 1.2,
			air_strike_radius = 30.0, -- metres
			air_strike_damage = 165.0, -- at the centre
			ion_radius = 38.0, -- metres, through any cover
			ion_damage = 175.0, -- at the centre, 58% at the edge
		},
	},
}
