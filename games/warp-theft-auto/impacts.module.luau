-- Swept car and fuselage contacts for the city's walking seats and streamed residents.
-- Coordinates are feet positions. The box derives from the Toyama chassis mesh and
-- the helicopter's solid hull; neither wheels nor decorative rotor blades knock people over.
local util = require("kit/util")
local v = vector.create
local impacts = {}
local RADIUS, HEIGHT, FALL_SPEED = .32, 1.70, 2.8
local HIP = v(0,.85,0)

local function chassis(car, id, dt)
	local heli = car.kind == "helicopter"
	local size = if heli then v(2.2,2,4.8) else v(1.96,1.30,4.44)
	local lift = v(0,if heli then 1 else .80,0)
	local pos, prev = car.pos+lift, (car.prevPos or car.pos)+lift
	local oldYaw = car.prevYaw or car.yaw
	local vel = (pos-prev)/dt
	return {id=id, pos=pos, prevPos=prev, size=size, yaw=car.yaw, prevYaw=oldYaw,
		right=util.right(car.yaw), forward=util.forward(car.yaw), oldRight=util.right(oldYaw),
		oldForward=util.forward(oldYaw), velocity=vel, speed=vector.magnitude(vel)}
end

-- Clip relative motion against the three slabs of the chassis expanded by
-- a walker's radius and height. Both endpoints use the actual heading at that tick.
local function axis(a, b, lo, hi, first, last)
	local d = b-a
	if math.abs(d)<.00001 then
		return if a<lo or a>hi then 2 else first, last
	end
	local enter, leave = (lo-a)/d, (hi-a)/d
	return math.max(first,math.min(enter,leave)), math.min(last,math.max(enter,leave))
end

local function contact(box, before, feet)
	local a, b = before-box.prevPos, feet-box.pos
	if math.min(math.abs(a.x),math.abs(b.x))>3.5 and a.x*b.x>0
		or math.min(math.abs(a.z),math.abs(b.z))>3.5 and a.z*b.z>0 then return nil end
	local w, h, l = box.size.x*.5+RADIUS, box.size.y*.5, box.size.z*.5+RADIUS
	local t, last = axis(a.y,b.y,-h-HEIGHT,h,0,1)
	if t>last then return nil end
	local x,z=vector.dot(b,box.right),vector.dot(b,box.forward)
	t,last=axis(vector.dot(a,box.oldRight),x,-w,w,t,last)
	if t>last then return nil end
	t,last=axis(vector.dot(a,box.oldForward),z,-l,l,t,last)
	if t>last then return nil end
	-- For a gentle contact, stand immediately outside the nearest side.
	local dx,dz=w-math.abs(x),l-math.abs(z)
	local out = if dx<dz then box.right*(if x>=0 then 1 else -1)*math.max(0,dx+.015)
		else box.forward*(if z>=0 then 1 else -1)*math.max(0,dz+.015)
	return t,out
end

local function routePoint(pop,c,feet,boxes)
	if not c.path then
		for _,box in boxes do
			if contact(box,feet,c.impact.home) then return feet end
		end
		return c.impact.home
	end
	local closest, when, best = nil, nil, math.huge
	for i,leg in pop.paths[c.path] do
		local scale = vector.dot(leg.delta,leg.delta)
		local at = math.clamp(vector.dot(feet-leg.a,leg.delta)/scale,0,1)
		-- If the usual route is blocked by a parked car, rejoin beyond its bumper.
		for extra=0,6,2 do
			local f=math.min(1,at+extra/math.sqrt(scale))
			local point=leg.a+leg.delta*f
			local blocked=false
			for _,box in boxes do
				if contact(box,feet,point) then blocked=true; break end
			end
			if not blocked then
				local d=vector.dot(point-feet,point-feet)
				if d<best then closest,when,best=point,(i-1)*c.segment+f*c.walkTime,d end
				break
			end
		end
	end
	return closest,when
end

local function place(p,pos,actor,seat)
	p.pos=pos
	if seat then
		p.walk.vel=util.ZERO
		world.place(p.body,pos+HIP,p.yaw)
	else
		if p.story then world.place(p.body,pos+HIP,p.yaw) end
		humans.set(actor,{pos=pos,yaw=p.yaw,pose=p.pose or "walk"})
	end
end

