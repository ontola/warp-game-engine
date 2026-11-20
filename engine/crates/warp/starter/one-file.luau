-- Hello Warp in one file: a capsule that runs and jumps on a floor,
-- collects glowing orbs and shows the score. Run it with
--   warp run hello.luau
-- and edit this file while it plays: saving reloads it, and the world and
-- `state` stay. The `--!` lines below are its settings (game.toml keys);
-- the sound patches are at the bottom, in `SOUNDS`. The API is in
-- types.d.luau (`warp new DIR` writes it) and engine/docs/warp-cli.md.
--! title = "Hello Warp"
--! seed = 7
--! sky = "afternoon"

local SPEED = 6 -- metres per second
local JUMP = 7.5 -- take-off speed, m/s
local GRAVITY = 20 -- m/s^2
local ORBS = 6
local WIN = 10 -- orbs to win
local LOOK = vector.create(0, 0.6, 0)

local function place_orb()
	local a = random.range(0, 2 * math.pi)
	local r = random.range(3, 9)
	world.spawn("sphere", {
		pos = vector.create(math.cos(a) * r, 0.8, math.sin(a) * r),
		size = 0.6,
		color = "#ffd24a",
		emissive = 3,
		solid = false,
		tag = "orb",
	})
end

local function forward(yaw)
	return vector.create(-math.sin(yaw), 0, -math.cos(yaw))
end

-- A runner for player `p`: a capsule 0.8 m wide and 1.8 m tall (its size
-- is half its height) in the player's colour: the first in the middle,
-- the others on a ring around it.
local function add_runner(p)
	local n = #state.runners
	local a, d = n * 1.7, if n == 0 then 0 else 2.5
	local body = world.spawn("capsule", { pos = vector.create(math.cos(a) * d, 1, math.sin(a) * d), size = vector.create(0.8, 0.9, 0.8), color = p.color, tag = "player" })
	table.insert(state.runners, { id = p.id, name = p.name, bot = p.bot, body = body, vel = vector.create(0, 0, 0), yaw = 0, grounded = false, score = 0 })
end

local function runner(id)
	for i, r in state.runners do
		if r.id == id then return r, i end
	end
	return nil
end

local function init()
	world.spawn("cube", { pos = vector.create(0, -0.5, 0), size = vector.create(120, 1, 120), color = "#6f9a5a", tag = "floor" })
	for i = 1, 3 do
		local a = i * 2.1
		world.spawn("cube", { pos = vector.create(math.cos(a) * 6, 0.5, math.sin(a) * 6), size = 1.2, color = "#b07a46", tag = "crate" })
	end
	state.runners = {}
	state.winner = nil
	for _, p in players.list() do
		add_runner(p)
	end
	for _ = 1, ORBS do
		place_orb()
	end
end

-- A bot's input: run towards the nearest orb, jump now and then.
local function bot_input(r)
	local me = world.get(r.body).pos
	local best, dist = nil, math.huge
	for _, orb in world.find("orb") do
		local d = vector.magnitude(world.get(orb).pos - me)
		if d < dist then best, dist = orb, d end
	end
	local input = { move_x = 0, move_y = 0, look_x = 0, look_y = 0, held = {}, pressed = {} }
	if best then
		local to = world.get(best).pos - me
		local want = math.atan2(-to.x, -to.z)
		local turn = (want - r.yaw + math.pi) % (2 * math.pi) - math.pi
		-- A bit slower and clumsier than a person, so people can win.
		input.look_x = -math.clamp(turn, -0.06, 0.06)
		input.move_y = if math.abs(turn) < 0.5 then 0.7 else 0.2
	end
	input.pressed.jump = random.chance(0.01)
	return input
end

