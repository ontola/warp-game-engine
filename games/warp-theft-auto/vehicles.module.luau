local util = require("kit/util")
local v = vector.create
local models = {}
local PAINT = { "sedan-ochre", "sedan", "sedan-pearl", "sedan-vermilion" }
local BASE = "assets/cars/"
local OFFSET = { v(-.759,0,-1.262), v(.759,0,-1.262), v(-.759,0,1.262), v(.759,0,1.262) }

local paints={ochre=PAINT[1],teal=PAINT[2],pearl=PAINT[3],vermilion=PAINT[4]}

function models.car(pos, yaw, index, paint)
	local model = BASE .. (paints[paint] or PAINT[(index-1)%#PAINT+1]) .. ".gltf"
	local body = world.spawn(model, {pos=pos+v(0,.33,0), rot=v(0,yaw,0), tag="vehicle"})
	local wheels = {}
	for i, offset in OFFSET do
		local name = if offset.x < 0 then "wheel-left" else "wheel-right"
		wheels[i] = world.spawn(BASE .. name .. ".gltf", {pos=offset, solid=false, tag="wheel"}, body)
	end
	return {kind="car", index=index, paint=paint, pos=pos, yaw=yaw, speed=0, body=body, wheels=wheels,
		native=vehicles.spawn(pos,yaw), rpm=950, gear=1, load=0, slip=0, contact=0, loose=0, soft=0, throttle=0}
end

-- Warp Humans' seated pose uses a floor anchor and faces +Z; sedans face -Z.
function models.seat(car,side)
	return car.pos+util.right(car.yaw)*(.44*side)+util.forward(car.yaw)*.04+v(0,.02,0),car.yaw+math.pi
end

function models.drive(input)
	local controls
	if input then
		local held = input.held or {}
		controls = {
			throttle = math.max(input.move_y or 0, if held.fire then 1 else 0, 0),
			brake = math.max(-(input.move_y or 0), if held.aim then 1 else 0, 0),
			steer = input.move_x or 0, handbrake = if held.south then 1 else 0,
		}
	end
	return controls
end

function models.sync(car)
	local p = vehicles.get(car.native)
	car.pos, car.yaw, car.active = p.pos - v(0,.33,0), p.rot.y, not p.parked
	car.speed, car.gear, car.rpm, car.cut = p.speed, p.gear, p.rpm, p.cut
	car.slip, car.torque, car.load = p.slip, p.torque, p.load
	car.contact, car.loose, car.soft = p.contact, p.loose, p.soft
	world.set(car.body, {pos=p.pos, rot=p.rot})
	for j, wheel in car.wheels do
		local w = p.wheels[j]
		world.set(wheel, {pos=OFFSET[j]+v(0,w.x,0), rot=v(w.z,w.y,0)})
	end
end

-- Skids, bubble cabin, fairing, tapered tail and articulated rotors share the
-- root's interpolated pose. The body centre sits one metre above the skids.
function models.helicopter(pos, yaw)
	local body = world.spawn("cube", {pos=pos+v(0,1,0), rot=v(0,yaw,0), size=v(2.2,2,4.8), visible=false, tag="vehicle"})
	local hull, glass, keel, steel, cream = "#db623d", "#224a5b", "#2b3940", "#778889", "#eedbaf"
	local function part(shape, offset, size, color, rot, parent)
		return world.spawn(shape, {pos=offset, size=size, color=color, rot=rot or util.ZERO, solid=false, tag="airframe"}, parent or body)
	end
	part("sphere", v(0,.12,-.14), v(2.4,1.82,3.58), hull)
	part("sphere", v(0,.34,-.70), v(2.30,1.52,2.92), glass)
	part("cube", v(0,.23,.25), v(2.28,.14,1.90), cream)
	part("sphere", v(0,1.02,.38), v(1.05,.66,2.25), hull)
	part("cube", v(0,.25,2.56), v(.42,.43,3.55), cream, v(-.12,0,0))
	part("cube", v(0,.85,4.17), v(.12,1.4,.60), hull, v(.24,0,0))
	part("cube", v(0,.46,3.63), v(1.7,.09,.40), hull)
	for _,side in {-1,1} do
		part("capsule", v(.98*side,-.91,-.12), v(.13,1.19,.13), keel, v(math.pi/2,0,0))
		for _,z in {-.77,.70} do
			part("cube", v(.78*side,-.65,z), v(.11,.64,.13), steel, v(0,0,-.48*side))
		end
	end
	part("cube", v(0,1.48,.15), v(.15,.55,.15), steel)
	local rotor = part("cube", v(0,1.78,.15), v(8.0,.06,.20), keel)
	part("cube", util.ZERO, v(.20,.06,8.0), keel, nil, rotor)
	local tail = part("cube", v(.16,.97,4.16), v(.07,1.43,.13), keel)
	return {kind="helicopter", body=body, rotor=rotor, tail=tail, pos=pos, yaw=yaw,
		vel=util.ZERO, speed=0, angle=0, grounded=true}
end

function models.fly(c, input, dt, city)
	local held = if input then input.held or {} else {}
	local steer = if input then input.move_x or 0 else 0
	local thrust = if input then input.move_y or 0 else 0
	c.yaw -= steer * dt * 1.65
	local up = if held.south then 1 else 0
	local down = if held.right_shoulder or held.east or not c.owner then 1 else 0
	local target = util.forward(c.yaw) * thrust * 26
	local h = util.damp(util.flat(c.vel), target, 2.4, dt)
	local vy = util.approach(c.vel.y, (up-down)*10, 18*dt)
	if c.pos.y >= 85 then vy = math.min(vy,0) end
	local out, contact = world.move(c.body, v(h.x,vy-.02,h.z), dt, {tag="city"})
	local center = world.get(c.body).pos
	local bounded = city.flight_bounds(center)
	if not city.on_land(bounded.x,bounded.z,0) then
		contact = false
		if bounded.y < 1.45 then
			bounded = v(bounded.x,1.45,bounded.z)
			out = v(out.x,math.max(0,out.y),out.z)
		end
	end
	world.set(c.body, {pos=bounded, rot=v(-thrust*.09,c.yaw,steer*.11)})
	c.pos, c.vel = bounded-v(0,1,0), out
	c.grounded, c.speed = contact, vector.magnitude(out)
	if contact and not c.owner then c.vel, c.speed = util.ZERO, 0 end
	c.angle = (c.angle + dt * (if c.owner then 27 else 9)) % (2*math.pi)
	world.set(c.rotor, {rot=v(0,c.angle,0)})
	world.set(c.tail, {rot=v(c.angle*2,0,0)})
end

return models
