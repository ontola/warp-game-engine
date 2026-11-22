-- kit/bot: inputs for bots, shaped like a person's, so the same
-- controller drives both. `local bot = require("kit/bot")`.
--
--   local inp = if p.bot then bot.walk_to(me, target) else input[p.id]
--   state.ctl[p.id]:update(inp, dt)

local util = require("kit/util")

local bot = {}

local BUTTONS = { "south", "east", "west", "north", "left_shoulder", "right_shoulder", "left_trigger", "right_trigger", "select", "start", "up", "down", "left", "right", "jump", "fire", "aim" }

-- A neutral input: no stick, no buttons.
function bot.input()
	local held, pressed = {}, {}
	for _, b in BUTTONS do held[b] = false; pressed[b] = false end
	return { move_x = 0, move_y = 0, look_x = 0, look_y = 0, aim_x = 0, aim_y = 0, held = held, pressed = pressed }
end

-- Press a button this tick (pressed and held).
function bot.press(inp, button: string)
	inp.pressed[button] = true
	inp.held[button] = true
	if button == "jump" or button == "south" then inp.pressed.south, inp.pressed.jump, inp.held.south, inp.held.jump = true, true, true, true end
	if button == "fire" or button == "right_trigger" then inp.pressed.fire, inp.pressed.right_trigger, inp.held.fire, inp.held.right_trigger = true, true, true, true end
	return inp
end

-- Walk (kit/controller walker) from `from` towards `to` on the ground,
-- relative to view_yaw (0 default). opts: slow (stop within this many
-- metres, 0.5), skill (0..1, 0.8: how straight and fast it walks).
function bot.walk_to(from: vector, to: vector, opts: { view_yaw: number?, slow: number?, skill: number? }?)
	opts = opts or {}
	local inp = bot.input()
	local d = util.flat(to - from)
	local dist = vector.magnitude(d)
	if dist < (opts.slow or 0.5) then return inp end
	local dir = d / dist
	local yaw = opts.view_yaw or 0
	local skill = opts.skill or 0.8
	inp.move_x = vector.dot(dir, util.right(yaw)) * skill
	inp.move_y = vector.dot(dir, util.forward(yaw)) * skill
	return inp
end

-- Aim the right stick at a point (twin-stick), relative to view_yaw.
function bot.aim_at(inp, from: vector, to: vector, view_yaw: number?)
	local d = util.flat(to - from)
	if vector.magnitude(d) > 0 then
		d = vector.normalize(d)
		inp.aim_x = vector.dot(d, util.right(view_yaw or 0))
		inp.aim_y = vector.dot(d, util.forward(view_yaw or 0))
	end
	return inp
end

-- Detours under ledges, per body: { x, until_time }.
local detours = {}
-- Until when a bot holds jump, per body.
local holds = {}
-- Where and since when a bot has not got anywhere, per body.
local stuck = {}

-- Run along X towards `to` in a side view (kit/controller platformer):
-- jumps when the target is higher or a wall is ahead; under a ledge it
-- first runs to the nearest opening above and keeps to that choice for a
-- moment. `body` is its entity; pass its controller as opts.ctl so it
-- double-jumps when falling short.
function bot.run_to(body: number, to: vector, opts: { skill: number?, ctl: any? }?)
	opts = opts or {}
	local inp = bot.input()
	local e = world.get(body)
	if not e then return inp end
	local up = vector.create(0, 1, 0)
	local q = { ignore = body }
	local goal = to.x
	local higher = to.y > e.pos.y + 1
	-- A ledge counts as in the way only below the target's height.
	local reach = math.clamp(to.y - e.pos.y + 0.3, 0.5, 3)
	local ceiling = higher and world.raycast(e.pos, up, reach, q) ~= nil
	local d = detours[body]
	if d and (game.time() > d.until_time or math.abs(d.x - e.pos.x) < 0.3 or not higher) then
		detours[body], d = nil, nil
	end
	if not d and ceiling then
		local toward = if to.x < e.pos.x then -1 else 1
		for _, off in { 1.5, -1.5, 3, -3, 4.5, -4.5, 6, -6, 8, -8, 10, -10 } do
			local x = e.pos.x + off * toward
			if not world.raycast(vector.create(x, e.pos.y, e.pos.z), up, reach, q) then
				d = { x = x + 0.6 * math.sign(off * toward), until_time = game.time() + 2.5 }
				detours[body] = d
				break
			end
		end
	end
	-- Stuck (no real progress for 3 s): wander off to one side for a bit.
	local t0 = game.time()
	local seen = stuck[body]
	if not seen or math.abs(e.pos.x - seen.x) > 1 or math.abs(e.pos.y - seen.y) > 1 then
		stuck[body] = { x = e.pos.x, y = e.pos.y, since = t0 }
	elseif t0 - seen.since > 3 then
		d = { x = e.pos.x + (if random.chance(0.5) then 5 else -5), until_time = t0 + 1.5 }
		detours[body] = d
		stuck[body] = { x = e.pos.x, y = e.pos.y, since = t0 }
	end
	if d then goal = d.x end
	local dx = goal - e.pos.x
	if math.abs(dx) > 0.3 then inp.move_x = math.sign(dx) * (opts.skill or 0.85) end
	local ahead = math.abs(dx) > 0.3 and world.raycast(e.pos, vector.create(math.sign(dx), 0, 0), 1, q) ~= nil
	local falling = opts.ctl ~= nil and not opts.ctl.grounded and opts.ctl.vel.y < -1
	local want = (higher and not ceiling and math.abs(to.x - e.pos.x) < 5) or (higher and falling) or ahead or random.chance(0.004)
	-- Like a person: press once, hold it for a full-height jump, let go.
	local t = game.time()
	local hold = holds[body]
	if hold and t < hold then
		inp.held.jump, inp.held.south = true, true
	elseif want and (hold == nil or t > hold + 0.08) then
		bot.press(inp, "jump")
		holds[body] = t + 0.35
	end
	return inp