local function resume(pop,p,actor,seat,boxes)
	if p.story then
		p.impact,p.externalPose,p.pose=nil,nil,"idle"
		world.place(p.body,p.pos+HIP,p.yaw)
		humans.set(actor,{pos=p.pos,yaw=p.yaw,pose="idle"})
	elseif seat then
		p.walk.yaw=p.yaw
		p.impact,p.externalPose=nil,nil
		humans.set(actor,{pos=p.pos,yaw=p.yaw+math.pi,pose="walk"})
	else
		p.impact.stage="return"
		p.impact.goal,p.impact.routeTime=routePoint(pop,p,p.pos,boxes)
	end
end

local function advance(pop,p,actor,seat,dt,boxes)
	local hit=p.impact
	hit.elapsed+=dt
	if hit.stage=="down" then
		local hips=humans.joint(actor,"hips")
		if hips then
			hit.hips=hips
			p.pos=v(hips.x,p.pos.y,hips.z)
			if seat or p.story then world.place(p.body,p.pos+HIP,p.yaw) end
		end
		if p.shot then return end
		if p.shotUntil then
			if hit.elapsed >= 4 then
				p.impact,p.externalPose,p.previousFeet,p.active=nil,nil,nil,false
				pop.selectionIn=0
			end
			return
		end
		-- Wait until the body and its immediate exit are clear of fast chassis.
		-- A body can come to rest relative to the bonnet while it is still rolling.
		local clear=hit.elapsed>=1.5
		if clear then
			for _,box in boxes do
				if box.speed>=FALL_SPEED and contact(box,p.pos,p.pos-box.velocity*.35) then clear=false; break end
			end
		end
		local feet=if clear then humans.recover(actor) else nil
		if feet then
			hit.stage,hit.elapsed,hit.hips="rise",0,nil
			p.pos,p.pose=feet,"rise"
			if seat or p.story then world.place(p.body,feet+HIP,p.yaw) end
		end
	elseif hit.stage=="rise" then
		humans.set(actor,{pos=p.pos,pose="rise",t=math.min(hit.elapsed,1)})
		if hit.elapsed>=1 then resume(pop,p,actor,seat,boxes) end
	else
		if not hit.goal then
			if hit.elapsed<(hit.retry or 0) then return end
			hit.goal,hit.routeTime=routePoint(pop,p,p.pos,boxes)
			hit.retry=hit.elapsed+.5
			if not hit.goal then return end
		end
		local delta=hit.goal-p.pos
		local length=vector.magnitude(delta)
		local step=math.min(length,1.55*dt)
		if length>.002 then
			p.yaw=math.atan2(delta.x,delta.z)
			p.pose="walk"
			local nextPos=p.pos+delta*(step/length)
			for _,box in boxes do
				local _,offset=contact(box,nextPos,nextPos)
				if offset and vector.magnitude(offset)>.002 then
					nextPos+=offset
					hit.goal=nil
					hit.retry=hit.elapsed+.5
				end
			end
			place(p,nextPos,actor,false)
		end
		if hit.goal and length<=step+.002 then
			if p.path then p.phase=hit.routeTime-pop.elapsed end
			p.impact,p.externalPose=nil,nil
		end
	end
end

-- An actor advances at most two metres per tick, including teleports handled below.
-- Insert the swept chassis plus that margin into 16 m cells for shared seat/NPC queries.
local CELL, MARGIN, EMPTY = 16, 5.5, {}
local function cell(x,z)
	return x*1024+z
end
local function indexBox(grid,box)
	for x=math.floor((math.min(box.pos.x,box.prevPos.x)-MARGIN)/CELL),math.floor((math.max(box.pos.x,box.prevPos.x)+MARGIN)/CELL) do
		for z=math.floor((math.min(box.pos.z,box.prevPos.z)-MARGIN)/CELL),math.floor((math.max(box.pos.z,box.prevPos.z)+MARGIN)/CELL) do
			local key=cell(x,z)
			local bucket=grid[key]
			if not bucket then bucket={}; grid[key]=bucket end
			table.insert(bucket,box)
		end
	end
end

