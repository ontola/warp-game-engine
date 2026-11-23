-- A story owns its cast and ordered objectives. The city continues to simulate
-- while a player edits, joins a car, drives a gate or follows a passenger.
local missions={validate=require("story_schema").validate}
local util=require("kit/util")
local explosions=require("explosions")
missions.warm=require("sidearm").warm

local function unequip(m)
	m.weapon,m.aiming,m.onTarget=nil,false,false
	m.contractPerson,m.contractRange=nil,nil
end

local function finish(m,status,message)
	unequip(m)
	m.status,m.objective,m.target,m.radius=status,message,nil,nil
	if status=="complete" then sound.play("dispatch",{db=-8}) end
end

local function enterStep(m,cast)
	local ctx=m.run
	local a=ctx.definition.steps[m.step]
	ctx.entered,ctx.boardCar,ctx.boardAt,ctx.previous,ctx.armed=false,nil,nil,nil,false
	m.objective,m.action=a.text,a.op
	m.radius=a.radius or (if a.op=="checkpoint" then 6 elseif a.op=="dropoff" then 7 else nil)
	if a.op=="checkpoint" and m.raceTime==nil then m.raceTime=0 end
	m.target=a.pos or (a.person and cast:position(a.person)) or (a.car and cast:position(a.car))
	if a.op=="contract" then
		m.weapon,m.contractPerson,m.contractRange="Arcade pistol",a.person,a.range or 30
	else
		unequip(m)
	end
end

function missions.start(definition,fleet,cast,ownerId,avatars)
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
	m.run={definition=definition}
	enterStep(m,cast)
	return m
end

function missions.stop(m,cast)
	if m then
		unequip(m)
		m.run=nil
	end
	cast:clear()
end

local function distance(a,b) return vector.magnitude(a-b) end
local function carFor(a,seat,cast)
	if seat.mode~="drive" or not seat.vehicle then return false end
	return not a.car or (cast.cars[a.car] and cast.cars[a.car].index==seat.vehicle)
end

local function pickup(m,ctx,a,seat,cast,fleet)
	local pos=cast:position(a.person)
	m.target=pos
	local car=seat.vehicle and fleet.vehicles[seat.vehicle]
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

local function checkpoint(ctx,a,seat,cast,radius)
	if not carFor(a,seat,cast) then ctx.previous,ctx.armed=nil,false; return false end
	local pos,r=seat.pos,radius
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

-- Combat records only confirmed shots. An unrelated hit can never complete
-- the current step. If a later step needs that actor, explain the failure now.
local function resolveShots(m,ctx,cast)
	local shots=m.shotPeople
	m.shotPeople=nil
	if not shots then return false end
	for i=m.step,#ctx.definition.steps do
		local step=ctx.definition.steps[i]
		if step.person and shots[step.person] and (i~=m.step or step.op~="contract") then
			finish(m,"failed",cast.people[step.person].name.." was shot.")
			return false
		end
	end
	return m.contractPerson~=nil and shots[m.contractPerson]==true
end

function missions.tick(m,fleet,cast,dt,input)
	if not m then return end
	local ctx=m.run
	if m.status~="active" then return end
	if not ctx then
		finish(m,"failed","Run this story again after updating the game.")
		return
	end
	local contractHit=resolveShots(m,ctx,cast)
	if m.status~="active" then return end
	if m.raceTime~=nil then m.raceTime+=dt end
	local seat=fleet.players[m.owner]
	if not seat then
		local replacement
		for id,p in fleet.players do if not p.bot and (not replacement or id<replacement) then replacement=id end end
		if not replacement then m.aiming,m.onTarget=false,false; return end
		m.owner,seat=replacement,fleet.players[replacement]
	end
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
		m.target=cast:position(a.person)
		done=cast:ride(a.person,a.car,a.seat or "passenger")
	elseif a.op=="dismount" then
		m.target=cast:position(a.person)
		done=cast:dismount(a.person)
	elseif a.op=="enter" then
		m.target=cast:position(a.car)
		done=carFor(a,seat,cast)
	elseif a.op=="pickup" then
		done=pickup(m,ctx,a,seat,cast,fleet)
	elseif a.op=="dropoff" then
		local rider=cast.people[a.person]
		local car=rider.rideId and fleet.vehicles[rider.rideId]
		done=car~=nil and math.abs(car.speed)<1.4 and distance(car.pos,a.pos)<=m.radius and cast:dismount(a.person)
	elseif a.op=="explosion" then
		explosions.play(a.pos,a.radius or 5)
		done=true
	elseif a.op=="checkpoint" then
		done=checkpoint(ctx,a,seat,cast,m.radius)
	elseif a.op=="contract" then
		m.target=cast:position(a.person)
		done=contractHit
	end
	if done then
		if m.step==m.total then finish(m,"complete","Story complete.")
		else m.step+=1; enterStep(m,cast) end
	end
end

return missions
