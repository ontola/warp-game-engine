local util = require("kit/util")
local damage = require("kit/car_damage")
local detachment = require("detachment")
local charred = require("charred")
local v = vector.create
local models = {}
local PAINT = { "sedan-ochre", "sedan", "sedan-pearl", "sedan-vermilion" }
local BASE = "assets/cars/"
local OFFSET = detachment.wheelOffsets

models.wheelOffsets=OFFSET

-- Match the grid boundaries declared by this model's glTF warpCrushGrid.
local CRUSH=damage.profile(.98,2.21,2.19,.48,1.04,1.16)
local IGNITION, BURN_SECONDS = .82, 6
local WRECK_SECONDS, SMOLDER_SECONDS = 24, 10
local FIRE_INTERVAL, FIRE_RANGE, VISIBLE_FIRES = .10, 110, 4

local paints={ochre=PAINT[1],teal=PAINT[2],pearl=PAINT[3],vermilion=PAINT[4]}

-- Local cross sections of the shipped Toyama mesh, relative to its axle-height
-- origin (.33 m above the road). All paints share the same bumper, bonnet and roof.
models.contactProfile="toyama"
function models.prepareContacts()
	local paths={}
	for _,name in PAINT do table.insert(paths,BASE..name..".gltf") end
	table.insert(paths,charred.body)
	world.warm_dents(paths,8)
	world.warm(charred.wheel(-1,true))
	world.warm(charred.wheel(1,true))
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

