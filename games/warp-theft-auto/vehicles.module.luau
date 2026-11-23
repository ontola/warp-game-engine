local util = require("kit/util")
local damage = require("kit/car_damage")
local v = vector.create
local models = {}
local PAINT = { "sedan-ochre", "sedan", "sedan-pearl", "sedan-vermilion" }
local BASE = "assets/cars/"
local OFFSET = { v(-.759,0,-1.262), v(.759,0,-1.262), v(-.759,0,1.262), v(.759,0,1.262) }

-- Match the grid boundaries declared by this model's glTF warpCrushGrid.
local CRUSH=damage.profile(.98,2.21,2.19,.48,1.04,1.16)

local paints={ochre=PAINT[1],teal=PAINT[2],pearl=PAINT[3],vermilion=PAINT[4]}

-- Local cross sections of the shipped Toyama mesh, relative to its axle-height
-- origin (.33 m above the road). All paints share the same bumper, bonnet and roof.
models.contactProfile="toyama"
function models.prepareContacts()
	vehicles.contact_profile(models.contactProfile,-.15,{
		{z=-2.21,width=1.00,top=.21}, {z=-2.10,width=1.30,top=.29},
		{z=-2.00,width=1.52,top=.40}, {z=-1.86,width=1.65,top=.48},
		{z=-1.47,width=1.72,top=.63}, {z=-1.00,width=1.74,top=.63},
		{z=-.88,width=1.74,roofWidth=1.55,top=.70}, {z=-.50,width=1.75,roofWidth=1.36,top=.91},
		{z=-.10,width=1.75,roofWidth=1.19,top=1.09}, {z=.50,width=1.75,roofWidth=1.18,top=1.12},
		{z=1.00,width=1.75,roofWidth=1.20,top=1.06}, {z=1.55,width=1.75,roofWidth=1.47,top=.77},
		{z=1.72,width=1.70,top=.71}, {z=1.90,width=1.66,top=.70},
		{z=2.07,width=1.55,top=.78}, {z=2.19,width=1.38,top=.31},
	})
end

