-- Decode author data before the mission creates a person, car or marker.
local city = require("world")
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
		if rawget(p,"x")~=nil then
			for k in next,p do if k~="x" and k~="y" and k~="z" then return nil end end
			x,y,z=rawget(p,"x"),rawget(p,"y"),rawget(p,"z")
		elseif list(p,3,false) and #p==3 then x,y,z=p[1],p[2],p[3] end
	end
	if finite(x) and finite(z) and number(y,-.25,100) and city.on_land(x,z,1) then return v(x,y,z) end
	return nil
end

local function value(t,kind)
	if kind=="key" then return if text(t,48) and string.match(t,"^[a-z][a-z0-9_-]*$") then t else nil
	elseif kind=="name" then return if text(t,80) then t else nil
	elseif kind=="text" then return if text(t,140) then t else nil
	elseif kind=="pos" then return position(t)
	elseif kind=="angle" then return if finite(t) then t%(2*math.pi) else nil
	elseif kind=="look" then return if number(t,0,71) and t%1==0 then t else nil
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
}
for _, spec in actions do spec.op="key"; spec.text="text" end

function schema.validate(raw)
	if not plain(raw) then return nil,"Story must be a table" end
	for key in next,raw do
		if key~="id" and key~="title" and key~="cast" and key~="steps" then return nil,"Unknown story field" end
	end
	local id,title=value(raw.id,"key"),value(raw.title,"name")
	if not id or not title then return nil,"Story needs an id and a title" end
	if not list(raw.cast,12,true) then return nil,"cast must be a dense list of up to 12 actors" end
	if not list(raw.steps,64,false) then return nil,"steps must contain 1 to 64 ordered actions" end
	local out={id=id,title=title,cast={},steps={}}
	local kinds, riding,occupied = {},{},{}
	local people,cars=0,0
	for i=1,#raw.cast do
		local actor,path=raw.cast[i],"cast["..i.."]"
		local kind=if plain(actor) then rawget(actor,"kind") else nil
		if type(kind)~="string" or not actors[kind] then return nil,path.." needs kind person or car" end
		local a,err=read(actor,actors[kind],path)
		if not a then return nil,err end
		if kinds[a.key] then return nil,path.." duplicates "..a.key end
		kinds[a.key]=a.kind
		if a.kind=="person" then people+=1 else cars+=1 end
		if people>8 or cars>4 then return nil,"A story supports up to 8 people and 4 cars" end
		table.insert(out.cast,a)
	end
	for i=1,#raw.steps do
		local step,path=raw.steps[i],"steps["..i.."]"
		local op=if plain(step) then rawget(step,"op") else nil
		if type(op)~="string" or not actions[op] then return nil,path.." has an unknown action" end
		local a,err=read(step,actions[op],path)
		if not a then return nil,err end
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
	return out
end
return schema
