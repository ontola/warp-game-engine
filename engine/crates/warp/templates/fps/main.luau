-- Frag Yard: a first-person arena deathmatch for 1 to 4 players in split
-- screen, with bots. First to 10 frags, or the most after three minutes.
-- Built on the game kit (KIT.md): controller.first_person moves, the
-- camera looks through the eyes, weapon shoots, round runs the match and
-- bot plays the bots. Edit this file while it plays: saving reloads it.
--
-- Controls: mouse or right stick looks, WASD or left stick moves, left
-- mouse or right trigger fires, Space or A jumps, R or X reloads, Shift
-- or LB sprints.

local controller = require("kit/controller")
local cam = require("kit/camera")
local ui = require("kit/ui")
local round = require("kit/round")
local bot = require("kit/bot")
local weapon = require("kit/weapon")
local util = require("kit/util")

local v = vector.create

local WIN = 10 -- frags to win
local LENGTH = 180 -- seconds per round
local SIZE = 44 -- arena width, metres
local RESPAWN = 2.5 -- seconds dead
local SAFE = 1.5 -- seconds nobody can hurt you after a (re)spawn
local HP = 100

local FLOOR = "#4d5560"
local WALL = "#8b8f96"
local CRATE = { "#a8743f", "#93653a", "#b98552" }
local ACCENT = "#ff7a3d"

-- The arena: a floor, outer walls, pillars, crates and two raised decks
-- with ramps, mirrored so every side is fair.
local function build_arena()
	world.spawn("cube", { pos = v(0, -0.5, 0), size = v(SIZE, 1, SIZE), color = FLOOR, roughness = 0.9, tag = "level" })
	local h = SIZE / 2
	for _, w in { { v(0, 3, -h), v(SIZE, 6, 1) }, { v(0, 3, h), v(SIZE, 6, 1) }, { v(-h, 3, 0), v(1, 6, SIZE) }, { v(h, 3, 0), v(1, 6, SIZE) } } do
		world.spawn("cube", { pos = w[1], size = w[2], color = WALL, tag = "level" })
	end
	-- Glowing trim along the walls' tops.
	for _, w in { { v(0, 6.05, -h), v(SIZE, 0.1, 1.05) }, { v(0, 6.05, h), v(SIZE, 0.1, 1.05) }, { v(-h, 6.05, 0), v(1.05, 0.1, SIZE) }, { v(h, 6.05, 0), v(1.05, 0.1, SIZE) } } do
		world.spawn("cube", { pos = w[1], size = w[2], color = ACCENT, emissive = 1.2, solid = false, tag = "level" })
	end
	for sx = -1, 1, 2 do
		for sz = -1, 1, 2 do
			world.spawn("cube", { pos = v(sx * 7, 2.5, sz * 7), size = v(2, 5, 2), color = WALL, tag = "level" })
			-- A deck in each corner, 2 m up, with a ramp down towards the middle.
			world.spawn("cube", { pos = v(sx * 16, 1, sz * 16), size = v(8, 2, 8), color = "#6d737c", tag = "level" })
			for i = 1, 4 do -- the ramp as steps (world.move climbs low steps)
				world.spawn("cube", { pos = v(sx * (11.5 - i * 0.8), i * 0.25 - 0.125 + 0.0, sz * 16), size = v(0.8, i * 0.5, 3), color = "#6d737c", tag = "level" })
			end
		end
	end
	for _ = 1, 14 do
		local p = util.random_in_circle(v(0, 0, 0), SIZE * 0.42)
		if math.abs(p.x) > 3 or math.abs(p.z) > 3 then
			local s = random.range(1, 1.8)
			world.spawn("cube", { pos = v(p.x, s / 2, p.z), rot = v(0, random.range(0, math.pi), 0), size = s, color = util.pick(CRATE), tag = "level" })
		end
	end
	lights.set({
		{ pos = v(0, 5, 0), color = ACCENT, intensity = 400, range = 14 },
		{ pos = v(-16, 4, -16), color = "#7fc8ff", intensity = 250, range = 12 },
		{ pos = v(16, 4, 16), color = "#7fc8ff", intensity = 250, range = 12 },
	})
