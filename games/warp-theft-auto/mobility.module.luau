local controller = require("kit/controller")
local cameraKit = require("kit/camera")
local util = require("kit/util")
local city = require("world")
local cape = require("west-cape")
local models = require("vehicles")
local detachment = require("detachment")
local weapons = require("weapon_catalog")
local v = vector.create
local mobility = {}
local HIP = v(0,.85,0)
local QUERY = { tag="city" }
local BOARD = 3.7
local FAST = 2.0

function mobility.add_car(m, pos, yaw, paint)
	local index=(m.lastCarId or #m.vehicles)+1
	local car = models.car(pos,yaw,index,paint)
	if m.lastCarId then m.lastCarId=index end
	m.vehicles[index]=car
	return car
end

function mobility.new(layout)
	vehicles.reset("city")
	models.prepareContacts()
	cape.fleetReady(layout.cape)
	local m = {players={}, vehicles={}, layout=layout}
	for _, spawn in city.vehicle_spawns(layout.seed) do
		if spawn.kind == "car" then mobility.add_car(m,spawn.pos,spawn.yaw)
		else table.insert(m.vehicles,models.helicopter(spawn.pos,spawn.yaw)) end
	end
	return m
end

local function nearest(m, p)
	local best, distance = nil, BOARD
	for i, car in m.vehicles do
		if car.owner or (car.storyDriver and car.storyDriver~=car.driverActor) or car.wrecked or math.abs(car.speed) > FAST then continue end
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

function mobility.exitAt(m, car, side)
	if math.abs(car.speed)>FAST or (car.kind=="helicopter" and not car.grounded) then return nil end
	local right, forward = util.right(car.yaw), util.forward(car.yaw)
	local width = if car.kind=="car" then 1.62 else 2.15
	right *= side or 1
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
		local pos = mobility.exitAt(m,car)
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
			if car.driverActor then
				local key=tostring(i)
				if not m.drivers:dismount(key,true) then return end
				m.drivers.people[key].escapeSource=p.pos
			end
			car.owner, car.auto, car.driver, car.panic = p.id, nil, nil, nil
			p.mode = if car.kind=="car" then "drive" else "fly"
			p.vehicle, p.nearby, p.view, p.pitch = i, nil, car.yaw, 0
		end
	end
end

local function walk(p,inp,dt)
	p.view -= inp.look_x or 0
	p.pitch = math.clamp(p.pitch+(inp.look_y or 0),-.7,.35)
	p.walk.speed = if inp.held and inp.held.right_shoulder then 6.5 else 3.8
	p.walk:update(inp,dt,p.view)
	p.pos = world.get(p.body).pos-HIP
	p.yaw, p.speed = p.walk.yaw, vector.magnitude(util.flat(p.walk.vel))
end

local function cameraFor(p, inp, dt)
	if p.bot then return end
	p.aiming=p.mode=="walk" and not p.impact and inp.held~=nil and inp.held.aim==true
	if p.aiming then
		local focus=p.pos+v(0,1.42,0)
		local forward=util.forward(p.view)
		local offset=-forward*1.45+util.right(p.view)*.46+v(0,.15,0)
		local hit=world.raycast(focus,offset,vector.magnitude(offset),QUERY)
		local eye=if hit then focus+vector.normalize(offset)*math.max(.12,hit.distance-.14) else focus+offset
		local ray=forward*math.cos(p.pitch)+v(0,math.sin(p.pitch),0)
		p.cam:look(eye,eye+ray*15,dt,weapons.get(p.weapon,p).aimFov or 49)
		return
	end
	local drive, fly = p.mode=="drive", p.mode=="fly"
	local doorBlend = p.doorBlend or 0
	local h, d = 2.7-.65*doorBlend, 4.3-1.6*doorBlend
	if drive or fly then
		p.view += util.angle_diff(p.yaw,p.view)*(1-math.exp(-5*dt)) - (inp.look_x or 0)
		p.pitch = math.clamp(util.damp(p.pitch,0,1.7,dt)+(inp.look_y or 0),-.55,.20)
		h, d = if drive then 3.2 else 5.0, if drive then 6.8 else 10
	end
	local target = if p.impact and p.impact.hips then p.impact.hips
		else p.pos + v(0,if fly then 1.5 elseif drive then 1.05 else 1.33,0)
	local offset = -util.forward(p.view)*d + v(0,h-(target.y-p.pos.y)-d*math.sin(p.pitch),0)
	local hit = world.raycast(target,offset,vector.magnitude(offset)+.2,QUERY)
	local cam = target+offset
	if hit then
		cam = target+vector.normalize(offset)*math.max(.18,hit.distance-.23)
		p.cam:snap()
	end
	p.cam:look(cam,target,dt,if fly then 65 elseif drive then 62 else 58)
end

-- A story car currently driven by a player belongs to that player until
-- they get out. Other mission-owned cars can be removed immediately.
function mobility.release_car(m,car)
	if car.owner then car.releaseWhenEmpty=true; return end
	vehicles.remove(car.native)
	world.remove(car.body)
	m.vehicles[car.index]=nil
end

function mobility.tick(m,dt,input)
	local ids = {}
	for id in m.players do table.insert(ids,id) end
	table.sort(ids)
	-- A simultaneous request for the last seat is resolved by stable player ID.
	for _,id in ids do
		local p, inp = m.players[id], input[id] or {}
		p.pistolCooldown=math.max(0,(p.pistolCooldown or 0)-dt)
		if not p.impact then
			if inp.pressed and inp.pressed.north then interact(m,p) end
			if not p.vehicle then walk(p,inp,dt) end
		end
	end
	for _, car in m.vehicles do
		car.prevPos, car.prevYaw = car.pos, car.yaw
		local inp = if car.owner then input[car.owner] or {} else nil
		if car.kind=="car" then
			local controls = if car.auto then car.auto(car,dt) else models.drive(inp)
			controls=models.damagedControls(car,controls)
			if controls or car.wasDriven then vehicles.drive(car.native,controls) end
			car.wasDriven = controls~=nil
			car.throttle = if controls then controls.throttle or 0 else 0
			if controls then car.active=true end
		elseif car.owner or not car.grounded then models.fly(car,inp,dt,city) end
	end
	vehicles.step(dt)
	local contacts=vehicles.impacts()
	for _, car in m.vehicles do
		if car.kind=="car" then
			if car.active then models.sync(car) end
			local contact=contacts[car.native]
			if contact then
				if car.panic and car.driver and contact.deltaV>=3 then
					-- A crash near the end of a panic can leave the driver pinned
					-- after gunfire stops. Give them time to escape on foot.
					car.driver.crashWatchUntil=game.time()+7
				end
				models.hit(car,contact.point,math.clamp((contact.deltaV-1.2)/14,0,1))
			end
		end
		if car.releaseWhenEmpty and not car.owner then mobility.release_car(m,car) end
	end
	detachment.tick(m,dt)
	models.streamDamage(m,dt)
	for _,id in ids do
		local p = m.players[id]
		if p.vehicle then
			local car = m.vehicles[p.vehicle]
			p.pos, p.yaw, p.speed = car.pos, car.yaw, math.abs(car.speed)
			if p.mode=="drive" then p.seatedPos,p.seatedYaw=models.seat(car,-1)
			else p.seatedPos,p.seatedYaw=nil,nil end
		elseif not p.impact then
			p.seatedPos,p.seatedYaw=nil,nil
			local i = nearest(m,p)
			p.nearby = if i then m.vehicles[i].kind else nil
		end
		cameraFor(p,input[id] or {},dt)
	end
end

return mobility
