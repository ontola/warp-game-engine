-- Resolve each seat's weapon against shared street crowds, interior residents,
-- rivals and authored cast. Projectiles keep flying after a weapon switch.
local Cast=require("story_cast")
local crowd=require("kit/crowd")
local fx=require("combat_fx")
local explosions=require("explosions")
local impacts=require("impacts")
local corpses=require("corpses")
local sidearm=require("sidearm")
local launcher=require("launcher")
local cars=require("vehicles")
local catalog=require("weapon_catalog")
local combat={}
local v=vector.create
local routes={
	{key="rook",name="Rook",a=v(8,0,-9),b=v(8,0,-16),look=0},
	{key="vex",name="Vex",a=v(9,0,-26),b=v(9,0,-34),look=1},
}
local marker={color="#ff3654",emissive=.45}
local markerPoints={v(-.23,.16,0),v(.23,.16,0),v(0,.16,-.23),v(0,.16,.23),v(0,-.12,0)}
local markerEdges={1,5,2,5,3,5,4,5}

function combat.new(fleet,pop)
	world.warm("wire",marker)
	fx.warm()
	launcher.warm()
	local s={players={},enemies={},projectiles={},cast=Cast.new(fleet,pop,crowd.new("rival")),effects=fx.new(),crates=explosions.placeCrates()}
	for _,r in routes do s.enemies[r.key]={health=2,respawnIn=0} end
	return s
end

function combat.reload(state)
	if state.combat then
		local s=state.combat
		Cast.bind(s.cast,state.fleet,state.population,crowd.new("rival"))
		for key,enemy in s.enemies do
			if enemy.health<=0 and s.cast.people[key] and not s.cast.people[key].dead then
				s.cast:adoptDeath(key)
				enemy.waitClear=true
			end
		end
	end
	if state.storyCast then Cast.bind(state.storyCast,state.fleet,state.population) end
end

local function removeMarker(enemy)
	if enemy.marker then world.remove(enemy.marker); enemy.marker=nil end
end

local function updateRivals(s,fleet,dt)
	s.cast:tick(dt)
	for _,route in routes do
		local enemy=s.enemies[route.key]
		local actor=s.cast.people[route.key]
		local dist=math.huge
		for _,seat in fleet.players do
			if not seat.bot then dist=math.min(dist,vector.magnitude(route.a-seat.pos)) end
		end
		enemy.respawnIn=math.max(0,enemy.respawnIn-dt)
		if enemy.health<=0 and dist>65 and (not actor or not actor.corpse) then enemy.waitClear=false end
		if actor and dist>85 and not actor.corpse then
			removeMarker(enemy)
			s.cast:remove(route.key)
			actor=nil
		end
		if actor and actor.dead and not actor.corpse and not enemy.waitClear and enemy.respawnIn==0 then
			s.cast:remove(route.key)
			actor=nil
		end
		if not actor and not enemy.waitClear and dist<65 and enemy.respawnIn==0 then
			actor=s.cast:person(route.key,{name=route.name,pos=route.a,look=route.look})
			if actor then
				enemy.health,enemy.returning=2,false
				local o=table.clone(marker)
				o.pos,o.tag,o.solid=actor.pos+v(0,2.35,0),"rival-marker",false
				o.points,o.edges,o.width=markerPoints,markerEdges,.048
				enemy.marker=world.spawn("wire",o)
			else
				-- A smaller device may fit only one rival rig at once.
				enemy.respawnIn=1
			end
		end
		if actor and enemy.health>0 then
			if not actor.impact and actor.arrived then
				s.cast:walk(route.key,{if enemy.returning then route.a else route.b},1.1)
				enemy.returning=not enemy.returning
			end
			world.set(enemy.marker,{pos=actor.pos+v(0,2.35,0),visible=not actor.impact})
		end
	end
end

local function visitPeople(state,minimum,maximum,visit)
	local s,cast=state.combat,state.storyCast
	if cast then
		for key,p in cast.people do
			if not p.shot and not p.dead then visit("cast",key,p,p.pos,p.actor) end
		end
	end
	for _,route in routes do
		local p=s.cast.people[route.key]
		if p and not p.dead and s.enemies[route.key].health>0 then visit("rival",route.key,p,p.pos,p.actor) end
	end
	impacts.visitResidents(state.population,minimum,maximum,function(id,c,feet,actor,velocity)
		visit("resident",id,c,feet,actor,velocity)
	end)
