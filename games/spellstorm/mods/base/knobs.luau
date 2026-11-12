-- Spellstorm's live knobs: the balance and look numbers anyone playing can
-- turn while the game runs (`knobs`, `set_knob`, `trial`, `keep`, `revert`
-- with `spellstorm play --live`). A saved edit applies on the next frame;
-- `set_knob` rewrites the value here. This file is the only copy: the sim
-- reads it built in when the game runs outside the repository. Everything
-- else is a constant in the code, or content in monsters.luau, biomes.luau,
-- spells.luau and tomes.luau.

-- @knobs
local KNOBS = {
	wizard_speed = 5.2, -- Wizard walking speed, m/s.
	roll_speed = 12.5, -- Dodge roll speed at its push off, m/s.
	roll_cooldown = 0.95, -- Seconds between two rolls.
	swing_recover = 0.7, -- Seconds after a sword swing before the next one.
	swing_reach = 1.7, -- The sword's reach, m.
	mana_regen = 3, -- Mana back per second.
	spell_damage = 1, -- Scales the damage of every wizard spell, bolt and sword hit.
	flash_brightness = 0.5, -- Scales how bright the glow of a hit, shot or burst is (looks only).
	flash_light = 0.5, -- Scales how much light a hit, shot or burst throws around (looks only).
	number_size = 0.5, -- Scales the damage numbers over whoever is hit (looks only).
	shadow_softness = 0.05, -- How big the staff's light is, in metres, for its shadows: 0 hard, larger softer, the more so the further from what casts them (looks only).
	monster_hp = 1, -- Scales monster health (on top of the difficulty setting).
	monster_damage = 1, -- Scales the damage monsters do (on top of the difficulty setting).
	monster_count = 1, -- Scales how many monsters a wave brings (on top of the difficulty setting).
	monster_speed = 1, -- Scales monster walking speed.
	wave_gap = 30, -- Metres of path between two waves.
	first_wave = 8, -- Metres before the first wave of a level.
	first_calm = 22, -- Metres before the first wave of a run's first level (a calm start).
	revive_need = 40, -- Revive progress needed to raise a fallen wizard.
	revive_rate = 14, -- Revive progress per second for each friend at the grave.
	tome_time = 0.9, -- Seconds standing in an altar's ring to take its tome.
	wards = 1, -- 1: walls of mist hold the way on until the fight there is done; 0: no walls.
	ward_needs_tomes = 1, -- 1: a shrine's wall opens once every wizard took a tome; 0: as soon as the shrine rises.
	practice = 0, -- 1: players who know nothing start on the practice strip; 0: never (off until Joep wants it back).
}
-- @end

return { knobs = KNOBS }
