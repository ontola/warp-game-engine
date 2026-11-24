-- Compile the authored cape into the shared geometry used by draw, physics,
-- navigation and missions. Also run by the offline GLB baker.
local function within(n,lo,hi)
    return type(n)=="number" and n>=lo and n<=hi
end

local function whole(n,lo,hi)
    return within(n,lo,hi) and n%1==0
end

local function list(t,lo,hi)
    if type(t)~="table" or #t<lo or #t>hi then return false end
    local count=0
    for key in t do
        if not whole(key,1,#t) then return false end
        count+=1
    end
    return count==#t
end

local function point(p)
    return list(p,2,2) and within(p[1],-8192,8192) and within(p[2],-8192,8192)
end

local placeNames={"grainYard","lookout","marinerWood","lookoutMeadow","lookoutStop","ferryTerminal"}
local function checkInput(def)
    if type(def)~="table" or def.kind~="west-cape-layout" then return "expected a west-cape-layout" end
    if not list(def.coast,2,96) then return "coast needs 2..96 west-to-east slices" end
    for i,p in def.coast do
        if not list(p,3,3) or not within(p[1],-8192,-178)
            or not within(p[2],-8192,8192) or not within(p[3],-8192,8192)
            or p[3]<=p[2] or (i>1 and p[1]<def.coast[i-1][1]+1) then
            return "coast slices must run west to east and have separate north and south banks"
        end
    end
    local east=def.coast[#def.coast]
    if east[1]~=-178 or east[2]~=-178 or east[3]~=178 then return "end the cape at the city seam {-178,-178,178}" end
    local ring,stage=def.asphalt,def.rally
    if type(ring)~="table" or not within(ring.width,4,20) or not within(ring.radius,1,200)
        or not point(ring.junction) or not list(ring.bends,2,64) then
        return "asphalt needs a junction, 2..64 bends, width 4..20 and radius 1..200"
    end
    for _,p in ring.bends do if not point(p) then return "asphalt bends must be finite {x,z} points" end end
    if type(stage)~="table" or not within(stage.width,3,14) or not within(stage.radius,1,200)
        or not within(stage.carAlong,-25,25) or not within(stage.carRight,-25,25)
        or not list(stage.bends,3,64) then return "rally needs 3..64 bends, width 3..14, radius 1..200 and a car offset within 25 m" end
    for i,p in stage.bends do
        if not list(p,2,3) or not within(p[1],-8192,8192) or not within(p[2],-8192,8192) then
            return "rally bends must be finite {x,z} points with an optional checkpoint sample"
        end
        if #p==3 and (i==1 or i==#stage.bends or not whole(p[3],1,4)) then
            return "interior rally bends accept checkpoint samples 1, 2, 3 or 4"
        end
    end
    local t=def.terrain
    if type(t)~="table" or not within(t.x,-8192,8192) or not within(t.z,-8192,8192)
        or not within(t.spacing,4,128) or not within(t.maxGrade,.05,.45)
        or not whole(t.rows,2,64) or not whole(t.columns,2,64) or t.rows*t.columns>1024 then
        return "terrain needs a finite origin, spacing 4..128, maxGrade 0.05..0.45 and at most 1024 samples"
    end
    if not list(t.hills,0,16) then return "terrain.hills accepts up to 16 hills" end
    for _,p in t.hills do
        if not list(p,5,5) or not within(p[1],-8192,8192) or not within(p[2],-8192,8192)
            or not within(p[3],1,2048) or not within(p[4],1,2048) or not within(p[5],0,160) then
            return "hills need finite {x,z,radiusX,radiusZ,height} and positive radii"
        end
    end
    if type(def.places)~="table" then return "places must name the cape's landmarks" end
    for _,name in placeNames do if not point(def.places[name]) then return "missing or invalid places."..name end end
    if not list(def.groves,0,16) then return "groves accepts up to 16 forest ellipses" end
    local cells=0
    for _,g in def.groves do
        if not list(g,4,4) or not within(g[1],-8192,8192) or not within(g[2],-8192,8192)
            or not within(g[3],1,256) or not within(g[4],1,256) then return "groves need finite {x,z,radiusX,radiusZ}" end
        -- Count the actual 27 m planting grid before generating any trunks.
        cells+=(math.ceil((g[1]+g[3])/27)-math.floor((g[1]-g[3])/27)+1)
            *(math.ceil((g[2]+g[4])/27)-math.floor((g[2]-g[4])/27)+1)
    end
    if cells>420 then return "forest planting exceeds 420 candidate cells" end
    return nil
end

local function distance(a,b)
    local dx,dz=b[1]-a[1],b[2]-a[2]
    return math.sqrt(dx*dx+dz*dz)
end

local function mix(a,b,t)
    return {a[1]+(b[1]-a[1])*t,a[2]+(b[2]-a[2])*t}
end

local function cm(n)
    return math.round(n*100)/100
end

local function rounded(points,radius,gates)
    local out={points[1]}
    for i=2,#points-1 do
        local a,p,b=points[i-1],points[i],points[i+1]
        local before,after=distance(a,p),distance(p,b)
        if math.min(before,after)<1 then return nil,"consecutive road bends must be at least one metre apart" end
        local trim=math.min(radius,before*.18,after*.18)
        local first,last=mix(p,a,trim/before),mix(p,b,trim/after)
        for k=0,3 do
            local t=k/3
            local q=mix(mix(first,p,t),mix(p,last,t),t)
            q={cm(q[1]),cm(q[2])}
            if distance(out[#out],q)<=.01 then return nil,"road curve doubles back on itself" end
            table.insert(out,q)
            if gates and p[3]==k+1 then table.insert(gates,q) end
        end
    end
    table.insert(out,points[#points])
    return out
end

local function nearest(path,p)
    local best,closest=math.huge,nil
    for i=2,#path do
        local a,b=path[i-1],path[i]
        local dx,dz=b[1]-a[1],b[2]-a[2]
        local t=math.clamp(((p[1]-a[1])*dx+(p[2]-a[2])*dz)/(dx*dx+dz*dz),0,1)
        local q=mix(a,b,t)
        local d=distance(p,q)
        if d<best then best,closest=d,q end
    end
    return closest
end

local function terrainGrid(def)
    local t={x=def.x,z=def.z,spacing=def.spacing,columns=def.columns,rows=def.rows,heights={}}
    local step,hills=def.maxGrade*t.spacing,def.hills
    local samples={}
    for j=0,t.rows-1 do
        for i=0,t.columns-1 do
            local x,z=t.x+i*t.spacing,t.z+j*t.spacing
            local f=math.min(1,math.min(i,j,t.columns-1-i,t.rows-1-j)/2)
            local height=0
            for _,h in hills do
                local dx,dz=(x-h[1])/h[3],(z-h[2])/h[4]
                height+=h[5]*math.exp(-dx*dx-dz*dz)
            end
            table.insert(samples,math.round(height*f*f*(3-2*f)*4)/4)
        end
    end
    -- A lower neighbour can limit this height only within height / step cells.
    local visits=0
    for k,height in samples do
        local i,j=(k-1)%t.columns,math.floor((k-1)/t.columns)
        local reach=math.ceil(height/step)
        local x0,x1=math.max(0,i-reach),math.min(t.columns-1,i+reach)
        local y0,y1=math.max(0,j-reach),math.min(t.rows-1,j+reach)
        visits+=(y1-y0+1)*(x1-x0+1)
        if visits>32000 then return nil,"terrain grading exceeds the live editing budget" end
        for y=y0,y1 do
            for x=x0,x1 do
                local dx,dz=x-i,y-j
                height=math.min(height,samples[y*t.columns+x+1]+step*math.sqrt(dx*dx+dz*dz))
            end
        end
        t.heights[k]=math.round(height*4)/4
    end
    return t
end

local function heightAt(t,x,z)
    local u,v=(x-t.x)/t.spacing,(z-t.z)/t.spacing
    local i,j=math.floor(u),math.floor(v)
    if i<0 or j<0 or i>=t.columns-1 or j>=t.rows-1 then return 0 end
    local fx,fz=u-i,v-j
    local k,h=1+j*t.columns+i,t.heights
    local a,b,c,d=h[k],h[k+1],h[k+t.columns],h[k+t.columns+1]
    return if fx+fz<=1 then a+(b-a)*fx+(c-a)*fz else d+(c-d)*(1-fx)+(b-d)*(1-fz)
end

local function onLand(coast,x,z,margin)
    if x<coast[1][1] or x>coast[#coast][1] then return false end
    for i=2,#coast do
        local a,b=coast[i-1],coast[i]
        if x<=b[1] then
            local t=(x-a[1])/(b[1]-a[1])
            return z>=a[2]+t*(b[2]-a[2])+margin and z<=a[3]+t*(b[3]-a[3])-margin
        end
    end
    return false
end

local function geometryKey(data)
    -- Mix numeric samples in explicit traversal order. Micrometre quantization
    -- is finer than the vertex precision of the baked mesh.
    local a,b=17,19
    local function add(n)
        local v=math.round(n*1e6)
        a=(a*257+v)%2147483647
        b=(b*263+v)%2147483629
    end
    for _,row in data.coast do for _,n in row do add(n) end end
    for _,road in data.roads do
        add(#road)
        for _,p in road do add(p[1]); add(p[2]) end
    end
    for i=1,math.min(2,#data.groves) do for _,n in data.groves[i] do add(n) end end
    add(data.places.grainYard[1])
    local t=data.terrain
    for _,n in {t.x,t.z,t.spacing,t.columns,t.rows,data.widths.asphalt,data.widths.gravel} do add(n) end
    for _,h in t.heights do add(h) end
    return string.format("%08x%08x",a,b)
end

local function build(def)
    local err=checkInput(def)
    if err then return nil,err end
    local ring,stage=def.asphalt,def.rally
    local widths={asphalt=ring.width,gravel=stage.width}
    local coast,groves,places={},{},{}
    for i,p in def.coast do coast[i]=table.clone(p) end
    for i,g in def.groves do
        if not onLand(coast,g[1],g[2],0) then return nil,"grove centre must be on land" end
        groves[i]=table.clone(g)
    end
    for _,name in placeNames do
        local p=def.places[name]
        local cityTerminal=name=="ferryTerminal" and math.abs(p[1])<=177 and math.abs(p[2])<=177
        if not cityTerminal and not onLand(coast,p[1],p[2],1) then return nil,name.." must be on playable land" end
        places[name]=table.clone(p)
    end

    local junction=table.clone(ring.junction)
    local connector={{coast[#coast][1],0},junction}
    if distance(connector[1],junction)<=18 then return nil,"junction needs room for the city connector" end
    local controls={junction}
    for _,p in ring.bends do table.insert(controls,p) end
    table.insert(controls,junction)
    local loop,ringError=rounded(controls,ring.radius)
    if not loop then return nil,ringError end
    if distance(junction,loop[2])<=17 or distance(junction,loop[#loop-1])<=17 then
        return nil,"junction needs 17 m of clearance on each arm"
    end
    local bends,gates,gateBends=table.clone(stage.bends),{},{}
    for i,p in bends do if p[3] then table.insert(gateBends,i) end end
    table.insert(gateBends,#bends)
    bends[1]=nearest(loop,bends[1])
    bends[#bends]=nearest(loop,bends[#bends])
    local trail,trailError=rounded(bends,stage.radius,gates)
    if not trail then return nil,trailError end
    table.insert(gates,trail[#trail])
    local a,b=trail[1],trail[2]
    local len=distance(a,b)
    local dx,dz=(b[1]-a[1])/len,(b[2]-a[2])/len
    local start={cm(a[1]+stage.carAlong*dx-stage.carRight*dz),cm(a[2]+stage.carAlong*dz+stage.carRight*dx)}
    if not onLand(coast,start[1],start[2],2) then return nil,"rally car must start on land" end
    -- The collision grid is rectangular. Keep its four edges inside the
    -- linear coastline, including any intervening indentations.
    local terrain=def.terrain
    local x0,x1=terrain.x,terrain.x+(terrain.columns-1)*terrain.spacing
    local z0,z1=terrain.z,terrain.z+(terrain.rows-1)*terrain.spacing
    for _,x in {x0,x1} do
        if not onLand(coast,x,z0,0) or not onLand(coast,x,z1,0) then
            return nil,"move or resize the rectangular terrain grid so it stays on land"
        end
    end
    for _,row in coast do
        if row[1]>x0 and row[1]<x1 and (row[2]>z0 or row[3]<z1) then
            return nil,"the terrain grid crosses a shoreline; resize it or reshape the bank"
        end
    end
    local clearance=stage.width*.5+.64
    for _,p in trail do
        if p[1]<x0+clearance or p[1]>x1-clearance or p[2]<z0+clearance or p[2]>z1-clearance then
            return nil,"move or expand the terrain grid to cover the gravel road and its shoulders"
        end
    end
    local t,terrainError=terrainGrid(terrain)
    if not t then return nil,terrainError end
    local roads={connector,loop,trail}
    local roadCellVisits=0
    for route,path in roads do
        local margin=(if route==3 then widths.gravel else widths.asphalt)*.5+.6
        local travelled=0
        for i=2,#path do
            local p,q=path[i-1],path[i]
            -- Linear banks and segments reach their minimum clearance at an
            -- endpoint or at an intervening coast slice.
            if not onLand(coast,p[1],p[2],margin) or not onLand(coast,q[1],q[2],margin) then return nil,"road leaves the cape" end
            for _,row in coast do
                if row[1]>math.min(p[1],q[1]) and row[1]<math.max(p[1],q[1]) then
                    local z=p[2]+(q[2]-p[2])*(row[1]-p[1])/(q[1]-p[1])
                    if z<row[2]+margin or z>row[3]-margin then return nil,"road crosses a bank" end
                end
            end
            -- These are the actual padded spatial bins in west-cape. A long
            -- diagonal can otherwise claim a rectangle of mostly empty cells.
            local nx=math.floor((math.max(p[1],q[1])+21)/24)-math.floor((math.min(p[1],q[1])-21)/24)+1
            local nz=math.floor((math.max(p[2],q[2])+21)/24)-math.floor((math.min(p[2],q[2])-21)/24)+1
            roadCellVisits+=nx*nz
            if roadCellVisits>7000 then return nil,"road index is too large; shorten or subdivide long diagonal segments" end
            local length=distance(p,q)
            travelled+=length
            if travelled>8192 then return nil,"a single road exceeds 8 km of centreline" end
            if route==3 then
                local count=math.ceil(length/4)
                local prev=heightAt(t,p[1],p[2])
                for j=1,count do
                    local u=j/count
                    local h=heightAt(t,p[1]+u*(q[1]-p[1]),p[2]+u*(q[2]-p[2]))
                    if math.abs(h-prev)>.30*length/count+1e-6 then return nil,"gravel grade exceeds 30%" end
                    prev=h
                end
            end
        end
    end
    local out={coast=coast,roads=roads,widths=widths,terrain=t,groves=groves,places=places,
        rally={start=start,yaw=cm(math.atan2(-dx,-dz)),gates=gates,gateBends=gateBends}}
    out.bakeKey=geometryKey(out)
    return out
end

local default,err=build(require("west_cape_layout"))
-- Initialization also works with an invalid source under hot reload: expose the
-- validator and its diagnostic, allowing the last good layout to stay active.
if not default then return {build=build,error=err} end
default.build=build
return default
