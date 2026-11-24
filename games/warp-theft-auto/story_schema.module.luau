-- Decode author data before the mission creates a person, car or marker.
local city = require("world")
local cape = require("west-cape")
local weapons = require("weapon_catalog")
local schema = {}
local v = vector.create
local function plain(t) return type(t)=="table" and getmetatable(t)==nil end
local function finite(n) return type(n)=="number" and n==n and math.abs(n)<math.huge end
local function number(n,lo,hi) return finite(n) and n>=lo and n<=hi end
local function text(s,max) return type(s)=="string" and #s>0 and #s<=max and not string.find(s,"%c") end
local function list(t,limit,empty)
	if not plain(t) then return false end
	local seen, last = 0, 0
	for k in next,t do
		if not number(k,1,limit) or k%1~=0 then return false end
		seen+=1
		last=math.max(last,k)
	end
	return seen==last and (empty or seen>0)
end

local function position(p)
	local x,y,z
	if typeof(p)=="vector" then x,y,z=p.x,p.y,p.z
	elseif plain(p) then
		if type(rawget(p,"place"))=="string" then
			local name,g=p.place,rawget(p,"index")
			for k in next,p do if k~="place" and k~="offset" and (k~="index" or name~="rallyGate") then return nil end end
			local data=cape.layout()
			local at
			if name=="rallyGate" then
				if not number(g,1,#data.rally.gates) or g%1~=0 then return nil end
				at=data.rally.gates[g]
			elseif name=="rallyStart" then at=data.rally.start
			else at=rawget(data.places,name) end
			if not at then return nil end
			x,y,z=at[1],0,at[2]
			local offset=rawget(p,"offset")
			if offset~=nil then
				if not list(offset,2,false) or #offset~=2 or
					not number(offset[1],-512,512) or not number(offset[2],-512,512) then return nil end
				local nx,nz=1,0
				if name=="ferryTerminal" then
					local _,_,east,south=cape.shoreFrame(x,z)
					nx,nz=east,south
				end
				x,z=x+nx*offset[1]-nz*offset[2],z+nz*offset[1]+nx*offset[2]
			end
		elseif rawget(p,"x")~=nil then
			for k in next,p do if k~="x" and k~="y" and k~="z" then return nil end end
			x,y,z=rawget(p,"x"),rawget(p,"y"),rawget(p,"z")
		elseif list(p,3,false) and #p==3 then x,y,z=p[1],p[2],p[3] end
	end
	if finite(x) and finite(z) and number(y,-.25,100) and city.on_land(x,z,1) then
		local floor = cape.height(x,z)
		return v(x,if floor>0 then math.max(y,floor) else y,z)
	end
	return nil
end

local function value(t,kind)
	if kind=="key" then return if text(t,48) and string.match(t,"^[a-z][a-z0-9_-]*$") then t else nil
	elseif kind=="name" then return if text(t,80) then t else nil
	elseif kind=="text" then return if text(t,140) then t else nil
	elseif kind=="pos" then return position(t)
	elseif kind=="angle" then return if finite(t) then t%(2*math.pi) else nil
	elseif kind=="weapon" then return if weapons.exists(t) then t else nil
	elseif kind=="weaponParams" then return weapons.parameters(t)
	elseif kind=="look" then return if number(t,0,71) and t%1==0 then t else nil
	elseif kind=="route" then return if t=="rally" then t else nil
	elseif kind=="notes" then
		if not list(t,63,true) then return nil end
		for _,note in t do if not text(note,140) then return nil end end
		return t
	elseif kind=="paint" then return if t=="ochre" or t=="teal" or t=="pearl" or t=="vermilion" then t else nil
	elseif kind=="seat" then return if t=="driver" or t=="passenger" then t else nil
	elseif kind=="speed" then return if number(t,.2,22) then t else nil
	elseif kind=="radius" then return if number(t,1,24) then t else nil
	elseif kind=="range" then return if number(t,2,75) then t else nil
	elseif kind=="bool" then return if type(t)=="boolean" then t else nil
	elseif kind=="points" then
		if not list(t,32,false) then return nil end
		local points={}
		for i=1,#t do
			local point=position(t[i])
			if not point then return nil end
			points[i]=point
		end
		return points
	end
	return nil
end

local function read(raw, fields, path)
	if not plain(raw) then return nil,path.." must be a table" end
	for k in next,raw do
		if type(k)~="string" or fields[k]==nil then return nil,path.." has an unknown field" end
	end
	local out={}
	for key, spec in fields do
		local a=rawget(raw,key)
		local optional=string.sub(spec,-1)=="?"
		if a==nil then
			if not optional then return nil,path.." needs "..key end
		else
			local parsed=value(a,if optional then string.sub(spec,1,-2) else spec)
			if parsed==nil then return nil,path.."."..key.." is invalid" end
			out[key]=parsed
		end
	end
	return out
end

local actors={
	person={kind="key",key="key",name="name?",pos="pos",yaw="angle?",look="look?"},
	car={kind="key",key="key",name="name?",pos="pos",yaw="angle?",paint="paint?"},
}
local actions={
	walk={person="key",points="points",speed="speed?",wait="bool?"},
	drive={car="key",points="points",speed="speed?",wait="bool?"},
	ride={person="key",car="key",seat="seat?"},
	dismount={person="key"},
	enter={car="key"},
	pickup={person="key",car="key?",radius="radius?"},
	dropoff={person="key",pos="pos",radius="radius?"},
	checkpoint={car="key?",pos="pos",radius="radius?"},
	contract={person="key",range="range?"},
	explosion={pos="pos",radius="radius?"},
	equip={weapon="weapon",params="weaponParams?"},
}
for _, spec in actions do spec.op="key"; spec.text="text" end

-- Expand a race route before it reaches the ordinary swept checkpoint logic.
local routeFields={op="key",route="route",car="key?",radius="radius?",text="text",finish="text?",notes="notes?"}
local function anchored(p)
	return plain(p) and type(rawget(p,"place"))=="string"
end

local function expand(raw)
	local steps,origins={},{}
	local usesMap=false
	for i,step in raw.steps do
		if plain(step) and rawget(step,"op")=="race_route" then
			local group,err=read(step,routeFields,"steps["..i.."]")
			if not group then return nil,err end
			usesMap=true
			local rally=cape.layout().rally
			local gates=rally.gates
			if #steps+#gates>64 then return nil,"A story can contain at most 64 expanded steps" end
			for n=1,#gates do
				local note=if n==#gates and group.finish then group.finish else (group.notes and group.notes[n]) or group.text
				table.insert(steps,{op="checkpoint",pos={place="rallyGate",index=n},car=group.car,radius=group.radius,text=note})
				table.insert(origins,{i,rally.gateBends[n]})
			end
		else
			if #steps==64 then return nil,"A story can contain at most 64 expanded steps" end
			table.insert(steps,step)
			table.insert(origins,{i,1})
			if plain(step) then
				if anchored(rawget(step,"pos")) then usesMap=true end
				local points=rawget(step,"points")
				if list(points,32,false) then
					for _,p in points do if anchored(p) then usesMap=true; break end end
				end
			end
		end
	end
	return steps,origins,usesMap
end

function schema.validate(raw)
	if not plain(raw) then return nil,"Story must be a table" end
	for key in next,raw do
		if key~="id" and key~="title" and key~="cast" and key~="steps" then return nil,"Unknown story field" end
	end
	local id,title=value(raw.id,"key"),value(raw.title,"name")
	if not id or not title then return nil,"Story needs an id and a title" end
	if not list(raw.cast,12,true) then return nil,"cast must be a dense list of up to 12 actors" end
	if not list(raw.steps,64,false) then return nil,"steps must contain 1 to 64 ordered actions" end
	local steps,origins,usesMap=expand(raw)
	if not steps then return nil,origins end
	local out={id=id,title=title,cast={},steps={}}
	local kinds, riding,occupied = {},{},{}
	local people,cars=0,0
	for i=1,#raw.cast do
		local actor,path=raw.cast[i],"cast["..i.."]"
		local kind=if plain(actor) then rawget(actor,"kind") else nil
		if type(kind)~="string" or not actors[kind] then return nil,path.." needs kind person or car" end
		local a,err=read(actor,actors[kind],path)
		if not a then return nil,err end
		if anchored(actor.pos) then
			usesMap=true
			if not a.yaw and actor.pos.place=="rallyStart" then a.yaw=cape.layout().rally.yaw end
		end
		if kinds[a.key] then return nil,path.." duplicates "..a.key end
		kinds[a.key]=a.kind
		if a.kind=="person" then people+=1 else cars+=1 end
		if people>8 or cars>4 then return nil,"A story supports up to 8 people and 4 cars" end
		table.insert(out.cast,a)
	end
	for i=1,#steps do
		local step,path=steps[i],"steps["..origins[i][1].."]"
		local op=if plain(step) then rawget(step,"op") else nil
		if type(op)~="string" or not actions[op] then return nil,path.." has an unknown action" end
		local a,err=read(step,actions[op],path)
		if not a then return nil,err end
		if a.params and weapons.get(a.weapon).kind~="hitscan" then
			return nil,path..".params needs a Growing Guns weapon"
		end
		if a.person and kinds[a.person]~="person" then return nil,path.." references an unknown person" end
		if a.car and kinds[a.car]~="car" then return nil,path.." references an unknown car" end
		if op=="pickup" or op=="ride" then
			if riding[a.person] then return nil,path.." boards someone already riding" end
			local seat=if op=="pickup" then "passenger" else a.seat or "passenger"
			local slot=(a.car or "$player").."/"..seat
			if occupied[slot] then return nil,path.." assigns an occupied seat" end
			occupied[slot]=a.person
			riding[a.person]=slot
		elseif op=="dropoff" or op=="dismount" then
			if not riding[a.person] then return nil,path.." needs a prior pickup or ride" end
			occupied[riding[a.person]]=nil
			riding[a.person]=nil
		elseif op=="walk" and riding[a.person] then return nil,path.." cannot walk while riding" end
		table.insert(out.steps,a)
	end
	if usesMap then out.mapSource,out.origins=raw,origins end
	return out
end
return schema
