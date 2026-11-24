-- Spellstorm's monsters (enemy.gd): one row per kind. What a kind does
-- (melee, ranged, slam, burrow, dash, lob; how it moves and aims) is Rust
-- in sim/src/enemy.rs; how tough, fast and strong it is lives here. This
-- file hot-reloads: a changed number applies to every monster at once.
-- Adding, removing or reordering kinds takes effect at the next start.
--
-- Each row starts from DEFAULT. Fields:
--   key, name     id (biomes and spawns name it) and the name on screen
--   model         whose body it wears (nil: its own), scaled by size
--   hp, speed, dmg, reach (m), keep (m, ranged kinds keep this far off)
--   el            its element; resist = { element = fraction } (negative: weak)
--   attack        melee, ranged, slam, burrow, dash or lob
--   shots         bullet patterns taken in turn: fan3, fan5, ring10, burst3,
--                 wave7, slow1, snipe; ring = bullets in a slam's ring
--   wind          wind-up seconds (nil: the attack's own)
--   mass, size, pack (how many come together), solo (never two in a wave)
--   undead, always_wet, practice (only on the practice strip), shield, boss
--   heals_from    an element that heals it; burst = element it dies in
--   raise         a kind it raises from the ground; summon = kinds a boss calls
--   spawn         ahead, far, under, flank or water
--   dash          { speed m/s, seconds }; trail = element it leaves behind
--   nova          a boss's blast element
--   tempo         a boss's pace: above 1 its patterns come further apart (nil: 1)
-- A module that returns its own `monsters` list replaces this one.

local DEFAULT = {
	model = nil,
	hp = 40,
	speed = 3,
	dmg = 9,
	reach = 1.3,
	keep = nil,
	el = "earth",
	size = 1,
	attack = "melee",
	shots = { "fan3" },
	wind = nil,
	ring = 0,
	mass = 1,
	undead = false,
	resist = {},
	heals_from = nil,
	burst = nil,
	always_wet = false,
	practice = false,
	raise = nil,
	solo = false,
	spawn = "ahead",
	dash = { 0, 0 },
	trail = nil,
	pack = 1,
	shield = false,
	boss = false,
	summon = {},
	nova = "void",
}


local kinds = {
	{ key = "ghoul", name = "Ghoul", speed = 4.3, size = 0.9 },
	{ key = "skeleton", name = "Skeleton", hp = 50, speed = 3.3, dmg = 10, reach = 1.4, undead = true, resist = { frost = 0.3, earth = -0.3, void = 0.5 } },
	{ key = "watcher", name = "Watcher", hp = 34, reach = 10, keep = 8, el = "void", attack = "ranged", resist = { void = 0.6, lightning = -0.4 } },
	{ key = "cultist", name = "Cultist", hp = 42, dmg = 11, reach = 9, keep = 7, el = "fire", attack = "ranged", shots = { "fan5", "ring10" }, resist = { fire = 0.6 } },
	{ key = "bile", name = "Bile Horror", hp = 95, speed = 2.1, dmg = 12, reach = 1.5, el = "water", size = 1.25, mass = 2, resist = { water = 1, earth = 0.2 }, burst = "mud" },
	{ key = "imp", name = "Imp", hp = 30, speed = 5.2, dmg = 8, reach = 7, keep = 5, el = "fire", size = 0.7, attack = "ranged", shots = { "burst3" }, resist = { frost = -0.5, water = -0.5 }, heals_from = "fire" },
	{ key = "brute", name = "Brute", hp = 240, speed = 2.7, dmg = 24, reach = 2.2, size = 1.7, attack = "slam", ring = 10, mass = 3.5, resist = { earth = 0.3 } },
	{ key = "frost_brute", name = "Frost Brute", hp = 260, speed = 2.5, dmg = 22, reach = 2.2, el = "frost", size = 1.7, attack = "slam", ring = 10, mass = 3.5, resist = { frost = 0.9, fire = -0.4 } },
	{ key = "wraith", name = "Wraith", hp = 45, speed = 3.8, dmg = 10, reach = 8, keep = 6, el = "frost", attack = "ranged", shots = { "wave7" }, undead = true, resist = { frost = 1, earth = 0.5 } },
	{ key = "drowned", name = "Drowned", hp = 55, speed = 3.1, dmg = 11, reach = 1.4, el = "water", resist = { water = 1, fire = 0.4 }, always_wet = true },
	{ key = "demon", name = "Demon", hp = 130, speed = 3.5, dmg = 16, reach = 1.7, el = "fire", size = 1.35, mass = 2, resist = { fire = 1, frost = -0.3 } },
	{ key = "bone_archer", name = "Bone Archer", model = "skeleton", hp = 32, speed = 3.2, dmg = 14, reach = 16, keep = 11, attack = "ranged", shots = { "snipe" }, wind = 1.1, undead = true, resist = { frost = 0.3, earth = -0.3, void = 0.5 }, spawn = "far" },
	{ key = "practice_archer", name = "Bone Archer", model = "skeleton", hp = 24, speed = 0, dmg = 5, reach = 18, keep = 11, attack = "ranged", shots = { "snipe" }, wind = 1.5, undead = true, practice = true },
	{ key = "practice_caster", name = "Cultist", model = "cultist", hp = 30, speed = 0, dmg = 4, reach = 14, keep = 9, el = "fire", attack = "ranged", shots = { "slow1" }, practice = true },
	{ key = "necromancer", name = "Necromancer", hp = 60, speed = 2.7, reach = 12, keep = 10, el = "void", attack = "ranged", resist = { void = 0.8, fire = -0.3 }, raise = "skeleton", solo = true, spawn = "far" },
	{ key = "lurker", name = "Lurker", hp = 70, speed = 3.4, dmg = 12, reach = 1.5, size = 1.1, attack = "burrow", mass = 1.6, resist = { earth = 0.4, frost = -0.3 }, spawn = "under" },
	{ key = "sprite", name = "Storm Sprite", hp = 26, speed = 5.5, dmg = 7, reach = 6.5, keep = 4.5, el = "lightning", size = 0.7, attack = "dash", wind = 0.5, resist = { earth = -0.6 }, heals_from = "lightning", dash = { 15, 0.32 } },
	{ key = "toad", name = "Bog Toad", hp = 55, speed = 2.4, dmg = 7, reach = 12, keep = 8, el = "mud", attack = "lob", wind = 0.7, resist = { water = 1, earth = 0.3, fire = -0.4 }, always_wet = true, spawn = "water" },
	{ key = "shieldbearer", name = "Shieldbearer", hp = 120, speed = 2.4, dmg = 14, reach = 1.6, size = 1.15, mass = 2.5, resist = { lightning = -0.3 }, shield = true },
	{ key = "hellhound", name = "Hellhound", hp = 38, speed = 5.6, dmg = 10, reach = 5.5, el = "fire", size = 0.85, attack = "dash", wind = 0.55, resist = { fire = 1, water = -0.5, frost = -0.3 }, spawn = "flank", dash = { 13, 0.3 }, trail = "fire", pack = 3 },
	{ key = "gravekeeper", name = "The Gravekeeper", model = "skeleton", hp = 1100, speed = 2.6, dmg = 26, reach = 2.6, size = 2.3, attack = "slam", ring = 16, mass = 8, undead = true, resist = { void = 0.8, frost = 0.3 }, boss = true, summon = { "skeleton", "skeleton", "skeleton", "watcher" }, tempo = 1.6 },
	{ key = "abbot", name = "The Bloated Abbot", model = "bile", hp = 1300, speed = 2, dmg = 26, reach = 2.6, el = "water", size = 2.4, attack = "slam", ring = 16, mass = 8, resist = { water = 1 }, boss = true, summon = { "cultist", "ghoul", "ghoul" }, nova = "mud" },
	{ key = "drowned_king", name = "The Drowned King", model = "drowned", hp = 1400, speed = 2.6, dmg = 26, reach = 2.6, el = "water", size = 2.3, attack = "slam", ring = 16, mass = 8, resist = { water = 1, fire = 0.5 }, always_wet = true, boss = true, summon = { "drowned", "drowned", "bile" }, nova = "water" },
	{ key = "warden", name = "The Frost Warden", model = "frost_brute", hp = 1600, speed = 2.4, dmg = 30, reach = 2.8, el = "frost", size = 2.4, attack = "slam", ring = 16, mass = 9, resist = { frost = 1, fire = -0.3 }, boss = true, summon = { "wraith", "wraith", "skeleton" }, nova = "ice" },
	{ key = "pit_lord", name = "The Pit Lord", model = "demon", hp = 2200, speed = 2.8, dmg = 34, reach = 3, el = "fire", size = 2.6, attack = "slam", ring = 16, mass = 10, resist = { fire = 1, frost = -0.3 }, boss = true, summon = { "imp", "imp", "demon" }, nova = "magma" },
	{ key = "bat", name = "Cave Bat", hp = 16, speed = 6, dmg = 6, reach = 5, keep = 3.5, el = "air", attack = "dash", wind = 0.45, resist = { air = 0.5, lightning = -0.4 }, spawn = "far", dash = { 14, 0.25 }, pack = 4 },
	{ key = "shambler", name = "Spore Shambler", hp = 85, speed = 2, dmg = 11, reach = 1.5, el = "poison", size = 1.15, mass = 2, resist = { poison = 1, fire = -0.5, water = 0.3 }, burst = "poison" },
	{ key = "crawler", name = "Crystal Crawler", hp = 60, speed = 2.6, reach = 10, keep = 7, el = "ice", attack = "ranged", shots = { "fan3", "burst3" }, wind = 0.8, mass = 1.6, resist = { frost = 1, ice = 1, earth = -0.4 } },
	{ key = "crystal_mother", name = "The Crystal Mother", model = "crawler", hp = 1500, speed = 2.4, dmg = 26, reach = 2.8, el = "ice", size = 2.5, attack = "slam", ring = 16, mass = 9, resist = { frost = 1, ice = 1, earth = -0.3 }, boss = true, summon = { "crawler", "bat", "bat" }, nova = "lightning" },
}

local monsters = {}
for i, row in kinds do
	local k = table.clone(DEFAULT)
	for f, v in row do
		k[f] = v
	end
	monsters[i] = k
end

return { monsters = monsters }
