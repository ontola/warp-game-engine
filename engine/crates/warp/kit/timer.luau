-- kit/timer: timers, cooldowns, tweens and easing, all on game time (so
-- replays match). `local timer = require("kit/timer")`.
--
--   state.timers = timer.new()            -- in init
--   state.timers:after(2, function() ... end)
--   state.timers:every(0.5, spawn_enemy)
--   state.timers:tick(dt)                 -- once per tick, in tick()
--
--   state.shot = timer.cooldown(0.25)     -- fire at most 4 times a second
--   if input.held.fire and state.shot:use() then ... end
--
--   local t = timer.tween(0, 1, 0.4, timer.ease.out_back)
--   world.set(id, { size = t:value() })   -- any tick or draw after

local timer = {}

local Timers = {}
Timers.__index = Timers

-- A list of timers. Keep it in state and call :tick(dt) every tick.
function timer.new()
	return setmetatable({ now = 0, list = {}, next = 1 }, Timers)
end

-- Call fn once after `seconds`. Returns an id for :cancel.
function Timers:after(seconds: number, fn: () -> ()): number
	local id = self.next
	self.next += 1
	table.insert(self.list, { id = id, at = self.now + seconds, fn = fn })
	return id
end

-- Call fn every `seconds` (the first time after one period), `count`
-- times or forever. fn may return false to stop.
function Timers:every(seconds: number, fn: () -> boolean?, count: number?): number
	local id = self.next
	self.next += 1
	table.insert(self.list, { id = id, at = self.now + seconds, fn = fn, every = math.max(seconds, 1e-6), left = count })
	return id
end

function Timers:cancel(id: number)
	for i, t in self.list do
		if t.id == id then table.remove(self.list, i) return end
	end
end

function Timers:clear() self.list = {} end

-- Seconds until timer `id` fires, or nil if it is gone.
function Timers:left(id: number): number?
	for _, t in self.list do
		if t.id == id then return t.at - self.now end
	end
	return nil
end

-- Advance by dt and run what is due, earliest first (ties by id).
function Timers:tick(dt: number)
	self.now += dt
	while true do
		local best, bi = nil, nil
		for i, t in self.list do
			if t.at <= self.now + 1e-9 and (best == nil or t.at < best.at or (t.at == best.at and t.id < best.id)) then
				best, bi = t, i
			end
		end
		if not best then return end
		if best.every then
			local again = best.fn()
			if best.left then best.left -= 1 end
			if again == false or (best.left and best.left <= 0) then
				self:cancel(best.id)
			else
				best.at += best.every
			end
		else
			table.remove(self.list, bi)
			best.fn()
		end
	end
end

local Cooldown = {}
Cooldown.__index = Cooldown

-- Something that can happen at most once per `seconds` of game time.
function timer.cooldown(seconds: number)
	return setmetatable({ length = seconds, ready_at = 0 }, Cooldown)
end

function Cooldown:ready(): boolean return game.time() >= self.ready_at - 1e-9 end

-- True (and starts the wait) when ready; false otherwise.
function Cooldown:use(): boolean
	if not self:ready() then return false end
	self.ready_at = game.time() + self.length
	return true
end

-- 0 just used .. 1 ready (for a HUD bar).
function Cooldown:progress(): number
	if self.length <= 0 then return 1 end
	return math.clamp(1 - (self.ready_at - game.time()) / self.length, 0, 1)
end

function Cooldown:reset() self.ready_at = 0 end

local Tween = {}
Tween.__index = Tween

-- A value going from `from` to `to` (numbers or vectors) over `seconds`
-- of game time from now, shaped by an easing function (default smooth).
function timer.tween(from: any, to: any, seconds: number, ease: ((number) -> number)?)
	return setmetatable({ from = from, to = to, start = game.time(), length = math.max(seconds, 1e-6), ease = ease or timer.ease.smooth }, Tween)
end

-- 0 .. 1 of the way, before easing.
function Tween:progress(): number return math.clamp((game.time() - self.start) / self.length, 0, 1) end
function Tween:done(): boolean return self:progress() >= 1 end
function Tween:value(): any
	local k = self.ease(self:progress())
	return self.from + (self.to - self.from) * k
end

-- Easing: each maps 0..1 to 0..1 (back and elastic overshoot).
local ease = {}
timer.ease = ease
function ease.linear(t) return t end
function ease.smooth(t) return t * t * (3 - 2 * t) end
function ease.in_quad(t) return t * t end
function ease.out_quad(t) return 1 - (1 - t) * (1 - t) end
function ease.in_out_quad(t) return if t < 0.5 then 2 * t * t else 1 - (-2 * t + 2) ^ 2 / 2 end
function ease.in_cubic(t) return t * t * t end
function ease.out_cubic(t) return 1 - (1 - t) ^ 3 end
function ease.in_out_cubic(t) return if t < 0.5 then 4 * t * t * t else 1 - (-2 * t + 2) ^ 3 / 2 end
function ease.out_back(t)
	local c1 = 1.70158
	local c3 = c1 + 1
	return 1 + c3 * (t - 1) ^ 3 + c1 * (t - 1) ^ 2
end
function ease.out_elastic(t)
	if t == 0 or t == 1 then return t end
	return 2 ^ (-10 * t) * math.sin((t * 10 - 0.75) * (2 * math.pi) / 3) + 1
end
function ease.out_bounce(t)
	local n, d = 7.5625, 2.75
	if t < 1 / d then return n * t * t
	elseif t < 2 / d then t -= 1.5 / d return n * t * t + 0.75
	elseif t < 2.5 / d then t -= 2.25 / d return n * t * t + 0.9375
	else t -= 2.625 / d return n * t * t + 0.984375 end
end

-- A 0 -> 1 -> 0 pulse over `seconds` (a hit flash, a pickup pop).
function ease.pulse(t) return math.sin(math.clamp(t, 0, 1) * math.pi) end

return timer
