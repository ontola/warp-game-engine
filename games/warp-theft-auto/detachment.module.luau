-- A hit stresses the hardware closest to its actual contact point. Original
-- panels and tires detach once; free pieces are shared by every local player.
local util=require("kit/util")
local charred=require("charred")
local detachment={}
local v=vector.create
local UP,DOWN=v(0,1,0),v(0,-1,0)
local CITY={tag="city"}
local WHEELS={v(-.759,0,-1.262),v(.759,0,-1.262),v(-.759,0,1.262),v(.759,0,1.262)}
detachment.wheelOffsets=WHEELS
local NAMES={"bonnet","door-lf","door-rf","door-lr","door-rr","boot"}
-- Half extents measured from the indexed triangles of each cooked panel.
local HALF={v(.705,.17,.59),v(.205,.535,.57),v(.205,.535,.57),v(.155,.535,.475),v(.155,.535,.475),v(.675,.235,.315)}
-- Mix large silhouettes and wheels early when several wrecks share the cap.
local BLAST_ORDER={1,7,2,6,10,3,4,8,5,9}
local TIRE_HALF=v(.118,.331,.331)
local CENTERS={}
local LIMIT,RESERVE=18,80

local function partCenter(car,id)
	if id>6 then return WHEELS[id-6] end
	local path=car.model.."#part-"..NAMES[id]
	local center=CENTERS[path] or world.part_center(path)
	if center then CENTERS[path]=center end
	return center
end

local function closestPart(car,p,amount)
	-- A tire is struck at hub height and on the outside of the car. The
	-- chassis above the wheel arch is a different piece of bodywork.
	for i,hub in WHEELS do
		if math.abs(p.x-hub.x)<.32 and (p.y-hub.y)^2+(p.z-hub.z)^2<.37^2 then return i+6 end
	end
	local side=math.abs(p.x)
	-- A direct end-on crash also loads the nearest lid through its bumper.
	if p.z<-.80 and (p.y>.14 or (amount>=.5 and p.z< -1.87)) and side<.88 then return 1 end
	if p.z>1.45 and (p.y>.12 or (amount>=.5 and p.z>1.86)) and side<.86 then return 6 end
	if side<.60 or p.y<-.14 or p.y>1.06 or p.z<-.94 or p.z>1.44 then return nil end
	-- Use the cooked door centers, including reverse travelling and either paint.
	local front=if p.x<0 then 2 else 3
	local a,b=partCenter(car,front),partCenter(car,front+2)
	if not a or not b then return nil end
	return if math.abs(p.z-a.z)<math.abs(p.z-b.z) then front else front+2
end

local function missing(car,id)
	if id>6 then return bit32.btest(car.missingWheels or 0,bit32.lshift(1,id-7)) end
	return bit32.btest(car.missingParts or 0,bit32.lshift(1,id))
end

local function queuePart(car,id,center,amount,blast)
	if id>6 then
		car.missingWheels=bit32.bor(car.missingWheels or 0,bit32.lshift(1,id-7))
		world.set(car.wheels[id-6],{visible=false})
	else
		car.missingParts=bit32.bor(car.missingParts or 0,bit32.lshift(1,id))
		car.damageDirty=true
	end
	car.pendingParts=car.pendingParts or {}
	table.insert(car.pendingParts,{id=id,center=center,amount=amount,blast=blast})
end

-- Claim each original part once. A normal hit earlier in this tick may have
-- already claimed the hinge without spawning the loose mesh yet.
function detachment.explode(car)
	if not car.model or not car.wheels or car.debrisExploded then return false end
	car.debrisExploded,car.blastDebrisPending=true,true
	if car.pendingParts then
		for _,part in car.pendingParts do
			part.blast=true
		end
	end
	for _,id in BLAST_ORDER do
		if missing(car,id) then continue end
		local center=partCenter(car,id)
		if center then queuePart(car,id,center,1,true) end
	end
	return true
end

