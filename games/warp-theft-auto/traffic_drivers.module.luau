-- The twelve traffic drivers keep their Warp Human identity, whether seated,
-- running from the car or retained as a ragdoll.
local Cast=require("story_cast")
local crowd=require("kit/crowd")
local traffic=require("traffic")
local models=require("vehicles")
local util=require("kit/util")
local v=vector.create
local drivers={}
local CITY={tag="city"}

function drivers.bind(fleet,pop)
	-- A live round opened before the dedicated pool was added can borrow
	-- fitted residents until the next launch reads the new game manifest.
	local fitter=if pcall(humans.room,"driver") then crowd.new("driver") else pop.pedestrians
	local cast=if fleet.drivers then Cast.bind(fleet.drivers,fleet,pop,fitter) else Cast.new(fleet,pop,fitter)
	fleet.drivers=cast
	for _,id in fleet.traffic do
		local car,key=fleet.vehicles[id],tostring(id)
		if not car or not car.driver or not car.auto or car.owner or cast.people[key] then continue end
		local p=cast:person(key,{name="Driver",deferWalker=true,pos=car.pos,yaw=car.yaw+math.pi})
		if p then
			p.trafficCar=id
			if cast:ride(key,car,"driver") then car.driverActor=p.actor
			else cast:remove(key) end
		end
	end
end

-- Start at the clear door selected by dismount and run toward open ground.
-- Score a few outgoing paths once, keeping both the car and street furniture solid.
local function leaveRoad(fleet,key,p,car,source)
	local feet=p.exitStep and p.exitStep.to or p.pos
	local outward=util.flat(feet-car.pos)
	local away=util.flat(feet-(source or car.pos))
	local heading=math.atan2(outward.x,outward.z)
	local best,score=nil,-math.huge
	for _,turn in {0,.55,-.55,1.14,-1.14,1.57,-1.57} do
		local dir=v(math.sin(heading+turn),0,math.cos(heading+turn))
		local eye=feet+v(0,.78,0)
		local distance=12
		local wall=world.raycast(eye,dir,distance,CITY)
		if wall then distance=math.max(0,wall.distance-.55) end
		local _,_,hit=models.raycast(fleet,eye,dir,distance,car)
		if hit then distance=math.max(0,hit-.85) end
		local gain=vector.magnitude(away+dir*distance)-vector.magnitude(away)
		local rank=distance+.8*gain
		if distance>.6 and rank>score then score,best=rank,feet+dir*distance end
	end
	if best then fleet.drivers:walk(key,{best},4.2+(car.index%5)*.22) end
end

local function visible(fleet,pos)
	for _,seat in fleet.players do
		if not seat.bot then
			local d=pos-seat.pos
			if vector.dot(d,d)<160*160 then return true end
		end
	end
	return false
end

function drivers.tick(fleet,dt)
	local cast=fleet.drivers
	if not cast then return end
	fleet.driverVisibilityIn=(fleet.driverVisibilityIn or 0)-dt
	local updateVisibility=fleet.driverVisibilityIn<=0
	if updateVisibility then fleet.driverVisibilityIn=.25 end
	for _,id in fleet.traffic do
		local key=tostring(id)
		local p,car=cast.people[key],fleet.vehicles[id]
		if not p or p.dead or not car then continue end
		if updateVisibility then
			local show=visible(fleet,p.pos)
			if p.visible~=show then p.visible=show; humans.set(p.actor,{visible=show}) end
		end
		if car.driverActor~=p.actor then continue end
		local d=car.driver
		if p.rideId and d then
			if (car.panic or (d.crashWatchUntil or 0)>game.time()) and math.abs(car.speed)<1.4 then
				d.blockedFor=(d.blockedFor or 0)+dt
				if d.blockedFor>=2.8 then d.evacuate=true end
			else d.blockedFor=nil end
		end
		if p.rideId and (car.wrecked or (d and d.evacuate)) and math.abs(car.speed)<2 then
			if not p.nextExit or game.time()>=p.nextExit then
				cast:dismount(key,true)
				p.nextExit=game.time()+.35
			end
		end
		if not p.rideId then
			local source=p.escapeSource or (car.panic and car.panic.source)
			traffic.driverDown(fleet,p)
			leaveRoad(fleet,key,p,car,source)
			p.escapeSource=nil
		end
	end
	cast:tick(dt)
end

return drivers
