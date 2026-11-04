-- The tomes found on shrines (relics.gd). Each changes numbers in a way
-- you can say in one sentence; none changes what a spell is. This file
-- hot-reloads: a new or changed tome shows on the next shrine that rises.
--
-- `mods` keys: an element's name (its damage times this), `dmg` (all your
-- damage times), `charge` (wind-up time times), `reach` (spray and beam
-- reach times), `enchant_time`, `sword`, `speed`, `heal` (blood you drink),
-- `afflicted` (damage on wet, burning, chilled or poisoned monsters),
-- `regen` (mana regen), `cost` (loading and casting), `souls` (mana from
-- souls), `magnet` (how far souls fly to you), `guard` (damage you take),
-- `status` (how deep your burns, poisons and chills go), `push` (knockback),
-- `bolt` (bolt damage), `swing` (staff swing recovery), `nova` (burst reach)
-- and `raise` (raising a friend) all multiply; `leech`, `thorns`, `wind`
-- (killing blows survived a level) and `chain` (lightning jumps) add;
-- `hp` and `mana` add max health and max mana; `echo` (novas go off twice),
-- `planter` (2 sentries and 5 traps) and `firewalker` (the ground never
-- hurts) are true or absent.
--
-- A shrine offers only tomes someone there can use: an element tome needs
-- one of its elements learned, Echo Stone the nova, Mason's Chisel the
-- plant, Runed Staff the enchant (relics.rs `Mod::need`). The power budget
-- per tome is in docs/balance.md, "Tomes".

local tomes = {
	{ name = "Black Grimoire", text = "Poison and Void +40%", mods = { poison = 1.4, void = 1.4 } },
	{ name = "Ember and Rime Codex", text = "Fire, Frost, Steam and Ice +40%", mods = { fire = 1.4, frost = 1.4, steam = 1.4, ice = 1.4 } },
	{ name = "Tide Scroll", text = "Water, Air, Mud and Storm +50%", mods = { water = 1.5, air = 1.5, mud = 1.5, storm = 1.5 } },
	{ name = "Storm Sigil", text = "Lightning, Earth and Magma +40%", mods = { lightning = 1.4, earth = 1.4, magma = 1.4 } },
	{ name = "Quick Hands", text = "Aimed spells wind up 40% faster", mods = { charge = 0.6 } },
	{ name = "Long Nozzle", text = "Sprays and beams reach 35% further", mods = { reach = 1.35 } },
	{ name = "Echo Stone", text = "Bursts around you go off twice", mods = { echo = true } },
	{ name = "Mason's Chisel", text = "Keep 2 sentries and 5 traps out at once", mods = { planter = true } },
	{ name = "Runed Staff", text = "Enchantments last twice as long", mods = { enchant_time = 2 } },
	{ name = "Whetstone", text = "Sword hits twice as hard", mods = { sword = 2 } },
	{ name = "Thick Hide", text = "+30 max health", mods = { hp = 30 } },
	{ name = "Glass Staff", text = "+30% to all your damage, -20 max health", mods = { dmg = 1.3, hp = -20 } },
	{ name = "Zephyr Feather", text = "You move 20% faster", mods = { speed = 1.2 } },
	{ name = "Red Chalice", text = "Blood you drink heals twice as much", mods = { heal = 2 } },
	{ name = "Pilgrim Boots", text = "Nothing on the ground can hurt or slow you", mods = { firewalker = true } },
	{ name = "Hunter's Mark", text = "+20% on wet, burning, chilled or poisoned monsters", mods = { afflicted = 1.2 } },
	{ name = "Pyre Psalter", text = "Fire and Magma +50%", mods = { fire = 1.5, magma = 1.5 } },
	{ name = "Frozen Heart", text = "Frost and Ice +50%", mods = { frost = 1.5, ice = 1.5 } },
	{ name = "Thunder Rod", text = "Lightning and Storm +50%", mods = { lightning = 1.5, storm = 1.5 } },
	{ name = "Tomb Dust", text = "Poison +60%", mods = { poison = 1.6 } },
	{ name = "Deep Well", text = "+40 max mana", mods = { mana = 40 } },
	{ name = "Moon Font", text = "Mana comes back 50% faster", mods = { regen = 1.5 } },
	{ name = "Miser's Ledger", text = "Loading and casting cost 20% less", mods = { cost = 0.8 } },
	{ name = "Soul Jar", text = "Souls give 50% more mana", mods = { souls = 1.5 } },
	{ name = "Lodestone", text = "Souls fly to you from afar and give 20% more", mods = { magnet = 3, souls = 1.2 } },
	{ name = "Iron Skin", text = "You take 20% less damage", mods = { guard = 0.8 } },
	{ name = "Plague Primer", text = "Your burns, poisons and chills go 50% deeper", mods = { status = 1.5 } },
	{ name = "Ram's Horn", text = "Spells knock monsters back twice as far, -10% hurt", mods = { push = 2, guard = 0.9 } },
	{ name = "Keen Gem", text = "Your bolts hit 80% harder", mods = { bolt = 1.8 } },
	{ name = "Quicksilver Grip", text = "Swing recovers twice as fast, hits 30% harder", mods = { swing = 0.5, sword = 1.3 } },
	{ name = "Blood Pact", text = "+40% to all your damage, but you take 20% more", mods = { dmg = 1.4, guard = 1.2 } },
	{ name = "Wayfarer's Charm", text = "+20 max health and you move 15% faster", mods = { hp = 20, speed = 1.15 } },
	{ name = "Vampire Fang", text = "Heal 3% of the damage you deal", mods = { leech = 0.03 } },
	{ name = "Bramble Mail", text = "Monsters that strike you take half of it back", mods = { thorns = 0.5 } },
	{ name = "Phoenix Feather", text = "Once a level a killing blow leaves you at 1 health", mods = { wind = 1 } },
	{ name = "Forked Rod", text = "Your chain lightning jumps to one more monster", mods = { chain = 1 } },
	{ name = "Wide Sigil", text = "Bursts around you reach 30% further", mods = { nova = 1.3 } },
	{ name = "Shepherd's Crook", text = "You raise fallen friends twice as fast", mods = { raise = 2 } },
}

return { tomes = tomes }
