-- A four-zone crush profile for cars facing local -Z, with distances in model
-- metres. State contains only plain numbers and survives a Luau hot reload.
local damage = {}
local v = vector.create

function damage.profile(halfWidth, front, rear, sideHinge, frontHinge, rearHinge)
	assert(halfWidth>sideHinge and sideHinge>0 and front>frontHinge and rear>rearHinge and math.min(frontHinge,rearHinge)>0)
	return {halfWidth=halfWidth,front=front,rear=rear,sideHinge=sideHinge,frontHinge=frontHinge,rearHinge=rearHinge}
end

function damage.new()
	return {zones={front=0,rear=0,left=0,right=0},total=0}
end

function damage.hit(state,profile,p,amount)
	if not (amount>0) then return false end
	local x=math.abs(p.x)/profile.halfWidth
	local z=math.abs(p.z)/(if p.z<0 then profile.front else profile.rear)
	local endZone=if p.z<0 then "front" else "rear"
	local sideZone=if p.x<0 then "left" else "right"
	-- Doors, bumpers and corners are continuous in body space. A corner
	-- transfers some crush to its neighbour instead of jumping between them.
	local endWeight=math.clamp(.5+2.5*(z-x),0,1)
	local zones=state.zones
	local oldEnd,oldSide,oldTotal=zones[endZone],zones[sideZone],state.total
	amount=math.min(amount,1)
	zones[endZone]=math.min(1,oldEnd+amount*endWeight)
	zones[sideZone]=math.min(1,oldSide+amount*(1-endWeight))
	state.total=math.min(1,oldTotal+amount*.55)
	state.disabled=state.total>=1
	return oldEnd~=zones[endZone] or oldSide~=zones[sideZone] or oldTotal~=state.total
end

-- Each cell of a 3x3 grid scales about its shared inner face. Positions on
-- either side of a seam map to precisely the same point at every damage level.
function damage.cellPose(profile,zones,column,row)
	local sx=if column==0 then 1-.67*zones.left elseif column==2 then 1-.67*zones.right else 1
	local sz=if row==0 then 1-.69*zones.front elseif row==2 then 1-.60*zones.rear else 1
	local hx=(column-1)*profile.sideHinge
	local hz=if row==0 then -profile.frontHinge elseif row==2 then profile.rearHinge else 0
	return v(hx*(1-sx),0,hz*(1-sz)),v(sx,1,sz)
end

function damage.controls(state,input,speed)
	if not input or not state or state.total<=0 then return input end
	local out=table.clone(input)
	local z=state.zones
	local power=if state.disabled then 0 else math.max(.12,1-.48*z.front-.35*state.total)
	out.throttle=(out.throttle or 0)*power
	-- The road solver uses the brake as reverse when stopped. Keep service
	-- braking at speed, and reduce reverse propulsion with the same power.
	if speed<.2 then out.brake=(out.brake or 0)*power end
	out.steer=math.clamp((out.steer or 0)*(1-.28*math.max(z.left,z.right))+.13*(z.right-z.left)*math.clamp(math.abs(speed)/8,0,1),-1,1)
	return out
end

return damage
