-- Shared by shootable cargo and authored story cues.
local explosions = {}
local v = vector.create
local crateSize = v(1.32,1.25,1.12)
local sites = {v(10.7,.625,-10.8), v(159,.625,65), v(165,.625,-105)}
local respawnTime = 15

function explosions.play(pos, radius)
    fx.explosion(pos, radius)
    local size = math.round(math.clamp(radius,2,24))
    sound.play("explosion_bang", {at=pos, radius=size, db=0})
    local rumbleDb = -10 + 3*math.clamp((size-6)/18,0,1)
    sound.play("explosion_rumble", {at=pos, radius=size, db=rumbleDb})
end

function explosions.placeCrates()
    local crates = {}
    for _,pos in sites do
        local id = world.spawn("cube", {pos=pos, size=crateSize, solid=true, tag="city",
            surface=30, color="#bd3829", roughness=.77})
        crates[id] = {pos=pos}
    end
    return crates
end

function explosions.hit(crate, id)
    if crate.respawnAt then return end
    crate.respawnAt = game.time() + respawnTime
    world.set(id, {visible=false, solid=false})
    explosions.play(crate.pos, 5)
end

-- A player can walk or park on the empty site while the crate is gone.
local function occupied(pos, fleet)
    for _,seat in fleet.players do
        if vector.magnitude(seat.pos-pos)<1.7 then return true end
    end
    for _,car in fleet.vehicles do
        if vector.magnitude(car.pos-pos)<3 then return true end
    end
    return false
end

function explosions.tick(crates, fleet)
    local now = game.time()
    for id,crate in crates do
        if crate.respawnAt and now >= crate.respawnAt then
            if occupied(crate.pos, fleet) then
                crate.respawnAt = now + .5
            else
                world.set(id, {visible=true, solid=true})
                crate.respawnAt = nil
            end
        end
    end
end

return explosions
