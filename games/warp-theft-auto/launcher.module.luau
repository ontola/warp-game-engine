-- Each grenade is a fixed-tick arcade projectile shared by all local seats.
local launcher={blastRadius=17, effectRadius=5.5}
local v=vector.create
local speed,cooldown=35,.95
local gravity=v(0,-21,0)
local shell={color="#785444",roughness=.65}
local fuse={color="#ffba62",emissive=.40,roughness=1}

function launcher.warm()
	world.warm("sphere",shell)
	world.warm("sphere",fuse)
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
	table.insert(shots,{owner=id,entity=entity,pos=start,dir=dir,velocity=velocity,remaining=flight})
	player.shotFlash,player.launcherCooldown=.12,cooldown
	sound.play("arcade_launcher",{at=start,db=-6})
	return true
end

function launcher.advance(state,dt,trace,detonate)
	local shots=state.combat.projectiles
	local i=1
	while i<=#shots do
		local shot=shots[i]
		local step=math.min(dt,shot.remaining)
		local delta=(shot.velocity+gravity*(step*.5))*step
		local distance=vector.magnitude(delta)
		local dir=if distance>.00001 then delta/distance else shot.dir
		-- The aimed point lies exactly on a surface. Accumulated integration
		-- roundoff can leave the last sweep a fraction of a millimetre short.
		local reach=if step==shot.remaining then distance+.002 else distance
		local target,point,solid=trace(state,shot.pos,dir,shot.pos,reach,true)
		shot.velocity+=gravity*step
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
