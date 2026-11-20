-- Live knobs: the match's feel in a few numbers, for whoever is playing
-- (the `knobs` and `set_knob` live tools, the session assistant and the
-- Splitscreen Club app's settings change them). Each one multiplies what the
-- cards give, for every player; 1 is the game as designed. A change saves
-- this file, which hot-reloads and applies at once, mid-round too.

-- @knobs
local KNOBS = {
	damage = 1.0, -- damage of every hit and blast, 0.25 to 3
	fire_rate = 1.0, -- how fast guns fire, 0.25 to 3
	move_speed = 1.0, -- how fast players run, 0.5 to 2
	bullet_speed = 1.0, -- how fast bullets fly, 0.25 to 3
	explosions = 1.0, -- blast radius of explosive rounds, 0.25 to 3
	health = 1.0, -- everyone's health, 0.5 to 3
}
-- @end

-- Weapon fields each knob scales.
local FIELDS = {
	fire_rate = "fire_rate_mult",
	move_speed = "move_speed_mult",
	bullet_speed = "bullet_speed_mult",
	explosions = "explosive_radius",
}

local function signature()
	local s = ""
	for _, k in { "damage", "fire_rate", "move_speed", "bullet_speed", "explosions", "health" } do
		s ..= k .. "=" .. tostring(KNOBS[k]) .. ";"
	end
	return s
end

-- The cards' values this round, before the knobs (weapons are rebuilt
-- from the cards each round).
local function capture(g)
	state.base = {}
	for p = 1, g:players() do
		local b = {}
		for _, field in FIELDS do b[field] = g:stat(p, field) end
		b.max_hp_bonus = g:stat(p, "max_hp_bonus")
		state.base[p] = b
	end
	state.round = g:round()
end

local function apply(g)
	if state.base == nil or state.round ~= g:round() then capture(g) end
	for p = 1, g:players() do
		local b = state.base[p]
		if b then
			for knob, field in FIELDS do
				if b[field] ~= nil then g:set_stat(p, field, b[field] * KNOBS[knob]) end
			end
			-- Health: 100 plus the cards' bonus, scaled; topped up when raised.
			local before = g:max_health(p)
			g:set_stat(p, "max_hp_bonus", math.floor((100 + b.max_hp_bonus) * KNOBS.health + 0.5) - 100)
			local gained = g:max_health(p) - before
			if gained > 0 and g:alive(p) then g:set_health(p, g:health(p) + gained) end
		end
	end
	state.applied = signature()
end

return {
	round_start = function(g)
		capture(g)
		apply(g)
	end,
	tick = function(g)
		if state.applied ~= signature() or state.round ~= g:round() then apply(g) end
	end,
	hit = function(g, e)
		if KNOBS.damage ~= 1 then return e.damage * KNOBS.damage end
		return nil
	end,
}
