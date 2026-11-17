-- A stray dog in boxes: body, head, snout, ears, tail and four legs that
-- trot. Faces +x at yaw 0. place() moves it; the legs swing with speed.

local v = vector.create

local M = {}

M.COAT = "#6a4a30"
M.DARK = "#3a2a1e"

local function part(shape, o, parent)
	o.solid = false
	o.tag = "deco"
	o.roughness = o.roughness or 0.95
	return world.spawn(shape, o, parent)
end

function M.warm()
	for _, c in { M.COAT, M.DARK, "#1a1a1a" } do world.warm("cube", { color = c, roughness = 0.95 }) end
end

function M.build(pos, yaw)
	local d = { pos = pos, yaw = yaw or 0, phase = 0, legs = {} }
	d.root = world.spawn("cube", { pos = pos, rot = v(0, d.yaw, 0), size = 0.05, visible = false, solid = false })
	part("cube", { pos = v(0, 0.48, 0), size = v(0.75, 0.3, 0.28), color = M.COAT }, d.root)
	d.head = part("cube", { pos = v(0.45, 0.66, 0), size = v(0.26, 0.24, 0.22), color = M.COAT }, d.root)
	part("cube", { pos = v(0.17, -0.04, 0), size = v(0.14, 0.11, 0.13), color = M.DARK }, d.head)
	part("cube", { pos = v(0.2, -0.02, 0), size = v(0.03, 0.04, 0.05), color = "#1a1a1a" }, d.head)
	part("cube", { pos = v(-0.04, 0.15, 0.07), size = v(0.06, 0.1, 0.05), color = M.DARK }, d.head)
	part("cube", { pos = v(-0.04, 0.15, -0.07), size = v(0.06, 0.1, 0.05), color = M.DARK }, d.head)
	d.tail = part("cube", { pos = v(-0.42, 0.6, 0), size = v(0.25, 0.06, 0.06), color = M.COAT, rot = v(0, 0, 0.6) }, d.root)
	for _, at in { v(0.28, 0.34, 0.1), v(0.28, 0.34, -0.1), v(-0.28, 0.34, 0.1), v(-0.28, 0.34, -0.1) } do
		local hip = world.spawn("cube", { pos = at, size = 0.02, visible = false, solid = false }, d.root)
		part("cube", { pos = v(0, -0.17, 0), size = v(0.08, 0.34, 0.08), color = M.COAT }, hip)
		table.insert(d.legs, hip)
	end
	return d
end

-- speed in m/s; bark lifts the head for a moment.
function M.place(d, pos, yaw, speed, dt, bark)
	d.pos, d.yaw = pos, yaw
	world.set(d.root, { pos = pos, rot = v(0, yaw, 0) })
	d.phase += dt * speed * 3.2
	local swing = math.sin(d.phase) * math.min(0.6, speed * 0.25)
	for i, hip in d.legs do
		local s = if i == 1 or i == 4 then swing else -swing
		world.set(hip, { rot = v(0, 0, s) })
	end
	world.set(d.tail, { rot = v(math.sin(d.phase * 2 + (bark or 0) * 9) * 0.5, 0, 0.6) })
	world.set(d.head, { rot = v(0, 0, (bark or 0) * 0.35) })
end

function M.remove(d)
	world.remove(d.root)
end

return M
