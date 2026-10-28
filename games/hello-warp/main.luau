-- Hello Warp: a capsule that runs and jumps on a floor, collects glowing
-- orbs and shows the score. Run it with `warp run .` and edit this file
-- while it plays: saving reloads it, and the world and `state` stay.
-- The API is in types.d.luau (and engine/docs/warp-cli.md).

local SPEED = 6 -- metres per second
local JUMP = 7.5 -- take-off speed, m/s
local GRAVITY = 20 -- m/s^2
local ORBS = 6
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

local function init()
	world.spawn("cube", { pos = vector.create(0, -0.5, 0), size = vector.create(120, 1, 120), color = "#6f9a5a", tag = "floor" })
	for i = 1, 3 do
		local a = i * 2.1
		world.spawn("cube", { pos = vector.create(math.cos(a) * 6, 0.5, math.sin(a) * 6), size = 1.2, color = "#b07a46", tag = "crate" })
	end
	-- A capsule 0.8 m wide and 1.8 m tall (its size is half its height).
	state.player = world.spawn("capsule", { pos = vector.create(0, 1, 0), size = vector.create(0.8, 0.9, 0.8), color = "#e8553f", tag = "player" })
	state.vel = vector.create(0, 0, 0)
	state.yaw = 0
	state.grounded = false
	state.score = 0
	for _ = 1, ORBS do
		place_orb()
	end
end

local function tick(dt, input)
	local p = input[1]
	state.yaw -= p.look_x
	local fwd = forward(state.yaw)
	local right = vector.create(-fwd.z, 0, fwd.x)
	local run = (right * p.move_x + fwd * p.move_y) * SPEED
	local vel = vector.create(run.x, state.vel.y - GRAVITY * dt, run.z)
	if p.pressed.jump and state.grounded then
		vel = vector.create(vel.x, JUMP, vel.z)
		sound.play("jump")
	end
	state.vel, state.grounded = world.move(state.player, vel, dt)
	world.set(state.player, { rot = vector.create(0, state.yaw, 0) })

	local me = world.get(state.player)
	for _, orb in world.overlap(me.pos, 0.8, { tag = "orb" }) do
		local at = world.get(orb).pos
		world.remove(orb)
		state.score += 1
		sound.play("pickup", { at = at, pitch = 1 + 0.06 * (state.score % 6) })
		place_orb()
	end
	if me.pos.y < -20 then
		world.set(state.player, { pos = vector.create(0, 2, 0) })
	end

	-- A camera behind and above the player.
	camera.set(1, { pos = me.pos - fwd * 7 + vector.create(0, 3.5, 0), target = me.pos + LOOK, fov = 60 })
end

local function draw()
	local _, h = hud.size()
	local s = h / 720
	hud.box(16 * s, 14 * s, 220 * s, 56 * s, { 0, 0, 0, 0.35 })
	hud.text("SCORE " .. state.score, 30 * s, 22 * s, { size = 36 * s })
	hud.text("WASD move  mouse look  Space jump", 30 * s, h - 40 * s, { size = 18 * s, color = { 1, 1, 1, 0.75 } })
end

-- Runs after this file is saved and reloaded (init does not run again).
local function reload()
	print("main.luau reloaded at score " .. state.score)
end

return { init = init, tick = tick, draw = draw, reload = reload }