end

local function characterBounds(p,feet,actor)
	if p.impact and p.impact.stage=="down" and actor then
		local hips=humans.joint(actor,"hips")
		if hips then return hips-v(0,.23,0),.55 end
	end
	return feet,if p.pose=="sit" then 1.25 else 1.94
end

-- A car's model profile is narrower at the bonnet and the boot than its overall
-- bounds. Resolve the ordinary solid geometry, skipping a sedan's coarse box.
local function sceneHit(fleet,origin,ray,distance)
	local hit=world.raycast(origin,ray,distance)
	local ignore={}
	while hit and world.get(hit.id).tag=="vehicle" do
		local sedan=false
		for _,car in fleet.vehicles do
			if car.body==hit.id and car.kind=="car" then sedan=true; break end
		end
		if not sedan then break end
		table.insert(ignore,hit.id)
		hit=world.raycast(origin,ray,distance,{ignore=ignore})
	end
	return hit
end

local function targetOnRay(state,origin,ray,from,distance,projectile,limited)
	local target
	local story=state.story
	visitPeople(state,origin,origin+ray*(distance+2),function(kind,key,p,feet,actor,velocity)
		local pos,height=characterBounds(p,feet,actor)
		local range=if projectile then distance+2 else distance
		if not projectile and kind=="cast" and story and key==story.contractPerson then
			range=if limited then math.min(range,story.contractRange) else story.contractRange
		end
		local along,point
		if actor and p.impact and p.impact.stage=="down" then
			along=humans.ragdoll_raycast(actor,origin,ray,range)
			if along then point=origin+ray*along end
		else
			along,point=sidearm.intersect(origin,ray,from,pos,range,height)
		end
		if along and (not (projectile or limited) or along<=distance) and (not target or along<target.along or
			(along==target.along and kind==target.kind and key<target.key)) then
			target={kind=kind,key=key,along=along,point=point,feet=feet,actor=actor,person=p,velocity=velocity,head=not p.impact and point.y>=pos.y+height*.82}
		end
	end)
	corpses.visit(state.population,origin,origin+ray*distance,function(id,_,feet,actor)
		local along=humans.ragdoll_raycast(actor,origin,ray,distance)
		if along and (not target or along<target.along or along==target.along and target.kind=="corpse" and id<target.key) then
			target={kind="corpse",key=id,along=along,point=origin+ray*along,feet=feet,actor=actor}
		end
	end)
	local limit=if target then math.max(0,target.along-.001) else distance
	if limit<=0 then return target,if target then target.point else origin end
	local car,carPoint,carDistance=cars.raycast(state.fleet,origin,ray,limit)
	if car then
		target={kind="car",key=car.index,car=car,along=carDistance,point=carPoint}
		limit=math.max(0,carDistance-.001)
	end
	local cover
	if limit>0 then
		cover=if projectile or limited then sceneHit(state.fleet,origin,ray,limit) else sidearm.sceneryHit(origin,ray,limit)
	end
	if cover then
		local crate=state.combat.crates[cover.id]
		if crate then return {kind="crate",key=cover.id,point=cover.point},cover.point,cover end
		return nil,cover.point,cover
	end
	return target,if target then target.point else origin+ray*distance
end

local function flinch(target,dir,damage)
	if not target.actor then return end
	if target.person.impact and target.person.impact.stage=="down" then
		humans.ragdoll_hit(target.actor,target.point,dir*math.clamp(damage*3.5,.3,12))
	else
		humans.hit(target.actor,dir*math.clamp(damage*.55,.12,1.1))
	end
end

local function hitTarget(state,target,ray,blast,damage,gibbed)
	local s,story,cast=state.combat,state.story,state.storyCast
	local key=target.key
	local push=if blast then ray*(2+math.min(7,damage))+v(0,1.4,0) else ray*math.clamp(3.5*damage,.3,12)
	if blast and target.kind~="rival" then
		local person=target.person
		person.blastWounds=(person.blastWounds or 0)+damage
		if person.blastWounds<1 then flinch(target,ray,damage); return true,false end
		person.blastWounds=nil
	end
	if target.kind=="resident" then
		return impacts.shootResident(state.population,state.fleet,key,target.feet,push,gibbed,target.point,target.velocity),true
	elseif target.kind=="cast" then
		local p=cast.people[key]
		if not cast:kill(key,push,gibbed,target.point) then return false,false end
		p.shot=true
		if story and story.status=="active" then
			story.shotPeople=story.shotPeople or {}
			story.shotPeople[key]=true
		end
		return true,true
	end
	local enemy=s.enemies[key]
	local lethal=enemy.health<=damage
	if lethal and not s.cast:kill(key,push,gibbed,target.point) then return false,false end
	if not lethal then flinch(target,ray,damage) end
	enemy.health=if lethal then 0 else enemy.health-damage
	if lethal then
		enemy.respawnIn,enemy.waitClear=45,true
		removeMarker(enemy)
	end
	return true,lethal
