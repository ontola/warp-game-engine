-- The base cards (Godot card_library.gd). This file hot-reloads: edit and
-- save while the game runs. API: ../API.md (generated from the Rust side).
--
-- A card is data: `mul` multiplies stats, then `add` adds to them (in f32,
-- inside the sim). `pick` runs once when the card is taken, for anything a
-- table of numbers cannot say. Ids are what `give P ID` uses.

local cards = {
	{ id = "damage", name = "DAMAGE", text = "x1.5 damage", color = { 1.0, 0.25, 0.25 }, firepower = true,
		mul = { damage = 1.5 } },
	{ id = "healthy", name = "HEALTHY", text = "+35 max health", color = { 0.45, 1.0, 0.55 }, firepower = false,
		add = { hp_bonus = 35 } },
	{ id = "steadyhands", name = "STEADY HANDS", text = "Tighter spread", color = { 0.55, 0.8, 1.0 }, firepower = true,
		mul = { spread = 0.65 } },
	{ id = "precision", name = "PRECISION", text = "Tighter spread, half recoil", color = { 0.55, 0.8, 1.0 }, firepower = true,
		mul = { spread = 0.4, recoil = 0.5 } },
	{ id = "sneakers", name = "SNEAKERS", text = "+15% move speed", color = { 0.9, 0.9, 0.4 }, firepower = false,
		mul = { speed = 1.15 } },
	{ id = "rapidfire", name = "RAPID FIRE", text = "x1.6 fire rate", color = { 1.0, 0.6, 0.2 }, firepower = true,
		mul = { fire_rate = 1.6 } },
	{ id = "fastrounds", name = "FAST ROUNDS", text = "x1.5 bullet speed", color = { 0.65, 0.92, 1.0 }, firepower = true,
		mul = { bullet_speed = 1.5 } },
	{ id = "bigmag", name = "BIG MAG", text = "+20 rounds", color = { 0.85, 0.7, 1.0 }, firepower = true,
		add = { mag_bonus = 20 } },
	{ id = "quickreload", name = "QUICK RELOAD", text = "x2 reload speed", color = { 0.85, 0.7, 1.0 }, firepower = true,
		mul = { reload = 2.0 } },
	{ id = "explosive", name = "EXPLOSIVE ROUNDS", text = "Bullets explode on impact", color = { 1.0, 0.4, 0.1 }, firepower = true,
		add = { explosive_damage = 25 },
		pick = function(g: Game, p: number)
			-- The first stack gives a sizeable blast, later ones grow it.
			local r = g:stat(p, "explosive_radius")
			g:set_stat(p, "explosive_radius", r + (if r <= 0 then 4 else 2.5))
		end },
}

return { cards = cards }
