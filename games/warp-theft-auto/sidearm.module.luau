-- The arcade pistol is shared by free-roam players and scripted contracts. A frame follows the
-- animated wrist; its children keep the authored offsets under pitch and yaw.
local sidearm={range=75}
local util=require("kit/util")
local launcherProp=require("launcher_prop")
local growingProp=require("growing_gun_prop")
local catalog=require("weapon_catalog")
local v=vector.create
local SCALE=.68
local dark={color="#34414a",roughness=.74}
local slide={color="#a8babc",roughness=.57,metal=.18}
local trim={color="#dc914e",roughness=.66}
local glow={color="#ffb84e",emissive=2.6,roughness=1}
local streak={color="#ffd988",emissive=1.4,roughness=1}
local cover={tag="city"}

function sidearm.warm()
	launcherProp.warm()
	growingProp.warm()
	for _,id in catalog.order do
		local color=catalog.get(id).color
		if color then world.warm("cube",{color=color,emissive=streak.emissive,roughness=streak.roughness}) end
	end
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
	ctx.tip,ctx.turn,ctx.propWeapon,ctx.propSpec,ctx.holder=nil,nil,nil,nil,nil
end

function sidearm.clearFlash(ctx)
	if ctx.flash then world.remove(ctx.flash); ctx.flash=nil end
	if ctx.trace then world.remove(ctx.trace); ctx.trace=nil end
	if ctx.leftTrace then world.remove(ctx.leftTrace); ctx.leftTrace=nil end
	if ctx.rightTrace then world.remove(ctx.rightTrace); ctx.rightTrace=nil end
end

-- A shot adds velocity to a damped spring. Rapid follow-up shots blend into
-- the existing kick. Keeping the shoulder up briefly gives hip fire a recovery.
function sidearm.recoil(ctx,spec)
	local kick=if spec.id=="launcher" then .22 elseif spec.id=="pistol" then .15
		else math.clamp(.058+.034*spec.damage+.011*(spec.pellets-1),.07,.24)
	ctx.kickSpeed=math.min((ctx.kickSpeed or 0)+kick*50,13)
	ctx.hipHold=.42
	if ctx.holder then
		local heading=ctx.turn.y
		humans.hit(ctx.holder,v(-math.sin(heading),0,-math.cos(heading))*(kick*1.3))
	end
end

function sidearm.follow(ctx,avatar,seat,ray,spec,dt,pressed)
	if not avatar or seat.mode~="walk" or seat.impact then sidearm.remove(ctx); return end
	local pistol=spec.id=="pistol"
	local launcher=spec.id=="launcher"
	local raised=seat.aiming or pressed or (ctx.hipHold or 0)>0
	ctx.hipHold=math.max(0,(ctx.hipHold or 0)-dt)
	local ready=util.damp(ctx.ready or 0,if raised then 1 else 0,if raised then 19 else 8,dt)
	ctx.ready=ready
	local targetYaw=if raised then seat.view else seat.yaw
	if raised and ray then targetYaw=math.atan2(-ray.x,-ray.z) end
	local yaw=ctx.holdYaw or seat.yaw
	ctx.holdYaw=yaw+util.angle_diff(targetYaw,yaw)*(1-math.exp(-17*dt))
	local heading=ctx.holdYaw+math.pi

	-- Exact fixed-step response of a critically damped angular spring.
	local kick,speed=ctx.kick or 0,ctx.kickSpeed or 0
	local damping=20
	local decay=math.exp(-damping*dt)
	local travel=(speed+damping*kick)*dt
	ctx.kick,ctx.kickSpeed=(kick+travel)*decay,(speed-damping*travel)*decay
	local rest=if pistol then -.83 elseif launcher then -.39 else -.47
	local look=if ray then math.asin(math.clamp(ray.y,-1,1)) else seat.pitch
	local pitch=rest+(look-rest)*ready+ctx.kick
	local grip=if pistol then v(.046+.095*ready,-.11+.12*ready,.39+.02*ready)
		elseif launcher then v(.03+.05*ready,-.19+.245*ready,.28-.06*ready)
		else v(.09+.025*ready,-.15+.13*ready,.30-.03*ready)
	grip-=v(0,0,ctx.kick*.28)
	local support=if pistol then v(.072,-.012,.045) elseif launcher then v(.083,.028,.245) else growingProp.support(spec)
	humans.set(avatar,{pose="hold",yaw=heading,grip=grip,support=support,pitch=pitch,t=ready})
	local wrist=humans.joint(avatar,"hand_r")
	if not wrist then sidearm.remove(ctx); return end
	local dir=v(math.sin(heading)*math.cos(pitch),math.sin(pitch),math.cos(heading)*math.cos(pitch))
	local rotation=v(-pitch,heading,0)
	local right=v(math.cos(heading),0,-math.sin(heading))
	local up=vector.cross(dir,right)
	local hand=wrist+dir*.048-up*.015
	-- A hot-reloaded older seat already holding a pistol has no propWeapon yet.
	local weapon=spec.id
	local growing=spec.kind=="hitscan"
	if ctx.prop and ((ctx.propWeapon or "pistol")~=weapon or (growing and ctx.propSpec~=spec)) then sidearm.remove(ctx) end
	if not ctx.prop then
		if growing then ctx.prop=growingProp.make(hand,rotation,spec)
		elseif weapon=="launcher" then ctx.prop=launcherProp.make(hand,rotation)
		else ctx.prop=make(hand,rotation) end
		ctx.propWeapon,ctx.propSpec=weapon,if growing then spec else nil
	else world.set(ctx.prop,{pos=hand,rot=rotation}) end
	if growing then ctx.tip=growingProp.tip(hand,dir,up,spec)
	elseif weapon=="launcher" then ctx.tip=launcherProp.tip(hand,dir,up)
	else ctx.tip=hand+(dir*.292+up*.084)*SCALE end
	ctx.turn,ctx.holder=rotation,avatar
