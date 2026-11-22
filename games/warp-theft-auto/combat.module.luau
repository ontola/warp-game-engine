-- One pistol per local seat. Each shot resolves against the nearest visible
-- person across the shared street crowd, interior residents, rivals and cast.
local Cast=require("story_cast")
local crowd=require("kit/crowd")
local fx=require("combat_fx")
local impacts=require("impacts")
local sidearm=require("sidearm")
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
	local s={players={},enemies={},cast=Cast.new(fleet,pop,crowd.new("rival")),effects=fx.new()}
	for _,r in routes do s.enemies[r.key]={health=2,respawnIn=0} end
	return s
end

function combat.reload(state)
	if state.combat then Cast.bind(state.combat.cast,state.fleet,state.population,crowd.new("rival")) end
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
		if actor and (dist>85 or (enemy.health<=0 and enemy.respawnIn<10.8)) then
			removeMarker(enemy)
			s.cast:remove(route.key)
			actor=nil
		end
		if not actor and dist<65 and enemy.respawnIn==0 then
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

local RANGE=sidearm.range

local function targetOnRay(state,seat,origin,ray)
	local s,story,cast=state.combat,state.story,state.storyCast
	local target
	-- Prefer the authored actor at a tie, e.g. when a story has placed two
	-- people at the exact same spot. A closer pedestrian always wins.
	local function consider(kind,key,p,feet,range,actor)
		local height
		local pos=feet
		if p.impact and p.impact.stage=="down" and actor then
			local hips=humans.joint(actor,"hips")
			if hips then pos,height=hips-v(0,.23,0),.55 end
		elseif p.pose=="sit" then height=1.25 end
		local along,point=sidearm.intersect(origin,ray,seat.pos,pos,range,height)
		if along and (not target or along<target.along or
			(along==target.along and kind==target.kind and key<target.key)) then
			target={kind=kind,key=key,along=along,point=point,feet=feet}
		end
	end
	if cast then
		for key,p in cast.people do
			if not p.shot then
				local range=if story and key==story.contractPerson then story.contractRange else RANGE
				consider("cast",key,p,p.pos,range,p.actor)
			end
		end
	end
	for _,route in routes do
		local p=s.cast.people[route.key]
		if p and s.enemies[route.key].health>0 then consider("rival",route.key,p,p.pos,RANGE,p.actor) end
	end
	impacts.visitResidents(state.population,origin,origin+ray*(RANGE+2),function(id,c,feet,actor)
		consider("resident",id,c,feet,RANGE,actor)
	end)
	if target and sidearm.clear(origin,ray,target.along) then return target end
	return nil
end

local function hitTarget(state,target,ray)
	local s,story,cast=state.combat,state.story,state.storyCast
	local key=target.key
	local push=ray*2+v(0,.3,0)
	if target.kind=="resident" then
		return impacts.shootResident(state.population,state.fleet,key,target.feet,push),true
	elseif target.kind=="cast" then
		local p=cast.people[key]
		if not cast:hit(key,push) then return false,false end
		p.shot=true
		if story and story.status=="active" then
			story.shotPeople=story.shotPeople or {}
			story.shotPeople[key]=true
		end
		return true,true
	end
	local enemy=s.enemies[key]
	local lethal=enemy.health==1
	if lethal and not s.cast:hit(key,push) then return false,false end
	enemy.health-=1
	if lethal then
		enemy.respawnIn=12
		removeMarker(enemy)
	end
	return true,lethal
end

local function shoot(state,id,player,seat,input)
	local pressed=sidearm.trigger(input)
	local enabled=seat.mode=="walk" and not seat.impact
	player.aiming,player.onTarget=enabled and seat.aiming==true,false
	if not enabled then
		sidearm.remove(player)
		return
	end
	local origin,ray
	if player.aiming then origin,ray=sidearm.ray(id)
	elseif pressed or player.shotFlash>0 then origin,ray=sidearm.hipRay(seat) end
	sidearm.follow(player,state.population.avatars[id],seat,ray)
	if not ray or (not player.aiming and not pressed) then return end
	local target=targetOnRay(state,seat,origin,ray)
	player.onTarget=target~=nil
	if not sidearm.discharge(player,player,seat,input,target and target.point,origin,ray) or not target then return end
	local hit,lethal=hitTarget(state,target,ray)
	if not hit then return end
	player.hits+=1
	if lethal then player.defeats+=1 end
	sidearm.confirmHit(player,target.point)
	fx.hit(state.combat.effects,target.point,ray,lethal,game.tick()+id)
end

function combat.tick(state,dt,input)
	local s,fleet,story=state.combat,state.fleet,state.story
	fx.tick(s.effects,dt)
	updateRivals(s,fleet,dt)
	for id,player in s.players do
		if not fleet.players[id] or fleet.players[id].bot then
			sidearm.remove(player)
			sidearm.clearFlash(player)
			s.players[id]=nil
		end
	end
	for _,info in players.list() do
		local id=info.id
		local seat=fleet.players[id]
		if not seat or seat.bot then continue end
		local player=s.players[id]
		if not player then
			player={hits=0,defeats=0,shotFlash=0,aiming=false,onTarget=false}
			s.players[id]=player
		end
		sidearm.tickFlash(player,player,dt)
		shoot(state,id,player,seat,input[id] or {})
		if story and story.weapon and story.owner==id then
			story.aiming,story.onTarget,story.shotFlash=player.aiming,player.onTarget,player.shotFlash
		end
	end
end

return combat
