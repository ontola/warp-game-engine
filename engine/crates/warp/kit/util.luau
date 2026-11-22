-- kit/util: small helpers every game needs. `local util = require("kit/util")`.
-- Yaw 0 faces -Z, +Y is up (the warp convention): forward(yaw) and right(yaw)
-- are the flat unit vectors a character at that yaw looks and steps along.

local util = {}
local v = vector.create

util.ZERO = v(0, 0, 0)
util.UP = v(0, 1, 0)

-- The flat direction a yaw faces, and the one to its right.
function util.forward(yaw: number): vector
	return v(-math.sin(yaw), 0, -math.cos(yaw))
end

function util.right(yaw: number): vector
	return v(math.cos(yaw), 0, -math.sin(yaw))
end

-- The yaw that faces along dir (its y is ignored); 0 for a zero vector.
function util.yaw_to(dir: vector): number
	if dir.x == 0 and dir.z == 0 then return 0 end
	return math.atan2(-dir.x, -dir.z)
end

-- a - b wrapped into -pi .. pi: how far to turn from b to reach a.
function util.angle_diff(a: number, b: number): number
	return (a - b + math.pi) % (2 * math.pi) - math.pi
end

-- Turn yaw towards target by at most step radians.
function util.turn_towards(yaw: number, target: number, step: number): number
	local d = util.angle_diff(target, yaw)
	return yaw + math.clamp(d, -step, step)
end

-- v with y = 0, and v with its y replaced.
function util.flat(p: vector): vector return v(p.x, 0, p.z) end
function util.with_y(p: vector, y: number): vector return v(p.x, y, p.z) end

function util.lerp(a: any, b: any, t: number): any return a + (b - a) * t end

-- Move a number towards target by at most step.
function util.approach(x: number, target: number, step: number): number
	if x < target then return math.min(x + step, target) end
	return math.max(x - step, target)
end

-- Frame-rate independent smoothing: rate is how fast (about 1/seconds) a
-- number or vector closes the gap. damp(cam, goal, 8, dt) each tick.
function util.damp(x: any, target: any, rate: number, dt: number): any
	return x + (target - x) * (1 - math.exp(-rate * dt))
end

-- Map x from a..b to c..d, clamped.
function util.remap(x: number, a: number, b: number, c: number, d: number): number
	local t = math.clamp((x - a) / (b - a), 0, 1)
	return c + (d - c) * t
end

function util.distance(a: vector, b: vector): number return vector.magnitude(b - a) end
function util.flat_distance(a: vector, b: vector): number return vector.magnitude(util.flat(b - a)) end

-- Length-limited vector: v scaled down to at most max long.
function util.clamp_length(p: vector, max: number): vector
	local m = vector.magnitude(p)
	if m > max and m > 0 then return p * (max / m) end
	return p
end

-- The entity with `tag` nearest to pos (optionally skipping one id):
-- returns id, distance, or nil.
function util.nearest(pos: vector, tag: string, skip: number?): (number?, number)
	local best, dist = nil, math.huge
	for _, id in world.find(tag) do
		if id ~= skip then
			local e = world.get(id)
			if e then
				local d = vector.magnitude(e.world_pos - pos)
				if d < dist then best, dist = id, d end
			end
		end
	end
	return best, dist
end

-- A random point on a flat disc around centre (radius r, y kept).
function util.random_in_circle(centre: vector, r: number): vector
	local a = random.range(0, 2 * math.pi)
	local d = r * math.sqrt(random.float())
	return centre + v(math.cos(a) * d, 0, math.sin(a) * d)
end

-- Shuffle a list in place with the game's seeded random.
function util.shuffle(list: { any }): { any }
	for i = #list, 2, -1 do
		local j = random.int(1, i)
		list[i], list[j] = list[j], list[i]
	end
	return list
end

-- Pick one item of a list (seeded).
function util.pick(list: { any }): any
	return list[random.int(1, #list)]
end

-- A player's colour as {r, g, b, a} with the given alpha (for hud calls).
function util.rgba(c: any, a: number): { number }
	if typeof(c) == "vector" then return { c.x, c.y, c.z, a } end
	return { c[1] or c.r or 1, c[2] or c.g or 1, c[3] or c.b or 1, a }
end

return util