end

-- A first-person shooter bot (kit/controller first_person): turns and
-- tilts towards `target` at a human-ish rate, walks to keep `range`
-- metres away while strafing, and holds fire once lined up with a clear
-- line of sight for a reaction time. opts: skill (0..1, 0.6: turn rate,
-- aim wobble and reaction), range (12),
-- ignore (ids the sight line passes through, such as its own body).
function bot.shoot_at(ctl, target: vector, opts: { skill: number?, range: number?, ignore: any? }?)
	opts = opts or {}
	local inp = bot.input()
	local skill = opts.skill or 0.6
	local eye = ctl:eye()
	-- Aim wobbles around the target, more for a weaker bot.
	local t = game.time()
	local k = ctl.body or 0
	local wobble = (1 - skill) * 0.9
	target += vector.create(math.sin(t * 1.3 + k) * wobble, math.sin(t * 0.9 + k * 2) * wobble * 0.5, math.cos(t * 1.1 + k) * wobble)
	local d = target - eye
	local flat = vector.magnitude(util.flat(d))
	local want_yaw = util.yaw_to(d)
	local want_pitch = math.atan2(d.y, math.max(flat, 0.01))
	local turn = 0.03 + 0.09 * skill
	local dy = util.angle_diff(want_yaw, ctl.yaw)
	inp.look_x = -math.clamp(dy, -turn, turn)
	inp.look_y = math.clamp(want_pitch - ctl.pitch, -turn, turn)
	local range = opts.range or 12
	inp.move_y = if flat > range + 2 then 1 elseif flat < range - 4 then -0.6 else 0
	-- Strafe one way, then the other, every couple of seconds.
	inp.move_x = if math.floor(game.time() / 1.7) % 2 == 0 then 0.6 else -0.6
	local lined_up = math.abs(dy) < 0.06 + (1 - skill) * 0.08 and math.abs(want_pitch - ctl.pitch) < 0.1
	local clear = lined_up and world.raycast(eye, vector.normalize(d), vector.magnitude(d) - 0.6, { ignore = opts.ignore }) == nil
	-- A reaction time before the first shot at a target in sight.
	if not clear then ctl.bot_sighted = nil elseif not ctl.bot_sighted then ctl.bot_sighted = t end
	if clear and t - ctl.bot_sighted >= 0.3 + (1 - skill) * 0.6 then
		do
			inp.held.fire, inp.held.right_trigger = true, true
			inp.pressed.fire, inp.pressed.right_trigger = true, true
		end
	end
	return inp
end

-- Drive (kit/controller kart) towards a point: steer and throttle.
-- `yaw` is the kart's yaw.
function bot.drive_to(from: vector, yaw: number, to: vector, opts: { skill: number? }?)
	opts = opts or {}
	local inp = bot.input()
	local want = util.yaw_to(to - from)
	local turn = util.angle_diff(want, yaw)
	inp.move_x = math.clamp(-turn * 2, -1, 1)
	inp.move_y = (opts.skill or 0.85) * (if math.abs(turn) > 1.2 then 0.4 else 1)
	return inp
end

return bot