local function person(pop,p,actor,seat,dt,boxes,grid)
	if p.impact and p.impact.stage=="down" then
		advance(pop,p,actor,seat,dt,boxes)
		p.previousFeet=p.pos
		return
	end
	if p.impact and p.impact.stage=="rise" then advance(pop,p,actor,seat,dt,boxes) end
	local before=p.previousFeet or p.pos
	if vector.magnitude(p.pos-before)>2 then before=p.pos end
	local first, vehicle, out = math.huge,nil,nil
	local nearby=grid[cell(math.floor(p.pos.x/CELL),math.floor(p.pos.z/CELL))]
	for _,box in nearby or EMPTY do
		local t,offset=contact(box,before,p.pos)
		if t and (not vehicle or box.speed>=FALL_SPEED and vehicle.speed<FALL_SPEED
			or (box.speed>=FALL_SPEED)==(vehicle.speed>=FALL_SPEED) and t<first) then
			first,vehicle,out=t,box,offset
		end
	end
	if vehicle and vehicle.speed>=FALL_SPEED then
		local velocity=vehicle.velocity*math.min(.83,10/vehicle.speed)
		local push=velocity+v(0,1+math.min(vehicle.speed*.10,2),0)
		if humans.ragdoll(actor,push) then
			local home=if p.impact then p.impact.home else p.pos
			p.impact={stage="down",elapsed=0,home=home,vehicle=vehicle.id}
			p.externalPose=true
			if p.story then p.wasHit=true end
			if seat then p.speed=0; p.walk.vel=util.ZERO; p.nearby=nil end
		end
	elseif vehicle and vector.magnitude(out)>0 then
		local home=p.pos
		place(p,p.pos+out,actor,seat)
		if not seat and not p.impact then
			p.impact={home=home,elapsed=0}
			p.externalPose=true
			resume(pop,p,actor,false,boxes)
		end
	end
	if p.impact and p.impact.stage=="return" then advance(pop,p,actor,false,dt,boxes) end
	p.previousFeet=p.pos
end

-- Far residents move linearly between the same two street samples the renderer
-- uses. Keep a small spatial index until the next crowd selection. Only cars
-- near a seat query it, so a helicopter crossing town doesn't fit every driver.
local indexedPop, revision, sampleTime, streetCells
local function indexResidents(pop)
	if not pop.streetRevision then return end
	if indexedPop~=pop or revision~=pop.streetRevision then
		indexedPop,revision,sampleTime,streetCells=pop,pop.streetRevision,pop.elapsed,{}
		for id,c in pop.street do
			if c.active~=false then
				local key=cell(math.floor(c.pos.x/CELL),math.floor(c.pos.z/CELL))
				local bucket=streetCells[key]
				if not bucket then bucket={}; streetCells[key]=bucket end
				table.insert(bucket,{id=id,a=c.pos,b=c.nextPos or c.pos})
			end
		end
	end
end

-- Fitted humans use exact feet; distant humans use the same interpolated sample
-- as the renderer. Only visit occupied street cells along this shot's bounds.
function impacts.visitResidents(pop, origin, endpoint, visit)
	for id,actor in pop.pedestrians.active do
		local c=pop.residents[id]
		if c and c.active~=false and not c.shotUntil then visit(id,c,c.pos,actor) end
	end
	indexResidents(pop)
	if not streetCells then return end
	local age=math.clamp((pop.elapsed-sampleTime)/.4,0,1)
	local margin=1.5 -- capsule width and travel between street snapshots
	for x=math.floor((math.min(origin.x,endpoint.x)-margin)/CELL),math.floor((math.max(origin.x,endpoint.x)+margin)/CELL) do
		for z=math.floor((math.min(origin.z,endpoint.z)-margin)/CELL),math.floor((math.max(origin.z,endpoint.z)+margin)/CELL) do
			for _,row in streetCells[cell(x,z)] or EMPTY do
				local c=pop.residents[row.id]
				if not c.shotUntil and c.active~=false and not pop.pedestrians.active[row.id] then
					visit(row.id,c,row.a+(row.b-row.a)*age)
				end
			end
		end
	end
end

function impacts.shootResident(pop, fleet, id, feet, push)
	local c=pop.residents[id]
	if not c or c.shotUntil then return false end
	local home=c.pos
	c.pos=feet
	local actor=pop.pedestrians.active[id]
	if not actor then
		local observers={}
		for _,seat in fleet.players do if not seat.bot then table.insert(observers,seat.pos) end end
		actor=pop.pedestrians:promote(id,pop.residents,observers)
	end
	c.shotUntil=pop.elapsed+20
	if actor and humans.ragdoll(actor,push) then
		c.impact={stage="down",elapsed=0,home=home}
		c.externalPose=true
		pop.impactResidents[id]=c
	else
		-- On a device with no compatible skin slot, release the distant stand-in
		-- at the next snapshot. Damage and visible impact effects still register.
		c.active=false
	end
	pop.selectionIn=0
	return true
end