-- The Toyama's glazing outlines, in the same axle-centred coordinates as
-- the contact profile. The two sides mirror each other about x=-.005.
local panes = {}
local function pane(center, rim)
	for i,a in rim do
		table.insert(panes,{a=center,u=a-center,b=rim[i%#rim+1]-center})
	end
end
pane(v(0,.812,-.608),{v(-.55,1.01,-.225),v(0,1.046,-.252),v(.55,1.01,-.225),
	v(.70,.615,-.849),v(.50,.616,-.958),v(0,.620,-1.009),v(-.50,.616,-.958),v(-.70,.615,-.849)})
pane(v(0,.869,1.322),{v(-.51,1.031,.98),v(0,1.046,1.018),v(.51,1.031,.98),
	v(.634,.725,1.478),v(.51,.727,1.57),v(.27,.73,1.609),v(-.27,.73,1.609),v(-.51,.727,1.57),v(-.634,.725,1.478)})
pane(v(.654,.825,-.127),{v(.727,.637,.226),v(.739,.604,-.546),v(.684,.75,-.518),
	v(.594,.979,-.101),v(.572,1.011,.294)})
pane(v(.649,.844,.685),{v(.605,.955,.832),v(.723,.671,1.07),v(.725,.641,.314),
	v(.572,1.011,.352),v(.587,.991,.729)})

function models.cabinOpening(car,origin,entry,contact)
	local hit=localPoint(car,entry)
	if hit.y<-.01 or hit.y>1.15 or hit.z< -1.10 or hit.z>1.66 then return false end
	local side=if hit.x<-.005 then -1 else 1
	-- The lower opening is exposed only once the corresponding door is off.
	if math.abs(hit.x)>.56 and hit.y<.67 then
		local front=hit.z>-.74 and hit.z<.20
		local rear=hit.z>.31 and hit.z<.89
		local door=(if side<0 then 2 else 3)+(if rear then 2 else 0)
		if (front or rear) and bit32.btest(car.missingParts or 0,bit32.lshift(1,door)) then return true end
	end
	if hit.y<.52 then return false end
	local target=localPoint(car,contact)
	local dir=vector.normalize(target-localPoint(car,origin))
	local offset=target-hit
	-- Only test the window at the first hull contact. A ray entering through
	-- the bonnet or a pillar cannot use a distant window to reach the cabin.
	local start=hit-dir*.12
	local length=math.min(.48,vector.dot(offset,dir)+.12)
	start=v(side*(start.x+.005),start.y,start.z)
	dir=v(side*dir.x,dir.y,dir.z)
	for _,tri in panes do
		local h=vector.cross(dir,tri.b)
		local det=vector.dot(tri.u,h)
		if math.abs(det)<.000001 then continue end
		local d=start-tri.a
		local u=vector.dot(d,h)/det
		if u<0 or u>1 then continue end
		local q=vector.cross(d,tri.u)
		local w=vector.dot(dir,q)/det
		if w<0 or u+w>1 then continue end
		local t=vector.dot(tri.b,q)/det
		if t>=0 and t<=length then return true end
	end
	return false
end

function models.closestPoint(car,worldPoint,radius)
	if car.kind~="car" then return nil end
	if radius then
		local d=worldPoint-(car.pos+v(0,.78,0))
		if vector.dot(d,d)>(radius+2.61)^2 then return nil end
	end
	return vehicles.closest_point(car.native,models.contactProfile,worldPoint)
end

function models.raycast(fleet,origin,direction,maxDistance,ignoreCar,secondIgnore)
	local best,limit=nil,maxDistance
	local cars=fleet.vehicles or fleet
	for _,car in cars do
		if car.kind~="car" or car==ignoreCar or car==secondIgnore then continue end
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

local function ignite(car,owner)
	if car.burnRemaining or car.wrecked or car.damage.total<IGNITION then return end
	car.burnRemaining,car.fireOwner=BURN_SECONDS,owner or 0
end

function models.hit(car,worldPoint,amount,owner)
	if car.kind~="car" or car.wrecked or not (amount>0) then return false end
	car.damage=car.damage or damage.new()
	local p=localPoint(car,worldPoint)
	local changed=damage.hit(car.damage,CRUSH,p,amount)
	ignite(car,owner)
	local dented=damage.dent(car.damage,CRUSH,p,amount)
	local old=car.missingParts or 0
	detachment.hit(car,p,amount)
	if not changed and not dented and old==(car.missingParts or 0) then return false end
	if dented or old~=(car.missingParts or 0) then vehicles.dent_query(car.native,models.contactProfile,car.damage.dents or {},car.body,car.missingParts or 0) end
	car.damageDirty=true
	return true
end

-- The hull, query surface and loose-parts mask all switch on the same tick.
-- This guard also consumes overlapping grenades and a fuse that expires alongside one.
function models.explode(car)
	if car.kind~="car" or car.wrecked then return false end
	car.wrecked,car.burnRemaining,car.wreckFireRemaining=true,nil,WRECK_SECONDS
	car.damage=car.damage or damage.new()
	car.damage.total,car.damage.disabled=1,true
	car.auto,car.driver,car.panic,car.throttle=nil,nil,nil,0
	if car.voice then sound.stop(car.voice); car.voice=nil end
	charred.apply(car)
	detachment.explode(car)
	local mask=car.missingParts or 0
	local dents=car.damage.dents or {}
	vehicles.dent_query(car.native,models.contactProfile,dents,car.body,mask)
	world.dent(car.body,if car.dentActive then dents else nil,mask)
	car.damageDirty=true
	return true
end

-- One emitter per car, shared by all views. Its flame roots follow the bonnet
-- while the old smoke stays in world space as the car drives away.
local function emitFire(car,pulse)
	local intensity=1-car.burnRemaining/BURN_SECONDS
	local phase=pulse*.96+car.index*2.3
	local dx=math.sin(phase)*.18
	local z=-1.42+.16*math.cos(phase)
	local height=1.2+intensity*1.15
	local width=.54+intensity*.30
	local life=math.clamp(2/(math.abs(car.speed)+1),.14,.40)
	local a=detachment.worldPoint(car,v(-.23+dx,.61,z))
	local b=detachment.worldPoint(car,v(.23+dx,.59,z+.14))
	fx.flame(a,width,height,life)
	fx.flame(b,width*.80,height*(.78+.12*math.sin(phase)),life)
	if (pulse+car.index)%3==0 then
		fx.exhaust_smoke((a+b)*.5+v(0,.45,0),.45+intensity*.38)
	end
end

-- The blast exposes the engine bay and passenger compartment. Keep the flame
-- roots on the current chassis pose while each emitted puff rises in world space.
local function emitWreckFire(car,pulse)
	local remaining=car.wreckFireRemaining
	local phase=pulse*.81+car.index*1.9
	local sway=math.sin(phase)
	local bay=detachment.worldPoint(car,v(-.18+.11*sway,.22,-1.37+.13*math.cos(phase)))
	if remaining>SMOLDER_SECONDS then
		local heat=(remaining-SMOLDER_SECONDS)/(WRECK_SECONDS-SMOLDER_SECONDS)
		local life=math.clamp(2/(math.abs(car.speed)+1),.14,.46)
		fx.flame(bay,.12+.56*heat,.14+(1.74+.17*sway)*heat,life)
		if heat>.25 then
			local cabin=detachment.worldPoint(car,v(.29,.37,.09+.15*sway))
			fx.flame(cabin,.35*heat,1.13*heat,life)
		end
	end
	local cadence=if remaining>4 then 3 else 5
	if (pulse+car.index)%cadence==0 then
		fx.exhaust_smoke(bay+v(0,.34,0),.22+1.70*math.sqrt(remaining/WRECK_SECONDS))
	end
end

function models.tickFires(fleet,dt,onExpired)
	fleet.fireFxIn=math.max(0,(fleet.fireFxIn or 0)-dt)
	local emit=fleet.fireFxIn==0
	if emit then fleet.fireFxIn=FIRE_INTERVAL end
	local visible,expired
	for _,car in fleet.vehicles do
		if car.kind~="car" or not car.damage then continue end
		if car.wrecked then
			if not car.wreckFireRemaining then continue end
			car.wreckFireRemaining-=dt
			if car.wreckFireRemaining<=0 then
				car.wreckFireRemaining=nil
				continue
			end
		else
			-- Bring previously damaged cars into the lifecycle after a live edit.
			if not car.burnRemaining then ignite(car) end
			if not car.burnRemaining then continue end
			car.burnRemaining=math.max(0,car.burnRemaining-dt)
			if car.burnRemaining<=BURN_SECONDS*.5 then charred.apply(car) end
			if car.burnRemaining==0 then
				expired=expired or {}
				table.insert(expired,car)
				continue
			end
		end
		if emit then
			local best=math.huge
			for _,seat in fleet.players do
				if seat.bot then continue end
				local d=seat.pos-car.pos
				best=math.min(best,vector.dot(d,d))
			end
			if best<FIRE_RANGE*FIRE_RANGE then
				visible=visible or {}
				table.insert(visible,{car=car,distance=best})
			end
		end
	end
	if visible then
		fleet.firePulse=(fleet.firePulse or 0)+1
		table.sort(visible,function(a,b)
			return if a.distance==b.distance then a.car.index<b.car.index else a.distance<b.distance
		end)
		for i=1,math.min(#visible,VISIBLE_FIRES) do
			local car=visible[i].car
			if car.wrecked then emitWreckFire(car,fleet.firePulse) else emitFire(car,fleet.firePulse) end
		end
	end
	if expired then
		table.sort(expired,function(a,b) return a.index<b.index end)
		for _,car in expired do if not car.wrecked then onExpired(car) end end
	end
end

function models.streamDamage(fleet,dt)
	fleet.crushIn=(fleet.crushIn or 0)-dt
	if fleet.crushIn>0 then return end
	fleet.crushIn=.12
	local near={}
	for _,car in fleet.vehicles do
		if car.kind~="car" or not car.damage or car.damage.total<=0 then continue end
		-- Retire panel children left by hot-reloading the former nine-cell model.
		if car.panels then
			for _,id in car.panels do world.remove(id) end
			car.panels=nil
			world.set(car.body,{visible=true})
		end
		local best=math.huge
		for _,p in fleet.players do
			local d=car.pos-p.pos
			best=math.min(best,vector.dot(d,d))
		end
		if best<48^2 then table.insert(near,{car=car,distance=best*(if car.dentActive then .86 else 1)}) end
	end
	table.sort(near,function(a,b)
		return a.distance<b.distance or (a.distance==b.distance and a.car.index<b.car.index)
	end)
	local selected={}
	local limit=math.min(8,#near)
	for i=1,limit do selected[near[i].car]=true end
	for _,car in fleet.vehicles do
		if car.kind~="car" or not car.damage or car.damage.total<=0 or selected[car] then continue end
		local mask=car.missingParts or 0
		if car.dentActive or car.shellMask~=mask then
			world.dent(car.body,nil,mask)
			car.dentActive,car.shellMask=nil,mask
		end
	end
	for i=1,limit do
		local car=near[i].car
		if not car.dentActive or car.damageDirty then
			if world.dent(car.body,car.damage.dents or {},car.missingParts or 0) then
				car.dentActive,car.damageDirty,car.shellMask=true,false,nil
			end
		end
	end
end

function models.damagedControls(car,controls)
	return detachment.controls(car,damage.controls(car.damage,controls,car.speed))
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

-- Keep the invisible flight collider independent of the imported airframe
-- and its moving parts. Local -Z is the aircraft's forward axis.
local APACHE_SCALE=1.7
models.apacheScale=APACHE_SCALE
models.apacheHull=v(2.2,2,4.8)*APACHE_SCALE

function models.helicopter(pos,yaw)
	local body=world.spawn("cube",{pos=pos+v(0,APACHE_SCALE,0),rot=v(0,yaw,0),size=models.apacheHull,visible=false,tag="vehicle"})
	local function asset(name,offset)
		return world.spawn("assets/apache/"..name..".gltf",{pos=offset*APACHE_SCALE,size=APACHE_SCALE,solid=false,tag="airframe"},body)
	end
	asset("body",util.ZERO)
	local rotor=asset("rotor",v(0,1.25,-.46))
	local tail=asset("tail-rotor",v(-.22,1.21,4.25))
	local turret=world.spawn("sphere",{pos=v(0,-.48,-2.08)*APACHE_SCALE,size=v(.43,.39,.47)*APACHE_SCALE,color="#38423d",solid=false,tag="apache-turret"},body)
	local barrel=world.spawn("cube",{pos=v(0,0,-.38)*APACHE_SCALE,size=v(.10,.10,.80)*APACHE_SCALE,color="#313834",solid=false,tag="apache-barrel"},turret)
	local tip=world.spawn("cube",{pos=v(0,0,-.43)*APACHE_SCALE,visible=false,solid=false,tag="mount"},barrel)
	local rockets={}
	for _,side in {-1,1} do
		table.insert(rockets,world.spawn("cube",{pos=v(side*1.46,-.19,-1.75)*APACHE_SCALE,visible=false,solid=false,tag="mount"},body))
	end
	-- Attach the seated human's hips to the cockpit; the human keeps their own scale.
	local function seatAnchor(forward,hipY)
		return world.spawn("cube",{pos=v(0,hipY,-forward)*APACHE_SCALE,visible=false,solid=false,tag="seat"},body)
	end
	local pilotSeat,gunnerSeat=seatAnchor(.66,.40),seatAnchor(1.67,.12)
	local searchlight=world.spawn("cube",{pos=v(.39,-.37,-2.89)*APACHE_SCALE,visible=false,solid=false,tag="mount"},body)
	return {kind="helicopter",body=body,searchlight=searchlight,pilotSeat=pilotSeat,gunnerSeat=gunnerSeat,rotor=rotor,tail=tail,turret=turret,gunTip=tip,rocketMounts=rockets,
		pos=pos,yaw=yaw,vel=util.ZERO,speed=0,angle=0,grounded=true}
end

local function flightAxes(rotation)
	local flatForward,flatRight=util.forward(rotation.y),util.right(rotation.y)
	local cp,sp=math.cos(rotation.x),math.sin(rotation.x)
	local cr,sr=math.cos(rotation.z),math.sin(rotation.z)
	local forward=flatForward*cp+util.UP*sp
	local untiltedUp=util.UP*cp-flatForward*sp
	return flatRight*cr+untiltedUp*sr,untiltedUp*cr-flatRight*sr,forward
end

function models.aimTurret(car,seat,input)
	local right,up,forward=flightAxes(world.get(car.body).rot)
	if not seat.sightYaw then
		local initial=forward*math.cos(.17)-up*math.sin(.17)
		seat.sightYaw,seat.sightPitch=util.yaw_to(initial),math.asin(initial.y)
	end
	local sensitivity=if input.held and input.held.aim then .42 else 1
	seat.sightYaw-=sensitivity*(input.look_x or 0)
	seat.sightPitch=math.clamp(seat.sightPitch+sensitivity*(input.look_y or 0),-math.pi*.499,math.pi*.499)
	local wanted=util.forward(seat.sightYaw)*math.cos(seat.sightPitch)+util.UP*math.sin(seat.sightPitch)
	local yaw=math.atan2(-vector.dot(wanted,right),vector.dot(wanted,forward))
	local pitch=math.asin(math.clamp(vector.dot(wanted,up),-1,1))
	seat.turretYaw=math.clamp(yaw,-1.95,1.95)
	seat.turretPitch=math.clamp(pitch,-1.32,.28)
	if seat.turretYaw~=yaw or seat.turretPitch~=pitch then
		-- Carry the sight with the airframe only while a mechanical stop binds.
		local flat=forward*math.cos(seat.turretYaw)-right*math.sin(seat.turretYaw)
		local reachable=flat*math.cos(seat.turretPitch)+up*math.sin(seat.turretPitch)
		seat.sightYaw,seat.sightPitch=util.yaw_to(reachable),math.asin(math.clamp(reachable.y,-1,1))
	end
	world.set(car.turret,{rot=v(seat.turretPitch,seat.turretYaw,0)})
	local pos=world.get(car.gunTip).world_pos
	local dir=vector.normalize(pos-world.get(car.turret).world_pos)
	seat.mountPos,seat.mountDir=pos,dir
end

function models.aimRocket(car,seat)
	local mounts=car.rocketMounts
	local pos=world.get(mounts[if car.rightRocket then 2 else 1]).world_pos
	local _,up,forward=flightAxes(world.get(car.body).rot)
	seat.mountPos=pos
	seat.mountDir=forward*math.cos(.16)-up*math.sin(.16)
end

function models.fly(c, input, dt, city)
	input=input or {}
	local held=input.held or {}
	local lift=math.clamp((input.move_y or 0)+(if held.south then 1 else 0)-(if held.right_shoulder or held.east then 1 else 0),-1,1)
	if not c.owner then lift=-1 end
	local yawDemand=input.move_x or 0
	local bank=input.aim_x or 0
	local nose=input.aim_y or 0
	-- The pad supplies absolute cyclic axes. Arrow keys hold an attitude,
	-- and relative mouse movement provides short cyclic corrections.
	if bank==0 then bank=math.clamp((input.look_x or 0)/(3*dt)+(if held.right then 1 else 0)-(if held.left then 1 else 0),-1,1) end
	if nose==0 then nose=math.clamp((input.look_y or 0)/(3*dt)+(if held.up then 1 else 0)-(if held.down then 1 else 0),-1,1) end
	if c.grounded and lift<=0 then bank,nose=0,0 end
	c.pitch=util.damp(c.pitch or 0,-nose*math.rad(30),3.8,dt)
	c.roll=util.damp(c.roll or 0,-bank*math.rad(38),4.2,dt)
	c.yawRate=util.damp(c.yawRate or 0,-yawDemand*1.55,4,dt)
	c.yaw+=c.yawRate*dt
	-- Vertical assist holds height at centred collective. Banking redirects
	-- lift sideways; levelling the disc leaves the existing momentum to drag.
	local upward=9.81
	local targetVy=lift*(if lift>=0 then 10 else 8)
	local vy=util.approach(c.vel.y,targetVy,14*dt)
	local thrust=upward+(vy-c.vel.y)/dt
	local sideways=-math.tan(c.roll)/math.cos(c.pitch)
	local forward=-math.tan(c.pitch)
	local acceleration=(util.right(c.yaw)*sideways+util.forward(c.yaw)*forward)*math.max(0,thrust)
	local h=(util.flat(c.vel)+acceleration*dt)*math.exp(-(if c.grounded then 4 else .27)*dt)
	if c.pos.y >= 85 then vy = math.min(vy,0) end
	local sweepY=if c.grounded and vy==0 then -.02 else vy
	local out, contact = world.move(c.body, v(h.x,sweepY,h.z), dt, {tag="city"})
	local center = world.get(c.body).pos
	local bounded = city.flight_bounds(center)
	if not city.on_land(bounded.x,bounded.z,0) then
		contact = false
		if bounded.y < APACHE_SCALE+.45 then
			bounded = v(bounded.x,APACHE_SCALE+.45,bounded.z)
			out = v(out.x,math.max(0,out.y),out.z)
		end
	end
	world.set(c.body, {pos=bounded, rot=v(c.pitch,c.yaw,c.roll)})
	c.pos, c.vel = bounded-v(0,APACHE_SCALE,0), out
	c.grounded, c.speed = contact, vector.magnitude(out)
	if contact and not c.owner then c.vel, c.speed = util.ZERO, 0 end
	c.angle = (c.angle + dt * (if c.owner then 27 else 9)) % (2*math.pi)
	world.set(c.rotor, {rot=v(0,c.angle,0)})
	world.set(c.tail, {rot=v(c.angle*2,0,0)})
end

return models