end

-- Existing ragdolls receive the impact at the actual limb, including sleeping bodies.
-- Fragmentation consumes ownership exactly once, before another seat or blast can hit.
local function hitBody(state,id,actor,feet,point,dir,strength,gib,owner)
	local origins
	if gib then
		origins=fx.fragmentOrigins(actor,feet)
		if not corpses.gib(state.population,id) then return false end
	elseif not humans.ragdoll_hit(actor,point,dir*math.clamp(strength*2,.3,12)) then
		return false
	end
	fx.hit(state.combat.effects,point,dir,gib,game.tick()+owner,origins,strength)
	return true
end

local function recordHit(state,owner,target,dir,blast,damage)
	if target.kind=="corpse" then
		if hitBody(state,target.key,target.actor,target.feet,target.point,dir,damage,blast and damage>=1.5,owner) then
			local player=state.combat.players[owner]
			if player then sidearm.confirmHit(player,target.point) end
		end
		return
	end
	local gibbed=blast and damage>=3
	local joints=if gibbed then fx.fragmentOrigins(target.actor,target.feet) else nil
	local hit,lethal=hitTarget(state,target,dir,blast,damage,gibbed)
	if not hit then return end
	local player=state.combat.players[owner]
	if player then
		player.hits+=1
		if lethal then player.defeats+=1 end
		sidearm.confirmHit(player,target.point)
	end
	fx.hit(state.combat.effects,target.point,dir,lethal,game.tick()+owner,joints,damage)
end

local function blastBlocked(fleet,center,delta,distance,receiver,sourceCar)
	local limit=distance-.04
	if limit<=0 then return false end
	local ray=delta/distance
	return cars.raycast(fleet,center,ray,limit,receiver,sourceCar)~=nil or sceneHit(fleet,center,ray,limit)~=nil
end

local function blastDamage(state,owner,center,radius,target,forward,sourceCar,grenade)
	local extent=v(radius,radius,radius)
	local hits={}
	local function candidate(kind,key,p,feet,actor,contact,velocity)
		local delta=contact-center
		local distance=vector.magnitude(delta)
		if distance>=radius then return end
		local direct=target and target.kind==kind and target.key==key
		if not direct and blastBlocked(state.fleet,center,delta,distance,nil,sourceCar) then return end
		-- Growing Guns uses quadratic falloff. The compact visible flash and the
		-- gameplay radius are independent, so the outer edge only grazes a target.
		local damage=9*(1-distance/radius)^2
		table.insert(hits,{kind=kind,key=key,feet=feet,actor=actor,person=p,point=contact,velocity=velocity,
			dir=if distance>.01 then delta/distance else forward,damage=damage})
	end
	visitPeople(state,center-extent,center+extent,function(kind,key,p,feet,actor,velocity)
		local pos,height=characterBounds(p,feet,actor)
		candidate(kind,key,p,feet,actor,pos+v(0,height*.5,0),velocity)
	end)
	corpses.visit(state.population,center-extent,center+extent,function(id,body,feet,actor)
		local contact=humans.ragdoll_closest(actor,center)
		if contact then candidate("corpse",id,body.person,feet,actor,contact) end
	end)
	-- Applying damage may release crowd rigs. Finish visiting before changing them.
	table.sort(hits,function(a,b) return if a.kind==b.kind then a.key<b.key else a.kind<b.kind end)
	for _,hit in hits do recordHit(state,owner,hit,hit.dir,true,hit.damage) end
	local carHits={}
	for _,car in state.fleet.vehicles do
		if car.kind~="car" or car==sourceCar or car.wrecked then continue end
		local point=cars.closestPoint(car,center,radius)
		if not point then continue end
		local delta=point-center
		local distance=vector.magnitude(delta)
		if distance>=radius then continue end
		if blastBlocked(state.fleet,center,delta,distance,car,sourceCar) then continue end
		table.insert(carHits,{car=car,point=point,damage=(1-distance/radius)^2,immediate=grenade and distance<=2.5})
	end
	-- Every visibility query sees the same pre-blast car shapes.
	local immediate
	for _,hit in carHits do
		if hit.immediate then
			immediate=immediate or {}
			table.insert(immediate,hit.car)
		else
			cars.hit(hit.car,hit.point,hit.damage,owner)
		end
	end
	return immediate
