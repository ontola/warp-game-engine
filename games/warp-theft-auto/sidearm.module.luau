-- The arcade pistol is shared by free-roam players and scripted contracts. A frame follows the
-- animated wrist; its children keep the authored offsets under pitch and yaw.
local sidearm={range=75}
local util=require("kit/util")
local v=vector.create
local SCALE=.68
local dark={color="#34414a",roughness=.74}
local slide={color="#a8babc",roughness=.57,metal=.18}
local trim={color="#dc914e",roughness=.66}
local glow={color="#ffb84e",emissive=2.6,roughness=1}
local streak={color="#ffd988",emissive=1.4,roughness=1}
local cover={tag="city"}

function sidearm.warm()
	for _,look in {dark,slide,trim} do world.warm("cube",look) end
	world.warm("sphere",glow)
	world.warm("cube",streak)
end

local function part(root,tag,look,pos,size,pitch)
	local o=table.clone(look)
	o.tag,o.pos,o.size,o.solid=tag,pos*SCALE,size*SCALE,false
	if pitch then o.rot=v(pitch,0,0) end
	return world.spawn("cube",o,root)
end

local function make(hand,rotation)
	local root=world.spawn("cube",{tag="story-sidearm",pos=hand,rot=rotation,visible=false,solid=false})
	part(root,"sidearm-grip",dark,v(0,-.017,.025),v(.076,.167,.082),-.27)
	part(root,"sidearm-frame",dark,v(0,.042,.106),v(.083,.058,.236))
	part(root,"sidearm-slide",slide,v(0,.084,.119),v(.086,.066,.293))
	part(root,"sidearm-trim",trim,v(0,.058,.218),v(.089,.020,.067))
	part(root,"sidearm-sight",dark,v(0,.124,.230),v(.021,.021,.027))
	return root
end

function sidearm.remove(ctx)
	if ctx.prop then world.remove(ctx.prop); ctx.prop=nil end
	ctx.tip,ctx.turn=nil,nil
end

function sidearm.clearFlash(ctx)
	if ctx.flash then world.remove(ctx.flash); ctx.flash=nil end
	if ctx.trace then world.remove(ctx.trace); ctx.trace=nil end
end

function sidearm.follow(ctx,avatar,seat,ray)
	local wrist=avatar and seat.mode=="walk" and not seat.impact and humans.joint(avatar,"hand_r")
	if not wrist then sidearm.remove(ctx); return end
	local yaw=seat.yaw
	local down=.78
	local dir=ray or v(-math.sin(yaw)*math.cos(down),-math.sin(down),-math.cos(yaw)*math.cos(down))
	-- The local slide extends toward +Z. YXZ is the world entity rotation order.
	local heading=math.atan2(dir.x,dir.z)
	local rotation=v(-math.asin(math.clamp(dir.y,-1,1)),heading,0)
	local right=v(math.cos(heading),0,-math.sin(heading))
	local up=vector.cross(dir,right)
	local hand=wrist+dir*.048-up*.015
	if not ctx.prop then ctx.prop=make(hand,rotation)
	else world.set(ctx.prop,{pos=hand,rot=rotation}) end
	ctx.tip,ctx.turn=hand+(dir*.292+up*.084)*SCALE,rotation
end

local function fire(ctx,point)
	if not ctx.tip then return end
	sidearm.clearFlash(ctx)
	local o=table.clone(glow)
	o.tag,o.pos,o.rot,o.size,o.solid="sidearm-flash",ctx.tip,ctx.turn,v(.108,.108,.22)*SCALE,false
	ctx.flash=world.spawn("sphere",o)
	if not point then return end
	local delta=point-ctx.tip
	local length=vector.magnitude(delta)
	if length<.15 then return end
	local dir=delta/length
	-- A brief world-space streak is visible to both the shooter and other seats.
	local trace=table.clone(streak)
	trace.pos=ctx.tip+delta*.5
	trace.rot=v(-math.asin(math.clamp(dir.y,-1,1)),math.atan2(dir.x,dir.z),0)
	trace.tag,trace.size,trace.solid="sidearm-trace",v(.025,.025,length),false
	ctx.trace=world.spawn("cube",trace)
