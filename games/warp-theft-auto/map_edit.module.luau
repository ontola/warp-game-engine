-- Coordinates are authored in metres. Apply a complete, validated layout between ticks.
local authored = require("west_cape_layout")
local derived = require("west-cape-data")
local bakedKey = require("west-cape-bake").key
local cape = require("west-cape")
local city = require("world")
local models = require("vehicles")
local stories = require("story_board")
local waypoint = require("waypoint")
local map = {}
local v = vector.create

local function copy(value)
    if type(value) ~= "table" then return value end
    local result = {}
    for k, item in value do result[k] = copy(item) end
    return result
end

local function equal(a, b)
    if a == b then return true end
    if type(a) ~= "table" or type(b) ~= "table" then return false end
    for k, value in a do if not equal(value, b[k]) then return false end end
    for k in b do if a[k] == nil then return false end end
    return true
end

function map.initial(state)
    state.mapFile = copy(authored)
    -- The runtime keeps plain game data in state, never a compiler function.
    local result = table.clone(derived)
    result.build = nil
    return result, result.bakeKey == bakedKey
end

function map.definition(state)
    return copy(state.layoutOverride or state.mapFile)
end

-- Keep grounded actors at their current X/Z and at the same distance above
-- the surface. Airborne actors retain altitude unless the raised land meets them.
local function support(old, new, pos, radius)
    if pos.x >= -city.BOUND then return 0 end
    local before = cape.heightIn(old.terrain, pos.x, pos.z)
    local after = cape.heightIn(new.terrain, pos.x, pos.z)
    if cape.containsIn(old.coast, pos.x, pos.z, radius) and
        not cape.containsIn(new.coast, pos.x, pos.z, radius) and pos.y < before + 2 then
        return nil
    end
    if pos.y <= before + .85 then return after-before end
    return math.max(0, after-pos.y)
end

local function install(state, definition, data)
    local source = state.layoutOverride or state.mapFile
    if equal(source, definition) then return "West Cape" end
    local current = state.city.cape.data
    local moves = {}
    for _, car in state.fleet.vehicles do
        local dy = support(current, data, car.pos, 1)
        if not dy then return nil, "Move the vehicle off the shoreline being removed, then apply again" end
        if dy ~= 0 then table.insert(moves, {car=car, dy=dy}) end
    end
    for _, seat in state.fleet.players do
        if seat.mode ~= "walk" then continue end
        local dy = support(current, data, seat.pos, .3)
        if not dy then return nil, "Move the player off the shoreline being removed, then apply again" end
        if dy ~= 0 then table.insert(moves, {seat=seat, dy=dy}) end
    end
    local ok, err = city.replaceLayout(state.city, data, data.bakeKey == bakedKey)
    if not ok then return nil, err end
    for _, move in moves do
        local delta = v(0, move.dy, 0)
        local car = move.car
        if move.seat then
            local seat = move.seat
            seat.pos += delta
            world.place(seat.body, world.get(seat.body).pos + delta)
        elseif car.native then
            vehicles.relocate(car.native, car.pos + delta)
            models.sync(car)
        else
            car.pos += delta
            world.place(car.body, world.get(car.body).pos + delta)
        end
    end
    state.layoutOverride = if equal(authored, definition) then nil else copy(definition)
    stories.layoutChanged(state)
    waypoint.update(state)
    return "West Cape"
end

function map.apply(state, definition)
    if type(definition) ~= "table" or definition.kind ~= "west-cape-layout" then
        return nil, "Expected a West Cape layout definition"
    end
    local data, err = derived.build(definition)
    if not data then return nil, err end
    return install(state, definition, data)
end

function map.reload(state)
    if not state.city.cape.data then return end
    cape.useLayout(state.city.cape.data)
    if not equal(state.mapFile, authored) then
        local ok, err = map.apply(state, authored)
        if not ok then game.log("Map edit: " .. err); return end
        state.mapFile = copy(authored)
        state.layoutOverride = nil
    end
end

return map