end

local SPAWNS = { v(-18, 3, -18), v(18, 3, 18), v(18, 3, -18), v(-18, 3, 18), v(0, 1, -19), v(0, 1, 19), v(-19, 1, 0), v(19, 1, 0) }

-- The spawn point farthest from everyone alive.
local function spawn_point()
	local best, far = SPAWNS[1], -1
	for _, s in SPAWNS do
		local near = math.huge
		for _, f in state.fighters do
			if f.hp:alive() then near = math.min(near, util.distance(world.get(f.body).pos, s)) end
		end
		near += random.range(0, 3) -- not always the same one
		if near > far then best, far = s, near end
	end
	return best
end

local function view_mask(f)
	-- Hidden in this player's own view only (bit i hides view i).
	return if f.view then bit32.lshift(1, f.view) else 0
end

local function add_fighter(p, view)
	local at = spawn_point()
	local body = world.spawn("capsule", { pos = at, size = v(0.8, 0.9, 0.8), color = p.color, roughness = 0.4, tag = "fighter" })
	local f = {
		id = p.id, bot = p.bot, view = view, body = body,
		ctl = controller.first_person(body, { spawn = at }),
		gun = weapon.new({ damage = 34, rate = 6, spread = 0.008, ammo = 24 }),
		hp = weapon.health(HP),
		cam = cam.rig(p.id, { fov = 75 }),
		dead_until = 0,
		safe_until = game.time() + (if state.round.phase == "countdown" then state.round.left else 0) + SAFE,
		frags = 0,
	}
	f.ctl.yaw = util.yaw_to(-at)
	-- The gun in your hands: seen in your own view only.
	f.viewmodel = world.spawn("cube", { size = v(0.05, 0.06, 0.24), color = "#9aa3b0", metal = 0.5, roughness = 0.4, solid = false, hide_views = bit32.band(0xff, bit32.bnot(view_mask(f))), visible = view ~= nil })
	world.set(body, { hide_views = view_mask(f) })
	table.insert(state.fighters, f)
	state.round:join(p.id)
end

local function fighter(id)
	for i, f in state.fighters do
		if f.id == id then return f, i end
	end
	return nil
end

local function by_body(body)
	for _, f in state.fighters do
		if f.body == body then return f end
	end
	return nil
end

local function init()
	sky.set("afternoon")
	post.set({ bloom = 0.3, contrast = 1.05, vignette = 0.25 })
	weapon.warm()
	world.warm("cube", { color = "#ff4040", emissive = 1 })
	build_arena()
	state.round = round.new({ win_score = WIN, length = LENGTH })
	state.fighters = {}
	local view = 0
	for _, p in players.list() do
		add_fighter(p, if p.bot then nil else view)
		if not p.bot then view += 1 end
		world.warm("cube", { color = p.color, emissive = 0.5 }) -- the frag burst
	end
end

local function kill(f, by)
	f.dead_until = game.time() + RESPAWN
	local at = world.get(f.body).pos
	fx.burst(at, { color = players.get(f.id) and players.get(f.id).color or "#ffffff", emissive = 0.5, count = 24, speed = 6, gravity = 9, life = 0.8 })
	sound.play("pickup", { at = at, pitch = 0.5 })
	world.set(f.body, { visible = false, solid = false })
	if by and by ~= f then state.round:add(by.id, 1) end
end

local function respawn(f)
	local at = spawn_point()
	f.hp:reset()
	f.ctl.vel = util.ZERO
	f.ctl.yaw = util.yaw_to(-at)
	f.ctl.pitch = 0
	world.set(f.body, { pos = at, visible = true, solid = true })
	f.safe_until = game.time() + SAFE
end

-- The nearest living enemy a bot can see (or just the nearest).
local function bot_target(f)
	local me = world.get(f.body).pos
	local best, dist = nil, math.huge
	for _, o in state.fighters do
		if o ~= f and o.hp:alive() then
			local d = util.distance(me, world.get(o.body).pos)
			if d < dist then best, dist = o, d end
		end
	end
	return best
end

