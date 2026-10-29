-- Spellstorm's spell grammar (a port of grammar.gd): which spell a queue of
-- elements plus a cast form makes, its name and its numbers, and what each
-- element does when it lands. This file hot-reloads: edit and save while
-- the game runs. API: ../README.md.
--
-- * The form is the button: RT casts ahead ("cast"), RB bursts around you
--   ("nova"), LS plants at your feet ("plant"), RS enchants your bolts and
--   staff ("enchant").
-- * The heaviest class decides the shape of a cast: solid throws, energy
--   strikes from the sky, spirit beams, flow sprays.
-- * Planting one or two elements makes a trap, a full queue a sentry.
-- * Within that class the element you have most of leads, every other one
--   rides along as payload. A full queue cast ahead is a cataclysm.
--
-- The game fits each spell's `rate` so every spell is worth about the same
-- (balance.rs); set `n.fit = false` to keep your own rate.

-- What one unit of each element does on a hit (element.gd).
local effects = {
	poison = { dmg = 6, poison = 1 },
	void = { dmg = 16 },
	fire = { dmg = 7, burn = 1 },
	frost = { dmg = 3, chill = 1 },
	water = { dmg = 1, wet = 1, push = 1.5 },
	lightning = { dmg = 12, shock = 1 },
	earth = { dmg = 16, push = 2 },
	air = { dmg = 1, push = 4 },
	steam = { dmg = 10, wet = 0.6, burn = 0.3 },
	ice = { dmg = 12, chill = 1.2 },
	mud = { dmg = 5, slow = 1, wet = 0.4 },
	magma = { dmg = 14, burn = 1.2 },
	storm = { dmg = 9, shock = 0.8, push = 2 },
}

local CAST_NOUN = {
	earth = "Boulder", ice = "Ice Shards", mud = "Mud Ball", magma = "Lava Bomb",
	lightning = "Thunderbolt", storm = "Thunderclap",
	poison = "Venom Beam", void = "Void Ray",
	fire = "Flamethrower", frost = "Frost Breath", water = "Water Jet", steam = "Steam Jet", air = "Gale",
}

local NOVA_NOUN = {
	poison = "Toxic Burst", void = "Void Burst", fire = "Fire Nova", frost = "Frost Nova", water = "Splash",
	lightning = "Thunder Ring", earth = "Shockwave", air = "Gust Ring", steam = "Steam Burst",
	ice = "Ice Ring", mud = "Mudslide", magma = "Eruption", storm = "Tempest",
}

local PLANT_NOUN = {
	earth = "Spike Trap", ice = "Ice Spikes", mud = "Mud Trap", magma = "Lava Trap",
	lightning = "Shock Trap", storm = "Thunder Trap", poison = "Poison Trap", void = "Void Mine",
	fire = "Fire Trap", frost = "Frost Trap", water = "Puddle Trap", steam = "Steam Vent", air = "Gust Trap",
}

local SENTRY_NOUN = {
	earth = "Stone Sentry", ice = "Ice Sentry", mud = "Mud Sentry", magma = "Lava Sentry",
	lightning = "Tesla Sentry", storm = "Storm Sentry", poison = "Venom Sentry", void = "Void Sentry",
	fire = "Fire Sentry", frost = "Frost Sentry", water = "Water Sentry", steam = "Steam Sentry", air = "Wind Sentry",
}

local OVERLOAD_NOUN = {
	fire = "Meteor", frost = "Blizzard", water = "Tidal Wave", lightning = "Thunderstorm",
	earth = "Earthquake", air = "Tornado", poison = "Plague", void = "Black Hole",
}

-- A full queue cast ahead, led by the first and ridden by the second.
local CATACLYSM = {
	["lightning+water"] = "Tempest", ["water+lightning"] = "Tempest",
	["fire+air"] = "Firestorm", ["air+fire"] = "Firestorm",
	["fire+earth"] = "Meteor Shower", ["earth+fire"] = "Eruption",
	["frost+air"] = "Whiteout", ["water+frost"] = "Frozen Sea",
	["void+lightning"] = "Event Horizon", ["poison+water"] = "Blight Tide",
}

