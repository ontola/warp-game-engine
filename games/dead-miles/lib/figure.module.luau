-- Low-poly people built from boxes: a root at the feet (yaw turns the
-- whole figure), a torso, a head, eyes that glow when a zombie winds up,
-- and arms and legs hung from pivots at the shoulders and hips so they
-- swing. Survivors and zombies share it; only the colours and the pose
-- differ. Seen from a high camera, the shapes and colours carry who is who.

local v = vector.create

local M = {}

-- Heights in metres: a figure is about 1.75 m tall.
local HIP, SHOULDER = 0.85, 1.38
M.HEIGHT = 1.75

-- Looks that figures switch between at run time, warmed in init so a
-- change never stalls a frame.
M.EYE_GLOW = { color = "#ff3a2a", emissive = 0.9 }
M.CORPSE = { color = "#3a2f2a", roughness = 1 }
M.GHOST = { color = "#cfe6ff", emissive = 0.35 }

local function part(shape, opts, parent)
	opts.solid = false
	return world.spawn(shape, opts, parent)
end

-- look = { shirt, skin, pants, eyes? }
function M.build(pos, yaw, look)
	local f = {}
	f.root = part("cube", { pos = pos, rot = v(0, yaw, 0), size = 0.05, visible = false })
	f.torso = part("cube", { pos = v(0, 1.12, 0), size = v(0.5, 0.56, 0.3), color = look.shirt, roughness = 0.9 }, f.root)
	f.head = part("cube", { pos = v(0, 1.56, 0), size = v(0.3, 0.32, 0.3), color = look.skin, roughness = 0.8 }, f.root)
	-- Eyes sit on the front of the head (the figure faces -Z).
	f.eyes = part("cube", { pos = v(0, 0.02, -0.155), size = v(0.22, 0.05, 0.02), color = look.eyes or "#1a1a1a", emissive = 0 }, f.head)
	f.arm_l = part("cube", { pos = v(-0.33, SHOULDER, 0), size = 0.02, visible = false }, f.root)
	f.arm_r = part("cube", { pos = v(0.33, SHOULDER, 0), size = 0.02, visible = false }, f.root)
	part("cube", { pos = v(0, -0.27, 0), size = v(0.15, 0.58, 0.15), color = look.shirt, roughness = 0.9 }, f.arm_l)
	part("cube", { pos = v(0, -0.27, 0), size = v(0.15, 0.58, 0.15), color = look.shirt, roughness = 0.9 }, f.arm_r)
	f.hand_l = part("cube", { pos = v(0, -0.6, 0), size = 0.12, color = look.skin }, f.arm_l)
	f.hand_r = part("cube", { pos = v(0, -0.6, 0), size = 0.12, color = look.skin }, f.arm_r)
	f.leg_l = part("cube", { pos = v(-0.13, HIP, 0), size = 0.02, visible = false }, f.root)
	f.leg_r = part("cube", { pos = v(0.13, HIP, 0), size = 0.02, visible = false }, f.root)
	part("cube", { pos = v(0, -0.42, 0), size = v(0.19, 0.85, 0.2), color = look.pants, roughness = 0.95 }, f.leg_l)
	part("cube", { pos = v(0, -0.42, 0), size = v(0.19, 0.85, 0.2), color = look.pants, roughness = 0.95 }, f.leg_r)
	f.phase = 0
	f.pose = "stand"
	return f
end

-- A held weapon in the right hand, pointing forward from the fist.
-- kind = nil hides it.
function M.hold(f, kind, looks)
	if f.weapon then
		world.remove(f.weapon)
		f.weapon = nil
	end
	local w = kind and looks[kind]
	if not w then return end
	f.weapon = part("cube", { pos = v(0, -0.02, -w.len / 2 + 0.05), size = v(w.w, w.w, w.len), color = w.color, metal = w.metal or 0, roughness = w.roughness or 0.6 }, f.hand_r)
end

function M.place(f, pos, yaw)
	world.place(f.root, pos, yaw)
end

local function pitch(id, a, roll)
	world.set(id, { rot = v(a, 0, roll or 0) })
end

-- Walk cycle and arm pose. speed in m/s. arms: "swing" (survivor),
-- "reach" (zombie arms forward), "raise" (zombie winding up),
-- "swing_r" (survivor mid melee swing, t 0..1), "aim" (gun forward).
function M.animate(f, dt, speed, arms, t)
	f.phase += dt * speed * 3.2
	local stride = math.min(speed / 2.5, 1) * 0.65
	local s = math.sin(f.phase) * stride
	pitch(f.leg_l, s)
	pitch(f.leg_r, -s)
	if arms == "reach" then
		local sway = math.sin(f.phase * 0.5) * 0.12
		pitch(f.arm_l, 1.35 + sway, -0.1)
		pitch(f.arm_r, 1.35 - sway, 0.1)
	elseif arms == "raise" then
		-- Arms up high and back: the tell before a swipe or a lunge.
		pitch(f.arm_l, 2.6, -0.35)
		pitch(f.arm_r, 2.6, 0.35)
	elseif arms == "swing_r" then
		-- From up and back to down across the body.
		local a = 2.4 - (t or 0) * 2.6
		pitch(f.arm_r, a, 0.25 - (t or 0) * 0.6)
		pitch(f.arm_l, -s * 0.8)
	elseif arms == "aim" then
		pitch(f.arm_r, 1.57)
		pitch(f.arm_l, 1.3, 0.35)
	elseif arms == "shove" then
		pitch(f.arm_l, 1.6, 0.2)
		pitch(f.arm_r, 1.6, -0.2)
	else
		pitch(f.arm_l, -s * 0.8)
		pitch(f.arm_r, s * 0.8)
	end
end

-- Lying on the back (asleep, downed or dead), or back on the feet.
function M.lie(f, pos, yaw, down)
	world.set(f.root, { pos = if down then pos + v(0, 0.15, 0) else pos, rot = v(if down then -1.5 else 0, yaw, 0) })
	f.pose = if down then "down" else "stand"
end

function M.eyes(f, glow, normal)
	if f.glowing == glow then return end
	f.glowing = glow
	if glow then
		world.set(f.eyes, M.EYE_GLOW)
	else
		world.set(f.eyes, normal)
	end
end

function M.remove(f)
	world.remove(f.root)
end

-- Warm every look a figure in these colours uses.
function M.warm(look)
	for _, c in { look.shirt, look.skin, look.pants } do
		world.warm("cube", { color = c, roughness = 0.9 })
		world.warm("cube", { color = c, roughness = 0.8 })
		world.warm("cube", { color = c, roughness = 0.95 })
		world.warm("cube", { color = c })
	end
end

return M
