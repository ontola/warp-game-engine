-- West Cape joins the city at sea level. The inland ridge, roads, walking
-- and vehicle physics share the height samples baked with the visible mesh.
local data = require("west-cape-data")
local forest = require("forest")
local cape = {}
local v = vector.create
local terrain = data.terrain

function cape.height(x,z)
    local gx,gz = (x-terrain.x)/terrain.spacing,(z-terrain.z)/terrain.spacing
    local i,j = math.floor(gx),math.floor(gz)
    if i<0 or j<0 or i>=terrain.columns-1 or j>=terrain.rows-1 then return 0 end
    local fx,fz = gx-i,gz-j
    local k = 1+j*terrain.columns+i
    local h = terrain.heights
    local a,b,c,d = h[k],h[k+1],h[k+terrain.columns],h[k+terrain.columns+1]
    if fx+fz<=1 then return a+(b-a)*fx+(c-a)*fz end
    return d+(c-d)*(1-fx)+(b-d)*(1-fz)
end

local CELL = 24
local MAX_SCALE = 1.08
local canopyTop = 0
forest.trunks(0,0,0,MAX_SCALE,function(_,_,_,h) canopyTop=math.max(canopyTop,h) end)
-- The main wood follows sheltered ground, with windbreaks on the approach
-- and lower stands beyond the ridge. The same footprints appear on the map.
cape.groves = {
    {-690,-99,120,83}, {-900,108,80,65}, {-516,102,48,25},
    {-748,-191,76,32}, {-896,192,64,30}, {-1062,42,65,31},
}
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
-- Every placement checks only road segments in its own 24 m cell. Pad the
-- bins by the largest tested footprint, the original 21 m woodland clearance.
local roadCells={}
for _,path in data.roads do
    for i=2,#path do
        local a,b=path[i-1],path[i]
        local edge={a[1],a[2],b[1],b[2]}
        for ix=math.floor((math.min(a[1],b[1])-21)/CELL),math.floor((math.max(a[1],b[1])+21)/CELL) do
            for iz=math.floor((math.min(a[2],b[2])-21)/CELL),math.floor((math.max(a[2],b[2])+21)/CELL) do
                local key=ix*1024+iz
                roadCells[key]=roadCells[key] or {}
                table.insert(roadCells[key],edge)
            end
        end
    end
end
local function clearRoad(x,z,radius)
    local edges=roadCells[grid(x,z)]
    if edges then
        for _,e in edges do
            if segmentDistance2(x,z,e[1],e[2],e[3],e[4])<radius*radius then return false end
        end
    end
    return true
end