end

-- The chassis containing the blast cannot shield its surroundings from itself.
-- Consume its transition before applying damage to the rest of the street.
local function explodeCar(state,car,owner,forward,grenade)
	if not cars.explode(car) then return end
	if state.story and state.story.status=="active" then state.story.checkWrecks=true end
	local center=car.pos+v(0,.76,0)
	explosions.play(center,6)
	return blastDamage(state,owner,center,if grenade then launcher.blastRadius else 14,nil,forward,car,grenade)
end

local function hitCrate(state,owner,id,forward,radius,grenade)
	local crate=state.combat.crates[id]
	if not explosions.hit(crate,id) then return end
	local immediate=blastDamage(state,owner,crate.pos,radius,nil,forward,nil,grenade)
	local shooter=state.combat.players[owner]
	if shooter then sidearm.confirmHit(shooter,crate.pos) end
	return immediate
end

local function detonate(state,shot,center,target)
	local immediate
	if target and target.kind=="crate" then
		immediate=hitCrate(state,shot.owner,target.key,shot.dir,launcher.blastRadius,true)
	elseif target and target.kind=="car" and not target.car.wrecked then
		-- The direct hit and the exploding vehicle produce a single blast.
		immediate=explodeCar(state,target.car,shot.owner,shot.dir,true)
		local player=state.combat.players[shot.owner]
		if player then sidearm.confirmHit(player,target.point) end
	else
		if target and target.kind=="car" then center-=shot.dir*.06 end
		explosions.play(center,launcher.effectRadius)
		immediate=blastDamage(state,shot.owner,center,launcher.blastRadius,target,shot.dir,nil,true)
	end
	if immediate then
		for _,car in immediate do explodeCar(state,car,shot.owner,shot.dir,false) end
	end
end

local function shoot(state,id,player,seat,input,dt)
	local spec=catalog.get(seat.weapon or "pistol",seat)
	local pressed=sidearm.trigger(input,spec.automatic)
	local enabled=seat.mode=="walk" and not seat.impact
	player.weapon=spec.id
	local gun=sidearm.magazine(player,spec)
	player.aiming,player.onTarget=enabled and seat.aiming==true,false
	if not enabled then
		player.ready,player.hipHold,player.kick,player.kickSpeed,player.holdYaw=nil,nil,nil,nil,nil
		sidearm.remove(player)
		return
	end
	local origin,ray
	if player.aiming then origin,ray=sidearm.ray(id)
	elseif pressed or player.shotFlash>0 then origin,ray=sidearm.hipRay(seat) end
	sidearm.follow(player,state.population.avatars[id],seat,ray,spec,dt,pressed)
	if not ray or (not player.aiming and not pressed) then return end
	local isLauncher=spec.kind=="launcher"
	local target,point=targetOnRay(state,origin,ray,seat.pos,spec.range,isLauncher,spec.kind=="hitscan")
	player.onTarget=target~=nil
	if not pressed then return end
	if isLauncher then
		if launcher.fire(state.combat.projectiles,id,player,seat,point,ray,spec.range) then sidearm.recoil(player,spec) end
		return
	end
	if not sidearm.discharge(player,seat,spec,gun,dt) then return end
	local spreads=spec.spread and spec.spread>0 and sidearm.pattern(ray,spec.spread,id,player.shotNumber)
	local contacts={}
	local tracePoint,left,right=point,nil,nil
	local lo,hi=math.huge,-math.huge
	local side=if spreads and spec.pellets>1 then vector.cross(ray,v(0,1,0)) else nil
	for pellet=1,spec.pellets or 1 do
		local hit,endPoint,dir=target,point,ray
		if spreads then
			dir=spreads()
			hit,endPoint=targetOnRay(state,origin,dir,seat.pos,spec.range,false,true)
		end
		if pellet==1 then tracePoint=endPoint end
		if side then
			local offset=vector.dot(side,dir)
			if offset<lo then lo,left=offset,endPoint end
			if offset>hi then hi,right=offset,endPoint end
		end
		if hit then
			local same
			for _,prior in contacts do
				if prior.kind==hit.kind and prior.key==hit.key then same=prior; break end
			end
			local damage=(spec.damage or 1)*(if hit.head then spec.headshot or 1 else 1)
			if same then same.damage+=damage
			else
				hit.dir,hit.damage=dir,damage
				table.insert(contacts,hit)
			end
		end
	end
	-- Draw the first and outermost actual pellet paths, up to three per seat.
	sidearm.flash(player,tracePoint,spec.color,left,right)
	for _,hit in contacts do
		if hit.kind=="crate" then
			hitCrate(state,id,hit.key,hit.dir,14)
		elseif hit.kind=="car" then
			cars.hit(hit.car,hit.point,math.min(.65,hit.damage*.065),id)
			sidearm.confirmHit(player,hit.point)
		else
			recordHit(state,id,hit,hit.dir,false,hit.damage)
		end
	end
