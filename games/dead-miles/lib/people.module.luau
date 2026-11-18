-- Who a new body is: a name and a line of before. Shown on the profile
-- card when someone gets a new person (main.luau). Later, hand-made
-- special characters can be added here with their own kit and stats.

local M = {}

local FIRST = {
	"Anna", "Bram", "Carla", "Dev", "Eli", "Fenna", "Gus", "Hana", "Ivo", "Jun",
	"Kees", "Lena", "Milo", "Noor", "Otis", "Pia", "Quinn", "Rosa", "Sam", "Tess",
	"Umar", "Vera", "Wim", "Xena", "Yara", "Zeno", "Bea", "Daan", "Femke", "Joris",
	"Lot", "Mo", "Nina", "Pim", "Ruth", "Sven", "Tara", "Ugo", "Wout", "Ilse",
}

local LAST = {
	"de Wit", "Bakker", "Visser", "Moreau", "Novak", "Okafor", "Lindqvist", "Ruiz",
	"Hoekstra", "Brandt", "Kowalski", "Ito", "Mendes", "Vos", "Achterberg", "Klein",
	"Duval", "Haddad", "Smit", "Ferreira",
}

-- What they did before, and something about them now.
local BEFORE = {
	"Drove the night bus for twenty years.",
	"Was a nurse on the late shift.",
	"Taught PE at a school that is ash now.",
	"Delivered parcels; knows every back street.",
	"Ran a corner shop and still counts everything.",
	"Was a bricklayer. Hands like shovels.",
	"Sold insurance. Nobody claims anymore.",
	"Studied philosophy. It has not helped.",
	"Worked the docks, nights mostly.",
	"Was a cook in a hotel kitchen.",
	"Repaired bicycles under a railway arch.",
	"Was a churchwarden; rang the bell once too often.",
	"Climbed walls for fun. Now for a living.",
	"Was a security guard who never saw anything happen.",
	"Kept bees. Misses the quiet.",
	"Was a hairdresser and talks to stay calm.",
}

local NOW = {
	"Hums when frightened.",
	"Keeps a photo nobody is allowed to see.",
	"Has not slept properly in weeks.",
	"Counts the steps between lamp posts.",
	"Swears they heard a radio last night.",
	"Wants to see the sea once more.",
	"Does not talk about the first night.",
	"Collects lighters.",
	"Still wears a wedding ring.",
	"Laughs at the wrong moments.",
	"Is looking for a brother who went east.",
	"Never turns their back to a door.",
}

-- Where they are from (made-up towns along the old line west).
local FROM = {
	"Eastwick", "Harrowgate", "Velde", "Oudeveen", "Marrowby", "Brakel", "Stonebridge",
	"Lindholm", "Kessel", "Ashcombe", "Noordhoek", "Fairhaven", "Wendlebury", "Graswijk",
}

-- Who they lost: what they call them, in their own words, and a name to
-- go with it.
local MEN = { "Bram", "Dev", "Gus", "Ivo", "Kees", "Milo", "Otis", "Umar", "Wim", "Zeno", "Daan", "Joris", "Pim", "Sven", "Wout", "Teun", "Jasper", "Ruben" }
local WOMEN = { "Anna", "Carla", "Fenna", "Hana", "Lena", "Noor", "Pia", "Rosa", "Tess", "Vera", "Yara", "Bea", "Femke", "Lot", "Nina", "Ruth", "Ilse", "Marit" }
local ANY = { "Sam", "Quinn", "Jun", "Mo", "Eli", "Robin", "Kim", "Charlie" }
local LOST = {
	{ "husband", MEN }, { "wife", WOMEN }, { "daughter", WOMEN }, { "son", MEN },
	{ "sister", WOMEN }, { "brother", MEN }, { "mother", WOMEN }, { "father", MEN },
	{ "best friend", ANY }, { "twin", ANY }, { "grandmother", WOMEN }, { "little brother", MEN },
	{ "little sister", WOMEN }, { "fiancée", WOMEN }, { "fiancé", MEN }, { "partner", ANY },
}

-- A new person: { name, first, before, now, from, who, lost }: who is
-- whom they lost ("sister"), lost that one's name. pick(t) chooses one of t
-- (the game's own dice, so who people are does not change the land).
function M.roll(pick)
	local first = pick(FIRST)
	local p = { name = first .. " " .. pick(LAST), first = first, before = pick(BEFORE), now = pick(NOW) }
	p.from = pick(FROM)
	local lost = pick(LOST)
	p.who = lost[1]
	p.lost = pick(lost[2])
	if p.lost == first then p.lost = pick(lost[2]) end
	return p
end

return M
