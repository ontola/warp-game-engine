-- Named actors borrow fitted Warp Humans from the city. Story cars are ordinary
-- Rapier sedans that can be driven by a local player or a waypoint controller.
local mobility=require("mobility")
local city=require("world")
local cape=require("west-cape")
local models=require("vehicles")
local util=require("kit/util")
local v=vector.create
local HIP=v(0,.85,0)
local QUERY={tag="city"}
local Cast={}
-- The metatable supplies methods and shared services outside serialized cast
-- data. Rebind it on a live edit without respawning a person or a car.
function Cast.bind(self,fleet,population,fitter)
	fleet.storyPeople=fleet.storyPeople or {}
	return setmetatable(self,{
		__index=Cast, fleet=fleet, population=population, fitter=fitter or population.pedestrians,
	})
end

function Cast.new(fleet,population,fitter)
	return Cast.bind({people={},cars={}},fleet,population,fitter)
end

local function duplicate(self,key)
	return self.people[key] or self.cars[key]
end

function Cast:person(key,opts)
	if duplicate(self,key) then return nil,"duplicate cast name" end
	local systems=getmetatable(self)
	local pop=systems.population
	local observers={}
	for _,seat in systems.fleet.players do if not seat.bot then table.insert(observers,seat.pos) end end
	local _,_,bodies,looks=humans.room(systems.fitter.pool)
	local look=opts.look
	if look==nil then
		look=0
		for i=1,#key do look=(look*31+string.byte(key,i))%looks end
	end
	local actor
	for offset=0,if opts.look~=nil then 0 else bodies-1 do
		local candidate=(look+offset)%looks
		actor=systems.fitter:allocate({pos=opts.pos,yaw=opts.yaw or 0,look=candidate},pop.residents,observers)
		if actor then look=candidate; break end
	end
	if not actor then return nil,"no fitted slot available for this look" end
	local p={name=opts.name or key,pos=opts.pos,yaw=opts.yaw or 0,look=look,actor=actor,
		pose=opts.pose or "idle",story=true,arrived=true}
	p.body=world.spawn("cube",{pos=p.pos+HIP,size=v(.6,1.7,.6),solid=false,visible=false,tag="story-walker"})
	self.people[key]=p
	systems.fleet.storyPeople[actor]=p
	humans.set(actor,{pose=p.pose})
	return p
end

function Cast:car(key,opts)
	if duplicate(self,key) then return nil,"duplicate cast name" end
	local car=mobility.add_car(getmetatable(self).fleet,opts.pos,opts.yaw or 0,opts.paint)
	car.name,car.arrived=opts.name or key,true
	self.cars[key]=car
	return car
end

function Cast:position(key)
	local actor=self.people[key] or self.cars[key]
	return if actor then actor.pos else nil
end

function Cast:arrived(key)
	local actor=self.people[key] or self.cars[key]
	return actor~=nil and actor.arrived==true
end

function Cast:walk(key,points,speed)
	local p=self.people[key]
	if not p or p.rideId or not points or #points==0 then return false end
	p.route,p.leg,p.pace,p.arrived=points,1,math.clamp(speed or 1.5,.15,6),false
	return true
end

function Cast:pose(key,pose)
	local p=self.people[key]
	if not p or p.rideId or p.impact then return false end
	p.route,p.pose,p.arrived=nil,pose,true
	humans.set(p.actor,{pose=pose})
	return true
end

local function resolveCar(self,key)
	if type(key)=="table" then return key end
	if type(key)=="number" then return getmetatable(self).fleet.vehicles[key] end
	return self.cars[key]
end

-- Round the nearer bumper when a passenger approaches the wrong side.
-- Both corner waypoints clear the car plus a person's standing radius.
function Cast:approach(key,vehicle,seat,speed)
	local p,car=self.people[key],resolveCar(self,vehicle)
	if not p or not car or car.kind~="car" then return false end
	local side=if seat=="driver" then -1 else 1
	local right,forward=util.right(car.yaw),util.forward(car.yaw)
	local delta=p.pos-car.pos
	local x,z=vector.dot(delta,right),vector.dot(delta,forward)
	local points={}
	if x*side<1.4 then
		local bumper=car.pos+forward*(if z>=0 then 2.85 else -2.85)
		if math.abs(z)<2.85 then table.insert(points,bumper-right*(1.58*side)) end
		table.insert(points,bumper+right*(1.58*side))
	end
	table.insert(points,car.pos+right*(1.55*side))
	return self:walk(key,points,speed)
end

local function seatPerson(p,car)
	p.pos,p.yaw=models.seat(car,if p.seat=="driver" then -1 else 1)
	p.pose="sit"
	world.place(p.body,p.pos+HIP,p.yaw)
	humans.set(p.actor,{pos=p.pos,yaw=p.yaw,pose="sit",visible=true})
end

