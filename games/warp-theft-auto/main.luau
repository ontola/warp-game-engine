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
	ambience.headlights(state.fleet.players, state.clock)
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
local MENU_CONTROLS = { { "move", "Walk, drive or fly" }, { "Y", "Take a vehicle or get out" }, { "X", "Stories" }, { "LB", "Weapon wheel (hold)" }, { "RB", "Sprint" }, { "A", "Handbrake, climb" } }
local function menu_controls(top)
	local w, h = hud.size()
	local k = h / 720
	local people = {}
	for _, pl in players.list() do
		if not pl.bot then table.insert(people, pl) end
	end
	if #people == 0 then return end
	local y0 = top + 30 * k
	hud.text("CONTROLS", 0, y0, { size = 16 * k, align = "center", width = w, color = { 1, 0.85, 0.4, 1 } })
	-- One player: the controls in a row (wrapping), so they fit under
	-- whatever the game shows; more: a column each, rows as tall as fit.
	if #people == 1 then
		local pl = people[1]
		local size, gap, maxw = 28 * k, 32 * k, math.min(w - 80 * k, 1000 * k)
		local items = {}
		for _, row in MENU_CONTROLS do
			local key = hud.key(pl.id, row[1])
			local bw = if key then math.max(size, hud.measure(key, size * 0.56) + size * 0.6) else size
			table.insert(items, { row = row, w = bw + 10 * k + hud.measure(row[2], 18 * k) })
		end
		local lines, line, lw = {}, {}, 0
		for _, it in items do
			if #line > 0 and lw + gap + it.w > maxw then table.insert(lines, { items = line, w = lw }); line, lw = {}, 0 end
			lw += (if #line > 0 then gap else 0) + it.w
			table.insert(line, it)
		end
		table.insert(lines, { items = line, w = lw })
		local y = y0 + 34 * k
		for _, l in lines do
			local x = w / 2 - l.w / 2
			for _, it in l.items do
				local bw = hud.button(pl.id, it.row[1], x, y, size)
				hud.text(it.row[2], x + bw + 10 * k, y + 3 * k, { size = 18 * k, color = { 0.95, 0.95, 0.95, 1 } })
				x += it.w + gap
			end
			y += 40 * k
		end
		return
	end
	local colw = 300 * k
	local left = w / 2 - colw * #people / 2
	local step = math.clamp((h - 12 * k - (y0 + 70 * k)) / #MENU_CONTROLS, 30 * k, 40 * k)
	for n, pl in people do
		local x = left + (n - 1) * colw + 30 * k
		local y = y0 + 34 * k
		hud.text(pl.name, x, y, { size = 20 * k, color = { pl.color.x, pl.color.y, pl.color.z, 1 } })
		y += 36 * k
		for _, row in MENU_CONTROLS do
			local bw = hud.button(pl.id, row[1], x, y, step * 0.75)
			hud.text(row[2], x + bw + 12 * k, y + 4 * k, { size = step * 0.47, width = colw - bw - 40 * k, color = { 0.95, 0.95, 0.95, 1 } })
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
