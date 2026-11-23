-- Swept car and fuselage contacts for the city's walking seats and streamed residents.
-- Coordinates are feet positions. The box derives from the Toyama chassis mesh and
-- the helicopter's solid hull; neither wheels nor decorative rotor blades knock people over.
local util = require("kit/util")
local models = require("vehicles")
local corpses = require("corpses")
local road = require("road_contact")
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
	return {id=id, native=car.native, owner=car.owner, missingWheels=car.missingWheels, pos=pos, prevPos=prev, size=size, yaw=car.yaw, prevYaw=oldYaw,
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

-- Sweep the pedestrian's horizontal circle against a yawed chassis rectangle.
-- A stop against a flat door and a glancing bumper corner get different normals.
local function contact(box, before, feet, dt)
	local a,b=before-box.prevPos,feet-box.pos
	if math.min(math.abs(a.x),math.abs(b.x))>3.5 and a.x*b.x>0
		or math.min(math.abs(a.z),math.abs(b.z))>3.5 and a.z*b.z>0 then return nil end
	local w,h,l=box.size.x*.5,box.size.y*.5,box.size.z*.5
	local t,last=axis(a.y,b.y,-h-HEIGHT,h,0,1)
	if t>last then return nil end
	local ax,az=vector.dot(a,box.oldRight),vector.dot(a,box.oldForward)
	local x,z=vector.dot(b,box.right),vector.dot(b,box.forward)
	t,last=axis(ax,x,-w-RADIUS,w+RADIUS,t,last)
	if t>last then return nil end
	t,last=axis(az,z,-l-RADIUS,l+RADIUS,t,last)
	if t>last then return nil end
	local dx,dz=x-ax,z-az
	local cx,cz=ax+dx*t,az+dz*t
	local nx,nz=cx-math.clamp(cx,-w,w),cz-math.clamp(cz,-l,l)
	local d2=nx*nx+nz*nz
	if d2>RADIUS*RADIUS+.00001 then
		-- The slab corner is outside the rounded footprint. Find the first
		-- true touch there; a walker beyond the corner can safely pass it.
		local speed2=dx*dx+dz*dz
		local along=nx*dx+nz*dz
		local disc=along*along-speed2*(d2-RADIUS*RADIUS)
		if along>=0 or speed2<.0000001 or disc<0 then return nil end
		local step=(-along-math.sqrt(disc))/speed2
		t+=step
		if t>last then return nil end
		nx,nz=nx+dx*step,nz+dz*step
		cx,cz=ax+dx*t,az+dz*t
	end
	local ex,ez=x-math.clamp(x,-w,w),z-math.clamp(z,-l,l)
	local edge=math.sqrt(ex*ex+ez*ez)
	local out
	if edge>.0001 then
		out=(box.right*ex+box.forward*ez)*(math.max(0,RADIUS+.015-edge)/edge)
	elseif w-math.abs(x)<l-math.abs(z) then
		out=box.right*(if x>=0 then 1 else -1)*(w+RADIUS+.015-math.abs(x))
	else
		out=box.forward*(if z>=0 then 1 else -1)*(l+RADIUS+.015-math.abs(z))
	end
	if not dt then return t,out end
	local len=math.sqrt(nx*nx+nz*nz)
	local cy=a.y+(b.y-a.y)*t
	local dy=math.min(h-cy,cy+h+HEIGHT)
	local ny=0
	if dy<math.min(w+RADIUS-math.abs(cx),l+RADIUS-math.abs(cz)) then
		nx,ny,nz=0,(if h-cy<cy+h+HEIGHT then 1 else -1),0
	elseif len>.0001 then
		nx,nz=nx/len,nz/len
	elseif w-math.abs(cx)<l-math.abs(cz) then
		nx,nz=(if cx>=0 then 1 else -1),0
	else
		nx,nz=0,(if cz>=0 then 1 else -1)
	end
	local closing=-(nx*dx+ny*(b.y-a.y)+nz*dz)/dt
	local normal=box.right*nx+v(0,ny,0)+box.forward*nz
	local surface=closing+vector.dot(feet-before,normal)/dt
	-- A passer-by moving alongside a fast car need not fall. Walking into
	-- a parked car is also a gentle separation, even when the walker sprints.
	return t,out,math.min(closing,surface)
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

-- Include room for the visual hull, a walking radius and a fixed-tick step.
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

local function nearbyCars(grid,pos)
	return grid[cell(math.floor(pos.x/CELL),math.floor(pos.z/CELL))]
end

local function person(pop,p,actor,seat,dt,boxes,grid,resident)
	if p.impact and p.impact.stage=="down" then
		if p.impact.elapsed>.30 then road.prone(pop,p,actor,seat,resident,nil,nearbyCars(grid,p.pos),dt) end
		advance(pop,p,actor,seat,dt,boxes)
		p.previousFeet=p.pos
		return
	end
	if p.impact and p.impact.stage=="rise" then advance(pop,p,actor,seat,dt,boxes) end
	local before=p.previousFeet or p.pos
	-- Ordinary walking, sprinting and authored routes stay below this speed.
	-- A teleported or newly streamed body starts from the new pose at rest.
	if vector.magnitude(p.pos-before)>12*dt then before=p.pos end
	local first,vehicle,out,fall,speed=math.huge,nil,nil,false,0
	local nearby=nearbyCars(grid,p.pos)
	for _,box in nearby or EMPTY do
		local t,offset,closing=contact(box,before,p.pos,dt)
		if t then
			local hard=closing>=FALL_SPEED
			if not vehicle or (hard and not fall) or (hard==fall and (t<first or t==first and box.id<vehicle.id)) then
				first,vehicle,out,fall,speed=t,box,offset,hard,closing
			end
		end
	end
	if fall then
		-- Keep the walker's momentum; the moving chassis supplies the entire
		-- collision impulse at the actual bone/bumper contact in the solver.
		if humans.ragdoll(actor,(p.pos-before)/dt,util.ZERO) then
			local home=if p.impact then p.impact.home elseif p.panic and p.panic.home then p.panic.home else p.pos
			p.panic=nil
			p.impact={stage="down",elapsed=0,home=home,vehicle=vehicle.id}
			road.bumper(pop,p,actor,seat,resident,vehicle,speed)
			p.externalPose=true
			if p.story then p.wasHit=true end
			if seat then p.speed=0; p.walk.vel=util.ZERO; p.nearby=nil end
		end
	elseif vehicle and vector.magnitude(out)>0 then
		local home=if p.panic and p.panic.home then p.panic.home else p.pos
		p.panic=nil
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
	local margin=3 -- capsule width plus up to .4 seconds of sprinting between snapshots
	for x=math.floor((math.min(origin.x,endpoint.x)-margin)/CELL),math.floor((math.max(origin.x,endpoint.x)+margin)/CELL) do
		for z=math.floor((math.min(origin.z,endpoint.z)-margin)/CELL),math.floor((math.max(origin.z,endpoint.z)+margin)/CELL) do
			for _,row in streetCells[cell(x,z)] or EMPTY do
				local c=pop.residents[row.id]
				if not c.shotUntil and c.active~=false and not pop.pedestrians.active[row.id] then
					local sweep=row.b-row.a
					visit(row.id,c,row.a+sweep*age,nil,if age<1 then sweep/.4 else util.ZERO)
				end
			end
		end
	end
end

-- Remove the living representations before custody of the fitted rig moves
-- to the corpse roster. Native removal normally reveals the last street LOD.
local function residentDeath(pop,fleet,id,c,actor,gibbed)
	c.panic=nil
	if id<=pop.streetCount then humans.hide_distant("resident",id) end
	pop.pedestrians.active[id]=nil
	if pop.impactResidents then pop.impactResidents[id]=nil end
	pop.selectionIn=0
	return corpses.retain(pop,fleet,c,actor,"resident",gibbed)
end

function impacts.shootResident(pop, fleet, id, feet, push, gibbed, point, velocity)
	local c=pop.residents[id]
	if not c or c.shotUntil or c.dead then return false end
	local home=c.pos
	c.pos=feet
	local actor=pop.pedestrians.active[id]
	if not gibbed then
		if not actor then
			local observers={}
			for _,seat in fleet.players do if not seat.bot then table.insert(observers,seat.pos) end end
			actor=pop.pedestrians:promote(id,pop.residents,observers)
			if not actor and corpses.freeLook(pop,fleet,"resident",c.look) then
				actor=pop.pedestrians:promote(id,pop.residents,observers)
			end
		end
		if actor then
			local down=c.impact and c.impact.stage=="down"
			-- Road contacts already applied their impulse in the chassis solver.
			local falling=if point then humans.ragdoll_impact(actor,point,push,velocity)
				else down or humans.ragdoll(actor,push)
			if not falling then humans.remove(actor); actor=nil
			elseif not down then c.impact={stage="down",elapsed=0,home=home} end
		end
	end
	return residentDeath(pop,fleet,id,c,actor,gibbed)
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
					local t,out,closing=contact(box,before,feet,dt)
					if not t or (closing<FALL_SPEED and vector.magnitude(out)<.002) then continue end
					local old=c.pos
					c.pos=feet
					local actor=pop.pedestrians:promote(row.id,pop.residents,observers)
					if actor then
						c.previousFeet=before
						person(pop,c,actor,false,dt,boxes,grid,row.id)
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

-- The engine accepts up to 64 nearby obstacles. Additional story cars across
-- town should neither consume the limit nor cause us to lose the striking car.
local function sendObstacles(boxes,roots)
	local nearby={}
	for _,box in boxes do
		local sweep=box.pos-box.prevPos
		local d2=vector.dot(sweep,sweep)
		local reach=8+vector.magnitude(box.size)*.5
		local closest=math.huge
		for _,root in roots do
			local toRoot=root-box.prevPos
			local fraction=if d2>.000001 then math.clamp(vector.dot(toRoot,sweep)/d2,0,1) else 0
			local delta=toRoot-sweep*fraction
			closest=math.min(closest,vector.dot(delta,delta))
		end
		if closest<=reach*reach then
			table.insert(nearby,{distance=closest,box=box})
		end
	end
	if #nearby>64 then
		table.sort(nearby,function(a,b)
			if a.distance==b.distance then return a.box.id<b.box.id end
			return a.distance<b.distance
		end)
	end
	local obstacles={}
	for i=1,math.min(#nearby,64) do
		local b=nearby[i].box
		obstacles[i]={id=b.id,vehicle=b.native,profile=if b.native then models.contactProfile else nil,pos=b.pos,prevPos=b.prevPos,size=b.size,yaw=b.yaw,prevYaw=b.prevYaw}
	end
	humans.obstacles(obstacles)
end

function impacts.tick(pop,fleet,dt)
	road.begin(pop)
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
		if c and c.shotUntil and c.impact and c.impact.stage=="down" and not c.dead then
			residentDeath(pop,fleet,id,c,actor,false)
			continue
		end
		if c and c.building and c.impact then
			advance(pop,c,actor,false,dt,EMPTY)
		elseif c and not c.building and c.active~=false then
			person(pop,c,actor,false,dt,boxes,grid,id)
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
	local roots={}
	corpses.tick(pop,fleet,roots)
	local function track(p)
		if p.impact and p.impact.stage=="down" then table.insert(roots,p.impact.hips or p.pos+HIP) end
	end
	for _,seat in fleet.players do track(seat) end
	for actor,p in fleet.storyPeople or EMPTY do
		if p.gibbed then continue end
		if not p.rideId then
			person(pop,p,actor,false,dt,boxes,grid)
			track(p)
		else p.previousFeet=nil end
	end
	for id in pop.pedestrians.active do
		local c=pop.residents[id]
		if c then track(c) end
	end
	corpses.visit(pop,nil,nil,function(id,record,feet,actor)
		road.prone(pop,record,actor,false,nil,id,nearbyCars(grid,record.pos),dt)
	end)
	road.finish(pop)
	if #roots>0 then sendObstacles(boxes,roots)
	elseif pop.hadRagdolls then humans.obstacles({}) end
	pop.hadRagdolls=#roots>0
end

return impacts