function Cast:ride(key,vehicle,seat)
	local p,car=self.people[key],resolveCar(self,vehicle)
	seat=seat or "passenger"
	if not p or not car or car.kind~="car" or p.impact or (seat~="driver" and seat~="passenger") then return false end
	if p.rideId then return p.rideId==car.index and p.seat==seat end
	if seat=="driver" and (car.owner or car.storyDriver) or seat=="passenger" and car.storyPassenger then return false end
	if seat=="driver" then car.storyDriver=p.actor else car.storyPassenger=p.actor end
	p.rideId,p.seat,p.route,p.arrived=car.index,seat,nil,true
	seatPerson(p,car)
	return true
end

local function releaseSeat(p,car)
	if car then
		if car.storyDriver==p.actor then car.storyDriver=nil; car.auto=nil end
		if car.storyPassenger==p.actor then car.storyPassenger=nil end
	end
	p.rideId,p.seat=nil,nil
end

function Cast:dismount(key)
	local p=self.people[key]
	if not p or not p.rideId then return false end
	local fleet=getmetatable(self).fleet
	local car=fleet.vehicles[p.rideId]
	local pos=if car then mobility.exitAt(fleet,car,if p.seat=="driver" then -1 else 1) else p.pos
	if not pos then return false end
	releaseSeat(p,car)
	p.pos,p.pose,p.arrived,p.previousFeet=pos,"idle",true,nil
	world.place(p.body,pos+HIP,p.yaw)
	humans.set(p.actor,{pos=pos,yaw=p.yaw,pose="idle"})
	return true
end

function Cast:drive(key,points,speed)
	local car=resolveCar(self,key)
	if not car or car.owner or not points or #points==0 then return false end
	local leg,pace=1,math.clamp(speed or 6,1,18)
	car.arrived=false
	car.auto=function(vehicle)
		local delta=util.flat(points[leg]-vehicle.pos)
		local dist=vector.magnitude(delta)
		if leg<#points and dist<math.max(3,math.abs(vehicle.speed)*.55) then
			leg+=1
			delta=util.flat(points[leg]-vehicle.pos)
			dist=vector.magnitude(delta)
		end
		local target=if leg==#points then math.min(pace,math.max(0,dist-1)*.9) else pace
		if leg==#points and dist<1.7 and math.abs(vehicle.speed)<.65 then
			vehicle.auto,vehicle.arrived=nil,true
			return nil
		end
		local turn=util.angle_diff(util.yaw_to(delta),vehicle.yaw)
		return {throttle=math.clamp((target-vehicle.speed)*.25,0,.75),
			brake=math.clamp((vehicle.speed-target)*.5,0,1),steer=math.clamp(-turn*1.7,-1,1)}
	end
	return true
end

function Cast:hit(key,push)
	local p=self.people[key]
	if not p then return false end
	p.wasHit,p.route,p.arrived=true,nil,false
	if p.rideId then
		if not self:dismount(key) then releaseSeat(p,getmetatable(self).fleet.vehicles[p.rideId]) end
	end
	if p.impact and p.impact.stage=="down" then return true end
	if not humans.ragdoll(p.actor,push or v(0,1,-2)) then return false end
	p.impact={stage="down",elapsed=0,home=p.pos}
	p.externalPose=true
	return true
end

function Cast:wasHit(key)
	local p=self.people[key]
	return p~=nil and p.wasHit==true
end

function Cast:tick(dt)
	local fleet=getmetatable(self).fleet
	for _,p in self.people do
		if p.impact then continue end
		if p.rideId then
			local car=fleet.vehicles[p.rideId]
			if car then seatPerson(p,car) end
		elseif p.route then
			local delta=p.route[p.leg]-p.pos
			local distance=vector.magnitude(delta)
			-- Ground varies by curb and floor thickness. Reach the authored
			-- horizontal waypoint even when collision holds feet a little higher.
			if vector.magnitude(util.flat(delta))<.080 and math.abs(delta.y)<.45 then
				p.leg+=1
				if p.leg>#p.route then p.route,p.pose,p.arrived=nil,"idle",true end
			else
				p.yaw=math.atan2(delta.x,delta.z)
				local velocity=delta*math.min(p.pace/distance,1/dt)
				local nextPos=p.pos+velocity*dt
				if nextPos.x >= -city.BOUND or not cape.remoteTrunk(fleet.layout.cape,nextPos,.32) then
					world.move(p.body,velocity,dt,QUERY)
				end
				p.pos,p.pose=world.get(p.body).pos-HIP,"walk"
			end
			humans.set(p.actor,{pos=p.pos,yaw=p.yaw,pose=p.pose})
		end
	end
end

function Cast:remove(key)
	local fleet=getmetatable(self).fleet
	local p=self.people[key]
	if p then
		if p.rideId then releaseSeat(p,fleet.vehicles[p.rideId]) end
		humans.remove(p.actor)
		world.remove(p.body)
		fleet.storyPeople[p.actor],self.people[key]=nil,nil
		return true
	end
	local car=self.cars[key]
	if not car then return false end
	for name,rider in self.people do
		if rider.rideId==car.index and not self:dismount(name) then return false end
	end
	car.auto=nil
	mobility.release_car(fleet,car)
	self.cars[key]=nil
	return true
end

function Cast:clear()
	for key in self.people do self:remove(key) end
	for key in self.cars do self:remove(key) end
end

return Cast