local function tick(dt, input)
	state.round:tick(dt)
	weapon.update()
	local playing = state.round:playing()
	for _, f in state.fighters do
		f.gun:tick(dt)
		if not f.hp:alive() then
			if game.time() >= f.dead_until and playing then respawn(f) end
		else
			local inp = input[f.id]
			local target = nil
			if f.bot then
				target = bot_target(f)
				inp = if target then bot.shoot_at(f.ctl, world.get(target.body).pos + v(0, 0.3, 0), { skill = 0.55, ignore = { f.body, target.body } }) else bot.input()
			end
			if not playing then
				local look = inp
				inp = bot.input()
				if look and not f.bot then inp.look_x, inp.look_y = look.look_x, look.look_y end
			end
			local ev = f.ctl:update(inp, dt)
			if ev.jumped then sound.play("jump", { at = world.get(f.body).pos }) end
			if ev.fell then f.cam:snap() end
			if playing then
				local shot = f.gun:fire(inp, f.ctl:eye(), f.ctl:aim(), { ignore = f.body })
				if shot then
					sound.play("jump", { at = shot.from, pitch = 2.2, db = -4 })
					for _, hit in shot.hits do
						local o = by_body(hit.id)
						fx.burst(hit.point, { color = if o then "#ff4040" else "#ffd27a", emissive = 1, count = 6, speed = 3, life = 0.25 })
						if o and o.hp:alive() and game.time() >= o.safe_until then
							players.rumble(o.id, 0.5, 0.15)
							if o.hp:hurt(hit.damage) then kill(o, f) end
						end
					end
				end
			end
		end
	end
	-- Cameras and the guns in hand, after everyone moved.
	for _, f in state.fighters do
		if f.view then
			local eye, aim = f.ctl:eye(), f.ctl:aim()
			if f.hp:alive() then
				f.cam:first_person(eye, f.ctl.yaw, f.ctl.pitch)
			else
				-- Dead: look down at the arena from above where you fell.
				f.cam:look(eye + v(0, 6, 4), eye, dt)
			end
			local right = util.right(f.ctl.yaw)
			local up = vector.cross(right, aim)
			local kick = f.gun.kick * 0.06
			world.set(f.viewmodel, { pos = eye + aim * (0.5 - kick) + right * 0.2 - up * 0.19, rot = v(f.ctl.pitch + kick, f.ctl.yaw, 0), visible = f.hp:alive() })
		end
	end
end

local function draw()
	for _, f in state.fighters do
		if f.view then
			local p = players.get(f.id)
			if p then ui.score(f.id, p.name, state.round:score(f.id), p.color) end
			ui.bar(f.id, "Health", f.hp:frac(), { color = if f.hp:frac() > 0.35 then "#7fe08a" else "#ff5050" })
			ui.bar(f.id, "Ammo " .. f.gun:ammo_text(), if f.gun:reloading() then 0 else f.gun.clip / f.gun.ammo, { row = 1, color = "#ffd27a" })
			if f.hp:alive() then
				ui.crosshair(f.id, { spread = f.gun.kick * 6 })
				local t = f.hp:since_hurt()
				if t < 0.25 then ui.flash(f.id, { 1, 0, 0, 0.3 * (1 - t / 0.25) }) end
			elseif state.round:playing() then
				ui.banner("Fragged", { player = f.id, sub = string.format("Back in %.0f", math.max(0, f.dead_until - game.time())), size = 54 })
			end
			-- Names over the others in your view.
			for _, o in state.fighters do
				if o ~= f and o.hp:alive() then
					local op = players.get(o.id)
					if op then ui.marker(world.get(o.body).pos, op.name, { player = f.id, lift = 1.3, size = 16, color = op.color }) end
				end
			end
		end
	end
	state.round:draw()
end

local function player_joined(p)
	local views = 0
	for _, f in state.fighters do if f.view then views += 1 end end
	add_fighter(p, if p.bot then nil else views)
end

local function player_left(p)
	local f, i = fighter(p.id)
	if f then
		world.remove(f.body)
		world.remove(f.viewmodel)
		table.remove(state.fighters, i)
	end
end

return { init = init, tick = tick, draw = draw, player_joined = player_joined, player_left = player_left }
