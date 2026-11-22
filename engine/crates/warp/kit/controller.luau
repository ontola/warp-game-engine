-- kit/controller: ready-made movement for a body (an entity from
-- world.spawn), driven by one player's input each tick. Collision is
-- world.move: the body slides along solid boxes as an upright cylinder of
-- its box's width and height. `local controller = require("kit/controller")`.
--
--   state.hero = controller.platformer(body, { speed = 7, jump = 11 })
--   local ev = state.hero:update(input[p.id], dt)   -- in tick()
--   if ev.jumped then sound.play("jump") end
--
-- Every controller keeps its velocity in .vel, whether it stands on
-- something in .grounded and where it faces in .yaw; update returns events
-- for this tick: jumped, landed, fell (dropped under kill_y and respawned)
-- and, where they apply, air_jumped and dashed. Options left out take the
-- defaults below. Bots pass an input from kit/bot.

local util = require("kit/util")
local v = vector.create
local ZERO = v(0, 0, 0)

local controller = {}

local function held(input, b) return input ~= nil and input.held ~= nil and input.held[b] == true end
local function pressed(input, b) return input ~= nil and input.pressed ~= nil and input.pressed[b] == true end
local function axis(input, a) return if input ~= nil and input[a] ~= nil then input[a] else 0 end

-- Move a vector towards target by at most step.
local function approach_vec(cur: vector, target: vector, step: number): vector
	local d = target - cur
	local m = vector.magnitude(d)
	if m <= step or m == 0 then return target end
	return cur + d * (step / m)
end

local function face(c, yaw)
	c.yaw = yaw
	world.set(c.body, { rot = v(0, yaw, 0) })
end

local function respawn_if_fallen(c, ev)
	local e = world.get(c.body)
	if e and e.pos.y < c.kill_y then
		world.set(c.body, { pos = c.spawn })
		c.vel = ZERO
		ev.fell = true
	end
end

local function base(body: number, opts: { [string]: any }?)
	opts = opts or {}
	local e = world.get(body)
	return {
		body = body,
		vel = ZERO,
		grounded = false,
		yaw = if e then e.rot.y else 0,
		spawn = opts.spawn or (if e then e.pos else ZERO),
		kill_y = opts.kill_y or -30,
		query = opts.query, -- a world.move query: { tag, ignore }
	}
end

------------------------------------------------------------------------------
-- Platformer: side view in the X-Y plane. move_x runs, jump (A, Space)
-- jumps; holding it jumps higher. Coyote time and a jump buffer forgive
-- late and early presses. Faces +X or -X.

local Platformer = {}
Platformer.__index = Platformer

function controller.platformer(body: number, opts: { [string]: any }?)
	opts = opts or {}
	local c = base(body, opts)
	c.speed = opts.speed or 7 -- run speed, m/s
	c.accel = opts.accel or 70 -- ground acceleration, m/s^2
	c.air_accel = opts.air_accel or 40
	c.friction = opts.friction or 60 -- slowing down on the ground with no input
	c.jump = opts.jump or 13 -- take-off speed, m/s (about 2.6 m high with the defaults)
	c.gravity = opts.gravity or 32 -- while rising with jump held
	c.fall_gravity = opts.fall_gravity or 55 -- falling, or jump let go: snappy
	c.max_fall = opts.max_fall or 28
	c.coyote = opts.coyote or 0.1 -- seconds after leaving a ledge a jump still works
	c.buffer = opts.buffer or 0.12 -- seconds a press before landing still jumps
	c.air_jumps = opts.air_jumps or 0 -- 1 for a double jump
	c.depth = opts.depth or (c.spawn.z) -- the plane's z; nil keeps any z
	if opts.depth == false then c.depth = nil end
	c.facing = 1
	c.since_ground = math.huge
	c.since_press = math.huge
	c.jumps_left = 0
	return setmetatable(c, Platformer)
end