function detachment.hit(car,localPoint,amount)
	if not car.model or amount<=0 then return false end
	local id=closestPart(car,localPoint,amount)
	-- Raycasts move inward with the deformed triangles. A deep hit may be
	-- beyond a door's original inner face but still be on that same door.
	if not id and car.partStress and car.damage and car.damage.dents then
		local best=math.huge
		for _,dent in car.damage.dents do
			local delta=localPoint-dent.center
			local depth=vector.dot(delta,dent.inward)
			if depth<=0 or depth>dent.depth+.08 then continue end
			local tangent=delta-dent.inward*depth
			local d2=vector.dot(tangent,tangent)
			if d2>=math.min(best,(dent.radius*.55)^2) then continue end
			local surfacePart=closestPart(car,dent.center+tangent,amount)
			if surfacePart and car.partStress[surfacePart] then id,best=surfacePart,d2 end
		end
	end
	if not id or missing(car,id) then return false end
	local wheel=id>6
	local center=partCenter(car,id)
	if not center then return false end
	car.partStress=car.partStress or {}
	local stress=(car.partStress[id] or 0)+amount
	car.partStress[id]=stress
	-- Light taps can crease the shell. One exceptional impact, or several
	-- hits to the same attachment, breaks the selected piece loose.
	if stress<(if wheel then .94 else .82) then return false end
	queuePart(car,id,center,math.clamp(amount,.15,1),false)
	return true
end

local function rotate(point,angles)
	local cz,sz=math.cos(angles.z),math.sin(angles.z)
	local cx,sx=math.cos(angles.x),math.sin(angles.x)
	local cy,sy=math.cos(angles.y),math.sin(angles.y)
	local x,y=cz*point.x-sz*point.y,sz*point.x+cz*point.y
	local z=sx*y+cx*point.z
	return v(cy*x+sy*z,cx*y-sx*point.z,-sy*x+cy*z)
end

function detachment.worldPoint(car,point)
	return car.pos+UP*.33+rotate(point,car.rotation or v(0,car.yaw,0))
end

local function inheritedVelocity(car,dt)
	if car.prevPos then return util.clamp_length((car.pos-car.prevPos)/dt,40) end
	return util.forward(car.yaw)*car.speed
end

local function scatter(car,item,outward)
	-- Deterministic variation separates the wheels and panels without using
	-- the shared city RNG. Add the impulse to the incoming world velocity.
	local spread=((car.index*5+item.part*11)%13)/12
	local planar=util.flat(outward)
	local radial=if vector.magnitude(planar)>.01 then vector.normalize(planar) else util.forward(car.yaw)
	local outwardSpeed=if item.wheel then 8.4 else 6.5
	item.vel+=radial*(outwardSpeed+spread*4)+UP*(5.3+2.8*spread)
	local tumble=vector.normalize(v(radial.z,.35-spread,-radial.x))
	item.spin=tumble*(6+spread*5)
	if item.wheel then item.spin+=v(math.clamp(car.speed/.331,-24,24)+8,0,0) end
end

local function release(fleet,car,part,dt)
	local list=fleet.debris
	if #list>=LIMIT then world.remove(table.remove(list,1).entity) end
	if world.count()>=world.capacity()-RESERVE then return end
	local id,center=part.id,part.center
	local wheel=id>6
	local rotation=car.rotation or v(0,car.yaw,0)
	local pos=detachment.worldPoint(car,center)
	local path=if wheel then charred.wheel(center.x,car.charred) else car.model.."#part-"..NAMES[id]
	local entity=world.spawn(path,{pos=pos,rot=rotation,tag="vehicle-debris",solid=false})
	local outward=if id==1 or id==6 then v(0,.36,math.sign(center.z)) else v(math.sign(center.x),.23,0)
	local item={entity=entity,source=car.body,part=id,wheel=wheel,door=id>=2 and id<=5,pos=pos,rot=rotation,
		vel=inheritedVelocity(car,dt),spin=util.ZERO,life=0,nextProbe=0}
	if part.blast then
		scatter(car,item,rotate(center,rotation))
	else
		item.vel+=rotate(outward,rotation)*(1.8+3.3*part.amount)+UP*(.7+part.amount)
		item.spin=if wheel then v(math.clamp(car.speed/.331,-28,28)+3,0,.8)
			else v(2.4+part.amount*2,(car.index%3-1)*1.7,math.sign(center.x)*4.2)
	end
	table.insert(list,item)
end

local function supportHeight(item)
	local half=HALF[item.part] or TIRE_HALF
	local cx=math.abs(math.cos(item.rot.x))
	return cx*(math.abs(math.sin(item.rot.z))*half.x+math.abs(math.cos(item.rot.z))*half.y)+math.abs(math.sin(item.rot.x))*half.z
end

