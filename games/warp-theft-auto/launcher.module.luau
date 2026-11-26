-- Each grenade is a fixed-tick arcade projectile shared by all local seats.
local launcher={blastRadius=17, effectRadius=5.5}
local v=vector.create
local speed,cooldown=35,.95
local gravity=v(0,-21,0)
local shell={color="#785444",roughness=.65}
local fuse={color="#ffba62",emissive=.40,roughness=1}

-- Growing Guns' finned silhouette, contrasting nose band and hot exhaust,
-- assembled from the same small meshes the city already has resident.
local casing={color="#a7ad98",roughness=.58}
local fin={color="#414844",roughness=.66}
local stripe={color="#ba5429",roughness=.6}
local flame={color="#ff822f",emissive=1.8,roughness=1}
local throat={color="#ffdf92",emissive=3.0,roughness=1}
local rocketParts={
	{"sphere",stripe,v(0,.26,0),v(.156,.068,.156),"rocket-band"},
	{"cube",fin,v(0,-.32,0),v(.31,.16,.03),"rocket-fin"},
	{"cube",fin,v(0,-.32,0),v(.03,.16,.31),"rocket-fin"},
	{"sphere",flame,v(0,-.65,0),v(.116,.49,.116),"rocket-exhaust"},
	{"sphere",throat,v(0,-.46,0),v(.13,.17,.13),"rocket-core"},
}
local smokeSpacing=1.65

function launcher.warm()
	world.warm("sphere",shell)
	world.warm("sphere",fuse)
	world.warm("capsule",casing)
	for _,part in rocketParts do world.warm(part[1],part[2]) end
end

function launcher.fire(shots,id,player,seat,aimPoint,aimDir,range)
	if (player.launcherCooldown or 0)>0 then return end
	local shoulder=seat.pos+v(0,1.25,0)
	local start=player.tip or shoulder
	-- Keep the muzzle on the player's side of cover even when the held prop clips it.
	local reach=start-shoulder
	local reachLength=vector.magnitude(reach)
	if reachLength>.001 and world.raycast(shoulder,reach,reachLength) then start=shoulder end
	local toAim=aimPoint-start
	local distance=math.clamp(vector.magnitude(toAim),.1,range)
	local dir=if vector.dot(toAim,aimDir)>.02 then vector.normalize(toAim) else aimDir
	local flight=distance/speed
	local velocity=dir*speed-gravity*(flight*.5)
	local o=table.clone(shell)
	o.tag,o.solid,o.pos,o.size="launcher-projectile",false,start,v(.11,.11,.17)
	local entity=world.spawn("sphere",o)
	local cap=table.clone(fuse)
	cap.tag,cap.solid,cap.pos,cap.size="launcher-fuse",false,v(0,.052,0),v(.034,.026,.04)
	world.spawn("sphere",cap,entity)
	table.insert(shots,{owner=id,source=seat.pos,entity=entity,pos=start,dir=dir,velocity=velocity,remaining=flight})
	player.shotFlash,player.launcherCooldown=.12,cooldown
	sound.play("arcade_launcher",{at=start,db=-6})
	return true
end

-- Like the Growing Guns air strike, the motor eases up along a straight
-- arcade path. The same swept segment drives contacts and the smoke wake.
function launcher.rocket(shots,owner,aircraft,pos,dir)
	dir=vector.normalize(dir)
	local body=table.clone(casing)
	body.pos,body.size,body.tag,body.solid=pos,v(.15,.43,.15),"apache-rocket",false
	body.rot=v(math.acos(math.clamp(dir.y,-1,1)),math.atan2(dir.x,dir.z),0)
	local entity=world.spawn("capsule",body)
	for _,part in rocketParts do
		local opts=table.clone(part[2])
		opts.pos,opts.size,opts.tag,opts.solid=part[3],part[4],part[5],false
		world.spawn(part[1],opts,entity)
	end
	table.insert(shots,{owner=owner,source=pos,aircraft=aircraft,entity=entity,pos=pos,
		dir=dir,velocity=dir*36,motor=dir*15,smokeIn=smokeSpacing,remaining=2.4})
	sound.play("arcade_launcher",{at=pos,db=-5})
end

local function rocketWake(shot,point,dir)
	local distance=vector.magnitude(point-shot.pos)
	local nextPuff=shot.smokeIn or smokeSpacing
	while nextPuff<=distance do
		-- Fix each puff in world space behind the attached exhaust so the
		-- trail stays in the air when its rocket is removed on contact.
		fx.exhaust_smoke(shot.pos+dir*(nextPuff-.87),.38)
		nextPuff+=smokeSpacing
	end
	shot.smokeIn=nextPuff-distance
end

function launcher.advance(state,dt,trace,detonate)
	local shots=state.combat.projectiles
	local i=1
	while i<=#shots do
		local shot=shots[i]
		local step=math.min(dt,shot.remaining)
		local acceleration=if shot.aircraft then shot.motor or v(0,0,0) else gravity
		local delta=(shot.velocity+acceleration*(step*.5))*step
		local distance=vector.magnitude(delta)
		local dir=if distance>.00001 then delta/distance else shot.dir
		-- The aimed point lies exactly on a surface. Accumulated integration
		-- roundoff can leave the last sweep a fraction of a millimetre short.
		local reach=if step==shot.remaining then distance+.002 else distance
		local target,point,solid=trace(state,shot,dir,reach)
		if shot.aircraft then rocketWake(shot,point,dir) end
		shot.velocity+=acceleration*step
		shot.remaining-=step
		shot.dir=dir
		if target or solid or shot.remaining<=0 then
			world.remove(shot.entity)
			table.remove(shots,i)
			local center=point+(if solid then solid.normal*.06 else v(0,0,0))
			detonate(state,shot,center,target)
		else
			shot.pos=point
			world.set(shot.entity,{pos=point})
			i+=1
		end
	end
end

return launcher
