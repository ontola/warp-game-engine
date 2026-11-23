-- A death transfers the actual fitted rig to the city. The walking crowd and
-- named missions can release their own records without resetting its pose.
local corpses={}
local v=vector.create
local RETAIN, NEAR, MAX=45,65,12
local PELVIS=v(0,.23,0)
local EMPTY={}

local function nearest(fleet,pos)
	local best=math.huge
	for _,seat in fleet.players do
		if not seat.bot then
			local d=pos-seat.pos
			best=math.min(best,vector.dot(d,d))
		end
	end
	return best
end

function corpses.observed(fleet,pos)
	return nearest(fleet,pos)<=NEAR*NEAR
end

local function retire(pop,id,gibbed)
	local body=pop.corpses and pop.corpses[id]
	if not body then return false end
	pop.corpses[id]=nil
	local person=body.person
	person.corpse=nil
	person.gibbed=gibbed or nil
	person.impact=if person.story then {stage=if gibbed then "gibbed" else "dead"} else nil
	if not person.story then person.pos=person.deathHome or person.pos end
	humans.remove(id)
	return true
end

-- Eviction considers every human observer. In the same eight-metre band
-- release the older rig first. The actor ID resolves ties deterministically.
local function eviction(pop,fleet,pool,look,variety)
	local worst,band,oldest=nil,-1,math.huge
	for id,body in pop.corpses or EMPTY do
		if pool and body.pool~=pool or look and body.look%variety~=look%variety then continue end
		local d=math.floor(math.sqrt(nearest(fleet,body.pos))/8)
		if d>band or d==band and (body.born<oldest or body.born==oldest and id<(worst or math.huge)) then
			worst,band,oldest=id,d,body.born
		end
	end
	return worst
end

-- If a relevant living actor cannot acquire its fitted body type, free one
-- compatible corpse slot. Other looks and pools cannot satisfy that request.
function corpses.freeLook(pop,fleet,pool,look)
	local _,_,variety=humans.room(pool)
	local id=eviction(pop,fleet,pool,look,variety)
	return id~=nil and retire(pop,id,false)
end

function corpses.retain(pop,fleet,person,actor,pool,gibbed)
	if person.dead then return false end
	person.dead=true
	person.deathHome=person.impact and person.impact.home or person.pos
	person.deathPos=(actor and humans.joint(actor,"hips")) or person.pos+v(0,.85,0)
	person.shotUntil=pop.elapsed+RETAIN
	person.active,person.externalPose,person.previousFeet=false,true,nil
	if gibbed or not actor then
		person.gibbed=gibbed or nil
		person.impact=if person.story then {stage=if gibbed then "gibbed" else "dead"} else nil
		if actor then humans.remove(actor) end
		return true
	end
	person.impact=person.impact or {stage="down",elapsed=0,home=person.pos}
	person.impact.stage,person.impact.hips="down",person.deathPos
	person.corpse=actor
	pop.corpses=pop.corpses or {}
	pop.corpses[actor]={person=person,pos=person.deathPos,born=pop.elapsed,pool=pool,look=person.look or 0}
	local total,pooled=0,0
	for _,body in pop.corpses do
		total+=1
		if body.pool==pool then pooled+=1 end
	end
	local _,capacity=humans.room(pool)
	local allowance=if pool=="resident" then math.clamp(math.ceil(capacity/3),1,10) else capacity
	if pooled>allowance then retire(pop,eviction(pop,fleet,pool),false)
	elseif total>MAX then retire(pop,eviction(pop,fleet),false) end
	return true
end

-- Retire a body only after the caller has sampled its joints and committed the
-- hit. Later pellets, overlapping blasts and other seats see an absent body.
function corpses.gib(pop,id)
	return retire(pop,id,true)
end

-- The loose radius encloses the limbs around the pelvis. The native ragdoll
-- query resolves the exact posed shape for a shortlisted body.
function corpses.visit(pop,a,b,visit)
	for id,body in pop.corpses or EMPTY do
		local p=body.pos
		if a and (p.x<math.min(a.x,b.x)-2 or p.x>math.max(a.x,b.x)+2
			or p.y<math.min(a.y,b.y)-2 or p.y>math.max(a.y,b.y)+2
			or p.z<math.min(a.z,b.z)-2 or p.z>math.max(a.z,b.z)+2) then continue end
		visit(id,body,p-PELVIS,id)
	end
end

function corpses.tick(pop,fleet,roots)
	for id,body in pop.corpses or EMPTY do
		local hips=humans.joint(id,"hips")
		if not hips or pop.elapsed-body.born>=RETAIN and not corpses.observed(fleet,hips) then
			retire(pop,id,false)
		else
			body.pos=hips
			local person=body.person
			person.pos,person.deathPos=hips-PELVIS,hips
			person.impact.hips=hips
			table.insert(roots,hips)
		end
	end
end

return corpses