function Platformer:update(input, dt: number)
	local ev = {}
	local mx = axis(input, "move_x")
	local was = self.grounded
	if pressed(input, "jump") then self.since_press = 0 else self.since_press += dt end
	if was then self.since_ground = 0; self.jumps_left = self.air_jumps else self.since_ground += dt end

	local vx, vy = self.vel.x, self.vel.y
	local moving = math.abs(mx) > 0.05
	local a = if moving then (if was then self.accel else self.air_accel) else (if was then self.friction else self.air_accel * 0.4)
	vx = util.approach(vx, mx * self.speed, a * dt)

	if self.since_press <= self.buffer then
		if self.since_ground <= self.coyote then
			vy = self.jump
			self.since_ground, self.since_press = math.huge, math.huge
			ev.jumped = true
		elseif self.jumps_left > 0 then
			self.jumps_left -= 1
			vy = self.jump * 0.9
			self.since_press = math.huge
			ev.jumped, ev.air_jumped = true, true
		end
	end
	local g = if vy > 0 and held(input, "jump") then self.gravity else self.fall_gravity
	vy = math.max(vy - g * dt, -self.max_fall)

	local out, grounded = world.move(self.body, v(vx, vy, 0), dt, self.query)
	self.vel, self.grounded = out, grounded
	ev.landed = grounded and not was
	if self.depth then
		local e = world.get(self.body)
		if e and math.abs(e.pos.z - self.depth) > 1e-4 then world.place(self.body, v(e.pos.x, e.pos.y, self.depth)) end
	end
	if mx > 0.1 then self.facing = 1 elseif mx < -0.1 then self.facing = -1 end
	face(self, if self.facing > 0 then -math.pi / 2 else math.pi / 2)
	respawn_if_fallen(self, ev)
	return ev
end

-- Bounce up (a stomp, a spring): sets the vertical speed.
function Platformer:bounce(speed: number?)
	self.vel = v(self.vel.x, speed or self.jump, 0)
	self.grounded = false
end

------------------------------------------------------------------------------
-- Walker: top-down, twin-stick, arena and party games. The stick moves
-- on the ground relative to a view yaw (0: up on the stick is -Z, away from
-- a camera behind +Z). Faces where it walks, or where the right stick aims
-- with face = "aim". Optional jump and dash (east: B, the second button).

local Walker = {}
Walker.__index = Walker

function controller.walker(body: number, opts: { [string]: any }?)
	opts = opts or {}
	local c = base(body, opts)
	c.speed = opts.speed or 6
	c.accel = opts.accel or 45
	c.friction = opts.friction or 40
	c.turn = opts.turn or 14 -- radians per second towards where it walks
	c.jump = opts.jump or 0 -- 0: no jumping
	c.gravity = opts.gravity or 28
	c.face = opts.face or "move" -- or "aim": the right stick decides (twin-stick)
	c.view_yaw = opts.view_yaw or 0
	c.dash = opts.dash or 0 -- dash speed in m/s; 0: no dash
	c.dash_time = opts.dash_time or 0.15
	c.dash_cooldown = opts.dash_cooldown or 0.7
	c.dash_left = 0
	c.dash_ready_at = 0
	c.dash_dir = ZERO
	return setmetatable(c, Walker)
end

-- The world direction the stick points, relative to view_yaw (length 0..1).
function Walker:stick(input): vector
	local d = util.right(self.view_yaw) * axis(input, "move_x") + util.forward(self.view_yaw) * axis(input, "move_y")
	return util.clamp_length(d, 1)
end