end

local function streakTo(tip,point,color)
	local delta=point-tip
	local length=vector.magnitude(delta)
	if length<.15 then return nil end
	local dir=delta/length
	local trace=table.clone(streak)
	if color then trace.color=color end
	trace.pos=tip+delta*.5
	trace.rot=v(-math.asin(math.clamp(dir.y,-1,1)),math.atan2(dir.x,dir.z),0)
	trace.tag,trace.size,trace.solid="sidearm-trace",v(.025,.025,length),false
	return world.spawn("cube",trace)
end

function sidearm.flash(ctx,point,color,left,right)
	if not ctx.tip then return end
	sidearm.clearFlash(ctx)
	local o=table.clone(glow)
	o.tag,o.pos,o.rot,o.size,o.solid="sidearm-flash",ctx.tip,ctx.turn,v(.108,.108,.22)*SCALE,false
	ctx.flash=world.spawn("sphere",o)
	ctx.trace=streakTo(ctx.tip,point,color)
	if left and vector.magnitude(left-point)>.08 then ctx.leftTrace=streakTo(ctx.tip,left,color) end
	if right and right~=left and vector.magnitude(right-point)>.08 then ctx.rightTrace=streakTo(ctx.tip,right,color) end
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

function sidearm.sceneryHit(origin,ray,distance)
	return world.raycast(origin,ray,distance,cover)
end

function sidearm.trigger(input,automatic)
	local pressed=input.pressed or {}
	local held=input.held or {}
	return pressed.fire or pressed.south or (automatic and (held.fire or held.south))
end

function sidearm.confirmHit(display,point)
	display.hitPoint,display.hitFlash=point,.22
end

-- Each player's magazines survive switching guns and live edits. The
-- fixed game clock also completes a reload that ran while the gun was holstered.
function sidearm.magazine(player,spec)
	if spec.kind~="hitscan" then
		player.ammo,player.magazine,player.reloadRemaining=nil,nil,0
		return nil
	end
	local now=game.time()
	player.magazines=player.magazines or {}
	local gun=player.magazines[spec.id]
	if not gun then
		gun={ammo=spec.magazine,capacity=spec.magazine,readyAt=0}
		player.magazines[spec.id]=gun
	end
	if gun.capacity~=spec.magazine then
		local spent=gun.capacity-gun.ammo
		gun.capacity=spec.magazine
		gun.ammo=if gun.reloadAt then 0 else math.clamp(gun.capacity-spent,0,gun.capacity)
		if gun.ammo==0 and not gun.reloadAt then gun.reloadAt=now+spec.reload end
	end
	if gun.reloadAt and now+.000001>=gun.reloadAt then
		gun.readyAt=math.max(gun.readyAt,gun.reloadAt)
		gun.ammo,gun.reloadAt=gun.capacity,nil
	end
	player.ammo,player.magazine=gun.ammo,gun.capacity
	player.reloadRemaining=if gun.reloadAt then math.max(0,gun.reloadAt-now) else 0
	return gun
end

function sidearm.discharge(player,seat,spec,gun,dt)
	local now=game.time()
	if gun then
		local tick=game.tick()
		local continuous=spec.automatic and gun.lastTriggerTick==tick-1
		gun.lastTriggerTick=tick
		if gun.ammo<=0 or now+.000001<gun.readyAt then return false end
		-- Carry fractional-tick cadence only while this trigger stays held.
		gun.readyAt=(if continuous then math.max(gun.readyAt,now-dt) else now)+spec.interval
		gun.ammo-=1
		if gun.ammo==0 then gun.reloadAt=now+spec.reload end
		player.ammo=gun.ammo
		player.reloadRemaining=if gun.reloadAt then spec.reload else 0
		sound.play("growing_guns_shot",{at=seat.pos,db=-12,damage=math.max(1,spec.damage),
			accuracy=math.clamp(1-spec.spread/.06,0,1),velocity=spec.velocity})
	else
		if (seat.pistolCooldown or 0)>0 then return false end
		seat.pistolCooldown=.28
		sound.play("arcade_pistol",{at=seat.pos,db=-7})
	end
	sidearm.recoil(player,spec)
	player.shotFlash=.10/(spec.velocity or 1)
	player.shotNumber=(player.shotNumber or 0)+1
	return true
end

-- One deterministic spread sequence per seat and discharge, independent of
-- the world's random generator, target iteration order and other players.
function sidearm.pattern(axis,spread,seat,serial)
	local bits=(seat*8191+serial*104729)%2147483647
	local function sample()
		bits=bits*48271%2147483647
		return bits/2147483647
	end
	local right=vector.cross(axis,if math.abs(axis.y)<.98 then v(0,1,0) else v(1,0,0))
	right=vector.normalize(right)
	local up=vector.cross(right,axis)
	return function()
		local angle=sample()*2*math.pi
		local radius=math.sqrt(sample())*math.tan(spread)
		return vector.normalize(axis+(right*math.cos(angle)+up*math.sin(angle))*radius)
	end
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
