-- A body for a person or a zombie: a Warp Human from one of game.toml's
-- pools when one is free, else a box figure (lib/figure). The game talks
-- to both the same way; `promote` and `demote` swap between them so the
-- humans go to what the camera sees.
--
-- Yaw is the game's (0 faces -z); Warp Humans face +z at 0.

local figure = require("lib/figure")
local v = vector.create

local M = {}
M.EYE_GLOW = figure.EYE_GLOW

local EYES_OFF = { color = "#1a1a1a", emissive = 0 }

local function forward(yaw) return v(-math.sin(yaw), 0, -math.cos(yaw)) end

local function place_eyes(b)
	local head = humans.head(b.human)
	if not head then return end
	local f = forward(b.yaw)
	world.place(b.eye, head + f * 0.1 - v(0, 0.06, 0), b.yaw)
end

-- A held weapon follows the right forearm, out past the hand.
local function place_weapon(b)
	if not b.weapon then return end
	local hand = humans.joint(b.human, "hand_r")
	local elbow = humans.joint(b.human, "elbow_r")
	if not (hand and elbow) then return end
	local d = hand - elbow
	local m = vector.magnitude(d)
	d = if m > 1e-4 then d / m else forward(b.yaw)
	local w = b.held_looks[b.held]
	world.set(b.weapon, { pos = hand + d * (w.len / 2 - 0.02), rot = v(math.asin(math.clamp(-d.y, -1, 1)), math.atan2(d.x, d.z), 0) })
end

local function human_weapon(b)
	if b.weapon then
		world.remove(b.weapon)
		b.weapon = nil
	end
	local w = b.held and b.held_looks and b.held_looks[b.held]
	if not (b.human and w) then return end
	b.weapon = world.spawn("cube", { pos = b.pos, size = v(w.w, w.w, w.len), color = w.color, metal = w.metal or 0, roughness = w.roughness or 0.6, solid = false, tag = "held" })
	place_weapon(b)
end

local function new_human(b)
	local id = humans.spawn(b.pool, { pos = b.pos, yaw = b.yaw + math.pi })
	if not id then return false end
	b.human = id
	b.eye = world.spawn("cube", { pos = b.pos, size = v(0.11, 0.025, 0.02), color = "#1a1a1a", emissive = 0, solid = false, tag = "eyes", visible = false })
	humans.set(id, { pose = b.pose, t = b.t })
	if b.glowing then
		world.set(b.eye, M.EYE_GLOW)
		world.set(b.eye, { visible = true })
	end
	place_eyes(b)
	human_weapon(b)
	return true
end

local function new_figure(b)
	b.fig = figure.build(b.pos, b.yaw, b.look)
	if b.held then figure.hold(b.fig, b.held, b.held_looks) end
	if b.pose == "down" or b.pose == "dead" then figure.lie(b.fig, b.pos, b.yaw, true) end
	if b.glowing then figure.eyes(b.fig, true) end
	if b.tint then world.set(b.fig.torso, b.tint) end
end

-- pool: the game.toml pool to take a human from (nil: always boxes).
function M.build(pos, yaw, look, pool)
	local b = { pos = pos, yaw = yaw, look = look, pool = pool, pose = "walk", t = 0, glowing = false }
	if not (pool and new_human(b)) then new_figure(b) end
	return b
end

function M.is_human(b) return b.human ~= nil end

-- Swap a box for a human (true when it worked).
function M.promote(b)
	if b.human or not b.pool then return false end
	if not new_human(b) then return false end
	figure.remove(b.fig)
	b.fig = nil
	return true
end

-- Swap a human for a box, freeing its place in the pool.
function M.demote(b)
	if not b.human then return end
	humans.remove(b.human)
	world.remove(b.eye)
	b.human, b.eye = nil, nil
	human_weapon(b)
	new_figure(b)
end

function M.place(b, pos, yaw)
	b.pos, b.yaw = pos, yaw
	if b.human then
		humans.set(b.human, { pos = pos, yaw = yaw + math.pi })
		place_eyes(b)
		place_weapon(b)
	elseif b.pose == "down" or b.pose == "dead" then
		figure.lie(b.fig, pos, yaw, true)
	else
		figure.place(b.fig, pos, yaw)
	end
end

-- The box figure's arm names, as poses.
local POSE = { swing = "walk", reach = "walk", raise = "raise", swing_r = "attack", aim = "aim", shove = "shove" }

-- speed in m/s (a human measures it itself); arms as in lib/figure.
function M.animate(b, dt, speed, arms, t)
	if b.human then
		local pose = POSE[arms] or "walk"
		if arms == "reach" and b.pool == nil then pose = "reach" end
		b.pose, b.t = pose, t or 0
		humans.set(b.human, { pose = pose, t = b.t })
		place_weapon(b)
	else
		figure.animate(b.fig, dt, speed, arms, t)
	end
end

-- Lying down (sleeping, downed) or getting up. `fall` plays the fall.
function M.lie(b, pos, yaw, down, fall)
	b.pos, b.yaw = pos, yaw
	b.pose = if down then (if fall then "dead" else "down") else "walk"
	b.t = 0
	if b.human then
		humans.set(b.human, { pos = pos, yaw = yaw + math.pi, pose = b.pose, t = 0 })
		place_eyes(b)
		place_weapon(b)
	else
		figure.lie(b.fig, pos, yaw, down)
	end
end

-- A fall in progress: call each tick while lying.
function M.fall(b, dt)
	if b.pose ~= "dead" then return end
	b.t = math.min(1, b.t + dt / 1.4)
	if b.human then
		humans.set(b.human, { t = b.t })
		place_weapon(b)
	end
	if b.t >= 1 then
		b.pose = "down"
		if b.human then humans.set(b.human, { pose = "down" }) end
	end
end

function M.eyes(b, glow, normal)
	if b.glowing == glow then return end
	b.glowing = glow
	if b.human then
		world.set(b.eye, if glow then M.EYE_GLOW else EYES_OFF)
		world.set(b.eye, { visible = glow })
	else
		figure.eyes(b.fig, glow, normal)
	end
end

function M.hold(b, kind, looks)
	b.held, b.held_looks = kind, looks
	if b.fig then figure.hold(b.fig, kind, looks) else human_weapon(b) end
end

-- Recolour a box figure's torso (a human keeps its own clothes).
function M.tint(b, look)
	if b.fig then world.set(b.fig.torso, look) end
	b.tint = look
end

function M.remove(b)
	if b.human then
		humans.remove(b.human)
		world.remove(b.eye)
		b.human, b.eye = nil, nil
	end
	if b.weapon then
		world.remove(b.weapon)
		b.weapon = nil
	end
	if b.fig then
		figure.remove(b.fig)
		b.fig = nil
	end
end

function M.warm(look)
	figure.warm(look)
	world.warm("cube", EYES_OFF)
end

return M