end

local function roadDamage(state,hit)
	local s,p=state.combat,hit.person
	local speed=hit.speed
	local dir=if vector.magnitude(hit.velocity)>.001 then vector.normalize(hit.velocity) else v(0,0,-1)
	local strength=math.clamp(speed*.12,.55,4)
	if hit.corpse then
		hitBody(state,hit.corpse,hit.actor,p.pos-v(0,.23,0),hit.pos,dir,strength,hit.gib,hit.owner)
		return
	end
	if p.dead then return end
	local killed=false
	if hit.lethal and not hit.seat then
		local push=dir*math.min(speed*.5,9)+v(0,1,0)
		if hit.resident then
			killed=impacts.shootResident(state.population,state.fleet,hit.resident,p.pos,push,false)
		else
			for key,enemy in s.enemies do
				if s.cast.people[key]==p and s.cast:kill(key,push) then
					enemy.health,enemy.respawnIn,enemy.waitClear=0,45,true
					removeMarker(enemy)
					killed=true
					break
				end
			end
			if not killed and state.storyCast then
				for key,actor in state.storyCast.people do
					if actor==p then
						killed=state.storyCast:kill(key,push)
						if killed then
							p.roadDeath=true
							if state.story and state.story.status=="active" then state.story.checkRoadDeaths=true end
						end
						break
					end
				end
			end
		end
	end
	fx.hit(s.effects,hit.pos,dir,killed,game.tick()+hit.owner,nil,strength)
end

function combat.tick(state,dt,input)
	local s,fleet,story=state.combat,state.fleet,state.story
	s.crates=s.crates or explosions.placeCrates()
	explosions.tick(s.crates,fleet)
	fx.tick(s.effects,dt)
	for _,hit in state.population.roadHits or {} do roadDamage(state,hit) end
	updateRivals(s,fleet,dt)
	for id,player in s.players do
		if not fleet.players[id] or fleet.players[id].bot then
			-- A departing player's grenade still lands. A later occupant of this
			-- seat should not receive the credit for its impact.
			for _,shot in s.projectiles or {} do if shot.owner==id then shot.owner=0 end end
			for _,car in fleet.vehicles do if car.fireOwner==id then car.fireOwner=nil end end
			sidearm.remove(player)
			sidearm.clearFlash(player)
			s.players[id]=nil
		end
	end
	s.projectiles=s.projectiles or {}
	launcher.advance(state,dt,targetOnRay,detonate)
	cars.tickFires(fleet,dt,function(car)
		explodeCar(state,car,car.fireOwner or 0,v(0,1,0),false)
	end)
	for _,info in players.list() do
		local id=info.id
		local seat=fleet.players[id]
		if not seat or seat.bot then continue end
		local player=s.players[id]
		if not player then
			player={hits=0,defeats=0,shotFlash=0,aiming=false,onTarget=false}
			s.players[id]=player
		end
		player.launcherCooldown=math.max(0,(player.launcherCooldown or 0)-dt)
		sidearm.tickFlash(player,player,dt)
		shoot(state,id,player,seat,input[id] or {},dt)
		if story and story.weapon and story.owner==id then
			story.aiming,story.onTarget,story.shotFlash=player.aiming,player.onTarget,player.shotFlash
		end
	end
end

return combat
