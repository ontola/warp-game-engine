-- The tomes found on shrines (relics.gd). Each changes numbers in a way
-- you can say in one sentence; none changes what a spell is. This file
-- hot-reloads: a new or changed tome shows on the next shrine that rises.
--
-- `mods` keys: an element's name (its damage times this), `dmg` (all your
-- damage times), `charge` (wind-up time times), `reach` (spray and beam
-- reach times), `enchant_time`, `sword`, `speed`, `heal` (blood you drink),
-- `afflicted` (damage on wet, burning, chilled or poisoned monsters) all
-- multiply; `hp` adds max health; `echo` (novas go off twice), `planter`
-- (2 sentries and 5 traps) and `firewalker` (the ground never hurts) are
-- true or absent.

local tomes = {
	{ name = "Black Grimoire", text = "Poison and Void +30%", mods = { poison = 1.3, void = 1.3 } },
	{ name = "Ember and Rime Codex", text = "Fire, Frost, Steam and Ice +35%", mods = { fire = 1.35, frost = 1.35, steam = 1.35, ice = 1.35 } },
	{ name = "Tide Scroll", text = "Water, Air, Mud and Storm +50%", mods = { water = 1.5, air = 1.5, mud = 1.5, storm = 1.5 } },
	{ name = "Storm Sigil", text = "Lightning, Earth and Magma +35%", mods = { lightning = 1.35, earth = 1.35, magma = 1.35 } },
	{ name = "Quick Hands", text = "Aimed spells wind up 40% faster", mods = { charge = 0.6 } },
	{ name = "Long Nozzle", text = "Sprays and beams reach 35% further", mods = { reach = 1.35 } },
	{ name = "Echo Stone", text = "Bursts around you go off twice", mods = { echo = true } },
	{ name = "Mason's Chisel", text = "Keep 2 sentries and 5 traps out at once", mods = { planter = true } },
	{ name = "Runed Staff", text = "Enchantments last twice as long", mods = { enchant_time = 2 } },
	{ name = "Whetstone", text = "Sword hits 60% harder", mods = { sword = 1.6 } },
	{ name = "Thick Hide", text = "+30 max health", mods = { hp = 30 } },
	{ name = "Glass Staff", text = "+15% to all your damage, -30 max health", mods = { dmg = 1.15, hp = -30 } },
	{ name = "Zephyr Feather", text = "You move 20% faster", mods = { speed = 1.2 } },
	{ name = "Red Chalice", text = "Blood you drink heals 60% more", mods = { heal = 1.6 } },
	{ name = "Pilgrim Boots", text = "Nothing on the ground can hurt or slow you", mods = { firewalker = true } },
	{ name = "Hunter's Mark", text = "+25% on wet, burning, chilled or poisoned monsters", mods = { afflicted = 1.25 } },
}

return { tomes = tomes }
