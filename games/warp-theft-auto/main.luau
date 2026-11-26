local city = require("world")
local cape = require("west-cape")
local map = require("map_edit")
local mobility = require("mobility")
local interiors = require("interiors")
local traffic = require("traffic")
local drivers = require("traffic_drivers")
local population = require("population")
local impacts = require("impacts")
local daycycle = require("kit/daycycle")
local ambience = require("ambience")
local dispatch = require("dispatch")
local instruments = require("instruments")
local stories = require("story_board")
local waypoint = require("waypoint")
local combat = require("combat")
local wheel = require("weapon_wheel")

local function join(player)
	mobility.join(state.fleet, player.id)
	state.fleet.players[player.id].bot = player.bot
end

local function init()
	fx.warm_explosions()
	state.seed = random.int(1, 1000000)
	local layout, baked = map.initial(state)
	state.city = city.build(state.seed, layout, baked)
	state.fleet = mobility.new(state.city)
	state.interiors = interiors.new(state.city.buildings)
	traffic.add(state.fleet)
	for _, player in players.list() do join(player) end
	state.population = population.new(state.city, state.seed)
	drivers.bind(state.fleet,state.population)
	state.combat = combat.new(state.fleet, state.population)
	state.clock = daycycle.new(16, 480, "clear")
	daycycle.tick(state.clock, 0)
	-- A maritime cumulus layer shared by the streets, parks and flight views.
	-- Seed the fair-weather cover and breeze without advancing the city RNG.
	sky.clouds({
		coverage = .44 + .12 * (state.seed % 97) / 96,
		base = 1400, thickness = 1000, far = 36000,
		wind_x = 4 + 2 * (state.seed % 53) / 52, wind_z = 1.5,
		shadow = .5, steps = 36, light_steps = 4, aerial = false,
	})
	city.set_clock(state.city, state.clock.hour)
	stories.warm()
	waypoint.warm()
	state.storyBoard = {choice = 1}
	state.job = dispatch.new(state.city)
	waypoint.update(state)
	light.set({exposure = "auto", exposure_min = 3, exposure_max = 16, adapt_speed = 6})
	post.set({bloom=0.1, saturation=1.04})
end

local function tick(dt, input)
	daycycle.tick(state.clock, dt)
	wheel.input(state, input)
	stories.input(state, input)
	mobility.tick(state.fleet, dt, input)
	drivers.tick(state.fleet,dt)
	-- Runtime stories can move all four players between the city and woods in
	-- one tick. Release the previous rooms and trunks before filling either pool.
	local wanted, views = interiors.release_unused(state.interiors,state.city.buildings,state.fleet.players)
	cape.update(state.city.cape,state.fleet.players)
	interiors.load_visible(state.interiors,state.city.buildings,wanted,views)
	population.tick(state.population, state.city, state.fleet.players, dt, state.clock.hour, state.interiors)
	if state.storyCast then state.storyCast:tick(dt) end
	impacts.tick(state.population, state.fleet, dt)
	combat.tick(state, dt, input)
	if state.story then stories.tick(state, dt, input) else dispatch.tick(state.job, state.fleet, dt) end
	waypoint.update(state)
	ambience.headlights(state.fleet, state.clock)
	if game.tick() % 4 == 0 then ambience.tick(state.fleet) end
	if game.tick() % 15 == 0 then
		city.set_clock(state.city, state.clock.hour)
		ambience.lamps(state.city, state.fleet.players, state.clock)
	end
	ambience.weaponLights(state.clock,state.combat.players)
end

local function draw()
	for _, player in players.list() do
		if not player.bot then instruments.draw(state.city, state.fleet, state.clock, state.job, player, state.story, state.storyBoard, stories.titles, state.combat.players[player.id]) end
	end
end

-- The start menu shows each player's controls on their own buttons (a
-- disc on a controller, the key on a keyboard or keyboard half).
local MENU_CONTROLS = { { "move", "Walk, drive or fly" }, { "Y", "Take a vehicle or get out" }, { "X", "Stories" }, { "LB", "Weapon wheel (hold)" }, { "RB", "Sprint" }, { "A", "Handbrake" } }
local function menu_controls(top)
	local w, h = hud.size()
	local k = hud.unit()
	-- One column per layout in use: players on the same kind of controller
	-- share one; a keyboard (or each keyboard half) gets its own.
	local groups, by = {}, {}
	for _, pl in players.by_couch() do
		if not pl.bot then
			local sig = ""
			for _, row in MENU_CONTROLS do sig ..= (hud.key(pl.id, row[1]) or "pad") .. "|" end
			if not by[sig] then
				by[sig] = { first = pl, names = {} }
				table.insert(groups, by[sig])
			end
			table.insert(by[sig].names, pl.name)
		end
	end
	if #groups == 0 then return end
	local y0 = top + 30 * k
	hud.text("CONTROLS", 0, y0, { size = 16 * k, align = "center", width = w, color = { 1, 0.85, 0.4, 1 } })
	local colw = 320 * k
	local left = w / 2 - colw * #groups / 2
	local head = if #groups > 1 then 30 * k else 0
	local step = math.clamp((h - 12 * k - (y0 + 34 * k + head)) / #MENU_CONTROLS, 22 * k, 40 * k)
	for n, g in groups do
		-- The column centred in its share of the width.
		local size, bmax, tmax = step * 0.75, 0, 0
		for _, row in MENU_CONTROLS do
			local key = hud.key(g.first.id, row[1])
			bmax = math.max(bmax, if key then math.max(size, hud.measure(key, size * 0.56) + size * 0.6) else size)
			tmax = math.max(tmax, hud.measure(row[2], step * 0.47))
		end
		local cw = bmax + 12 * k + tmax
		local x = left + (n - 1) * colw + math.max(10 * k, (colw - cw) / 2)
		local y = y0 + 34 * k
		if #groups > 1 then
			local c = g.first.color
			hud.text(table.concat(g.names, ", "), x, y, { size = 18 * k, width = colw - 50 * k, color = { c.x, c.y, c.z, 1 } })
			y += head
		end
		for _, row in MENU_CONTROLS do
			-- Buttons centred on one line, the labels lined up after them.
			local bw = hud.measure(hud.key(g.first.id, row[1]) or "", size * 0.56)
			bw = if hud.key(g.first.id, row[1]) then math.max(size, bw + size * 0.6) else size
			hud.button(g.first.id, row[1], x + (bmax - bw) / 2, y, size)
			hud.text(row[2], x + bmax + 12 * k, y + step * 0.1, { size = step * 0.47, width = colw - bmax - 50 * k, color = { 0.95, 0.95, 0.95, 1 } })
			y += step
		end
	end
end


return {
	menu = menu_controls,
	init = init, tick = tick, draw = draw, player_joined = join,
	reload = function()
		map.reload(state)
		traffic.reload(state.fleet)
		drivers.bind(state.fleet,state.population)
		combat.reload(state)
	end,
	get_layout = function() return map.definition(state) end,
	apply_layout = function(definition) return map.apply(state, definition) end,
	apply_story = function(definition)
		if definition.kind == "west-cape-layout" then return map.apply(state, definition) end
		local title, err = stories.start(state, definition)
		if title then waypoint.update(state) end
		return title, err
	end,
	player_left = function(player) mobility.leave(state.fleet, player.id) end,
}
