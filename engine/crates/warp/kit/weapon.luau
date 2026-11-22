-- kit/weapon: hitscan guns (rifles, pistols, shotguns) with fire rate,
-- spread, ammo and reload, plus health and tracers. On game time and the
-- seeded random, so replays match. `local weapon = require("kit/weapon")`.
--
--   state.gun = weapon.new({ damage = 25, rate = 8, ammo = 30 })
--   local shot = state.gun:fire(input, ctl:eye(), ctl:aim(), { ignore = body })
--   if shot then
--       for _, hit in shot.hits do ... hit.id, hit.point, hit.damage ... end
--   end
--   state.gun:tick(dt); weapon.update()  -- once per tick: reload, kick, tracers
--
--   state.hp = weapon.health(100)        -- :hurt(n) -> true when it dies

local util = require("kit/util")
local v = vector.create

local weapon = {}

local Gun = {}
Gun.__index = Gun

-- opts: damage (25 per pellet), rate (shots per second, 8), auto (true:
-- holding fire keeps shooting; false: one shot per press), spread (radians,
-- 0.01), pellets (1; 8 for a shotgun), range (120 m), ammo (nil: endless),
-- reload (seconds, 1.4), button ("fire": right trigger, left mouse),
-- tracer (a colour, or false for none).
function weapon.new(opts: { [string]: any }?)
	opts = opts or {}
	return setmetatable({
		damage = opts.damage or 25,
		rate = opts.rate or 8,
		auto = if opts.auto == nil then true else opts.auto,
		spread = opts.spread or 0.01,
		pellets = opts.pellets or 1,
		range = opts.range or 120,
		ammo = opts.ammo,
		clip = opts.ammo,
		reload_time = opts.reload or 1.4,
		button = opts.button or "fire",
		tracer = if opts.tracer == nil then "#ffe08a" else opts.tracer,
		ready_at = 0,
		reload_until = 0,
		kick = 0, -- 0..1 after a shot, for a crosshair or view kick
	}, Gun)
end

function Gun:reloading(): boolean return game.time() < self.reload_until end

-- Start reloading (also automatic when the clip is empty).
function Gun:reload()
	if self.ammo == nil or self.clip == self.ammo or self:reloading() then return end
	self.reload_until = game.time() + self.reload_time
end

local function jitter(dir: vector, spread: number): vector
	if spread <= 0 then return dir end
	local side = vector.normalize(vector.cross(dir, if math.abs(dir.y) > 0.95 then v(1, 0, 0) else v(0, 1, 0)))
	local up = vector.cross(side, dir)
	local a = random.range(0, 2 * math.pi)
	local r = spread * math.sqrt(random.float())
	return vector.normalize(dir + side * (math.cos(a) * r) + up * (math.sin(a) * r))
end

-- Pull the trigger if input asks for it (held or pressed, per `auto`).
-- Returns nil when it did not shoot, else { from, dir, hits = { {id,
-- point, normal, distance, damage} }, ends = { points } }. query: a
-- world.raycast query; pass { ignore = shooter_body } so it misses itself.
function Gun:fire(input, from: vector, dir: vector, query: { [string]: any }?)
	local t = game.time()
	if self:reloading() then return nil end
	if self.reload_until > 0 and self.ammo then self.clip = self.ammo; self.reload_until = 0 end -- (tick does this too)
	local want = input ~= nil and ((self.auto and input.held and input.held[self.button]) or (input.pressed and input.pressed[self.button]))
	if input ~= nil and input.pressed and input.pressed.west then self:reload() return nil end
	if not want or t < self.ready_at - 1e-9 then return nil end
	if self.ammo and self.clip <= 0 then self:reload() return nil end
	self.ready_at = t + 1 / self.rate
	if self.ammo then self.clip -= 1 end
	self.kick = 1
	local shot = { from = from, dir = dir, hits = {}, ends = {} }
	for _ = 1, self.pellets do
		local d = jitter(vector.normalize(dir), self.spread)
		local hit = world.raycast(from, d, self.range, query)
		local stop = if hit then hit.point else from + d * self.range
		table.insert(shot.ends, stop)
		if hit then
			hit.damage = self.damage
			table.insert(shot.hits, hit)
		end
		if self.tracer then weapon.tracer(from + d * 0.6, stop, self.tracer) end
	end
	if self.ammo and self.clip <= 0 then self:reload() end
	return shot
end

-- Call once per tick: the kick settles.
function Gun:tick(dt: number)
	self.kick = math.max(0, self.kick - dt * 8)
	if self.reload_until > 0 and not self:reloading() and self.ammo then self.clip = self.ammo; self.reload_until = 0 end
end

-- "12 / 30", "reloading", or "" for endless ammo (for the HUD).
function Gun:ammo_text(): string
	if self.ammo == nil then return "" end
	if self:reloading() then return "reloading" end
	return tostring(self.clip) .. " / " .. tostring(self.ammo)
end

-- Tracers: thin glowing beams that live a few ticks.
local beams = {}

function weapon.tracer(from: vector, to: vector, color: any?)
	local d = to - from
	local len = vector.magnitude(d)
	if len < 0.05 then return end
	local yaw = util.yaw_to(d)
	local pitch = math.atan2(d.y, vector.magnitude(util.flat(d)))
	local id = world.spawn("cube", { pos = from + d * 0.5, rot = v(pitch, yaw, 0), size = v(0.03, 0.03, len), color = color or "#ffe08a", emissive = 2, solid = false, tag = "tracer" })
	table.insert(beams, { id = id, until_time = game.time() + 0.05 })
end

-- Remove spent tracers; call once per tick.
function weapon.update()
	local t = game.time()
	for i = #beams, 1, -1 do
		if t >= beams[i].until_time then
			world.remove(beams[i].id)
			table.remove(beams, i)
		end
	end
	-- Beams left over from before a reload of this module.
	if #beams == 0 then
		for _, id in world.find("tracer") do world.remove(id) end
	end
end

-- Warm the tracer look in init so the first shot does not stall.
function weapon.warm(color: any?)
	world.warm("cube", { color = color or "#ffe08a", emissive = 2 })
end

local Health = {}
Health.__index = Health

-- Hit points: :hurt(n) (true when this hit kills), :heal(n), :alive(),
-- :frac() (0..1 for ui.bar), :reset(), and .hp, .max.
function weapon.health(max: number)
	return setmetatable({ hp = max, max = max, hurt_at = -1 }, Health)
end

function Health:alive(): boolean return self.hp > 0 end
function Health:frac(): number return math.clamp(self.hp / self.max, 0, 1) end
function Health:reset() self.hp = self.max end
function Health:heal(n: number) if self.hp > 0 then self.hp = math.min(self.max, self.hp + n) end end
function Health:hurt(n: number): boolean
	if self.hp <= 0 then return false end
	self.hp = math.max(0, self.hp - n)
	self.hurt_at = game.time()
	return self.hp <= 0
end
-- Seconds since the last hurt (for a red flash).
function Health:since_hurt(): number return if self.hurt_at < 0 then math.huge else game.time() - self.hurt_at end

return weapon
