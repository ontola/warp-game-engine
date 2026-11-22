-- A story owns its cast and ordered objectives. The city continues to simulate
-- while a player edits, joins a car, drives a gate or follows a passenger.
local missions={validate=require("story_schema").validate}
local util=require("kit/util")
local v=vector.create
local live=setmetatable({},{__mode="k"})
local serial=0

local function unequip(m)
	local ctx=live[m]
	local seat=ctx and ctx.fleet.players[m.owner]
	if seat and seat.storyPistol==ctx.serial then seat.storyPistol,seat.storyAim=nil,nil end
	m.weapon,m.aiming,m.onTarget=nil,false,false
end

local function finish(m,status,message)
	unequip(m)
	m.status,m.objective,m.target,m.radius=status,message,nil,nil
	if status=="complete" then sound.play("dispatch",{db=-8}) end
end

local function enterStep(m,cast)
	local ctx=live[m]
	local a=ctx.definition.steps[m.step]
	ctx.entered,ctx.boardCar,ctx.boardAt,ctx.previous,ctx.armed=false,nil,nil,nil,false
	m.objective,m.radius=a.text,a.radius
	if a.op=="checkpoint" and m.raceTime==nil then m.raceTime=0 end
	m.target=a.pos or (a.person and cast:position(a.person)) or (a.car and cast:position(a.car))
	if a.op=="contract" then
		m.weapon="Arcade pistol"
		local seat=ctx.fleet.players[m.owner]
		if seat then seat.storyPistol=ctx.serial end
	else
		unequip(m)
	end
end

function missions.start(definition,fleet,cast,ownerId)
	-- The caller validates the complete source before switching stories.
	if not fleet.players[ownerId] then return nil,"Choose an active player for this story" end
	for _,a in definition.cast do
		local actor,err
		if a.kind=="person" then actor,err=cast:person(a.key,a)
		else actor,err=cast:car(a.key,a) end
		if not actor then cast:clear(); return nil,err or ("Unable to place "..a.key) end
	end
	local m={id=definition.id,title=definition.title,owner=ownerId,step=1,total=#definition.steps,
		status="active",aiming=false,onTarget=false,shotFlash=0}
	serial+=1
	live[m]={definition=definition,fleet=fleet,serial=serial}
	enterStep(m,cast)
	return m
end

function missions.stop(m,cast)
	if m then unequip(m); live[m]=nil end
	cast:clear()
end

local function distance(a,b) return vector.magnitude(a-b) end
local function carFor(a,seat,cast)
	if seat.mode~="drive" or not seat.vehicle then return false end
	return not a.car or (cast.cars[a.car] and cast.cars[a.car].index==seat.vehicle)
end

local function pickup(m,ctx,a,seat,cast)
	local pos=cast:position(a.person)
	m.target=pos
	local car=seat.vehicle and ctx.fleet.vehicles[seat.vehicle]
	local stopped=car and carFor(a,seat,cast) and math.abs(car.speed)<1.4
	if not stopped or distance(pos,car.pos)>(a.radius or 7) then
		if ctx.boardCar then cast:pose(a.person,"idle"); ctx.boardCar,ctx.boardAt=nil,nil end
		return false
	end
	-- Walk around the last few metres to the stationary passenger door.
	local door=car.pos+util.right(car.yaw)*1.55
	if ctx.boardCar~=seat.vehicle or not ctx.boardAt or distance(door,ctx.boardAt)>.65 then
		ctx.boardCar,ctx.boardAt=seat.vehicle,door
		cast:approach(a.person,car,"passenger",1.65)
	end
	if distance(pos,door)>.40 then return false end
	return cast:ride(a.person,seat.vehicle,"passenger")
end

