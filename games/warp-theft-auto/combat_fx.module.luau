-- Directional hits and lightweight bouncing fragments for the arcade sidearm.
-- The cooked droplet, flesh-cut silhouette, and alpha-cut splat come from Growing Guns.
local effects = {}
local v = vector.create
local UP, DOWN = v(0, 1, 0), v(0, -1, 0)
local DROP = "assets/effects/blood-drop.glb"
local SHARD = "assets/effects/blood-shard.glb"
local SPLAT = "assets/effects/blood-splat.glb"
local CITY = {tag="city"}
local SCENERY = {tag="scenery", all=true}
local ROOM = {tag="interior_detail", all=true}

-- Shared by all local seats and story/roaming encounters. When full, replace the oldest
-- of the same kind so persistent stains never crowd out the next visible hit.
local live = {drop={}, shard={}, splat={}}
local limits = {drop=16, shard=7, splat=8}

function effects.warm()
	for _, model in {DROP, SHARD, SPLAT} do world.warm(model) end
end

function effects.new()
	return {serial=0}
end

local function spawn(ctx, kind, model, pos, rot, size, life)
	local items = live[kind]
	if #items >= limits[kind] then world.remove(table.remove(items, 1).id) end
	local id = world.spawn(model, {pos=pos, rot=rot, size=size, solid=false, tag="combat-"..kind})
	local item = {id=id, owner=ctx, pos=pos, rot=rot, size=size, age=0, life=life}
	table.insert(items, item)
	return item
end

-- The collision slab sits below visual layers such as crosswalks, plaza paving and rugs.
-- Only inspect the few centimetres above the supporting surface for a better contact.
local function ground(pos)
	local support = world.raycast(pos, DOWN, 12, CITY)
	if not support or support.normal.y < .8 then return nil end
	local y = support.point.y
	local start = support.point + UP*.19
	for _, filter in {SCENERY, ROOM} do
		local detail = world.raycast(start, DOWN, .20, filter)
		if detail and detail.normal.y > .8 then y = math.max(y, detail.point.y) end
	end
	return y
end

local function facing(velocity)
	local dir = vector.normalize(velocity)
	return v(-math.asin(math.clamp(dir.y, -1, 1)), math.atan2(dir.x, dir.z), 0)
end

function effects.hit(ctx, pos, ray, lethal, seed)
	ctx.serial += 1
	local bits = math.floor(seed or ctx.serial * 719 + pos.x * 239 + pos.z * 631) % 2147483647
	local function draw(a, b)
		bits = (48271 * math.max(1, bits)) % 2147483647
		return a + (b-a) * bits / 2147483647
	end
	local axis = if vector.magnitude(ray) > .001 then vector.normalize(ray) else v(0,0,1)
	local side = vector.cross(UP, axis)
	side = if vector.magnitude(side) > .01 then vector.normalize(side) else v(1,0,0)
	local floorY = ground(pos)
	for i=1, if lethal then 8 else 5 do
		local velocity = axis*draw(2.2,5.5) + side*draw(-2,2) + UP*draw(.3,2.7)
		local origin = pos + axis*.06
		-- The contact point is inside the torso. Kick two leading streaks off its near
		-- surface so feedback reads immediately from the player's shoulder camera.
		if i <= 2 then
			local sign = if i==1 then -1 else 1
			origin = pos - axis*.24 + side*(sign*.13)
			velocity = side*(sign*draw(3.4,4.8)) - axis*draw(.4,1.2) + UP*draw(.8,2.5)
		end
		local radius = draw(.049,.09)
		local size = v(radius,radius,radius*draw(1.2,2.1))
		local drop = spawn(ctx, "drop", DROP, origin, facing(velocity), size, draw(.48,.82))
		drop.velocity, drop.floor = velocity, floorY
	end
	if lethal then
		for i=1, 4 do
			local radius = draw(.085,.18)
			local size = v(radius*draw(.7,1.1), radius*draw(1.1,1.8), radius)
			local rot = v(draw(-2,2),draw(-3,3),draw(-2,2))
			local shard = spawn(ctx, "shard", SHARD, pos, rot, size, draw(3.3,4.7))
			shard.velocity = axis*draw(1.2,3.5) + side*draw(-2.2,2.2) + UP*draw(1.5,3.8)
			shard.spin, shard.floor = v(draw(-10,10),draw(-10,10),draw(-10,10)), floorY
		end
	end
	-- Growing Guns sprays footprints along travel with lateral scatter and one dominant pool.
	for i=1, if lethal then 2 else 1 do
		local at = pos + axis*draw(.18,if i==1 then .65 else 1.6) + side*draw(-.48,.48)
		local y = ground(at)
		if y then
			local width = if i==1 and lethal then draw(.95,1.38) else draw(.48,.86)
			local heading = math.atan2(-axis.z,axis.x) + draw(-.55,.55)
			local mark = spawn(ctx, "splat", SPLAT, v(at.x,y+.009,at.z), v(0,heading,0), v(width,1,width*draw(.7,1)), draw(8,12))
			-- Let the first drops arrive before the footprint reaches its full extent.
			world.set(mark.id, {size=mark.size*.08})
		end
	end
end

local function update(item, kind, dt)
	local remaining = item.life - item.age
	if kind == "splat" then
		if item.age < .35 or not item.spread or remaining < .6 then
			local scale = math.min(1, item.age/.32, remaining/.6)
			world.set(item.id, {size=item.size*math.max(.01,scale)})
			item.spread = item.age >= .32
		end
		return true
	end
	if not item.resting then
		item.velocity += DOWN*(10.8*dt)
		item.pos += item.velocity*dt
		if kind == "drop" then
			if item.floor and item.pos.y <= item.floor+.02 then return false end
			item.rot = facing(item.velocity)
		else
			item.rot += item.spin*dt
			local height = item.floor and item.floor + item.size.y*.36
			if height and item.pos.y <= height then
				item.pos = v(item.pos.x,height,item.pos.z)
				item.velocity = v(item.velocity.x*.36,math.abs(item.velocity.y)*.18,item.velocity.z*.36)
				item.spin *= .40
				item.resting = vector.magnitude(item.velocity) < .65
			end
		end
		world.set(item.id, {pos=item.pos, rot=item.rot})
	end
	if remaining < .28 then world.set(item.id, {size=item.size*math.max(.01,remaining/.28)}) end
	return true
end

function effects.tick(ctx, dt)
	for kind, items in live do
		for i=#items,1,-1 do
			local item = items[i]
			if item.owner ~= ctx then continue end
			item.age += dt
			if item.age >= item.life or not update(item,kind,dt) then
				world.remove(item.id)
				table.remove(items,i)
			end
		end
	end
end

function effects.clear(ctx)
	for _, items in live do
		for i=#items,1,-1 do
			if items[i].owner == ctx then world.remove(table.remove(items,i).id) end
		end
	end
end

return effects
