-- Coin Climb: a side-view platformer for 1 to 4 players. Run and jump up
-- a tower of ledges and grab coins; first to 12, or the most when the
-- clock runs out. Built on the game kit (engine/docs/kit.md): the
-- controller, camera, HUD, round flow and bots are require("kit/...").
-- Edit this file while it plays: saving reloads it.

local controller = require("kit/controller")
local cam = require("kit/camera")
local ui = require("kit/ui")
local round = require("kit/round")
local bot = require("kit/bot")
local util = require("kit/util")

local v = vector.create

local WIN = 12 -- coins to win
local LENGTH = 120 -- seconds per round
local COINS = 8 -- coins in the level at once
local WIDTH = 24 -- level width, metres
local FLOORS = 10 -- rows of ledges
local ROW = 2.2 -- metres between rows: a jump clears about 2.6, a climber is 1.3 tall

local GROUND = "#5d8a4e"
local LEDGE = { "#c9895a", "#b97a52", "#d49b68" }
local COIN = "#ffcc33"

-- The level: a floor, walls, and rows of ledges with gaps, from the seed.
local function build_level()
	world.spawn("cube", { pos = v(0, -0.5, 0), size = v(WIDTH + 4, 1, 6), color = GROUND, tag = "ground" })
	world.spawn("cube", { pos = v(-WIDTH / 2 - 1, 15, 0), size = v(2, 32, 6), color = "#6b5a4a", tag = "wall" })
	world.spawn("cube", { pos = v(WIDTH / 2 + 1, 15, 0), size = v(2, 32, 6), color = "#6b5a4a", tag = "wall" })
	-- A backdrop so the view is never empty sky.
	world.spawn("cube", { pos = v(0, 14, -4), size = v(WIDTH + 30, 40, 1), color = "#93b5c9", roughness = 1, solid = false })
	state.ledges = {}
	for row = 1, FLOORS do
		local x = -WIDTH / 2
		while x < WIDTH / 2 - 1 do
			local len = random.range(2.5, 6)
			local gap = random.range(1.5, 3.5)
			if x + len > WIDTH / 2 then len = WIDTH / 2 - x end
			local top = row * ROW
			world.spawn("cube", { pos = v(x + len / 2, top - 0.25, 0), size = v(len, 0.5, 3), color = util.pick(LEDGE), tag = "ledge" })
			table.insert(state.ledges, { x = x + len / 2, y = top, len = len })
			x += len + gap
		end
	end
end

-- A coin on a random ledge.
local function place_coin()
	local l = util.pick(state.ledges)
	local x = l.x + random.range(-l.len / 2 + 0.4, l.len / 2 - 0.4)
	world.spawn("sphere", { pos = v(x, l.y + 0.7, 0), size = 0.55, color = COIN, emissive = 1.5, roughness = 0.3, solid = false, tag = "coin" })
end

local function add_climber(p)
	local x = -WIDTH / 2 + 3 + (#state.climbers % 4) * 2
	local body = world.spawn("capsule", { pos = v(x, 1, 0), size = v(0.6, 0.65, 0.6), color = p.color, roughness = 0.5, solid = false, tag = "player" }) -- not solid: players pass through each other
	table.insert(state.climbers, {
		id = p.id,
		bot = p.bot,
		body = body,
		ctl = controller.platformer(body, { speed = 7.5, air_jumps = 1, kill_y = -10, spawn = v(x, 2, 0) }),
		cam = cam.rig(p.id, { fov = 50 }),
	})
	state.round:join(p.id)
end

local function init()
	sky.set("morning")
	post.set({ bloom = 0.4, saturation = 1.1, vignette = 0.25 })
	world.warm("sphere", { color = COIN, emissive = 1.5, roughness = 0.3 })
	build_level()
	state.round = round.new({ win_score = WIN, length = LENGTH })
	state.climbers = {}
	for _, p in players.list() do add_climber(p) end
	for _ = 1, COINS do place_coin() end
end

local function climber(id)
	for i, c in state.climbers do
		if c.id == id then return c, i end
	end
	return nil
end

-- A bot goes for the coin that is easiest to reach: climbing costs more
-- than running.
local function bot_target(pos)
	local best, cost = nil, math.huge
	for _, id in world.find("coin") do
		local at = world.get(id).pos
		local up = at.y - pos.y
		local k = math.abs(at.x - pos.x) + math.max(up, 0) * 4 + math.max(-up, 0)
		if k < cost then best, cost = at, k end
	end
	return best
end

local function tick(dt, input)
	state.round:tick(dt)
	local playing = state.round:playing()
	for _, c in state.climbers do
		local me = world.get(c.body)
		local inp = input[c.id]
		if c.bot then
			local coin = bot_target(me.pos)
			inp = if coin then bot.run_to(c.body, coin, { ctl = c.ctl }) else bot.input()
		end
		if not playing then inp = bot.input() end
		local ev = c.ctl:update(inp, dt)
		if ev.jumped then sound.play("jump", { at = me.pos, pitch = if ev.air_jumped then 1.3 else 1 }) end
		if ev.fell then c.cam:snap() end
		if playing then
			for _, coin in world.overlap(me.pos, 0.7, { tag = "coin" }) do
				local at = world.get(coin).pos
				world.remove(coin)
				fx.burst(at, { color = COIN, emissive = 0.6, count = 10, speed = 3 })
				sound.play("pickup", { at = at, pitch = 1 + 0.05 * (state.round:score(c.id) % 6) })
				players.rumble(c.id, 0.3, 0.1)
				state.round:add(c.id, 1)
				place_coin()
			end
		end
		if not c.bot then c.cam:side(me.pos, dt, { vel = c.ctl.vel, distance = 15, height = 2.5 }) end
	end
end

local function draw()
	for _, c in state.climbers do
		if not c.bot then
			local p = players.get(c.id)
			if p then ui.score(c.id, p.name, state.round:score(c.id), p.color) end
		end
	end
	state.round:draw()
end

local function player_joined(p) add_climber(p) end

local function player_left(p)
	local c, i = climber(p.id)
	if c then
		world.remove(c.body)
		table.remove(state.climbers, i)
	end
end

return { init = init, tick = tick, draw = draw, player_joined = player_joined, player_left = player_left }
