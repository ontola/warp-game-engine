-- Cook an edited cape into one indexed Warp mesh. The stock layout can load its
-- prebuilt equivalent. World-space metres and the terrain triangle diagonal
-- are shared with the tyre/foot contact grid.
local mesh = {}
local v = vector.create
local up = v(0,1,0)

local function height(t,x,z)
    local u,w=(x-t.x)/t.spacing,(z-t.z)/t.spacing
    local i,j=math.floor(u),math.floor(w)
    if i<0 or j<0 or i>=t.columns-1 or j>=t.rows-1 then return 0 end
    local k=j*t.columns+i+1
    local h=t.heights
    local a,b,c,d=h[k],h[k+1],h[k+t.columns],h[k+t.columns+1]
    local fx,fz=u-i,w-j
    if fx+fz<=1 then return a+(b-a)*fx+(c-a)*fz end
    return d+(c-d)*(1-fx)+(b-d)*(1-fz)
end

-- Split a convex polygon at an X/Z line, keeping its intersection points in
-- both halves. This also works for cell diagonals (component X+Z).
local function split(parts,nx,nz,plane)
    local result={}
    for _,p in parts do
        local low,high=false,false
        for _,a in p do
            local d=nx*a.x+nz*a.z-plane
            if d < -.00008 then low=true elseif d > .00008 then high=true end
        end
        if not (low and high) then
            table.insert(result,p)
        else
            local l,h={},{}
            for i,a in p do
                local b=p[i%#p+1]
                local da,db=nx*a.x+nz*a.z-plane,nx*b.x+nz*b.z-plane
                if da<=.00008 then table.insert(l,a) end
                if da>=-.00008 then table.insert(h,a) end
                if (da<-.00008 and db>.00008) or (da>.00008 and db<-.00008) then
                    local hit=a+(b-a)*(da/(da-db))
                    table.insert(l,hit);table.insert(h,hit)
                end
            end
            table.insert(result,l);table.insert(result,h)
        end
    end
    return result
end

local function tessellate(t,poly,emit)
    local xmin,xmax,zmin,zmax=math.huge,-math.huge,math.huge,-math.huge
    for _,p in poly do
        xmin,xmax=math.min(xmin,p.x),math.max(xmax,p.x)
        zmin,zmax=math.min(zmin,p.z),math.max(zmax,p.z)
    end
    local dx=t.spacing
    if xmax<=t.x or xmin>=t.x+(t.columns-1)*dx or zmax<=t.z or zmin>=t.z+(t.rows-1)*dx then
        emit(poly);return
    end
    local parts={poly}
    for i=math.max(0,math.ceil((xmin-t.x)/dx)),math.min(t.columns-1,math.floor((xmax-t.x)/dx)) do
        parts=split(parts,1,0,t.x+i*dx)
    end
    for j=math.max(0,math.ceil((zmin-t.z)/dx)),math.min(t.rows-1,math.floor((zmax-t.z)/dx)) do
        parts=split(parts,0,1,t.z+j*dx)
    end
    for _,p in parts do
        local mid=v(0,0,0)
        for _,point in p do mid+=point end
        mid/=#p
        local i,j=math.floor((mid.x-t.x)/dx),math.floor((mid.z-t.z)/dx)
        if i>=0 and i<t.columns-1 and j>=0 and j<t.rows-1 then
            for _,patch in split({p},1,1,t.x+t.z+(i+j+1)*dx) do emit(patch) end
        else
            emit(p)
        end
    end
end

local function unit(a,b)
    return vector.normalize(b-a)
end

local function ribbon(path)
    local pts,cross,chain={},{},{0}
    for i,p in path do
        pts[i]=v(p[1],0,p[2])
        if i>1 then chain[i]=chain[i-1]+vector.magnitude(pts[i]-pts[i-1]) end
    end
    for i,p in pts do
        local before=if i>1 then unit(pts[i-1],p) else unit(p,pts[i+1])
        local after=if i<#pts then unit(p,pts[i+1]) else before
        local tangent=vector.normalize(before+after)
        -- Turn controls are validated before spawning. Cap a nearly folded
        -- miter at four shoulder widths so it cannot reach across the map.
        cross[i]=v(-tangent.z,0,tangent.x)/math.max(.25,vector.dot(tangent,before))
    end
    return pts,cross,chain
end

function mesh.prepare(data)
    local terrain,coast=data.terrain,data.coast
    local parts,lookups={},{}
    local function material(hex,surface,uv)
        local part={color=hex,roughness=1,positions={},normals={},indices={},surface=surface,
            uvs=if uv then {} else nil}
        table.insert(parts,part)
        lookups[part]={}
        return part
    end
    local grass=material("#6d7a57",40)
    local sand=material("#938c76")
    local rock=material("#807f71")
    local asphalt=material("#616567")
    local shoulder=material("#807b70")
    local paint=material("#c3c2ad")
    local yellow=material("#c2b781")
    local gravel=material("#806d5a",41,true)
    local function patch(part,poly,uvFor,bank)
        local lookup=lookups[part]
        local ids={}
        for i,p in poly do
            local k=p
            local idx=lookup[k]
            if not idx then
                local n=if bank then up else vector.normalize(v(height(terrain,p.x-6,p.z)-height(terrain,p.x+6,p.z),12,
                    height(terrain,p.x,p.z-6)-height(terrain,p.x,p.z+6)))
                local pos=if bank then p else v(p.x,p.y+height(terrain,p.x,p.z),p.z)
                table.insert(part.positions,pos);table.insert(part.normals,n)
                idx=#part.positions;lookup[k]=idx
                if uvFor then table.insert(part.uvs,uvFor(p)) end
            end
            ids[i]=idx
        end
        for i=2,#ids-1 do
            local a,b,c=ids[1],ids[i],ids[i+1]
            local ab,ac=part.positions[b]-part.positions[a],part.positions[c]-part.positions[a]
            local orientation=ab.z*ac.x-ab.x*ac.z
            if math.abs(orientation)<.000003 then continue end
            table.insert(part.indices,a)
            table.insert(part.indices,if orientation>0 then b else c)
            table.insert(part.indices,if orientation>0 then c else b)
        end
    end
    local function surface(part,poly,uv)
        tessellate(terrain,poly,function(points) patch(part,points,uv,false) end)
    end
    for i=2,#coast do
        local a,b=coast[i-1],coast[i]
        surface(grass,{v(a[1],0,a[2]),v(a[1],0,a[3]),v(b[1],0,b[3]),v(b[1],0,b[2])})
    end
    -- The sandy/rocky shelf slopes below sea level, ending at the city join.
    local shore={}
    for i=#coast,1,-1 do table.insert(shore,v(coast[i][1],0,coast[i][3])) end
    for _,p in coast do table.insert(shore,v(p[1],0,p[2])) end
    local outer,bluffs={},{}
    for i,p in shore do
        local tangent=unit(shore[math.max(1,i-1)],shore[math.min(#shore,i+1)])
        local bluff=(.5+.5*math.sin(p.x*.013+p.z*.019))^2
        local reach=12+(1-bluff)*15+2*math.sin(p.x*.077)
        outer[i]=v(math.min(coast[#coast][1],p.x+tangent.z*reach),-5.7,p.z-tangent.x*reach)
        bluffs[i]=bluff
    end
    for i=2,#shore do
        patch(if bluffs[i]+bluffs[i-1]>1.30 then rock else sand,
            {shore[i-1],shore[i],outer[i],outer[i-1]},nil,true)
    end
    local function strip(part,a,b,sa,sb,left,right,lift,uv)
        local upY=v(0,lift,0)
        surface(part,{a+left*sa+upY,a+right*sa+upY,b+right*sb+upY,b+left*sb+upY},uv)
    end
    -- The two asphalt paths share one flush junction as they do in the shipped mesh.
    local center=data.roads[1][#data.roads[1]]
    local join=v(center[1],0,center[2])
    local half=data.widths.asphalt*.5
    local ends={}
    for arm=1,2 do
        local path=table.clone(data.roads[arm])
        local function trim(p)
            local endpt=v(p[1],0,p[2])
            local q=join+unit(join,endpt)*math.min(half*3.78,vector.magnitude(endpt-join)*.45)
            return {q.x,q.z}
        end
        path[#path]=trim(path[#path-1])
        if arm==2 then path[1]=trim(path[2]) end
        local pts,sides,chain=ribbon(path)
        for i=1,#pts-1 do
            local a,b,sa,sb=pts[i],pts[i+1],sides[i],sides[i+1]
            local verge=half+.6
            strip(shoulder,a,b,sa,sb,-verge,-half,.039)
            strip(shoulder,a,b,sa,sb,half,verge,.039)
            strip(asphalt,a,b,sa,sb,-half,half,.039)
            for _,side in {-1,1} do
                strip(paint,a,b,sa,sb,(half*.895-.06)*side,(half*.895+.06)*side,.054)
            end
            local dir=unit(a,b)
            local left=v(-dir.z,0,dir.x)
            for station=math.floor(chain[i]/17)*17,chain[i+1],17 do
                local lo,hi=math.max(chain[i],station),math.min(chain[i+1],station+4.5)
                if hi>lo+.001 then
                    strip(yellow,a+dir*(lo-chain[i]),a+dir*(hi-chain[i]),left,left,-.088,.088,.061)
                end
            end
        end
        local function endCap(i)
            for _,side in {-1,1} do
                local p=pts[i]+sides[i]*half*side
                table.insert(ends,{p=p,outer=pts[i]+sides[i]*(half+.6)*side,arm=arm*2+(if i==1 then 1 else 0),
                    angle=math.atan2(p.z-join.z,p.x-join.x)})
            end
        end
        if arm==2 then endCap(1) end
        endCap(#pts)
    end
    table.sort(ends,function(a,b) return a.angle<b.angle end)
    local lift=v(0,.039,0)
    for i,a in ends do
        local b=ends[i%#ends+1]
        surface(asphalt,{join+lift,a.p+lift,b.p+lift})
        if a.arm~=b.arm then surface(shoulder,{a.p+lift,a.outer+lift,b.outer+lift,b.p+lift}) end
    end
    local pts,sides,chain=ribbon(data.roads[3])
    local gravelHalf=data.widths.gravel*.5
    for i=1,#pts-1 do
        local a,b=pts[i],pts[i+1]
        local direction=unit(a,b)
        local function uv(p)
            local d=p-a
            local across=(d.z*direction.x-d.x*direction.z)*(2.60/gravelHalf)
            local along=chain[i]+math.clamp(vector.dot(d,direction),0,chain[i+1]-chain[i])
            return v(.5+across/8,along/1024,0)
        end
        local visualHalf=gravelHalf*(3.24/2.60)
        strip(gravel,a,b,sides[i],sides[i+1],-visualHalf,visualHalf,.025,uv)
    end
    local used={}
    for _,part in parts do if #part.indices>0 then table.insert(used,part) end end
    return used
end

function mesh.spawnPrepared(parts)
    return world.mesh(parts,{tag="west-cape",solid=false})
end

function mesh.spawn(data)
    return mesh.spawnPrepared(mesh.prepare(data))
end

return mesh
