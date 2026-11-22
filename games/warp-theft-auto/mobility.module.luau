local controller = require("kit/controller")
local cameraKit = require("kit/camera")
local util = require("kit/util")
local city = require("world")
local models = require("vehicles")
local v = vector.create
local mobility = {}
local HIP = v(0,.85,0)
local QUERY = { tag="city" }
local BOARD = 3.7
local FAST = 2.0

function mobility.add_car(m, pos, yaw)
	local car = models.car(pos,yaw,#m.vehicles+1)
	table.insert(m.vehicles, car)
	return car
end

function mobility.new(layout)
	vehicles.reset("city")
	local m = {players={}, vehicles={}, layout=layout}
	for _, spawn in city.vehicle_spawns() do
		if spawn.kind == "car" then mobility.add_car(m,spawn.pos,spawn.yaw)
		else table.insert(m.vehicles,models.helicopter(spawn.pos,spawn.yaw)) end
	end
	return m
end

local function nearest(m, p)
	local best, distance = nil, BOARD
	for i, car in m.vehicles do
		if car.owner or math.abs(car.speed) > FAST then continue end
		local d = vector.magnitude(p.pos-car.pos)
		if d < distance then best, distance = i, d end
	end
	return best
end

function mobility.join(m, id)
	if m.players[id] then return end
	local pos = city.spawn(id)
	local body = world.spawn("cube", {pos=pos+HIP, size=v(.64,1.7,.64), solid=false, visible=false, tag="walker"})
	local p = { id=id, body=body, pos=pos, yaw=0, view=0, pitch=0, speed=0, mode="walk",
		walk=controller.walker(body, {speed=3.8,accel=30,turn=11}), cam=cameraKit.rig(id,{smooth=10}) }
	m.players[id] = p
	local i = nearest(m,p)
	p.nearby = if i then m.vehicles[i].kind else nil
end

function mobility.leave(m, id)
	local p = m.players[id]
	if not p then return end
	if p.vehicle then m.vehicles[p.vehicle].owner=nil end
	world.remove(p.body)
	m.players[id] = nil
end

local function exitAt(m, car)
	if math.abs(car.speed)>FAST or (car.kind=="helicopter" and not car.grounded) then return nil end
	local right, forward = util.right(car.yaw), util.forward(car.yaw)
	local width = if car.kind=="car" then 1.62 else 2.15
	for _, delta in {right*width,-right*width,-forward*3.3,forward*3.3} do
		local pos = car.pos+delta
		local y = city.roof(m.layout,pos.x,pos.z,.34)
		if math.abs(y-car.pos.y)>1.15 or city.blocked(m.layout,pos.x,pos.z,.34,y) then continue end
		pos = util.with_y(pos,y+.02)
		local clear = true
		for _, other in m.vehicles do
			if other~=car and vector.magnitude(other.pos-pos)<2.3 then clear=false; break end
		end
		if clear then return pos end
	end
	return nil
end

local function interact(m,p)
	if p.vehicle then
		local car = m.vehicles[p.vehicle]
		local pos = exitAt(m,car)
		if pos then
			car.owner, p.vehicle, p.mode = nil, nil, "walk"
			p.pos, p.view, p.walk.yaw, p.speed = pos, car.yaw, car.yaw, 0
			p.walk.vel = util.ZERO
			world.place(p.body,pos+HIP,car.yaw)
		end
	else
		local i = nearest(m,p)
		if i then
			local car = m.vehicles[i]
			car.owner, car.auto = p.id, nil
			p.mode = if car.kind=="car" then "drive" else "fly"
			p.vehicle, p.nearby, p.view, p.pitch = i, nil, car.yaw, 0
		end
	end
end

local function walk(p,inp,dt)
	p.view -= inp.look_x or 0
	p.pitch = math.clamp(p.pitch+(inp.look_y or 0),-.7,.35)
	p.walk.speed = if inp.held and (inp.held.right_shoulder or inp.held.left_shoulder) then 6.5 else 3.8
	p.walk:update(inp,dt,p.view)
	p.pos = world.get(p.body).pos-HIP
	p.yaw, p.speed = p.walk.yaw, vector.magnitude(util.flat(p.walk.vel))
end

local function cameraFor(p, inp, dt)
	if p.bot then return end
	local drive, fly = p.mode=="drive", p.mode=="fly"
	local h, d = 2.7, 4.3
	if p.building or p.doorway then h, d = 2.05, 2.7 end
	if drive or fly then
		p.view += util.angle_diff(p.yaw,p.view)*(1-math.exp(-5*dt)) - (inp.look_x or 0)
		p.pitch = math.clamp(util.damp(p.pitch,0,1.7,dt)+(inp.look_y or 0),-.55,.20)
		h, d = if drive then 3.2 else 5.0, if drive then 6.8 else 10
	end
	local target = p.pos + v(0,if fly then 1.5 elseif drive then 1.05 else 1.33,0)
	local offset = -util.forward(p.view)*d + v(0,h-(target.y-p.pos.y)-d*math.sin(p.pitch),0)
	local hit = world.raycast(target,offset,vector.magnitude(offset)+.2,QUERY)
	local cam = target+offset
	if hit then
		cam = target+vector.normalize(offset)*math.max(.18,hit.distance-.23)
		p.cam:snap()
	end
	p.cam:look(cam,target,dt,if fly then 65 elseif drive then 62 else 58)
end

function mobility.tick(m,dt,input)
	local ids = {}
	for id in m.players do table.insert(ids,id) end
	table.sort(ids)
	-- A simultaneous request for the last seat is resolved by stable player ID.
	for _,id in ids do
		local p, inp = m.players[id], input[id] or {}
		if inp.pressed and inp.pressed.north then interact(m,p) end
		if not p.vehicle then walk(p,inp,dt) end
	end
	for _, car in m.vehicles do
		local inp = if car.owner then input[car.owner] or {} else nil
		if car.kind=="car" then
			local controls = if car.auto then car.auto(car,dt) else models.drive(inp)
			if controls or car.wasDriven then vehicles.drive(car.native,controls) end
			car.wasDriven = controls~=nil
			car.throttle = if controls then controls.throttle or 0 else 0
			if controls then car.active=true end
		elseif car.owner or not car.grounded then models.fly(car,inp,dt,city.BOUND) end
	end
	vehicles.step(dt)
	for _, car in m.vehicles do
		if car.kind=="car" and car.active then models.sync(car) end
	end
	for _,id in ids do
		local p = m.players[id]
		if p.vehicle then
			local car = m.vehicles[p.vehicle]
			p.pos, p.yaw, p.speed = car.pos, car.yaw, math.abs(car.speed)
		else
			local i = nearest(m,p)
			p.nearby = if i then m.vehicles[i].kind else nil
		end
		cameraFor(p,input[id] or {},dt)
	end
end

return mobility