end

-- Both modes use the same seat camera, capsule and city visibility query.
function sidearm.ray(id)
	local cam=camera.get(id)
	local axis=cam and cam.target-cam.pos
	if not axis or vector.magnitude(axis)<.01 then return nil,nil end
	return cam.pos,vector.normalize(axis)
end

function sidearm.hipRay(seat)
	-- The idle camera looks down at its own avatar. Fire along the view heading
	-- from shoulder height so an ordinary click can reach someone across a street.
	local ray=util.forward(seat.view)*math.cos(seat.pitch)+v(0,math.sin(seat.pitch),0)
	return seat.pos+v(0,1.25,0),ray
end

local function sphereEntry(offset,ray,radius)
	local along=-vector.dot(offset,ray)
	local dist2=vector.dot(offset,offset)
	if dist2<=radius*radius then return 0 end
	local disc=along*along-(dist2-radius*radius)
	if along<0 or disc<0 then return math.huge end
	return along-math.sqrt(disc)
end

-- First contact with a vertical capsule. Compare all candidates before the one
-- cover query. The impact point is on the near surface, including at steep angles.
function sidearm.intersect(origin,ray,feet,pos,range,height)
	local delta=feet-pos
	if vector.dot(delta,delta)>range*range then return nil end
	height=height or 1.94
	local r=math.min(.42,height*.5)
	local bottom,top=r,height-r
	local o=origin-pos
	local inside=o-v(0,math.clamp(o.y,bottom,top),0)
	if vector.dot(inside,inside)<=r*r then return 0,origin end
	local axis=ray.x*ray.x+ray.z*ray.z
	local first=math.huge
	if axis>.00001 then
		local along=-(o.x*ray.x+o.z*ray.z)
		local disc=along*along-axis*(o.x*o.x+o.z*o.z-r*r)
		-- Missing the infinite cylinder rules out both rounded ends too.
		if disc<0 or (along<0 and o.x*o.x+o.z*o.z>r*r) then return nil end
		local t=(along-math.sqrt(disc))/axis
		local y=o.y+t*ray.y
		if t>=0 and y>=bottom and y<=top then first=t end
	end
	first=math.min(first,sphereEntry(o-v(0,bottom,0),ray,r),sphereEntry(o-v(0,top,0),ray,r))
	if first==math.huge then return nil end
	return first,origin+ray*first
end

function sidearm.clear(origin,ray,along)
	return along<=.001 or world.raycast(origin,ray,along-.001,cover)==nil
end

function sidearm.trigger(input)
	local pressed=input.pressed
	return pressed and (pressed.fire or pressed.south)
end

function sidearm.confirmHit(display,point)
	display.hitPoint,display.hitFlash=point,.22
end

function sidearm.discharge(ctx,display,seat,input,point,origin,ray)
	if not sidearm.trigger(input) or (seat.pistolCooldown or 0)>0 then return false end
	if not point and ray then
		local wall=world.raycast(origin,ray,sidearm.range,cover)
		point=if wall then wall.point else origin+ray*sidearm.range
	end
	display.shotFlash,seat.pistolCooldown=.10,.28
	fire(ctx,point)
	sound.play("arcade_pistol",{at=seat.pos,db=-7})
	return true
end

function sidearm.tickFlash(ctx,display,dt)
	display.shotFlash=math.max(0,display.shotFlash-dt)
	if display.shotFlash==0 then sidearm.clearFlash(ctx) end
	if display.hitPoint then
		display.hitFlash=math.max(0,display.hitFlash-dt)
		if display.hitFlash==0 then display.hitPoint=nil end
	end
end

return sidearm
