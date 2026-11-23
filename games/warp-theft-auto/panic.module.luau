-- A panic route is sampled at an absolute time. Fitted people can move every
-- tick while the distant crowd asks for the same route a fraction ahead.
local refuge = require("refuge")
local panic = {}
local WALK_BACK = 1.65

local function distanceSq(a, b)
	local d = a-b
	return vector.dot(d,d)
end

local function onLoop(path, cursor)
	local leg = path[1 + math.floor(cursor)%4]
	return leg.a + leg.delta*(cursor%1), leg.yaw
end

local function nearestCursor(path, feet)
	local best, cursor = math.huge, 0
	for i, leg in path do
		local f = math.clamp(vector.dot(feet-leg.a,leg.delta)/vector.dot(leg.delta,leg.delta),0,1)
		local d = distanceSq(feet,leg.a+leg.delta*f)
		if d<best then best,cursor=d,i-1+f end
	end
	return cursor%4
end

-- Keep both possible escape directions on the resident's own sidewalk lane.
-- Stop at the safest corner if continued fire would send them around the
-- block and back towards the shooter.
local function sidewalkFlight(pop,c,feet,source)
	local pathId = c.path
	local path = pop.paths[pathId]
	local start = nearestCursor(path,feet)
	local ahead, behind = onLoop(path,start+.15), onLoop(path,start-.15)
	if distanceSq(behind,source)>distanceSq(ahead,source) then
		pathId += if (pathId-1)%4<2 then 2 else -2
		path,start = pop.paths[pathId],(-start)%4
	end
	local length = vector.magnitude(path[1].delta)
	local best,limit = distanceSq(feet,source),0
	for i,leg in path do
		local d, forward = distanceSq(leg.a,source),(i-1-start)%4
		if d>best or (d==best and forward<limit) then best,limit=d,forward end
	end
	return {path=pathId,cursor=start,legLength=length,limit=limit*length}
end

local function walkingDistance(p,time)
	local outward = math.clamp(p.offset + p.direction*p.speed*(math.min(time,p.quiet)-p.started),0,p.limit)
	return math.max(0,outward - math.max(0,time-p.quiet)*WALK_BACK)
end

local function shelterFlight(pop,c,feet,source,id)
	local route = refuge.plan(pop,c,feet,source,id)
	local legs, length, prev = {},0,feet
	for _,point in route do
		local d = vector.magnitude(point-prev)
		if d>.01 then
			table.insert(legs,{a=prev,delta=point-prev,start=length,length=d,yaw=math.atan2(point.x-prev.x,point.z-prev.z)})
			length+=d
			prev=point
		end
	end
	return {home=feet,homeYaw=c.yaw,homeTime=pop.elapsed+c.phase,legs=legs,limit=length,refuge=prev}
end

function panic.hear(pop,c,id,feet,source,delay)
	local now,p = pop.elapsed,c.panic
	local variation = (id*37+pop.seed)%101/100
	local quiet = now+5+variation*3
	if p and now<p.quiet then
		local nearbySource = distanceSq(source,p.source)<6*6
		local stronger = distanceSq(feet,source)+2*2<distanceSq(feet,p.source)
		if nearbySource or (now-p.started<.65 and not stronger) then
			p.quiet=quiet
			return
		end
	end
	local start=now
	if c.path then
		-- Distant people finish the interpolation already submitted to the GPU.
		-- Their next snapshot starts fleeing from that same endpoint.
		if delay and (not p or now<p.started) then
			start,feet=now+delay,c.nextPos or feet
		end
		p=sidewalkFlight(pop,c,feet,source)
		p.offset,p.direction=0,1
	else
		if not p then p=shelterFlight(pop,c,feet,source,id) end
		p.offset=if p.started then walkingDistance(p,now) else 0
		p.direction=if p.offset>.05 and distanceSq(p.home,source)>distanceSq(p.refuge,source) then -1 else 1
	end
	p.speed,p.started,p.source,p.quiet=4.3+1.5*variation,start,source,quiet
	c.panic=p
end

-- Return the normal routine's clock when it resumes. A future LOD sample
-- may cross that instant before the fitted actor does; only commit on 'now'.
function panic.sample(pop,c,time,preview)
	local p=c.panic
	local normalTime
	if time<p.started then return time+c.phase end
	if p.cursor then
		local distance=math.min(p.limit,p.speed*(math.min(time,p.quiet)-p.started))
		local cursor=(p.cursor+distance/p.legLength)%4
		if time<p.quiet then
			c.pos,c.yaw=onLoop(pop.paths[p.path],cursor)
			c.pose=if distance<p.limit then "walk" else "idle"
			return nil
		end
		normalTime=math.floor(cursor)*c.segment+(cursor%1)*c.walkTime+time-p.quiet
	else
		local distance=walkingDistance(p,time)
		if time<p.quiet or distance>0 then
			local returning=time>p.quiet or p.direction<0
			local moving=if returning then distance>0 else distance<p.limit
			c.pos,c.yaw=p.home,p.homeYaw
			for i,leg in p.legs do
				if distance<leg.start+leg.length or i==#p.legs then
					c.pos=leg.a+leg.delta*math.clamp((distance-leg.start)/leg.length,0,1)
					c.yaw=leg.yaw+(if returning then math.pi else 0)
					break
				end
			end
			c.pose=if moving then "walk" else "idle"
			return nil
		end
		local resumed=p.quiet+walkingDistance(p,p.quiet)/WALK_BACK
		c.pos,c.yaw=p.home,p.homeYaw
		normalTime=p.homeTime+time-resumed
	end
	if not preview then
		c.phase,c.panic=normalTime-time,nil
		if p.path then c.path=p.path end
	end
	return normalTime,p.path
end

return panic