local function checkpoint(ctx,a,seat,cast)
	if not carFor(a,seat,cast) then ctx.previous,ctx.armed=nil,false; return false end
	local pos,r=seat.pos,a.radius or 6
	local previous=ctx.previous
	ctx.previous=pos
	if not previous then ctx.armed=distance(pos,a.pos)>r; return false end
	if not ctx.armed then ctx.armed=distance(pos,a.pos)>r; return false end
	-- Swept crossing also works between ticks at racing speed. Switching cars
	-- or starting inside a gate cannot award it without leaving and re-entering.
	local delta=pos-previous
	local len2=vector.dot(delta,delta)
	local t=if len2>0 then math.clamp(vector.dot(a.pos-previous,delta)/len2,0,1) else 0
	return distance(previous+delta*t,a.pos)<=r
end

local function contract(m,ctx,a,seat,cast,inp)
	local pos=cast:position(a.person)
	m.target=pos
	m.aiming=seat.mode=="walk" and not seat.impact and inp.held~=nil and inp.held.aim==true
	m.onTarget=false
	if not m.aiming or not pos then return false end
	local cam=camera.get(m.owner)
	local axis=cam.target-cam.pos
	if vector.magnitude(axis)<.01 then return false end
	local ray=vector.normalize(axis)
	local along=vector.dot(pos+v(0,1.05,0)-cam.pos,ray)
	local inRange=along>0 and distance(seat.pos,pos)<=(a.range or 30)
	if inRange then
		local point=cam.pos+ray*along
		local nearest=pos+v(0,math.clamp(point.y-pos.y,.36,1.53),0)
		if distance(point,nearest)<.48 then
			local wall=world.raycast(cam.pos,ray,math.max(.02,along-.37),{tag="city"})
			m.onTarget=wall==nil
		end
	end
	if not (inp.pressed and inp.pressed.fire) or ctx.cooldown>0 then return false end
	m.shotFlash,ctx.cooldown=.10,.28
	sound.play("arcade_pistol",{at=seat.pos,db=-7})
	return m.onTarget and cast:hit(a.person,ray*2+v(0,.3,0))
end

function missions.tick(m,fleet,cast,dt,input)
	if not m then return end
	m.shotFlash=math.max(0,(m.shotFlash or 0)-dt)
	if m.status~="active" then return end
	if m.raceTime~=nil then m.raceTime+=dt end
	local ctx=live[m]
	if not ctx then return end
	local seat=fleet.players[m.owner]
	if not seat then
		local replacement
		for id,p in fleet.players do if not p.bot and (not replacement or id<replacement) then replacement=id end end
		if not replacement then return end
		m.owner,seat=replacement,fleet.players[replacement]
		if m.weapon then seat.storyPistol=ctx.serial end
	end
	if m.weapon then seat.storyPistol=ctx.serial end
	ctx.cooldown=math.max(0,(ctx.cooldown or 0)-dt)
	local a=ctx.definition.steps[m.step]
	local done=false
	if a.op=="walk" or a.op=="drive" then
		local key=a.person or a.car
		if not ctx.entered then
			local started=if a.op=="walk" then cast:walk(key,a.points,a.speed) else cast:drive(key,a.points,a.speed)
			if not started then finish(m,"failed","The route could not start."); return end
			ctx.entered=true
		end
		m.target=cast:position(key)
		done=a.wait==false or cast:arrived(key)
	elseif a.op=="ride" then
		done=cast:ride(a.person,a.car,a.seat or "passenger")
	elseif a.op=="dismount" then
		done=cast:dismount(a.person)
	elseif a.op=="enter" then
		m.target=cast:position(a.car)
		done=carFor(a,seat,cast)
	elseif a.op=="pickup" then
		done=pickup(m,ctx,a,seat,cast)
	elseif a.op=="dropoff" then
		local rider=cast.people[a.person]
		local car=rider.rideId and fleet.vehicles[rider.rideId]
		done=car~=nil and math.abs(car.speed)<1.4 and distance(car.pos,a.pos)<=(a.radius or 7) and cast:dismount(a.person)
	elseif a.op=="checkpoint" then
		done=checkpoint(ctx,a,seat,cast)
	elseif a.op=="contract" then
		done=contract(m,ctx,a,seat,cast,input[m.owner] or {})
	end
	if done then
		if m.step==m.total then finish(m,"complete","Story complete.")
		else m.step+=1; enterStep(m,cast) end
	end
end

return missions
