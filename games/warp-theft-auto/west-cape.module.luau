-- West Cape joins the city at sea level. The inland ridge, roads, walking
-- and vehicle physics share the height samples baked with the visible mesh.
local default = table.clone(require("west-cape-data"))
default.build=nil
local bakedKey=require("west-cape-bake").key
local meshes = require("cape_mesh")
local data = default
local forest = require("forest")
-- Older club hosts can still run the full-detail woodland.
local phone = game.phone and game.phone()
local standMesh = if phone then "assets/plants/forest-stand-low.glb" else "assets/plants/forest-stand.glb"
local trailMesh = if phone then "assets/plants/trail-copse-low.gltf" else "assets/plants/trail-copse.gltf"
local slopeMesh = if phone then "assets/plants/street-tree-low.glb" else "assets/plants/street-tree.glb"
local cape = {}
local v = vector.create
function cape.heightIn(terrain,x,z)
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

function cape.height(x,z)
    return cape.heightIn(data.terrain,x,z)
end

local CELL, ROAD_CELL = 12,48
local STRIDE = 2048
local MAX_SCALE = 1.08
local canopyTop = 0
forest.trunks(0,0,0,MAX_SCALE,function(_,_,_,h) canopyTop=math.max(canopyTop,h) end)
local function grid(x,z)
    return math.floor(x/CELL)*STRIDE+math.floor(z/CELL)
end

function cape.wooded(x,z)
    for _,g in data.groves do
        if ((x-g[1])/g[3])^2+((z-g[2])/g[4])^2<=1 then return true end
    end
    return false
end

local function segmentDistance2(x,z,ax,az,bx,bz)
    local dx,dz=bx-ax,bz-az
    local t=math.clamp(((x-ax)*dx+(z-az)*dz)/(dx*dx+dz*dz),0,1)
    return (x-ax-t*dx)^2+(z-az-t*dz)^2
end

function cape.containsIn(coast,x,z,r)
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

function cape.contains(x,z,r)
    return cape.containsIn(data.coast,x,z,r)
end

function cape.layout()
    return data
end

