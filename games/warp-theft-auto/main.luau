local city = require("world")
local mobility = require("mobility")
local interiors = require("interiors")
local traffic = require("traffic")
local population = require("population")
local impacts = require("impacts")
local daycycle = require("kit/daycycle")
local ambience = require("ambience")
local dispatch = require("dispatch")
local instruments = require("instruments")
local stories = require("story_board")

local function join(player)
	mobility.join(state.fleet, player.id)
	state.fleet.players[player.id].bot = player.bot
end

local function init()
	state.seed = random.int(1, 1000000)
	state.city = city.build(state.seed)
	state.fleet = mobility.new(state.city)
	state.interiors = interiors.new(state.city.buildings)
	traffic.add(state.fleet)
	for _, player in players.list() do join(player) end
	state.population = population.new(state.city, state.seed)
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
	state.storyBoard = {choice = 1}
	state.job = dispatch.new(state.city)
	light.set({exposure = "auto", exposure_min = 3, exposure_max = 16, adapt_speed = 6})
	post.set({bloom=0.1, saturation=1.04})
end

local function tick(dt, input)
	daycycle.tick(state.clock, dt)
	stories.input(state, input)
	mobility.tick(state.fleet, dt, input)
	interiors.update(state.interiors, state.city.buildings, state.fleet.players)
	population.tick(state.population, state.city, state.fleet.players, dt, state.clock.hour, state.interiors)
	if state.storyCast then state.storyCast:tick(dt) end
	impacts.tick(state.population, state.fleet, dt)
	if state.story then stories.tick(state, dt, input) else dispatch.tick(state.job, state.fleet, dt) end
	ambience.headlights(state.fleet.players, state.clock)
	if game.tick() % 4 == 0 then ambience.tick(state.fleet) end
	if game.tick() % 15 == 0 then
		city.set_clock(state.city, state.clock.hour)
		ambience.lamps(state.city, state.fleet.players, state.clock)
	end
end

local function draw()
	for _, player in players.list() do
		if not player.bot then instruments.draw(state.city, state.fleet, state.clock, state.job, player, state.story, state.storyBoard, stories.titles) end
	end
end

return {
	init = init, tick = tick, draw = draw, player_joined = join,
	apply_story = function(definition) return stories.start(state, definition) end,
	player_left = function(player) mobility.leave(state.fleet, player.id) end,
}