local function run(r, p, dt)
	r.yaw -= p.look_x
	local fwd = forward(r.yaw)
	local right = vector.create(-fwd.z, 0, fwd.x)
	local go = (right * p.move_x + fwd * p.move_y) * SPEED
	local vel = vector.create(go.x, r.vel.y - GRAVITY * dt, go.z)
	if p.pressed.jump and r.grounded then
		vel = vector.create(vel.x, JUMP, vel.z)
		sound.play("jump", { at = world.get(r.body).pos })
	end
	r.vel, r.grounded = world.move(r.body, vel, dt)
	world.set(r.body, { rot = vector.create(0, r.yaw, 0) })

	local me = world.get(r.body)
	for _, orb in world.overlap(me.pos, 0.8, { tag = "orb" }) do
		local at = world.get(orb).pos
		world.remove(orb)
		r.score += 1
		sound.play("pickup", { at = at, pitch = 1 + 0.06 * (r.score % 6) })
		players.rumble(r.id, 0.3, 0.1)
		place_orb()
		if r.score >= WIN and not state.winner then
			state.winner = r.name
			game.log(r.name .. " won Hello Warp")
			game.finish()
		end
	end
	if me.pos.y < -20 then
		world.set(r.body, { pos = vector.create(0, 2, 0) })
	end

	-- A camera behind and above (bots have none to look through).
	if not r.bot then
		camera.set(r.id, { pos = me.pos - fwd * 7 + vector.create(0, 3.5, 0), target = me.pos + LOOK, fov = 60 })
	end
end

local function tick(dt, input)
	if state.winner then return end
	for _, r in state.runners do
		run(r, if r.bot then bot_input(r) else input[r.id], dt)
	end
end

local function draw()
	local _, h = hud.size()
	local s = h / 720
	for _, r in state.runners do
		if not r.bot then
			local x, y = hud.view(r.id)
			hud.box(x + 16 * s, y + 14 * s, 260 * s, 56 * s, { 0, 0, 0, 0.35 })
			hud.text(r.name .. "  " .. r.score, x + 30 * s, y + 22 * s, { size = 36 * s })
		end
	end
	if state.winner then
		local w = hud.size()
		hud.text(state.winner .. " wins!", 0, h * 0.42, { size = 64 * s, align = "center", width = w })
	end
end

-- Splitscreen Club seated someone new mid-game, or someone left.
local function player_joined(p)
	add_runner(p)
end

local function player_left(p)
	local r, i = runner(p.id)
	if r then
		world.remove(r.body)
		table.remove(state.runners, i)
	end
end

-- Names follow the lobby live.
local function players_changed()
	for _, p in players.list() do
		local r = runner(p.id)
		if r then r.name = p.name end
	end
end

-- Runs after this file is saved and reloaded (init does not run again).
local function reload()
	print("reloaded with " .. #state.runners .. " runners")
end


-- The sound patches (engine/docs/audio.md), as `sounds/NAME.luau` would
-- hold them: sound.play("pickup") plays SOUNDS.pickup.
local SOUNDS = {
	jump = [==[
-- The jump: a short upward sine sweep, 260 Hz to 620 Hz. 0.18 s.
length(0.18)

local f = expr("lerp(260, 620, 1 - exp(-t * 25))")
return expr("sin(tau * f * t) * exp(-t * 14) * 0.3 * (t < 0.18)", { f = f })
]==],
	pickup = [==[
-- The orb pickup: two quick rising sine notes (880 Hz, then 1320 Hz 70 ms
-- later) with a bright overtone, softly saturated. 0.35 s.
-- sound.play("pickup", { at = pos, pitch = 1.1, bright = 0.5 })
--
-- Params:
--   bright  the overtone's level, 0 to 2 (default 1)
-- Render it without speakers: warp audio render sounds/pickup.luau
length(0.35)

local bright = param("bright", 1)
local low = expr("sin(tau * 880 * t) * exp(-t * 18) * 0.4")
local high = expr("sin(tau * 1320 * (t - 0.07)) * exp(-(t - 0.07) * 14) * 0.4 * step(0.07, t)")
local shine = expr("sin(tau * 2640 * (t - 0.07)) * exp(-(t - 0.07) * 30) * 0.12 * k * step(0.07, t)", { k = bright })
return expr("tanh((a + b + c) * 1.2) * 0.6 * (t < 0.35)", { a = low, b = high, c = shine })
]==],
}

return {
	sounds = SOUNDS,
	init = init,
	tick = tick,
	draw = draw,
	reload = reload,
	player_joined = player_joined,
	player_left = player_left,
	players_changed = players_changed,
}
