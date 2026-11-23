-- Road contacts feed the same bounded combat effects as other hits. Only check
-- tire geometry for a downed fitted body near a moving car.
local models=require("vehicles")
local road={}
local v=vector.create
local MIN_BUMPER,LETHAL_BUMPER=8,18
local MIN_WHEEL,GIB_WHEEL=4,12
local TIRE_RADIUS,TIRE_HALF_WIDTH=.331,.118
local BODY_RADIUS=.16

function road.begin(pop)
	pop.roadHits=pop.roadHits or {}
	table.clear(pop.roadHits)
	pop.tireTracks=pop.tireTracks or {}
end

local function report(pop,kind,p,actor,seat,resident,corpse,car,pos,speed,lethal,gib)
	table.insert(pop.roadHits,{kind=kind,person=p,actor=actor,seat=seat,resident=resident,corpse=corpse,
		owner=car.owner or 0,vehicle=car.id,pos=pos,velocity=car.velocity,speed=speed,lethal=lethal,gib=gib})
end

function road.bumper(pop,p,actor,seat,resident,car,speed)
	if not car.native or speed<MIN_BUMPER then return end
	local point=p.pos+v(0,.53,0)-vector.normalize(car.velocity)*.17
	report(pop,"bumper",p,actor,seat,resident,nil,car,point,speed,not seat and speed>=LETHAL_BUMPER,false)
end

-- Intersect the bone's motion relative to the wheel with a finite tire
-- cylinder. Clip across the tread first, then test the circular side view.
local function sweep(car,old,point,wheel,dt)
	local a,b=old-car.prevPos,point-car.pos
	local ax,bx=vector.dot(a,car.oldRight)-wheel.x,vector.dot(b,car.right)-wheel.x
	local dx=bx-ax
	local lo,hi=0,1
	local width=TIRE_HALF_WIDTH+BODY_RADIUS
	if math.abs(dx)<.00001 then
		if math.abs(ax)>width then return nil end
	else
		local t0,t1=(-width-ax)/dx,(width-ax)/dx
		lo,hi=math.max(lo,math.min(t0,t1)),math.min(hi,math.max(t0,t1))
		if lo>hi then return nil end
	end
	local az=vector.dot(a,car.oldForward)+wheel.z
	local dz=vector.dot(b,car.forward)+wheel.z-az
	-- The chassis-query center is .80 m above the road; wheel centers are .33 m.
	local ay,dy=a.y+.47,b.y-a.y
	local d2=dz*dz+dy*dy
	local t=if d2>.000001 then math.clamp(-(az*dz+ay*dy)/d2,lo,hi) else lo
	if (az+dz*t)^2+(ay+dy*t)^2>(TIRE_RADIUS+BODY_RADIUS)^2 then return nil end
	local speed=math.min(car.speed,math.sqrt(dx*dx+dz*dz)/dt)
	if speed>=MIN_WHEEL then return old+(point-old)*t,speed end
	return nil
end

function road.prone(pop,p,actor,seat,resident,corpse,nearby,dt)
	local tick=game.tick()
	local track=pop.tireTracks[actor]
	-- No nearby cars also ends a pass; a car can return on a later approach.
	if not nearby or #nearby==0 then return end
	local hips=humans.joint(actor,"hips")
	local chest=humans.joint(actor,"chest")
	if not hips or not chest then return end
	if not track or track.tick<tick-1 then
		track={hips=hips,chest=chest,passes={}}
		pop.tireTracks[actor]=track
	end
	for _,car in nearby do
		if not car.native then continue end
		local delta=chest-car.pos
		local overlaps=math.abs(vector.dot(delta,car.right))<1.85 and math.abs(vector.dot(delta,car.forward))<3.10
		if track.passes[car.id] then
			if overlaps then track.passes[car.id]=tick; continue end
			track.passes[car.id]=nil
		end
		-- A standing pedestrian, a passenger on the bonnet or a car driving
		-- across an overpass must never count as a tire crossing a fallen body.
		if car.speed<MIN_WHEEL or math.max(hips.y,chest.y)>car.pos.y-.14 then continue end
		local oldHips,oldChest=track.hips,track.chest
		for i,wheel in models.wheelOffsets do
			if car.missingWheels and bit32.btest(car.missingWheels,bit32.lshift(1,i-1)) then continue end
			local pos,speed=sweep(car,oldHips,hips,wheel,dt)
			if not pos then pos,speed=sweep(car,oldChest,chest,wheel,dt) end
			if not pos then pos,speed=sweep(car,(oldHips+oldChest)*.5,(hips+chest)*.5,wheel,dt) end
			if pos then
				report(pop,"wheel",p,actor,seat,resident,corpse,car,pos,speed,not seat,corpse~=nil and speed>=GIB_WHEEL)
				track.passes[car.id]=tick
				break
			end
		end
	end
	for car,last in track.passes do if last<tick then track.passes[car]=nil end end
	track.hips,track.chest,track.tick=hips,chest,tick
end

function road.finish(pop)
	local tick=game.tick()
	for actor,track in pop.tireTracks do if track.tick~=tick then pop.tireTracks[actor]=nil end end
end

return road
