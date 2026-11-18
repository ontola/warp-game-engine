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

-- A new person: { name, before, now }. pick(t) chooses one of t (the
-- game's own dice, so who people are does not change the land).
function M.roll(pick)
	return { name = pick(FIRST) .. " " .. pick(LAST), before = pick(BEFORE), now = pick(NOW) }
end

return M