function Walker:update(input, dt: number, view_yaw: number?)
	if view_yaw then self.view_yaw = view_yaw end
	local ev = {}
	local was = self.grounded
	local dir = self:stick(input)
	local h = util.flat(self.vel)
	local vy = self.vel.y

	if self.dash > 0 and pressed(input, "east") and game.time() >= self.dash_ready_at then
		local d = if vector.magnitude(dir) > 0.1 then vector.normalize(dir) else util.forward(self.yaw)
		self.dash_dir, self.dash_left = d, self.dash_time
		self.dash_ready_at = game.time() + self.dash_cooldown
		ev.dashed = true
	end
	if self.dash_left > 0 then
		self.dash_left -= dt
		h = self.dash_dir * self.dash
	else
		local a = if vector.magnitude(dir) > 0.05 then self.accel else self.friction
		h = approach_vec(h, dir * self.speed, a * dt)
	end
	if self.jump > 0 and was and pressed(input, "jump") then vy = self.jump; ev.jumped = true end
	vy -= self.gravity * dt

	local out, grounded = world.move(self.body, v(h.x, vy, h.z), dt, self.query)
	self.vel, self.grounded = out, grounded
	ev.landed = grounded and not was

	local aim = util.right(self.view_yaw) * axis(input, "aim_x") + util.forward(self.view_yaw) * axis(input, "aim_y")
	if self.face == "aim" and vector.magnitude(aim) > 0.3 then
		face(self, util.yaw_to(aim))
	elseif vector.magnitude(dir) > 0.1 then
		face(self, util.turn_towards(self.yaw, util.yaw_to(dir), self.turn * dt))
	end
	respawn_if_fallen(self, ev)
	return ev
end

-- A shove from outside (a hit, an explosion): adds to the velocity.
function Walker:push(impulse: vector)
	self.vel += impulse
end