-- Project a destination onto the outer shoreline of the attached city and cape.
-- The returned axis points out to sea; its quarter turn follows the quay.
function cape.shoreFrame(x,z)
    local best,px,pz,nx,nz=math.huge,0,0,0,1
    local function edge(ax,az,bx,bz)
        local dx,dz=bx-ax,bz-az
        local len2=dx*dx+dz*dz
        local t=math.clamp(((x-ax)*dx+(z-az)*dz)/len2,0,1)
        local tx,tz=ax+t*dx,az+t*dz
        local d=(x-tx)^2+(z-tz)^2
        if d<best then
            local len=math.sqrt(len2)
            best,px,pz,nx,nz=d,tx,tz,dz/len,-dx/len
        end
    end
    local coast=data.coast
    for i=2,#coast do
        local a,b=coast[i-1],coast[i]
        edge(a[1],a[2],b[1],b[2])
        edge(b[1],b[3],a[1],a[3])
    end
    local first,last=coast[1],coast[#coast]
    edge(first[1],first[3],first[1],first[2])
    edge(last[1],last[2],178,-178)
    edge(178,-178,178,178)
    edge(178,178,last[1],last[3])
    return px,pz,nx,nz
end

-- Keep the live cape, its Rapier solids and a replacement being staged separate.
local function keep(c,id,solid)
    table.insert(c.entities,id)
    if solid then table.insert(c.solids,id) end
    return id
end
local function stage(c,asset,opts)
    table.insert(c.pending,{asset=asset,opts=opts})
end
local function boxOptions(pos,size,yaw,visible,color)
    return {pos=pos,size=size,rot=v(0,yaw,0),visible=visible,
        solid=true,tag="city",color=color or "#665d4e"}
end
local function stageBox(c,pos,size,yaw,visible,color)
    stage(c,"cube",boxOptions(pos,size,yaw,visible,color))
end
local function shore(c,ax,az,bx,bz)
    local dx,dz=bx-ax,bz-az
    stageBox(c,v((ax+bx)*.5,1.4,(az+bz)*.5),v(.32,3.0,math.sqrt(dx*dx+dz*dz)+.3),
        math.atan2(dx,dz),false)
end
-- Every placement checks only road segments in its spatial cell. Pad the
-- bins by the largest tested footprint, the original 21 m woodland clearance.
local roadCells={}
local boundWest,boundZ
function cape.useLayout(layout)
    data=layout or default
    roadCells={}
    local coast=data.coast
    boundWest=coast[1][1]-30
    boundZ=0
    for _,row in coast do boundZ=math.max(boundZ,-row[2],row[3]) end
    boundZ+=48
    for route,path in data.roads do
        local extra=(if route==3 then data.widths.gravel-5.2 else data.widths.asphalt-9)*.5
        for i=2,#path do
            local a,b=path[i-1],path[i]
            local edge={a[1],a[2],b[1],b[2],extra}
            local pad=21+math.max(0,extra)
            -- Index short sections of the line. An 8 km diagonal must not fill its
            -- whole bounding rectangle just to find trees beside its shoulders.
            local dx,dz=b[1]-a[1],b[2]-a[2]
            local count=math.max(1,math.ceil(math.max(math.abs(dx),math.abs(dz))/ROAD_CELL))
            local ax,az=a[1],a[2]
            for j=1,count do
                local bx,bz=a[1]+dx*j/count,a[2]+dz*j/count
                for ix=math.floor((math.min(ax,bx)-pad)/ROAD_CELL),math.floor((math.max(ax,bx)+pad)/ROAD_CELL) do
                    for iz=math.floor((math.min(az,bz)-pad)/ROAD_CELL),math.floor((math.max(az,bz)+pad)/ROAD_CELL) do
                        local key=ix*STRIDE+iz
                        local bin=roadCells[key]
                        if not bin then bin={}; roadCells[key]=bin end
                        if bin[#bin]~=edge then table.insert(bin,edge) end
                    end
                end
                ax,az=bx,bz
            end
        end
    end
end

function cape.flightBounds()
    return boundWest,boundZ
end
if default.coast then cape.useLayout(default) end

local function clearRoad(x,z,radius)
    local edges=roadCells[math.floor(x/ROAD_CELL)*STRIDE+math.floor(z/ROAD_CELL)]
    if edges then
        for _,e in edges do
            local r=math.max(0,radius+e[5])
            if segmentDistance2(x,z,e[1],e[2],e[3],e[4])<r*r then return false end
        end
    end
    return true
end

-- The middle saddle is wooded on both shoulders. The later high switchbacks
-- stay open to the coast. All roots follow the actual gravel centreline.
local function plantTrail(c,path,noise,trunk,centers)
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
                        if not clearRoad(tx,tz,5.25) or not cape.contains(tx,tz,1) then clear=false end
                        base=math.min(base,cape.height(tx,tz))
                    end)
                    if not clear then continue end
                    stage(c,trailMesh,{pos=v(cx,base,cz),rot=v(0,yaw,0),
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
        bodies[#bodies+1]=world.spawn("cube",boxOptions(v(t[1],(t[4]+base)*.5,t[2]),v(t[3]*2,t[4]-base,t[3]*2),0,false))
    end
    return bodies
end

local function populate(c,seed,limit)
    local rows=data.coast
    for i=2,#rows do
        local a,b=rows[i-1],rows[i]
        local lo,hi=math.min(a[2],b[2]),math.max(a[3],b[3])
        stageBox(c,v((a[1]+b[1])*.5,-1.5,(lo+hi)*.5),v(b[1]-a[1],3,hi-lo),0,false)
        shore(c,a[1],a[2],b[1],b[2])
        shore(c,a[1],a[3],b[1],b[3])
    end
    shore(c,rows[1][1],rows[1][2],rows[1][1],rows[1][3])

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
    c.trailStands=plantTrail(c,data.roads[3],noise,trunk,trailCenters)
    local function nearTrailWood(x,z)
        for _,p in trailCenters do
            if (p[1]-x)^2+(p[2]-z)^2<42^2 then return true end
        end
        return false
    end
    for _,g in data.groves do
        for i=math.floor((g[1]-g[3])/27),math.ceil((g[1]+g[3])/27) do
            for j=math.floor((g[2]-g[4])/27),math.ceil((g[2]+g[4])/27) do
                if #c.pending>limit or c.trees>8192 then return end
                local x,z=i*27+6*(noise(i,j,1)-.5),j*27+6*(noise(i,j,2)-.5)
                if ((x-g[1])/g[3])^2+((z-g[2])/g[4])^2>1 or not clearRoad(x,z,21) or not cape.contains(x,z,16) then continue end
                local yaw=noise(i,j,3)*math.pi*2
                local scale=.90+(MAX_SCALE-.90)*noise(i,j,4)
                local low,high=math.huge,-math.huge
                forest.trunks(x,z,yaw,scale,function(tx,tz)
                    local y=cape.height(tx,tz)
                    low,high=math.min(low,y),math.max(high,y)
                end)
                if high-low<1.5 then
                    -- Seating the shared mesh at its lowest root keeps every stem planted.
                    stage(c,standMesh,forest.options(x,low,z,yaw,scale))
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
                        stage(c,slopeMesh,{pos=v(tx,y-.04,tz),rot=v(0,yaw+n*2.4,0),
                            size=v(k,k,k),solid=false,tag="woodland-tree"})
                        trunk(tx,tz,forest.streetRadius*k,y+forest.streetHeight*k,y-.04)
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
            local yard=data.places.grainYard
            if not (math.abs(x-yard[1])<26 and math.abs(z-yard[2])<26) and clearRoad(x,z,10) and cape.contains(x,z,5) then
                local k=1.05+.40*noise(n,row,26)
                stage(c,"assets/plants/shrub-bed.glb",{pos=v(x,cape.height(x,z)-.05,z),
                    rot=v(0,.18*(row*2-3),0),size=v(k*2.6,k,k),solid=false,tag="cape-hedge"})
            end
        end
    end
    -- The farm and lookout have solid footprints for walkers, tyres and roof queries.
    local function structure(x,y,z,w,h,d,color,visible)
        stageBox(c,v(x,y,z),v(w,h,d),0,visible,color)
        table.insert(c.structures,{x=x,z=z,halfW=w*.5,halfD=d*.5,top=y+h*.5})
    end
    local yard=data.places.grainYard
    local x,z=yard[1],yard[2]
    local y=cape.height(x,z)
    stage(c,"assets/grain-yard.glb",{pos=v(x,y,z),solid=false,tag="grain-yard"})
    for _,bin in {{-4.9,10.75},{4.9,9.55}} do
        structure(x+bin[1],y+bin[2]/2,z,7.25,bin[2],7.25,"#42626c",false)
    end
    structure(x,y+7.6,z+4.85,2.12,15.2,2.16,"#42626c",false)
    local lookout=data.places.lookout
    x,z=lookout[1],lookout[2]
    y=cape.height(x,z)
    for _,dx in {-2.5,2.5} do
        for _,dz in {-2,2} do structure(x+dx,y+1.65,z+dz,.23,3.3,.23,"#73573d",true) end
    end
    structure(x,y+3.4,z,6.5,.28,5.5,"#a24433",true)
    for _,dz in {-1.55,1.55} do structure(x,y+.28,z+dz,3.8,.55,.55,"#84613f",true) end
end

function cape.installSolids(c)
    local boxes=table.create(c.trees)
    for _,trunks in c.cells do
        for _,t in trunks do
            local base=t[5]
            table.insert(boxes,{pos=v(t[1],(t[4]+base)*.5,t[2]),size=v(t[3]*2,t[4]-base,t[3]*2)})
        end
    end
    vehicles.set_static_region("west-cape",c.solids,boxes)
end

local function removeCape(c)
    for _,ids in c.active do
        for _,id in ids do world.remove(id) end
    end
    for _,id in c.entities do world.remove(id) end
end

-- Upper bound for all walking and flying observers. Neighbourhoods can overlap or
-- have their centre in an empty cell, so accumulate from each occupied cell.
local function trunkBudget(c)
    local neighbours={}
    local largest=0
    for key,trees in c.cells do
        for dx=-1,1 do
            for dz=-1,1 do
                local k=key+dx*STRIDE+dz
                neighbours[k]=(neighbours[k] or 0)+#trees
                largest=math.max(largest,neighbours[k])
            end
        end
    end
    return math.min(4*largest,c.trees)
end

local function instantiate(c,baked)
    local id,err
    if baked then id=world.spawn("assets/west-cape.glb",{pos=v(0,0,0),solid=false,tag="west-cape"})
    else
        id,err=meshes.spawn(data)
        if not id then return nil,err end
    end
    keep(c,id)
    local t=data.terrain
    keep(c,world.heightfield({origin=v(t.x,0,t.z),spacing=t.spacing,
        columns=t.columns,heights=t.heights,tag="city",grip="grass",
        roads={{points=data.roads[1],width=data.widths.asphalt,grip="asphalt"},
               {points=data.roads[2],width=data.widths.asphalt,grip="asphalt"},
               {points=data.roads[3],width=data.widths.gravel,grip="gravel"}}}),true)
    for _,o in c.pending do keep(c,world.spawn(o.asset,o.opts),o.opts.solid) end
    c.pending=nil
    return true
end

function cape.build(seed,layout,baked,previous)
    local old=data
    layout=layout or default
    if not layout.coast then return nil,layout.error or "Invalid cape layout" end
    if layout.build then
        layout=table.clone(layout)
        layout.build=nil
    end
    cape.useLayout(layout)
    local c={data=layout,cells={},active={},viewerCells={},pending={},entities={},solids={},structures={},stands=0,slopeTrees=0,trees=0}
    local free=world.capacity()-world.count()
    local oldCount=0
    if previous then
        oldCount=#previous.entities
        for _,ids in previous.active do oldCount+=#ids end
    end
    populate(c,seed,free)
    local needed=#c.pending+2
    local freeAfter=free+oldCount-needed
    local function reject(message)
        removeCape(c)
        cape.useLayout(old)
        return nil,message
    end
    if needed>free or c.trees>8192 then return reject("This layout exceeds available cape geometry slots") end
    if previous and (freeAfter<trunkBudget(c)+32 or (#previous.entities<needed and freeAfter<300)) then
        return reject("This layout needs more room for resident objects and forest collision")
    end
    local ok,message=instantiate(c,baked~=false and data.bakeKey==bakedKey)
    if not ok then return reject(message) end
    if previous then
        cape.installSolids(c)
        removeCape(previous)
    end
    return c
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
    if not changed and not (c.deferred and game.tick()%12==0) then return end
    local wanted={}
    for _,key in c.viewerCells do
        for dx=-1,1 do
            for dz=-1,1 do
                local k=key+dx*STRIDE+dz
                if c.cells[k] then wanted[k]=math.min(wanted[k] or math.huge,dx*dx+dz*dz) end
            end
        end
    end
    local available=world.capacity()-world.count()-32
    for _,ids in c.active do available+=#ids end
    local required=0
    for k in wanted do required+=#c.cells[k] end
    c.deferred=required>available
    if c.deferred then
        -- Under transient crowd/effect pressure retain the nearest collision cells
        -- first. The fleet always has the complete region independently of this pool.
        local ranked={}
        for k in wanted do table.insert(ranked,k) end
        table.sort(ranked,function(a,b)
            return wanted[a]<wanted[b] or (wanted[a]==wanted[b] and a<b)
        end)
        for _,k in ranked do
            local n=#c.cells[k]
            if n<=available then available-=n else wanted[k]=nil end
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
    for _,b in c.structures do
        local dx=math.max(math.abs(x-b.x)-b.halfW,0)
        local dz=math.max(math.abs(z-b.z)-b.halfD,0)
        if dx*dx+dz*dz<=r*r then height=math.max(height,b.top) end
    end
    for ix=math.floor((x-r-.5)/CELL),math.floor((x+r+.5)/CELL) do
        for iz=math.floor((z-r-.5)/CELL),math.floor((z+r+.5)/CELL) do
            local trunks=c.cells[ix*STRIDE+iz]
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