local ADJ = {
	poison = "Venomous", void = "Cursed", fire = "Burning", frost = "Freezing", water = "Soaking",
	lightning = "Shocking", earth = "Crushing", air = "Gusting", steam = "Scalding", ice = "Glacial",
	mud = "Muddy", magma = "Molten", storm = "Thundering",
}

local ENCHANT_ADJ = { earth = "Stone", air = "Wind" }

-- The heaviest class first.
local CAST_SHAPE = { solid = "projectile", energy = "strike", spirit = "beam", flow = "spray" }
local RANK = { solid = 1, energy = 2, spirit = 3, flow = 4 }

local STEP = 5 / QUEUE_SIZE

local function first(slots, e)
	for i, s in slots do
		if s == e then
			return i
		end
	end
	return math.huge
end

-- The element with the highest count among `pool`; ties go to the one
-- conjured first.
local function most(slots, count, pool)
	local best = pool[1]
	for _, e in pool do
		local ce, cb = count[e] or 0, count[best] or 0
		if ce > cb or (ce == cb and first(slots, e) < first(slots, best)) then
			best = e
		end
	end
	return best
end

local function adjectives(list)
	local s = ""
	for i = 1, math.min(2, #list) do
		s ..= ADJ[list[i]] .. " "
	end
	return s
end

-- A fused slot counts as this base element in a cataclysm.
local FUSED_BASE = { steam = "water", ice = "frost", mud = "earth", magma = "fire", storm = "lightning" }

-- A full queue cast ahead: the base element with the most slots leads
-- (fused slots count as their base), the rest ride.
local function cataclysm(spell, slots)
	local order, score = {}, {}
	local function bump(b, n)
		if not score[b] then
			score[b] = 0
			table.insert(order, b)
		end
		score[b] += n
	end
	for _, e in slots do
		local b = FUSED_BASE[e] or e
		bump(b, 2)
		for _, part in ELEMENTS[e].parts do
			if part ~= b then
				bump(part, 1)
			end
		end
	end
	local lead = nil
	for _, b in order do
		if lead == nil or score[b] > score[lead] or (score[b] == score[lead] and RANK[ELEMENTS[b].kind] < RANK[ELEMENTS[lead].kind]) then
			lead = b
		end
	end
	local riders = {}
	for _, b in order do
		if b ~= lead then
			table.insert(riders, b)
		end
	end
	table.sort(riders, function(a, b)
		if score[a] ~= score[b] then
			return score[a] > score[b]
		end
		return ELEMENTS[a].index < ELEMENTS[b].index
	end)
	spell.shape = "overload"
	spell.primary = lead
	spell.riders = riders
	local named = riders[1] and CATACLYSM[lead .. "+" .. riders[1]]
	spell.name = named or (adjectives(riders) .. (OVERLOAD_NOUN[lead] or "Cataclysm"))
end

-- Size, range and timing of the spell's shape. `rate` is how hard it hits
-- (before the game fits it); a shape without one never hits.
local function numbers(spell, count, mods)
	local p = #spell.slots * STEP
	local lead = (spell.primary and count[spell.primary] or 0) * STEP
	local shape = spell.shape
	if shape == "spray" then
		return { range = (3.2 + 0.9 * lead) * mods.reach, angle = 26 + 4 * p, duration = 2.5 + 0.5 * p, rate = 1 }
	elseif shape == "beam" then
		return { range = 15 * mods.reach, width = 0.25 + 0.08 * p, duration = 3 + 0.5 * p, rate = 1.3 }
	elseif shape == "strike" then
		return { range = 8 + 0.5 * p, radius = 1 + 0.2 * p + 0.1 * lead, rate = 1 }
	elseif shape == "projectile" then
		local ice = spell.primary == "ice"
		local pieces = if ice then math.round(3 + 2 * lead) else 1
		return {
			pieces = pieces,
			spread = 8 + 3 * pieces,
			speed = 16,
			radius = 0.25 + 0.12 * lead,
			charge_time = mods.charge,
			rate = 1 / pieces * (if pieces > 1 then 1.4 else 1),
			splash = if spell.primary == "magma" or spell.primary == "mud" then 2.2 + 0.4 * lead else 0,
			pierce = ice,
		}
	elseif shape == "nova" then
		return { radius = 2.4 + 0.65 * p, rate = 1.25 }
	elseif shape == "trap" then
		return { radius = 2.2 + 0.35 * p, field = 1.4 + 0.35 * p, field_time = 5 + p, stun = 0.8 + 0.25 * lead, duration = 30, rate = 1.6 }
	elseif shape == "sentry" then
		return { range = 9, interval = 0.3, duration = 14, rate = 0.5 }
	elseif shape == "enchant" then
		return { duration = ENCHANT_TIME * mods.enchant_time }
	elseif shape == "overload" then
		return { radius = 5, rate = 4 }
	elseif shape == "staff" then
		return { range = 1.8, rate = 1 }
	elseif shape == "stomp" then
		return { radius = 2.5, rate = 1 }
	end
	return {}
end

-- `slots` in queue order (element names), `form` the button, `mods` the
-- caster's tomes: they change numbers, never the shape or the name.
local function compose(slots, form, mods)
	local count = {}
	for _, e in slots do
		count[e] = (count[e] or 0) + 1
	end
	local spell = { slots = slots, shape = "fizzle", name = "", payload = {}, riders = {} }
	if #slots == 0 then
		if form == "cast" or form == "plant" then
			spell.shape, spell.name = "staff", "Staff Swing"
		elseif form == "nova" then
			spell.shape, spell.name = "stomp", "Stomp"
		else
			spell.shape, spell.name = "fizzle", "Fizzle"
		end
		spell.key = form .. "|" .. spell.shape
		spell.n = numbers(spell, count, mods)
		return spell
	end

	local primary
	if form == "nova" or form == "enchant" then
		primary = most(slots, count, slots)
	else
		local best = nil
		for _, e in slots do
			local k = ELEMENTS[e].kind
			if best == nil or RANK[k] < RANK[best] then
				best = k
			end
		end
		local heavy = {}
		for _, e in slots do
			if ELEMENTS[e].kind == best then
				table.insert(heavy, e)
			end
		end
		primary = most(slots, count, heavy)
		if form == "cast" then
			spell.shape = CAST_SHAPE[best]
		elseif #slots >= QUEUE_SIZE then
			spell.shape = "sentry"
		else
			spell.shape = "trap"
		end
	end
	spell.primary = primary

	local payload = {}
	for _, e in slots do
		if e ~= primary and not table.find(payload, e) then
			table.insert(payload, e)
		end
	end
	table.sort(payload, function(a, b)
		if count[a] ~= count[b] then
			return count[a] > count[b]
		end
		return first(slots, a) < first(slots, b)
	end)
	spell.payload = payload

	if form == "cast" then
		if #slots >= QUEUE_SIZE then
			cataclysm(spell, slots)
		else
			spell.name = adjectives(payload) .. CAST_NOUN[primary]
		end
	elseif form == "nova" then
		spell.shape = "nova"
		spell.name = adjectives(payload) .. NOVA_NOUN[primary]
	elseif form == "plant" then
		local noun = if spell.shape == "sentry" then SENTRY_NOUN[primary] else PLANT_NOUN[primary]
		spell.name = adjectives(payload) .. noun
	else
		spell.shape = "enchant"
		local words = {}
		for _, e in { primary, payload[1], payload[2] } do
			if #words < 2 then
				table.insert(words, ENCHANT_ADJ[e] or ELEMENTS[e].name)
			end
		end
		spell.name = table.concat(words, " ") .. " Enchantment"
	end

	-- The grimoire's key: form, shape, lead and the payload in element order.
	local sorted = table.clone(payload)
	table.sort(sorted, function(a, b)
		return ELEMENTS[a].index < ELEMENTS[b].index
	end)
	local list = {}
	for _, e in sorted do
		table.insert(list, tostring(ELEMENTS[e].index))
	end
	spell.key = `{form}|{spell.shape}|{ELEMENTS[primary].index}|{table.concat(list, ",")}`
	spell.n = numbers(spell, count, mods)
	return spell
end

return { compose = compose, effects = effects }
