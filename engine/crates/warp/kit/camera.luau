-- kit/camera: camera rigs that follow things smoothly. One rig per view.
-- `local cam = require("kit/camera")`.
--
--   state.cams[p.id] = cam.rig(p.id)                  -- in init, per person
--   state.cams[p.id]:chase(pos, yaw, dt)              -- in tick: behind and above
--   state.cams[p.id]:side(pos, dt)                    -- platformer side view
--   state.cams[p.id]:top_down(pos, dt)                -- from above, tilted
--   state.cam = cam.rig()   state.cam:frame(points, dt)  -- one shared view
--                                                     -- (game.toml split = false)
-- Move cameras in tick, not draw: sound listeners follow them. The first
-- call of a rig snaps; later ones ease with `smooth` (higher is tighter).

local util = require("kit/util")
local v = vector.create

local cam = {}

local Rig = {}
Rig.__index = Rig

-- A rig for a player's camera; nil player: the first person's (the one
-- shared view of a game with split = false).
function cam.rig(player: number?, opts: { fov: number?, smooth: number? }?)
	opts = opts or {}
	return setmetatable({ player = player, fov = opts.fov or 60, smooth = opts.smooth or 7, pos = nil, target = nil }, Rig)
end

local function first_person(): number?
	for _, p in players.list() do
		if not p.bot then return p.id end
	end
	return nil
end

-- Ease towards a camera position and target, then set the camera.
function Rig:look(pos: vector, target: vector, dt: number, fov: number?)
	if self.pos == nil or dt <= 0 then
		self.pos, self.target = pos, target
	else
		self.pos = util.damp(self.pos, pos, self.smooth, dt)
		self.target = util.damp(self.target, target, self.smooth * 1.5, dt)
	end
	if fov then self.fov = fov end
	local off = v(0, 0, 0)
	if self.shake_until and game.time() < self.shake_until then
		local k = (self.shake_until - game.time()) / 0.25 * self.shake_strength
		local t = game.time() * 60
		off = v(math.sin(t * 1.7) * k, math.sin(t * 2.3 + 1) * k, 0)
	end
	local id = self.player or first_person()
	if id then camera.set(id, { pos = self.pos + off, target = self.target + off, fov = self.fov }) end
end

-- Forget the eased position: the next call snaps (after a respawn).
function Rig:snap() self.pos = nil end

-- Behind and above something facing `yaw`. opts: distance (7), height
-- (3), look_height (1, above focus), pitch (radians, tilts the view).
function Rig:chase(focus: vector, yaw: number, dt: number, opts: { [string]: number }?)
	opts = opts or {}
	local d, h = opts.distance or 7, opts.height or 3
	local pitch = opts.pitch or 0
	local back = util.forward(yaw) * (-d * math.cos(pitch))
	local pos = focus + back + v(0, h - d * math.sin(pitch), 0)
	self:look(pos, focus + v(0, opts.look_height or 1, 0), dt, opts.fov)
end

-- A side view along -Z (a platformer). opts: distance (16), height (3),
-- look_height (1.5), lead (metres ahead in the direction it moves; give
-- the velocity as `vel` for that).
function Rig:side(focus: vector, dt: number, opts: { [string]: any }?)
	opts = opts or {}
	local lead = if opts.vel then math.clamp(opts.vel.x * 0.25, -1, 1) * (opts.lead or 2) else 0
	local f = focus + v(lead, 0, 0)
	self:look(f + v(0, opts.height or 3, opts.distance or 16), f + v(0, opts.look_height or 1.5, 0), dt, opts.fov)
end

-- From above, tilted towards -Z (top-down and twin-stick games). opts:
-- height (16), back (7: how far behind on +Z; 0 looks straight down).
function Rig:top_down(focus: vector, dt: number, opts: { [string]: number }?)
	opts = opts or {}
	local back = opts.back or 7
	self:look(focus + v(0, opts.height or 16, back), focus + v(0, 0, -0.01), dt, opts.fov)
end

-- Fit every point in one view (a shared camera for an arena or party
-- game). opts: pitch (0.95 rad down), yaw (0: looking towards -Z),
-- margin (1.3), min_distance (10), max_distance (80).
function Rig:frame(points: { vector }, dt: number, opts: { [string]: number }?)
	opts = opts or {}
	if #points == 0 then return end
	local c = v(0, 0, 0)
	for _, p in points do c += p end
	c /= #points
	local r = 0
	for _, p in points do r = math.max(r, vector.magnitude(p - c)) end
	local half = math.rad(self.fov) / 2
	local d = math.clamp((r * (opts.margin or 1.3)) / math.tan(half) + 2, opts.min_distance or 10, opts.max_distance or 80)
	local pitch, yaw = opts.pitch or 0.95, opts.yaw or 0
	local back = util.forward(yaw) * (-d * math.cos(pitch))
	self:look(c + back + v(0, d * math.sin(pitch), 0), c, dt, opts.fov)
end

-- Shake for a quarter second (a hit, a landing): strength in metres.
function Rig:shake(strength: number?)
	self.shake_until = game.time() + 0.25
	self.shake_strength = strength or 0.3
end

return cam
