local util = require("kit/util")
local city = require("world")
local mobility = require("mobility")
local v = vector.create
local traffic = {}
local LANE = 2.7

-- Six circuits, each with two drivers starting half a lap apart.
local routes = {
	{-1,-1, 1,1, 4,8}, {1,-2, 3,1, 1,9}, {-3,0, -1,2, 2,7},
	{-3,-3, -1,-1, 3,8}, {0,1, 3,3, 4,8.5}, {-3,2, 0,3, 1,7.5},
}

local function corner(i)
	return (i-1)%4+1
end

local function newDriver(fleet,slot)
	local lap=math.floor((slot-1)/#routes)
	local r=routes[(slot-1)%#routes+1]
	local w,n,e,s=r[1]*city.STEP,r[2]*city.STEP,r[3]*city.STEP,r[4]*city.STEP
	local function lane(offset)
		return {v(w+offset,0,n+offset),v(e-offset,0,n+offset),v(e-offset,0,s-offset),v(w+offset,0,s-offset)}
	end
	return {lanes={[1]=lane(LANE),[-1]=lane(-LANE)},step=1,goal=corner(r[5]+2*lap+1),pace=r[6]-.35*lap,
		risk=((fleet.layout.seed%997+slot*37)%101)/100,waiting=0,cleared=false}
end

function traffic.frighten(car,source)
	local d=car.driver
	if not d or not car.auto or car.owner or (car.storyDriver and car.storyDriver~=car.driverActor) or car.wrecked or (car.damage and car.damage.disabled) then return false end
	local now=game.time()
	local p=car.panic
	if p then
		p.untilTime=now+7+3*d.risk
		-- Burst fire and two players shooting across a lane must not make a
		-- driver change their mind about a turn every frame.
		if now-p.start<2 or util.flat_distance(p.source,source)<8 then return true end
	else
		p={}
		car.panic=p
	end
	p.source,p.start,p.untilTime=source,now,now+7+3*d.risk
	d.waiting,d.cleared=0,false
	local threat=util.flat(source-car.pos)
	if vector.dot(threat,util.forward(car.yaw))>math.max(2,.66*vector.magnitude(threat)) and not d.turning then
		-- A head-on threat calls for a U-turn into the opposite travel lane.
		-- The same circuit can then be driven legally in the other direction.
		d.goal,d.step=corner(d.goal-d.step),-d.step
		d.turning=true
	end
	local side=vector.dot(threat,util.right(car.yaw))
	p.swerve=if math.abs(side)>.35 then -math.sign(side) else 0
	return true
end

function traffic.driverDown(fleet,person)
	-- The corpse system takes over rendering a fallen driver. A hidden far
	-- driver can still be hit, then approached by a different player.
	if person.dead and person.body then
		world.remove(person.body)
		person.body=nil
	end
	if person.corpse and person.visible==false then
		person.visible=true
		humans.set(person.actor,{visible=true})
	end
	local car=fleet.vehicles[person.trafficCar]
	if not car or car.driverActor~=person.actor then return end
	if car.storyDriver==person.actor then car.storyDriver=nil end
	car.driverActor,car.driver,car.auto,car.panic=nil,nil,nil,nil
end

local function controls(fleet,car,dt)
	local d,p=car.driver,car.panic
	local now=game.time()
	if car.burnRemaining or (car.damage and car.damage.disabled) then d.evacuate=true end
	if d.evacuate then
		return {brake=if math.abs(car.speed)>1 then .9 else 0,handbrake=if math.abs(car.speed)<2 then 1 else 0}
	end
	if p and now>=p.untilTime then p,car.panic=nil,nil end
	local lane=d.lanes[d.step]
	local pos,speed=car.pos,math.abs(car.speed)
	local goal=lane[d.goal]
	local delta=util.flat(goal-pos)
	local distance=vector.magnitude(delta)
	local target=d.pace
	if p then
		local fast=d.pace+7+3*d.risk
		-- Keep speed on the straight, roll through stops and brake late for turns.
		target=math.min(fast,7+2*d.risk+math.max(0,distance-4)*.53)
		if distance<8.5 then d.cleared=true end
		local leg=goal-lane[corner(d.goal-d.step)]
		local passed=vector.dot(delta,leg)<=0 and distance<20
		if distance<4+math.min(3.5,speed*.22) or passed then
			d.goal=corner(d.goal+d.step)
			d.cleared=false
			delta=util.flat(lane[d.goal]-pos)
		end
	elseif distance<13 and not d.cleared then
		target=math.max(0,(distance-7)*.9)
		if speed<.7 and distance<8.5 then
			d.waiting+=dt
			if d.waiting>=1.8 then d.cleared,d.waiting=true,0 end
		end
	elseif d.cleared then
		target=4
	end
	if not p and distance<4 and d.cleared then
		d.goal,d.cleared=corner(d.goal+d.step),false
	end

	local heading,right=util.forward(car.yaw),util.right(car.yaw)
	local error=util.angle_diff(util.yaw_to(delta),car.yaw)
	if d.turning then
		if math.abs(error)<.55 then d.turning=nil
		else target=math.min(target,5) end
	elseif p and math.abs(error)<.5 then
		-- A short flinch away from the firing side, then steer back into lane.
		error-=p.swerve*.22*math.clamp(1-(now-p.start)/.9,0,1)
	end
	local function inLane(other,range,width)
		local offset=util.flat(other-pos)
		local along=vector.dot(heading,offset)
		return along>0 and along<range and math.abs(vector.dot(right,offset))<width
	end
	-- At panic speeds the shortened following distance can exceed the car's
	-- actual braking ability. Rapier handles the ensuing contacts and damage.
	local following=if p then 5.4-2*d.risk else 7
	for _,other in fleet.vehicles do
		local width=if other.auto or other.owner then 2.1 else 1.9
		if car~=other and math.abs(other.pos.y-pos.y)<2.5 and inLane(other.pos,following,width) then target=0 end
	end
	for _,rider in fleet.players do
		if rider.mode=="walk" and inLane(rider.pos,if p then following-.9 else 7,1.65) then target=0 end
	end
	return {
		throttle=math.clamp((target-speed)*.23,0,if p then 1 else .65),
		brake=math.clamp((speed-target)*.4,0,1),steer=math.clamp(-error*1.7,-1,1),
		handbrake=if target<.1 and speed<.7 then 1 else 0,
	}
end

local function spawnPosition(fleet,from,goal)
	for _,fraction in {.5,.34,.66,.22,.78} do
		local candidate=from+(goal-from)*fraction
		local clear=true
		for _,other in fleet.vehicles do
			if vector.magnitude(candidate-other.pos)<6 then clear=false; break end
		end
		if clear then return candidate end
	end
	error("traffic circuit has no clear starting position")
end

function traffic.add(fleet)
	fleet.traffic={}
	local drive=function(car,dt) return controls(fleet,car,dt) end
	for slot=1,2*#routes do
		local d=newDriver(fleet,slot)
		local lane=d.lanes[d.step]
		local goal=lane[d.goal]
		local pos=spawnPosition(fleet,lane[corner(d.goal-1)],goal)
		local car=mobility.add_car(fleet,pos,util.yaw_to(goal-pos))
		car.driver,car.auto=d,drive
		table.insert(fleet.traffic,car.index)
	end
	fleet.baseCars=#fleet.vehicles
	fleet.lastCarId=fleet.baseCars
end

-- Rebind the controller on a live script edit. Older rounds stored the
-- waypoint in a closure; recover the occupied road segment without moving the car.
function traffic.reload(fleet)
	local drive=function(car,dt) return controls(fleet,car,dt) end
	for slot,id in fleet.traffic do
		local car=fleet.vehicles[id]
		if not car or not car.auto or car.owner or (car.storyDriver and car.storyDriver~=car.driverActor) or car.wrecked then continue end
		if not car.driver then
			if fleet.drivers then continue end -- an evacuated car can now belong to a story
			local d=newDriver(fleet,slot)
			local best=math.huge
			for i,b in d.lanes[1] do
				local a=d.lanes[1][corner(i-1)]
				local leg=b-a
				local f=math.clamp(vector.dot(car.pos-a,leg)/vector.dot(leg,leg),0,1)
				local score=util.flat_distance(car.pos,a+leg*f)-2*vector.dot(util.forward(car.yaw),vector.normalize(leg))
				if score<best then best,d.goal=score,i end
			end
			car.driver=d
		end
		car.auto=drive
	end
end

return traffic