local function integrate(item,dt)
	if item.settleRoll then
		local pitch=util.angle_diff(0,item.rot.x)
		local roll=util.angle_diff(item.settleRoll,item.rot.z)
		local follow=math.min(1,10*dt)
		item.rot=v(item.rot.x+pitch*follow,item.rot.y,item.rot.z+roll*follow)
		item.pos=v(item.pos.x,item.floor+supportHeight(item),item.pos.z)
		item.rest=math.abs(pitch)+math.abs(roll)<.03
		world.set(item.entity,{pos=item.pos,rot=item.rot})
		return
	end
	item.vel+=DOWN*(9.81*dt)
	if item.life>=item.nextProbe then
		local horizon=.12
		local ahead=item.pos+item.vel*horizon
		local floor=world.raycast(ahead+UP*.6,DOWN,12,CITY)
		item.floor=if floor then floor.point.y else nil
		local arc=ahead-item.pos
		local wall=world.raycast(item.pos,arc,vector.magnitude(arc)+.16,CITY)
		item.wall=if wall and math.abs(wall.normal.y)<.65 then wall else nil
		item.nextProbe=item.life+horizon
	end
	item.rot+=item.spin*dt
	local nextPos=item.pos+item.vel*dt
	local wall=item.wall
	if wall and vector.dot(item.vel,wall.normal)<0 then
		local depth=.15-vector.dot(nextPos-wall.point,wall.normal)
		if depth>0 then
			nextPos+=wall.normal*depth
			item.vel=(item.vel-wall.normal*(1.3*vector.dot(item.vel,wall.normal)))*.72
			item.wall,item.nextProbe=nil,0
		end
	end
	local radius=supportHeight(item)
	if item.floor and nextPos.y<item.floor+radius and item.vel.y<0 then
		nextPos=v(nextPos.x,item.floor+radius,nextPos.z)
		local drag=if item.wheel then .91 else .65
		item.vel=v(item.vel.x*drag,-item.vel.y*.24,item.vel.z*drag)
		item.spin*=drag
		if vector.magnitude(item.vel)<.58 then
			item.settleRoll=if item.door or item.wheel then (if math.sin(item.rot.z)>=0 then 1 else -1)*math.pi*.5
				else (if math.cos(item.rot.z)>=0 then 0 else math.pi)
		end
	end
	item.pos=nextPos
	world.set(item.entity,{pos=nextPos,rot=item.rot})
end

function detachment.tick(fleet,dt)
	fleet.debris=fleet.debris or {}
	local list=fleet.debris
	local pending,maxParts=nil,0
	for _,car in fleet.vehicles do
		if car.blastDebrisPending then
			car.blastDebrisPending=nil
			for _,item in list do
				if item.source~=car.body then continue end
				local delta=item.pos-car.pos
				if vector.dot(delta,delta)>4.5^2 then continue end
				if car.charred then charred.piece(item.entity) end
				item.rest,item.settleRoll,item.wall,item.floor=nil,nil,nil,nil
				item.life,item.nextProbe=0,0
				scatter(car,item,delta)
			end
		end
		if car.pendingParts then
			pending=pending or {}
			table.insert(pending,car)
			maxParts=math.max(maxParts,#car.pendingParts)
		end
	end
	if pending then
		-- Interleave cars bursting on the same tick. Even a chain explosion
		-- creates at most LIMIT loose entities before the next rendered frame.
		local claimed=0
		for i=1,maxParts do
			for _,car in pending do
				local part=car.pendingParts[i]
				if part then
					release(fleet,car,part,dt)
					claimed+=1
					if claimed>=LIMIT then break end
				end
			end
			if claimed>=LIMIT then break end
		end
		for _,car in pending do car.pendingParts=nil end
	end
	for i=#list,1,-1 do
		local piece=list[i]
		piece.life+=dt
		if piece.life>=40 or piece.pos.y< -16 then
			world.remove(piece.entity)
			table.remove(list,i)
		elseif not piece.rest then integrate(piece,dt) end
	end
end

-- Preserve service braking while reducing power and directional grip. A car
-- missing all its tires cannot keep accelerating in either direction.
function detachment.controls(car,controls)
	local mask=car.missingWheels or 0
	if not controls or mask==0 then return controls end
	local missing=0
	for i=0,3 do if bit32.btest(mask,bit32.lshift(1,i)) then missing+=1 end end
	local power=math.max(0,1-.29*missing)
	local out=table.clone(controls)
	out.throttle=(out.throttle or 0)*power
	if car.speed<.2 then out.brake=(out.brake or 0)*power end
	local frontLeft,frontRight=bit32.btest(mask,1),bit32.btest(mask,2)
	local pull=if frontLeft==frontRight then 0 elseif frontLeft then -.22 else .22
	out.steer=math.clamp((out.steer or 0)*(1-.16*missing)+pull*math.clamp(math.abs(car.speed)/6,0,1),-1,1)
	return out
end

return detachment
