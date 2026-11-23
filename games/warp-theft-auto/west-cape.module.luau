-- West Cape is a flat coastal terrace. Roads, shoreline colliders and the map
-- use the cross-sections baked with its render mesh.
local data = require("west-cape-data")
local forest = require("forest")
local cape = {}
local v = vector.create
local CELL = 24
local MAX_SCALE = 1.08
local canopyTop = 0
forest.trunks(0,0,0,MAX_SCALE,function(_,_,_,h) canopyTop=math.max(canopyTop,h) end)
cape.groves = {{-690,-99,120,83},{-900,108,80,65}}
local groves = cape.groves
local function grid(x,z)
    return math.floor(x/CELL)*1024+math.floor(z/CELL)
end

function cape.wooded(x,z)
    for _,g in groves do
        if ((x-g[1])/g[3])^2+((z-g[2])/g[4])^2<=1 then return true end
    end
    return false
end

local function segmentDistance2(x,z,ax,az,bx,bz)
    local dx,dz=bx-ax,bz-az
    local t=math.clamp(((x-ax)*dx+(z-az)*dz)/(dx*dx+dz*dz),0,1)
    return (x-ax-t*dx)^2+(z-az-t*dz)^2
end

function cape.contains(x,z,r)
    local coast=data.coast
    if x<coast[1][1]+r or x>coast[#coast][1] then return false end
    for i=2,#coast do
        local a,b=coast[i-1],coast[i]
        if a[1]>x+r then break end
        if b[1]<x-r then continue end
        if x>=a[1] and x<=b[1] then
            local f=(x-a[1])/(b[1]-a[1])
            if z<a[2]+(b[2]-a[2])*f or z>a[3]+(b[3]-a[3])*f then return false end
        end
        if r>0 and (segmentDistance2(x,z,a[1],a[2],b[1],b[2])<r*r
            or segmentDistance2(x,z,a[1],a[3],b[1],b[3])<r*r) then return false end
    end
    return true
end

local function box(pos,size,yaw,visible,color)
    return world.spawn("cube",{pos=pos,size=size,rot=v(0,yaw,0),visible=visible,
        solid=true,tag="city",color=color or "#665d4e"})
end
local function shore(ax,az,bx,bz)
    local dx,dz=bx-ax,bz-az
    -- A narrow guard at the cliff top stops ground traffic but allows overflight.
    box(v((ax+bx)*.5,1.4,(az+bz)*.5),v(.32,3.0,math.sqrt(dx*dx+dz*dz)+.3),
        math.atan2(dx,dz),false)
end
local function clearRoad(x,z,radius)
    for _,path in data.roads do
        for i=2,#path do
            local a,b=path[i-1],path[i]
            if segmentDistance2(x,z,a[1],a[2],b[1],b[2])<radius*radius then return false end
        end
    end
    return true
end
local function trunkBodies(trunks)
    local bodies=table.create(#trunks)
    for _,t in trunks do
        bodies[#bodies+1]=box(v(t[1],t[4]*.5,t[2]),v(t[3]*2,t[4],t[3]*2),0,false)
    end
    return bodies
end

function cape.build(seed)
    local c={cells={},active={},viewerCells={},stands=0,trees=0}
    world.spawn("assets/west-cape.glb",{pos=v(0,0,0),solid=false,tag="west-cape"})
    local rows=data.coast
    for i=2,#rows do
        local a,b=rows[i-1],rows[i]
        local lo,hi=math.min(a[2],b[2]),math.max(a[3],b[3])
        box(v((a[1]+b[1])*.5,-1.5,(lo+hi)*.5),v(b[1]-a[1],3,hi-lo),0,false)
        shore(a[1],a[2],b[1],b[2])
        shore(a[1],a[3],b[1],b[3])
    end
    shore(rows[1][1],rows[1][2],rows[1][1],rows[1][3])

    -- Each forest draw covers one stand. Gaps in the canopy follow the actual
    -- road ribbon and reveal both its shoulders around bends.
    local function noise(i,j,salt)
        local n=(seed%2147483647 + (i+50)*7385609 + (j+20)*1934963 + salt*834927)%2147483647
        return (((n*48271)%2147483647)*16807%2147483647)/2147483647
    end
    for _,g in groves do
        for i=math.floor((g[1]-g[3])/27),math.ceil((g[1]+g[3])/27) do
            for j=math.floor((g[2]-g[4])/27),math.ceil((g[2]+g[4])/27) do
                local x,z=i*27+6*(noise(i,j,1)-.5),j*27+6*(noise(i,j,2)-.5)
                if not cape.wooded(x,z) or not clearRoad(x,z,21) then continue end
                local yaw=noise(i,j,3)*math.pi*2
                local scale=.90+(MAX_SCALE-.90)*noise(i,j,4)
                forest.place(x,0,z,yaw,scale)
                c.stands+=1
                forest.trunks(x,z,yaw,scale,function(tx,tz,r,h)
                    local key=grid(tx,tz)
                    local rows=c.cells[key]
                    if not rows then rows={}; c.cells[key]=rows end
                    rows[#rows+1]={tx,tz,r,h}
                    c.trees+=1
                end)
            end
        end
    end
    -- The fleet takes one snapshot of the island's static geometry in init.
    -- Cars then use Rapier's spatial index everywhere on the island.
    for key,trunks in c.cells do c.active[key]=trunkBodies(trunks) end
    return c
end

function cape.fleetReady(c)
    for _,ids in c.active do
        for _,id in ids do world.remove(id) end
    end
    c.active={}
end

-- Keep only neighbouring walk/flight collision cells in the general-purpose
-- world. Four local players share each cell and never duplicate its bodies.
function cape.update(c,players)
    if not c then return end
    local changed=false
    for id in c.viewerCells do
        if not players[id] then c.viewerCells[id]=nil; changed=true end
    end
    for id,p in players do
        local key=nil
        if p.pos.y<canopyTop+4 and p.pos.x< -490 and p.pos.x> -1020 then key=grid(p.pos.x,p.pos.z) end
        if c.viewerCells[id]~=key then c.viewerCells[id]=key; changed=true end
    end
    if not changed then return end
    local wanted={}
    for _,key in c.viewerCells do
        for dx=-1,1 do
            for dz=-1,1 do
                local k=key+dx*1024+dz
                if c.cells[k] then wanted[k]=true end
            end
        end
    end
    for k,ids in c.active do
        if not wanted[k] then
            for _,id in ids do world.remove(id) end
            c.active[k]=nil
        end
    end
    for k in wanted do
        if not c.active[k] then c.active[k]=trunkBodies(c.cells[k]) end
    end
end

function cape.roof(c,x,z,r)
    local height=0
    for ix=math.floor((x-r-.5)/CELL),math.floor((x+r+.5)/CELL) do
        for iz=math.floor((z-r-.5)/CELL),math.floor((z+r+.5)/CELL) do
            local trunks=c.cells[ix*1024+iz]
            if not trunks then continue end
            for _,t in trunks do
                local dx=math.max(math.abs(x-t[1])-t[3],0)
                local dz=math.max(math.abs(z-t[2])-t[3],0)
                if dx*dx+dz*dz<=r*r then height=math.max(height,t[4]) end
            end
        end
    end
    return height
end

-- Scripted walkers can cross the cape while every player is in town. Their
-- short movement steps query the stored trunk footprints outside the live cells.
function cape.remoteTrunk(c,pos,r)
    return c~=nil and not c.active[grid(pos.x,pos.z)] and cape.roof(c,pos.x,pos.z,r)>pos.y+.05
end

return cape
