-- Port Meridian adapters for the original Growing Guns gun and upgrade functions.
local gg = require("growing_guns_data")
local catalog = {order={"pistol","launcher","rifle","shotgun","uzi","sniper"}}
local presets = {
    pistol={id="pistol",name="Pistol",kind="pistol",range=75},
    launcher={id="launcher",name="Grenade launcher",kind="launcher",range=75},
}

local function rifle(id, name, card)
    local gun = gg.gun
    local w = {damage_mult=1,fire_rate_mult=1,mag_size_bonus=0,reload_mult=1,
        spread=gun.spread,extra_projectiles=0,bullet_speed_mult=1,headshot_mult=1,
        bullet_color=vector.create(1,.9,.3)}
    if card then gg.cards[card](w) end
    local pellets = 1+w.extra_projectiles
    return {id=id,name=name,kind="hitscan",range=75,automatic=true,pellets=pellets,
        damage=gun.damage*w.damage_mult/22,spread=w.spread,
        interval=math.max(gun.fire_interval/w.fire_rate_mult,pellets/240),
        magazine=gun.mag_size+w.mag_size_bonus,reload=gun.reload_time/w.reload_mult,
        headshot=gun.headshot*w.headshot_mult,aimFov=if w.special=="zoom" then 30 else 49,
        velocity=w.bullet_speed_mult, color=w.bullet_color}
end
presets.rifle=rifle("rifle","Rifle")
presets.shotgun=rifle("shotgun","Shotgun","shotgun")
presets.uzi=rifle("uzi","Uzi","uzi")
presets.sniper=rifle("sniper","Sniper","sniper")

function catalog.get(id,seat)
    local builds=seat and seat.weaponBuilds
    return (builds and builds[id]) or presets[id] or presets.pistol
end

function catalog.exists(id)
    return type(id)=="string" and presets[id]~=nil
end

local limits = {
    pellets={1,8},magazine={1,60},damage={.1,6},spread={0,.30},
    interval={.06,2},reload={.15,5},velocity={.5,3},range={2,75},aimFov={22,65},headshot={1,4},
}

-- Detach validated numbers from the author VM before the story can change a seat.
function catalog.parameters(input)
    if type(input)~="table" or getmetatable(input)~=nil then return nil end
    local out={}
    for key,value in next,input do
        if key=="automatic" then
            if type(value)~="boolean" then return nil end
        else
            local lim=limits[key]
            if not lim or type(value)~="number" or not (value>=lim[1] and value<=lim[2]) then return nil end
            if (key=="pellets" or key=="magazine") and value%1~=0 then return nil end
        end
        out[key]=value
    end
    return out
end

function catalog.equip(seat,id,params)
    seat.weapon=id
    if params then
        local spec=table.clone(presets[id])
        for k,v in params do spec[k]=v end
        seat.weaponBuilds=seat.weaponBuilds or {}
        seat.weaponBuilds[id]=spec
    elseif seat.weaponBuilds then
        seat.weaponBuilds[id]=nil
    end
end

return catalog
