local v = vector.create

-- Shape, surface kind (also the ink group), size, offset, colour.
local parts = {
    {"capsule",1,v(.19,.31,.19),v(-.15,.39,0),"#4e4642"},
    {"capsule",1,v(.19,.31,.19),v(.15,.39,0),"#4e4642"},
    {"traveller.glb",0,v(1,1,1),v(0,0,0),"#ffffff"},
}

local function pose()
    local forward = v(-math.sin(state.yaw),0,-math.cos(state.yaw))
    local right = v(-forward.z,0,forward.x)
    local feet = v(state.x,state.z*.055,state.z)
    for i,part in parts do
        local offset = part[4]
        local stride = if i <= 2 then (if i == 1 then 1 else -1)*math.sin(state.phase) else 0
        local pos = feet + right*offset.x - forward*offset.z + v(0,offset.y,0) + forward*(stride*.16)
        world.set(state.body[i],{pos=pos,rot=v(stride*.2,state.yaw,0)})
    end
    local target = feet + forward*2.3 + right*.63 + v(0,1.46,0)
    local player = players.list()[1]
    if player then camera.set(player.id,{pos=feet-forward*4.5+right*.68+v(0,2.05,0),target=target,fov=49}) end
end

local function init()
    world.spawn("meadow.glb",{solid=false})
    state.x,state.z,state.yaw,state.phase = 0,0,0,0
    state.body = {}
    for _,part in parts do
        table.insert(state.body,world.spawn(part[1],{surface=part[2],size=part[3],color=part[5],solid=false}))
    end
    light.set({sun=v(-.52,.68,.50),sun_lux=43000,sky_lux=15000,exposure=12.7,fog_density=0})
    post.set({bloom=0,grain=0,vignette=0,saturation=1.16})
    pose()
end

local function tick(dt,input)
    local player = players.list()[1]
    local p = player and input[player.id]
    if p then
        state.yaw -= p.look_x
        local dx = p.move_x*math.cos(state.yaw)-p.move_y*math.sin(state.yaw)
        local dz = -p.move_x*math.sin(state.yaw)-p.move_y*math.cos(state.yaw)
        local length = math.max(1,math.sqrt(dx*dx+dz*dz))
        state.x = math.clamp(state.x+dx*dt*2.6/length,-8,8)
        state.z = math.clamp(state.z+dz*dt*2.6/length,-13,4)
        state.phase += math.min(1,math.sqrt(dx*dx+dz*dz))*dt*7.5
    end
    pose()
end

return {init=init,tick=tick}