------------------------------------------------------------------------------
-- Third person: look_x (mouse, right stick) turns, the stick walks
-- relative to where it faces, jump jumps. .pitch follows look_y for a
-- camera (kit/camera's chase).

local Third = {}
Third.__index = Third

function controller.third_person(body: number, opts: { [string]: any }?)
	opts = opts or {}
	local c = base(body, opts)
	c.speed = opts.speed or 6
	c.sprint = opts.sprint or 1.6 -- speed factor while left_shoulder is held
	c.accel = opts.accel or 50
	c.jump = opts.jump or 7.5
	c.gravity = opts.gravity or 22
	c.pitch = 0
	return setmetatable(c, Third)
end

function Third:update(input, dt: number)
	local ev = {}
	local was = self.grounded
	self.yaw -= axis(input, "look_x")
	self.pitch = math.clamp(self.pitch + axis(input, "look_y"), -(self.max_pitch or 1.2), self.max_pitch or 0.9)
	local speed = self.speed * (if held(input, "left_shoulder") then self.sprint else 1)
	local dir = util.clamp_length(util.right(self.yaw) * axis(input, "move_x") + util.forward(self.yaw) * axis(input, "move_y"), 1)
	local h = approach_vec(util.flat(self.vel), dir * speed, self.accel * dt)
	local vy = self.vel.y
	if was and pressed(input, "jump") then vy = self.jump; ev.jumped = true end
	vy -= self.gravity * dt
	local out, grounded = world.move(self.body, v(h.x, vy, h.z), dt, self.query)
	self.vel, self.grounded = out, grounded
	ev.landed = grounded and not was
	face(self, self.yaw)
	respawn_if_fallen(self, ev)
	return ev
end

-- First person: the same body as third person, tuned for a shooter
-- (quicker, a lower jump, a wider look range). Use with kit/camera's
-- first_person and kit/weapon; ctl:eye() and ctl:aim() give the view.
function controller.first_person(body: number, opts: { [string]: any }?)
	opts = opts or {}
	local c = controller.third_person(body, {
		speed = opts.speed or 7, sprint = opts.sprint or 1.35, accel = opts.accel or 70,
		jump = opts.jump or 6.5, gravity = opts.gravity or 20,
		spawn = opts.spawn, kill_y = opts.kill_y, query = opts.query,
	})
	c.eye_height = opts.eye_height or 0.7 -- above the body's centre
	c.max_pitch = 1.45
	return c
end

-- Where the eyes are and where they look (first and third person).
function Third:eye(): vector
	local e = world.get(self.body)
	return (if e then e.pos else ZERO) + v(0, self.eye_height or 0.7, 0)
end

function Third:aim(): vector
	return util.forward(self.yaw) * math.cos(self.pitch) + v(0, math.sin(self.pitch), 0)
end

------------------------------------------------------------------------------
-- Kart: arcade driving. Throttle is the stick forward or the right
-- trigger, brake and reverse the stick back or the left trigger, steering
-- the stick sideways. grip below about 4 drifts.

local Kart = {}
Kart.__index = Kart

function controller.kart(body: number, opts: { [string]: any }?)
	opts = opts or {}
	local c = base(body, opts)
	c.max_speed = opts.max_speed or 24
	c.accel = opts.accel or 16
	c.brake = opts.brake or 35
	c.reverse = opts.reverse or 7 -- top speed backwards
	c.drag = opts.drag or 4 -- slowing with no throttle, m/s^2
	c.turn = opts.turn or 2.4 -- radians per second at full lock and speed
	c.grip = opts.grip or 8 -- how fast the velocity follows the nose
	c.gravity = opts.gravity or 25
	c.speed = 0 -- along the nose, m/s
	c.boost_left = 0
	c.boost_speed = 0
	return setmetatable(c, Kart)
end

function Kart:update(input, dt: number)
	local ev = {}
	local was = self.grounded
	local throttle = math.max(axis(input, "move_y"), if held(input, "fire") then 1 else 0)
	local brake = math.max(-axis(input, "move_y"), if held(input, "aim") then 1 else 0)
	local top = self.max_speed
	if self.boost_left > 0 then
		self.boost_left -= dt
		top += self.boost_speed
		throttle = 1
	end
	if throttle > 0.05 then
		local cap = top * throttle
		if self.speed < cap then
			self.speed = math.min(self.speed + self.accel * dt * (if self.boost_left > 0 then 3 else 1), cap)
		else
			self.speed = util.approach(self.speed, cap, self.drag * dt)
		end
	elseif brake > 0.05 then
		if self.speed > 0 then self.speed = math.max(self.speed - self.brake * brake * dt, 0)
		else self.speed = math.max(self.speed - self.accel * 0.5 * brake * dt, -self.reverse) end
	else
		self.speed = util.approach(self.speed, 0, self.drag * dt)
	end
	if self.speed > top then self.speed = util.approach(self.speed, top, self.brake * dt) end

	local steer = axis(input, "move_x")
	local grip_k = math.clamp(math.abs(self.speed) / 6, 0, 1)
	self.yaw -= steer * self.turn * grip_k * dt * (if self.speed < 0 then -1 else 1)

	local fwd = util.forward(self.yaw)
	local h = util.damp(util.flat(self.vel), fwd * self.speed, self.grip, dt)
	local vy = self.vel.y - self.gravity * dt
	local out, grounded = world.move(self.body, v(h.x, vy, h.z), dt, self.query)
	self.vel, self.grounded = out, grounded
	-- A wall takes the speed it stopped.
	local along = vector.dot(util.flat(out), fwd)
	if math.abs(along) < math.abs(vector.dot(h, fwd)) - 0.5 then
		if math.abs(self.speed) > 8 then ev.bumped = true end
		self.speed = along
	end
	ev.landed = grounded and not was
	face(self, self.yaw)
	respawn_if_fallen(self, ev)
	return ev
end

-- Extra top speed for `seconds` (a boost pad, a mushroom).
function Kart:boost(seconds: number, extra: number?)
	self.boost_left = seconds
	self.boost_speed = extra or 10
end

-- How fast it drives now, 0..1 of max_speed (for engine sound and the HUD).
function Kart:speed01(): number return math.clamp(math.abs(self.speed) / self.max_speed, 0, 1) end

return controller
