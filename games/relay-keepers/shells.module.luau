-- Littoral hermit crabs. Their spiral shells act as live springboards.
local jump=require("jump")
local M={}
function M.start(specs)
    if not specs then return nil end
    local crabs={}
    for _,v in specs do
        table.insert(crabs,{x=v.from+(v.to-v.from)*(v.phase or 0),y=v.y,dir=1})
    end
    return crabs
end
function M.step(l,s,dt)
    local b=l.bell
    local hush=false
    if b then
        for _,r in s.crew do
            if r.on and math.abs(r.x-b[1])<0.57 and math.abs(r.y-b[2])<0.34 then hush=true end
        end
        if s.crate and b[2]==0 and math.abs(s.crate-b[1])<0.64 then hush=true end
    end
    if hush and not s.hush then sound.play("dock",{pitch=1.32,db=-10}) end
    s.hush=hush
    if hush then return end
    for i,v in l.crabs do
        local crab=s.crabs[i]
        local x=crab.x+crab.dir*dt*(v.speed or 1)
        if x>v.to then x=v.to-(x-v.to) crab.dir=-1
        elseif x<v.from then x=v.from+(v.from-x) crab.dir=1 end
        crab.x=math.clamp(x,v.from,v.to)
    end
end
function M.touch(s,r,prev)
    for _,c in s.crabs do
        if math.abs(r.x-c.x)<0.61 then
            local crown=c.y+0.62
            if prev>=crown-0.001 and r.y<=crown and r.vy<=0 then
                r.y=crown
                jump.launch(r,jump.shell,false)
                sound.play("jump",{pitch=0.68,db=-5})
                return
            elseif not s.hush and r.y<crown-0.07 and r.y+1>c.y+0.12 and (r.spawnGlow or 0)<=0 then
                r.y=-3
                return
            end
        end
    end
end
function M.draw(rect,xy,l,s)
    if not s.crabs then return end
    local ink,hot,rust,sand="#293640","#f48160","#ac4b50","#ffe0a0"
    if l.bell then
        local x,y=xy(l.bell[1],l.bell[2])
        rect(x-22,y-4,44,4,ink)
        rect(x-18,y-(if s.hush then 6 else 10),36,5,if s.hush then "#93e4be" else hot)
        rect(x-6,y-28,12,10,ink) rect(x-10,y-19,20,4,ink)
        rect(x-3,y-26,6,7,sand) rect(x-2,y-14,4,3,rust)
    end
    for _,c in s.crabs do
        local x,y=xy(c.x,c.y)
        local function a(dx,dy,w,h,col)
            rect(x+(if c.dir>0 then dx else -dx-w),y+dy,w,h,col)
        end
        for i=0,2 do
            local sway=if s.hush then 0 else math.sin(s.time*11+i*2)*2
            a(-18+i*12+sway,-9,6,8,ink) a(-21+i*12+sway,-3,9,3,rust)
        end
        a(-20,-16,42,9,ink) a(-17,-15,35,6,hot)
        a(-22,-27,8,15,ink) a(-18,-32,25,20,ink) a(-11,-35,14,5,ink) a(5,-26,6,14,ink)
        a(-17,-27,23,14,rust) a(-12,-31,16,18,sand) a(-17,-28,11,8,hot)
        a(-12,-26,12,3,rust) a(-3,-25,3,8,rust) a(-9,-19,6,3,rust)
        if s.hush then
            a(12,-16,9,2,ink)
        else
            a(13,-27,4,16,ink) a(20,-25,4,12,ink)
            a(13,-27,3,3,sand) a(20,-25,3,3,sand)
            a(21,-14,9,5,rust) a(29,-20,6,9,hot) a(33,-24,4,6,ink) a(28,-24,3,7,ink)
        end
    end
end
return M
