-- Curbside furniture occupies the strip between the asphalt edge (6 m from
-- the street axis) and the pedestrian circuit (8 m). Models share mesh data.
local furniture = {}
local v = vector.create
local LAMP = "assets/street/lamp.glb"
local BIN = "assets/street/bin.glb"

local function clearFrontage(city, pos)
    for _, building in city.buildings do
        local door = building.door
        if math.abs(pos.x-door.x) < 1.8 and pos.z >= door.z-.5 and pos.z < door.z+16 then
            return false
        end
    end
    for _, tree in city.plantSites do
        if vector.magnitude(pos-tree) < 1.65 then return false end
    end
    return true
end

local function site(city, curbStart, tangent, step, offset, avoid)
    for _,along in {offset,step-offset,13,step-13} do
        local pos = curbStart+tangent*along
        if clearFrontage(city,pos) and (not avoid or vector.magnitude(avoid-pos)>=1.8) then
            return pos
        end
    end
    return nil
end

local function lamp(city, pos, direction)
    local yaw = math.atan2(direction.x,direction.z)
    world.spawn(LAMP,{pos=pos,rot=v(0,yaw,0),solid=false,tag="street-lamp"})
    table.insert(city.lamps,pos+direction*1.87+v(0,5.23,0))
end

local function bin(pos, direction)
    local yaw = math.atan2(direction.x,direction.z)
    world.spawn(BIN,{pos=pos,rot=v(0,yaw,0),solid=true,tag="street-bin"})
end

function furniture.build(city, step, noise)
    for axis=1,2 do
        local vertical = axis==1
        local tangent = if vertical then v(0,0,1) else v(1,0,0)
        for street=-3,3 do
            local line = street*step
            for block=-3,2 do
                -- Alternate opposite curbs on successive blocks. Perimeter
                -- furniture faces the road from the inhabited side of the island.
                local side = if street==-3 then 1 elseif street==3 then -1
                    elseif (street+block+city.seed+axis)%2==0 then 1 else -1
                local base = block*step
                local offset = 16+24*noise(city.seed,street,block,101+axis)
                local binOffset = if offset<28 then 34+6*noise(city.seed,street,block,105+axis)
                    else 16+6*noise(city.seed,street,block,105+axis)

                local starter = vertical and street==0 and block==0
                local busStop = vertical and street==0 and block==-1
                if starter then side, offset, binOffset = 1, 17.4, 14.1
                elseif busStop then side, offset, binOffset = -1, 19, 31.9 end

                local towardsRoad = if vertical then v(-side,0,0) else v(0,0,-side)
                local start = if vertical then v(line,0,base) else v(base,0,line)
                -- Each downtown frontage is lit. Alternate quieter blocks,
                -- along the seawall, lights mark every third block.
                local phase = city.seed+street*3+axis
                local streetRing = math.abs(street)
                local hasLamp = streetRing==0
                    or (streetRing==1 and (block==-1 or block==0))
                    or (streetRing<3 and (block+phase)%2==0)
                    or (streetRing==3 and (block+phase)%3==0)
                local pole
                if hasLamp then
                    pole = site(city,start-towardsRoad*6.74,tangent,step,offset,nil)
                    if pole then lamp(city,pole,towardsRoad) end
                end

                -- Every central transit block gets a bin. Secondary streets
                -- use a service interval of three or six blocks per curb.
                local hasBin = if streetRing==0 then (block>=-2 and block<=1)
                        or block==(if phase%2==0 then -3 else 2)
                    elseif streetRing==1 then (block+phase)%3==0
                    elseif streetRing==2 or street==3 then (block+phase)%6==0
                    else false
                if hasBin then
                    local basket = site(city,start-towardsRoad*6.78,tangent,step,binOffset,pole)
                    if basket then bin(basket,-towardsRoad) end
                end
            end
        end
    end
end

return furniture
