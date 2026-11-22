-- Two marked rivals patrol the sidewalk across the first intersection. Civilians
-- and other seats never enter the arcade pistol's list of eligible targets.
local Cast=require("story_cast")
local crowd=require("kit/crowd")
local fx=require("combat_fx")
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
-- The city already owns the fleet and avatars. Avoid duplicating them in game state.
local systems=setmetatable({},{__mode="k"})

function combat.new(fleet,pop)
	world.warm("wire",marker)
	fx.warm()
	local s={players={},enemies={},cast=Cast.new(fleet,pop,crowd.new("rival")),effects=fx.new()}
	for _,r in routes do s.enemies[r.key]={health=2,respawnIn=0} end
	systems[s]={fleet=fleet,avatars=pop.avatars}
	return s
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

local function contractOwner(story,seats)
	if not story or story.status~="active" or not story.weapon then return nil end
	if seats[story.owner] then return story.owner end
	-- Missions hands a disconnected owner's contract to the lowest remaining seat.
	local owner
	for id,seat in seats do if not seat.bot and (not owner or id<owner) then owner=id end end
	return owner
end

local function shoot(s,id,player,seat,input)
	local pressed=input.pressed and input.pressed.fire
	local aimed=seat.aiming==true
	local enabled=seat.mode=="walk" and not seat.impact
	player.aiming,player.onTarget=enabled and aimed,false
	if not enabled then
		sidearm.remove(player)
		return
	end
	local origin,ray
	if aimed then origin,ray=sidearm.ray(id)
	elseif pressed or player.shotFlash>0 then origin,ray=sidearm.hipRay(seat) end
	sidearm.follow(player,systems[s].avatars[id],seat,ray)
	if not ray then return end
	local key,point,nearest=nil,nil,math.huge
	for _,route in routes do
		local actor=s.cast.people[route.key]
		if actor and not actor.impact and s.enemies[route.key].health>0 then
			local along,hit=sidearm.contact(origin,ray,seat.pos,actor.pos,42)
			if along and along<nearest then nearest,key,point=along,route.key,hit end
		end
	end
	player.onTarget=key~=nil
	if not sidearm.discharge(player,player,seat,input) or not key then return end
	local enemy=s.enemies[key]
	local lethal=enemy.health==1
	if lethal and not s.cast:hit(key,ray*2+v(0,.3,0)) then return end
	player.hits+=1
	enemy.health-=1
	fx.hit(s.effects,point,ray,lethal,game.tick()+id)
	if lethal then
		player.defeats+=1
		enemy.respawnIn=12
		removeMarker(enemy)
	end
end

function combat.tick(s,story,dt,input)
	local fleet=systems[s].fleet
	fx.tick(s.effects,dt)
	updateRivals(s,fleet,dt)
	local owner=contractOwner(story,fleet.players)
	for id,player in s.players do
		if not fleet.players[id] or fleet.players[id].bot then
			sidearm.remove(player)
			sidearm.clearFlash(player)
			s.players[id]=nil
		end
	end
	-- A simultaneous hit is awarded in player order.
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
		if owner==id then
			player.aiming,player.onTarget=false,false
			sidearm.remove(player)
		else
			shoot(s,id,player,seat,input[id] or {})
		end
	end
end

return combat
