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
-- Where the body is drawn: out of sight (under the ground) while hidden.
local function at(b, pos) return if b.hidden then pos - v(0, 40, 0) else pos end

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

-- A torch in the left hand: a short dark body with a lit lens. It goes
-- where the hand goes (the walk's bob and sway), pointed mostly where the
-- body faces, and it settles with a little lag. b.torch_at and b.torch_dir
-- are where its light starts and goes (nil for a box figure).
M.TORCH_BODY = { color = "#2a2624", roughness = 0.5, metal = 0.5 }
M.TORCH_LENS = { color = "#ffe6c0", emissive = 2, roughness = 0.3 }
local function place_torch(b)
	if not b.torch then return end
	local hand = humans.joint(b.human, "hand_l")
	local elbow = humans.joint(b.human, "elbow_l")
	if not (hand and elbow) then return end
	local arm = hand - elbow
	local m = vector.magnitude(arm)
	arm = if m > 1e-4 then arm / m else forward(b.yaw)
	local f = forward(b.yaw)
	local want = f * 2.2 + arm * 0.6 + v(0, -0.35, 0)
	want = want / vector.magnitude(want)
	local d = b.torch_dir or want
	d = d + (want - d) * 0.3
	d = d / vector.magnitude(d)
	b.torch_dir = d
	local rot = v(math.asin(math.clamp(-d.y, -1, 1)), math.atan2(d.x, d.z), 0)
	world.set(b.torch, { pos = hand + d * 0.06, rot = rot })
	world.set(b.lens, { pos = hand + d * 0.165, rot = rot })
	b.torch_at = hand + d * 0.3
end

function M.torch(b, on)
	on = on and b.human ~= nil and not b.hidden
	if on and not b.torch then
		b.torch = world.spawn("cube", { pos = b.pos, size = v(0.045, 0.045, 0.2), color = M.TORCH_BODY.color, roughness = M.TORCH_BODY.roughness, metal = M.TORCH_BODY.metal, solid = false, tag = "held" })
		b.lens = world.spawn("cube", { pos = b.pos, size = v(0.06, 0.06, 0.03), color = M.TORCH_LENS.color, emissive = M.TORCH_LENS.emissive, roughness = M.TORCH_LENS.roughness, solid = false, tag = "held" })
		place_torch(b)
	elseif not on and b.torch then
		world.remove(b.torch)
		world.remove(b.lens)
		b.torch, b.lens, b.torch_at, b.torch_dir = nil, nil, nil, nil
	end
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
	local id = humans.spawn(b.pool, { pos = at(b, b.pos), yaw = b.yaw + math.pi })
	if not id then return false end
	b.human = id
	b.eye = world.spawn("cube", { pos = b.pos, size = v(0.11, 0.025, 0.02), color = "#1a1a1a", emissive = 0, solid = false, tag = "eyes", visible = false })
	humans.set(id, { pose = b.pose, t = b.t, tired = b.tired or 0 })
	if b.glowing then
		world.set(b.eye, M.EYE_GLOW)
		world.set(b.eye, { visible = true })
	end
	place_eyes(b)
	human_weapon(b)
	return true
end

local function new_figure(b)
	b.fig = figure.build(at(b, b.pos), b.yaw, b.look)
	if b.held then figure.hold(b.fig, b.held, b.held_looks) end
	if b.pose == "down" or b.pose == "dead" then figure.lie(b.fig, at(b, b.pos), b.yaw, true) end
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
	M.torch(b, false)
	humans.remove(b.human)
	world.remove(b.eye)
	b.human, b.eye = nil, nil
	human_weapon(b)
	new_figure(b)
end

function M.place(b, pos, yaw)
	b.pos, b.yaw = pos, yaw
	pos = at(b, pos)
	if b.human then
		humans.set(b.human, { pos = pos, yaw = yaw + math.pi })
		place_eyes(b)
		place_weapon(b)
		place_torch(b)
	elseif b.pose == "down" or b.pose == "dead" then
		figure.lie(b.fig, pos, yaw, true)
	else
		figure.place(b.fig, pos, yaw)
	end
end

-- The box figure's arm names, as poses.
local POSE = { swing = "walk", reach = "walk", raise = "raise", swing_r = "attack", attack = "attack", punch = "punch", jab = "jab", aim = "aim", shove = "shove" }

-- speed in m/s (a human measures it itself); arms as in lib/figure.
function M.animate(b, dt, speed, arms, t)
	if b.human then
		local pose = POSE[arms] or "walk"
		if arms == "reach" and b.pool == nil then pose = "reach" end
		b.pose, b.t = pose, t or 0
		humans.set(b.human, { pose = pose, t = b.t })
		place_weapon(b)
		place_torch(b)
	else
		if arms == "attack" or arms == "punch" or arms == "jab" then arms = "swing_r" end
		figure.animate(b.fig, dt, speed, arms, t)
	end
end

-- Out of breath, 0..1: bent over and heaving (humans only).
function M.tired(b, x)
	if not b.human then return end
	local was = b.tired or 0
	if was == x or (x > 0 and math.abs(was - x) < 0.01) then return end
	b.tired = x
	humans.set(b.human, { tired = x })
end

-- Wound up for a heavy blow, -1..1 (humans only).
function M.coil(b, x)
	if not b.human then return end
	local was = b.coil or 0
	if was == x or (x ~= 0 and math.abs(was - x) < 0.01) then return end
	b.coil = x
	humans.set(b.human, { coil = x })
end

-- Struck from `push` (its length the strength): the upper body jolts away
-- and sways back (humans only; a box figure just takes the knock).
function M.hit(b, push)
	if b.human then humans.hit(b.human, push) end
end

-- Gone limp along `push` (m/s): a ragdoll that settles where it falls
-- (humans only; a box figure just falls). Call before M.lie.
function M.ragdoll(b, push)
	if b.human then humans.ragdoll(b.human, push) end
end

-- Lying down (sleeping, downed) or getting up. `fall` plays the fall.
function M.lie(b, pos, yaw, down, fall)
	b.pos, b.yaw = pos, yaw
	b.pose = if down then (if fall then "dead" else "down") else "walk"
	b.t = 0
	pos = at(b, pos)
	if b.human then
		humans.set(b.human, { pos = pos, yaw = yaw + math.pi, pose = b.pose, t = 0 })
		place_eyes(b)
		place_weapon(b)
	else
		figure.lie(b.fig, pos, yaw, down)
	end
end

-- A held pose for someone who waits: "idle", "crouch", "wave", "down"
-- (lying) and the like (a box figure can only lie or stand).
function M.pose(b, pose)
	if b.pose == pose then return end
	b.pose, b.t = pose, 0
	if b.human then
		humans.set(b.human, { pose = pose, t = 0 })
		place_weapon(b)
	else
		figure.lie(b.fig, at(b, b.pos), b.yaw, pose == "down")
	end
end

-- Getting up off the ground, t 0..1 (from lying, onto the knees, upright).
-- A box figure lies until M.lie(.., false).
function M.rise(b, t)
	b.pose, b.t = "rise", t
	if b.human then
		humans.set(b.human, { pose = "rise", t = t })
		place_eyes(b)
	end
end

-- Where a joint of a human is now ("head", "chest", "hips", ...), or nil.
function M.joint(b, name)
	if not b.human then return nil end
	if name == "head" then return humans.head(b.human) end
	return humans.joint(b.human, name)
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

-- Fog of war: nobody sees this one. Hidden bodies are drawn under the
-- ground, so a human keeps its place in the pool.
function M.hide(b, hidden)
	hidden = hidden or nil
	if b.hidden == hidden then return end
	b.hidden = hidden
	if b.pose == "down" or b.pose == "dead" then
		local pose, t = b.pose, b.t
		M.lie(b, b.pos, b.yaw, true, pose == "dead")
		b.pose, b.t = pose, t
		if b.human then humans.set(b.human, { pose = pose, t = t }) end
	else
		M.place(b, b.pos, b.yaw)
	end
end

function M.remove(b)
	M.torch(b, false)
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
