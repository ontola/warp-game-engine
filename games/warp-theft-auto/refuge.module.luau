-- Small walkable areas shared by the people in a furnished room or park.
-- Plan once when a shot is heard; the crowd samples the resulting polyline.
local refuge = {}
local v = vector.create
local RADIUS = .30

function refuge.area(x,z,width,depth)
	return {x0=x-width/2+RADIUS, x1=x+width/2-RADIUS,
		z0=z-depth/2+RADIUS, z1=z+depth/2-RADIUS, obstacles={}}
end

function refuge.block(area,x,z,width,depth)
	local x0,x1 = x-width/2-RADIUS, x+width/2+RADIUS
	local z0,z1 = z-depth/2-RADIUS, z+depth/2+RADIUS
	if x1<=area.x0 or x0>=area.x1 or z1<=area.z0 or z0>=area.z1 then return end
	table.insert(area.obstacles,{x0=x0,x1=x1,z0=z0,z1=z1})
end

local function slab(pos,dir,lo,hi)
	if math.abs(dir)<.00001 then
		if pos<lo or pos>hi then return math.huge,-math.huge end
		return -math.huge,math.huge
	end
	local a,b=(lo-pos)/dir,(hi-pos)/dir
	return math.min(a,b),math.max(a,b)
end

local function clearance(area,pos,dir,budget)
	local _,endX=slab(pos.x,dir.x,area.x0,area.x1)
	local _,endZ=slab(pos.z,dir.z,area.z0,area.z1)
	local reach=math.min(budget,endX-.025,endZ-.025)
	local standUp=0
	for _,b in area.obstacles do
		local x0,x1=slab(pos.x,dir.x,b.x0,b.x1)
		local z0,z1=slab(pos.z,dir.z,b.z0,b.z1)
		local enter,leave=math.max(x0,z0),math.min(x1,z1)
		if leave<0 or enter>leave then continue end
		if enter<0 then
			-- A seated activity starts inside its own chair. Take a short
			-- step off its edge; never run down the length of a sofa.
			local dx=pos.x-math.clamp(pos.x,b.x0+RADIUS,b.x1-RADIUS)
			local dz=pos.z-math.clamp(pos.z,b.z0+RADIUS,b.z1-RADIUS)
			if dx*dir.x+dz*dir.z<-.001 then return 0 end
			standUp=math.max(standUp,leave)
		else
			reach=math.min(reach,enter-.025)
		end
	end
	return if standUp>1 or reach<=standUp then 0 else reach
end

local turns={0,1,-1,2,-2,3,-3,4,-4,5,-5,6}

-- Old live sessions predate the shared floor plan. Read nearby colliders once
-- on their first alarm and keep those residents inside their immediate area.
local function recoverArea(c,from)
	local area=refuge.area(from.x,from.z,17,17)
	local function collect(tag,all)
		for _,id in world.overlap(from+v(0,.85,0),13,{tag=tag,all=all}) do
			local e=world.get(id)
			if not all and not e.solid then continue end
			local p,size=e.world_pos,e.size
			if p.y-size.y/2>=from.y+1.85 or p.y+size.y/2<=from.y+.12 then continue end
			local s,co=math.abs(math.sin(e.rot.y)),math.abs(math.cos(e.rot.y))
			refuge.block(area,p.x,p.z,size.x*co+size.z*s,size.x*s+size.z*co)
		end
	end
	collect("city",false)
	if not c.building then collect("scenery",true) end
	c.refuge=area
	return area
end

function refuge.plan(pop,c,from,source,id)
	local area=c.refuge or recoverArea(c,from)
	local points, pos, previous = {}, from, nil
	local budget=13+(id%5)
	local spread=((id*37+(pop.seed or 0))%17-8)*math.pi/180
	for _=1,3 do
		local away=pos-source
		local heading=math.atan2(away.x,away.z)+spread
		local best, bestDir, bestReach, bestScore=nil,nil,0,-math.huge
		-- Include the exact aisle directions: some chairs sit close enough
		-- to their desk that the first step must be straight sideways.
		for i=1,#turns+4 do
			local angle=if i<=#turns then heading+turns[i]*math.pi/6 else (i-#turns)*math.pi/2
			local dir=v(math.sin(angle),0,math.cos(angle))
			if previous and vector.dot(previous,dir)<-.12 then continue end
			local reach=clearance(area,pos,dir,budget)
			if reach<.45 then continue end
			local point=pos+dir*reach
			local score=vector.magnitude(point-source)+reach*.15
			if score>bestScore then best,bestDir,bestReach,bestScore=point,dir,reach,score end
		end
		if not best or (previous and bestScore<vector.magnitude(pos-source)) then break end
		table.insert(points,best)
		pos,previous,budget=best,bestDir,budget-bestReach
		if budget<.45 then break end
	end
	return points
end

return refuge