-- The middle saddle is wooded on both shoulders. The later high switchbacks
-- stay open to the coast. All roots follow the actual gravel centreline.
local function plantTrail(path,noise,trunk,centers)
    local distance,station,index,stands=0,285,0,0
    for i=2,#path do
        local a,b=path[i-1],path[i]
        local dx,dz=b[1]-a[1],b[2]-a[2]
        local length=math.sqrt(dx*dx+dz*dz)
        local nx,nz=-dz/length,dx/length
        while station<=distance+length and station<=517 do
            local t=(station-distance)/length
            local x,z=a[1]+t*dx,a[2]+t*dz
            for _,side in {-1,1} do
                local width=7.5+noise(index,side,41)*3.8
                local along=(noise(index,side,42)-.5)*5
                local scale=.81+.23*noise(index,side,43)
                for attempt=0,3 do
                    local offset=(width+2.4*attempt)*side
                    local cx,cz=x+nx*offset+nz*along,z+nz*offset-nx*along
                    local gx=cape.height(cx+2,cz)-cape.height(cx-2,cz)
                    local gz=cape.height(cx,cz+2)-cape.height(cx,cz-2)
                    local yaw=math.atan2(gx,gz)+(if noise(index,side,44)<.5 then math.pi else 0)
                    local base,clear=math.huge,true
                    forest.trailTrunks(cx,cz,yaw,scale,function(tx,tz)
                        if not clearRoad(tx,tz,5.25) then clear=false end
                        base=math.min(base,cape.height(tx,tz))
                    end)
                    if not clear then continue end
                    world.spawn("assets/plants/trail-copse.gltf",{pos=v(cx,base,cz),rot=v(0,yaw,0),
                        size=v(scale,scale,scale),tag="trail-woodland",solid=false})
                    forest.trailTrunks(cx,cz,yaw,scale,function(tx,tz,r,h)
                        trunk(tx,tz,r,base+h,cape.height(tx,tz)-.04)
                    end)
                    centers[#centers+1]={cx,cz}
                    stands+=1
                    break
                end
            end
            index+=1
            station+=12.6+4.4*noise(index,0,45)
        end
        distance+=length
        if station>517 then break end
    end
    return stands
end
local function trunkBodies(trunks)
    local bodies=table.create(#trunks)
    for _,t in trunks do
        local base=t[5] or 0
        bodies[#bodies+1]=box(v(t[1],(t[4]+base)*.5,t[2]),v(t[3]*2,t[4]-base,t[3]*2),0,false)
    end
    return bodies
end

function cape.build(seed)
    local c={cells={},active={},viewerCells={},stands=0,slopeTrees=0,trees=0}
    world.spawn("assets/west-cape.glb",{pos=v(0,0,0),solid=false,tag="west-cape"})
    world.heightfield({origin=v(terrain.x,0,terrain.z),spacing=terrain.spacing,
        columns=terrain.columns,heights=terrain.heights,tag="city",grip="grass",
        roads={{points=data.roads[2],width=9,grip="asphalt"},
              {points=data.roads[3],width=5.2,grip="gravel"}}})
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
        -- Independent lanes for canopy gaps, heading and root position.
        local n=bit32.bxor(seed,i*7385609,j*1934963,salt*834927)
        n=bit32.band(bit32.bxor(n,bit32.rshift(n,16))*40503,0xffffffff)
        n=bit32.band(bit32.bxor(n,bit32.rshift(n,13))*38637,0xffffffff)
        return bit32.bxor(n,bit32.rshift(n,16))/4294967296
    end
    local function trunk(x,z,r,top,base)
        local key=grid(x,z)
        local trunks=c.cells[key]
        if not trunks then trunks={}; c.cells[key]=trunks end
        trunks[#trunks+1]={x,z,r,top,base}
        c.trees+=1
    end
    local trailCenters={}
    c.trailStands=plantTrail(data.roads[3],noise,trunk,trailCenters)
    local function nearTrailWood(x,z)
        for _,p in trailCenters do
            if (p[1]-x)^2+(p[2]-z)^2<42^2 then return true end
        end
        return false
    end
    for _,g in groves do
        for i=math.floor((g[1]-g[3])/27),math.ceil((g[1]+g[3])/27) do
            for j=math.floor((g[2]-g[4])/27),math.ceil((g[2]+g[4])/27) do
                local x,z=i*27+6*(noise(i,j,1)-.5),j*27+6*(noise(i,j,2)-.5)
                if ((x-g[1])/g[3])^2+((z-g[2])/g[4])^2>1 or not clearRoad(x,z,21) then continue end
                local yaw=noise(i,j,3)*math.pi*2
                local scale=.90+(MAX_SCALE-.90)*noise(i,j,4)
                local low,high=math.huge,-math.huge
                forest.trunks(x,z,yaw,scale,function(tx,tz)
                    local y=cape.height(tx,tz)
                    low,high=math.min(low,y),math.max(high,y)
                end)
                if high-low<1.5 then
                    -- Seating the shared mesh at its lowest root keeps every stem planted.
                    forest.place(x,low,z,yaw,scale)
                    c.stands+=1
                    forest.trunks(x,z,yaw,scale,function(tx,tz,r,h)
                        trunk(tx,tz,r,low+h,cape.height(tx,tz)-.04)
                    end)
                elseif not nearTrailWood(x,z) then
                    -- On slopes a few independently rooted broadleaves leave clear
                    -- views of the higher heath and rock through the forest edge.
                    for n=1,if noise(i,j,20)>.88 then 2 else 1 do
                        local tx=x+19*(noise(i,j,n+7)-.5)
                        local tz=z+19*(noise(i,j,n+10)-.5)
                        local y=cape.height(tx,tz)
                        local k=1.22+.78*noise(i,j,n+13)
                        world.spawn("assets/plants/street-tree.glb",{pos=v(tx,y-.04,tz),rot=v(0,yaw+n*2.4,0),
                            size=v(k,k,k),solid=false,tag="woodland-tree"})
                        trunk(tx,tz,.20*k,y+6.2*k,y-.04)
                        c.slopeTrees+=1
                    end
                end
            end
        end
    end
    -- Two broken hedgerows border the meadow at the city gate. Each shrub
    -- reuses the park mesh and sits independently on its own patch of ground.
    for row=1,2 do
        for n=1,8 do
            if noise(n,row,24)<.22 then continue end
            local x=-207-n*14
            local z=(if row==1 then -1 else 1)*(19+n*1.43)+2*noise(n,row,25)
            if not (x>=-281 and x<=-230 and z>=0 and z<=48) and clearRoad(x,z,10) then
                local k=1.05+.40*noise(n,row,26)
                world.spawn("assets/plants/shrub-bed.glb",{pos=v(x,cape.height(x,z)-.05,z),
                    rot=v(0,.18*(row*2-3),0),size=v(k*2.6,k,k),solid=false,tag="cape-hedge"})
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
        if p.pos.x< -178 and p.pos.x>data.coast[1][1] and p.pos.y<cape.height(p.pos.x,p.pos.z)+canopyTop+4 then
            key=grid(p.pos.x,p.pos.z)
        end
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