function models.car(pos, yaw, index, paint)
	local model = BASE .. (paints[paint] or PAINT[(index-1)%#PAINT+1]) .. ".gltf"
	local body = world.spawn(model, {pos=pos+v(0,.33,0), rot=v(0,yaw,0), tag="vehicle"})
	local wheels = {}
	for i, offset in OFFSET do
		local name = if offset.x < 0 then "wheel-left" else "wheel-right"
		wheels[i] = world.spawn(BASE .. name .. ".gltf", {pos=offset, solid=false, tag="wheel"}, body)
	end
	return {kind="car", index=index, paint=paint, pos=pos, yaw=yaw, speed=0, body=body, wheels=wheels,
		model=model, damage=damage.new(), native=vehicles.spawn(pos,yaw), rpm=950, gear=1, load=0, slip=0, contact=0, loose=0, soft=0, throttle=0}
end

local function localPoint(car,point)
	local r=car.rotation or v(0,car.yaw,0)
	local d=point-(car.pos+v(0,.33,0))
	local cy,sy=math.cos(r.y),math.sin(r.y)
	local x,z=cy*d.x-sy*d.z,sy*d.x+cy*d.z
	local cx,sx=math.cos(r.x),math.sin(r.x)
	local y=cx*d.y+sx*z
	local cz,sz=math.cos(r.z),math.sin(r.z)
	return v(cz*x+sz*y,-sz*x+cz*y,-sx*d.y+cx*z)
end

function models.closestPoint(car,worldPoint,radius)
	if car.kind~="car" then return nil end
	if radius then
		local d=worldPoint-(car.pos+v(0,.78,0))
		if vector.dot(d,d)>(radius+2.61)^2 then return nil end
	end
	return vehicles.closest_point(car.native,models.contactProfile,worldPoint)
end

function models.raycast(fleet,origin,direction,maxDistance,ignoreCar)
	local best,limit=nil,maxDistance
	local cars=fleet.vehicles or fleet
	for _,car in cars do
		if car.kind~="car" or car==ignoreCar then continue end
		local d=car.pos+v(0,.78,0)-origin
		local along=math.clamp(vector.dot(d,direction),0,limit)
		local offset=d-direction*along
		if vector.dot(offset,offset)>6.8 then continue end
		local hit=vehicles.raycast(car.native,models.contactProfile,origin,direction,limit)
		if hit and (not best or hit<limit or (hit==limit and car.index<best.index)) then best,limit=car,hit end
	end
	if best then return best,origin+direction*limit,limit end
	return nil
end

function models.hit(car,worldPoint,amount)
	if car.kind~="car" then return false end
	car.damage=car.damage or damage.new()
	if not damage.hit(car.damage,CRUSH,localPoint(car,worldPoint),amount) then return false end
	local _,frontLeft=damage.cellPose(CRUSH,car.damage.zones,0,0)
	local _,rearRight=damage.cellPose(CRUSH,car.damage.zones,2,2)
	vehicles.crush_query(car.native,models.contactProfile,v(-CRUSH.sideHinge,0,-CRUSH.frontHinge),v(CRUSH.sideHinge,0,CRUSH.rearHinge),v(frontLeft.x,1,rearRight.x),v(frontLeft.z,1,rearRight.z))
	car.damageDirty=true
	return true
end

local function updatePanels(car)
	local creating=not car.panels
	local panel=car.panels or {}
	local model=car.model or world.get(car.body).shape
	for row=0,2 do
		for column=0,2 do
			local i=row*3+column
			local pos,size=damage.cellPose(CRUSH,car.damage.zones,column,row)
			if creating then
				panel[i]=world.spawn(model.."#crush"..i,{pos=pos,size=size,solid=false,tag="damage-panel"},car.body)
			else world.set(panel[i],{pos=pos,size=size}) end
		end
	end
	if creating then
		car.panels=panel
		world.set(car.body,{visible=false})
	end
	car.damageDirty=false
end

-- A single mesh remains in the distance. For the nearest damaged cars, skin
-- the same original surfaces across the crush zones. Reserve space for rooms
-- and transient effects, regardless of how many cars have been damaged.
function models.streamDamage(fleet,dt)
	fleet.crushIn=(fleet.crushIn or 0)-dt
	if fleet.crushIn>0 then return end
	fleet.crushIn=.15
	local near={}
	local used=0
	for _,car in fleet.vehicles do
		if car.panels then used+=9 end
		if car.kind~="car" or not car.damage or car.damage.total<=0 then continue end
		local best=math.huge
		for _,p in fleet.players do
			local d=car.pos-p.pos
			best=math.min(best,vector.dot(d,d))
		end
		if best<48^2 then table.insert(near,{car=car,distance=best*(if car.panels then .86 else 1)}) end
	end
	table.sort(near,function(a,b)
		return a.distance<b.distance or (a.distance==b.distance and a.car.index<b.car.index)
	end)
	local limit=math.clamp(math.floor((world.capacity()-world.count()+used-96)/9),0,8)
	local selected={}
	for i=1,math.min(#near,limit) do selected[near[i].car]=true end
	for _,car in fleet.vehicles do
		if car.panels and not selected[car] then
			world.set(car.body,{visible=true})
			for _,id in car.panels do world.remove(id) end
			car.panels=nil
		end
	end
	for i=1,math.min(#near,limit) do
		local car=near[i].car
		if not car.panels or car.damageDirty then updatePanels(car) end
	end
end

function models.damagedControls(car,controls)
	return damage.controls(car.damage,controls,car.speed)
end

-- Warp Humans' seated pose uses a floor anchor and faces +Z; sedans face -Z.
function models.seat(car,side)
	return car.pos+util.right(car.yaw)*(.44*side)+util.forward(car.yaw)*.04+v(0,.02,0),car.yaw+math.pi
end

function models.drive(input)
	local controls
	if input then
		local held = input.held or {}
		controls = {
			throttle = math.max(input.move_y or 0, if held.fire then 1 else 0, 0),
			brake = math.max(-(input.move_y or 0), if held.aim then 1 else 0, 0),
			steer = input.move_x or 0, handbrake = if held.south then 1 else 0,
		}
	end
	return controls
end

function models.sync(car)
	local p = vehicles.get(car.native)
	car.rotation=p.rot
	car.pos, car.yaw, car.active = p.pos - v(0,.33,0), p.rot.y, not p.parked
	car.speed, car.gear, car.rpm, car.cut = p.speed, p.gear, p.rpm, p.cut
	car.slip, car.torque, car.load = p.slip, p.torque, p.load
	car.contact, car.loose, car.soft = p.contact, p.loose, p.soft
	world.set(car.body, {pos=p.pos, rot=p.rot})
	for j, wheel in car.wheels do
		local w = p.wheels[j]
		world.set(wheel, {pos=OFFSET[j]+v(0,w.x,0), rot=v(w.z,w.y,0)})
	end
end

-- Skids, bubble cabin, fairing, tapered tail and articulated rotors share the
-- root's interpolated pose. The body centre sits one metre above the skids.
function models.helicopter(pos, yaw)
	local body = world.spawn("cube", {pos=pos+v(0,1,0), rot=v(0,yaw,0), size=v(2.2,2,4.8), visible=false, tag="vehicle"})
	local hull, glass, keel, steel, cream = "#db623d", "#224a5b", "#2b3940", "#778889", "#eedbaf"
	local function part(shape, offset, size, color, rot, parent)
		return world.spawn(shape, {pos=offset, size=size, color=color, rot=rot or util.ZERO, solid=false, tag="airframe"}, parent or body)
	end
	part("sphere", v(0,.12,-.14), v(2.4,1.82,3.58), hull)
	part("sphere", v(0,.34,-.70), v(2.30,1.52,2.92), glass)
	part("cube", v(0,.23,.25), v(2.28,.14,1.90), cream)
	part("sphere", v(0,1.02,.38), v(1.05,.66,2.25), hull)
	part("cube", v(0,.25,2.56), v(.42,.43,3.55), cream, v(-.12,0,0))
	part("cube", v(0,.85,4.17), v(.12,1.4,.60), hull, v(.24,0,0))
	part("cube", v(0,.46,3.63), v(1.7,.09,.40), hull)
	for _,side in {-1,1} do
		part("capsule", v(.98*side,-.91,-.12), v(.13,1.19,.13), keel, v(math.pi/2,0,0))
		for _,z in {-.77,.70} do
			part("cube", v(.78*side,-.65,z), v(.11,.64,.13), steel, v(0,0,-.48*side))
		end
	end
	part("cube", v(0,1.48,.15), v(.15,.55,.15), steel)
	local rotor = part("cube", v(0,1.78,.15), v(8.0,.06,.20), keel)
	part("cube", util.ZERO, v(.20,.06,8.0), keel, nil, rotor)
	local tail = part("cube", v(.16,.97,4.16), v(.07,1.43,.13), keel)
	return {kind="helicopter", body=body, rotor=rotor, tail=tail, pos=pos, yaw=yaw,
		vel=util.ZERO, speed=0, angle=0, grounded=true}
end

function models.fly(c, input, dt, city)
	local held = if input then input.held or {} else {}
	local steer = if input then input.move_x or 0 else 0
	local thrust = if input then input.move_y or 0 else 0
	c.yaw -= steer * dt * 1.65
	local up = if held.south then 1 else 0
	local down = if held.right_shoulder or held.east or not c.owner then 1 else 0
	local target = util.forward(c.yaw) * thrust * 26
	local h = util.damp(util.flat(c.vel), target, 2.4, dt)
	local vy = util.approach(c.vel.y, (up-down)*10, 18*dt)
	if c.pos.y >= 85 then vy = math.min(vy,0) end
	local out, contact = world.move(c.body, v(h.x,vy-.02,h.z), dt, {tag="city"})
	local center = world.get(c.body).pos
	local bounded = city.flight_bounds(center)
	if not city.on_land(bounded.x,bounded.z,0) then
		contact = false
		if bounded.y < 1.45 then
			bounded = v(bounded.x,1.45,bounded.z)
			out = v(out.x,math.max(0,out.y),out.z)
		end
	end
	world.set(c.body, {pos=bounded, rot=v(-thrust*.09,c.yaw,steer*.11)})
	c.pos, c.vel = bounded-v(0,1,0), out
	c.grounded, c.speed = contact, vector.magnitude(out)
	if contact and not c.owner then c.vel, c.speed = util.ZERO, 0 end
	c.angle = (c.angle + dt * (if c.owner then 27 else 9)) % (2*math.pi)
	world.set(c.rotor, {rot=v(0,c.angle,0)})
	world.set(c.tail, {rot=v(c.angle*2,0,0)})
end

return models