local function promoteContacts(pop,fleet,dt,boxes,grid)
	indexResidents(pop)
	if not streetCells then return end
	local observers={}
	for _,seat in fleet.players do if not seat.bot then table.insert(observers,seat.pos) end end
	local age=math.clamp((pop.elapsed-sampleTime)/.4,0,1)
	local previous=math.max(0,age-dt/.4)
	for _,box in boxes do
		local visible=false
		for _,eye in observers do
			local offset=eye-box.pos
			if vector.dot(offset,offset)<50*50 then visible=true; break end
		end
		if not visible then continue end
		local a,b=box.prevPos,box.pos
		for x=math.floor((math.min(a.x,b.x)-MARGIN)/CELL),math.floor((math.max(a.x,b.x)+MARGIN)/CELL) do
			for z=math.floor((math.min(a.z,b.z)-MARGIN)/CELL),math.floor((math.max(a.z,b.z)+MARGIN)/CELL) do
				for _,row in streetCells[cell(x,z)] or EMPTY do
					local c=pop.residents[row.id]
					if c.impact or c.shotUntil or pop.pedestrians.active[row.id] then continue end
					local delta=row.b-row.a
					local feet,before=row.a+delta*age,row.a+delta*previous
					local t,out=contact(box,before,feet)
					if not t or (box.speed<FALL_SPEED and vector.magnitude(out)<.002) then continue end
					local old=c.pos
					c.pos=feet
					local actor=pop.pedestrians:promote(row.id,pop.residents,observers)
					if actor then
						c.previousFeet=before
						person(pop,c,actor,false,dt,boxes,grid)
						if c.impact then pop.impactResidents[row.id]=c end
						pop.selectionIn=0
					else
						c.pos=old
					end
				end
			end
		end
	end
end

function impacts.tick(pop,fleet,dt)
	pop.impactResidents=pop.impactResidents or {}
	local boxes,grid={},{}
	for i,car in fleet.vehicles do
		local box=chassis(car,i,dt)
		boxes[i]=box
		indexBox(grid,box)
	end
	-- Native ragdolls receive just moving/parked colliders near their rigs.
	for id,seat in fleet.players do
		local actor=pop.avatars[id]
		if actor and seat.mode=="walk" then person(pop,seat,actor,true,dt,boxes,grid)
		else seat.previousFeet=nil end
	end
	for id,actor in pop.pedestrians.active do
		local c=pop.residents[id]
		if c and c.building and c.impact then
			advance(pop,c,actor,false,dt,EMPTY)
		elseif c and not c.building and c.active~=false then
			person(pop,c,actor,false,dt,boxes,grid)
			if c.impact then pop.impactResidents[id]=c end
		end
	end
	promoteContacts(pop,fleet,dt,boxes,grid)
	-- If streaming releases a rig, its physics ends with it. Reattach the
	-- offscreen resident to their ordinary route using the last known feet.
	for id,c in pop.impactResidents do
		if not c.impact then
			pop.impactResidents[id]=nil
		elseif not pop.pedestrians.active[id] then
			if c.shotUntil then
				c.pos,c.impact,c.externalPose,c.previousFeet,c.active=c.impact.home,nil,nil,nil,false
				pop.impactResidents[id]=nil
				pop.selectionIn=0
				continue
			end
			local point,when=routePoint(pop,c,c.pos,boxes)
			if not point then point,when=routePoint(pop,c,c.pos,EMPTY) end
			c.pos,c.impact,c.externalPose,c.previousFeet=point,nil,nil,nil
			if c.path then c.phase=when-pop.elapsed end
			pop.impactResidents[id]=nil
		end
	end
	local moving=false
	for _,seat in fleet.players do if seat.impact and seat.impact.stage=="down" then moving=true; break end end
	for actor,p in fleet.storyPeople or EMPTY do
		if not p.rideId then
			person(pop,p,actor,false,dt,boxes,grid)
			if p.impact and p.impact.stage=="down" then moving=true end
		else p.previousFeet=nil end
	end
	if not moving then
		for id in pop.pedestrians.active do
			local c=pop.residents[id]
			if c and c.impact and c.impact.stage=="down" then moving=true; break end
		end
	end
	if moving then
		local obstacles={}
		for _,b in boxes do table.insert(obstacles,{id=b.id,pos=b.pos,prevPos=b.prevPos,size=b.size,yaw=b.yaw,prevYaw=b.prevYaw}) end
		humans.obstacles(obstacles)
	elseif pop.hadRagdolls then humans.obstacles({}) end
	pop.hadRagdolls=moving
end

return impacts
